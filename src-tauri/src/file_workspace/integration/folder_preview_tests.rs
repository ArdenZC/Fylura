use super::{
    folder::FolderPreviewTestGate,
    runtime::FileWorkspaceRuntime,
    types::{
        BrowseOpenRequest, BrowsePageDto, BrowseStartEnumerationRequest, PreviewCreateRequest,
        PreviewSessionRequest, PreviewSnapshotDto, PreviewSwitchSourceRequest,
    },
};
use crate::{
    db::Database,
    file_workspace::{
        contracts::{PreviewHostKind, PreviewSourceRef, WorkspacePlatform},
        preview::PreviewRepresentation,
        preview_folder::FolderSummaryPayloadV1,
    },
    platform::macos::quick_look::MacThumbnailService,
    scheduler::{CancellationToken, SchedulerConfig, WorkScheduler},
};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    thread,
};

struct Fixture {
    root: PathBuf,
    scheduler: Arc<WorkScheduler>,
}

fn folder_preview_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp-tests")
            .join(format!("w3-07-folder-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("folder-a").join("src")).expect("folder-a");
        fs::create_dir_all(root.join("folder-b").join("docs")).expect("folder-b");
        fs::write(root.join("folder-a").join("README.md"), b"a").expect("a readme");
        fs::write(root.join("folder-a").join("package.json"), b"{}").expect("a package");
        fs::write(
            root.join("folder-a").join("src").join("main.rs"),
            b"fn main() {}",
        )
        .expect("a source");
        fs::write(root.join("folder-b").join("README.md"), b"b").expect("b readme");
        fs::write(
            root.join("folder-b").join("docs").join("guide.txt"),
            b"guide",
        )
        .expect("b guide");
        fs::write(root.join("root.txt"), b"root").expect("root file");
        Self {
            root,
            scheduler: Arc::new(WorkScheduler::new(SchedulerConfig::default())),
        }
    }

    fn runtime(&self) -> FileWorkspaceRuntime {
        let database = Database::open(self.root.join("zen-canvas.sqlite3")).expect("database");
        let runtime = FileWorkspaceRuntime::new_with_scheduler_for_test(
            database,
            MacThumbnailService::new(self.root.join("legacy-thumbnail-cache")),
            self.root.join("thumbnail-cache"),
            Arc::clone(&self.scheduler),
        )
        .expect("workspace runtime");
        assert!(
            !Arc::ptr_eq(&runtime.inner.scheduler, &WorkScheduler::global()),
            "folder preview fixtures must not use the process-global scheduler"
        );
        runtime
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn platform() -> WorkspacePlatform {
    if cfg!(target_os = "windows") {
        WorkspacePlatform::Windows
    } else {
        WorkspacePlatform::Macos
    }
}

fn open_root(runtime: &FileWorkspaceRuntime, fixture: &Fixture) -> (String, BrowsePageDto) {
    let opened = runtime
        .open_browse(BrowseOpenRequest {
            global_entry_id: None,
            platform: platform(),
            routing_hint: fixture.root.to_string_lossy().into_owned(),
            display_hint: Some("W3-07 folder lifecycle".to_string()),
        })
        .expect("root Browse opens");
    let page = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: opened.session_id.clone(),
            request_id: "w3-07-root".to_string(),
            path_ref: opened.root_path_ref,
            page_size: 256,
            query: Default::default(),
        })
        .expect("root Browse enumerates");
    (opened.session_id, page)
}

fn source_for(page: &BrowsePageDto, name: &str) -> PreviewSourceRef {
    let entry = page
        .entries
        .iter()
        .find(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("Browse entry {name} missing"));
    match &entry.entry_ref {
        crate::file_workspace::BrowseEntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        } => PreviewSourceRef::Ephemeral {
            browse_session_id: browse_session_id.clone(),
            entry_id: entry_id.clone(),
        },
    }
}

fn create_preview(
    runtime: &FileWorkspaceRuntime,
    source: PreviewSourceRef,
    request_id: &str,
) -> String {
    runtime
        .create_preview(PreviewCreateRequest {
            request_id: request_id.to_string(),
            source,
            host_kind: PreviewHostKind::ZenFloating,
        })
        .expect("preview creates")
        .preview_id
}

