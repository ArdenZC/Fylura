use super::{
    runtime::FileWorkspaceRuntime,
    types::{
        BrowseOpenRequest, BrowseSessionRequest, PreviewAssetRequestDto, PreviewSessionRequest,
        PreviewSwitchSourceRequest,
    },
};
use crate::{
    db::Database,
    file_workspace::{
        contracts::{
            ContentReadEligibility, MaterializationState, PreviewHostKind, PreviewSourceRef,
            WorkspacePlatform,
        },
        preview::{
            PreparedPreview, PreviewAssetError, PreviewCompleteness, PreviewOperationContext,
            PreviewProvider, PreviewProviderDescriptor, PreviewProviderEnvironment,
            PreviewProviderError, PreviewProviderRegistry, PreviewProviderResult,
            PreviewRepresentation, PreviewResolveRequest, PreviewSession, PreviewSessionConfig,
            PreviewTask, ProviderProbe, SourceResolveError, SourceResolver,
        },
        preview_asset::{PreviewAssetPublishGate, PreviewAssetRevokeGate},
        PreviewCapabilities, PreviewHost, PreviewSourceSnapshot,
    },
    platform::macos::quick_look::MacThumbnailService,
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
            .join(format!(
                "w3-01-preview-lifecycle-{name}-{}",
                uuid::Uuid::new_v4()
            ));
        fs::create_dir_all(&root).expect("fixture root");
        fs::write(root.join("entry.txt"), b"preview lifecycle fixture").expect("fixture file");
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
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Debug, Default)]
struct RaceState {
    first_token: Option<String>,
    second_result: Option<Result<String, PreviewAssetError>>,
    release_second: bool,
}

#[derive(Debug, Default)]
struct RaceGate {
    state: Mutex<RaceState>,
    wake: Condvar,
}

impl RaceGate {
    fn first_published(&self, token: String) {
        let mut state = self.state.lock().expect("race state lock");
        state.first_token = Some(token);
        self.wake.notify_all();
    }

    fn wait_for_first(&self) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut state = self.state.lock().expect("race state lock");
        while state.first_token.is_none() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "preview provider did not publish first asset"
            );
            let (next, timeout) = self
                .wake
                .wait_timeout(state, remaining)
                .expect("race state wait");
            state = next;
            assert!(
                !timeout.timed_out(),
                "preview provider did not publish first asset"
            );
        }
        state.first_token.clone().expect("first asset token")
    }

    fn release_second(&self) {
        let mut state = self.state.lock().expect("race state lock");
        state.release_second = true;
        self.wake.notify_all();
    }

    fn wait_for_second_release(&self) {
        let mut state = self.state.lock().expect("race state lock");
        while !state.release_second {
            state = self.wake.wait(state).expect("race state wait");
        }
    }

    fn second_published(&self, result: Result<String, PreviewAssetError>) {
        let mut state = self.state.lock().expect("race state lock");
        state.second_result = Some(result);
        self.wake.notify_all();
    }

    fn wait_for_second(&self) -> Result<String, PreviewAssetError> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut state = self.state.lock().expect("race state lock");
        while state.second_result.is_none() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "preview provider did not attempt second asset"
            );
            let (next, timeout) = self
                .wake
                .wait_timeout(state, remaining)
                .expect("race state wait");
            state = next;
            assert!(
                !timeout.timed_out(),
                "preview provider did not attempt second asset"
            );
        }
        state.second_result.clone().expect("second asset result")
    }
}

struct RacePrepared {
    gate: Arc<RaceGate>,
}

impl PreparedPreview for RacePrepared {
    fn load(
        &mut self,
        context: &PreviewOperationContext,
        environment: PreviewProviderEnvironment<'_>,
    ) -> Result<crate::file_workspace::PreviewProviderResult, PreviewProviderError> {
        let publisher = environment
            .asset_publisher
            .ok_or(PreviewProviderError::Failed)?;
        let first = publisher
            .publish_asset(context, "image/png", b"old-asset".to_vec())
            .map_err(|_| PreviewProviderError::Failed)?;
        self.gate.first_published(first);
        self.gate.wait_for_second_release();
        let second = publisher.publish_asset(context, "image/png", b"late-asset".to_vec());
        self.gate.second_published(second);
        Err(PreviewProviderError::Cancelled)
    }

