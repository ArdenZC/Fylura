struct RecoveryOutcomeProvider {
    source: GlobalSourceDescriptor,
    outcome: u8,
    rebuilds: AtomicUsize,
}
impl GlobalIndexProvider for RecoveryOutcomeProvider {
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        Ok(vec![self.source.clone()])
    }
    fn start_initial_index(
        &self,
        _: &GlobalSourceDescriptor,
        _: &mut dyn GlobalIndexSink,
        _: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        unreachable!()
    }
    fn resume_incremental_sync(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        match self.outcome {
            0 => Err(GlobalIndexError::RebuildRequired("changed journal".into())),
            1 => {
                cancel.store(true, Ordering::Release);
                Err(GlobalIndexError::Paused)
            }
            2 => Err(GlobalIndexError::Paused),
            _ => {
                sink.set_source_state(
                    &source.volume.id,
                    INDEX_STATUS_PERMISSION_REQUIRED,
                    Some("access denied"),
                )?;
                Err(GlobalIndexError::Provider("access denied".into()))
            }
        }
    }
    fn rebuild(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        _: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.rebuilds.fetch_add(1, Ordering::AcqRel);
        sink.checkpoint(&source.volume.id, Some("new-journal"), Some("100"))?;
        Ok(())
    }
    fn pause(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }
    fn status(&self) -> Result<String, GlobalIndexError> {
        Ok("test".into())
    }
    fn shutdown(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }
}

#[test]
fn typed_recovery_survives_cycle_and_next_admitted_rebuild_reaches_ready() {
    for outcome in 0..4 {
        let path = TestDatabasePath::new();
        let db = Database::open(&path.0).unwrap();
        let source = GlobalSourceDescriptor {
            volume: GlobalVolume {
                id: "recovery-test".into(),
                platform: "windows".into(),
                stable_volume_id: "recovery-test".into(),
                display_name: "test".into(),
                mount_path: "test://metadata-only".into(),
                filesystem_type: "NTFS".into(),
                drive_kind: "fixed".into(),
                enabled: true,
                provider: PROVIDER_WINDOWS_MFT_USN.into(),
                index_status: INDEX_STATUS_READY.into(),
                last_error: None,
                journal_id: Some("old".into()),
                journal_cursor: Some("42".into()),
                last_full_index_at: Some(1),
                last_incremental_sync_at: Some(1),
                entry_count: 0,
                created_at: 1,
                updated_at: 1,
            },
        };
        db.upsert_global_volume(&source.volume).unwrap();
        let provider = RecoveryOutcomeProvider {
            source,
            outcome,
            rebuilds: AtomicUsize::new(0),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let wake = GlobalIndexWakeSlot::default();
        run_index_cycle(&provider, &db, &cancel, &wake).unwrap();
        let first = db.get_global_volume("recovery-test").unwrap().unwrap();
        match outcome {
            0 => {
                assert_eq!(first.index_status, INDEX_STATUS_REBUILD_REQUIRED);
                assert_eq!(wake.snapshot().notifications, 1);
                assert_eq!(provider.rebuilds.load(Ordering::Acquire), 0);
                run_index_cycle(&provider, &db, &cancel, &wake).unwrap();
                assert_eq!(provider.rebuilds.load(Ordering::Acquire), 1);
                let final_source = db.get_global_volume("recovery-test").unwrap().unwrap();
                assert_eq!(final_source.index_status, INDEX_STATUS_READY);
                assert_eq!(final_source.journal_cursor.as_deref(), Some("100"));
                assert!(final_source.last_full_index_at.unwrap() > 1);
            }
            1 | 2 => {
                assert_eq!(first.index_status, INDEX_STATUS_PAUSED);
                assert!(first.last_error.is_none());
            }
            _ => assert_eq!(first.index_status, INDEX_STATUS_PERMISSION_REQUIRED),
        }
    }
}

const SERVICE_BUSY_MESSAGE: &str = "Another global index operation is already running";
const DIAGNOSTIC_EVIDENCE_LIMIT: usize = 8;
const DIAGNOSTIC_ID_MAX_BYTES: usize = 128;

#[derive(Clone, Copy)]
enum DiagnosticOutcome {
    Success,
    ServiceBusy,
    PermissionDenied,
}

struct BoundedDiagnosticEvidence {
    events: std::collections::VecDeque<serde_json::Value>,
}

struct DiagnosticEvent<'a> {
    source_id: &'a str,
    operation_id: &'a str,
    request_id: &'a str,
    service_status: Option<&'a str>,
    request_result: &'a str,
    error_category: &'a str,
    persisted_source_status: &'a str,
}

