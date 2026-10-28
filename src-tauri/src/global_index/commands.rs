use super::coordinator::{sqlite_contention_kind, GlobalIndexCoordinator, GlobalIndexError};
use super::models::*;
use crate::db::{Database, DbError};
use crate::window_auth::require_main_window;
use std::fs;
#[cfg(not(target_os = "macos"))]
use std::process::Command;
use tauri::{Runtime, State, WebviewWindow};

#[tauri::command]
pub fn search_global_entries(
    db: State<'_, Database>,
    coordinator: State<'_, GlobalIndexCoordinator>,
    request: GlobalSearchRequest,
) -> Result<GlobalSearchResponse, String> {
    let (request_id, normalized_query, offset) = validate_global_search_request(&request)?;
    let snapshot = db
        .search_global_entries_snapshot(&normalized_query, request.limit, offset)
        .map_err(global_index_database_error)?;
    let coordinator_status_result = coordinator.status();
    let coordinator_database_busy = coordinator_status_result
        .as_ref()
        .err()
        .is_some_and(|error| sqlite_contention_kind(error).is_some());
    let coordinator_status = coordinator_status_result.as_ref().ok();
    let coordinator_conflict = coordinator_status
        .as_ref()
        .map(|status| !same_global_index_facts(status, &snapshot.index_status))
        .unwrap_or(true);
    let mut index_status = snapshot.index_status;
    if let Some(status) = coordinator_status.filter(|status| {
        matches!(
            status.status.as_str(),
            INDEX_STATUS_DATABASE_BUSY | INDEX_STATUS_SERVICE_BUSY
        )
    }) {
        index_status.status = status.status.clone();
        index_status.last_error = status.last_error.clone();
        index_status.collection_complete = false;
    } else if coordinator_database_busy {
        index_status.status = INDEX_STATUS_DATABASE_BUSY.to_string();
        index_status.last_error = Some("global_index_database_busy".to_string());
        index_status.collection_complete = false;
    }
    index_status.provider_status = coordinator.provider_status().ok();
    let has_enabled_sources = snapshot.source_health.iter().any(|source| source.enabled);
    let collection_complete = index_status.collection_complete && !coordinator_conflict;
    let result_state = global_search_result_state(
        snapshot.results.is_empty(),
        collection_complete,
        has_enabled_sources,
        &index_status,
        coordinator_conflict,
    );
    Ok(GlobalSearchResponse {
        version: 2,
        request_id,
        normalized_query,
        results: snapshot.results,
        index_status,
        collection_complete,
        result_state: result_state.to_string(),
        source_revision: snapshot.source_revision,
        source_health: snapshot.source_health,
    })
}

fn validate_global_search_request(
    request: &GlobalSearchRequest,
) -> Result<(String, String, u32), String> {
    if request.version != 2 {
        return Err("global_search_contract_version_unsupported".to_string());
    }
    let request_id = request.request_id.trim();
    if request_id.is_empty() || request_id.len() > 128 {
        return Err("global_search_request_id_invalid".to_string());
    }
    let normalized_query = request.query.trim().to_string();
    if normalized_query.chars().count() > 512 {
        return Err("global_search_query_too_long".to_string());
    }
    let offset = request
        .cursor
        .as_deref()
        .map(parse_search_cursor)
        .transpose()?
        .unwrap_or(request.offset);
    Ok((request_id.to_string(), normalized_query, offset))
}

fn same_global_index_facts(left: &GlobalIndexStatus, right: &GlobalIndexStatus) -> bool {
    left.enabled == right.enabled
        && left.status == right.status
        && left.processed_entries == right.processed_entries
        && left.collection_complete == right.collection_complete
        && left.total_entries == right.total_entries
        && left.indexed_volumes == right.indexed_volumes
        && left.ready_volumes == right.ready_volumes
        && left.pending_volumes == right.pending_volumes
        && left.last_sync_at == right.last_sync_at
        && left.last_error == right.last_error
}

