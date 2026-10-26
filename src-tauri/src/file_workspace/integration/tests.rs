use super::{
    types::{
        BrowseCancelRequest, BrowseCompletionDto, BrowseOpenRequest, BrowseRestoreRequest,
        BrowseRetainPathRequest, BrowseStartEnumerationRequest, ChangePendingRequest,
        ChangeStartRequest, LocationBrowseRequest, PreviewAssetRequestDto, PreviewCreateRequest,
        PreviewSessionRequest, ThumbnailCancelRequest, ThumbnailRequestDto, ThumbnailVariantDto,
    },
    FileWorkspaceRuntime, FileWorkspaceRuntimeOwner,
};
use crate::{
    db::{scan::ScanAdmissionOptions, Database},
    file_workspace::{
        contracts::{
            ContentReadEligibility, LocationRef, PreviewHostKind, PreviewSourceRef, WorkClass,
            WorkspacePlatform,
        },
        preview::{PreviewAssetPublisher, PreviewCancellation, PreviewOperationContext},
        thumbnail::{
            ThumbnailRenderContext, ThumbnailRenderOutput, ThumbnailRenderRequest,
            ThumbnailRenderer, ThumbnailRendererDescriptor, ThumbnailRendererError,
        },
    },
    platform::macos::quick_look::MacThumbnailService,
    scanner::ManagedScanRequest,
    scheduler::ResourceHints,
};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
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
            .join(format!("w1-10-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("fixture root");
        fs::create_dir(root.join("nested")).expect("nested directory");
        fs::write(root.join("entry.txt"), b"workspace integration").expect("fixture file");
        Self { root }
    }

    fn runtime(&self) -> FileWorkspaceRuntime {
        let database = Database::open(self.root.join("zen-canvas.sqlite3")).expect("database");
        FileWorkspaceRuntime::new(
            database,
            MacThumbnailService::new(self.root.join("legacy-thumbnail-cache")),
            self.root.join("thumbnail-cache"),
        )
        .expect("workspace runtime")
    }

    fn runtime_with_renderer(&self, renderer: Arc<dyn ThumbnailRenderer>) -> FileWorkspaceRuntime {
        let database = Database::open(self.root.join("zen-canvas-custom-thumbnail.sqlite3"))
            .expect("database");
        FileWorkspaceRuntime::new_with_thumbnail_renderer_for_test(
            database,
            renderer,
            self.root.join("custom-thumbnail-cache"),
        )
        .expect("workspace runtime")
    }

    fn runtime_owner(&self) -> FileWorkspaceRuntimeOwner {
        let database = Database::open(self.root.join("runtime-owner.sqlite3")).expect("database");
        FileWorkspaceRuntimeOwner::new(
            database,
            MacThumbnailService::new(self.root.join("legacy-owner-thumbnail-cache")),
            self.root.join("owner-thumbnail-cache"),
            self.root.join("owner-native-preview"),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn runtime_owner_is_lazy_single_generation_and_recreates_after_teardown() {
    let fixture = Fixture::new("runtime-owner-generations");
    let owner = fixture.runtime_owner();

    assert_eq!(owner.current_generation(), None);
    assert!(!owner.is_initialized());
    owner
        .activate_generation(1)
        .expect("activate Main generation 1");
    assert!(owner
        .current_if_initialized(1)
        .expect("empty owner")
        .is_none());
    assert!(!owner.is_initialized());

    let first = owner.acquire(1).expect("first lazy runtime");
    let concurrent = owner.acquire(1).expect("reuse runtime in generation");
    assert!(Arc::ptr_eq(&first.inner, &concurrent.inner));
    assert!(owner.is_initialized());

    owner.dispose_generation(1).expect("dispose generation 1");
    assert_eq!(owner.current_generation(), None);
    assert!(!owner.is_initialized());
    assert!(owner
        .current_if_initialized(1)
        .expect("closed generation cleanup is a no-op")
        .is_none());
    assert!(first
        .open_browse(open_request(&fixture))
        .expect_err("old clone is revoked")
        .contains("file_workspace_runtime_disposed"));
    assert!(!first.dispose(), "owner already disposed the generation");
    assert_eq!(
        owner
            .acquire(1)
            .err()
            .expect("old generation cannot be reused"),
        "file_workspace_main_generation_inactive"
    );

    owner
        .activate_generation(2)
        .expect("activate Main generation 2");
    let reopened = owner.acquire(2).expect("reopened lazy runtime");
    assert!(!Arc::ptr_eq(&first.inner, &reopened.inner));
    owner.dispose_generation(2).expect("dispose generation 2");
}

#[test]
fn runtime_owner_concurrent_first_acquires_share_one_runtime() {
    let fixture = Fixture::new("runtime-owner-concurrent-acquire");
    let owner = Arc::new(fixture.runtime_owner());
    owner.activate_generation(1).expect("activate generation");
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let workers = (0..8)
        .map(|_| {
            let owner = Arc::clone(&owner);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                owner.acquire(1).expect("concurrent acquire")
            })
        })
        .collect::<Vec<_>>();
    let runtimes = workers
        .into_iter()
        .map(|worker| worker.join().expect("acquire worker"))
        .collect::<Vec<_>>();
    assert!(runtimes
        .iter()
        .all(|runtime| Arc::ptr_eq(&runtimes[0].inner, &runtime.inner)));
    owner
        .dispose_generation(1)
        .expect("dispose concurrent generation");
}

#[test]
fn runtime_owner_teardown_racing_first_acquire_never_leaves_a_live_generation() {
    let fixture = Fixture::new("runtime-owner-acquire-teardown-race");
    let owner = Arc::new(fixture.runtime_owner());
    owner.activate_generation(1).expect("activate generation");
    let barrier = Arc::new(std::sync::Barrier::new(3));

    let acquiring_owner = Arc::clone(&owner);
    let acquiring_barrier = Arc::clone(&barrier);
    let acquire = thread::spawn(move || {
        acquiring_barrier.wait();
        acquiring_owner.acquire(1)
    });

    let disposing_owner = Arc::clone(&owner);
    let disposing_barrier = Arc::clone(&barrier);
    let dispose = thread::spawn(move || {
        disposing_barrier.wait();
        disposing_owner.dispose_generation(1)
    });

    barrier.wait();
    let acquired = acquire.join().expect("acquire thread");
    dispose
        .join()
        .expect("dispose thread")
        .expect("teardown succeeds");
    assert_eq!(owner.current_generation(), None);
    assert!(!owner.is_initialized());

    if let Ok(runtime) = acquired {
        assert!(runtime
            .open_browse(open_request(&fixture))
            .expect_err("a clone returned during teardown is revoked")
            .contains("file_workspace_runtime_disposed"));
    }
}

fn platform() -> WorkspacePlatform {
    if cfg!(target_os = "windows") {
        WorkspacePlatform::Windows
    } else {
        WorkspacePlatform::Macos
    }
}

fn open_request(fixture: &Fixture) -> BrowseOpenRequest {
    BrowseOpenRequest {
        global_entry_id: None,
        platform: platform(),
        routing_hint: fixture.root.to_string_lossy().into_owned(),
        display_hint: Some("Integration fixture".to_string()),
    }
}

fn admit_managed_root(runtime: &FileWorkspaceRuntime, fixture: &Fixture) -> String {
    let admission = runtime
        .inner
        .database
        .admit_managed_scan(&ScanAdmissionOptions {
            request: ManagedScanRequest {
                roots: vec![fixture.root.to_string_lossy().into_owned()],
                request_key: Some("w2-r3-location-action".to_string()),
                dedupe: false,
            },
            run_id_override: None,
        })
        .expect("managed scan admission");
    admission
        .runs
        .first()
        .expect("managed scan run")
        .scan_root_id
        .clone()
}

#[derive(Default)]
struct RendererGate {
    state: Mutex<usize>,
    wake: Condvar,
}

impl RendererGate {
    fn mark_entered(&self) {
        let mut count = self.state.lock().expect("renderer gate lock");
        *count += 1;
        self.wake.notify_all();
    }

    fn count(&self) -> usize {
        *self.state.lock().expect("renderer gate lock")
    }

    fn wait_until_count(&self, expected: usize) {
        let mut count = self.state.lock().expect("renderer gate lock");
        while *count < expected {
            count = self.wake.wait(count).expect("renderer gate wait");
        }
    }
}

struct BlockingThumbnailRenderer {
    gate: Arc<RendererGate>,
}

impl ThumbnailRenderer for BlockingThumbnailRenderer {
    fn descriptor(&self) -> ThumbnailRendererDescriptor {
        ThumbnailRendererDescriptor::new(
            "test.blocking",
            "w1-10-integration-test-v1",
            ResourceHints {
                cpu: 1,
                io: 1,
                open_handles: 0,
                decoder: 0,
                native_preview: 0,
                provider_network: 0,
            },
        )
    }

    fn render(
        &self,
        _request: ThumbnailRenderRequest,
        context: &ThumbnailRenderContext,
    ) -> Result<ThumbnailRenderOutput, ThumbnailRendererError> {
        self.gate.mark_entered();
        while !context.is_explicitly_cancelled() {
            thread::yield_now();
        }
        Err(ThumbnailRendererError::Cancelled)
    }
}

fn assert_runtime_resources_are_empty(runtime: &FileWorkspaceRuntime) {
    let counts = runtime.resource_counts();
    assert_eq!(counts.browse_sessions, 0);
    assert_eq!(counts.change_monitors, 0);
    assert_eq!(counts.thumbnail_requests, 0);
    assert_eq!(counts.preview_sessions, 0);
    assert_eq!(counts.browse_service_sessions, 0);
}

#[test]
fn browse_restore_uses_fresh_refs_and_ephemeral_read_resolution() {
    let fixture = Fixture::new("fresh-refs");
    let runtime = fixture.runtime();
    let first = runtime
        .open_browse(open_request(&fixture))
        .expect("open browse");
    assert_eq!(
        first.location.kind,
        crate::file_workspace::LocationKind::Unknown
    );
    assert_eq!(
        first.location.availability,
        crate::file_workspace::LocationAvailability::Available
    );
    assert!(first.location.capabilities.can_browse);
    assert!(!first.location.capabilities.can_read_metadata);
    assert!(!first.location.capabilities.can_preview);
    assert!(!first.location.capabilities.can_watch);
    assert!(!first.location.capabilities.can_request_materialization);
    assert!(!first.location.capabilities.can_add_to_library);
    let first_page = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: first.session_id.clone(),
            request_id: "request-1".to_string(),
            path_ref: first.root_path_ref.clone(),
            page_size: 16,
            query: Default::default(),
        })
        .expect("first page");
    let entry = first_page.entries.first().expect("fixture entry");
    let source = match &entry.entry_ref {
        crate::file_workspace::BrowseEntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        } => PreviewSourceRef::Ephemeral {
            browse_session_id: browse_session_id.clone(),
            entry_id: entry_id.clone(),
        },
    };

    assert!(runtime.inner.browse.resolve_entry(&entry.entry_ref).is_ok());
    assert_ne!(
        runtime.inner.read_gate.content_read_eligibility(&source),
        crate::file_workspace::ContentReadEligibility::SourceUnavailable
    );

    let restored = runtime
        .restore_browse(BrowseRestoreRequest {
            locator: crate::file_workspace::WorkspaceRestoreLocator::Browse {
                platform: platform(),
                routing_hint: fixture.root.to_string_lossy().into_owned(),
                display_hint: Some("Restored fixture".to_string()),
            },
        })
        .expect("restore browse");
    assert_ne!(first.session_id, restored.session_id);
    assert_ne!(first.root_path_ref, restored.root_path_ref);

    runtime
        .dispose_browse(super::types::BrowseSessionRequest {
            session_id: first.session_id.clone(),
        })
        .expect("dispose first");
    assert!(runtime
        .inner
        .browse
        .resolve_entry(&entry.entry_ref)
        .is_err());
    runtime.dispose();
}