impl BoundedDiagnosticEvidence {
    fn new() -> Self {
        Self {
            events: std::collections::VecDeque::new(),
        }
    }

    fn push(&mut self, event: DiagnosticEvent<'_>) {
        for (field, value) in [
            ("source_id", event.source_id),
            ("operation_id", event.operation_id),
            ("request_id", event.request_id),
        ] {
            assert!(
                value.len() <= DIAGNOSTIC_ID_MAX_BYTES,
                "diagnostic {field} exceeds the fixed evidence bound"
            );
        }
        assert_ne!(event.operation_id, event.request_id);
        if self.events.len() == DIAGNOSTIC_EVIDENCE_LIMIT {
            self.events.pop_front();
        }
        self.events.push_back(serde_json::json!({
            "source_id": event.source_id,
            "operation_id": event.operation_id,
            "request_id": event.request_id,
            "service_status": event.service_status,
            "request_result": event.request_result,
            "error_category": event.error_category,
            "persisted_source_status": event.persisted_source_status,
            "identity_kind": "test_fixture"
        }));
    }

    fn json(&self) -> String {
        serde_json::json!({
            "limit": DIAGNOSTIC_EVIDENCE_LIMIT,
            "events": &self.events
        })
        .to_string()
    }

    fn assert_sanitized(&self) {
        let evidence = self.json();
        assert!(!evidence.contains("mount_path"));
        assert!(!evidence.contains("test://"));
        assert!(!evidence.contains("file_content"));
    }

    fn emit(&self) {
        self.assert_sanitized();
        println!("issue328_diagnostic_evidence {}", self.json());
    }
}

fn diagnostic_source(
    id: &str,
    index_status: &str,
    last_full_index_at: Option<i64>,
) -> GlobalSourceDescriptor {
    GlobalSourceDescriptor {
        volume: GlobalVolume {
            id: id.to_string(),
            platform: "windows".to_string(),
            stable_volume_id: format!("test-volume-{id}"),
            display_name: format!("test-{id}"),
            mount_path: format!("test://{id}"),
            filesystem_type: "ntfs".to_string(),
            drive_kind: "fixed".to_string(),
            enabled: true,
            provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
            index_status: index_status.to_string(),
            last_error: None,
            journal_id: None,
            journal_cursor: None,
            last_full_index_at,
            last_incremental_sync_at: last_full_index_at,
            entry_count: 0,
            created_at: 1,
            updated_at: 1,
        },
    }
}

