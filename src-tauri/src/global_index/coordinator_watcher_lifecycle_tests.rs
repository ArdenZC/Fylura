// No native drive reads: the production Windows request route/admission and
// DatabaseIndexSink execute against an isolated database with injected work.
struct WatcherLifecycleProvider {
    source: GlobalSourceDescriptor,
    failed: AtomicBool,
    calls: Mutex<Vec<SourceOperationKind>>,
    boundary: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl WatcherLifecycleProvider {
    fn operate(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        kind: SourceOperationKind,
    ) -> Result<(), GlobalIndexError> {
        self.calls.lock().unwrap().push(kind);
        crate::global_index::windows::WindowsGlobalIndexProvider::run_lifecycle_fixture(
            source,
            sink,
            kind,
            self.failed.load(Ordering::Acquire),
            |sink| {
                sink.checkpoint(&source.volume.id, Some("fixture-journal"), Some("42"))?;
                if let Some(action) = self.boundary.lock().unwrap().take() {
                    action();
                }
                Ok(())
            },
        )
    }
}
impl GlobalIndexProvider for WatcherLifecycleProvider {
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        Ok(vec![self.source.clone()])
    }
    fn start_initial_index(
        &self,
        s: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        _: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.operate(s, sink, SourceOperationKind::Initial)
    }
    fn resume_incremental_sync(
        &self,
        s: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        _: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.operate(s, sink, SourceOperationKind::Incremental)
    }
    fn rebuild(
        &self,
        s: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        _: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.operate(s, sink, SourceOperationKind::Rebuild)
    }
    fn has_external_source_operation_admission(&self) -> bool {
        true
    }
    fn pause(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }
    fn shutdown(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }
    fn status(&self) -> Result<String, GlobalIndexError> {
        Ok("fixture".into())
    }
    // Accelerate the production worker's audit tick; the Windows provider keeps 300 seconds.
    fn topology_audit_interval(&self) -> Option<Duration> {
        Some(Duration::from_millis(25))
    }
    fn audit_source_topology(
        &self,
    ) -> Result<Option<Vec<GlobalSourceDescriptor>>, GlobalIndexError> {
        Ok(Some(vec![self.source.clone()]))
    }
}
fn watcher_fixture() -> (TestDatabasePath, Database, WatcherLifecycleProvider) {
    let path = TestDatabasePath::new();
    let db = Database::open(&path.0).unwrap();
    let mut volume = crate::global_index::tests::test_volume();
    volume.mount_path = path
        .0
        .with_extension("nonexistent-watcher-root")
        .to_string_lossy()
        .into_owned();
    db.upsert_global_volume(&volume).unwrap();
    (
        path,
        db,
        WatcherLifecycleProvider {
            source: GlobalSourceDescriptor { volume },
            failed: AtomicBool::new(true),
            calls: Mutex::new(vec![]),
            boundary: Mutex::new(None),
        },
    )
}
#[test]
fn watcher_failure_initial_incremental_rebuild_and_recovery_keep_one_finalizer() {
    let (_path, db, provider) = watcher_fixture();
    let cancel = Arc::new(AtomicBool::new(false));
    let wake = GlobalIndexWakeSlot::default();
    for expected in [
        SourceOperationKind::Initial,
        SourceOperationKind::Incremental,
        SourceOperationKind::Rebuild,
        SourceOperationKind::Incremental,
    ] {
        if expected == SourceOperationKind::Rebuild {
            db.request_global_index_rebuild(&provider.source.volume.id)
                .unwrap();
        }
        run_index_cycle(&provider, &db, &cancel, &wake).unwrap();
        let row = db
            .get_global_volume(&provider.source.volume.id)
            .unwrap()
            .unwrap();
        assert!(
            row.last_full_index_at.is_some(),
            "successful baseline must persist full timestamp"
        );
        assert!(row.last_incremental_sync_at.is_some());
        assert_eq!(row.index_status, INDEX_STATUS_WATCHER_UNAVAILABLE);
        assert_eq!(row.journal_cursor.as_deref(), Some("42"));
        assert!(db.has_watcher_unavailable_sources().unwrap());
        let op = db.global_index_operation_state(&row.id).unwrap().unwrap();
        assert_eq!(op.owner_id, None);
        assert!(!op.rebuild_pending());
        assert_eq!(*provider.calls.lock().unwrap().last().unwrap(), expected);
        eprintln!("round7 watcher kind={expected:?} status={} full={:?} incremental={:?} owner={:?} rebuild_pending={} cursor={:?}", row.index_status, row.last_full_index_at, row.last_incremental_sync_at, op.owner_id, op.rebuild_pending(), row.journal_cursor);
    }
    provider.failed.store(false, Ordering::Release);
    run_index_cycle(&provider, &db, &cancel, &wake).unwrap();
    assert_eq!(
        db.get_global_volume(&provider.source.volume.id)
            .unwrap()
            .unwrap()
            .index_status,
        INDEX_STATUS_READY
    );
    assert!(
        !db.has_watcher_unavailable_sources().unwrap(),
        "recovery stops extra polling"
    );
}
#[test]
fn watcher_failure_pause_disable_and_new_rebuild_fence_old_completion() {
    for boundary in ["pause", "disable", "rebuild"] {
        let (_path, db, provider) = watcher_fixture();
        let cancel = Arc::new(AtomicBool::new(false));
        let other = Database::open(db.path()).unwrap();
        let id = provider.source.volume.id.clone();
        let pause = cancel.clone();
        *provider.boundary.lock().unwrap() = Some(Box::new(move || match boundary {
            "pause" => pause.store(true, Ordering::Release),
            "disable" => other.set_global_volume_enabled(&id, false).unwrap(),
            _ => {
                other.request_global_index_rebuild(&id).unwrap();
            }
        }));
        let result = run_index_cycle(&provider, &db, &cancel, &GlobalIndexWakeSlot::default());
        let row = db
            .get_global_volume(&provider.source.volume.id)
            .unwrap()
            .unwrap();
        let op = db.global_index_operation_state(&row.id).unwrap().unwrap();
        if boundary == "pause" {
            result.unwrap();
            assert_eq!(row.index_status, INDEX_STATUS_PAUSED);
        } else if boundary == "disable" {
            result.unwrap(); // owner loss is reconciled by the Coordinator
            assert!(!row.enabled);
            assert_eq!(row.last_full_index_at, None, "fenced work cannot finalize");
        } else {
            result.unwrap();
            assert!(
                op.rebuild_pending(),
                "a rebuild requested during initial indexing remains pending"
            );
        }
        assert_eq!(op.owner_id, None);
    }
}