#[test]
fn location_browse_re_admits_managed_and_ephemeral_sources_with_fresh_refs() {
    let fixture = Fixture::new("location-action-fresh");
    let runtime = fixture.runtime();
    let scan_root_id = admit_managed_root(&runtime, &fixture);

    let managed = runtime
        .browse_location(LocationBrowseRequest {
            location: LocationRef::Managed {
                scan_root_id: scan_root_id.clone(),
            },
        })
        .expect("managed Location browse admission");
    assert_eq!(
        managed.location.kind,
        crate::file_workspace::LocationKind::Unknown
    );
    assert_eq!(
        managed.location.availability,
        crate::file_workspace::LocationAvailability::Available
    );
    assert_eq!(
        managed.location.freshness,
        crate::file_workspace::LocationFreshness::NotApplicable
    );
    assert!(managed.location.capabilities.can_browse);
    assert!(!managed.location.capabilities.can_read_metadata);
    assert!(!managed.location.capabilities.can_preview);
    assert!(!managed.location.capabilities.can_watch);
    assert!(!managed.location.capabilities.can_request_materialization);
    assert!(!managed.location.capabilities.can_add_to_library);

    let source = runtime
        .open_browse(open_request(&fixture))
        .expect("ephemeral source browse");
    let ephemeral = runtime
        .browse_location(LocationBrowseRequest {
            location: source.location.location_ref.clone(),
        })
        .expect("ephemeral Location browse admission");
    assert_ne!(source.session_id, ephemeral.session_id);
    assert_ne!(
        source.location.location_ref,
        ephemeral.location.location_ref
    );
    assert_ne!(source.root_path_ref, ephemeral.root_path_ref);
    assert_eq!(
        ephemeral.location.kind,
        crate::file_workspace::LocationKind::Unknown
    );
    assert_eq!(
        ephemeral.location.availability,
        crate::file_workspace::LocationAvailability::Available
    );
    assert!(ephemeral.location.capabilities.can_browse);
    assert!(!ephemeral.location.capabilities.can_read_metadata);

    let managed_projection = runtime
        .list_locations()
        .expect("location projection")
        .into_iter()
        .find(|descriptor| {
            descriptor.location_ref
                == LocationRef::Managed {
                    scan_root_id: scan_root_id.clone(),
                }
        })
        .expect("managed location projection");
    assert_eq!(
        managed_projection.freshness,
        crate::file_workspace::LocationFreshness::Reconciling
    );
    assert!(!managed_projection.capabilities.can_browse);

    assert!(
        serde_json::from_value::<LocationBrowseRequest>(serde_json::json!({
            "location": {
                "kind": "managed",
                "scanRootId": scan_root_id
            },
            "displayPath": "C:/renderer-controlled"
        }))
        .is_err()
    );

    runtime.dispose();
    assert_runtime_resources_are_empty(&runtime);
}