struct MultiVolumeDiagnosticProvider {
    sources: Vec<GlobalSourceDescriptor>,
    outcomes: std::sync::Mutex<
        std::collections::HashMap<String, std::collections::VecDeque<DiagnosticOutcome>>,
    >,
    operations: std::sync::Mutex<Vec<(String, &'static str)>>,
    operation_signal: Option<std::sync::mpsc::SyncSender<(String, &'static str)>>,
    operation_gate: Option<OperationGate>,
}

struct OperationGate {
    source_id: String,
    started: std::sync::mpsc::SyncSender<()>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    used: AtomicBool,
}

impl MultiVolumeDiagnosticProvider {
    fn new(
        sources: Vec<GlobalSourceDescriptor>,
        outcomes: std::collections::HashMap<
            String,
            std::collections::VecDeque<DiagnosticOutcome>,
        >,
        operation_signal: Option<std::sync::mpsc::SyncSender<(String, &'static str)>>,
        operation_gate: Option<OperationGate>,
    ) -> Self {
        Self {
            sources,
            outcomes: std::sync::Mutex::new(outcomes),
            operations: std::sync::Mutex::new(Vec::new()),
            operation_signal,
            operation_gate,
        }
    }

    fn run_operation(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        operation: &'static str,
    ) -> Result<(), GlobalIndexError> {
        let source_id = source.volume.id.clone();
        let outcome = self
            .outcomes
            .lock()
            .expect("diagnostic outcomes")
            .get_mut(&source_id)
            .and_then(std::collections::VecDeque::pop_front)
            .unwrap_or(DiagnosticOutcome::Success);
        if !matches!(outcome, DiagnosticOutcome::ServiceBusy) {
            let kind = match operation {
                "start_initial_index" => SourceOperationKind::Initial,
                "rebuild" => SourceOperationKind::Rebuild,
                _ => SourceOperationKind::Incremental,
            };
            sink.admit_source_operation(
                &source.volume,
                &uuid::Uuid::new_v4().to_string(),
                kind,
                SourceOperationRoute::Service,
            )?;
        }
        self.operations
            .lock()
            .expect("diagnostic operation log")
            .push((source_id.clone(), operation));
        if let Some(signal) = &self.operation_signal {
            let _ = signal.try_send((source_id.clone(), operation));
        }
        if let Some(gate) = &self.operation_gate {
            if gate.source_id == source_id && !gate.used.swap(true, Ordering::AcqRel) {
                gate.started
                    .send(())
                    .expect("multi-volume test observes the active source");
                gate.release
                    .lock()
                    .expect("multi-volume operation gate")
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .expect("multi-volume test releases the active source");
            }
        }
        match outcome {
            DiagnosticOutcome::Success => Ok(()),
            DiagnosticOutcome::ServiceBusy => {
                Err(GlobalIndexError::ServiceBusy(SERVICE_BUSY_MESSAGE.to_string()))
            }
            DiagnosticOutcome::PermissionDenied => {
                sink.set_source_state(
                    &source_id,
                    INDEX_STATUS_PERMISSION_REQUIRED,
                    Some("source_access_denied"),
                )?;
                Err(GlobalIndexError::Provider("source_access_denied".to_string()))
            }
        }
    }

    fn operation_count(&self, source_id: &str) -> usize {
        self.operations
            .lock()
            .expect("diagnostic operation log")
            .iter()
            .filter(|(id, _)| id.as_str() == source_id)
            .count()
    }
}

impl GlobalIndexProvider for MultiVolumeDiagnosticProvider {
    fn has_external_source_operation_admission(&self) -> bool {
        true
    }

    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        Ok(self.sources.clone())
    }

    fn start_initial_index(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        _: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.run_operation(source, sink, "start_initial_index")
    }

    fn resume_incremental_sync(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        _: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.run_operation(source, sink, "resume_incremental_sync")
    }

    fn rebuild(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        _: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.run_operation(source, sink, "rebuild")
    }

    fn pause(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }

    fn status(&self) -> Result<String, GlobalIndexError> {
        Ok("test".to_string())
    }

    fn shutdown(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }
}

#[test]
fn multivolume_busy_and_source_failures_keep_independent_durable_statuses() {
    let path = TestDatabasePath::new();
    let db = Database::open(&path.0).expect("open isolated multi-volume database");
    let source_a = diagnostic_source("source-a", INDEX_STATUS_READY, Some(1));
    let mut source_b = diagnostic_source("source-b", INDEX_STATUS_ERROR, Some(1));
    source_b.volume.last_error = Some("previous_service_busy".to_string());
    let mut source_c = diagnostic_source(
        "source-c",
        INDEX_STATUS_PERMISSION_REQUIRED,
        Some(1),
    );
    source_c.volume.last_error = Some("source_access_denied".to_string());
    let sources = vec![source_a, source_b, source_c];
    for source in &sources {
        db.upsert_global_volume(&source.volume)
            .expect("seed fixture source");
    }

    let outcomes = std::collections::HashMap::from([
        (
            "source-a".to_string(),
            std::collections::VecDeque::from([DiagnosticOutcome::Success]),
        ),
        (
            "source-b".to_string(),
            std::collections::VecDeque::from([
                DiagnosticOutcome::ServiceBusy,
                DiagnosticOutcome::Success,
            ]),
        ),
        (
            "source-c".to_string(),
            std::collections::VecDeque::from([DiagnosticOutcome::PermissionDenied]),
        ),
    ]);
    let (operation_started_tx, operation_started_rx) = std::sync::mpsc::sync_channel(1);
    let (release_operation_tx, release_operation_rx) = std::sync::mpsc::sync_channel(1);
    let provider = Arc::new(MultiVolumeDiagnosticProvider::new(
        sources,
        outcomes,
        None,
        Some(OperationGate {
            source_id: "source-a".to_string(),
            started: operation_started_tx,
            release: std::sync::Mutex::new(release_operation_rx),
            used: AtomicBool::new(false),
        }),
    ));
    let cancel = Arc::new(AtomicBool::new(false));
    let wake = Arc::new(GlobalIndexWakeSlot::default());

    let cycle_db = db.clone();
    let cycle_cancel = cancel.clone();
    let cycle_wake = wake.clone();
    let cycle_provider = provider.clone();
    let cycle = thread::spawn(move || {
        run_index_cycle(
            cycle_provider.as_ref(),
            &cycle_db,
            &cycle_cancel,
            &cycle_wake,
        )
    });
    operation_started_rx
        .recv_timeout(std::time::Duration::from_secs(3))
        .expect("first volume enters the active indexing operation");

    let active_source = db.get_global_volume("source-a").unwrap().unwrap();
    let prior_busy_source = db.get_global_volume("source-b").unwrap().unwrap();
    let prior_permission_source = db.get_global_volume("source-c").unwrap().unwrap();
    assert_eq!(active_source.index_status, INDEX_STATUS_SYNCING);
    let active_operation = db
        .global_index_operation_state("source-a")
        .unwrap()
        .unwrap();
    assert_eq!(active_operation.operation_state.as_deref(), Some("running"));
    assert!(active_operation.owner_id.is_some());
    assert_eq!(prior_busy_source.index_status, INDEX_STATUS_ERROR);
    assert_eq!(
        prior_busy_source.last_error.as_deref(),
        Some("previous_service_busy")
    );
    assert_eq!(
        prior_permission_source.index_status,
        INDEX_STATUS_PERMISSION_REQUIRED
    );
    assert_eq!(
        prior_permission_source.last_error.as_deref(),
        Some("source_access_denied")
    );
    let active_aggregate = db.global_index_status().expect("active aggregate status");
    assert_eq!(active_aggregate.status, INDEX_STATUS_ERROR);
    assert_eq!(active_aggregate.indexed_volumes, 3);
    assert_eq!(active_aggregate.ready_volumes, 0);
    assert_eq!(active_aggregate.pending_volumes, 1);
    assert_eq!(active_aggregate.total_entries, 0);
    assert!(!active_aggregate.collection_complete);

    let mut evidence = BoundedDiagnosticEvidence::new();
    evidence.push(DiagnosticEvent {
        source_id: "source-a",
        operation_id: "fixture-operation-a-active",
        request_id: "fixture-request-a-active",
        service_status: None,
        request_result: "accepted",
        error_category: "none",
        persisted_source_status: &active_source.index_status,
    });
    evidence.push(DiagnosticEvent {
        source_id: "source-b",
        operation_id: "fixture-operation-b-prior",
        request_id: "fixture-request-b-prior",
        service_status: None,
        request_result: "prior_rejected",
        error_category: "service_busy",
        persisted_source_status: &prior_busy_source.index_status,
    });
    evidence.push(DiagnosticEvent {
        source_id: "source-c",
        operation_id: "fixture-operation-c-prior",
        request_id: "fixture-request-c-prior",
        service_status: None,
        request_result: "prior_failed",
        error_category: "source_permission_error",
        persisted_source_status: &prior_permission_source.index_status,
    });

    release_operation_tx
        .send(())
        .expect("release active first volume");
    cycle
        .join()
        .expect("multi-volume cycle thread remains alive")
        .expect("source-level outcomes do not abort the remaining volume cycle");

    let source_a = db.get_global_volume("source-a").unwrap().unwrap();
    let source_b = db.get_global_volume("source-b").unwrap().unwrap();
    let source_c = db.get_global_volume("source-c").unwrap().unwrap();
    assert_eq!(source_a.index_status, INDEX_STATUS_READY);
    assert_eq!(source_b.index_status, INDEX_STATUS_ERROR);
    assert_eq!(source_b.last_error.as_deref(), Some("previous_service_busy"));
    assert_eq!(
        source_c.index_status,
        INDEX_STATUS_PERMISSION_REQUIRED,
        "a genuine source error keeps its typed source state"
    );
    assert_eq!(provider.operation_count("source-a"), 1);
    assert_eq!(provider.operation_count("source-b"), 1);
    assert_eq!(provider.operation_count("source-c"), 1);
    assert_eq!(wake.snapshot().notifications, 0);

    let aggregate = db.global_index_status().expect("aggregate source status");
    assert_eq!(aggregate.status, "partial");
    assert_eq!(aggregate.indexed_volumes, 3);
    assert_eq!(aggregate.ready_volumes, 1);
    assert_eq!(aggregate.pending_volumes, 0);
    assert!(!aggregate.collection_complete);

    evidence.push(DiagnosticEvent {
        source_id: "source-a",
        operation_id: "fixture-operation-a-cycle-1",
        request_id: "fixture-request-a-cycle-1",
        service_status: None,
        request_result: "accepted",
        error_category: "none",
        persisted_source_status: &source_a.index_status,
    });
    evidence.push(DiagnosticEvent {
        source_id: "source-b",
        operation_id: "fixture-operation-b-cycle-1",
        request_id: "fixture-request-b-cycle-1",
        service_status: Some("indexing"),
        request_result: "rejected",
        error_category: "service_busy",
        persisted_source_status: &source_b.index_status,
    });
    evidence.push(DiagnosticEvent {
        source_id: "source-c",
        operation_id: "fixture-operation-c-cycle-1",
        request_id: "fixture-request-c-cycle-1",
        service_status: None,
        request_result: "failed",
        error_category: "source_permission_error",
        persisted_source_status: &source_c.index_status,
    });
    evidence.emit();

    // The explicit second cycle is the only retry exercised here. The cycle
    // itself neither retries the busy volume nor signals an automatic retry.
    wake.notify(GlobalIndexWakeReason::ExplicitCommand);
    run_index_cycle(provider.as_ref(), &db, &cancel, &wake)
        .expect("a later explicit cycle can retry the busy source");
    let retried_source = db.get_global_volume("source-b").unwrap().unwrap();
    assert_eq!(retried_source.index_status, INDEX_STATUS_READY);
    assert_eq!(provider.operation_count("source-b"), 2);
    assert_eq!(provider.operation_count("source-c"), 2);
    evidence.push(DiagnosticEvent {
        source_id: "source-b",
        operation_id: "fixture-operation-b-retry",
        request_id: "fixture-request-b-retry",
        service_status: None,
        request_result: "explicit_wake_succeeded",
        error_category: "none",
        persisted_source_status: &retried_source.index_status,
    });
    evidence.emit();

    drop(db);
    drop(path);
}

#[test]
fn diagnostic_evidence_is_bounded_and_excludes_paths_and_content() {
    let mut evidence = BoundedDiagnosticEvidence::new();
    for index in 0..(DIAGNOSTIC_EVIDENCE_LIMIT + 3) {
        evidence.push(DiagnosticEvent {
            source_id: &format!("source-{index}"),
            operation_id: &format!("fixture-operation-{index}"),
            request_id: &format!("fixture-request-{index}"),
            service_status: Some("indexing"),
            request_result: "rejected",
            error_category: "service_busy",
            persisted_source_status: INDEX_STATUS_ERROR,
        });
    }

    assert_eq!(evidence.events.len(), DIAGNOSTIC_EVIDENCE_LIMIT);
    let serialized = evidence.json();
    assert!(serialized.contains("source_id"));
    assert!(serialized.contains("operation_id"));
    assert!(serialized.contains("request_id"));
    assert!(serialized.contains("service_status"));
    assert!(serialized.contains("request_result"));
    assert!(serialized.contains("error_category"));
    assert!(!serialized.contains("mount_path"));
    assert!(!serialized.contains("test://"));
    assert!(!serialized.contains("file_content"));
    evidence.emit();
}

struct BlockingLifecycleProvider {
    source: GlobalSourceDescriptor,
    operations: std::sync::Mutex<Vec<&'static str>>,
    first_started: std::sync::mpsc::SyncSender<()>,
    release_first: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    active_operations: AtomicUsize,
    max_active_operations: AtomicUsize,
}

impl BlockingLifecycleProvider {
    fn run_operation(&self, operation: &'static str) -> Result<(), GlobalIndexError> {
        let active = self.active_operations.fetch_add(1, Ordering::AcqRel) + 1;
        self.max_active_operations
            .fetch_max(active, Ordering::AcqRel);
        let call_number = {
            let mut operations = self.operations.lock().expect("lifecycle operation log");
            operations.push(operation);
            operations.len()
        };
        if call_number == 1 {
            self.first_started
                .send(())
                .expect("lifecycle test waits for first operation");
            self.release_first
                .lock()
                .expect("lifecycle release receiver")
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("test releases the first operation");
        }
        self.active_operations.fetch_sub(1, Ordering::AcqRel);
        Ok(())
    }

    fn operation_log(&self) -> Vec<&'static str> {
        self.operations
            .lock()
            .expect("lifecycle operation log")
            .clone()
    }
}

impl GlobalIndexProvider for BlockingLifecycleProvider {
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        Ok(vec![self.source.clone()])
    }