    fn cleanup(&mut self) {}
}

struct RaceProvider {
    descriptor: PreviewProviderDescriptor,
    gate: Arc<RaceGate>,
}

impl PreviewProvider for RaceProvider {
    fn descriptor(&self) -> &PreviewProviderDescriptor {
        &self.descriptor
    }

    fn probe(
        &self,
        _snapshot: &PreviewSourceSnapshot,
        _context: &PreviewOperationContext,
    ) -> ProviderProbe {
        ProviderProbe::Compatible
    }

    fn prepare(
        &self,
        _snapshot: &PreviewSourceSnapshot,
        _context: &PreviewOperationContext,
    ) -> Result<Box<dyn PreparedPreview>, PreviewProviderError> {
        Ok(Box::new(RacePrepared {
            gate: Arc::clone(&self.gate),
        }))
    }
}

struct OneShotAssetPrepared {
    gate: Arc<RaceGate>,
}

impl PreparedPreview for OneShotAssetPrepared {
    fn load(
        &mut self,
        context: &PreviewOperationContext,
        environment: PreviewProviderEnvironment<'_>,
    ) -> Result<PreviewProviderResult, PreviewProviderError> {
        let publisher = environment
            .asset_publisher
            .ok_or(PreviewProviderError::Failed)?;
        let token = publisher
            .publish_asset(context, "image/png", b"new-asset".to_vec())
            .map_err(|_| PreviewProviderError::Failed)?;
        self.gate.first_published(token);
        Ok(PreviewProviderResult {
            representation: PreviewRepresentation::Text {
                text: "new preview".to_string(),
                language: Some("text".to_string()),
            },
            completeness: PreviewCompleteness::Complete,
            warnings: Vec::new(),
        })
    }

    fn cleanup(&mut self) {}
}

struct OneShotAssetProvider {
    descriptor: PreviewProviderDescriptor,
    gate: Arc<RaceGate>,
}

impl PreviewProvider for OneShotAssetProvider {
    fn descriptor(&self) -> &PreviewProviderDescriptor {
        &self.descriptor
    }

    fn probe(
        &self,
        _snapshot: &PreviewSourceSnapshot,
        _context: &PreviewOperationContext,
    ) -> ProviderProbe {
        ProviderProbe::Compatible
    }

    fn prepare(
        &self,
        _snapshot: &PreviewSourceSnapshot,
        _context: &PreviewOperationContext,
    ) -> Result<Box<dyn PreparedPreview>, PreviewProviderError> {
        Ok(Box::new(OneShotAssetPrepared {
            gate: Arc::clone(&self.gate),
        }))
    }
}

struct StaticResolver {
    snapshot: PreviewSourceSnapshot,
}

impl SourceResolver for StaticResolver {
    fn resolve(
        &self,
        request: &PreviewResolveRequest,
        context: &PreviewOperationContext,
    ) -> Result<PreviewSourceSnapshot, SourceResolveError> {
        context.ensure_active().map_err(|error| match error {
            crate::file_workspace::PreviewContextError::Cancelled
            | crate::file_workspace::PreviewContextError::StalePublication => {
                SourceResolveError::Cancelled
            }
            crate::file_workspace::PreviewContextError::TimedOut => SourceResolveError::Timeout,
        })?;
        if request.source != self.snapshot.source {
            return Err(SourceResolveError::SourceMismatch);
        }
        Ok(self.snapshot.clone())
    }
}

fn platform() -> WorkspacePlatform {
    if cfg!(target_os = "windows") {
        WorkspacePlatform::Windows
    } else {
        WorkspacePlatform::Macos
    }
}

fn source(browse_session_id: &str, entry_id: &str) -> PreviewSourceRef {
    PreviewSourceRef::Ephemeral {
        browse_session_id: browse_session_id.to_string(),
        entry_id: entry_id.to_string(),
    }
}