fn start_preview_async(
    runtime: &FileWorkspaceRuntime,
    preview_id: String,
) -> thread::JoinHandle<Result<PreviewSnapshotDto, String>> {
    let runtime = runtime.clone();
    thread::spawn(move || {
        runtime.start_preview(PreviewSessionRequest {
            preview_id,
            native_presentation: None,
        })
    })
}

fn assert_lease_added(
    baseline: crate::scheduler::SchedulerSnapshot,
    runtime: &FileWorkspaceRuntime,
) {
    let active = runtime.inner.scheduler.snapshot();
    assert_eq!(
        active.running,
        baseline.running + 1,
        "folder lease must add one active work item"
    );
    assert_eq!(
        active.granted.io,
        baseline.granted.io + 1,
        "folder lease must consume one IO slot"
    );
    assert_eq!(
        active.granted.open_handles,
        baseline.granted.open_handles + 1,
        "folder lease must consume one open-handle slot"
    );
}

fn assert_lease_released(
    baseline: crate::scheduler::SchedulerSnapshot,
    runtime: &FileWorkspaceRuntime,
) {
    let after = runtime.inner.scheduler.snapshot();
    assert_eq!(
        after.running, baseline.running,
        "folder lease must be released"
    );
    assert_eq!(after.granted.io, baseline.granted.io);
    assert_eq!(after.granted.open_handles, baseline.granted.open_handles);
}

fn folder_payload(snapshot: &PreviewSnapshotDto) -> FolderSummaryPayloadV1 {
    let envelope = snapshot
        .representation
        .as_ref()
        .expect("folder representation");
    let PreviewRepresentation::FolderSummary { encoded_summary } = &envelope.representation else {
        panic!("expected FolderSummary representation");
    };
    serde_json::from_str(encoded_summary).expect("FolderSummary payload")
}

fn cleanup_preview_and_browse(
    runtime: &FileWorkspaceRuntime,
    preview_id: &str,
    browse_session_id: &str,
    page: BrowsePageDto,
) {
    runtime
        .release_page(super::types::BrowseReleasePageRequest { page })
        .expect("visible Browse page releases");
    runtime
        .dispose_preview(PreviewSessionRequest {
            preview_id: preview_id.to_string(),
            native_presentation: None,
        })
        .expect("preview disposes");
    runtime
        .dispose_browse(super::types::BrowseSessionRequest {
            session_id: browse_session_id.to_string(),
        })
        .expect("visible Browse session disposes");
}

#[test]
fn folder_preview_scheduler_owners_are_isolated() {
    let scheduler_a = Arc::new(WorkScheduler::new(SchedulerConfig::default()));
    let scheduler_b = Arc::new(WorkScheduler::new(SchedulerConfig::default()));
    let adapter_a = crate::scheduler::adapters::FolderPreviewResourceLeaseAdapter::new(Arc::clone(
        &scheduler_a,
    ));
    let lease = adapter_a
        .try_acquire(
            "folder-preview-a",
            "folder-session-a",
            CancellationToken::new(),
        )
        .expect("folder preview lease on owner A");

    assert_eq!(scheduler_a.snapshot().running, 1);
    assert_eq!(scheduler_b.snapshot().running, 0);
    assert_eq!(
        scheduler_b.snapshot().granted,
        crate::scheduler::ResourceHints::empty()
    );

    drop(lease);
    assert_eq!(scheduler_a.snapshot().running, 0);
    assert_eq!(scheduler_b.snapshot().running, 0);
}

