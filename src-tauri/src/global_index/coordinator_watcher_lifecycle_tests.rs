// No native drive reads: the production Windows request route/admission and
// DatabaseIndexSink execute against an isolated database with injected work.
struct WatcherLifecycleProvider {
    source: GlobalSourceDescriptor,
    failed: AtomicBool,
    calls: Mutex<Vec<SourceOperationKind>>,
    boundary: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl WatcherLifecycleProvider {
    fn operate(&self, source: &GlobalSourceDescriptor, sink: &mut dyn GlobalIndexSink, kind: SourceOperationKind) -> Result<(), GlobalIndexError> {
        self.calls.lock().unwrap().push(kind);
        crate::global_index::windows::WindowsGlobalIndexProvider::run_lifecycle_fixture(
            source, sink, kind, self.failed.load(Ordering::Acquire), |sink| {
                sink.checkpoint(&source.volume.id, Some("fixture-journal"), Some("42"))?;
                if let Some(action) = self.boundary.lock().unwrap().take() { action(); }
                Ok(())
            })
    }
}
impl GlobalIndexProvider for WatcherLifecycleProvider {
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> { Ok(vec![self.source.clone()]) }
    fn start_initial_index(&self, s: &GlobalSourceDescriptor, sink: &mut dyn GlobalIndexSink, _: &AtomicBool) -> Result<(), GlobalIndexError> { self.operate(s, sink, SourceOperationKind::Initial) }
    fn resume_incremental_sync(&self, s: &GlobalSourceDescriptor, sink: &mut dyn GlobalIndexSink, _: &AtomicBool) -> Result<(), GlobalIndexError> { self.operate(s, sink, SourceOperationKind::Incremental) }
    fn rebuild(&self, s: &GlobalSourceDescriptor, sink: &mut dyn GlobalIndexSink, _: &AtomicBool) -> Result<(), GlobalIndexError> { self.operate(s, sink, SourceOperationKind::Rebuild) }
    fn has_external_source_operation_admission(&self) -> bool { true }
    fn pause(&self) -> Result<(), GlobalIndexError> { Ok(()) }
    fn shutdown(&self) -> Result<(), GlobalIndexError> { Ok(()) }
    fn status(&self) -> Result<String, GlobalIndexError> { Ok("fixture".into()) }
}
fn watcher_fixture() -> (TestDatabasePath, Database, WatcherLifecycleProvider) {
    let path = TestDatabasePath::new();
    let db = Database::open(&path.0).unwrap();
    let mut volume = crate::global_index::tests::test_volume();
    volume.mount_path = path.0.with_extension("nonexistent-watcher-root").to_string_lossy().into_owned();
    db.upsert_global_volume(&volume).unwrap();
    (path, db, WatcherLifecycleProvider { source: GlobalSourceDescriptor { volume }, failed: AtomicBool::new(true), calls: Mutex::new(vec![]), boundary: Mutex::new(None) })
}
#[test]
fn watcher_failure_initial_incremental_rebuild_and_recovery_keep_one_finalizer() {
    let (_path, db, provider) = watcher_fixture();
    let cancel = Arc::new(AtomicBool::new(false));
    let wake = GlobalIndexWakeSlot::default();
    for expected in [SourceOperationKind::Initial, SourceOperationKind::Incremental, SourceOperationKind::Rebuild, SourceOperationKind::Incremental] {
        if expected == SourceOperationKind::Rebuild { db.request_global_index_rebuild(&provider.source.volume.id).unwrap(); }
        run_index_cycle(&provider, &db, &cancel, &wake).unwrap();
        let row = db.get_global_volume(&provider.source.volume.id).unwrap().unwrap();
        assert!(row.last_full_index_at.is_some(), "successful baseline must persist full timestamp");
        assert!(row.last_incremental_sync_at.is_some());
        assert_eq!(row.index_status, INDEX_STATUS_WATCHER_UNAVAILABLE);
        assert_eq!(row.journal_cursor.as_deref(), Some("42"));
        assert!(db.has_watcher_unavailable_sources().unwrap());
        let op = db.global_index_operation_state(&row.id).unwrap().unwrap();
        assert_eq!(op.owner_id, None);
        assert!(!op.rebuild_pending());
        assert_eq!(*provider.calls.lock().unwrap().last().unwrap(), expected);
    }
    provider.failed.store(false, Ordering::Release);
    run_index_cycle(&provider, &db, &cancel, &wake).unwrap();
    assert_eq!(db.get_global_volume(&provider.source.volume.id).unwrap().unwrap().index_status, INDEX_STATUS_READY);
    assert!(!db.has_watcher_unavailable_sources().unwrap(), "recovery stops extra polling");
}
#[test]
fn watcher_failure_pause_disable_and_new_rebuild_fence_old_completion() {
    for boundary in ["pause", "disable", "rebuild"] {
        let (_path, db, provider) = watcher_fixture();
        let cancel = Arc::new(AtomicBool::new(false));
        let other = Database::open(&db.path()).unwrap();
        let id = provider.source.volume.id.clone();
        let pause = cancel.clone();
        *provider.boundary.lock().unwrap() = Some(Box::new(move || match boundary {
            "pause" => pause.store(true, Ordering::Release),
            "disable" => other.set_global_volume_enabled(&id, false).unwrap(),
            _ => { other.request_global_index_rebuild(&id).unwrap(); }
        }));
        let result = run_index_cycle(&provider, &db, &cancel, &GlobalIndexWakeSlot::default());
        let row = db.get_global_volume(&provider.source.volume.id).unwrap().unwrap();
        let op = db.global_index_operation_state(&row.id).unwrap().unwrap();
        if boundary == "pause" {
            result.unwrap();
            assert_eq!(row.index_status, INDEX_STATUS_PAUSED);
        } else {
            assert!(matches!(result, Err(GlobalIndexError::ServiceOutcomeUnknown(_))));
            if boundary == "disable" { assert!(!row.enabled); }
            else { assert!(op.rebuild_pending()); }
        }
        assert_eq!(op.owner_id, None);
    }
}
