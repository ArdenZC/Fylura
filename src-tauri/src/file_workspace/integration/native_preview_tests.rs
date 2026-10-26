#[cfg(target_os = "macos")]
use super::types::PreviewSnapshotDto;
use super::{
    types::{
        BrowseOpenRequest, BrowseSessionRequest, BrowseStartEnumerationRequest,
        PreviewCreateRequest, PreviewSessionRequest, PreviewSwitchSourceRequest,
    },
    FileWorkspaceRuntime,
};
#[cfg(target_os = "macos")]
use crate::file_workspace::{
    integration::types::{PreviewNativeBounds, PreviewNativePresentation},
    preview::PreviewRepresentation,
};
use crate::{
    db::Database,
    file_workspace::{
        native_preview::{
            access::{
                NativePreviewAccessError, NativePreviewAccessRequest,
                NativePreviewAccessResolveRequest,
            },
            host_provided::{
                HostProvidedReadSource, HostProvidedRegistration, HostProvidedSourceError,
            },
        },
        BoundedContentRead, BrowseEntryRef, PreviewCancellation, PreviewHostKind,
        PreviewOperationContext, PreviewSourceRef, WorkspacePlatform,
    },
    platform::macos::quick_look::MacThumbnailService,
};
#[cfg(target_os = "macos")]
use std::path::Path;
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp-tests")
            .join(format!("w4-01-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("fixture root");
        fs::write(root.join("alpha.txt"), vec![b'a'; 512 * 1024 + 17]).expect("alpha fixture");
        fs::write(root.join("beta.txt"), b"native preview beta").expect("beta fixture");
        fs::write(
            root.join("native.pdf"),
            b"%PDF-1.4\n% native preview fixture\n",
        )
        .expect("native PDF fixture");
        Self { root }
    }

    fn runtime(&self) -> FileWorkspaceRuntime {
        FileWorkspaceRuntime::new(
            Database::open(self.root.join("zen-canvas.sqlite3")).expect("database"),
            MacThumbnailService::new(self.root.join("legacy-thumbnail-cache")),
            self.root.join("thumbnail-cache"),
        )
        .expect("workspace runtime")
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

fn open_sources(
    runtime: &FileWorkspaceRuntime,
    fixture: &Fixture,
) -> (String, PreviewSourceRef, PreviewSourceRef, PreviewSourceRef) {
    let opened = runtime
        .open_browse(BrowseOpenRequest {
            global_entry_id: None,
            platform: platform(),
            routing_hint: fixture.root.to_string_lossy().into_owned(),
            display_hint: Some("W4-01 native fixture".to_string()),
        })
        .expect("open browse fixture");
    let page = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: opened.session_id.clone(),
            request_id: "w4-native-enumerate".to_string(),
            path_ref: opened.root_path_ref,
            page_size: 16,
            query: Default::default(),
        })
        .expect("enumerate fixture");

    let source_for = |name: &str| {
        let entry = page
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap_or_else(|| panic!("missing fixture entry {name}"));
        match &entry.entry_ref {
            BrowseEntryRef::Ephemeral {
                browse_session_id,
                entry_id,
            } => PreviewSourceRef::Ephemeral {
                browse_session_id: browse_session_id.clone(),
                entry_id: entry_id.clone(),
            },
        }
    };

    (
        opened.session_id,
        source_for("alpha.txt"),
        source_for("beta.txt"),
        source_for("native.pdf"),
    )
}

fn stage_for_preview(
    runtime: &FileWorkspaceRuntime,
    preview_id: &str,
    request_id: &str,
    source: PreviewSourceRef,
) -> PathBuf {
    stage_request_for_preview(runtime, preview_id, request_id, source).1
}

fn stage_request_for_preview(
    runtime: &FileWorkspaceRuntime,
    preview_id: &str,
    request_id: &str,
    source: PreviewSourceRef,
) -> (NativePreviewAccessResolveRequest, PathBuf) {
    let source_version = runtime
        .inner
        .read_gate
        .current_source_version(&source)
        .expect("source version");
    let context = PreviewOperationContext::for_backend_content_read(
        preview_id.to_string(),
        request_id.to_string(),
        source_version.clone(),
        PreviewCancellation::default(),
        Instant::now() + Duration::from_secs(5),
    );
    let handle = runtime
        .inner
        .native_preview_access
        .stage(
            NativePreviewAccessRequest {
                session_id: preview_id.to_string(),
                request_id: request_id.to_string(),
                source,
                source_version: source_version.clone(),
                host: PreviewHostKind::ZenFloating,
            },
            &context,
        )
        .expect("stage native preview");
    let resolve_request = NativePreviewAccessResolveRequest {
        token: handle.token,
        session_id: preview_id.to_string(),
        request_id: request_id.to_string(),
        source_version,
        host: PreviewHostKind::ZenFloating,
    };
    let staged_path = runtime
        .inner
        .native_preview_access
        .resolve(&resolve_request)
        .expect("resolve staged native preview");
    (resolve_request, staged_path)
}