#[test]
fn real_browse_adapter_issues_and_releases_folder_lease_after_success() {
    let _test_lock = folder_preview_test_lock()
        .lock()
        .expect("folder preview test lock");
    let fixture = Fixture::new("success");
    let runtime = fixture.runtime();
    let (browse_session_id, page) = open_root(&runtime, &fixture);
    let source = source_for(&page, "folder-a");
    let preview_id = create_preview(&runtime, source, "w3-07-success");
    let baseline_scheduler = runtime.inner.scheduler.snapshot();
    let baseline_resources = runtime.resource_counts();
    let visible_before = runtime
        .inner
        .browse
        .active_enumeration_debug(&browse_session_id)
        .expect("visible Browse enumeration identity")
        .expect("visible Browse enumeration");
    assert_eq!(visible_before.session_id, page.session_id);
    assert_eq!(visible_before.request_id, page.request_id);
    assert_eq!(visible_before.enumeration_id, page.enumeration_id);
    assert_eq!(visible_before.current_cursor, page.next_cursor);
    let gate = Arc::new(FolderPreviewTestGate::default());
    runtime
        .inner
        .folder_enumeration
        .set_test_gate(Some(Arc::clone(&gate)));

    let task = start_preview_async(&runtime, preview_id.clone());
    gate.wait_for_lease();
    assert_lease_added(baseline_scheduler, &runtime);
    let during = runtime.resource_counts();
    assert_eq!(
        during.browse_service_sessions,
        baseline_resources.browse_service_sessions + 1
    );
    let visible_during = runtime
        .inner
        .browse
        .active_enumeration_debug(&browse_session_id)
        .expect("visible Browse identity during folder preview")
        .expect("visible Browse enumeration during folder preview");
    assert_eq!(visible_during, visible_before);
    assert_eq!(
        (
            page.session_id.as_str(),
            page.request_id.as_str(),
            page.enumeration_id.as_str(),
            page.next_cursor.as_deref()
        ),
        (
            visible_during.session_id.as_str(),
            visible_during.request_id.as_str(),
            visible_during.enumeration_id.as_str(),
            visible_during.current_cursor.as_deref()
        ),
        "temporary preview Browse work must not mutate visible Browse identity or cursor"
    );
    gate.release_lease();
    gate.wait_for_page();
    let visible_entry = page
        .entries
        .iter()
        .find(|entry| entry.name == "folder-a")
        .expect("folder entry");
    assert!(
        runtime
            .inner
            .browse
            .resolve_entry(&visible_entry.entry_ref)
            .is_ok(),
        "visible Browse ref must remain valid while preview enumerates"
    );
    gate.release_page();
    let result = task
        .join()
        .expect("preview worker join")
        .expect("folder preview succeeds");
    assert_eq!(result.state, super::types::PreviewSessionStateDto::Ready);
    assert_eq!(result.active_provider_id.as_deref(), Some("builtin.folder"));
    let payload = folder_payload(&result);
    assert_eq!(payload.folder_name, "folder-a");
    assert_eq!(
        payload.progress.state,
        crate::file_workspace::preview_folder::FolderSummaryStateV1::Complete
    );
    assert_eq!(payload.progress.accepted_children, 3);
    assert_eq!(payload.kind_counts.directories, 1);
    let visible_after = runtime
        .inner
        .browse
        .active_enumeration_debug(&browse_session_id)
        .expect("visible Browse identity after folder preview")
        .expect("visible Browse enumeration after folder preview");
    assert_eq!(visible_after, visible_before);
    assert!(
        runtime
            .inner
            .browse
            .resolve_entry(&visible_entry.entry_ref)
            .is_ok(),
        "visible Browse ref must remain valid after temporary preview work"
    );
    assert_lease_released(baseline_scheduler, &runtime);
    assert_eq!(
        runtime.resource_counts().browse_service_sessions,
        baseline_resources.browse_service_sessions
    );
    runtime.inner.folder_enumeration.set_test_gate(None);
    cleanup_preview_and_browse(&runtime, &preview_id, &browse_session_id, page);
    assert_eq!(runtime.resource_counts().browse_service_sessions, 0);
}

#[test]
fn real_folder_provider_failure_after_lease_returns_to_scheduler_baseline() {
    let _test_lock = folder_preview_test_lock()
        .lock()
        .expect("folder preview test lock");
    let fixture = Fixture::new("failure");
    let runtime = fixture.runtime();
    let (browse_session_id, page) = open_root(&runtime, &fixture);
    let source = source_for(&page, "folder-a");
    let preview_id = create_preview(&runtime, source, "w3-07-failure");
    let baseline_scheduler = runtime.inner.scheduler.snapshot();
    let gate = Arc::new(FolderPreviewTestGate::default());
    runtime
        .inner
        .folder_enumeration
        .set_test_gate(Some(Arc::clone(&gate)));
    let task = start_preview_async(&runtime, preview_id.clone());
    gate.wait_for_lease();
    assert_lease_added(baseline_scheduler, &runtime);
    fs::remove_dir_all(fixture.root.join("folder-a")).expect("remove folder after lease issue");
    gate.release_lease();
    let result = task.join().expect("failure worker join");
    assert!(result.is_err(), "source invalidation must remain terminal");
    assert_lease_released(baseline_scheduler, &runtime);
    runtime.inner.folder_enumeration.set_test_gate(None);
    cleanup_preview_and_browse(&runtime, &preview_id, &browse_session_id, page);
}

