use super::{
    runtime::FileWorkspaceRuntime,
    runtime_owner::FileWorkspaceRuntimeOwner,
    types::{
        encode_preview_asset_ipc_response, encode_thumbnail_ipc_response, BrowseCancelRequest,
        BrowseNextPageRequest, BrowseOpenRequest, BrowseReleasePageRequest,
        BrowseReleasePathRequest, BrowseRestoreRequest, BrowseRetainPathRequest,
        BrowseSessionRequest, BrowseStartEnumerationRequest, ChangePendingRequest,
        ChangeRefreshRequest, ChangeStartRequest, LocationBrowseRequest, PreviewAssetRequestDto,
        PreviewCreateRequest, PreviewSessionRequest, PreviewSwitchSourceRequest,
        ReadEligibilityRequest, ThumbnailCancelRequest, ThumbnailRequestDto,
    },
};
use crate::window_auth::require_main_window;
use tauri::{Runtime, State, WebviewWindow};

#[cfg(target_os = "macos")]
use crate::platform::macos::native_preview::NativePreviewAttachError;

#[cfg(target_os = "macos")]
fn attach_native_preview<R: Runtime>(
    window: &WebviewWindow<R>,
    runtime: &FileWorkspaceRuntime,
    snapshot: &super::types::PreviewSnapshotDto,
    presentation: Option<&super::types::PreviewNativePresentation>,
) -> Result<super::types::PreviewSnapshotDto, String> {
    if let Some(presentation) = presentation {
        if let Err(error) = runtime.inner.native_preview_host.attach(
            window,
            std::sync::Arc::clone(&runtime.inner.native_preview_access),
            snapshot,
            presentation,
        ) {
            return match error {
                NativePreviewAttachError::Presentation(error) => {
                    runtime.native_preview_attach_failed(snapshot, presentation, error)
                }
                error => Err(error.into_message()),
            };
        }
    }
    Ok(snapshot.clone())
}

#[cfg(not(target_os = "macos"))]
fn attach_native_preview<R: Runtime>(
    _window: &WebviewWindow<R>,
    _runtime: &FileWorkspaceRuntime,
    _snapshot: &super::types::PreviewSnapshotDto,
    _presentation: Option<&super::types::PreviewNativePresentation>,
) -> Result<super::types::PreviewSnapshotDto, String> {
    Ok(_snapshot.clone())
}

/// Every blocking integration operation crosses this boundary. The async
/// command remains available to sibling cancellation commands while the
/// filesystem, database, watcher join, or shared service wait is in flight.
async fn spawn_runtime<T, F>(
    runtime: FileWorkspaceRuntime,
    operation: &'static str,
    work: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&FileWorkspaceRuntime) -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || work(&runtime))
        .await
        .map_err(|error| format!("{operation}_task_failed:{error}"))?
}

#[tauri::command]
pub async fn file_workspace_browse_open<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: BrowseOpenRequest,
) -> Result<super::types::BrowseOpenResponse, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    spawn_runtime(runtime.clone(), "workspace_browse_open", move |runtime| {
        runtime.open_browse(request)
    })
    .await
}

#[tauri::command]
pub async fn file_workspace_browse_restore<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: BrowseRestoreRequest,
) -> Result<super::types::BrowseOpenResponse, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    spawn_runtime(
        runtime.clone(),
        "workspace_browse_restore",
        move |runtime| runtime.restore_browse(request),
    )
    .await
}

#[tauri::command]
pub async fn file_workspace_location_browse<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: LocationBrowseRequest,
) -> Result<super::types::BrowseOpenResponse, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    spawn_runtime(
        runtime.clone(),
        "workspace_location_browse",
        move |runtime| runtime.browse_location(request),
    )
    .await
}

#[tauri::command]
pub async fn file_workspace_browse_start_enumeration<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: BrowseStartEnumerationRequest,
) -> Result<super::types::BrowsePageDto, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    spawn_runtime(
        runtime.clone(),
        "workspace_browse_start_enumeration",
        move |runtime| runtime.start_enumeration(request),
    )
    .await
}

#[tauri::command]
pub async fn file_workspace_browse_next_page<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: BrowseNextPageRequest,
) -> Result<super::types::BrowsePageDto, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    spawn_runtime(
        runtime.clone(),
        "workspace_browse_next_page",
        move |runtime| runtime.next_page(request),
    )
    .await
}