fn create_preview(
    runtime: &FileWorkspaceRuntime,
    request_id: &str,
    source: PreviewSourceRef,
) -> String {
    runtime
        .create_preview(PreviewCreateRequest {
            request_id: request_id.to_string(),
            source,
            host_kind: PreviewHostKind::ZenFloating,
        })
        .expect("create preview")
        .preview_id
}

fn assert_native_empty(runtime: &FileWorkspaceRuntime) {
    let (records, inflight, bytes) = runtime.inner.native_preview_access.counts();
    assert_eq!((records, inflight, bytes), (0, 0, 0));
}

fn assert_no_native_stage_roots(fixture: &Fixture) {
    let roots = fs::read_dir(fixture.root.join("native-preview"))
        .expect("native preview root")
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".native-preview-")
        })
        .count();
    assert_eq!(roots, 0);
}

#[test]
fn preview_cancel_dispose_and_switch_revoke_native_staging() {
    let fixture = Fixture::new("preview-lifecycle");
    let runtime = fixture.runtime();
    let (_browse_session_id, alpha, beta, _) = open_sources(&runtime, &fixture);

    let cancel_id = create_preview(&runtime, "cancel-request", alpha.clone());
    let cancel_path = stage_for_preview(&runtime, &cancel_id, "cancel-request", alpha.clone());
    assert!(cancel_path.is_file());
    runtime
        .cancel_preview(PreviewSessionRequest {
            preview_id: cancel_id,
            native_presentation: None,
        })
        .expect("cancel preview");
    assert!(!cancel_path.exists());
    assert_native_empty(&runtime);

    let dispose_id = create_preview(&runtime, "dispose-request", alpha.clone());
    let dispose_path = stage_for_preview(&runtime, &dispose_id, "dispose-request", alpha.clone());
    runtime
        .dispose_preview(PreviewSessionRequest {
            preview_id: dispose_id,
            native_presentation: None,
        })
        .expect("dispose preview");
    assert!(!dispose_path.exists());
    assert_native_empty(&runtime);

    let switch_id = create_preview(&runtime, "switch-request-a", alpha.clone());
    let switch_path = stage_for_preview(&runtime, &switch_id, "switch-request-a", alpha);
    runtime
        .switch_preview_source(PreviewSwitchSourceRequest {
            preview_id: switch_id,
            request_id: "switch-request-b".to_string(),
            source: beta,
        })
        .expect("switch preview source");
    assert!(!switch_path.exists());
    assert_native_empty(&runtime);

    assert!(runtime.dispose());
}

#[test]
fn browse_dispose_revokes_ephemeral_preview_native_staging() {
    let fixture = Fixture::new("browse-dispose");
    let runtime = fixture.runtime();
    let (browse_session_id, alpha, _, _) = open_sources(&runtime, &fixture);
    let preview_id = create_preview(&runtime, "browse-preview", alpha.clone());
    let staged_path = stage_for_preview(&runtime, &preview_id, "browse-preview", alpha);

    runtime
        .dispose_browse(BrowseSessionRequest {
            session_id: browse_session_id,
        })
        .expect("dispose browse");
    assert!(!staged_path.exists());
    assert_native_empty(&runtime);
    assert_eq!(
        runtime
            .inner
            .preview_sessions
            .lock()
            .expect("preview sessions")
            .len(),
        0
    );
    assert!(runtime.dispose());
}

#[cfg(target_os = "macos")]
fn assert_staged_request_revoked(
    runtime: &FileWorkspaceRuntime,
    request: &NativePreviewAccessResolveRequest,
    staged_path: &Path,
) {
    assert!(runtime
        .inner
        .native_preview_access
        .resolve(request)
        .is_err());
    assert!(!staged_path.exists());
    assert_native_empty(runtime);
}

