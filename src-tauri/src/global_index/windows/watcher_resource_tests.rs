// Real notify resources on task-owned directories; native enumeration is
// injected, never performed on the host's volumes.
struct ResourceFixture {
    root: PathBuf,
    db: crate::db::Database,
    provider: WindowsGlobalIndexProvider,
    sources: Mutex<Vec<GlobalSourceDescriptor>>,
    calls: std::sync::atomic::AtomicUsize,
    operation_kinds: Mutex<Vec<SourceOperationKind>>,
    fail_next: std::sync::atomic::AtomicBool,
    unknown_next: std::sync::atomic::AtomicBool,
    poll_interval: Option<std::time::Duration>,
    _cleanup: ResourceDirectory,
}

struct ResourceDirectory(PathBuf);
impl Drop for ResourceDirectory {
    fn drop(&mut self) {
        // Windows notify/SQLite can briefly retain handles after watcher
        // shutdown. Never panic during unwinding; assert cleanup separately.
        for attempt in 0..20 {
            match std::fs::remove_dir_all(&self.0) {
                Ok(()) => return,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(error) if attempt == 19 => {
                    eprintln!(
                        "Windows Global Index test fixture cleanup failed at {}: {error}",
                        self.0.display()
                    );
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
    }
}

impl ResourceFixture {
    fn new() -> Self {
        Self::new_with_poll(None)
    }

    fn new_with_poll(poll_interval: Option<std::time::Duration>) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp-tests")
            .join(format!("round9-resources-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("watched")).unwrap();
        let db = crate::db::Database::open(root.join("fixture.sqlite3")).unwrap();
        let mut provider = WindowsGlobalIndexProvider::with_wake(
            std::sync::Arc::new(GlobalIndexWakeSlot::default()),
            db.path().to_path_buf(),
        );
        provider.force_direct = true;
        let mut source = windows_source();
        source.volume.last_full_index_at = None;
        source.volume.last_incremental_sync_at = None;
        source.volume.mount_path = root.join("watched").to_string_lossy().into_owned();
        db.upsert_global_volume(&source.volume).unwrap();
        Self {
            _cleanup: ResourceDirectory(root.clone()),
            root,
            db,
            provider,
            sources: Mutex::new(vec![source]),
            calls: Default::default(),
            operation_kinds: Mutex::new(Vec::new()),
            fail_next: Default::default(),
            unknown_next: Default::default(),
            poll_interval,
        }
    }

    fn selected(&self) -> GlobalSourceDescriptor {
        let id = self.sources.lock().unwrap()[0].volume.id.clone();
        GlobalSourceDescriptor {
            volume: self.db.get_global_volume(&id).unwrap().unwrap(),
        }
    }

    fn operate(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
        kind: SourceOperationKind,
        work: impl FnOnce(&mut dyn GlobalIndexSink) -> Result<(), GlobalIndexError>,
    ) -> Result<(), GlobalIndexError> {
        let command = match kind {
            SourceOperationKind::Initial => IndexServiceCommand::StartInitialIndex {
                source_id: source.volume.id.clone(),
            },
            SourceOperationKind::Incremental => IndexServiceCommand::ResumeIncrementalSync {
                source_id: source.volume.id.clone(),
            },
            SourceOperationKind::Rebuild => IndexServiceCommand::Rebuild {
                source_id: source.volume.id.clone(),
            },
        };
        let request = WindowsGlobalIndexProvider::source_request(command, source);
        self.provider
            .run_source_request(&request, source, sink, cancel, |_, sink| work(sink))
    }

    fn cycle(&self) {
        crate::global_index::coordinator::run_index_cycle_for_test(
            self,
            &self.db,
            &std::sync::Arc::new(AtomicBool::new(false)),
            &self.provider.wake,
        )
        .unwrap();
    }

    fn disable(&self, id: &str) {
        self.provider
            .update_source_lifecycle(id, &mut || {
                self.db.set_global_volume_enabled(id, false)?;
                Ok(true)
            })
            .unwrap();
        self.provider.source_enabled_changed(id, false);
    }

    fn revoke_authorization(&self, id: &str) {
        self.provider
            .update_source_lifecycle(id, &mut || {
                self.db.invalidate_missing_global_index_source(id, true)?;
                Ok(true)
            })
            .unwrap();
        self.provider.source_enabled_changed(id, false);
    }

    fn has_watcher(&self, id: &str) -> bool {
        self.provider
            .change_watchers
            .lock()
            .unwrap()
            .contains_key(id)
    }

    fn index(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
        kind: SourceOperationKind,
    ) -> Result<(), GlobalIndexError> {
        self.operate(source, sink, cancel, kind, |sink| {
            self.calls.fetch_add(1, Ordering::AcqRel);
            self.operation_kinds.lock().unwrap().push(kind);
            sink.checkpoint(&source.volume.id, Some("fixture-journal"), Some("43"))?;
            if self.fail_next.swap(false, Ordering::AcqRel) {
                return Err(GlobalIndexError::Provider(
                    "injected transient index failure".into(),
                ));
            }
            if self.unknown_next.swap(false, Ordering::AcqRel) {
                return Err(GlobalIndexError::ServiceOutcomeUnknown(
                    "injected unknown Service outcome".into(),
                ));
            }
            Ok(())
        })
    }
}

impl GlobalIndexProvider for ResourceFixture {
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        Ok(self.sources.lock().unwrap().clone())
    }
    fn start_initial_index(
        &self,
        s: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.index(s, sink, cancel, SourceOperationKind::Initial)
    }
    fn resume_incremental_sync(
        &self,
        s: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.index(s, sink, cancel, SourceOperationKind::Incremental)
    }
    fn rebuild(
        &self,
        s: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.index(s, sink, cancel, SourceOperationKind::Rebuild)
    }
    fn has_external_source_operation_admission(&self) -> bool {
        true
    }
    fn pause(&self) -> Result<(), GlobalIndexError> {
        self.provider.pause()
    }
    fn shutdown(&self) -> Result<(), GlobalIndexError> {
        self.provider.shutdown()
    }
    fn status(&self) -> Result<String, GlobalIndexError> {
        Ok("resource-fixture".into())
    }
    fn topology_audit_interval(&self) -> Option<std::time::Duration> {
        self.poll_interval
    }
    fn audit_source_topology(
        &self,
    ) -> Result<Option<Vec<GlobalSourceDescriptor>>, GlobalIndexError> {
        Ok(self
            .poll_interval
            .map(|_| self.sources.lock().unwrap().clone()))
    }
    fn update_source_lifecycle(
        &self,
        id: &str,
        update: &mut dyn FnMut() -> Result<bool, GlobalIndexError>,
    ) -> Result<(), GlobalIndexError> {
        self.provider.update_source_lifecycle(id, update)
    }
}

impl Drop for ResourceFixture {
    fn drop(&mut self) {
        self.provider.clear_native_change_watchers();
        // Database pool closes after the fixture; preserve its files until then.
    }
}

struct ResourceFixtureWorker {
    cancel: std::sync::Arc<AtomicBool>,
    wake: std::sync::Arc<GlobalIndexWakeSlot>,
    worker: Option<std::thread::JoinHandle<Result<(), GlobalIndexError>>>,
}

impl ResourceFixtureWorker {
    fn start(fixture: std::sync::Arc<ResourceFixture>) -> Self {
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let wake = fixture.provider.wake.clone();
        let worker_cancel = cancel.clone();
        let worker_wake = wake.clone();
        let worker = std::thread::spawn(move || {
            crate::global_index::coordinator::run_index(
                fixture.clone(),
                fixture.db.clone(),
                worker_cancel,
                worker_wake,
            )
        });
        Self {
            cancel,
            wake,
            worker: Some(worker),
        }
    }

    fn stop(&mut self) -> Result<(), GlobalIndexError> {
        self.cancel.store(true, Ordering::Release);
        self.wake.shutdown();
        self.worker.take().unwrap().join().expect("join index worker")
    }
}

impl Drop for ResourceFixtureWorker {
    fn drop(&mut self) {
        if self.worker.is_some() {
            let _ = self.stop();
        }
    }
}

fn wait_for_resource_fixture(predicate: impl Fn() -> bool, message: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !predicate() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(predicate(), "{message}");
}

#[test]
fn real_watcher_fixture_teardown_releases_test_owned_directory() {
    let root = {
        let fixture = ResourceFixture::new();
        fixture.cycle();
        let root = fixture.root.clone();
        assert!(fixture.has_watcher(&fixture.selected().volume.id));
        root
    };
    assert!(
        !root.exists(),
        "Windows watcher and SQLite test resources retained their fixture directory: {}",
        root.display()
    );
}

#[test]
fn admitted_transient_incremental_error_keeps_healthy_watcher_and_next_change_recovers() {
    use crate::global_index::models::{INDEX_STATUS_ERROR, INDEX_STATUS_READY};
    use crate::global_index::wake::GlobalIndexWakeReason;

    let fixture = std::sync::Arc::new(ResourceFixture::new());
    fixture.cycle();
    let source = fixture.selected();
    let id = source.volume.id.clone();
    let last_full_index_at = source.volume.last_full_index_at;
    let initial_calls = fixture.calls.load(Ordering::Acquire);
    let mut worker = ResourceFixtureWorker::start(fixture.clone());

    wait_for_resource_fixture(
        || {
            fixture.calls.load(Ordering::Acquire) == initial_calls + 1
                && fixture.db.get_global_volume(&id).unwrap().unwrap().index_status
                    == INDEX_STATUS_READY
                && fixture.provider.wake.snapshot().blocked_waits > 0
        },
        "coordinator did not finish its initial incremental cycle",
    );
    let lease_before_error = fixture
        .provider
        .change_watchers
        .lock()
        .unwrap()
        .get(&id)
        .expect("healthy native watcher installed")
        .registration_id
        .clone();

    fixture.fail_next.store(true, Ordering::Release);
    fixture.provider.wake.notify(GlobalIndexWakeReason::ExplicitCommand);
    wait_for_resource_fixture(
        || {
            fixture.calls.load(Ordering::Acquire) == initial_calls + 2
                && fixture.db.get_global_volume(&id).unwrap().unwrap().index_status
                    == INDEX_STATUS_ERROR
        },
        "accepted transient failure was not recorded",
    );
    let failed = fixture.db.get_global_volume(&id).unwrap().unwrap();
    assert_eq!(failed.index_status, INDEX_STATUS_ERROR);
    assert!(failed
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("injected transient index failure")));
    assert_eq!(failed.last_full_index_at, last_full_index_at);
    {
        let watchers = fixture.provider.change_watchers.lock().unwrap();
        let current = watchers
            .get(&id)
            .expect("transient index failure removed the healthy watcher");
        assert_eq!(current.registration_id, lease_before_error);
        assert!(!current.has_failed());
    }
    let error_calls = fixture.calls.load(Ordering::Acquire);
    let wake_before_change = fixture.provider.wake.snapshot();
    std::fs::write(
        Path::new(&source.volume.mount_path).join("after-transient-error.txt"),
        b"wake the coordinator",
    )
    .unwrap();
    wait_for_resource_fixture(
        || {
            let wake = fixture.provider.wake.snapshot();
            // One filesystem write can produce multiple coalesced wake hints
            // and more than one valid incremental cycle. Verify observable
            // recovery, not an exact scheduling count.
            fixture.calls.load(Ordering::Acquire) >= error_calls + 1
                && fixture.db.get_global_volume(&id).unwrap().unwrap().index_status
                    == INDEX_STATUS_READY
                && wake.notifications > wake_before_change.notifications
                && wake.delivered > wake_before_change.delivered
        },
        "file change did not wake the coordinator after the transient error",
    );
    assert_eq!(
        fixture.db.get_global_volume(&id).unwrap().unwrap().last_full_index_at,
        last_full_index_at,
        "event recovery must stay incremental instead of replaying MFT baseline"
    );
    let kinds = fixture.operation_kinds.lock().unwrap();
    assert_eq!(kinds[0], SourceOperationKind::Initial);
    assert!(kinds[1..]
        .iter()
        .all(|kind| *kind == SourceOperationKind::Incremental));
    drop(kinds);
    worker.stop().unwrap();
}

#[test]
fn failed_watcher_is_degraded_then_recovers_on_the_bounded_incremental_poll() {
    use crate::global_index::models::{
        INDEX_STATUS_READY, INDEX_STATUS_WATCHER_UNAVAILABLE,
    };
    use std::time::Duration;

    let fixture = std::sync::Arc::new(ResourceFixture::new_with_poll(Some(
        Duration::from_millis(600),
    )));
    fixture.cycle();
    let source = fixture.selected();
    let id = source.volume.id.clone();
    let initial_calls = fixture.calls.load(Ordering::Acquire);
    fixture
        .provider
        .change_watchers
        .lock()
        .unwrap()
        .get(&id)
        .expect("real watcher installed")
        .mark_failed_for_test();
    fixture.fail_next.store(true, Ordering::Release);

    let blocked_waits_before_worker = fixture.provider.wake.snapshot().blocked_waits;
    let mut worker = ResourceFixtureWorker::start(fixture.clone());
    wait_for_resource_fixture(
        || {
            fixture.calls.load(Ordering::Acquire) == initial_calls + 1
                && fixture.db.get_global_volume(&id).unwrap().unwrap().index_status
                    == INDEX_STATUS_WATCHER_UNAVAILABLE
                && !fixture.has_watcher(&id)
                && fixture.provider.wake.snapshot().blocked_waits > blocked_waits_before_worker
        },
        "failed native watcher did not release its resource and report degradation",
    );
    assert!(fixture.db.has_watcher_unavailable_sources().unwrap());
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        fixture.calls.load(Ordering::Acquire),
        initial_calls + 1,
        "degraded watcher recovery must wait for the bounded poll"
    );
    let degraded = fixture.db.get_global_volume(&id).unwrap().unwrap();
    assert_eq!(degraded.index_status, INDEX_STATUS_WATCHER_UNAVAILABLE);
    assert!(degraded
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("injected transient index failure")));

