//! Manual Library/Browse intent adapter. The operation journal/executor remains
//! the only mutation authority. Tickets are bounded Main-lifetime references,
//! not paths, managed imports, a clipboard filesystem or a second work queue.
use super::*;
mod clipboard;
use crate::db::{LibrarySelectionV1, OperationPreviewDto};
use crate::file_workspace::browse::BrowseSelectionCompletion;
use crate::file_workspace::integration::FileWorkspaceRuntime;
use crate::file_workspace::{BrowseEntryRef, EntryRef};
use crate::fs_safety::{DirectoryIdentity, PhysicalFileIdentity, VerifiedDirectory};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub(crate) const LIMIT: usize = 1000;
pub(crate) const EXTERNAL_PATH_LIMIT: usize = 256;
#[cfg(target_os = "macos")]
const EXTERNAL_TREE_ENTRY_LIMIT: usize =
    crate::fs_safety::identity::DIRECTORY_IDENTITY_BUFFER_BATCH_SIZE;
const EXTERNAL_TREE_DEPTH_LIMIT: usize = crate::fs_safety::identity::DIRECTORY_IDENTITY_DEPTH_LIMIT;
#[derive(Default)]
pub(crate) struct TransferState {
    sources: HashMap<String, Source>,
    previews: HashMap<String, TransferPlan>,
}
#[derive(Clone)]
pub(crate) struct Source {
    ticket: TransferTicket,
    file_id: String,
    identity: FileIdentityFingerprint,
    relative_path: PathBuf,
    tree_guard: Option<TreeGuard>,
    browse_selection: Option<BrowseSelectionOrigin>,
}