#[cfg(target_os = "macos")]
#[test]
fn detach_failure_still_revokes_cancel_dispose_switch_and_browse_teardown() {
    let fixture = Fixture::new("detach-failure-lifecycle");
    let runtime = fixture.runtime();
    let (browse_session_id, alpha, beta, _) = open_sources(&runtime, &fixture);

    let cancel_id = create_preview(&runtime, "cancel-failure", alpha.clone());
    let (cancel_request, cancel_path) =
        stage_request_for_preview(&runtime, &cancel_id, "cancel-failure", alpha.clone());
    runtime
        .inner
        .native_preview_host
        .fail_next_detach_for_test();
    assert!(runtime
        .cancel_preview(PreviewSessionRequest {
            preview_id: cancel_id,
            native_presentation: None,
        })
        .is_err());
    assert_staged_request_revoked(&runtime, &cancel_request, &cancel_path);

    let dispose_id = create_preview(&runtime, "dispose-failure", alpha.clone());
    let (dispose_request, dispose_path) =
        stage_request_for_preview(&runtime, &dispose_id, "dispose-failure", alpha.clone());
    runtime
        .inner
        .native_preview_host
        .fail_next_detach_for_test();
    assert!(runtime
        .dispose_preview(PreviewSessionRequest {
            preview_id: dispose_id,
            native_presentation: None,
        })
        .is_err());
    assert_staged_request_revoked(&runtime, &dispose_request, &dispose_path);

    let switch_id = create_preview(&runtime, "switch-failure-a", alpha.clone());
    let (switch_request, switch_path) =
        stage_request_for_preview(&runtime, &switch_id, "switch-failure-a", alpha.clone());
    runtime
        .inner
        .native_preview_host
        .fail_next_detach_for_test();
    assert!(runtime
        .switch_preview_source(PreviewSwitchSourceRequest {
            preview_id: switch_id,
            request_id: "switch-failure-b".to_string(),
            source: beta,
        })
        .is_err());
    assert_staged_request_revoked(&runtime, &switch_request, &switch_path);

    let browse_preview_id = create_preview(&runtime, "browse-failure", alpha.clone());
    let (browse_request, browse_path) =
        stage_request_for_preview(&runtime, &browse_preview_id, "browse-failure", alpha);
    runtime
        .inner
        .native_preview_host
        .fail_next_detach_for_test();
    assert!(runtime
        .dispose_browse(BrowseSessionRequest {
            session_id: browse_session_id,
        })
        .is_err());
    assert_staged_request_revoked(&runtime, &browse_request, &browse_path);
    assert!(runtime.dispose());
}

#[cfg(target_os = "macos")]
fn native_presentation_for_snapshot(snapshot: &PreviewSnapshotDto) -> PreviewNativePresentation {
    let Some(crate::file_workspace::preview::PreviewRepresentation::NativeOpaque { host, token }) =
        snapshot
            .representation
            .as_ref()
            .map(|envelope| &envelope.representation)
    else {
        panic!("expected native representation");
    };
    PreviewNativePresentation {
        host: *host,
        token: token.clone(),
        source_version: snapshot
            .source_version
            .clone()
            .expect("native source version"),
        bounds: PreviewNativeBounds {
            x: 0,
            y: 0,
            width: 400,
            height: 300,
        },
    }
}

#[cfg(target_os = "macos")]
#[test]
fn native_presentation_failure_revokes_exact_claim_after_detach_failure() {
    let fixture = Fixture::new("native-failure-cleanup");
    let runtime = fixture.runtime();
    let (_browse_session_id, _, _, native_source) = open_sources(&runtime, &fixture);
    let preview_id = create_preview(&runtime, "native-failure", native_source);
    let snapshot = runtime
        .start_preview(PreviewSessionRequest {
            preview_id: preview_id.clone(),
            native_presentation: None,
        })
        .expect("native preview starts");
    let presentation = native_presentation_for_snapshot(&snapshot);
    let request = NativePreviewAccessResolveRequest {
        token: presentation.token.clone(),
        session_id: snapshot.session_id.clone(),
        request_id: snapshot.request_id.clone(),
        source_version: presentation.source_version.clone(),
        host: presentation.host,
    };
    let staged_path = runtime
        .inner
        .native_preview_access
        .resolve(&request)
        .expect("native stage resolves before attach failure");

    runtime
        .inner
        .native_preview_host
        .fail_next_detach_for_test();
    let result = runtime.native_preview_attach_failed(
        &snapshot,
        &presentation,
        "test_native_presentation_failed".to_string(),
    );
    let error = result.expect_err("detach failure remains observable");
    assert!(error.contains("macos_quick_look_test_detach_failed"));
    assert_staged_request_revoked(&runtime, &request, &staged_path);
    let fallback = runtime
        .snapshot_preview(PreviewSessionRequest {
            preview_id,
            native_presentation: None,
        })
        .expect("snapshot after native fallback");
    assert!(matches!(
        fallback
            .representation
            .as_ref()
            .map(|envelope| &envelope.representation),
        Some(PreviewRepresentation::Metadata { .. })
    ));
    assert!(runtime.dispose());
}