// Always stop/join before the database path guard removes WAL/SHM, including
// assertion-unwind paths. A detached worker would make fixture cleanup unsafe.
struct WatcherFixtureWorker {
    cancel: Arc<AtomicBool>,
    wake: Arc<GlobalIndexWakeSlot>,
    worker: Option<JoinHandle<Result<(), GlobalIndexError>>>,
}
impl WatcherFixtureWorker {
    fn stop(&mut self) -> Result<(), GlobalIndexError> {
        self.cancel.store(true, Ordering::Release);
        self.wake.shutdown();
        self.worker
            .take()
            .unwrap()
            .join()
            .expect("join fixture worker")
    }
}
impl Drop for WatcherFixtureWorker {
    fn drop(&mut self) {
        if self.worker.is_some() {
            let _ = self.stop();
        }
    }
}

#[test]
fn watcher_poll_worker_uses_incremental_then_stops_after_recovery() {
    let (_path, db, provider) = watcher_fixture();
    let provider = Arc::new(provider);
    let cancel = Arc::new(AtomicBool::new(false));
    let wake = Arc::new(GlobalIndexWakeSlot::default());
    let worker = {
        let provider = provider.clone();
        let db = db.clone();
        let cancel = cancel.clone();
        let wake = wake.clone();
        thread::spawn(move || run_index(provider, db, cancel, wake))
    };
    let mut worker_guard = WatcherFixtureWorker {
        cancel: cancel.clone(),
        wake: wake.clone(),
        worker: Some(worker),
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while provider.calls.lock().unwrap().len() < 3 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        provider.calls.lock().unwrap().len() >= 3,
        "bounded audit must wake degraded source"
    );
    provider.failed.store(false, Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(5);
    while db
        .get_global_volume(&provider.source.volume.id)
        .unwrap()
        .unwrap()
        .index_status
        != INDEX_STATUS_READY
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        db.get_global_volume(&provider.source.volume.id)
            .unwrap()
            .unwrap()
            .index_status,
        INDEX_STATUS_READY
    );
    assert!(!db.has_watcher_unavailable_sources().unwrap());
    let count = provider.calls.lock().unwrap().len();
    thread::sleep(Duration::from_millis(120));
    assert_eq!(
        provider.calls.lock().unwrap().len(),
        count,
        "unchanged topology + healthy watcher must stay idle"
    );
    worker_guard.stop().unwrap();
    let calls = provider.calls.lock().unwrap();
    assert_eq!(calls[0], SourceOperationKind::Initial);
    assert!(calls[1..]
        .iter()
        .all(|kind| *kind == SourceOperationKind::Incremental));
}