fn snapshot(source: PreviewSourceRef) -> PreviewSourceSnapshot {
    PreviewSourceSnapshot::new(
        source,
        "preview-race-source-version",
        crate::file_workspace::PreviewMetadata {
            display_name: "race.txt".to_string(),
            media_type: Some("text/plain".to_string()),
            extension: Some("txt".to_string()),
            size_bytes: Some(9),
            modified_at_epoch_ms: Some(1),
            materialization: MaterializationState::Local,
            read_eligibility: ContentReadEligibility::MetadataOnly,
        },
        PreviewCapabilities::all(),
    )
}

fn start_race_preview(
    runtime: &FileWorkspaceRuntime,
    preview_id: &str,
    source: PreviewSourceRef,
    gate: Arc<RaceGate>,
) -> (PreviewSession, PreviewTask, String) {
    let session = PreviewSession::new(PreviewSessionConfig::new(
        preview_id,
        "old-request",
        source.clone(),
        PreviewHost::new(PreviewHostKind::ZenFloating, PreviewCapabilities::all()),
    ));
    runtime
        .inner
        .preview_sessions
        .lock()
        .expect("preview session map")
        .insert(preview_id.to_string(), session.clone());

    let provider = Arc::new(RaceProvider {
        descriptor: PreviewProviderDescriptor::new(
            "test.preview-race",
            100,
            PreviewCapabilities::all(),
            vec![PreviewHostKind::ZenFloating],
            false,
        ),
        gate: Arc::clone(&gate),
    });
    let providers: Vec<Arc<dyn PreviewProvider>> = vec![provider];
    let registry = Arc::new(PreviewProviderRegistry::new(providers).expect("provider registry"));
    let task = session
        .start_with_environment(
            Arc::new(StaticResolver {
                snapshot: snapshot(source),
            }),
            registry,
            crate::file_workspace::PreviewProviderEnvironmentHandle::with_asset_publisher(
                runtime.inner.preview_assets.clone(),
            ),
        )
        .expect("preview task starts");
    let asset_token = gate.wait_for_first();
    (session, task, asset_token)
}