fn global_search_result_state(
    results_empty: bool,
    collection_complete: bool,
    has_enabled_sources: bool,
    index_status: &GlobalIndexStatus,
    coordinator_conflict: bool,
) -> &'static str {
    if results_empty && !has_enabled_sources {
        return "no_source";
    }
    if coordinator_conflict {
        return if results_empty { "pending" } else { "partial" };
    }
    if results_empty {
        if collection_complete {
            "empty"
        } else if matches!(
            index_status.status.as_str(),
            INDEX_STATUS_ERROR
                | INDEX_STATUS_DATABASE_BUSY
                | INDEX_STATUS_SERVICE_BUSY
                | INDEX_STATUS_UNAVAILABLE
                | INDEX_STATUS_PERMISSION_REQUIRED
                | INDEX_STATUS_SPOTLIGHT_UNAVAILABLE
                | INDEX_STATUS_SPOTLIGHT_NOT_INDEXED
                | INDEX_STATUS_SPOTLIGHT_EXTERNAL_NOT_INDEXED
                | INDEX_STATUS_FSEVENTS_UNAVAILABLE
        ) {
            "failed"
        } else {
            "pending"
        }
    } else if collection_complete {
        "complete"
    } else {
        "partial"
    }
}

#[tauri::command]
pub fn get_global_index_status(
    coordinator: State<'_, GlobalIndexCoordinator>,
) -> Result<GlobalIndexStatus, String> {
    load_global_index_status(coordinator.inner())
}

/// Presents one row per mount path, newest first.
///
/// A cloud mount (e.g. Google Drive for desktop) can re-register under a fresh
/// volume GUID when it reconnects, leaving an older row behind for the same
/// drive letter. The historical row stays in the database untouched — nothing
/// is deleted, merged or re-linked — and duplicates are harmless for coverage
/// because they are never authorized or enabled. Pure so the non-destructive
/// presentation contract is testable without Tauri state.
pub(crate) fn newest_row_per_mount_path(mut volumes: Vec<GlobalVolume>) -> Vec<GlobalVolume> {
    volumes.sort_by_key(|volume| std::cmp::Reverse(volume.updated_at));
    let mut seen_mount_paths = std::collections::HashSet::new();
    volumes.retain(|volume| seen_mount_paths.insert(volume.mount_path.clone()));
    volumes
}

#[tauri::command]
pub fn list_global_index_sources(
    db: State<'_, Database>,
) -> Result<Vec<GlobalIndexSource>, String> {
    let volumes = db
        .list_global_volumes()
        .map_err(global_index_database_error)?;
    Ok(newest_row_per_mount_path(volumes)
        .into_iter()
        .map(|volume| GlobalIndexSource {
            can_pause: volume.index_status == INDEX_STATUS_INDEXING
                || volume.index_status == INDEX_STATUS_SYNCING,
            can_rebuild: true,
            technical_detail: Some(GlobalIndexTechnicalDetail {
                journal_id: volume.journal_id.clone(),
                journal_cursor: volume.journal_cursor.clone(),
                provider: volume.provider.clone(),
                filesystem_type: volume.filesystem_type.clone(),
            }),
            volume,
        })
        .collect())
}

#[tauri::command]
pub fn start_global_index<R: Runtime>(
    window: WebviewWindow<R>,
    coordinator: State<'_, GlobalIndexCoordinator>,
) -> Result<(), String> {
    require_main_window(&window)?;
    coordinator.start().map_err(global_index_error_message)
}

#[tauri::command]
pub fn pause_global_index<R: Runtime>(
    window: WebviewWindow<R>,
    coordinator: State<'_, GlobalIndexCoordinator>,
) -> Result<(), String> {
    require_main_window(&window)?;
    coordinator.pause().map_err(global_index_error_message)
}

#[tauri::command]
pub fn resume_global_index<R: Runtime>(
    window: WebviewWindow<R>,
    coordinator: State<'_, GlobalIndexCoordinator>,
) -> Result<(), String> {
    require_main_window(&window)?;
    coordinator.resume().map_err(global_index_error_message)
}

#[tauri::command]
pub fn rebuild_global_index_source<R: Runtime>(
    window: WebviewWindow<R>,
    coordinator: State<'_, GlobalIndexCoordinator>,
    source_id: Option<String>,
) -> Result<(), String> {
    require_main_window(&window)?;
    coordinator
        .rebuild(source_id)
        .map_err(global_index_error_message)
}