    fn start_initial_index(
        &self,
        _: &GlobalSourceDescriptor,
        _: &mut dyn GlobalIndexSink,
        _: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.run_operation("start_initial_index")
    }

    fn resume_incremental_sync(
        &self,
        _: &GlobalSourceDescriptor,
        _: &mut dyn GlobalIndexSink,
        _: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.run_operation("resume_incremental_sync")
    }

    fn rebuild(
        &self,
        _: &GlobalSourceDescriptor,
        _: &mut dyn GlobalIndexSink,
        _: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.run_operation("rebuild")
    }

    fn pause(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }

    fn status(&self) -> Result<String, GlobalIndexError> {
        Ok("test".to_string())
    }

    fn shutdown(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }
}

#[test]
fn repeated_same_source_lifecycle_requests_are_serialized_and_rebuild_wakes_coalesce() {
    let path = TestDatabasePath::new();
    let db = Database::open(&path.0).expect("open isolated lifecycle database");
    let source = diagnostic_source("source-lifecycle", INDEX_STATUS_DISCOVERED, None);
    db.upsert_global_volume(&source.volume)
        .expect("seed lifecycle source");
    let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let provider = Arc::new(BlockingLifecycleProvider {
        source: source.clone(),
        operations: std::sync::Mutex::new(Vec::new()),
        first_started: started_tx,
        release_first: std::sync::Mutex::new(release_rx),
        active_operations: AtomicUsize::new(0),
        max_active_operations: AtomicUsize::new(0),
    });
    let wake = Arc::new(GlobalIndexWakeSlot::default());
    let coordinator =
        GlobalIndexCoordinator::with_provider(db.clone(), provider.clone(), wake.clone());
    let mut shutdown_guard = CoordinatorShutdownGuard {
        coordinator: coordinator.clone(),
        active: true,
    };

    coordinator.start().expect("start coordinator");
    started_rx
        .recv_timeout(std::time::Duration::from_secs(3))
        .expect("initial Start request enters provider");

    coordinator.start().expect("repeat Start while operation is active");
    coordinator.resume().expect("Resume while operation is active");
    coordinator
        .rebuild(Some(source.volume.id.clone()))
        .expect("first rebuild request while operation is active");
    coordinator
        .rebuild(Some(source.volume.id.clone()))
        .expect("duplicate rebuild request while operation is active");
    release_tx.send(()).expect("release in-flight Start request");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while provider.operation_log().len() < 2 && std::time::Instant::now() < deadline {
        thread::yield_now();
    }
    wait_for_idle_wait(&wake, 0);

    assert_eq!(
        provider.operation_log(),
        vec!["start_initial_index", "rebuild"],
        "repeated lifecycle signals coalesce to one follow-up cycle for this source"
    );
    assert_eq!(provider.max_active_operations.load(Ordering::Acquire), 1);
    assert_eq!(
        db.get_global_volume(&source.volume.id)
            .unwrap()
            .unwrap()
            .index_status,
        INDEX_STATUS_READY
    );

    let mut evidence = BoundedDiagnosticEvidence::new();
    evidence.push(DiagnosticEvent {
        source_id: &source.volume.id,
        operation_id: "fixture-operation-lifecycle-1",
        request_id: "fixture-request-start-1",
        service_status: Some("indexing"),
        request_result: "accepted",
        error_category: "none",
        persisted_source_status: "indexing",
    });
    evidence.push(DiagnosticEvent {
        source_id: &source.volume.id,
        operation_id: "fixture-operation-lifecycle-2",
        request_id: "fixture-request-rebuild-1",
        service_status: Some("indexing"),
        request_result: "accepted_after_coalesced_wake",
        error_category: "none",
        persisted_source_status: INDEX_STATUS_READY,
    });
    evidence.emit();

    coordinator.shutdown().expect("shutdown lifecycle coordinator");
    shutdown_guard.active = false;
    drop(coordinator);
    drop(db);
    drop(path);
}