fn run_lifecycle_race(action: LifecycleAction) {
    let fixture = Fixture::new(action.name());
    let runtime = fixture.runtime();
    let preview_id = format!("preview-race-{}", action.name());
    let (browse_session_id, preview_source) = if action == LifecycleAction::BrowseTeardown {
        let opened = runtime
            .open_browse(BrowseOpenRequest {
                global_entry_id: None,
                platform: platform(),
                routing_hint: fixture.root.to_string_lossy().into_owned(),
                display_hint: Some("Preview race fixture".to_string()),
            })
            .expect("browse session opens");
        (
            opened.session_id.clone(),
            source(&opened.session_id, "synthetic-entry"),
        )
    } else {
        (
            "browse-race".to_string(),
            source("browse-race", "synthetic-entry"),
        )
    };

    let revoke_gate = Arc::new(PreviewAssetRevokeGate::default());
    runtime
        .inner
        .preview_assets
        .set_revoke_gate_for_test(Some(Arc::clone(&revoke_gate)));
    let race_gate = Arc::new(RaceGate::default());
    let (session, task, old_asset_token) = start_race_preview(
        &runtime,
        &preview_id,
        preview_source,
        Arc::clone(&race_gate),
    );
    let old_publication = session
        .current_publication()
        .expect("session-issued publication token");
    assert_eq!(old_publication.request_id(), "old-request");
    assert_eq!(
        old_publication.source_version(),
        Some("preview-race-source-version")
    );

    let control_runtime = runtime.clone();
    let control_preview_id = preview_id.clone();
    let control_browse_session_id = browse_session_id.clone();
    let control = thread::spawn(move || match action {
        LifecycleAction::Cancel => control_runtime
            .cancel_preview(PreviewSessionRequest {
                preview_id: control_preview_id,
                native_presentation: None,
            })
            .map(|_| ()),
        LifecycleAction::Switch => control_runtime
            .switch_preview_source(PreviewSwitchSourceRequest {
                preview_id: control_preview_id,
                request_id: "new-request".to_string(),
                source: source("new-browse", "new-entry"),
            })
            .map(|_| ()),
        LifecycleAction::Dispose => control_runtime
            .dispose_preview(PreviewSessionRequest {
                preview_id: control_preview_id,
                native_presentation: None,
            })
            .map(|_| ()),
        LifecycleAction::BrowseTeardown => control_runtime
            .dispose_browse(BrowseSessionRequest {
                session_id: control_browse_session_id,
            })
            .map(|_| ()),
    });

    revoke_gate.wait_until_entered();
    let authority_revoked_before_revoke_returns = match action {
        LifecycleAction::Switch => !old_publication.is_current(),
        LifecycleAction::Cancel | LifecycleAction::Dispose | LifecycleAction::BrowseTeardown => {
            session.current_publication().is_none()
        }
    };
    let lifecycle_state_before_revoke_returns = session.state();
    let switched_request_before_revoke_returns = session.request().request_id;

    race_gate.release_second();
    let second_result = race_gate.wait_for_second();
    let old_asset_read_after_registry_clear =
        runtime.request_preview_asset(PreviewAssetRequestDto {
            preview_id: preview_id.clone(),
            request_id: "old-request".to_string(),
            source_version: "preview-race-source-version".to_string(),
            asset_token: old_asset_token.clone(),
            offset_bytes: None,
            max_bytes: None,
        });
    revoke_gate.release();
    let control_result = control.join().expect("lifecycle control thread");
    runtime.inner.preview_assets.set_revoke_gate_for_test(None);
    let task_result = task.join();

    assert!(
        control_result.is_ok(),
        "lifecycle control failed: {control_result:?}"
    );
    assert!(
        authority_revoked_before_revoke_returns,
        "{action:?} kept old publication authority while asset revoke was paused"
    );
    assert!(
        second_result.is_err(),
        "{action:?} allowed a late asset publication: {second_result:?}"
    );
    assert!(
        old_asset_read_after_registry_clear.is_err(),
        "{action:?} retained the old exact-token asset"
    );
    assert_eq!(runtime.inner.preview_assets.counts(), (0, 0));
    assert!(matches!(
        task_result,
        Err(crate::file_workspace::PreviewRunError::Cancelled)
            | Err(crate::file_workspace::PreviewRunError::StalePublication)
    ));
    match action {
        LifecycleAction::Switch => {
            assert_eq!(
                lifecycle_state_before_revoke_returns,
                crate::file_workspace::PreviewSessionState::Resolving
            );
            assert_eq!(switched_request_before_revoke_returns, "new-request");
            assert_eq!(session.request().request_id, "new-request");
        }
        LifecycleAction::Cancel => {
            assert_eq!(
                lifecycle_state_before_revoke_returns,
                crate::file_workspace::PreviewSessionState::Cancelled
            );
        }
        LifecycleAction::Dispose | LifecycleAction::BrowseTeardown => {
            assert_eq!(
                lifecycle_state_before_revoke_returns,
                crate::file_workspace::PreviewSessionState::Disposed
            );
        }
    }
    runtime.dispose();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleAction {
    Cancel,
    Switch,
    Dispose,
    BrowseTeardown,
}

impl LifecycleAction {
    fn name(self) -> &'static str {
        match self {
            Self::Cancel => "cancel",
            Self::Switch => "switch",
            Self::Dispose => "dispose",
            Self::BrowseTeardown => "browse-teardown",
        }
    }
}

#[test]
fn cancel_revokes_asset_after_preview_authority() {
    run_lifecycle_race(LifecycleAction::Cancel);
}