#[test]
fn list_locations_preserves_live_ephemeral_browse_admission_until_disposal() {
    let fixture = Fixture::new("location-list-live-ephemeral");
    let runtime = fixture.runtime();
    let opened = runtime
        .open_browse(open_request(&fixture))
        .expect("open Browse fixture");
    let expected_ref = opened.location.location_ref.clone();

    for _ in 0..3 {
        let relisted = runtime
            .list_locations()
            .expect("list live locations")
            .into_iter()
            .find(|descriptor| descriptor.location_ref == expected_ref)
            .expect("live ephemeral location remains listed");
        assert_eq!(relisted.kind, crate::file_workspace::LocationKind::Unknown);
        assert_eq!(
            relisted.availability,
            crate::file_workspace::LocationAvailability::Available
        );
        assert!(relisted.capabilities.can_browse);
        assert!(!relisted.capabilities.can_read_metadata);
        assert!(!relisted.capabilities.can_preview);
        assert!(!relisted.capabilities.can_watch);
        assert!(!relisted.capabilities.can_request_materialization);
        assert!(!relisted.capabilities.can_add_to_library);
    }

    runtime
        .dispose_browse(super::types::BrowseSessionRequest {
            session_id: opened.session_id.clone(),
        })
        .expect("dispose Browse fixture");
    assert!(runtime
        .list_locations()
        .expect("list locations after disposal")
        .into_iter()
        .all(|descriptor| descriptor.location_ref != expected_ref));
    assert_eq!(
        runtime.browse_location(LocationBrowseRequest {
            location: expected_ref,
        }),
        Err("workspace_location_ref_stale".to_string())
    );

    runtime.dispose();
    assert_runtime_resources_are_empty(&runtime);
}

#[test]
fn location_browse_rejects_unknown_forged_disposed_and_unavailable_refs() {
    let fixture = Fixture::new("location-action-invalid");
    let runtime = fixture.runtime();
    let unknown = runtime.browse_location(LocationBrowseRequest {
        location: LocationRef::Managed {
            scan_root_id: "unknown-scan-root".to_string(),
        },
    });
    assert_eq!(unknown, Err("workspace_location_ref_unknown".to_string()));

    let source = runtime
        .open_browse(open_request(&fixture))
        .expect("ephemeral source browse");
    let (browse_session_id, location_id) = match &source.location.location_ref {
        LocationRef::Ephemeral {
            browse_session_id,
            location_id,
        } => (browse_session_id.clone(), location_id.clone()),
        LocationRef::Managed { .. } => panic!("open Browse must publish an ephemeral LocationRef"),
    };
    let forged = runtime.browse_location(LocationBrowseRequest {
        location: LocationRef::Ephemeral {
            browse_session_id: browse_session_id.clone(),
            location_id: "forged-location-id".to_string(),
        },
    });
    assert_eq!(forged, Err("workspace_location_ref_mismatch".to_string()));

    let other_source = runtime
        .open_browse(open_request(&fixture))
        .expect("second ephemeral source browse");
    let cross_session = runtime.browse_location(LocationBrowseRequest {
        location: LocationRef::Ephemeral {
            browse_session_id: other_source.session_id.clone(),
            location_id: location_id.clone(),
        },
    });
    assert_eq!(
        cross_session,
        Err("workspace_location_ref_mismatch".to_string())
    );

    runtime
        .dispose_browse(super::types::BrowseSessionRequest {
            session_id: browse_session_id.clone(),
        })
        .expect("dispose source");
    let disposed = runtime.browse_location(LocationBrowseRequest {
        location: LocationRef::Ephemeral {
            browse_session_id,
            location_id,
        },
    });
    assert_eq!(disposed, Err("workspace_location_ref_stale".to_string()));

    runtime
        .dispose_browse(super::types::BrowseSessionRequest {
            session_id: other_source.session_id,
        })
        .expect("dispose second source");

    let unavailable_root = fixture.root.join("unavailable-source");
    fs::create_dir(&unavailable_root).expect("unavailable source root");
    let unavailable_source = runtime
        .open_browse(BrowseOpenRequest {
            global_entry_id: None,
            platform: platform(),
            routing_hint: unavailable_root.to_string_lossy().into_owned(),
            display_hint: Some("Unavailable source".to_string()),
        })
        .expect("second ephemeral source browse");
    let unavailable_location = unavailable_source.location.location_ref.clone();
    fs::remove_dir_all(&unavailable_root).expect("remove source root for unavailable evidence");
    let unavailable = runtime.browse_location(LocationBrowseRequest {
        location: unavailable_location,
    });
    assert_eq!(unavailable, Err("browse_directory_not_found".to_string()));

    runtime.dispose();
    assert_runtime_resources_are_empty(&runtime);
}