#[derive(Clone)]
struct BrowseSelectionOrigin {
    session_id: String,
    snapshot_id: String,
    path: PathBuf,
    identity: PhysicalFileIdentity,
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

const MAX_PERSISTED_BASKET_ENTRIES: usize = 100_000;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BasketEntryDto {
    pub id: String,
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub status: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BasketPageDto {
    pub entries: Vec<BasketEntryDto>,
    pub total: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BasketPageRequest {
    pub offset: usize,
    pub limit: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BasketEntriesRequest {
    pub entry_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BasketAddRequest {
    pub ticket_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BasketMoveReconcileRequest {
    pub moves: Vec<BasketMoveReconcileItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BasketMoveReconcileItem {
    pub entry_id: String,
    pub operation_log_id: String,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureSelectionPageRequest {
    pub session_id: String,
    pub snapshot_id: String,
    pub offset: usize,
    #[serde(default)]
    pub excluded_entry_ids: Vec<String>,
    #[serde(default)]
    pub advance_snapshot: bool,
}
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TransferAction {
    Copy,
    Move,
    Duplicate,
    Rename,
    Trash,
    SystemTrash,
    CreateDirectory,
    CreateFile,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferBrowseDestination {
    pub session_id: String,
    pub path_ref: crate::file_workspace::BrowsePathRef,
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
    pub destination_browse: Option<TransferBrowseDestination>,
    #[serde(default)]
    pub new_name: Option<String>,
    #[serde(default)]
    pub new_names: HashMap<String, String>,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeDragOutRequest {
    pub ticket_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ZipCreateRequest {
    pub ticket_ids: Vec<String>,
    pub output_path: String,
    #[serde(default = "default_zip_compression_level")]
    pub compression_level: i32,
}

fn default_zip_compression_level() -> i32 {
    6
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZipExtractTarget {
    Here,
    SameNameFolder,
    ChosenDirectory,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ZipExtractRequest {
    pub ticket_id: String,
    pub target: ZipExtractTarget,
    #[serde(default)]
    pub destination_directory: Option<String>,
    pub conflict_policy: archive::ZipConflictPolicy,
}

impl FileWorkspaceRuntime {
    pub(crate) fn capture_selection_snapshot_page(
        &self,
        request: CaptureSelectionPageRequest,
    ) -> Result<Vec<TransferTicket>, String> {
        self.ensure_live()?;
        let entries = self
            .inner
            .browse
            .selection_snapshot_page(
                &request.session_id,
                &request.snapshot_id,
                request.offset,
                LIMIT,
                &request.excluded_entry_ids,
            )
            .map_err(|error| format!("browse_{error}"))?;
        let captured = entries
            .into_iter()
            .map(|entry| {
                let origin = request.advance_snapshot.then(|| BrowseSelectionOrigin {
                    session_id: request.session_id.clone(),
                    snapshot_id: request.snapshot_id.clone(),
                    path: entry.path.clone(),
                    identity: entry.identity.clone(),
                });
                let mut source = self.capture_transfer_source(
                    entry.path,
                    super::RECOVERY_ACTION_FILE_ID.into(),
                    Some(entry.identity),
                    Some(entry.kind),
                )?;
                source.browse_selection = origin;
                Ok(source)
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.register_transfer_sources(captured, Vec::new())
    }

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
            let (path, file_id, expected_identity, expected_kind) = match reference {
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
                    (PathBuf::from(detail.path), file_id, None, None)
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
                    (
                        resolved.path,
                        super::RECOVERY_ACTION_FILE_ID.into(),
                        resolved.identity,
                        Some(resolved.kind),
                    )
                }
            };
            captured.push(self.capture_transfer_source(
                path,
                file_id,
                expected_identity,
                expected_kind,
            )?);
        }
        if !request.external_paths.is_empty() {
            captured.extend(capture_external_sources(request.external_paths)?);
        }
        self.register_transfer_sources(captured, request.release_ids)
    }

    pub(crate) fn basket_add(&self, request: BasketAddRequest) -> Result<usize, String> {
        self.ensure_live()?;
        if request.ticket_ids.is_empty() || request.ticket_ids.len() > LIMIT {
            return Err("transfer_selection_limit_1000".into());
        }
        let state = self
            .inner
            .transfers
            .lock()
            .map_err(|_| "transfer_state_unavailable")?;
        let mut seen = HashSet::new();
        let sources = request
            .ticket_ids
            .iter()
            .filter(|id| seen.insert((*id).clone()))
            .map(|id| {
                state
                    .sources
                    .get(id)
                    .cloned()
                    .ok_or_else(|| "transfer_reference_expired".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        drop(state);

        let records = sources
            .into_iter()
            .map(|source| {
                if file_namespace_fingerprint(Path::new(&source.ticket.path))? != source.identity {
                    return Err("transfer_source_changed".to_string());
                }
                let volume_id = source
                    .identity
                    .platform_volume_id
                    .clone()
                    .ok_or_else(|| "transfer_identity_unavailable".to_string())?;
                let object_id = source
                    .identity
                    .platform_file_id
                    .clone()
                    .ok_or_else(|| "transfer_identity_unavailable".to_string())?;
                let identity_json =
                    serde_json::to_string(&source.identity).map_err(|error| error.to_string())?;
                Ok((
                    source.ticket.path,
                    source.ticket.name,
                    source.ticket.is_directory,
                    identity_json,
                    volume_id,
                    object_id,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;

        let mut conn = self
            .inner
            .database
            .conn()
            .map_err(|error| error.to_string())?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let mut added = 0usize;
        let mut total: i64 = tx
            .query_row("SELECT COUNT(*) FROM file_workspace_basket", [], |row| {
                row.get(0)
            })
            .map_err(|error| error.to_string())?;
        for (path, name, is_directory, identity_json, volume_id, object_id) in records {
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM file_workspace_basket WHERE path=?1 AND volume_id=?2 AND object_id=?3)",
                    params![path, volume_id, object_id],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if exists {
                continue;
            }
            if total >= MAX_PERSISTED_BASKET_ENTRIES as i64 {
                return Err("basket_limit_100000".into());
            }
            tx.execute(
                "INSERT INTO file_workspace_basket(id,path,name,is_directory,identity_json,file_id,volume_id,object_id,added_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",
                params![
                    new_job_id("file-basket"),
                    path,
                    name,
                    i64::from(is_directory),
                    identity_json,
                    super::RECOVERY_ACTION_FILE_ID,
                    volume_id,
                    object_id,
                    basket_timestamp(),
                ],
            )
            .map_err(|error| error.to_string())?;
            added += 1;
            total += 1;
        }
        tx.commit().map_err(|error| error.to_string())?;
        Ok(added)
    }

    pub(crate) fn basket_page(&self, request: BasketPageRequest) -> Result<BasketPageDto, String> {
        self.ensure_live()?;
        if request.limit == 0
            || request.limit > 100
            || request.offset > MAX_PERSISTED_BASKET_ENTRIES
        {
            return Err("basket_page_invalid".into());
        }
        let conn = self
            .inner
            .database
            .conn()
            .map_err(|error| error.to_string())?;
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM file_workspace_basket", [], |row| {
                row.get(0)
            })
            .map_err(|error| error.to_string())?;
        let mut statement = conn
            .prepare("SELECT id,path,name,is_directory,identity_json,volume_id,object_id FROM file_workspace_basket ORDER BY added_at,id LIMIT ?1 OFFSET ?2")
            .map_err(|error| error.to_string())?;
        let raw = statement
            .query_map(
                params![request.limit as i64, request.offset as i64],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)? != 0,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let entries = raw
            .into_iter()
            .map(
                |(id, path, name, is_directory, identity_json, volume_id, object_id)| {
                    let status =
                        match serde_json::from_str::<FileIdentityFingerprint>(&identity_json) {
                            Ok(identity)
                                if identity.platform_volume_id.as_deref()
                                    == Some(volume_id.as_str())
                                    && identity.platform_file_id.as_deref()
                                        == Some(object_id.as_str()) =>
                            {
                                basket_entry_status(Path::new(&path), is_directory, &identity)
                            }
                            _ => "changed".to_string(),
                        };
                    BasketEntryDto {
                        id,
                        name,
                        path,
                        is_directory,
                        status,
                    }
                },
            )
            .collect();
        Ok(BasketPageDto {
            entries,
            total: total as usize,
        })
    }

    pub(crate) fn basket_capture(
        &self,
        request: BasketEntriesRequest,
    ) -> Result<Vec<TransferTicket>, String> {
        self.ensure_live()?;
        if request.entry_ids.is_empty() || request.entry_ids.len() > LIMIT {
            return Err("transfer_selection_limit_1000".into());
        }
        let records = {
            let mut conn = self
                .inner
                .database
                .conn()
                .map_err(|error| error.to_string())?;
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Deferred)
                .map_err(|error| error.to_string())?;
            let mut records = Vec::with_capacity(request.entry_ids.len());
            let mut seen = HashSet::new();
            for id in request.entry_ids {
                if !seen.insert(id.clone()) {
                    continue;
                }
                let raw = tx
                    .query_row(
                        "SELECT path,file_id,is_directory,identity_json FROM file_workspace_basket WHERE id=?1",
                        [&id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, i64>(2)? != 0,
                                row.get::<_, String>(3)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(|error| error.to_string())?
                    .ok_or("basket_entry_not_found")?;
                records.push(raw);
            }
            tx.commit().map_err(|error| error.to_string())?;
            records
        };
        let mut sources = Vec::with_capacity(records.len());
        for (path, file_id, is_directory, identity_json) in records {
            let expected: FileIdentityFingerprint =
                serde_json::from_str(&identity_json).map_err(|_| "basket_identity_invalid")?;
            if basket_entry_status(Path::new(&path), is_directory, &expected) != "available" {
                return Err("basket_entry_unavailable".into());
            }
            let source = self.capture_transfer_source(PathBuf::from(path), file_id, None, None)?;
            if source.identity != expected || source.ticket.is_directory != is_directory {
                return Err("basket_entry_changed".into());
            }
            sources.push(source);
        }
        self.register_transfer_sources(sources, Vec::new())
    }

    pub(crate) fn basket_remove(&self, request: BasketEntriesRequest) -> Result<usize, String> {
        self.ensure_live()?;
        if request.entry_ids.is_empty() || request.entry_ids.len() > LIMIT {
            return Err("basket_entry_selection_invalid".into());
        }
        let mut conn = self
            .inner
            .database
            .conn()
            .map_err(|error| error.to_string())?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let mut removed = 0;
        for id in request.entry_ids {
            removed += tx
                .execute("DELETE FROM file_workspace_basket WHERE id=?1", [id])
                .map_err(|error| error.to_string())?;
        }
        tx.commit().map_err(|error| error.to_string())?;
        Ok(removed)
    }

    pub(crate) fn basket_clear(&self) -> Result<usize, String> {
        self.ensure_live()?;
        let conn = self
            .inner
            .database
            .conn()
            .map_err(|error| error.to_string())?;
        conn.execute("DELETE FROM file_workspace_basket", [])
            .map_err(|error| error.to_string())
    }

    pub(crate) fn basket_reconcile_moves(
        &self,
        request: BasketMoveReconcileRequest,
    ) -> Result<usize, String> {
        self.ensure_live()?;
        if request.moves.len() > LIMIT {
            return Err("transfer_selection_limit_1000".into());
        }
        let mut updated = 0usize;
        for item in request.moves {
            let log = self
                .inner
                .database
                .get_operation_log_by_id(&item.operation_log_id)
                .map_err(|error| error.to_string())?
                .ok_or("basket_move_log_not_found")?;
            if log.operation_type != "move" || log.status != "success" {
                continue;
            }
            let conn = self
                .inner
                .database
                .conn()
                .map_err(|error| error.to_string())?;
            let row = conn
                .query_row(
                    "SELECT path,is_directory,identity_json FROM file_workspace_basket WHERE id=?1",
                    [&item.entry_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)? != 0,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(|error| error.to_string())?;
            drop(conn);
            let Some((old_path, is_directory, old_identity_json)) = row else {
                continue;
            };
            let old_identity: FileIdentityFingerprint =
                serde_json::from_str(&old_identity_json).map_err(|_| "basket_identity_invalid")?;
            if old_path != log.path_before
                || old_identity.platform_file_id.as_deref()
                    != log.source_platform_file_id.as_deref()
                || old_identity.platform_volume_id.as_deref()
                    != log.source_platform_volume_id.as_deref()
            {
                continue;
            }
            let target = Path::new(&log.path_after);
            let target_identity = match file_namespace_fingerprint(target) {
                Ok(identity)
                    if identity.platform_file_id.as_deref()
                        == log.target_platform_file_id.as_deref()
                        && identity.platform_volume_id.as_deref()
                            == log.target_platform_volume_id.as_deref()
                        && target.is_dir() == is_directory =>
                {
                    identity
                }
                _ => continue,
            };
            let volume_id = target_identity
                .platform_volume_id
                .as_deref()
                .ok_or("transfer_identity_unavailable")?;
            let object_id = target_identity
                .platform_file_id
                .as_deref()
                .ok_or("transfer_identity_unavailable")?;
            let mut conn = self
                .inner
                .database
                .conn()
                .map_err(|error| error.to_string())?;
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| error.to_string())?;
            let changed = tx
                .execute(
                    "UPDATE file_workspace_basket SET path=?1,name=?2,identity_json=?3,volume_id=?4,object_id=?5,updated_at=?6 WHERE id=?7 AND path=?8 AND identity_json=?9",
                    params![
                        log.path_after,
                        log.name_after,
                        serde_json::to_string(&target_identity).map_err(|error| error.to_string())?,
                        volume_id,
                        object_id,
                        basket_timestamp(),
                        item.entry_id,
                        old_path,
                        old_identity_json,
                    ],
                )
                .map_err(|error| error.to_string())?;
            tx.commit().map_err(|error| error.to_string())?;
            updated += changed;
        }
        Ok(updated)
    }

    fn capture_transfer_source(
        &self,
        path: PathBuf,
        file_id: String,
        expected_identity: Option<PhysicalFileIdentity>,
        expected_kind: Option<crate::file_workspace::browse::BrowseEntryKind>,
    ) -> Result<Source, String> {
        if let Some(expected_identity) = expected_identity {
            let current = crate::fs_safety::capture_browse_identity(&path)
                .map_err(|error| format!("transfer_source_changed: {error}"))?;
            if current != expected_identity
                || expected_kind.is_some_and(|kind| {
                    path.is_dir()
                        != (kind == crate::file_workspace::browse::BrowseEntryKind::Directory)
                })
            {
                return Err("transfer_source_changed".into());
            }
        }
        #[cfg(windows)]
        ensure_transfer_source_content_ready(&path)?;
        let path = if cfg!(windows) && path.is_dir() {
            if !path.is_absolute() {
                return Err("transfer_source_invalid".into());
            }
            crate::fs_safety::capture_source_object_identity(&path)
                .map_err(|error| error.to_string())?;
            ensure_general_file_operation_allowed(&path)?;
            path.canonicalize().map_err(|error| error.to_string())?
        } else {
            validate_source_path(&path)?
        };
        VerifiedDirectory::open_existing(path.parent().ok_or("transfer_source_invalid")?)
            .map_err(|error| error.to_string())?;
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
        Ok(Source {
            ticket,
            file_id,
            identity,
            relative_path,
            tree_guard: None,
            browse_selection: None,
        })
    }

    fn register_transfer_sources(
        &self,
        mut captured: Vec<Source>,
        release_ids: Vec<String>,
    ) -> Result<Vec<TransferTicket>, String> {
        let mut state = self
            .inner
            .transfers
            .lock()
            .map_err(|_| "transfer_state_unavailable")?;
        let release_ids = release_ids.into_iter().collect::<HashSet<_>>();
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

    fn zip_sources(&self, ticket_ids: &[String]) -> Result<Vec<archive::ZipSource>, String> {
        self.ensure_live()?;
        if ticket_ids.is_empty() || ticket_ids.len() > LIMIT {
            return Err("transfer_selection_limit_1000".into());
        }
        let unique = ticket_ids.iter().collect::<HashSet<_>>();
        if unique.len() != ticket_ids.len() {
            return Err("transfer_selection_invalid".into());
        }
        let sources = {
            let state = self
                .inner
                .transfers
                .lock()
                .map_err(|_| "transfer_state_unavailable")?;
            ticket_ids
                .iter()
                .map(|id| {
                    state
                        .sources
                        .get(id)
                        .cloned()
                        .ok_or("transfer_reference_expired")
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        sources
            .into_iter()
            .map(|source| {
                let path = validate_transfer_source_path(Path::new(&source.ticket.path))?;
                if path.is_dir() != source.ticket.is_directory
                    || file_namespace_fingerprint(&path)? != source.identity
                {
                    return Err("transfer_source_changed".into());
                }
                if let Some(guard) = source.tree_guard.as_ref() {
                    validate_tree_guard(guard)?;
                }
                let directory_tree_fingerprint = source
                    .ticket
                    .is_directory
                    .then(|| directory_tree_snapshot(&path))
                    .transpose()?;
                Ok(archive::ZipSource {
                    path,
                    identity: source.identity,
                    directory_tree_fingerprint,
                })
            })
            .collect()
    }

    fn zip_source(&self, ticket_id: &str) -> Result<archive::ZipSource, String> {
        self.ensure_live()?;
        let source = self
            .inner
            .transfers
            .lock()
            .map_err(|_| "transfer_state_unavailable")?
            .sources
            .get(ticket_id)
            .cloned()
            .ok_or("transfer_reference_expired")?;
        if source.ticket.is_directory {
            return Err("zip_archive_must_be_file".into());
        }
        let path = validate_transfer_source_path(Path::new(&source.ticket.path))?;
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
        {
            return Err("zip_archive_required".into());
        }
        if file_namespace_fingerprint(&path)? != source.identity {
            return Err("transfer_source_changed".into());
        }
        Ok(archive::ZipSource {
            path,
            identity: source.identity,
            directory_tree_fingerprint: None,
        })
    }

    #[cfg_attr(target_os = "linux", allow(dead_code))]
    pub(crate) fn native_drag_out_paths(
        &self,
        ticket_ids: &[String],
    ) -> Result<Vec<PathBuf>, String> {
        self.ensure_live()?;
        if ticket_ids.is_empty() || ticket_ids.len() > EXTERNAL_PATH_LIMIT {
            return Err("transfer_selection_limit_256".into());
        }
        let unique: HashSet<_> = ticket_ids.iter().collect();
        if unique.len() != ticket_ids.len() {
            return Err("transfer_selection_invalid".into());
        }
        let sources = {
            let state = self
                .inner
                .transfers
                .lock()
                .map_err(|_| "transfer_state_unavailable")?;
            ticket_ids
                .iter()
                .map(|id| {
                    state
                        .sources
                        .get(id)
                        .cloned()
                        .ok_or("transfer_reference_expired")
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut paths = Vec::with_capacity(sources.len());
        for source in sources {
            let path = validate_transfer_source_path(Path::new(&source.ticket.path))?;
            if file_namespace_fingerprint(&path)? != source.identity {
                return Err("transfer_source_changed".into());
            }
            #[cfg(windows)]
            ensure_transfer_source_content_ready(&path)?;
            VerifiedDirectory::open_existing(path.parent().ok_or("transfer_source_invalid")?)
                .map_err(|error| error.to_string())?;
            paths.push(path);
        }
        Ok(paths)
    }

    pub(crate) fn preview_transfer(
        &self,
        request: TransferPreviewRequest,
    ) -> Result<TransferPreview, String> {
        self.ensure_live()?;
        if matches!(
            request.action,
            TransferAction::CreateDirectory | TransferAction::CreateFile
        ) {
            let is_directory = request.action == TransferAction::CreateDirectory;
            let invalid_request = if is_directory {
                "transfer_create_directory_request_invalid"
            } else {
                "transfer_create_file_request_invalid"
            };
            if !request.ticket_ids.is_empty()
                || request.new_name.is_none()
                || !request.new_names.is_empty()
                || request.destination_entry.is_some()
                || !request.keep_both.is_empty()
                || !request.skip.is_empty()
            {
                return Err(invalid_request.into());
            }
            let directory = self.resolve_transfer_destination(&request)?;
            let name = request.new_name.as_deref().ok_or(invalid_request)?;
            let operation = if is_directory {
                create_directory_operation(&directory, name)?
            } else {
                create_file_operation(&directory, name)?
            };
            let ticket_id = if is_directory {
                "create-directory"
            } else {
                "create-file"
            };
            let target = Path::new(&operation.target_path);
            if path_entry_exists(target)? {
                return Ok(TransferPreview {
                    id: new_job_id("transfer-preview"),
                    destination_directory: normalize_path(&directory),
                    rows: vec![TransferRow {
                        ticket_id: ticket_id.into(),
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
                    ticket_id: ticket_id.into(),
                    preview: Some(preview_dto(&operation)),
                    error: None,
                    skipped: false,
                }],
            });
        }
        if request.action == TransferAction::Rename {
            let single_name = request.new_names.is_empty()
                && request.ticket_ids.len() == 1
                && request.new_name.is_some();
            let batch_names = request.new_name.is_none()
                && request.new_names.len() == request.ticket_ids.len()
                && request
                    .ticket_ids
                    .iter()
                    .all(|id| request.new_names.contains_key(id));
            if (!single_name && !batch_names)
                || request.destination_entry.is_some()
                || !request.keep_both.is_empty()
                || (!batch_names && !request.skip.is_empty())
            {
                return Err("transfer_rename_selection_invalid".into());
            }
        }
        if request.action == TransferAction::Trash
            && (request.new_name.is_some()
                || !request.new_names.is_empty()
                || request.destination_entry.is_some()
                || !request.keep_both.is_empty()
                || !request.skip.is_empty())
        {
            return Err("transfer_trash_request_invalid".into());
        }
        if request.action == TransferAction::SystemTrash
            && (request.new_name.is_some()
                || !request.new_names.is_empty()
                || request.destination_entry.is_some()
                || !request.keep_both.is_empty()
                || !request.skip.is_empty())
        {
            return Err("transfer_trash_request_invalid".into());
        }
        if !matches!(
            request.action,
            TransferAction::Rename | TransferAction::Trash | TransferAction::SystemTrash
        ) && (request.new_name.is_some() || !request.new_names.is_empty())
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
        let mut checked_trees = HashMap::<PathBuf, Result<String, String>>::new();
        let mut trees = Vec::new();
        for source in sources.iter().filter_map(|(_, source)| source.as_ref()) {
            if let Some(guard) = &source.tree_guard {
                if !checked_trees.contains_key(&guard.root) {
                    let validation = validate_tree_guard(guard).map_err(|error| {
                        if error == "symlink_or_reparse_point" {
                            error
                        } else {
                            "transfer_source_tree_changed".into()
                        }
                    });
                    checked_trees.insert(guard.root.clone(), validation);
                    trees.push(guard.clone());
                }
            }
        }
        let mut operations = Vec::new();
        let mut rows = Vec::new();
        let mut reserved = HashSet::new();
        // Windows streaming regular-file transfers need at least these known
        // bytes. Directories/metadata and concurrent quota changes remain
        // checked by actual writes. Do not apply logical-size rejection to
        // macOS clone/sparse copies, which need not allocate that payload.
        #[cfg(windows)]
        let space = crate::file_workspace::integration::storage::inspect(&directory, false);
        #[cfg(windows)]
        let destination_volume = crate::fs_safety::capture_source_object_identity(&directory)
            .ok()
            .and_then(|identity| identity.platform_volume_id);
        #[cfg(windows)]
        let mut reserved_copy_bytes = 0u64;
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
                if let Some(Err(error)) = source
                    .tree_guard
                    .as_ref()
                    .and_then(|guard| checked_trees.get(&guard.root))
                {
                    return Err(error.clone());
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
                        .new_names
                        .get(&id)
                        .cloned()
                        .or_else(|| request.new_name.clone())
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
                    && matches!(
                        request.action,
                        TransferAction::Copy | TransferAction::Move | TransferAction::Duplicate
                    )
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
                if !matches!(
                    request.action,
                    TransferAction::Trash | TransferAction::SystemTrash
                ) && conflict(&name, &reserved)?
                {
                    return Err("transfer_target_exists".into());
                }
                let operation_directory = if matches!(
                    request.action,
                    TransferAction::Rename | TransferAction::Trash | TransferAction::SystemTrash
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
                let target = if matches!(
                    request.action,
                    TransferAction::Trash | TransferAction::SystemTrash
                ) {
                    source_path.to_path_buf()
                } else {
                    target_for_name(&name)?
                };
                let validated_tree_fingerprint = source
                    .tree_guard
                    .as_ref()
                    .and_then(|guard| checked_trees.get(&guard.root))
                    .and_then(|result| result.as_ref().ok())
                    .map(String::as_str);
                #[cfg(windows)]
                let required_bytes = {
                    let copies_payload = matches!(
                        request.action,
                        TransferAction::Copy | TransferAction::Duplicate
                    ) || request.action == TransferAction::Move
                        && source.identity.platform_volume_id != destination_volume;
                    let bytes = if copies_payload && !source.ticket.is_directory {
                        source.identity.size
                    } else {
                        0
                    };
                    let required = reserved_copy_bytes
                        .checked_add(bytes)
                        .ok_or("storage_insufficient_space")?;
                    crate::file_workspace::integration::storage::check_capacity(&space, required)?;
                    required
                };
                // The guard above has already enumerated this exact source
                // tree. Reuse that verified fingerprint to bind the new
                // operation ID instead of repeating the full walk.
                let operation = make_operation_with_tree_fingerprint(
                    &source.file_id,
                    source_path,
                    operation_directory,
                    &target,
                    request.action,
                    validated_tree_fingerprint,
                )?;
                let preview = preview_dto(&operation);
                #[cfg(windows)]
                {
                    reserved_copy_bytes = required_bytes;
                }
                if matches!(
                    request.action,
                    TransferAction::Copy | TransferAction::Move | TransferAction::Duplicate
                ) {
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
    #[cfg(all(test, any(target_os = "windows", target_os = "macos")))]
    pub(crate) fn take_transfer(
        &self,
        request: TransferExecuteRequest,
    ) -> Result<ExecuteMovesRequest, String> {
        self.take_transfer_with_sources(request)
            .map(|(execution, _)| execution)
    }

    pub(crate) fn take_transfer_with_sources(
        &self,
        request: TransferExecuteRequest,
    ) -> Result<(ExecuteMovesRequest, Vec<Source>), String> {
        self.ensure_live()?;
        if !request.confirmed {
            return Err("transfer_confirmation_required".into());
        }
        let (plan, selection_sources) = {
            let mut state = self
                .inner
                .transfers
                .lock()
                .map_err(|_| "transfer_state_unavailable")?;
            let plan = match state.previews.remove(&request.preview_id) {
                Some(plan) => plan,
                None => {
                    #[cfg(test)]
                    eprintln!(
                        "transfer stale at take: preview missing id={}",
                        request.preview_id
                    );
                    return Err("operation_preview_stale".into());
                }
            };
            let selection_sources = plan
                .ticket_ids
                .iter()
                .filter_map(|ticket_id| state.sources.get(ticket_id).cloned())
                .filter(|source| source.browse_selection.is_some())
                .collect::<Vec<_>>();
            (plan, selection_sources)
        };
        let mut validated_trees = HashMap::<String, String>::new();
        for guard in &plan.trees {
            validated_trees.insert(normalize_path(&guard.root), validate_tree_guard(guard)?);
        }
        for operation in &plan.operations {
            let tree_fingerprint = validated_trees
                .get(&operation.source_path)
                .map(String::as_str);
            // `validate_tree_guard` just recomputed and matched the full tree
            // snapshot; pass that proof into operation revalidation so this
            // confirmation boundary scans the directory only once.
            let revalidation = revalidate_with_tree_fingerprint(operation, tree_fingerprint);
            if let Err(_error) = &revalidation {
                #[cfg(test)]
                eprintln!(
                    "transfer stale at take: revalidation error={_error} op={} source={} target={}",
                    operation.id, operation.source_path, operation.target_path
                );
            }
            revalidation?;
        }
        Ok((
            ExecuteMovesRequest {
                operations: plan.operations,
            },
            selection_sources,
        ))
    }

    fn resolve_transfer_destination(
        &self,
        request: &TransferPreviewRequest,
    ) -> Result<PathBuf, String> {
        if request.destination_entry.is_some() && request.destination_browse.is_some() {
            return Err("transfer_destination_invalid".into());
        }
        if let Some(destination) = &request.destination_browse {
            let directory = self
                .inner
                .browse
                .resolve_watch_target(&destination.session_id, &destination.path_ref)
                .map_err(|error| error.to_string())?;
            let bound = directory
                .as_path()
                .canonicalize()
                .map_err(|error| error.to_string())?;
            let requested = PathBuf::from(&request.destination_directory)
                .canonicalize()
                .map_err(|error| error.to_string())?;
            if bound != requested {
                return Err("transfer_destination_changed".into());
            }
        }
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
        crate::file_workspace::integration::storage::check_destination(&directory, 0)?;
        Ok(directory)
    }
}

fn action_name(action: TransferAction) -> &'static str {
    match action {
        TransferAction::Copy => "copy",
        TransferAction::Move => "move",
        TransferAction::Duplicate => "duplicate",
        TransferAction::Rename => "rename",
        TransferAction::Trash => "move_to_trash",
        TransferAction::SystemTrash => "move_to_system_trash",
        TransferAction::CreateDirectory => "create_directory",
        TransferAction::CreateFile => "create_file",
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
    validated_tree_fingerprint: Option<&str>,
) -> Result<(String, DirectoryIdentity), String> {
    validate_safe_file_name(
        target
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("transfer_target_invalid")?,
    )?;
    let source = if matches!(
        action,
        TransferAction::Rename | TransferAction::Trash | TransferAction::SystemTrash
    ) {
        validate_namespace_entry_path(source)?
    } else {
        validate_transfer_source_path(source)?
    };
    #[cfg(windows)]
    if matches!(
        action,
        TransferAction::Copy | TransferAction::Move | TransferAction::Duplicate
    ) {
        ensure_transfer_source_content_ready(&source)?;
    }
    let source_is_directory = source.is_dir();
    ensure_not_safe_trash_operation(&source)?;
    // Directory operations stay on the shared atomic-move path. The
    // platform-specific commit layer decides whether the verified volume
    // identities permit an atomic rename or require a staged copy/verify move.
    let tree_fingerprint = if source_is_directory
        && matches!(
            action,
            TransferAction::Copy | TransferAction::Move | TransferAction::Duplicate
        ) {
        Some(match validated_tree_fingerprint {
            Some(fingerprint) => fingerprint.to_string(),
            None => directory_tree_snapshot(&source)?,
        })
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
    if matches!(
        action,
        TransferAction::Copy | TransferAction::Move | TransferAction::Duplicate
    ) && source_is_directory
        && canonical_root.starts_with(&source)
    {
        return Err("transfer_directory_into_itself".into());
    }
    let nearest_identity = if matches!(
        action,
        TransferAction::Rename | TransferAction::Trash | TransferAction::SystemTrash
    ) {
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
    // A bounded plan may use the selected folder as the verified root while
    // moving a regular file into a child folder. Keep actual same-directory
    // moves and directory self-moves rejected.
    if action == TransferAction::Move
        && source.parent() == Some(canonical_root.as_path())
        && (source_is_directory || target.parent() == Some(destination_root))
    {
        return Err("transfer_move_same_directory".into());
    }
    let target_path_identity = match action {
        TransferAction::Trash => "safe-trash-target".to_string(),
        TransferAction::SystemTrash => "system-trash-target".to_string(),
        _ => normalize_path(target),
    };
    let digest = blake3::hash(
        format!(
            "transfer:v5:{file_id}:{}:{}:{action:?}:{identity:?}:{tree_fingerprint:?}:{}:{:?}",
            normalize_path(&source),
            target_path_identity,
            normalize_path(&canonical_root),
            destination.identity(),
        )
        .as_bytes(),
    );
    let destination_path = normalize_path(&canonical_root);
    let tree_fingerprint = tree_fingerprint.as_deref().unwrap_or("none");
    Ok((
        format!(
            "op-transfer-v5-{}-{}-{}",
            digest.to_hex(),
            tree_fingerprint,
            encode_path_for_transfer_id(&destination_path)
        ),
        nearest_identity,
    ))
}

#[cfg(windows)]
pub(crate) fn ensure_transfer_source_content_ready(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if crate::platform::windows::cloud_metadata::path_needs_download(path, &metadata) {
        return Err("cloud_file_requires_download".into());
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test, feature = "native-qa"))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn make_operation(
    file_id: &str,
    source: &Path,
    directory: &Path,
    target: &Path,
    action: TransferAction,
) -> Result<OperationPreviewRequest, String> {
    make_operation_with_tree_fingerprint(file_id, source, directory, target, action, None)
}

fn make_operation_with_tree_fingerprint(
    file_id: &str,
    source: &Path,
    directory: &Path,
    target: &Path,
    action: TransferAction,
    validated_tree_fingerprint: Option<&str>,
) -> Result<OperationPreviewRequest, String> {
    let (id, _) = snapshot(
        file_id,
        source,
        directory,
        target,
        action,
        validated_tree_fingerprint,
    )?;
    let target_path = match action {
        TransferAction::Trash => safe_trash_operation_target(source, &id),
        TransferAction::SystemTrash => system_trash_operation_target(),
        _ => normalize_path(target),
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
    revalidate_with_tree_fingerprint(operation, None)
}

fn revalidate_with_tree_fingerprint(
    operation: &OperationPreviewRequest,
    validated_tree_fingerprint: Option<&str>,
) -> Result<DirectoryIdentity, String> {
    if matches!(
        operation.operation_type.as_str(),
        "create_directory" | "create_file"
    ) {
        let current = if operation.operation_type == "create_directory" {
            create_directory_operation(Path::new(&operation.source_path), &operation.new_name)?
        } else {
            create_file_operation(Path::new(&operation.source_path), &operation.new_name)?
        };
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
        "duplicate" => TransferAction::Duplicate,
        "rename" => TransferAction::Rename,
        "move_to_trash" => TransferAction::Trash,
        "move_to_system_trash" => TransferAction::SystemTrash,
        _ => return Err("operation_preview_stale".into()),
    };
    let destination_root = decode_transfer_root(&operation.id).inspect_err(|_| {
        #[cfg(test)]
        eprintln!(
            "transfer stale at take: destination root decode failed op={}",
            operation.id
        );
    })?;
    let source = Path::new(&operation.source_path);
    if action == TransferAction::Trash
        && operation.target_path != safe_trash_operation_target(source, &operation.id)
        || action == TransferAction::SystemTrash
            && operation.target_path != system_trash_operation_target()
    {
        return Err("operation_preview_stale".into());
    }
    let snapshot_target = if matches!(action, TransferAction::Trash | TransferAction::SystemTrash) {
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
        validated_tree_fingerprint,
    )?;
    if id != operation.id {
        #[cfg(test)]
        eprintln!(
            "transfer stale at take: id changed old={} new={}",
            operation.id, id
        );
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
    let (version, payload) = if let Some(payload) = id.strip_prefix("op-transfer-v5-") {
        (5, payload)
    } else if let Some(payload) = id.strip_prefix("op-transfer-v4-") {
        (4, payload)
    } else {
        return Err("operation_preview_stale".into());
    };
    let (digest, payload) = payload.split_once('-').ok_or("operation_preview_stale")?;
    let encoded_path = if version == 5 {
        // Older v5 IDs represented a missing directory fingerprint as "-",
        // which creates an ambiguous triple separator before the hex path.
        // Keep those transient previews readable while using an unambiguous
        // non-hex sentinel for newly generated file previews.
        if let Some(encoded_path) = payload.strip_prefix("--") {
            encoded_path
        } else {
            payload
                .split_once('-')
                .map(|(_, encoded_path)| encoded_path)
                .ok_or("operation_preview_stale")?
        }
    } else {
        payload
    };
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

pub(crate) fn transfer_directory_tree_fingerprint(id: &str) -> Option<&str> {
    let marker = id.rfind("op-transfer-v5-")?;
    let payload = id.get(marker + "op-transfer-v5-".len()..)?;
    let (digest, payload) = payload.split_once('-')?;
    let (fingerprint, encoded_path) = payload.split_once('-')?;
    (digest.len() == 64
        && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        && fingerprint.len() == 64
        && fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit())
        && !encoded_path.is_empty()
        && encoded_path.bytes().all(|byte| byte.is_ascii_hexdigit()))
    .then_some(fingerprint)
}

const DIRECTORY_TREE_JOURNAL_PREFIX: &str = "directory-tree-v1:";

pub(crate) fn encode_directory_tree_journal_fingerprint(fingerprint: &str) -> Option<String> {
    if fingerprint.len() == 64 && fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Some(format!("{DIRECTORY_TREE_JOURNAL_PREFIX}{fingerprint}"))
    } else {
        None
    }
}

pub(crate) fn directory_tree_journal_fingerprint(value: Option<&str>) -> Option<&str> {
    let fingerprint = value?.strip_prefix(DIRECTORY_TREE_JOURNAL_PREFIX)?;
    (fingerprint.len() == 64 && fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then_some(fingerprint)
}

pub(crate) fn execute(
    operation: &OperationPreviewRequest,
    context: OperationExecutionContext<'_>,
) -> Result<FileOperationResult, FileMutationError> {
    #[cfg(target_os = "macos")]
    let directory_copy_identity =
        if matches!(operation.operation_type.as_str(), "copy" | "duplicate")
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
    if operation.operation_type == "create_file" {
        return create_empty_file_with_safety(
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
    if operation.operation_type == "move_to_system_trash" {
        return move_to_system_trash_with_safety(
            operation.source_path.clone(),
            context.app_data_dir,
            context.expected_identity,
            context.cancel_flag,
            context.phase_observer,
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
    if matches!(operation.operation_type.as_str(), "copy" | "duplicate") {
        #[cfg(target_os = "macos")]
        let expected_identity = directory_copy_identity
            .as_ref()
            .or(context.expected_identity);
        #[cfg(not(target_os = "macos"))]
        let expected_identity = context.expected_identity;
        crate::fs_safety::atomic_move::atomic_copy_with_verified_destination(
            &source,
            &target,
            &operation.operation_type,
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

fn create_file_operation(directory: &Path, name: &str) -> Result<OperationPreviewRequest, String> {
    validate_safe_file_name(name)?;
    let parent = VerifiedDirectory::open_existing(directory).map_err(|error| error.to_string())?;
    ensure_general_file_operation_allowed(parent.path())?;
    ensure_not_safe_trash_operation(parent.path())?;
    let target = parent.path().join(name);
    ensure_not_safe_trash_operation(&target)?;
    let digest = blake3::hash(
        format!(
            "create-file:v1:{}:{:?}:{name}",
            normalize_path(parent.path()),
            parent.identity()
        )
        .as_bytes(),
    );
    Ok(OperationPreviewRequest {
        id: format!("op-transfer-{}", digest.to_hex()),
        file_id: super::RECOVERY_ACTION_FILE_ID.into(),
        operation_type: "create_file".into(),
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

fn system_trash_operation_target() -> String {
    if cfg!(target_os = "windows") {
        "System Recycle Bin".into()
    } else {
        "System Trash".into()
    }
}

fn basket_entry_status(
    path: &Path,
    is_directory: bool,
    expected: &FileIdentityFingerprint,
) -> String {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.is_dir() != is_directory || metadata.file_type().is_symlink() {
                return "changed".into();
            }
            match file_namespace_fingerprint(path) {
                Ok(current) if current == *expected => "available".into(),
                _ => "changed".into(),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut ancestor = path.parent();
            while let Some(candidate) = ancestor {
                if let Ok(metadata) = std::fs::symlink_metadata(candidate) {
                    if metadata.is_dir() {
                        if let Ok(identity) =
                            crate::fs_safety::capture_source_object_identity(candidate)
                        {
                            return if identity.platform_volume_id != expected.platform_volume_id {
                                "offline".into()
                            } else {
                                "unavailable".into()
                            };
                        }
                    }
                }
                ancestor = candidate.parent();
            }
            "offline".into()
        }
        Err(_) => "unavailable".into(),
    }
}

fn basket_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
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
        #[cfg(windows)]
        ensure_transfer_source_content_ready(&candidate)?;
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
        browse_selection: None,
    })
}

pub(crate) fn complete_browse_selection_batch(
    runtime: &FileWorkspaceRuntime,
    sources: &[Source],
    logs: &[OperationLogDto],
) {
    let mut groups = HashMap::<(String, String), Vec<BrowseSelectionCompletion>>::new();
    for source in sources {
        let Some(origin) = &source.browse_selection else {
            continue;
        };
        let successful_removal = logs.iter().any(|log| {
            transfer_target_key(Path::new(&log.path_before))
                == transfer_target_key(Path::new(&source.ticket.path))
                && log.status == "success"
                && matches!(
                    log.operation_type.as_str(),
                    "move" | "move_to_trash" | "move_to_system_trash"
                )
        });
        groups
            .entry((origin.session_id.clone(), origin.snapshot_id.clone()))
            .or_default()
            .push(BrowseSelectionCompletion {
                path: origin.path.clone(),
                identity: origin.identity.clone(),
                removed: successful_removal,
            });
    }
    for ((session_id, snapshot_id), completed) in groups {
        // The next page call performs the same full identity check and reports
        // a stale snapshot if an outside change raced with this journal batch.
        let _ = runtime.inner.browse.complete_selection_snapshot_batch(
            &session_id,
            &snapshot_id,
            &completed,
        );
    }
}

pub(crate) fn directory_tree_snapshot(root: &Path) -> Result<String, String> {
    let root_identity = crate::fs_safety::capture_source_object_identity(root)
        .map_err(|error| error.to_string())?;
    if !root.is_dir() {
        return Err("transfer_source_invalid".into());
    }
    let root_namespace = file_namespace_fingerprint(root)?;
    let mut snapshot = TransferTreeSnapshot::new();
    snapshot.push(
        format!(".:directory:{root_identity:?}:{root_namespace:?}"),
        0,
    )?;
    let mut entry_count = 0usize;
    collect_directory_tree(
        root,
        root,
        0,
        &mut entry_count,
        &mut snapshot,
        root_namespace.platform_volume_id.as_deref(),
    )?;
    Ok(snapshot.finish())
}

struct TransferTreeSnapshot {
    records: Vec<String>,
    unordered: Option<crate::fs_safety::identity::OrderIndependentDigest>,
    name_budget: crate::fs_safety::identity::DirectoryEntryNameBudget,
    reserved_record_bytes: usize,
}

impl TransferTreeSnapshot {
    fn new() -> Self {
        Self {
            records: Vec::new(),
            unordered: None,
            name_budget: crate::fs_safety::identity::DirectoryEntryNameBudget::default(),
            reserved_record_bytes: 0,
        }
    }

    fn push(&mut self, record: String, entry_count: usize) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        if self.unordered.is_none() && entry_count > EXTERNAL_TREE_ENTRY_LIMIT {
            let mut unordered = crate::fs_safety::identity::OrderIndependentDigest::default();
            for record in std::mem::take(&mut self.records) {
                update_transfer_snapshot_digest(&mut unordered, &record);
            }
            self.name_budget.release(self.reserved_record_bytes);
            self.reserved_record_bytes = 0;
            self.unordered = Some(unordered);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = entry_count;

        if let Some(unordered) = &mut self.unordered {
            update_transfer_snapshot_digest(unordered, &record);
        } else {
            let reserved = self
                .name_budget
                .reserve(record.len())
                .ok_or_else(|| "transfer_directory_memory_budget_exceeded".to_string())?;
            if self.records.try_reserve(1).is_err() {
                self.name_budget.release(reserved);
                return Err("transfer_directory_memory_allocation_failed".into());
            }
            self.reserved_record_bytes = self.reserved_record_bytes.saturating_add(reserved);
            self.records.push(record);
        }
        Ok(())
    }

    fn finish(mut self) -> String {
        if let Some(unordered) = self.unordered {
            unordered.finalize_hex(b"transfer-tree-v2\0")
        } else {
            self.records.sort();
            let mut hasher = blake3::Hasher::new();
            for record in self.records {
                hasher.update(record.as_bytes());
                hasher.update(&[0]);
            }
            hasher.finalize().to_hex().to_string()
        }
    }
}

fn update_transfer_snapshot_digest(
    digest: &mut crate::fs_safety::identity::OrderIndependentDigest,
    record: &str,
) {
    digest.update_bytes(record.as_bytes());
    digest.update_bytes(&[0]);
}

fn collect_directory_tree(
    root: &Path,
    directory: &Path,
    depth: usize,
    entry_count: &mut usize,
    snapshot: &mut TransferTreeSnapshot,
    root_volume: Option<&str>,
) -> Result<(), String> {
    if depth >= EXTERNAL_TREE_DEPTH_LIMIT {
        return Err("transfer_directory_depth_limit".into());
    }
    for entry in std::fs::read_dir(directory).map_err(|error| error.to_string())? {
        *entry_count = (*entry_count).saturating_add(1);
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let physical_identity = crate::fs_safety::capture_source_object_identity(&path)
            .map_err(|error| error.to_string())?;
        let identity = file_namespace_fingerprint(&path)?;
        if root_volume.is_some() && identity.platform_volume_id.as_deref() != root_volume {
            return Err("transfer_nested_volume_unsupported".into());
        }
        ensure_general_file_operation_allowed(&path)?;
        let relative_path = path
            .strip_prefix(root)
            .map_err(|_| "transfer_source_invalid")?
            .to_path_buf();
        let relative_display = utf8_relative_path(&relative_path)?;
        if path.is_dir() {
            snapshot.push(
                format!("{relative_display}:directory:{physical_identity:?}:{identity:?}"),
                *entry_count,
            )?;
            collect_directory_tree(root, &path, depth + 1, entry_count, snapshot, root_volume)?;
        } else if path.is_file() {
            validate_source_path(&path)?;
            snapshot.push(
                format!("{relative_display}:file:{physical_identity:?}:{identity:?}"),
                *entry_count,
            )?;
        } else {
            return Err("transfer_source_invalid".into());
        }
    }
    Ok(())
}

fn validate_tree_guard(guard: &TreeGuard) -> Result<String, String> {
    let fingerprint = directory_tree_snapshot(&guard.root)?;
    if fingerprint == guard.fingerprint {
        Ok(fingerprint)
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
        let (request, selection_sources) = runtime.take_transfer_with_sources(request)?;
        let paths = request
            .operations
            .iter()
            .flat_map(|op| [op.source_path.clone(), op.target_path.clone()])
            .collect::<Vec<_>>();
        let db = &runtime.inner.database;
        let emitter = TauriOperationProgressEmitter::new(app.clone());
        let result = reconciliation::with_reconciliation_wake(&app, db, &paths, || {
            execute_moves_with_persistence_with_progress_and_app_data(
                db,
                request,
                cancel_flag,
                &emitter,
                app_data_dir,
                None,
            )
        })?;
        complete_browse_selection_batch(&runtime, &selection_sources, &result.logs);
        Ok(result)
    })
    .await
    .map_err(|e| format!("transfer_execution_failed:{e}"))?
}

pub(crate) async fn create_zip_registered<R: Runtime>(
    app: AppHandle<R>,
    runtime: FileWorkspaceRuntime,
    cancel: &OperationCancellationToken,
    request: ZipCreateRequest,
) -> Result<ExecuteMovesResult, String> {
    let guard = cancel.begin()?;
    let cancel_flag = Arc::clone(&cancel.cancel);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let sources = runtime.zip_sources(&request.ticket_ids)?;
        let output = validate_target_path(Path::new(&request.output_path))?;
        if !output
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
        {
            return Err("zip_output_extension_required".into());
        }
        let output_path = normalize_path(&output);
        let source_path = normalize_path(&sources[0].path);
        let source_names = sources
            .iter()
            .map(|source| {
                source
                    .path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("item")
            })
            .collect::<Vec<_>>();
        let source_summary = if source_names.len() == 1 {
            source_names[0].to_string()
        } else {
            format!("{} ({} items)", source_names[0], source_names.len())
        };
        let batch_id = new_job_id("operation-batch");
        let created_at = current_timestamp_ms().to_string();
        let mut log = make_archive_operation_log(
            &batch_id,
            &created_at,
            "zip_create",
            source_path,
            output_path.clone(),
            source_summary,
            Path::new(&output_path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("archive.zip")
                .to_string(),
        );
        apply_source_fingerprint(&mut log, &sources[0].identity);
        let db = &runtime.inner.database;
        db.save_operation_logs(&batch_id, std::slice::from_ref(&log))
            .map_err(|error| format!("zip_history_prepare_failed:{error}"))?;
        let paths = sources
            .iter()
            .map(|source| normalize_path(&source.path))
            .chain(std::iter::once(output_path.clone()))
            .collect::<Vec<_>>();
        let emitter = TauriOperationProgressEmitter::new(app.clone());
        emit_zip_progress(
            &emitter,
            &batch_id,
            archive::ZipProgress {
                phase: "preparing".into(),
                ..Default::default()
            },
        );
        let mut last_progress_at = std::time::Instant::now();
        let result = reconciliation::with_reconciliation_wake(&app, db, &paths, || {
            archive::create_zip(
                &sources,
                &output,
                request.compression_level,
                &cancel_flag,
                |progress| {
                    if zip_progress_is_due(&progress, last_progress_at) {
                        emit_zip_progress(&emitter, &batch_id, progress);
                        last_progress_at = std::time::Instant::now();
                    }
                },
            )
        });
        match result {
            Ok(output) => {
                emit_terminal_zip_progress(&emitter, &batch_id, "completed");
                if let Ok(output_identity) = file_namespace_fingerprint(&output) {
                    apply_target_fingerprint(&mut log, &output_identity);
                }
                log.status = "success".into();
                log.operation_phase = "completed".into();
                log.path_after = normalize_path(&output);
                log.target_path = log.path_after.clone();
                log.error_message = None;
                db.save_operation_logs(&batch_id, std::slice::from_ref(&log))
                    .map_err(|error| {
                        format!("zip_history_finalize_failed:{}:{error}", log.path_after)
                    })?;
                Ok(ExecuteMovesResult {
                    logs: vec![log],
                    batch_id,
                })
            }
            Err(error) => {
                emit_terminal_zip_progress(&emitter, &batch_id, "failed");
                log.status = "failed".into();
                log.operation_phase = if error == "transfer_cancelled" {
                    "rolled_back".into()
                } else {
                    "completed".into()
                };
                log.error_message = Some(error.clone());
                db.save_operation_logs(&batch_id, std::slice::from_ref(&log))
                    .map_err(|save_error| {
                        format!("{error}; zip_history_finalize_failed:{save_error}")
                    })?;
                Err(error)
            }
        }
    })
    .await
    .map_err(|error| format!("zip_create_task_failed:{error}"))?
}

pub(crate) async fn extract_zip_registered<R: Runtime>(
    app: AppHandle<R>,
    runtime: FileWorkspaceRuntime,
    cancel: &OperationCancellationToken,
    request: ZipExtractRequest,
) -> Result<ExecuteMovesResult, String> {
    let guard = cancel.begin()?;
    let cancel_flag = Arc::clone(&cancel.cancel);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let source = runtime.zip_source(&request.ticket_id)?;
        let same_name_target = matches!(request.target, ZipExtractTarget::SameNameFolder);
        let destination = match request.target {
            ZipExtractTarget::Here => archive::validate_destination_directory(
                source.path.parent().ok_or("zip_archive_parent_invalid")?,
            )?,
            ZipExtractTarget::SameNameFolder => archive::same_name_extract_path(&source.path)?,
            ZipExtractTarget::ChosenDirectory => {
                archive::validate_destination_directory(Path::new(
                    request
                        .destination_directory
                        .as_deref()
                        .ok_or("zip_destination_directory_required")?,
                ))?
            }
        };
        let source_path = normalize_path(&source.path);
        let destination_path = normalize_path(&destination);
        let name = source
            .path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("zip_archive_name_invalid")?
            .to_string();
        let batch_id = new_job_id("operation-batch");
        let created_at = current_timestamp_ms().to_string();
        let mut log = make_archive_operation_log(
            &batch_id,
            &created_at,
            "zip_extract",
            source_path.clone(),
            destination_path.clone(),
            name,
            destination
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("folder")
                .to_string(),
        );
        apply_source_fingerprint(&mut log, &source.identity);
        let db = &runtime.inner.database;
        db.save_operation_logs(&batch_id, std::slice::from_ref(&log))
            .map_err(|error| format!("zip_history_prepare_failed:{error}"))?;
        let paths = vec![source_path, destination_path.clone()];
        let emitter = TauriOperationProgressEmitter::new(app.clone());
        emit_zip_progress(
            &emitter,
            &batch_id,
            archive::ZipProgress {
                phase: "preparing".into(),
                ..Default::default()
            },
        );
        let mut last_progress_at = std::time::Instant::now();
        let mut same_name_directory_created = false;
        let result = reconciliation::with_reconciliation_wake(&app, db, &paths, || {
            if same_name_target {
                same_name_directory_created = archive::ensure_extract_destination(&destination)?;
                archive::validate_destination_directory(&destination)?;
            }
            archive::extract_zip(
                &source.path,
                &source.identity,
                &destination,
                request.conflict_policy,
                &cancel_flag,
                |progress| {
                    if zip_progress_is_due(&progress, last_progress_at) {
                        emit_zip_progress(&emitter, &batch_id, progress);
                        last_progress_at = std::time::Instant::now();
                    }
                },
            )
        });
        let result = match result {
            Err(error)
                if same_name_directory_created && !error.starts_with("zip_extract_partial:") =>
            {
                Err(format!(
                    "zip_extract_partial:destination_folder_created:{error}"
                ))
            }
            result => result,
        };
        match result {
            Ok(_) => {
                emit_terminal_zip_progress(&emitter, &batch_id, "completed");
                log.status = "success".into();
                log.operation_phase = "completed".into();
                log.error_message = None;
                db.save_operation_logs(&batch_id, std::slice::from_ref(&log))
                    .map_err(|error| {
                        format!("zip_history_finalize_failed:{}:{error}", log.path_after)
                    })?;
                Ok(ExecuteMovesResult {
                    logs: vec![log],
                    batch_id,
                })
            }
            Err(error) => {
                emit_terminal_zip_progress(&emitter, &batch_id, "failed");
                log.status = "failed".into();
                log.operation_phase = if error.starts_with("zip_extract_partial:") {
                    "manual_review".into()
                } else {
                    "rolled_back".into()
                };
                log.error_message = Some(error.clone());
                db.save_operation_logs(&batch_id, std::slice::from_ref(&log))
                    .map_err(|save_error| {
                        format!("{error}; zip_history_finalize_failed:{save_error}")
                    })?;
                Err(error)
            }
        }
    })
    .await
    .map_err(|error| format!("zip_extract_task_failed:{error}"))?
}

fn make_archive_operation_log(
    batch_id: &str,
    created_at: &str,
    operation_type: &str,
    source_path: String,
    target_path: String,
    old_name: String,
    new_name: String,
) -> OperationLogDto {
    let operation = OperationPreviewRequest {
        id: new_job_id(operation_type),
        file_id: super::RECOVERY_ACTION_FILE_ID.into(),
        operation_type: operation_type.into(),
        source_path,
        target_path: target_path.clone(),
        old_name: old_name.clone(),
        new_name,
        is_executable: Some(true),
    };
    let mut log = make_operation_log(
        batch_id,
        created_at,
        0,
        &operation,
        "pending",
        None,
        target_path,
    );
    log.can_undo = false;
    log.can_restore = false;
    log.restore_status = "not_restored".into();
    log.operation_phase = "prepared".into();
    log
}

fn emit_zip_progress(
    emitter: &impl OperationProgressEmitter,
    batch_id: &str,
    progress: archive::ZipProgress,
) {
    emitter.emit_progress(OperationProgressPayload {
        kind: "execute".into(),
        batch_id: batch_id.to_string(),
        processed: progress.processed_bytes,
        total: progress.total_bytes,
        current_path: progress.current_path,
        phase: Some(progress.phase),
        processed_items: Some(progress.processed_items),
        total_items: Some(progress.total_items),
        processed_bytes: Some(progress.processed_bytes),
        total_bytes: Some(progress.total_bytes),
    });
}

fn zip_progress_is_due(progress: &archive::ZipProgress, last_emit: std::time::Instant) -> bool {
    progress.processed_items >= progress.total_items
        || last_emit.elapsed() >= std::time::Duration::from_millis(200)
}

fn emit_terminal_zip_progress(
    emitter: &impl OperationProgressEmitter,
    batch_id: &str,
    phase: &str,
) {
    emit_zip_progress(
        emitter,
        batch_id,
        archive::ZipProgress {
            phase: phase.to_string(),
            ..Default::default()
        },
    );
}

#[cfg(test)]
mod transfer_root_tests {
    use super::{decode_transfer_root, encode_path_for_transfer_id};

    #[test]
    fn transfer_root_decoder_accepts_file_and_directory_v5_ids() {
        let root = std::env::temp_dir().join("fylura-transfer-root");
        let encoded = encode_path_for_transfer_id(root.to_str().expect("UTF-8 test path"));
        let digest = "a".repeat(64);
        let fingerprint = "b".repeat(64);

        for id in [
            format!("op-transfer-v5-{digest}-none-{encoded}"),
            // File previews created by the earlier v5 writer used "-" as
            // the missing-fingerprint sentinel, yielding three separators.
            format!("op-transfer-v5-{digest}---{encoded}"),
            format!("op-transfer-v5-{digest}-{fingerprint}-{encoded}"),
        ] {
            assert_eq!(decode_transfer_root(&id).unwrap(), root);
        }
    }
}