    wait_for_resource_fixture(
        || {
            fixture.calls.load(Ordering::Acquire) == initial_calls + 2
                && fixture.db.get_global_volume(&id).unwrap().unwrap().index_status
                    == INDEX_STATUS_READY
                && fixture.has_watcher(&id)
        },
        "bounded incremental poll did not reinstall the failed watcher",
    );
    assert!(!fixture.db.has_watcher_unavailable_sources().unwrap());
    assert!(fixture
        .provider
        .change_watchers
        .lock()
        .unwrap()
        .get(&id)
        .is_some_and(|watcher| !watcher.has_failed()));
    std::thread::sleep(Duration::from_millis(650));
    assert_eq!(
        fixture.calls.load(Ordering::Acquire),
        initial_calls + 2,
        "successful watcher recovery must stop the extra polling"
    );
    let kinds = fixture.operation_kinds.lock().unwrap();
    assert_eq!(kinds[0], SourceOperationKind::Initial);
    assert!(kinds[1..]
        .iter()
        .all(|kind| *kind == SourceOperationKind::Incremental));
    drop(kinds);
    worker.stop().unwrap();
}

#[test]
fn authorization_revocation_during_failed_incremental_revokes_watcher_and_blocks_followup_indexing() {
    use crate::global_index::coordinator::database_sink_for_test;
    use crate::global_index::models::SCOPE_AUTHORIZATION_NOT_AUTHORIZED;
    use std::sync::Barrier;
    use std::time::Duration;

    let fixture = ResourceFixture::new();
    fixture.cycle();
    let source = fixture.selected();
    let id = source.volume.id.clone();
    let reached = Barrier::new(2);
    let resume = Barrier::new(2);
    std::thread::scope(|scope| {
        let task = scope.spawn(|| {
            let mut sink = database_sink_for_test(fixture.db.clone());
            fixture.operate(
                &source,
                &mut sink,
                &AtomicBool::new(false),
                SourceOperationKind::Incremental,
                |sink| {
                    sink.checkpoint(&id, Some("fixture-journal"), Some("44"))?;
                    reached.wait();
                    resume.wait();
                    Err(GlobalIndexError::Provider(
                        "injected error concurrent with consent withdrawal".into(),
                    ))
                },
            )
        });
        reached.wait();
        fixture.revoke_authorization(&id);
        resume.wait();
        assert!(task.join().unwrap().is_err());
    });
    assert!(!fixture.has_watcher(&id));
    let disabled = fixture.db.get_global_volume(&id).unwrap().unwrap();
    assert!(!disabled.enabled);
    assert_eq!(
        disabled.scope_authorization_state,
        SCOPE_AUTHORIZATION_NOT_AUTHORIZED
    );
    let notifications_after_revoke = fixture.provider.wake.snapshot().notifications;
    let indexed_calls_after_revoke = fixture.calls.load(Ordering::Acquire);
    std::fs::write(
        Path::new(&source.volume.mount_path).join("after-consent-withdrawal.txt"),
        b"must not be observed",
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(
        fixture.provider.wake.snapshot().notifications,
        notifications_after_revoke,
        "revoked watcher published a later filesystem event"
    );
    fixture.cycle();
    assert_eq!(
        fixture.calls.load(Ordering::Acquire),
        indexed_calls_after_revoke,
        "disabled source was indexed after its in-flight error"
    );
    assert!(!fixture.db.get_global_volume(&id).unwrap().unwrap().enabled);
}

#[test]
fn unknown_service_outcome_does_not_enter_watcher_unavailable_poll_recovery() {
    use crate::global_index::models::INDEX_STATUS_WATCHER_UNAVAILABLE;
    use crate::global_index::wake::GlobalIndexWakeReason;
    use std::time::Duration;

    let fixture = std::sync::Arc::new(ResourceFixture::new_with_poll(Some(
        Duration::from_millis(100),
    )));
    fixture.cycle();
    let source = fixture.selected();
    let id = source.volume.id.clone();
    let initial_calls = fixture.calls.load(Ordering::Acquire);
    fixture
        .provider
        .change_watchers
        .lock()
        .unwrap()
        .get(&id)
        .expect("real watcher installed")
        .mark_failed_for_test();
    fixture.unknown_next.store(true, Ordering::Release);
    // Model the one callback wake emitted when the real watcher reports its
    // failure. The coordinator consumes it for this cycle before the uncertain
    // Service outcome is returned.
    fixture
        .provider
        .wake
        .notify(GlobalIndexWakeReason::ProviderChange);
    let mut worker = ResourceFixtureWorker::start(fixture.clone());

    wait_for_resource_fixture(
        || {
            fixture.calls.load(Ordering::Acquire) == initial_calls + 1
                && fixture
                    .db
                    .global_index_operation_state(&id)
                    .unwrap()
                    .unwrap()
                    .operation_state
                    .as_deref()
                    == Some("unknown")
        },
        "uncertain Service result was not retained as unknown",
    );
    let source_after_unknown = fixture.db.get_global_volume(&id).unwrap().unwrap();
    assert_ne!(source_after_unknown.index_status, INDEX_STATUS_WATCHER_UNAVAILABLE);
    assert!(!fixture.db.has_watcher_unavailable_sources().unwrap());
    let calls_after_unknown = fixture.calls.load(Ordering::Acquire);
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(
        fixture.calls.load(Ordering::Acquire),
        calls_after_unknown,
        "unknown Service result was replayed by bounded watcher recovery"
    );
    worker.stop().unwrap();
}

#[test]
fn watcher_resources_follow_capability_revocation_and_explicit_reconsent() {
    use crate::global_index::models::PROVIDER_WINDOWS_UNSUPPORTED;
    // The production classifier distinguishes NTFS loss, USN loss and GUID
    // distrust. Every case executes actual Coordinator rediscovery and notify
    // handle destruction, then capability recovery and explicit reconsent.
    for (guid, filesystem, flags) in [
        (Some("fixture-guid"), "fat32", 0x0200_0000),
        (Some("fixture-guid"), "ntfs", 0),
        (None, "ntfs", 0x0200_0000),
    ] {
        let fixture = ResourceFixture::new();
        fixture.cycle();
        let original = fixture.selected();
        let id = original.volume.id.clone();
        assert!(fixture.has_watcher(&id));
        let classified = volumes::classify_volume(guid, 7, filesystem, 3, flags);
        assert_eq!(classified.provider, PROVIDER_WINDOWS_UNSUPPORTED);
        let mut downgraded = original.clone();
        downgraded.volume.provider = classified.provider.into();
        downgraded.volume.filesystem_type = filesystem.into();
        downgraded.volume.enabled = false;
        downgraded.volume.scope_authorization_state = SCOPE_AUTHORIZATION_NOT_AUTHORIZED.into();
        fixture.sources.lock().unwrap()[0] = downgraded;
        fixture.cycle();
        let revoked = fixture.selected();
        assert!(!revoked.volume.enabled);
        assert_eq!(
            revoked.volume.scope_authorization_state,
            SCOPE_AUTHORIZATION_NOT_AUTHORIZED
        );
        assert!(!fixture.has_watcher(&id));
        let calls = fixture.calls.load(Ordering::Acquire);
        let mut recovered = original.clone();
        recovered.volume.enabled = false;
        recovered.volume.scope_authorization_state = SCOPE_AUTHORIZATION_NOT_AUTHORIZED.into();
        fixture.sources.lock().unwrap()[0] = recovered.clone();
        fixture.cycle();
        assert!(!fixture.has_watcher(&id));
        assert!(!fixture.selected().volume.enabled);
        assert_eq!(fixture.calls.load(Ordering::Acquire), calls);
        assert!(fixture.db.set_global_volume_enabled(&id, true).is_err());
        assert!(fixture
            .db
            .authorize_global_volume(&id, false, &recovered.volume)
            .is_err());
        fixture
            .provider
            .update_source_lifecycle(&id, &mut || {
                fixture
                    .db
                    .authorize_global_volume(&id, true, &recovered.volume)?;
                Ok(true)
            })
            .unwrap();
        fixture.cycle();
        assert!(fixture.has_watcher(&id));
        assert_eq!(fixture.selected().volume.index_status, INDEX_STATUS_READY);
    }
}

#[test]
fn identity_replacement_removes_old_watcher_and_preserves_other_source() {
    let fixture = ResourceFixture::new();
    fixture.cycle();
    let old = fixture.selected();
    let mut other = old.clone();
    other.volume.id = "independent-source".into();
    other.volume.stable_volume_id = "independent-stable".into();
    other.volume.mount_path = fixture.root.join("other").to_string_lossy().into_owned();
    std::fs::create_dir(&other.volume.mount_path).unwrap();
    fixture.db.upsert_global_volume(&other.volume).unwrap();
    fixture.sources.lock().unwrap().push(other.clone());
    fixture.cycle();
    assert!(fixture.has_watcher(&other.volume.id));
    let mut replacement = old.clone();
    replacement.volume.id = "new-identity".into();
    replacement.volume.stable_volume_id = "new-guid".into();
    replacement.volume.enabled = false;
    replacement.volume.scope_authorization_state = SCOPE_AUTHORIZATION_NOT_AUTHORIZED.into();
    fixture.sources.lock().unwrap()[0] = replacement.clone();
    fixture.cycle();
    assert!(!fixture.has_watcher(&old.volume.id));
    assert!(!fixture.has_watcher(&replacement.volume.id));
    assert!(fixture.has_watcher(&other.volume.id));
    fixture.disable(&other.volume.id);
    assert!(!fixture.has_watcher(&other.volume.id));
    assert!(fixture
        .db
        .get_global_volume(&old.volume.id)
        .unwrap()
        .is_some());
    let old_row = fixture
        .db
        .get_global_volume(&old.volume.id)
        .unwrap()
        .unwrap();
    assert!(!old_row.enabled);
    assert_eq!(
        old_row.scope_authorization_state,
        SCOPE_AUTHORIZATION_NOT_AUTHORIZED
    );
    fixture.sources.lock().unwrap()[0] = old.clone();
    fixture.cycle();
    assert!(
        !fixture.has_watcher(&old.volume.id),
        "returning identity needs fresh explicit consent"
    );
}

#[test]
fn disable_pause_failure_and_owner_loss_at_post_work_registration_boundary() {
    use crate::global_index::coordinator::database_sink_for_test;
    use std::sync::Barrier;
    for action in [
        "disable",
        "pause",
        "provider-pause",
        "owner-lost",
        "service-error",
    ] {
        let fixture = ResourceFixture::new();
        fixture.cycle();
        let source = fixture.selected();
        let cancel = AtomicBool::new(false);
        let reached = Barrier::new(2);
        let resume = Barrier::new(2);
        std::thread::scope(|scope| {
            let task = scope.spawn(|| {
                let mut sink = database_sink_for_test(fixture.db.clone());
                fixture.operate(
                    &source,
                    &mut sink,
                    &cancel,
                    SourceOperationKind::Incremental,
                    |sink| {
                        sink.checkpoint(&source.volume.id, Some("fixture-journal"), Some("44"))?;
                        reached.wait();
                        resume.wait();
                        if action == "service-error" {
                            Err(GlobalIndexError::ServiceOutcomeUnknown(
                                "injected terminal failure".into(),
                            ))
                        } else {
                            Ok(())
                        }
                    },
                )
            });
            reached.wait();
            match action {
                "disable" => fixture.disable(&source.volume.id),
                "pause" => {
                    cancel.store(true, Ordering::Release);
                    fixture.provider.clear_native_change_watchers();
                }
                "provider-pause" => fixture.provider.clear_native_change_watchers(),
                "owner-lost" => {
                    fixture
                        .db
                        .invalidate_missing_global_index_source(&source.volume.id, false)
                        .unwrap();
                }
                _ => {}
            }
            resume.wait();
            let result = task.join().unwrap();
            if action != "pause" && action != "provider-pause" {
                assert!(result.is_err());
            }
        });
        assert!(
            !fixture.has_watcher(&source.volume.id),
            "resource survived {action}"
        );
    }
}

#[test]
fn real_watcher_initial_incremental_rebuild_resume_and_source_isolation() {
    let fixture = ResourceFixture::new();
    fixture.cycle();
    let first = fixture.selected();
    let id = &first.volume.id;
    assert!(first.volume.last_full_index_at.is_some());
    assert!(first.volume.last_incremental_sync_at.is_some());
    assert_eq!(first.volume.journal_cursor.as_deref(), Some("43"));
    assert!(fixture.has_watcher(id));
    fixture.cycle();
    assert_eq!(
        fixture.selected().volume.last_full_index_at,
        first.volume.last_full_index_at
    );
    assert!(fixture.db.request_global_index_rebuild(id).unwrap());
    fixture.cycle();
    let state = fixture
        .db
        .global_index_operation_state(id)
        .unwrap()
        .unwrap();
    assert!(!state.rebuild_pending());
    assert!(state.owner_id.is_none());
    assert!(fixture.has_watcher(id));
    fixture.provider.clear_native_change_watchers();
    assert!(!fixture.has_watcher(id));
    fixture.cycle();
    assert!(fixture.has_watcher(id));

    let mut second = first.clone();
    second.volume.id = "second-source".into();
    second.volume.stable_volume_id = "second-guid".into();
    second.volume.mount_path = fixture.root.join("second").to_string_lossy().into_owned();
    std::fs::create_dir(&second.volume.mount_path).unwrap();
    fixture.db.upsert_global_volume(&second.volume).unwrap();
    fixture.sources.lock().unwrap().push(second.clone());
    fixture.cycle();
    assert!(fixture.has_watcher(&second.volume.id));
    fixture.disable(id);
    assert!(!fixture.has_watcher(id));
    assert!(fixture.has_watcher(&second.volume.id));
    let calls = fixture.calls.load(Ordering::Acquire);
    fixture.cycle();
    assert_eq!(fixture.calls.load(Ordering::Acquire), calls + 1);
    assert!(fixture.has_watcher(&second.volume.id));
    fixture
        .provider
        .update_source_lifecycle(id, &mut || {
            fixture.db.set_global_volume_enabled(id, true)?;
            Ok(true)
        })
        .unwrap();
    fixture.cycle();
    assert!(fixture.has_watcher(id));
    assert_eq!(fixture.selected().volume.index_status, INDEX_STATUS_READY);
}

#[test]
fn callback_queued_between_revoke_commit_and_cleanup_cannot_publish() {
    let fixture = ResourceFixture::new();
    fixture.cycle();
    let source = fixture.selected();
    let callback = fixture
        .provider
        .change_watchers
        .lock()
        .unwrap()
        .get(&source.volume.id)
        .unwrap()
        .queued_signal_for_test(fixture.provider.wake.clone());
    let notifications = fixture.provider.wake.snapshot().notifications;
    let (started, received) = std::sync::mpsc::sync_channel(0);
    std::thread::scope(|scope| {
        let mut pending = None;
        fixture
            .provider
            .update_source_lifecycle(&source.volume.id, &mut || {
                fixture
                    .db
                    .set_global_volume_enabled(&source.volume.id, false)?;
                pending = Some(scope.spawn(|| {
                    started.send(()).unwrap();
                    callback();
                }));
                received
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                Ok(true)
            })
            .unwrap();
        pending.take().unwrap().join().unwrap();
    });
    assert_eq!(
        fixture.provider.wake.snapshot().notifications,
        notifications
    );
    assert!(!fixture.has_watcher(&source.volume.id));
}

#[test]
fn rejected_admission_and_stale_cleanup_preserve_successor_watcher_lease() {
    use crate::global_index::coordinator::database_sink_for_test;
    use std::sync::Barrier;
    for boundary in ["busy", "successor-error", "successor-owner-lost"] {
        let fixture = ResourceFixture::new();
        fixture.cycle();
        let source = fixture.selected();
        let reached = Barrier::new(2);
        let resume = Barrier::new(2);
        std::thread::scope(|scope| {
            let task = scope.spawn(|| {
                let mut sink = database_sink_for_test(fixture.db.clone());
                fixture.operate(
                    &source,
                    &mut sink,
                    &AtomicBool::new(false),
                    SourceOperationKind::Incremental,
                    |_| {
                        reached.wait();
                        resume.wait();
                        if boundary == "successor-error" {
                            Err(GlobalIndexError::ServiceOutcomeUnknown(
                                "obsolete failure".into(),
                            ))
                        } else {
                            Ok(())
                        }
                    },
                )
            });
            reached.wait();
            if boundary == "busy" {
                let mut contender = database_sink_for_test(fixture.db.clone());
                assert!(matches!(
                    fixture.operate(
                        &source,
                        &mut contender,
                        &AtomicBool::new(false),
                        SourceOperationKind::Incremental,
                        |_| panic!("busy source was indexed")
                    ),
                    Err(GlobalIndexError::SourceBusy(_))
                ));
            } else {
                fixture.disable(&source.volume.id);
                fixture
                    .provider
                    .update_source_lifecycle(&source.volume.id, &mut || {
                        fixture
                            .db
                            .set_global_volume_enabled(&source.volume.id, true)?;
                        Ok(true)
                    })
                    .unwrap();
                fixture.cycle();
            }
            assert!(fixture.has_watcher(&source.volume.id));
            let lease = fixture
                .provider
                .change_watchers
                .lock()
                .unwrap()
                .get(&source.volume.id)
                .unwrap()
                .registration_id
                .clone();
            resume.wait();
            let result = task.join().unwrap();
            assert_eq!(result.is_ok(), boundary == "busy");
            let notifications_before_change = fixture.provider.wake.snapshot().notifications;
            std::fs::write(
                Path::new(&source.volume.mount_path)
                    .join(format!("successor-{boundary}.txt")),
                b"successor watcher must remain live",
            )
            .unwrap();
            wait_for_resource_fixture(
                || {
                    fixture.provider.wake.snapshot().notifications
                        > notifications_before_change
                },
                "stale request failure left no live watcher to publish a file change",
            );
            let watchers = fixture.provider.change_watchers.lock().unwrap();
            let current = watchers
                .get(&source.volume.id)
                .expect("obsolete request destroyed successor watcher");
            if boundary != "busy" {
                assert_eq!(current.registration_id, lease);
            }
            assert!(!current.has_failed());
        });
    }
}