#[test]
fn managed_location_permission_state_blocks_action_without_path_fallback() {
    let fixture = Fixture::new("location-action-permission");
    let runtime = fixture.runtime();
    let scan_root_id = admit_managed_root(&runtime, &fixture);
    let connection = rusqlite::Connection::open(runtime.inner.database.path())
        .expect("direct test database connection");
    connection
        .execute(
            "UPDATE scan_roots SET health_status = 'permission_required' WHERE id = ?1",
            rusqlite::params![scan_root_id],
        )
        .expect("set permission health");

    let result = runtime.browse_location(LocationBrowseRequest {
        location: LocationRef::Managed { scan_root_id },
    });
    assert_eq!(
        result,
        Err("workspace_location_permission_denied".to_string())
    );
    runtime.dispose();
    assert_runtime_resources_are_empty(&runtime);
}

#[test]
fn integration_known_count_is_present_only_after_complete_enumeration() {
    let fixture = Fixture::new("known-count-parity");
    let runtime = fixture.runtime();
    let opened = runtime
        .open_browse(open_request(&fixture))
        .expect("open browse");
    let partial = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: opened.session_id.clone(),
            request_id: "known-count".to_string(),
            path_ref: opened.root_path_ref.clone(),
            page_size: 1,
            query: Default::default(),
        })
        .expect("partial page");
    assert_eq!(partial.completion, BrowseCompletionDto::Partial);
    assert_eq!(partial.known_count, None);

    let mut complete = partial;
    while complete.completion == BrowseCompletionDto::Partial {
        complete = runtime
            .next_page(super::types::BrowseNextPageRequest {
                session_id: opened.session_id.clone(),
                cursor: complete.next_cursor.take().expect("partial cursor"),
                page_size: 1,
            })
            .expect("next page");
        if complete.completion == BrowseCompletionDto::Partial {
            assert_eq!(complete.known_count, None);
        }
    }
    assert_eq!(complete.completion, BrowseCompletionDto::Complete);
    assert!(complete.known_count.is_some());
    runtime.dispose();
}

#[test]
fn progressive_pages_keep_prior_entry_refs_until_session_dispose() {
    let fixture = Fixture::new("progressive-page-ownership");
    let runtime = fixture.runtime();
    let opened = runtime
        .open_browse(open_request(&fixture))
        .expect("open browse");
    let first = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: opened.session_id.clone(),
            request_id: "page-ownership".to_string(),
            path_ref: opened.root_path_ref.clone(),
            page_size: 1,
            query: Default::default(),
        })
        .expect("first page");
    assert_eq!(first.completion, BrowseCompletionDto::Partial);
    let first_entry = first.entries.first().expect("first entry");
    let first_source = match &first_entry.entry_ref {
        crate::file_workspace::BrowseEntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        } => PreviewSourceRef::Ephemeral {
            browse_session_id: browse_session_id.clone(),
            entry_id: entry_id.clone(),
        },
    };
    let second = runtime
        .next_page(super::types::BrowseNextPageRequest {
            session_id: opened.session_id.clone(),
            cursor: first.next_cursor.clone().expect("first cursor"),
            page_size: 1,
        })
        .expect("second page");
    let second_entry = second.entries.first().expect("second entry");

    // The prior batch remains addressable after the next batch is published;
    // both entry resolution and the backend-only Read Gate still see it.
    assert!(runtime
        .inner
        .browse
        .resolve_entry(&first_entry.entry_ref)
        .is_ok());
    assert!(runtime
        .inner
        .browse
        .resolve_entry(&second_entry.entry_ref)
        .is_ok());
    assert_ne!(
        runtime
            .inner
            .read_gate
            .content_read_eligibility(&first_source),
        crate::file_workspace::ContentReadEligibility::SourceUnavailable
    );

    runtime
        .dispose_browse(super::types::BrowseSessionRequest {
            session_id: opened.session_id.clone(),
        })
        .expect("dispose browse");
    assert!(runtime
        .inner
        .browse
        .resolve_entry(&first_entry.entry_ref)
        .is_err());
    assert!(runtime
        .inner
        .browse
        .resolve_entry(&second_entry.entry_ref)
        .is_err());
    assert_eq!(
        runtime
            .inner
            .read_gate
            .content_read_eligibility(&first_source),
        crate::file_workspace::ContentReadEligibility::SourceUnavailable
    );
    runtime.dispose();
}

#[test]
fn retained_history_path_survives_page_and_enumeration_teardown() {
    let fixture = Fixture::new("history-path-retention");
    let runtime = fixture.runtime();
    let opened = runtime
        .open_browse(open_request(&fixture))
        .expect("open browse");
    let parent = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: opened.session_id.clone(),
            request_id: "parent".to_string(),
            path_ref: opened.root_path_ref.clone(),
            page_size: 8,
            query: Default::default(),
        })
        .expect("parent page");
    let child_path = parent
        .entries
        .iter()
        .find_map(|entry| entry.path_ref.clone())
        .expect("nested path");
    runtime
        .retain_path(BrowseRetainPathRequest {
            session_id: opened.session_id.clone(),
            path_ref: child_path.clone(),
        })
        .expect("retain history path");
    runtime
        .cancel_enumeration(BrowseCancelRequest {
            session_id: opened.session_id.clone(),
            enumeration: Some(crate::file_workspace::BrowseEnumerationRef {
                session_id: opened.session_id.clone(),
                request_id: parent.request_id.clone(),
                enumeration_id: parent.enumeration_id.clone(),
            }),
            request_id: None,
        })
        .expect("cancel parent enumeration");
    runtime
        .release_page(super::types::BrowseReleasePageRequest { page: parent })
        .expect("release parent page");

    runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: opened.session_id.clone(),
            request_id: "child".to_string(),
            path_ref: child_path,
            page_size: 8,
            query: Default::default(),
        })
        .expect("retained child path remains usable");
    runtime.dispose();
}