#[tauri::command]
pub fn set_global_index_source_enabled<R: Runtime>(
    window: WebviewWindow<R>,
    coordinator: State<'_, GlobalIndexCoordinator>,
    source_id: String,
    enabled: bool,
) -> Result<(), String> {
    require_main_window(&window)?;
    coordinator
        .set_source_enabled(&source_id, enabled)
        .map_err(global_index_error_message)
}

#[tauri::command]
pub fn authorize_global_index_source<R: Runtime>(
    window: WebviewWindow<R>,
    coordinator: State<'_, GlobalIndexCoordinator>,
    source_id: String,
    confirmed: bool,
) -> Result<(), String> {
    require_main_window(&window)?;
    coordinator
        .authorize_source(&source_id, confirmed)
        .map_err(global_index_error_message)
}

#[tauri::command]
pub fn open_global_search_result(db: State<'_, Database>, entry_id: String) -> Result<(), String> {
    let entry = revalidated_global_entry(db.inner(), &entry_id)?;
    open_path(&entry.path)
}

#[tauri::command]
pub fn reveal_global_search_result(
    db: State<'_, Database>,
    entry_id: String,
) -> Result<(), String> {
    let entry = revalidated_global_entry(db.inner(), &entry_id)?;
    crate::file_ops::reveal_in_folder(entry.path)
}

#[tauri::command]
pub fn list_managed_scopes(db: State<'_, Database>) -> Result<Vec<ManagedScope>, String> {
    db.list_managed_scopes()
        .map_err(global_index_database_error)
}

#[tauri::command]
pub fn add_managed_scope<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: AddManagedScopeRequest,
) -> Result<ManagedScope, String> {
    require_main_window(&window)?;
    db.add_managed_scope(request)
        .map_err(global_index_database_error)
}

#[tauri::command]
pub fn remove_managed_scope<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    id: String,
) -> Result<bool, String> {
    require_main_window(&window)?;
    db.remove_managed_scope(&id)
        .map_err(global_index_database_error)
}

#[tauri::command]
pub fn update_managed_scope_policy<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: UpdateManagedScopePolicyRequest,
) -> Result<ManagedScope, String> {
    require_main_window(&window)?;
    db.update_managed_scope_policy(request)
        .map_err(global_index_database_error)
}

#[tauri::command]
pub fn get_ai_management_status(db: State<'_, Database>) -> Result<AiManagementStatus, String> {
    db.ai_management_status()
        .map_err(global_index_database_error)
}

fn global_index_database_error(error: DbError) -> String {
    global_index_error_message(GlobalIndexError::Database(error))
}

pub(crate) fn global_index_error_message(error: GlobalIndexError) -> String {
    if sqlite_contention_kind(&error).is_some() {
        "global_index_database_busy".to_string()
    } else if matches!(&error, GlobalIndexError::ServiceBusy(_)) {
        "global_index_service_busy".to_string()
    } else if matches!(
        &error,
        GlobalIndexError::PermissionRequired(_) | GlobalIndexError::WindowsIo(5 | 1314)
    ) {
        "global_index_permission_required".to_string()
    } else {
        error.to_string()
    }
}

fn load_global_index_status(
    coordinator: &GlobalIndexCoordinator,
) -> Result<GlobalIndexStatus, String> {
    let mut status = coordinator.status().map_err(global_index_error_message)?;
    status.provider_status = coordinator.provider_status().ok();
    // Coverage is complete only when every enabled source is ready. Degraded,
    // paused, permission, rebuild and unavailable states remain explicitly
    // incomplete even if some rows are searchable.
    status.collection_complete = status.status == INDEX_STATUS_READY;
    Ok(status)
}

fn parse_search_cursor(cursor: &str) -> Result<u32, String> {
    let cursor = cursor.trim();
    if cursor.is_empty() || cursor.len() > 16 {
        return Err("global_search_cursor_invalid".to_string());
    }
    cursor
        .parse::<u32>()
        .map_err(|_| "global_search_cursor_invalid".to_string())
}

