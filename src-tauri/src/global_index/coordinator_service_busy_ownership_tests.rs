// Escalation diagnostics, NOT repaired-contract tests. They retain and expose
// master's unsafe behavior so green characterization cannot be called FIX READY.
use std::sync::mpsc::Receiver;

struct UnadmittedBusyProvider {
    source: GlobalSourceDescriptor,
    reached: SyncSender<&'static str>,
    release: Mutex<Receiver<()>>,
    blocked_once: AtomicBool,
}

impl UnadmittedBusyProvider {
    fn reject(&self, operation: &'static str) -> Result<(), GlobalIndexError> {
        // Public start leaves its existing Startup wake pending. Block only
        // the first call so that ordinary follow-up cycle is not a fake deadlock.
        if !self.blocked_once.swap(true, Ordering::AcqRel) {
            self.reached.send(operation).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(30))
                .expect("test must release the rejected request");
        }
        // No sink call/event/admitted indexing work occurs. On Windows this is
        // the production decoder applied to the existing exact v3 wire shape.
        #[cfg(target_os = "windows")]
        {
            let response = super::super::windows::service::IndexServiceResponse {
                protocol_version: 3,
                request_id: "ownership-diagnostic-rejected".into(),
                ok: false,
                error_code: Some("index_service_busy".into()),
                message: Some(SERVICE_BUSY_MESSAGE.into()),
                status: Some("indexing".into()),
            };
            let wire = serde_json::to_vec(&response).unwrap();
            let decoded = serde_json::from_slice(&wire).unwrap();
            Err(super::super::windows::decode_response_for_ownership_test(
                &decoded,
            ))
        }
        #[cfg(not(target_os = "windows"))]
        Err(GlobalIndexError::Provider(SERVICE_BUSY_MESSAGE.into()))
    }
}

impl GlobalIndexProvider for UnadmittedBusyProvider {
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        Ok(vec![self.source.clone()])
    }
    fn start_initial_index(
        &self,
        _: &GlobalSourceDescriptor,
        _: &mut dyn GlobalIndexSink,
        _: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.reject("initial")
    }
    fn resume_incremental_sync(
        &self,
        _: &GlobalSourceDescriptor,
        _: &mut dyn GlobalIndexSink,
        _: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.reject("resume")
    }
    fn rebuild(
        &self,
        _: &GlobalSourceDescriptor,
        _: &mut dyn GlobalIndexSink,
        _: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.reject("rebuild")
    }
    fn pause(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }
    fn status(&self) -> Result<String, GlobalIndexError> {
        Ok("test-rejection-only".into())
    }
    fn shutdown(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }
}