struct DropTrackedSource {
    drops: Arc<AtomicUsize>,
}

impl Drop for DropTrackedSource {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
    }
}

impl HostProvidedReadSource for DropTrackedSource {
    fn read_bounded(
        &self,
        _offset_bytes: u64,
        _max_bytes: u32,
        _context: &crate::file_workspace::native_preview::host_provided::HostProvidedReadContext,
    ) -> Result<BoundedContentRead, HostProvidedSourceError> {
        Ok(BoundedContentRead {
            bytes: b"shell-owned".to_vec(),
            complete: true,
        })
    }
}

#[test]
fn runtime_dispose_revokes_native_and_host_provided_resources() {
    let fixture = Fixture::new("runtime-dispose");
    let runtime = fixture.runtime();
    let (_browse_session_id, alpha, _, _) = open_sources(&runtime, &fixture);
    let preview_id = create_preview(&runtime, "runtime-preview", alpha.clone());
    let staged_path = stage_for_preview(&runtime, &preview_id, "runtime-preview", alpha);

    let drops = Arc::new(AtomicUsize::new(0));
    runtime
        .inner
        .host_provided
        .register(HostProvidedRegistration {
            host: PreviewHostKind::WindowsPreviewHandler,
            generation_id: "runtime-host-generation".to_string(),
            source: Arc::new(DropTrackedSource {
                drops: Arc::clone(&drops),
            }),
        })
        .expect("register host-provided source");
    assert_eq!(runtime.inner.host_provided.count(), 1);

    assert!(runtime.dispose());
    assert!(!staged_path.exists());
    assert_native_empty(&runtime);
    assert_eq!(runtime.inner.host_provided.count(), 0);
    assert_eq!(drops.load(Ordering::Acquire), 1);
}

#[test]
fn runtime_dispose_cancels_inflight_native_staging_and_releases_composition() {
    let fixture = Fixture::new("runtime-inflight-dispose");
    let runtime = fixture.runtime();
    let (_browse_session_id, alpha, _, _) = open_sources(&runtime, &fixture);
    let preview_id = create_preview(&runtime, "runtime-inflight", alpha.clone());
    let source_version = runtime
        .inner
        .read_gate
        .current_source_version(&alpha)
        .expect("source version");
    let context = PreviewOperationContext::for_backend_content_read(
        preview_id.clone(),
        "runtime-inflight",
        source_version.clone(),
        PreviewCancellation::default(),
        Instant::now() + Duration::from_secs(5),
    );
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    runtime
        .inner
        .native_preview_access
        .set_after_first_copy_chunk_hook(Some(Arc::new(move || {
            entered_tx
                .send(())
                .expect("staging worker entered copy hook");
            release_rx
                .lock()
                .expect("copy hook release lock")
                .recv_timeout(Duration::from_secs(5))
                .expect("release in-flight staging");
        })));

    let worker_registry = Arc::clone(&runtime.inner.native_preview_access);
    let worker_source = alpha;
    let worker_version = source_version.clone();
    let worker_context = context;
    let worker = thread::spawn(move || {
        worker_registry.stage(
            NativePreviewAccessRequest {
                session_id: preview_id,
                request_id: "runtime-inflight".to_string(),
                source: worker_source,
                source_version: worker_version,
                host: PreviewHostKind::ZenFloating,
            },
            &worker_context,
        )
    });

    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("multi-chunk staging reached first-copy hook");
    let during = runtime.resource_counts();
    assert_eq!(
        (
            during.native_preview_records,
            during.native_preview_inflight,
            during.native_preview_bytes
        ),
        (0, 1, 0)
    );
    assert_eq!(runtime.inner.read_gate.active_lease_count(), 1);

    assert!(runtime.dispose());
    assert!(runtime.ensure_live().is_err());
    let after_dispose = runtime.resource_counts();
    assert_eq!(after_dispose.native_preview_records, 0);
    assert_eq!(after_dispose.native_preview_bytes, 0);
    assert_eq!(runtime.inner.read_gate.active_lease_count(), 0);

    release_tx
        .send(())
        .expect("release disposed staging worker");
    assert_eq!(
        worker.join().unwrap(),
        Err(NativePreviewAccessError::Cancelled)
    );
    assert_native_empty(&runtime);
    assert_eq!(runtime.inner.read_gate.active_lease_count(), 0);
    assert_no_native_stage_roots(&fixture);
    assert_eq!(runtime.inner.host_provided.count(), 0);
}