pub(crate) fn revalidated_global_entry(
    db: &Database,
    entry_id: &str,
) -> Result<GlobalEntry, String> {
    if entry_id.trim().is_empty() || entry_id.len() > 256 {
        return Err("global_search_entry_id_invalid".to_string());
    }
    let entry = db
        .get_global_entry(entry_id)
        .map_err(global_index_database_error)?
        .filter(|entry| !entry.is_stale)
        .ok_or_else(|| "global_search_result_not_found".to_string())?;
    let volume = db
        .get_global_volume(&entry.volume_id)
        .map_err(global_index_database_error)?
        .ok_or_else(|| "global_search_source_not_found".to_string())?;
    if !volume.enabled {
        return Err("global_search_source_disabled".to_string());
    }
    if !trusted_global_provider(&entry.source_provider)
        || !trusted_global_provider(&volume.provider)
        || entry.source_provider != volume.provider
    {
        return Err("global_search_provider_untrusted".to_string());
    }
    if !path_is_same_or_child(&entry.path, &volume.mount_path) {
        return Err("global_search_path_outside_source".to_string());
    }
    let metadata = fs::symlink_metadata(&entry.path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => "global_search_result_missing".to_string(),
        _ => "global_search_result_unreadable".to_string(),
    })?;
    if metadata.file_type().is_symlink() {
        return Err("global_search_result_identity_changed".to_string());
    }
    if metadata.is_dir() != entry.is_directory {
        return Err("global_search_result_kind_changed".to_string());
    }
    if !live_entry_identity_matches(&entry, &metadata) {
        return Err("global_search_result_identity_changed".to_string());
    }
    Ok(entry)
}

fn trusted_global_provider(provider: &str) -> bool {
    matches!(
        provider,
        PROVIDER_WINDOWS_MFT_USN
            | PROVIDER_WINDOWS_RECURSIVE_FALLBACK
            | PROVIDER_MACOS_SPOTLIGHT
            | PROVIDER_MACOS_FSEVENTS_RECONCILE
            | PROVIDER_RECURSIVE_FALLBACK
    )
}

fn path_is_same_or_child(path: &str, root: &str) -> bool {
    let path = normalize_path(path).trim_end_matches('/').to_string();
    let root = normalize_path(root).trim_end_matches('/').to_string();
    !root.is_empty() && (path == root || path.starts_with(&format!("{root}/")))
}

fn live_entry_identity_matches(entry: &GlobalEntry, metadata: &fs::Metadata) -> bool {
    if let Some(path_identity) = entry.platform_file_id.strip_prefix("path:") {
        return path_identity == normalize_path(&entry.path);
    }
    live_native_file_id(&entry.path, metadata)
        .is_some_and(|identity| identity == entry.platform_file_id)
}

#[cfg(target_os = "macos")]
fn live_native_file_id(_path: &str, metadata: &fs::Metadata) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    Some(format!(
        "mac:dev:{:x}:ino:{:x}",
        metadata.dev(),
        metadata.ino()
    ))
}

#[cfg(windows)]
fn live_native_file_id(path: &str, _metadata: &fs::Metadata) -> Option<String> {
    use std::fs::OpenOptions;
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT,
    };
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return None;
    }
    let file_id = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Some(format!("{file_id:016x}"))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn live_native_file_id(_path: &str, _metadata: &fs::Metadata) -> Option<String> {
    None
}