#[test]
fn browse_cancel_wire_requires_exactly_one_identity() {
    let enumeration = serde_json::json!({
        "sessionId": "session",
        "requestId": "request",
        "enumerationId": "enumeration"
    });
    let valid_enumeration = serde_json::json!({
        "sessionId": "session",
        "enumeration": enumeration
    });
    let valid_request = serde_json::json!({
        "sessionId": "session",
        "requestId": "request"
    });
    assert!(serde_json::from_value::<BrowseCancelRequest>(valid_enumeration).is_ok());
    assert!(serde_json::from_value::<BrowseCancelRequest>(valid_request).is_ok());
    assert!(
        serde_json::from_value::<BrowseCancelRequest>(serde_json::json!({
            "sessionId": "session"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<BrowseCancelRequest>(serde_json::json!({
            "sessionId": "session",
            "requestId": "request",
            "enumeration": {
                "sessionId": "session",
                "requestId": "request",
                "enumerationId": "enumeration"
            }
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<BrowseCancelRequest>(serde_json::json!({
            "sessionId": "session",
            "requestId": ""
        }))
        .is_err()
    );
}

#[test]
fn thumbnail_wire_does_not_accept_renderer_supplied_source_generation() {
    let valid = serde_json::json!({
        "requestId": "thumbnail-request",
        "source": {
            "kind": "ephemeral",
            "browseSessionId": "session",
            "entryId": "entry"
        },
        "variant": "small",
        "workClass": "interactive"
    });
    assert!(serde_json::from_value::<ThumbnailRequestDto>(valid.clone()).is_ok());
    assert!(
        serde_json::from_value::<ThumbnailRequestDto>(serde_json::json!({
            "requestId": "thumbnail-request",
            "source": {
                "kind": "ephemeral",
                "browseSessionId": "session",
                "entryId": "entry"
            },
            "variant": "small",
            "workClass": "interactive",
            "sourceGeneration": "caller-guessed"
        }))
        .is_err()
    );
}

#[test]
fn thumbnail_generation_is_derived_only_from_live_browse_entry_ownership() {
    let fixture = Fixture::new("thumbnail-generation-authority");
    let runtime = fixture.runtime();
    let opened = runtime
        .open_browse(open_request(&fixture))
        .expect("open browse");
    let first = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: opened.session_id.clone(),
            request_id: "generation-first".to_string(),
            path_ref: opened.root_path_ref.clone(),
            page_size: 1,
            query: Default::default(),
        })
        .expect("first page");
    let first_entry = first.entries[0].entry_ref.clone();
    assert_eq!(
        runtime.inner.browse.resolve_entry_generation(&first_entry),
        Ok(first.enumeration_id.clone())
    );

    let second = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: opened.session_id.clone(),
            request_id: "generation-second".to_string(),
            path_ref: opened.root_path_ref.clone(),
            page_size: 1,
            query: Default::default(),
        })
        .expect("superseding page");
    assert!(runtime
        .inner
        .browse
        .resolve_entry_generation(&first_entry)
        .is_err());

    let second_entry = second.entries[0].entry_ref.clone();
    runtime
        .release_page(super::types::BrowseReleasePageRequest { page: second })
        .expect("release page");
    assert!(runtime
        .inner
        .browse
        .resolve_entry_generation(&second_entry)
        .is_err());

    runtime.dispose();
    assert!(runtime
        .inner
        .browse
        .resolve_entry_generation(&second_entry)
        .is_err());
}

#[test]
fn integration_cancel_entrypoint_cancels_real_in_flight_browse_request() {
    let fixture = Fixture::new("browse-cancellation");
    let runtime = fixture.runtime();
    let opened = runtime
        .open_browse(open_request(&fixture))
        .expect("open browse");
    let gate = Arc::new(super::super::browse::TestPublishGate::default());
    runtime
        .inner
        .browse
        .set_test_publish_gate(Arc::clone(&gate));

    let worker_runtime = runtime.clone();
    let session_id = opened.session_id.clone();
    let path_ref = opened.root_path_ref.clone();
    let worker = thread::spawn(move || {
        worker_runtime.start_enumeration(BrowseStartEnumerationRequest {
            session_id,
            request_id: "in-flight-cancel".to_string(),
            path_ref,
            page_size: 1,
            query: Default::default(),
        })
    });
    gate.wait_until_reached();

    runtime
        .cancel_enumeration(BrowseCancelRequest {
            session_id: opened.session_id,
            enumeration: None,
            request_id: Some("in-flight-cancel".to_string()),
        })
        .expect("cancel pending enumeration");
    gate.release();

    assert_eq!(
        worker.join().expect("browse worker join"),
        Err("browse_cancelled".to_string())
    );
    runtime.dispose();
    assert_runtime_resources_are_empty(&runtime);
}

#[test]
fn change_monitor_and_preview_reuse_ephemeral_browse_refs() {
    let fixture = Fixture::new("change-preview");
    let runtime = fixture.runtime();
    let opened = runtime
        .open_browse(open_request(&fixture))
        .expect("open browse");
    let monitor = runtime
        .start_change_monitor(ChangeStartRequest {
            session_id: opened.session_id.clone(),
            path_ref: opened.root_path_ref.clone(),
        })
        .expect("start monitor");
    assert!(runtime
        .pending_change(ChangePendingRequest {
            monitor_id: monitor.monitor_id.clone(),
        })
        .expect("pending")
        .is_none());

    let entry_page = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: opened.session_id,
            request_id: "preview-request".to_string(),
            path_ref: opened.root_path_ref,
            page_size: 16,
            query: Default::default(),
        })
        .expect("entry page");
    let entry = entry_page.entries.first().expect("entry");
    let entry_kind = entry.kind;
    let source = match &entry.entry_ref {
        crate::file_workspace::BrowseEntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        } => PreviewSourceRef::Ephemeral {
            browse_session_id: browse_session_id.clone(),
            entry_id: entry_id.clone(),
        },
    };
    let read_eligibility = runtime.inner.read_gate.content_read_eligibility(&source);
    let preview = runtime
        .create_preview(PreviewCreateRequest {
            request_id: "preview-request".to_string(),
            source,
            host_kind: PreviewHostKind::ZenFloating,
        })
        .expect("create preview");
    let started = runtime
        .start_preview(PreviewSessionRequest {
            preview_id: preview.preview_id.clone(),
            native_presentation: None,
        })
        .expect("bounded text preview");
    assert_eq!(started.state, super::types::PreviewSessionStateDto::Ready);
    if read_eligibility == ContentReadEligibility::Eligible {
        assert!(matches!(
            started
                .representation
                .as_ref()
                .map(|value| &value.representation),
            Some(crate::file_workspace::PreviewRepresentation::Text { text, language })
                if text == "workspace integration" && language.is_none()
        ));
    } else {
        match entry_kind {
            super::types::BrowseEntryKindDto::Directory => assert!(matches!(
                started
                    .representation
                    .as_ref()
                    .map(|value| &value.representation),
                Some(crate::file_workspace::PreviewRepresentation::FolderSummary { .. })
            )),
            super::types::BrowseEntryKindDto::File => assert!(matches!(
                started
                    .representation
                    .as_ref()
                    .map(|value| &value.representation),
                Some(crate::file_workspace::PreviewRepresentation::Metadata { .. })
            )),
        }
    }
    runtime
        .dispose_change_monitor(ChangePendingRequest {
            monitor_id: monitor.monitor_id,
        })
        .expect("dispose monitor");
    runtime
        .dispose_preview(PreviewSessionRequest {
            preview_id: preview.preview_id,
            native_presentation: None,
        })
        .expect("dispose preview");
    runtime.dispose();
}