#[test]
fn real_folder_cancel_releases_lease_without_waiting_for_a_page() {
    let _test_lock = folder_preview_test_lock()
        .lock()
        .expect("folder preview test lock");
    let fixture = Fixture::new("cancel");
    let runtime = fixture.runtime();
    let (browse_session_id, page) = open_root(&runtime, &fixture);
    let source = source_for(&page, "folder-a");
    let preview_id = create_preview(&runtime, source, "w3-07-cancel");
    let baseline_scheduler = runtime.inner.scheduler.snapshot();
    let gate = Arc::new(FolderPreviewTestGate::default());
    runtime
        .inner
        .folder_enumeration
        .set_test_gate(Some(Arc::clone(&gate)));
    let task = start_preview_async(&runtime, preview_id.clone());
    gate.wait_for_lease();
    assert_lease_added(baseline_scheduler, &runtime);
    runtime
        .cancel_preview(PreviewSessionRequest {
            preview_id: preview_id.clone(),
            native_presentation: None,
        })
        .expect("preview cancellation");
    gate.release_lease();
    let result = task.join().expect("cancel worker join");
    assert_eq!(result, Err("preview_cancelled".to_string()));
    assert_lease_released(baseline_scheduler, &runtime);
    runtime.inner.folder_enumeration.set_test_gate(None);
    cleanup_preview_and_browse(&runtime, &preview_id, &browse_session_id, page);
}

#[test]
fn stale_folder_a_cannot_publish_after_switch_to_folder_b() {
    let _test_lock = folder_preview_test_lock()
        .lock()
        .expect("folder preview test lock");
    let fixture = Fixture::new("stale-switch");
    let runtime = fixture.runtime();
    let (browse_session_id, page) = open_root(&runtime, &fixture);
    let source_a = source_for(&page, "folder-a");
    let source_b = source_for(&page, "folder-b");
    let preview_id = create_preview(&runtime, source_a, "w3-07-stale-a");
    let baseline_scheduler = runtime.inner.scheduler.snapshot();
    let gate = Arc::new(FolderPreviewTestGate::default());
    runtime
        .inner
        .folder_enumeration
        .set_test_gate(Some(Arc::clone(&gate)));
    let task_a = start_preview_async(&runtime, preview_id.clone());
    gate.wait_for_lease();
    assert_lease_added(baseline_scheduler, &runtime);
    gate.release_lease();
    gate.wait_for_page();
    runtime
        .switch_preview_source(PreviewSwitchSourceRequest {
            preview_id: preview_id.clone(),
            request_id: "w3-07-stale-b".to_string(),
            source: source_b,
        })
        .expect("switch source");
    let switched = runtime
        .snapshot_preview(PreviewSessionRequest {
            preview_id: preview_id.clone(),
            native_presentation: None,
        })
        .expect("switched snapshot");
    assert_eq!(switched.request_id, "w3-07-stale-b");
    assert!(
        switched.representation.is_none(),
        "switch must clear A's partial representation"
    );
    gate.release_page();
    let old_result = task_a.join().expect("stale worker join");
    assert!(
        matches!(old_result, Err(ref error) if error == "preview_stale_publication" || error == "preview_cancelled"),
        "stale A result: {old_result:?}"
    );
    assert_lease_released(baseline_scheduler, &runtime);
    runtime.inner.folder_enumeration.set_test_gate(None);

    let result_b = runtime
        .start_preview(PreviewSessionRequest {
            preview_id: preview_id.clone(),
            native_presentation: None,
        })
        .expect("folder B preview succeeds");
    let payload_b = folder_payload(&result_b);
    assert_eq!(payload_b.folder_name, "folder-b");
    assert_eq!(result_b.request_id, "w3-07-stale-b");
    assert_lease_released(baseline_scheduler, &runtime);
    cleanup_preview_and_browse(&runtime, &preview_id, &browse_session_id, page);
}