/// This operation only flips BrowseService's cancellation token and therefore
/// stays on the async command lane. It can run while a sibling spawn_blocking
/// page request is still reading/enumerating.
#[tauri::command]
pub async fn file_workspace_browse_cancel_enumeration<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: BrowseCancelRequest,
) -> Result<(), String> {
    require_main_window(&window)?;
    if let Some(runtime) = runtime.current_if_initialized_for_window(&window)? {
        runtime.cancel_enumeration(request)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn file_workspace_browse_release_page<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: BrowseReleasePageRequest,
) -> Result<(), String> {
    require_main_window(&window)?;
    let Some(runtime) = runtime.current_if_initialized_for_window(&window)? else {
        return Ok(());
    };
    spawn_runtime(
        runtime.clone(),
        "workspace_browse_release_page",
        move |runtime| runtime.release_page(request),
    )
    .await
}

#[tauri::command]
pub async fn file_workspace_browse_release_path<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: BrowseReleasePathRequest,
) -> Result<(), String> {
    require_main_window(&window)?;
    let Some(runtime) = runtime.current_if_initialized_for_window(&window)? else {
        return Ok(());
    };
    spawn_runtime(
        runtime.clone(),
        "workspace_browse_release_path",
        move |runtime| runtime.release_path(request),
    )
    .await
}

#[tauri::command]
pub async fn file_workspace_browse_retain_path<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: BrowseRetainPathRequest,
) -> Result<(), String> {
    require_main_window(&window)?;
    let Some(runtime) = runtime.current_if_initialized_for_window(&window)? else {
        return Ok(());
    };
    spawn_runtime(
        runtime.clone(),
        "workspace_browse_retain_path",
        move |runtime| runtime.retain_path(request),
    )
    .await
}

#[tauri::command]
pub async fn file_workspace_browse_dispose<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: BrowseSessionRequest,
) -> Result<(), String> {
    require_main_window(&window)?;
    let Some(runtime) = runtime.current_if_initialized_for_window(&window)? else {
        return Ok(());
    };
    spawn_runtime(
        runtime.clone(),
        "workspace_browse_dispose",
        move |runtime| runtime.dispose_browse(request),
    )
    .await
}

#[tauri::command]
pub async fn file_workspace_location_list<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
) -> Result<Vec<crate::file_workspace::LocationDescriptor>, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    spawn_runtime(runtime.clone(), "workspace_location_list", |runtime| {
        runtime.list_locations()
    })
    .await
}

#[tauri::command]
pub async fn file_workspace_change_start<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: ChangeStartRequest,
) -> Result<super::types::ChangeStartResponse, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    spawn_runtime(runtime.clone(), "workspace_change_start", move |runtime| {
        runtime.start_change_monitor(request)
    })
    .await
}

#[tauri::command]
pub async fn file_workspace_change_pending<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: ChangePendingRequest,
) -> Result<Option<super::types::ChangePendingResponse>, String> {
    require_main_window(&window)?;
    match runtime.current_if_initialized_for_window(&window)? {
        Some(runtime) => runtime.pending_change(request),
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn file_workspace_change_refresh<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: ChangeRefreshRequest,
) -> Result<super::types::BrowsePageDto, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    spawn_runtime(
        runtime.clone(),
        "workspace_change_refresh",
        move |runtime| runtime.refresh_change(request),
    )
    .await
}

#[tauri::command]
pub async fn file_workspace_change_dispose<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: ChangePendingRequest,
) -> Result<(), String> {
    require_main_window(&window)?;
    let Some(runtime) = runtime.current_if_initialized_for_window(&window)? else {
        return Ok(());
    };
    spawn_runtime(
        runtime.clone(),
        "workspace_change_dispose",
        move |runtime| runtime.dispose_change_monitor(request),
    )
    .await
}

#[tauri::command]
pub async fn file_workspace_read_eligibility<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: ReadEligibilityRequest,
) -> Result<super::types::ReadEligibilityResponse, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    spawn_runtime(
        runtime.clone(),
        "workspace_read_eligibility",
        move |runtime| runtime.read_eligibility(request),
    )
    .await
}

#[tauri::command]
pub async fn file_workspace_thumbnail_request<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: ThumbnailRequestDto,
) -> Result<tauri::ipc::Response, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    let payload = spawn_runtime(
        runtime.clone(),
        "workspace_thumbnail_request",
        move |runtime| {
            let artifact = runtime.request_thumbnail(request)?;
            encode_thumbnail_ipc_response(&artifact)
        },
    )
    .await?;
    Ok(tauri::ipc::Response::new(payload))
}