#[test]
fn cancel_rejects_asset_after_first_active_check_before_registry_lock() {
    let fixture = Fixture::new("publish-asset-toctou");
    let runtime = fixture.runtime();
    let preview_id = "preview-race-publish-asset-toctou";
    let race_gate = Arc::new(RaceGate::default());
    let (session, task, old_asset_token) = start_race_preview(
        &runtime,
        preview_id,
        source("browse-race", "synthetic-entry"),
        Arc::clone(&race_gate),
    );
    let publish_gate = Arc::new(PreviewAssetPublishGate::default());
    runtime
        .inner
        .preview_assets
        .set_publish_gate_for_test(Some(Arc::clone(&publish_gate)));

    race_gate.release_second();
    publish_gate.wait_until_entered();

    let control_runtime = runtime.clone();
    let control = thread::spawn(move || {
        control_runtime.cancel_preview(PreviewSessionRequest {
            preview_id: preview_id.to_string(),
            native_presentation: None,
        })
    });
    let control_result = control.join().expect("cancel control thread");
    assert_eq!(control_result, Ok(true));
    assert!(session.current_publication().is_none());

    let old_asset_read_after_cleanup = runtime.request_preview_asset(PreviewAssetRequestDto {
        preview_id: preview_id.to_string(),
        request_id: "old-request".to_string(),
        source_version: "preview-race-source-version".to_string(),
        asset_token: old_asset_token,
        offset_bytes: None,
        max_bytes: None,
    });
    assert!(old_asset_read_after_cleanup.is_err());
    assert_eq!(runtime.inner.preview_assets.counts(), (0, 0));

    publish_gate.release();
    let second_result = race_gate.wait_for_second();
    runtime.inner.preview_assets.set_publish_gate_for_test(None);
    let task_result = task.join();

    assert!(matches!(
        second_result,
        Err(PreviewAssetError::Cancelled | PreviewAssetError::StalePublication)
    ));
    assert_eq!(runtime.inner.preview_assets.counts(), (0, 0));
    assert!(matches!(
        task_result,
        Err(crate::file_workspace::PreviewRunError::Cancelled)
            | Err(crate::file_workspace::PreviewRunError::StalePublication)
    ));
    runtime.dispose();
}

#[test]
fn switch_revokes_asset_after_old_preview_authority() {
    run_lifecycle_race(LifecycleAction::Switch);
}

#[test]
fn switch_cleanup_preserves_asset_from_concurrent_new_request_start() {
    let fixture = Fixture::new("switch-new-request-start");
    let runtime = fixture.runtime();
    let preview_id = "preview-race-switch-new-request";
    let old_gate = Arc::new(RaceGate::default());
    let (session, old_task, old_asset_token) = start_race_preview(
        &runtime,
        preview_id,
        source("browse-race", "synthetic-entry"),
        Arc::clone(&old_gate),
    );
    let old_publication = session
        .current_publication()
        .expect("old session publication");
    let cleanup_gate = Arc::new(PreviewAssetRevokeGate::default());
    runtime
        .inner
        .preview_assets
        .set_cleanup_gate_for_test(Some(Arc::clone(&cleanup_gate)));

    let switch_runtime = runtime.clone();
    let switch = thread::spawn(move || {
        switch_runtime.switch_preview_source(PreviewSwitchSourceRequest {
            preview_id: preview_id.to_string(),
            request_id: "new-request".to_string(),
            source: source("new-browse", "new-entry"),
        })
    });
    cleanup_gate.wait_until_entered();
    assert!(!old_publication.is_current());
    assert_eq!(session.request().request_id, "new-request");

    let new_gate = Arc::new(RaceGate::default());
    let new_provider = Arc::new(OneShotAssetProvider {
        descriptor: PreviewProviderDescriptor::new(
            "test.preview-new-request",
            100,
            PreviewCapabilities::all(),
            vec![PreviewHostKind::ZenFloating],
            false,
        ),
        gate: Arc::clone(&new_gate),
    });
    let new_registry = Arc::new(
        PreviewProviderRegistry::new(vec![new_provider as Arc<dyn PreviewProvider>])
            .expect("new-request provider registry"),
    );
    let new_task = session
        .start_with_environment(
            Arc::new(StaticResolver {
                snapshot: snapshot(source("new-browse", "new-entry")),
            }),
            new_registry,
            crate::file_workspace::PreviewProviderEnvironmentHandle::with_asset_publisher(
                runtime.inner.preview_assets.clone(),
            ),
        )
        .expect("new request starts while switch cleanup is paused");
    let new_asset_token = new_gate.wait_for_first();
    let new_task_result = new_task.join();

    old_gate.release_second();
    let old_second_result = old_gate.wait_for_second();
    let old_task_result = old_task.join();

    cleanup_gate.release();
    let switch_result = switch.join().expect("switch control thread");
    runtime.inner.preview_assets.set_cleanup_gate_for_test(None);

    let old_asset_read = runtime.request_preview_asset(PreviewAssetRequestDto {
        preview_id: preview_id.to_string(),
        request_id: "old-request".to_string(),
        source_version: "preview-race-source-version".to_string(),
        asset_token: old_asset_token,
        offset_bytes: None,
        max_bytes: None,
    });
    let new_asset_read = runtime
        .request_preview_asset(PreviewAssetRequestDto {
            preview_id: preview_id.to_string(),
            request_id: "new-request".to_string(),
            source_version: "preview-race-source-version".to_string(),
            asset_token: new_asset_token,
            offset_bytes: None,
            max_bytes: None,
        })
        .expect("new request asset survives old cleanup");

    assert!(switch_result.is_ok(), "switch failed: {switch_result:?}");
    assert!(old_asset_read.is_err(), "old asset survived switch cleanup");
    assert_eq!(new_asset_read.bytes, b"new-asset");
    assert_eq!(
        runtime.inner.preview_assets.counts(),
        (1, b"new-asset".len())
    );
    assert!(
        new_task_result.is_ok(),
        "new request failed: {new_task_result:?}"
    );
    assert!(matches!(
        old_second_result,
        Err(PreviewAssetError::Cancelled | PreviewAssetError::StalePublication)
    ));
    assert!(matches!(
        old_task_result,
        Err(crate::file_workspace::PreviewRunError::Cancelled)
            | Err(crate::file_workspace::PreviewRunError::StalePublication)
    ));
    runtime.dispose();
}