#[test]
fn preview_asset_transport_is_exactly_bound_and_revoked_by_runtime_lifecycle() {
    let fixture = Fixture::new("preview-asset-lifecycle");
    let runtime = fixture.runtime();
    let preview = runtime
        .create_preview(PreviewCreateRequest {
            request_id: "asset-request".to_string(),
            source: PreviewSourceRef::HostProvided {
                host_token: "host-token".to_string(),
            },
            host_kind: PreviewHostKind::ZenFloating,
        })
        .expect("create asset preview");
    let context = PreviewOperationContext::for_backend_content_read(
        preview.preview_id.clone(),
        "asset-request",
        "asset-version",
        PreviewCancellation::default(),
        Instant::now() + Duration::from_secs(5),
    );
    let token = runtime
        .inner
        .preview_assets
        .publish_asset(&context, "image/png", vec![1, 2, 3])
        .expect("publish bounded preview asset");
    let artifact = runtime
        .request_preview_asset(PreviewAssetRequestDto {
            preview_id: preview.preview_id.clone(),
            request_id: "asset-request".to_string(),
            source_version: "asset-version".to_string(),
            asset_token: token.clone(),
            offset_bytes: None,
            max_bytes: None,
        })
        .expect("read exact preview asset");
    assert_eq!(artifact.media_type, "image/png");
    assert_eq!(artifact.bytes, vec![1, 2, 3]);
    assert!(runtime
        .request_preview_asset(PreviewAssetRequestDto {
            preview_id: preview.preview_id.clone(),
            request_id: "wrong-request".to_string(),
            source_version: "asset-version".to_string(),
            asset_token: token.clone(),
            offset_bytes: None,
            max_bytes: None,
        })
        .is_err());
    assert!(runtime
        .request_preview_asset(PreviewAssetRequestDto {
            preview_id: preview.preview_id.clone(),
            request_id: "asset-request".to_string(),
            source_version: "wrong-version".to_string(),
            asset_token: token.clone(),
            offset_bytes: None,
            max_bytes: None,
        })
        .is_err());

    assert!(runtime
        .cancel_preview(PreviewSessionRequest {
            preview_id: preview.preview_id.clone(),
            native_presentation: None,
        })
        .expect("cancel preview asset owner"));
    assert_eq!(runtime.inner.preview_assets.counts(), (0, 0));
    assert!(runtime
        .request_preview_asset(PreviewAssetRequestDto {
            preview_id: preview.preview_id,
            request_id: "asset-request".to_string(),
            source_version: "asset-version".to_string(),
            asset_token: token,
            offset_bytes: None,
            max_bytes: None,
        })
        .is_err());

    let disposed_preview = runtime
        .create_preview(PreviewCreateRequest {
            request_id: "asset-dispose-request".to_string(),
            source: PreviewSourceRef::HostProvided {
                host_token: "host-token-2".to_string(),
            },
            host_kind: PreviewHostKind::ZenPinned,
        })
        .expect("create disposable asset preview");
    let disposed_context = PreviewOperationContext::for_backend_content_read(
        disposed_preview.preview_id.clone(),
        "asset-dispose-request",
        "asset-dispose-version",
        PreviewCancellation::default(),
        Instant::now() + Duration::from_secs(5),
    );
    runtime
        .inner
        .preview_assets
        .publish_asset(&disposed_context, "image/png", vec![9])
        .expect("publish disposable preview asset");
    assert!(runtime
        .dispose_preview(PreviewSessionRequest {
            preview_id: disposed_preview.preview_id,
            native_presentation: None,
        })
        .expect("dispose preview asset owner"));
    assert_eq!(runtime.inner.preview_assets.counts(), (0, 0));

    runtime.dispose();
    assert_runtime_resources_are_empty(&runtime);
    assert_eq!(
        runtime
            .inner
            .preview_assets
            .publish_asset(&context, "image/png", vec![4]),
        Err(crate::file_workspace::PreviewAssetError::Disposed)
    );
}

#[test]
fn runtime_owned_resources_return_to_steady_state_after_repeated_target_teardown() {
    let fixture = Fixture::new("bounded-target-lifecycle");
    let runtime = fixture.runtime();

    for index in 0..8 {
        let opened = runtime
            .open_browse(open_request(&fixture))
            .expect("open browse target");
        let page = runtime
            .start_enumeration(BrowseStartEnumerationRequest {
                session_id: opened.session_id.clone(),
                request_id: format!("lifecycle-{index}"),
                path_ref: opened.root_path_ref.clone(),
                page_size: 16,
                query: Default::default(),
            })
            .expect("browse page");
        let entry = page
            .entries
            .iter()
            .find(|entry| entry.kind == super::types::BrowseEntryKindDto::File)
            .expect("file entry");
        let source = match &entry.entry_ref {
            crate::file_workspace::BrowseEntryRef::Ephemeral {
                browse_session_id,
                entry_id,
            } => PreviewSourceRef::Ephemeral {
                browse_session_id: browse_session_id.clone(),
                entry_id: entry_id.clone(),
            },
        };
        runtime
            .start_change_monitor(ChangeStartRequest {
                session_id: opened.session_id.clone(),
                path_ref: opened.root_path_ref.clone(),
            })
            .expect("change monitor");
        runtime
            .create_preview(PreviewCreateRequest {
                request_id: format!("preview-{index}"),
                source,
                host_kind: PreviewHostKind::ZenFloating,
            })
            .expect("preview session");

        let counts = runtime.resource_counts();
        assert!(counts.browse_sessions <= 1);
        assert!(counts.browse_service_sessions <= 1);
        assert!(counts.change_monitors <= 1);
        assert!(counts.preview_sessions <= 1);
        assert!(counts.thumbnail_requests <= 1);

        runtime
            .dispose_browse(super::types::BrowseSessionRequest {
                session_id: opened.session_id,
            })
            .expect("dispose target");
        assert_runtime_resources_are_empty(&runtime);
    }

    runtime.dispose();
    assert_runtime_resources_are_empty(&runtime);
}