/// Cancellation is deliberately a short, direct async command so it can run
/// while the sibling thumbnail request is waiting in spawn_blocking.
#[tauri::command]
pub async fn file_workspace_thumbnail_cancel<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: ThumbnailCancelRequest,
) -> Result<bool, String> {
    require_main_window(&window)?;
    match runtime.current_if_initialized_for_window(&window)? {
        Some(runtime) => runtime.cancel_thumbnail(request),
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn file_workspace_preview_create<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: PreviewCreateRequest,
) -> Result<super::types::PreviewSnapshotDto, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    spawn_runtime(
        runtime.clone(),
        "workspace_preview_create",
        move |runtime| runtime.create_preview(request),
    )
    .await
}

#[tauri::command]
pub async fn file_workspace_preview_snapshot<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: PreviewSessionRequest,
) -> Result<super::types::PreviewSnapshotDto, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    let presentation = request.native_presentation.clone();
    let snapshot = runtime.snapshot_preview(request)?;
    attach_native_preview(&window, &runtime, &snapshot, presentation.as_ref())
}

#[tauri::command]
pub async fn file_workspace_preview_start<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: PreviewSessionRequest,
) -> Result<super::types::PreviewSnapshotDto, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    let presentation = request.native_presentation.clone();
    let snapshot = spawn_runtime(runtime.clone(), "workspace_preview_start", move |runtime| {
        runtime.start_preview(request)
    })
    .await?;
    attach_native_preview(&window, &runtime, &snapshot, presentation.as_ref())
}

#[tauri::command]
pub async fn file_workspace_preview_cancel<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: PreviewSessionRequest,
) -> Result<bool, String> {
    require_main_window(&window)?;
    match runtime.current_if_initialized_for_window(&window)? {
        Some(runtime) => runtime.cancel_preview(request),
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn file_workspace_preview_dispose<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: PreviewSessionRequest,
) -> Result<bool, String> {
    require_main_window(&window)?;
    match runtime.current_if_initialized_for_window(&window)? {
        Some(runtime) => runtime.dispose_preview(request),
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn file_workspace_preview_switch_source<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: PreviewSwitchSourceRequest,
) -> Result<super::types::PreviewSnapshotDto, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    spawn_runtime(
        runtime.clone(),
        "workspace_preview_switch_source",
        move |runtime| runtime.switch_preview_source(request),
    )
    .await
}

#[tauri::command]
pub async fn file_workspace_preview_asset_request<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: PreviewAssetRequestDto,
) -> Result<tauri::ipc::Response, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    let payload = spawn_runtime(
        runtime.clone(),
        "workspace_preview_asset_request",
        move |runtime| {
            let artifact = runtime.request_preview_asset(request)?;
            encode_preview_asset_ipc_response(&artifact)
        },
    )
    .await?;
    Ok(tauri::ipc::Response::new(payload))
}

#[tauri::command]
pub async fn file_workspace_transfer_capture<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: crate::file_ops::manual_transfer::CaptureRequest,
) -> Result<Vec<crate::file_ops::manual_transfer::TransferTicket>, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    spawn_runtime(
        runtime.clone(),
        "workspace_transfer_capture",
        move |runtime| runtime.capture_transfer(request),
    )
    .await
}
#[tauri::command]
pub async fn file_workspace_transfer_preview<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: crate::file_ops::manual_transfer::TransferPreviewRequest,
) -> Result<crate::file_ops::manual_transfer::TransferPreview, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    spawn_runtime(
        runtime.clone(),
        "workspace_transfer_preview",
        move |runtime| runtime.preview_transfer(request),
    )
    .await
}
#[tauri::command]
pub async fn file_workspace_transfer_execute<R: Runtime>(
    window: WebviewWindow<R>,
    app: tauri::AppHandle<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    cancel: State<'_, crate::file_ops::OperationCancellationToken>,
    request: crate::file_ops::manual_transfer::TransferExecuteRequest,
) -> Result<crate::file_ops::ExecuteMovesResult, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    crate::file_ops::manual_transfer::execute_registered(app, runtime, cancel.inner(), request)
        .await
}
#[tauri::command]
pub async fn file_workspace_places<R: Runtime>(
    window: WebviewWindow<R>,
    runtime: State<'_, FileWorkspaceRuntimeOwner>,
    request: super::places::PlacesRequest,
) -> Result<Vec<super::places::SavedPlace>, String> {
    require_main_window(&window)?;
    let runtime = runtime.acquire_for_window(&window)?;
    spawn_runtime(runtime.clone(), "workspace_places", move |runtime| {
        runtime.places(request)
    })
    .await
}
