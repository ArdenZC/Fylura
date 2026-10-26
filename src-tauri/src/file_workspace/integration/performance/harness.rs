use super::{fixture::WorkspaceFixture, metrics};
use crate::{
    db::Database,
    file_workspace::{
        browse::BrowseLimits,
        contracts::WorkspacePlatform,
        integration::{types::BrowseOpenRequest, FileWorkspaceRuntime},
        thumbnail::ThumbnailRenderer,
    },
    platform::macos::quick_look::MacThumbnailService,
};
use serde_json::json;
use std::{path::Path, sync::Arc};

fn platform() -> WorkspacePlatform {
    if cfg!(target_os = "windows") {
        WorkspacePlatform::Windows
    } else if cfg!(target_os = "macos") {
        WorkspacePlatform::Macos
    } else {
        panic!("File Workspace performance harness is unsupported on this platform")
    }
}

pub(super) fn runtime_for(fixture: &WorkspaceFixture) -> FileWorkspaceRuntime {
    FileWorkspaceRuntime::new_with_native_preview_root(
        Database::open(fixture.state_path().join("zen-canvas.sqlite3"))
            .expect("open harness database"),
        MacThumbnailService::new(fixture.state_path().join("legacy-thumbnail-cache")),
        fixture.state_path().join("thumbnail-cache"),
        fixture.state_path().join("native-preview"),
    )
    .expect("create File Workspace runtime")
}

pub(super) fn runtime_for_browse_limits(
    fixture: &WorkspaceFixture,
    max_entry_refs: usize,
    max_path_refs: usize,
    max_process_entry_refs: usize,
    max_process_path_refs: usize,
) -> FileWorkspaceRuntime {
    FileWorkspaceRuntime::new_with_browse_limits_for_test(
        Database::open(fixture.state_path().join("zen-canvas.sqlite3"))
            .expect("open harness database"),
        MacThumbnailService::new(fixture.state_path().join("legacy-thumbnail-cache")),
        fixture.state_path().join("thumbnail-cache"),
        BrowseLimits {
            max_sessions: 32,
            max_page_size: 256,
            max_path_refs,
            max_entry_refs,
            max_process_path_refs,
            max_process_entry_refs,
        },
    )
    .expect("create bounded File Workspace runtime")
}

pub(super) fn runtime_for_with_renderer(
    fixture: &WorkspaceFixture,
    renderer: Arc<dyn ThumbnailRenderer>,
) -> FileWorkspaceRuntime {
    FileWorkspaceRuntime::new_with_thumbnail_renderer_for_test(
        Database::open(fixture.state_path().join("zen-canvas.sqlite3"))
            .expect("open harness database"),
        renderer,
        fixture.state_path().join("thumbnail-cache"),
    )
    .expect("create renderer-backed File Workspace runtime")
}

pub(super) fn open_fixture(
    runtime: &FileWorkspaceRuntime,
    fixture: &WorkspaceFixture,
    display_hint: &str,
) -> crate::file_workspace::integration::types::BrowseOpenResponse {
    try_open_fixture(runtime, fixture, display_hint).expect("admit real filesystem fixture")
}

pub(super) fn try_open_fixture(
    runtime: &FileWorkspaceRuntime,
    fixture: &WorkspaceFixture,
    display_hint: &str,
) -> Result<crate::file_workspace::integration::types::BrowseOpenResponse, String> {
    try_open_path(runtime, fixture.path(), display_hint)
}

pub(super) fn open_path(
    runtime: &FileWorkspaceRuntime,
    path: &Path,
    display_hint: &str,
) -> crate::file_workspace::integration::types::BrowseOpenResponse {
    try_open_path(runtime, path, display_hint).expect("admit real filesystem path")
}

pub(super) fn try_open_path(
    runtime: &FileWorkspaceRuntime,
    path: &Path,
    display_hint: &str,
) -> Result<crate::file_workspace::integration::types::BrowseOpenResponse, String> {
    runtime.open_browse(BrowseOpenRequest {
        global_entry_id: None,
        platform: platform(),
        routing_hint: path.to_string_lossy().into_owned(),
        display_hint: Some(display_hint.to_string()),
    })
}

#[test]
#[ignore = "W1-11 File Workspace/Foundation performance harness"]
fn harness_smoke() {
    let fixture = WorkspaceFixture::smoke();
    let runtime = runtime_for(&fixture);
    let opened = open_fixture(&runtime, &fixture, "W1-11 harness");
    let page = runtime
        .start_enumeration(
            crate::file_workspace::integration::types::BrowseStartEnumerationRequest {
                session_id: opened.session_id.clone(),
                request_id: "workspace-foundation-smoke".to_string(),
                path_ref: opened.root_path_ref,
                page_size: 2,
                query: Default::default(),
            },
        )
        .expect("enumerate first useful page");
    assert!(
        !page.entries.is_empty(),
        "real fixture should publish a first page"
    );
    assert!(page.entries.iter().all(|entry| {
        matches!(
            entry.entry_ref,
            crate::file_workspace::BrowseEntryRef::Ephemeral { .. }
        )
    }));
    assert!(runtime.dispose());

    metrics::emit_metric(
        "harness_smoke",
        metrics::HARD_PASS,
        [
            ("fixture_entries".to_string(), json!(3)),
            ("requested_page_size".to_string(), json!(2)),
            ("published_entries".to_string(), json!(page.entries.len())),
            ("fixture_root_scope".to_string(), json!("repository-local")),
            ("raw_path_authority".to_string(), json!(false)),
        ],
    );
}