#[test]
fn change_hint_invalidates_old_page_and_refreshes_through_browse_service() {
    let fixture = Fixture::new("change-refresh");
    let runtime = fixture.runtime();
    let opened = runtime
        .open_browse(open_request(&fixture))
        .expect("open browse");
    let page = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: opened.session_id.clone(),
            request_id: "before-change".to_string(),
            path_ref: opened.root_path_ref.clone(),
            page_size: 1,
            query: Default::default(),
        })
        .expect("page before change");
    let monitor = runtime
        .start_change_monitor(ChangeStartRequest {
            session_id: opened.session_id.clone(),
            path_ref: opened.root_path_ref,
        })
        .expect("monitor");
    let monitor_handle = runtime
        .inner
        .monitors
        .lock()
        .expect("monitor registry")
        .get(&monitor.monitor_id)
        .expect("monitor handle")
        .monitor
        .clone();
    monitor_handle.inject_hint_for_integration_test(
        crate::file_workspace::change::EphemeralChangeKind::ContentChanged,
    );
    let pending = runtime
        .pending_change(ChangePendingRequest {
            monitor_id: monitor.monitor_id.clone(),
        })
        .expect("pending change")
        .expect("coalesced hint");
    let serialized = serde_json::to_string(&pending).expect("pending JSON");
    assert!(!serialized.contains("path"));
    let cursor = page.next_cursor.expect("cursor before change");
    let old_page_error = runtime
        .next_page(super::types::BrowseNextPageRequest {
            session_id: opened.session_id.clone(),
            cursor,
            page_size: 1,
        })
        .expect_err("old page must be stale");
    assert_eq!(old_page_error, "browse_enumeration_stale");
    let refreshed = runtime
        .refresh_change(super::types::ChangeRefreshRequest {
            monitor_id: monitor.monitor_id.clone(),
            request_id: "after-change".to_string(),
            page_size: 1,
            query: Default::default(),
        })
        .expect("fresh page");
    assert_eq!(refreshed.request_id, "after-change");
    runtime
        .cancel_enumeration(BrowseCancelRequest {
            session_id: refreshed.session_id.clone(),
            enumeration: Some(crate::file_workspace::BrowseEnumerationRef {
                session_id: refreshed.session_id.clone(),
                request_id: refreshed.request_id.clone(),
                enumeration_id: refreshed.enumeration_id.clone(),
            }),
            request_id: None,
        })
        .expect("cancel refreshed enumeration");
    runtime
        .dispose_change_monitor(ChangePendingRequest {
            monitor_id: monitor.monitor_id,
        })
        .expect("dispose monitor");
    runtime.dispose();
}

#[test]
fn real_filesystem_mutation_burst_refreshes_without_publishing_stale_pages() {
    let fixture = Fixture::new("real-change-burst");
    let runtime = fixture.runtime();
    let opened = runtime
        .open_browse(open_request(&fixture))
        .expect("open browse");
    let before = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: opened.session_id.clone(),
            request_id: "real-burst-before".to_string(),
            path_ref: opened.root_path_ref.clone(),
            page_size: 1,
            query: Default::default(),
        })
        .expect("page before real filesystem mutation");
    let monitor = runtime
        .start_change_monitor(ChangeStartRequest {
            session_id: opened.session_id.clone(),
            path_ref: opened.root_path_ref.clone(),
        })
        .expect("start monitor");
    let monitor_handle = runtime
        .inner
        .monitors
        .lock()
        .expect("monitor registry")
        .get(&monitor.monitor_id)
        .expect("monitor handle")
        .monitor
        .clone();

    let original = fixture.root.join("entry.txt");
    let renamed = fixture.root.join("entry-renamed.txt");
    std::fs::rename(&original, &renamed).expect("rename real fixture entry");
    std::fs::write(fixture.root.join("burst-0.txt"), b"created").expect("create burst entry");
    std::fs::write(fixture.root.join("burst-1.txt"), b"created").expect("create burst entry");
    std::fs::write(fixture.root.join("burst-2.txt"), b"created").expect("create burst entry");
    std::fs::remove_file(fixture.root.join("burst-1.txt")).expect("delete burst entry");

    // The OS watcher is real, while the deterministic injected hint makes the
    // refresh boundary stable across supported runners. The refresh itself
    // rereads the mutated filesystem through BrowseService.
    monitor_handle.inject_hint_for_integration_test(
        crate::file_workspace::change::EphemeralChangeKind::Renamed,
    );
    let _pending = runtime
        .pending_change(ChangePendingRequest {
            monitor_id: monitor.monitor_id.clone(),
        })
        .expect("pending real filesystem burst")
        .expect("real filesystem burst should request refresh");

    let old_cursor = before.next_cursor.expect("cursor before real burst");
    assert_eq!(
        runtime
            .next_page(super::types::BrowseNextPageRequest {
                session_id: opened.session_id.clone(),
                cursor: old_cursor,
                page_size: 1,
            })
            .expect_err("old page must be stale after real mutation burst"),
        "browse_enumeration_stale"
    );

    let mut refreshed = runtime
        .refresh_change(super::types::ChangeRefreshRequest {
            monitor_id: monitor.monitor_id.clone(),
            request_id: "real-burst-after".to_string(),
            page_size: 32,
            query: Default::default(),
        })
        .expect("refresh mutated filesystem");
    let mut names = refreshed
        .entries
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();
    while let Some(cursor) = refreshed.next_cursor.take() {
        refreshed = runtime
            .next_page(super::types::BrowseNextPageRequest {
                session_id: refreshed.session_id.clone(),
                cursor,
                page_size: 32,
            })
            .expect("continue refreshed real filesystem enumeration");
        names.extend(refreshed.entries.iter().map(|entry| entry.name.clone()));
    }
    assert!(names.iter().any(|name| name == "entry-renamed.txt"));
    assert!(!names.iter().any(|name| name == "entry.txt"));
    assert!(names.iter().any(|name| name == "burst-0.txt"));
    assert!(names.iter().any(|name| name == "burst-2.txt"));
    assert!(!names.iter().any(|name| name == "burst-1.txt"));

    runtime
        .dispose_change_monitor(ChangePendingRequest {
            monitor_id: monitor.monitor_id,
        })
        .expect("dispose real mutation monitor");
    runtime.dispose();
    assert_runtime_resources_are_empty(&runtime);
}