fn open_path(path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("global_search_path_empty".to_string());
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer.exe")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("global_search_open_failed: {error}"))
    }
    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::finder::open_path(std::path::Path::new(path))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("global_search_open_failed: {error}"))
    }
    #[cfg(not(any(windows, unix)))]
    {
        Err("global_search_open_unsupported".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(id: &str, mount_path: &str, updated_at: i64) -> GlobalVolume {
        let mut volume = crate::global_index::tests::test_volume();
        volume.id = id.to_string();
        volume.stable_volume_id = format!(r"\\?\Volume{{{id}}}\");
        volume.mount_path = mount_path.to_string();
        volume.updated_at = updated_at;
        volume.enabled = false;
        volume.scope_authorization_state = SCOPE_AUTHORIZATION_NOT_AUTHORIZED.into();
        volume.provider = PROVIDER_WINDOWS_UNSUPPORTED.into();
        volume.entry_count = 0;
        volume
    }

    #[test]
    fn duplicate_rows_for_one_mount_path_present_only_the_newest() {
        // Google Drive for desktop re-registers under a fresh volume GUID when
        // it reconnects, leaving two rows for G:\ with different stable ids.
        let rows = newest_row_per_mount_path(vec![
            volume("42ff7b96-d1ab-11f1", "G:\\", 1_760_000_000),
            volume("f8cd1153-d296-11f1", "G:\\", 1_760_000_900),
        ]);

        assert_eq!(rows.len(), 1, "one card per mount path");
        assert_eq!(rows[0].stable_volume_id, r"\\?\Volume{f8cd1153-d296-11f1}\");
        assert!(
            rows[0].updated_at > 1_760_000_000,
            "the newest identity wins so a stale row cannot mask a reconnect"
        );
    }

    #[test]
    fn dedupe_keeps_every_other_mount_and_orders_by_recency() {
        let rows = newest_row_per_mount_path(vec![
            volume("a", "G:\\", 10),
            volume("b", "C:\\", 30),
            volume("c", "G:\\", 20),
            volume("d", "Z:\\", 5),
        ]);

        let mounts: Vec<&str> = rows.iter().map(|row| row.mount_path.as_str()).collect();
        assert_eq!(mounts, vec!["C:\\", "G:\\", "Z:\\"]);
        assert_eq!(rows[1].stable_volume_id, r"\\?\Volume{c}\");
    }

    #[test]
    fn reused_letter_does_not_hide_active_or_actionable_history() {
        for state in ["enabled", "permission", "legacy", "entries"] {
            let mut old = volume("old", "G:\\", 1);
            match state {
                "enabled" => old.enabled = true,
                "permission" => old.index_status = INDEX_STATUS_PERMISSION_REQUIRED.into(),
                "legacy" => {
                    old.scope_authorization_state =
                        SCOPE_AUTHORIZATION_LEGACY_CONFIRMATION_REQUIRED.into()
                }
                _ => old.entry_count = 1,
            }
            let rows = newest_row_per_mount_path(vec![old, volume("new", "G:\\", 2)]);
            assert_eq!(rows.len(), 2, "must retain actionable history: {state}");
        }
    }

    #[test]
    fn sqlite_and_service_busy_use_distinct_stable_error_codes() {
        let database_busy =
            GlobalIndexError::Database(DbError::Sqlite(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                None,
            )));
        let remote_database_busy = GlobalIndexError::DatabaseBusy("database busy".into());
        let service_busy = GlobalIndexError::ServiceBusy("database is locked".into());

        assert_eq!(
            global_index_error_message(database_busy),
            "global_index_database_busy"
        );
        assert_eq!(
            global_index_error_message(remote_database_busy),
            "global_index_database_busy"
        );
        assert_eq!(
            global_index_error_message(service_busy),
            "global_index_service_busy",
            "a service error containing SQLite-like words stays a service error"
        );
    }

    #[test]
    fn windows_access_denied_codes_surface_as_permission_required_without_losing_ipc_code() {
        for code in [5, 1314] {
            assert_eq!(
                global_index_error_message(GlobalIndexError::WindowsIo(code)),
                "global_index_permission_required"
            );
        }
        assert_eq!(
            global_index_error_message(GlobalIndexError::PermissionRequired(
                "native access denied".into()
            )),
            "global_index_permission_required"
        );
    }

    #[test]
    fn m1_r3_owned_result_revalidates_for_open_and_reveal_without_launching() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp-tests")
            .join(format!(
                "m1-r3-activation-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
        fs::create_dir_all(root.join("fixtures")).unwrap();
        let fixture = root.join("fixtures/owned.txt");
        fs::write(&fixture, b"owned fixture").unwrap();
        let db = Database::open(root.join("index.sqlite3")).unwrap();
        let mut volume = crate::global_index::tests::test_volume();
        volume.mount_path = root.join("fixtures").to_string_lossy().into_owned();
        volume.provider = PROVIDER_RECURSIVE_FALLBACK.into();
        volume.index_status = INDEX_STATUS_READY.into();
        db.upsert_global_volume(&volume).unwrap();
        db.upsert_global_entries_batch(&[GlobalEntryInput::from_path(
            &volume.id,
            &fixture,
            PROVIDER_RECURSIVE_FALLBACK,
        )])
        .unwrap();
        let result = db
            .search_global_entries_snapshot("owned.txt", 10, 0)
            .unwrap()
            .results
            .remove(0);
        // Both activation commands share this check; do not open an OS app in CI.
        assert_eq!(
            revalidated_global_entry(&db, &result.id).unwrap().path,
            fixture.to_string_lossy()
        );
        db.set_global_volume_enabled(&volume.id, false).unwrap();
        assert_eq!(
            revalidated_global_entry(&db, &result.id).unwrap_err(),
            "global_search_source_disabled"
        );
        db.set_global_volume_enabled(&volume.id, true).unwrap();
        fs::remove_file(&fixture).unwrap();
        assert_eq!(
            revalidated_global_entry(&db, &result.id).unwrap_err(),
            "global_search_result_missing"
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }

    fn request(request_id: &str, query: &str) -> GlobalSearchRequest {
        GlobalSearchRequest {
            version: 2,
            request_id: request_id.to_string(),
            query: query.to_string(),
            limit: 80,
            offset: 0,
            cursor: None,
        }
    }

    #[test]
    fn v2_request_validation_echoes_identity_and_normalizes_query() {
        let (request_id, query, offset) =
            validate_global_search_request(&request("request-7", "  报告  "))
                .expect("valid request");
        assert_eq!(request_id, "request-7");
        assert_eq!(query, "报告");
        assert_eq!(offset, 0);

        let mut unsupported = request("request-8", "report");
        unsupported.version = 1;
        assert_eq!(
            validate_global_search_request(&unsupported),
            Err("global_search_contract_version_unsupported".to_string())
        );
    }

    #[test]
    fn result_state_exposes_collection_completeness() {
        let ready = GlobalIndexStatus {
            platform: "test".to_string(),
            enabled: true,
            status: INDEX_STATUS_READY.to_string(),
            provider_status: None,
            processed_entries: 1,
            collection_complete: true,
            total_entries: 1,
            indexed_volumes: 1,
            ready_volumes: 1,
            pending_volumes: 0,
            last_sync_at: None,
            last_error: None,
        };
        assert_eq!(
            global_search_result_state(true, false, true, &ready, false),
            "pending"
        );
        for busy_status in [INDEX_STATUS_DATABASE_BUSY, INDEX_STATUS_SERVICE_BUSY] {
            assert_eq!(
                global_search_result_state(
                    true,
                    false,
                    true,
                    &GlobalIndexStatus {
                        enabled: true,
                        status: busy_status.to_string(),
                        collection_complete: false,
                        ..ready.clone()
                    },
                    false
                ),
                "failed",
                "busy indexes must not be reported as zero-match searches"
            );
        }
        assert_eq!(
            global_search_result_state(
                true,
                false,
                true,
                &GlobalIndexStatus {
                    enabled: true,
                    status: "authorization_required".to_string(),
                    indexed_volumes: 1,
                    pending_volumes: 1,
                    ..ready.clone()
                },
                false
            ),
            "pending",
            "an unconfirmed legacy source must not turn an empty cache into a zero-result claim"
        );
        assert_eq!(
            global_search_result_state(false, false, true, &ready, false),
            "partial"
        );
        assert_eq!(
            global_search_result_state(true, true, true, &ready, false),
            "empty"
        );
        assert_eq!(
            global_search_result_state(false, true, true, &ready, false),
            "complete"
        );
        assert_eq!(
            global_search_result_state(true, false, false, &ready, false),
            "no_source"
        );
        assert_eq!(
            global_search_result_state(
                true,
                false,
                true,
                &GlobalIndexStatus {
                    status: INDEX_STATUS_ERROR.to_string(),
                    ..ready.clone()
                },
                false
            ),
            "failed"
        );
        assert_eq!(
            global_search_result_state(false, true, true, &ready, true),
            "partial"
        );
    }

    #[test]
    fn open_revalidation_helpers_reject_untrusted_provider_and_path_escape() {
        assert!(trusted_global_provider(PROVIDER_RECURSIVE_FALLBACK));
        assert!(!trusted_global_provider("renderer-cache"));
        assert!(path_is_same_or_child("/volume/reports/a.txt", "/volume"));
        assert!(!path_is_same_or_child("/volume-escape/a.txt", "/volume"));
    }
    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn global_search_parent_browse_reuses_admission_and_rejects_stale_sources() {
        use crate::file_workspace::integration::{types::BrowseOpenRequest, FileWorkspaceRuntime};
        use crate::file_workspace::WorkspacePlatform;
        use crate::platform::macos::quick_look::MacThumbnailService;
        let root = std::env::temp_dir().join(format!("u1-s1-{}", uuid::Uuid::new_v4()));
        let parent = root.join("fixtures");
        fs::create_dir_all(parent.join("child-folder")).unwrap();
        fs::write(parent.join("target.txt"), b"owned fixture").unwrap();
        let db = Database::open(root.join("index.sqlite3")).unwrap();
        let mut volume = crate::global_index::tests::test_volume();
        volume.mount_path = parent.to_string_lossy().into_owned();
        volume.provider = PROVIDER_RECURSIVE_FALLBACK.into();
        db.upsert_global_volume(&volume).unwrap();
        let runtime = FileWorkspaceRuntime::new(
            db.clone(),
            MacThumbnailService::new(root.join("legacy")),
            root.join("cache"),
        )
        .unwrap();
        let platform = if cfg!(windows) {
            WorkspacePlatform::Windows
        } else {
            WorkspacePlatform::Macos
        };
        for name in ["target.txt", "child-folder"] {
            let path = parent.join(name);
            let mut input =
                GlobalEntryInput::from_path(&volume.id, &path, PROVIDER_RECURSIVE_FALLBACK);
            // Native platform IDs exercise exact-object focus; path fallback
            // deliberately exercises parent-only routing on other platforms.
            let native_id = live_native_file_id(&input.path, &fs::symlink_metadata(&path).unwrap());
            if let Some(id) = &native_id {
                input.platform_file_id = id.clone();
            }
            db.upsert_global_entries_batch(&[input.clone()]).unwrap();
            let result = db
                .search_global_entries_snapshot(name, 10, 0)
                .unwrap()
                .results
                .remove(0);
            let request = || BrowseOpenRequest {
                platform,
                routing_hint: "renderer-forged-path".into(),
                display_hint: Some("forged".into()),
                global_entry_id: Some(result.id.clone()),
            };
            let opened = runtime.open_browse(request()).unwrap();
            let page = opened.initial_page.as_ref().unwrap();
            assert!(page.entries.iter().any(|entry| entry.name == name));
            if native_id.is_some() {
                let focused = opened.focus_entry_ref.as_ref().unwrap();
                let resolved = runtime.inner.browse.resolve_entry(focused).unwrap();
                assert_eq!(resolved.path, path);
            } else {
                assert!(opened.focus_entry_ref.is_none());
            }
            assert_ne!(opened.location.display_name, "forged");
            assert_ne!(
                opened.session_id,
                runtime.open_browse(request()).unwrap().session_id
            );
            db.set_global_volume_enabled(&volume.id, false).unwrap();
            assert_eq!(
                runtime.open_browse(request()).unwrap_err(),
                "global_search_source_disabled"
            );
            db.set_global_volume_enabled(&volume.id, true).unwrap();
            db.mark_global_entry_stale(&result.id).unwrap();
            assert_eq!(
                runtime.open_browse(request()).unwrap_err(),
                "global_search_result_not_found"
            );
            db.upsert_global_entries_batch(&[input]).unwrap();
            fs::rename(&path, parent.join(format!("retired-{name}"))).unwrap();
            assert_eq!(
                runtime.open_browse(request()).unwrap_err(),
                "global_search_result_missing"
            );
            if name.ends_with(".txt") {
                fs::write(&path, b"replacement").unwrap();
                if native_id.is_some() {
                    assert_eq!(
                        runtime.open_browse(request()).unwrap_err(),
                        "global_search_result_identity_changed"
                    );
                } else {
                    assert!(runtime
                        .open_browse(request())
                        .unwrap()
                        .focus_entry_ref
                        .is_none());
                }
            }
        }
        runtime.dispose();
        drop(runtime);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