#[test]
fn busy_source_waits_for_a_new_wake_before_an_explicit_retry() {
    let path = TestDatabasePath::new();
    let db = Database::open(&path.0).expect("open isolated retry database");
    let source = diagnostic_source("source-retry", INDEX_STATUS_READY, Some(1));
    db.upsert_global_volume(&source.volume)
        .expect("seed retry source");
    let (operation_tx, operation_rx) = std::sync::mpsc::sync_channel(2);
    let outcomes = std::collections::HashMap::from([(
        source.volume.id.clone(),
        std::collections::VecDeque::from([
            DiagnosticOutcome::ServiceBusy,
            DiagnosticOutcome::Success,
        ]),
    )]);
    let provider = Arc::new(MultiVolumeDiagnosticProvider::new(
        vec![source.clone()],
        outcomes,
        Some(operation_tx),
        None,
    ));
    let cancel = Arc::new(AtomicBool::new(false));
    let wake = Arc::new(GlobalIndexWakeSlot::default());
    let worker = {
        let db = db.clone();
        let cancel = cancel.clone();
        let wake = wake.clone();
        let provider = provider.clone();
        thread::spawn(move || run_index(provider, db, cancel, wake))
    };

    assert_eq!(
        operation_rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("initial busy attempt"),
        (source.volume.id.clone(), "resume_incremental_sync")
    );
    wait_for_idle_wait(&wake, 0);
    assert_eq!(provider.operation_count(&source.volume.id), 1);
    assert_eq!(wake.snapshot().notifications, 0);
    assert_eq!(
        db.get_global_volume(&source.volume.id)
            .unwrap()
            .unwrap()
            .index_status,
        INDEX_STATUS_READY
    );

    let prior_blocked_waits = wake.snapshot().blocked_waits;
    wake.notify(GlobalIndexWakeReason::ExplicitCommand);
    assert_eq!(
        operation_rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("explicit wake triggers one retry"),
        (source.volume.id.clone(), "resume_incremental_sync")
    );
    wait_for_idle_wait(&wake, prior_blocked_waits);
    let recovered = db.get_global_volume(&source.volume.id).unwrap().unwrap();
    assert_eq!(recovered.index_status, INDEX_STATUS_READY);
    assert_eq!(provider.operation_count(&source.volume.id), 2);

    cancel.store(true, Ordering::Release);
    wake.notify(GlobalIndexWakeReason::Shutdown);
    worker
        .join()
        .expect("join retry lifecycle worker")
        .expect("retry lifecycle exits cleanly");

    drop(db);
    drop(path);
}