struct BusyReleaseGuard(Option<SyncSender<()>>);
impl BusyReleaseGuard {
    fn release(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
impl Drop for BusyReleaseGuard {
    fn drop(&mut self) {
        self.release();
    }
}

struct BusyCycleJoinGuard {
    release: BusyReleaseGuard,
    worker: Option<JoinHandle<Result<SourceTopologySignature, GlobalIndexError>>>,
}
impl BusyCycleJoinGuard {
    fn finish(&mut self) {
        self.release.release();
        self.worker.take().unwrap().join().unwrap().unwrap();
    }
}
impl Drop for BusyCycleJoinGuard {
    fn drop(&mut self) {
        self.release.release();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[test]
fn issue380_ownership_blocker_rebuild_mutates_before_service_admission() {
    let path = TestDatabasePath::new();
    let db = Database::open(&path.0).unwrap();
    let mut source = diagnostic_source(
        "ownership-rebuild",
        INDEX_STATUS_PERMISSION_REQUIRED,
        Some(1),
    );
    source.volume.last_error = Some("confirmed_access_denied".into());
    source.volume.journal_id = Some("confirmed-journal".into());
    source.volume.journal_cursor = Some("42".into());
    db.upsert_global_volume(&source.volume).unwrap();
    db.conn().unwrap().execute(
        "INSERT INTO global_entries(id, volume_id, platform_file_id, name, name_normalized, path, path_normalized, source_provider, last_seen_at) VALUES ('ownership-entry', ?1, 'file-1', 'metadata', 'metadata', 'test://metadata', 'test://metadata', ?2, 1)",
        rusqlite::params![source.volume.id, source.volume.provider],
    ).unwrap();
    let before = db.get_global_volume(&source.volume.id).unwrap().unwrap();
    let (reached_tx, reached_rx) = sync_channel(1);
    let (release_tx, release_rx) = sync_channel(1);
    let provider = Arc::new(UnadmittedBusyProvider {
        source: source.clone(),
        reached: reached_tx,
        release: Mutex::new(release_rx),
        blocked_once: AtomicBool::new(false),
    });
    let wake = Arc::new(GlobalIndexWakeSlot::default());
    let coordinator = GlobalIndexCoordinator::with_provider(db.clone(), provider, wake.clone());
    let _shutdown = CoordinatorShutdownGuard {
        coordinator: coordinator.clone(),
        active: true,
    };
    // Drop order releases the provider before shutdown joins its worker on panic.
    let mut release = BusyReleaseGuard(Some(release_tx));
    coordinator.rebuild(Some(source.volume.id.clone())).unwrap();
    assert_eq!(
        reached_rx.recv_timeout(Duration::from_secs(30)).unwrap(),
        "rebuild"
    );
    let unadmitted = db.get_global_volume(&source.volume.id).unwrap().unwrap();
    let stale: i64 = db
        .conn()
        .unwrap()
        .query_row(
            "SELECT is_stale FROM global_entries WHERE id = 'ownership-entry'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(before.index_status, INDEX_STATUS_PERMISSION_REQUIRED);
    assert_eq!(unadmitted.index_status, INDEX_STATUS_INDEXING);
    assert!(unadmitted.last_error.is_none());
    assert_eq!(
        stale, 1,
        "public rebuild stales rows before any provider sink call/admission"
    );
    assert_eq!(unadmitted.journal_cursor, before.journal_cursor);
    assert_eq!(unadmitted.last_full_index_at, before.last_full_index_at);
    assert_eq!(
        unadmitted.last_incremental_sync_at,
        before.last_incremental_sync_at
    );
    println!("issue380_ownership_blocker {{\"operation\":\"public_rebuild\",\"admitted\":false,\"prior\":\"permission_required\",\"before_response\":\"indexing\",\"prior_error_cleared\":true,\"entry_stale_before_admission\":true}}");
    release.release();
    wait_for_idle_wait(&wake, 0);
    let rejected = db.get_global_volume(&source.volume.id).unwrap().unwrap();
    assert_eq!(rejected.index_status, INDEX_STATUS_ERROR);
    assert!(rejected
        .last_error
        .as_deref()
        .unwrap()
        .contains(SERVICE_BUSY_MESSAGE));
    coordinator.shutdown().unwrap();
}

#[test]
fn issue380_ownership_blocker_late_busy_overwrites_committed_owner_health() {
    for (n, owner_status) in [
        INDEX_STATUS_READY,
        INDEX_STATUS_PERMISSION_REQUIRED,
        INDEX_STATUS_REBUILD_REQUIRED,
        INDEX_STATUS_ERROR,
    ]
    .into_iter()
    .enumerate()
    {
        let path = TestDatabasePath::new();
        let db = Database::open(&path.0).unwrap();
        let source =
            diagnostic_source(&format!("ownership-owner-{n}"), INDEX_STATUS_READY, Some(1));
        db.upsert_global_volume(&source.volume).unwrap();
        let (reached_tx, reached_rx) = sync_channel(1);
        let (release_tx, release_rx) = sync_channel(1);
        let provider = Arc::new(UnadmittedBusyProvider {
            source: source.clone(),
            reached: reached_tx,
            release: Mutex::new(release_rx),
            blocked_once: AtomicBool::new(false),
        });
        let wake = Arc::new(GlobalIndexWakeSlot::default());
        let worker = {
            let db = db.clone();
            let wake = wake.clone();
            thread::spawn(move || {
                run_index_cycle(
                    provider.as_ref(),
                    &db,
                    &Arc::new(AtomicBool::new(false)),
                    &wake,
                )
            })
        };
        let mut cycle = BusyCycleJoinGuard {
            release: BusyReleaseGuard(Some(release_tx)),
            worker: Some(worker),
        };
        assert_eq!(
            reached_rx.recv_timeout(Duration::from_secs(30)).unwrap(),
            "resume"
        );
        assert_eq!(
            db.get_global_volume(&source.volume.id)
                .unwrap()
                .unwrap()
                .index_status,
            INDEX_STATUS_SYNCING
        );
        // Commit a distinct owner result after the contender's speculative write
        // and before its busy reply. No sleep, timestamp collision or retry.
        db.update_global_volume_state(
            &source.volume.id,
            owner_status,
            (owner_status != INDEX_STATUS_READY).then_some("confirmed_owner_cause"),
            Some("owner-journal"),
            Some("99"),
            Some(7),
            Some(8),
        )
        .unwrap();
        let owner = db.get_global_volume(&source.volume.id).unwrap().unwrap();
        cycle.finish();
        let rejected = db.get_global_volume(&source.volume.id).unwrap().unwrap();
        assert_ne!(rejected.last_error, owner.last_error, "current generic busy rewrites another owner's cause even when degraded status survives");
        assert!(rejected
            .last_error
            .as_deref()
            .unwrap()
            .contains(SERVICE_BUSY_MESSAGE));
        if owner_status == INDEX_STATUS_READY || owner_status == INDEX_STATUS_ERROR {
            assert_eq!(rejected.index_status, INDEX_STATUS_ERROR);
        } else {
            assert_eq!(rejected.index_status, owner_status);
        }
        assert_eq!(rejected.journal_cursor, owner.journal_cursor);
        assert_eq!(rejected.last_full_index_at, owner.last_full_index_at);
        assert_eq!(
            rejected.last_incremental_sync_at,
            owner.last_incremental_sync_at
        );
        println!("issue380_ownership_blocker {{\"operation\":\"late_busy\",\"admitted\":false,\"committed_owner_status\":\"{owner_status}\",\"final_status\":\"{}\",\"committed_cause_overwritten\":true,\"owner_checkpoint_preserved\":true}}", rejected.index_status);
        assert_eq!(
            wake.snapshot().notifications,
            u64::from(owner_status == INDEX_STATUS_REBUILD_REQUIRED)
        );
    }
}