#[test]
fn dispose_revokes_asset_after_preview_authority() {
    run_lifecycle_race(LifecycleAction::Dispose);
}

#[test]
fn browse_teardown_disposes_preview_before_revoking_assets() {
    run_lifecycle_race(LifecycleAction::BrowseTeardown);
}

#[test]
fn failed_switch_preserves_old_authority_and_exact_asset() {
    let fixture = Fixture::new("failed-switch");
    let runtime = fixture.runtime();
    let preview_id = "preview-race-failed-switch";
    runtime.inner.preview_assets.set_revoke_gate_for_test(None);
    let race_gate = Arc::new(RaceGate::default());
    let (session, task, old_asset_token) = start_race_preview(
        &runtime,
        preview_id,
        source("browse-race", "synthetic-entry"),
        Arc::clone(&race_gate),
    );
    let old_publication = session
        .current_publication()
        .expect("session-issued publication token");
    let result = runtime.switch_preview_source(PreviewSwitchSourceRequest {
        preview_id: preview_id.to_string(),
        request_id: "   ".to_string(),
        source: source("new-browse", "new-entry"),
    });
    assert_eq!(result, Err("preview_request_invalid".to_string()));
    assert!(old_publication.is_current());
    let artifact = runtime
        .request_preview_asset(PreviewAssetRequestDto {
            preview_id: preview_id.to_string(),
            request_id: "old-request".to_string(),
            source_version: "preview-race-source-version".to_string(),
            asset_token: old_asset_token,
            offset_bytes: None,
            max_bytes: None,
        })
        .expect("failed switch preserves old exact-token asset");
    assert_eq!(artifact.bytes, b"old-asset");

    race_gate.release_second();
    assert!(race_gate.wait_for_second().is_ok());
    runtime
        .cancel_preview(PreviewSessionRequest {
            preview_id: preview_id.to_string(),
            native_presentation: None,
        })
        .expect("cleanup failed-switch preview");
    assert!(matches!(
        task.join(),
        Err(crate::file_workspace::PreviewRunError::Cancelled)
            | Err(crate::file_workspace::PreviewRunError::StalePublication)
    ));
    assert_eq!(runtime.inner.preview_assets.counts(), (0, 0));
    runtime.dispose();
}