#[test]
fn sqlite_busy_while_finishing_releases_the_local_owner_for_service_recovery() {
    let path = TestDatabasePath::new();
    let db = Database::open(&path.0).expect("open completion contention database");
    let source = diagnostic_source("source-finish-busy", INDEX_STATUS_READY, Some(1));
    db.upsert_global_volume(&source.volume)
        .expect("seed source health");
    db.set_test_busy_timeout_for_all_connections(
        5_000,
        std::time::Duration::from_millis(20),
    )
    .expect("bound lock wait for this contention regression");

    let mut sink = DatabaseIndexSink {
        db: db.clone(),
        owners: HashMap::new(),
    };
    sink.admit_source_operation(
        &source.volume,
        "service-operation-with-contended-completion",
        SourceOperationKind::Incremental,
        SourceOperationRoute::Service,
    )
    .expect("persist service admission before provider effects");

    let blocker = rusqlite::Connection::open(&path.0).expect("open SQLite writer contender");
    blocker
        .execute_batch("BEGIN IMMEDIATE")
        .expect("hold the SQLite writer lock");
    let result = sink.finish_source_operation(
        &source.volume.id,
        INDEX_STATUS_READY,
        None,
        None,
        Some(2),
        false,
    );
    assert!(matches!(
        result,
        Err(GlobalIndexError::Database(DbError::Sqlite(
            rusqlite::Error::SqliteFailure(error, _)
        ))) if error.code == rusqlite::ErrorCode::DatabaseBusy
    ));
    drop(sink);
    blocker
        .execute_batch("ROLLBACK")
        .expect("release the SQLite writer lock");

    let before_retry = db
        .get_global_volume(&source.volume.id)
        .expect("read committed health after lock release")
        .expect("source remains present");
    assert_eq!(before_retry.index_status, INDEX_STATUS_SYNCING);
    assert_eq!(before_retry.last_error, None);

    let mut retry_sink = DatabaseIndexSink {
        db: db.clone(),
        owners: HashMap::new(),
    };
    retry_sink
        .admit_source_operation(
            &source.volume,
            "service-operation-after-contended-completion",
            SourceOperationKind::Incremental,
            SourceOperationRoute::Service,
        )
        .expect("a fresh admitted service operation reconciles the old owner");
    retry_sink
        .finish_source_operation(
            &source.volume.id,
            INDEX_STATUS_READY,
            None,
            None,
            Some(3),
            false,
        )
        .expect("successful follow-up completion commits source health");
    assert_eq!(
        db.get_global_volume(&source.volume.id)
            .expect("read completed source")
            .expect("source remains present")
            .index_status,
        INDEX_STATUS_READY
    );
}