#[test]
fn thumbnail_registry_reserves_before_service_and_cancels_reserved_running_and_completed() {
    let fixture = Fixture::new("thumbnail-registry-race");
    let renderer_gate = Arc::new(RendererGate::default());
    let runtime = fixture.runtime_with_renderer(Arc::new(BlockingThumbnailRenderer {
        gate: Arc::clone(&renderer_gate),
    }));
    let opened = runtime
        .open_browse(open_request(&fixture))
        .expect("open browse");
    let page = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: opened.session_id.clone(),
            request_id: "thumbnail-source".to_string(),
            path_ref: opened.root_path_ref.clone(),
            page_size: 16,
            query: Default::default(),
        })
        .expect("source page");
    let source = page
        .entries
        .iter()
        .find(|entry| entry.kind == super::types::BrowseEntryKindDto::File)
        .expect("file source entry")
        .entry_ref
        .clone();
    let request_for = move |request_id: &str| ThumbnailRequestDto {
        request_id: request_id.to_string(),
        source: source.clone().into(),
        variant: ThumbnailVariantDto::Small,
        work_class: WorkClass::Interactive,
        session_id: Some(opened.session_id.clone()),
    };

    let reservation_gate = Arc::new(super::runtime::ThumbnailReservationGate::default());
    runtime.set_thumbnail_reservation_gate(Arc::clone(&reservation_gate));
    let first_id = "thumbnail-reserved";
    let first_runtime = runtime.clone();
    let first_request = request_for(first_id);
    let first_worker = thread::spawn(move || first_runtime.request_thumbnail(first_request));
    reservation_gate.wait_until_reached();

    assert_eq!(
        runtime
            .request_thumbnail(request_for(first_id))
            .expect_err("duplicate request must be rejected"),
        "thumbnail_request_in_flight"
    );
    assert_eq!(
        runtime.resource_counts().thumbnail_requests,
        1,
        "duplicate must not create a second owner"
    );
    assert!(runtime
        .cancel_thumbnail(ThumbnailCancelRequest {
            request_id: first_id.to_string(),
        })
        .expect("cancel reserved request"));
    assert!(!runtime
        .cancel_thumbnail(ThumbnailCancelRequest {
            request_id: first_id.to_string(),
        })
        .expect("duplicate reserved cancel"));
    reservation_gate.release();
    assert_eq!(
        first_worker.join().expect("reserved worker join"),
        Err("thumbnail_request_cancelled".to_string())
    );
    assert!(!runtime
        .cancel_thumbnail(ThumbnailCancelRequest {
            request_id: first_id.to_string(),
        })
        .expect("completed reserved cancel"));

    let second_id = "thumbnail-running";
    let renderer_count_before = renderer_gate.count();
    let second_runtime = runtime.clone();
    let second_worker =
        thread::spawn(move || second_runtime.request_thumbnail(request_for(second_id)));
    renderer_gate.wait_until_count(renderer_count_before + 1);
    assert!(runtime
        .cancel_thumbnail(ThumbnailCancelRequest {
            request_id: second_id.to_string(),
        })
        .expect("cancel running request"));
    assert!(!runtime
        .cancel_thumbnail(ThumbnailCancelRequest {
            request_id: second_id.to_string(),
        })
        .expect("duplicate running cancel"));
    assert_eq!(
        second_worker.join().expect("running worker join"),
        Err("thumbnail_request_cancelled".to_string())
    );
    assert!(!runtime
        .cancel_thumbnail(ThumbnailCancelRequest {
            request_id: second_id.to_string(),
        })
        .expect("completed running cancel"));

    for _ in 0..1000 {
        if runtime.inner.thumbnail.active_request_count() == 0 {
            break;
        }
        thread::yield_now();
    }
    assert_eq!(runtime.inner.thumbnail.active_request_count(), 0);
    runtime.dispose();
    assert_runtime_resources_are_empty(&runtime);
}

#[test]
fn thumbnail_ipc_response_is_bounded_binary_and_path_free() {
    let payload =
        super::types::encode_thumbnail_ipc_response(&super::types::ThumbnailArtifactDto {
            cache_key: "logical-cache-key".to_string(),
            bytes: vec![1, 2, 3, 4],
        })
        .expect("binary thumbnail payload");
    assert_eq!(&payload[..4], b"ZCTH");
    assert_eq!(payload[4], 1);
    assert!(!payload.starts_with(b"{"));
    assert!(payload
        .windows(b"logical-cache-key".len())
        .any(|window| { window == b"logical-cache-key" }));
    assert!(
        super::types::encode_thumbnail_ipc_response(&super::types::ThumbnailArtifactDto {
            cache_key: "key".to_string(),
            bytes: vec![0; super::types::THUMBNAIL_IPC_MAX_BYTES + 1],
        })
        .is_err()
    );
}

#[test]
fn preview_asset_ipc_response_is_bounded_binary_and_path_free() {
    let payload =
        super::types::encode_preview_asset_ipc_response(&super::types::PreviewAssetArtifactDto {
            media_type: "image/png".to_string(),
            bytes: vec![1, 2, 3, 4],
        })
        .expect("binary preview asset payload");
    assert_eq!(&payload[..4], b"ZCAS");
    assert_eq!(payload[4], 1);
    assert!(!payload.starts_with(b"{"));
    assert!(payload
        .windows(b"image/png".len())
        .any(|window| window == b"image/png"));
    assert!(!payload
        .windows(b"C:\\secret".len())
        .any(|window| window == b"C:\\secret"));
    assert!(super::types::encode_preview_asset_ipc_response(
        &super::types::PreviewAssetArtifactDto {
            media_type: "image/png".to_string(),
            bytes: vec![0; super::types::PREVIEW_ASSET_IPC_MAX_BYTES + 1],
        }
    )
    .is_err());
    assert!(super::types::encode_preview_asset_ipc_response(
        &super::types::PreviewAssetArtifactDto {
            media_type: "image/\n".to_string(),
            bytes: vec![1],
        }
    )
    .is_err());
}

#[cfg(not(target_os = "macos"))]
#[test]
fn shared_thumbnail_surface_is_explicitly_unsupported_without_native_renderer() {
    let fixture = Fixture::new("thumbnail-unsupported");
    let runtime = fixture.runtime();
    let opened = runtime
        .open_browse(open_request(&fixture))
        .expect("open browse");
    let page = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: opened.session_id.clone(),
            request_id: "thumbnail-request".to_string(),
            path_ref: opened.root_path_ref,
            page_size: 16,
            query: Default::default(),
        })
        .expect("page");
    let entry = page.entries.first().expect("entry");
    let error = runtime
        .request_thumbnail(ThumbnailRequestDto {
            request_id: "thumbnail-request".to_string(),
            source: entry.entry_ref.clone().into(),
            variant: ThumbnailVariantDto::Small,
            work_class: WorkClass::Interactive,
            session_id: Some(opened.session_id),
        })
        .expect_err("non-mac renderer must fail closed");
    assert_eq!(error, "thumbnail_renderer_unsupported");
    runtime.dispose();
}
