//! Actual ServiceRuntime -> PipeSink -> production exchange/admission path.
//! Controlled providers are test doubles, not native MFT or released binaries.
use super::super::service::p1_tests::{owned_entry, source, Sink};
use super::super::service::{
    exchange, IndexServiceAdmission, IndexServiceCallError, IndexServiceFrame, IndexServiceIo,
};
use super::*;
use std::path::PathBuf;
use std::sync::{mpsc, Barrier, Mutex};
use std::time::Duration;

struct Provider {
    gate: Option<(Arc<Barrier>, Arc<Barrier>)>,
    fault: Option<&'static str>,
}
impl GlobalIndexProvider for Provider {
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        Ok(vec![GlobalSourceDescriptor { volume: source() }])
    }
    fn start_initial_index(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.scan(source, sink, cancel, true)
    }
    fn resume_incremental_sync(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.scan(source, sink, cancel, false)
    }
    fn pause(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }
    fn status(&self) -> Result<String, GlobalIndexError> {
        Ok("controlled-test-provider".into())
    }
    fn shutdown(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }
}
impl Provider {
    fn scan(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
        full: bool,
    ) -> Result<(), GlobalIndexError> {
        if let Some((entered, release)) = &self.gate {
            entered.wait();
            release.wait();
        }
        if cancel.load(Ordering::Acquire) {
            return Err(GlobalIndexError::Paused);
        }
        sink.set_source_provider(&source.volume.id, PROVIDER_WINDOWS_MFT_USN)?;
        if self.fault == Some("lookup") {
            let entry = sink
                .find_entry_by_identity(&source.volume.id, "f", "p", "name")?
                .expect("controlled owned entry");
            sink.mark_entry_stale(&entry.id)?;
        } else if let Some(fault) = self.fault {
            return Err(match fault {
                "permission" => GlobalIndexError::PermissionRequired("same message".into()),
                "io" => GlobalIndexError::WindowsIo(32),
                "windows_io_5" => GlobalIndexError::WindowsIo(5),
                "windows_io_1314" => GlobalIndexError::WindowsIo(1314),
                "sqlite_busy" => actual_sqlite_busy_error(),
                _ => GlobalIndexError::Provider("same message".into()),
            });
        }
        if full {
            sink.mark_volume_entries_stale(&source.volume.id)?;
        }
        sink.checkpoint(&source.volume.id, Some("7"), Some("42"))
    }
}

fn actual_sqlite_busy_error() -> GlobalIndexError {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "zen-canvas-service-host-busy-{}-{}.sqlite3",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let writer = rusqlite::Connection::open(&path).expect("open isolated SQLite lock holder");
    writer
        .execute_batch("CREATE TABLE busy_probe(value INTEGER); INSERT INTO busy_probe VALUES(1);")
        .expect("prepare isolated SQLite lock holder");
    let contender = rusqlite::Connection::open(&path).expect("open isolated SQLite contender");
    contender
        .busy_timeout(Duration::ZERO)
        .expect("disable wait in isolated contention fixture");
    writer
        .execute_batch("BEGIN IMMEDIATE")
        .expect("hold isolated SQLite write lock");
    let error = contender
        .execute("UPDATE busy_probe SET value=2", [])
        .expect_err("second isolated writer returns SQLite BUSY");
    assert!(matches!(
        &error,
        rusqlite::Error::SqliteFailure(code, _) if code.code == rusqlite::ErrorCode::DatabaseBusy
    ));
    writer
        .execute_batch("ROLLBACK")
        .expect("release isolated SQLite lock");
    drop(contender);
    drop(writer);
    std::fs::remove_file(path).expect("remove only this test's SQLite database");
    GlobalIndexError::Database(error.into())
}

struct CoordinatorTestDb(PathBuf);

impl CoordinatorTestDb {
    fn new() -> Self {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "zen-canvas-service-coordinator-{}-{}.sqlite3",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        assert!(!path.exists(), "task-owned fixture path must be unique");
        Self(path)
    }
}

impl Drop for CoordinatorTestDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut path = self.0.as_os_str().to_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(path));
        }
    }
}

#[test]
fn p1_service_sqlite_busy_survives_ipc_v4_and_is_not_service_busy() {
    let runtime = runtime(Provider {
        gate: None,
        fault: Some("sqlite_busy"),
    });
    let mut volume = source();
    volume.last_full_index_at = Some(1);
    volume.journal_id = Some("7".into());
    volume.journal_cursor = Some("42".into());
    let descriptor = GlobalSourceDescriptor {
        volume: volume.clone(),
    };
    let request = IndexServiceRequest::new(
        IndexServiceCommand::ResumeIncrementalSync {
            source_id: volume.id.clone(),
        },
        Some(volume.clone()),
    );
    let (mut client, handle) = start(runtime, request.clone());
    client.serialize_frames = true;
    let captured_response = client.captured_response.clone();
    let coordinator_provider = DesktopCoordinatorProvider {
        windows: super::super::WindowsGlobalIndexProvider::new(),
        source: descriptor,
        request,
        io: Mutex::new(Some(client)),
    };
    let fixture = CoordinatorTestDb::new();
    let db = crate::db::Database::open(&fixture.0).expect("open isolated coordinator database");
    db.upsert_global_volume(&volume)
        .expect("seed already-authorized test source");
    crate::global_index::coordinator::run_index_cycle_for_test(
        &coordinator_provider,
        &db,
        &Arc::new(AtomicBool::new(false)),
        &crate::global_index::wake::GlobalIndexWakeSlot::default(),
    )
    .expect("Desktop coordinator records the service response");
    handle.join().unwrap().unwrap();

    let response = captured_response
        .lock()
        .expect("captured IPC response")
        .clone()
        .expect("desktop received the service-host response");
    assert_eq!(
        response.protocol_version,
        super::super::service::IPC_PROTOCOL_VERSION
    );
    assert!(!response.ok);
    assert_eq!(
        response.error_code.as_deref(),
        Some("index_database_busy"),
        "SQLite BUSY must retain its structured identity across IPC"
    );
    assert_eq!(
        response.status.as_deref(),
        Some(crate::global_index::models::INDEX_STATUS_DATABASE_BUSY),
        "the terminal IPC status must describe the failure, not the request still unwinding"
    );
    let decoded = super::super::service_errors::decode(&response);
    assert!(matches!(decoded, GlobalIndexError::DatabaseBusy(_)));
    assert_eq!(
        crate::global_index::commands::global_index_error_message(decoded),
        "global_index_database_busy",
        "the UI-facing command reports SQLite BUSY"
    );
    assert_eq!(
        db.get_global_volume(&volume.id)
            .unwrap()
            .unwrap()
            .index_status,
        crate::global_index::models::INDEX_STATUS_DATABASE_BUSY,
        "the Desktop coordinator persists SQLite BUSY separately from Service Busy"
    );
}
struct Connection {
    id: String,
    frames: mpsc::Sender<IndexServiceFrame>,
    lookups: mpsc::Receiver<IndexServiceLookupResponse>,
    response_gate: Option<(Arc<Barrier>, Arc<Barrier>)>,
    response_failure: bool,
}
impl IndexServiceConnection for Connection {
    fn send_event(&mut self, event: IndexServiceEvent) -> Result<(), String> {
        self.frames
            .send(IndexServiceFrame::Event {
                request_id: self.id.clone(),
                event,
            })
            .map_err(|e| e.to_string())
    }
    fn send_response(&mut self, response: IndexServiceResponse) -> Result<(), String> {
        if let Some((entered, release)) = &self.response_gate {
            entered.wait();
            release.wait();
        }
        if self.response_failure {
            return Err("controlled terminal response send failure".into());
        }
        self.frames
            .send(IndexServiceFrame::Response { response })
            .map_err(|e| e.to_string())
    }
    fn read_lookup_response(&mut self, id: &str) -> Result<IndexServiceLookupResponse, String> {
        let response = self
            .lookups
            .recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|e| e.to_string())?;
        let actual = match &response {
            IndexServiceLookupResponse::ParentPath { lookup_id, .. }
            | IndexServiceLookupResponse::Entry { lookup_id, .. } => lookup_id,
        };
        if actual != id {
            return Err("lookup identity mismatch".into());
        }
        Ok(response)
    }
}
struct Client {
    frames: mpsc::Receiver<IndexServiceFrame>,
    lookups: mpsc::Sender<IndexServiceLookupResponse>,
    prefetched: Option<IndexServiceFrame>,
    serialize_frames: bool,
    captured_response: Arc<Mutex<Option<IndexServiceResponse>>>,
}
impl IndexServiceIo for Client {
    fn send_request(&mut self, request: &IndexServiceRequest) -> Result<(), IndexServiceCallError> {
        if self.serialize_frames {
            let bytes = serde_json::to_vec(request)
                .map_err(|error| IndexServiceCallError::Protocol(error.to_string()))?;
            let _: IndexServiceRequest = serde_json::from_slice(&bytes)
                .map_err(|error| IndexServiceCallError::Protocol(error.to_string()))?;
        }
        Ok(())
    }
    fn read_frame(&mut self) -> Result<IndexServiceFrame, IndexServiceCallError> {
        let mut frame = if let Some(frame) = self.prefetched.take() {
            frame
        } else {
            self.frames
                .recv_timeout(std::time::Duration::from_secs(10))
                .map_err(|e| IndexServiceCallError::UnknownOutcome(e.to_string()))?
        };
        if self.serialize_frames {
            let bytes = serde_json::to_vec(&frame)
                .map_err(|error| IndexServiceCallError::Protocol(error.to_string()))?;
            frame = serde_json::from_slice(&bytes)
                .map_err(|error| IndexServiceCallError::Protocol(error.to_string()))?;
        }
        if let IndexServiceFrame::Response { response } = &frame {
            *self
                .captured_response
                .lock()
                .expect("captured terminal response") = Some(response.clone());
        }
        Ok(frame)
    }
    fn send_lookup(&mut self, response: IndexServiceLookupResponse) -> Result<(), String> {
        self.lookups.send(response).map_err(|e| e.to_string())
    }
}
fn start(
    runtime: Arc<ServiceRuntime>,
    request: IndexServiceRequest,
) -> (Client, std::thread::JoinHandle<Result<(), String>>) {
    start_with_response_gate(runtime, request, None, false)
}
fn start_with_response_gate(
    runtime: Arc<ServiceRuntime>,
    request: IndexServiceRequest,
    response_gate: Option<(Arc<Barrier>, Arc<Barrier>)>,
    response_failure: bool,
) -> (Client, std::thread::JoinHandle<Result<(), String>>) {
    let (tx, rx) = mpsc::channel();
    let (ltx, lrx) = mpsc::channel();
    let captured_response = Arc::new(Mutex::new(None));
    let mut connection = Connection {
        id: request.request_id.clone(),
        frames: tx,
        lookups: lrx,
        response_gate,
        response_failure,
    };
    let handle = std::thread::spawn(move || runtime.handle_request(request, &mut connection));
    (
        Client {
            frames: rx,
            lookups: ltx,
            prefetched: None,
            serialize_frames: false,
            captured_response,
        },
        handle,
    )
}
fn runtime(provider: Provider) -> Arc<ServiceRuntime> {
    let mut runtime = ServiceRuntime::new();
    runtime.provider = Box::new(provider);
    Arc::new(runtime)
}

struct DesktopCoordinatorProvider {
    windows: super::super::WindowsGlobalIndexProvider,
    source: GlobalSourceDescriptor,
    request: IndexServiceRequest,
    io: Mutex<Option<Client>>,
}

impl DesktopCoordinatorProvider {
    fn run(&self, sink: &mut dyn GlobalIndexSink) -> Result<(), GlobalIndexError> {
        let mut io = self.io.lock().expect("coordinator test IPC client");
        let io = io.as_mut().expect("single-use coordinator test IPC");
        self.windows
            .run_with_test_io_and_fallback(&self.request, sink, io, |_, _| {
                Err(GlobalIndexError::Provider(
                    "unexpected Direct fallback".into(),
                ))
            })
    }
}

impl GlobalIndexProvider for DesktopCoordinatorProvider {
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        Ok(vec![self.source.clone()])
    }

    fn start_initial_index(
        &self,
        _source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        _cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.run(sink)
    }

    fn resume_incremental_sync(
        &self,
        _source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        _cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.run(sink)
    }

    fn pause(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }

    fn status(&self) -> Result<String, GlobalIndexError> {
        Ok(PROVIDER_WINDOWS_MFT_USN.into())
    }

    fn shutdown(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }

    fn has_external_source_operation_admission(&self) -> bool {
        true
    }
}
fn finish(
    request: &IndexServiceRequest,
    client: &mut Client,
    sink: &mut Sink,
) -> Result<IndexServiceResponse, IndexServiceCallError> {
    let mut admission = IndexServiceAdmission::new(request);
    let response = exchange(request, client, |event| admission.apply(sink, event))?;
    admission.finish(&response)?;
    Ok(response)
}
#[test]
fn p1_runtime_initial_resume_and_rebuild_complete_through_real_pipe_sink_and_client() {
    for command in [
        IndexServiceCommand::StartInitialIndex {
            source_id: "v".into(),
        },
        IndexServiceCommand::ResumeIncrementalSync {
            source_id: "v".into(),
        },
        IndexServiceCommand::Rebuild {
            source_id: "v".into(),
        },
    ] {
        let runtime = runtime(Provider {
            gate: None,
            fault: None,
        });
        let request = IndexServiceRequest::new(command, Some(source()));
        let (mut client, handle) = start(runtime.clone(), request.clone());
        let mut sink = Sink::default();
        assert!(finish(&request, &mut client, &mut sink).unwrap().ok);
        handle.join().unwrap().unwrap();
        assert!(!runtime.busy.load(Ordering::Acquire));
        assert_eq!(
            sink.writes,
            if matches!(
                request.command,
                IndexServiceCommand::ResumeIncrementalSync { .. }
            ) {
                2
            } else {
                3
            }
        );
    }
}
#[test]
fn p1_runtime_busy_competitors_have_no_admission_or_sink_effects() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let runtime = runtime(Provider {
        gate: Some((entered.clone(), release.clone())),
        fault: None,
    });
    let owner = IndexServiceRequest::new(
        IndexServiceCommand::StartInitialIndex {
            source_id: "v".into(),
        },
        Some(source()),
    );
    let (mut owner_client, owner_handle) = start(runtime.clone(), owner.clone());
    entered.wait();
    for command in [
        IndexServiceCommand::StartInitialIndex {
            source_id: "v".into(),
        },
        IndexServiceCommand::ResumeIncrementalSync {
            source_id: "v".into(),
        },
        IndexServiceCommand::Rebuild {
            source_id: "v".into(),
        },
    ] {
        let request = IndexServiceRequest::new(command, Some(source()));
        let (mut client, handle) = start(runtime.clone(), request.clone());
        let mut sink = Sink::default();
        let response = finish(&request, &mut client, &mut sink).unwrap();
        assert!(matches!(
            super::super::service_errors::decode(&response),
            GlobalIndexError::ServiceBusy(_)
        ));
        assert_eq!(sink.writes, 0);
        handle.join().unwrap().unwrap();
    }
    release.wait();
    assert!(
        finish(&owner, &mut owner_client, &mut Sink::default())
            .unwrap()
            .ok
    );
    owner_handle.join().unwrap().unwrap();
}

#[test]
fn p1_runtime_busy_reservation_covers_terminal_response_send_success_and_failure() {
    for response_failure in [false, true] {
        let runtime = runtime(Provider {
            gate: None,
            fault: None,
        });
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let owner = IndexServiceRequest::new(
            IndexServiceCommand::StartInitialIndex {
                source_id: "v".into(),
            },
            Some(source()),
        );
        let (mut owner_client, owner_handle) = start_with_response_gate(
            runtime.clone(),
            owner.clone(),
            Some((entered.clone(), release.clone())),
            response_failure,
        );
        entered.wait();
        assert!(runtime.busy.load(Ordering::Acquire));

        let contender = IndexServiceRequest::new(
            IndexServiceCommand::Rebuild {
                source_id: "v".into(),
            },
            Some(source()),
        );
        let (mut contender_client, contender_handle) = start(runtime.clone(), contender.clone());
        let early_frame = contender_client
            .frames
            .recv_timeout(std::time::Duration::from_secs(1))
            .ok();
        let busy_before_owner_send_ends = matches!(
            &early_frame,
            Some(IndexServiceFrame::Response { response })
                if !response.ok && response.error_code.as_deref() == Some("index_service_busy")
        );
        if let Some(frame) = early_frame {
            contender_client.prefetched = Some(frame);
        }

        release.wait();
        let owner_result = owner_handle.join().expect("owner runtime thread");
        assert_eq!(owner_result.is_err(), response_failure);
        if !response_failure {
            let response = finish(&owner, &mut owner_client, &mut Sink::default()).unwrap();
            assert!(response.ok);
        }

        let mut contender_sink = Sink::default();
        let contender_response = finish(&contender, &mut contender_client, &mut contender_sink)
            .expect("contender receives a terminal response");
        contender_handle
            .join()
            .expect("contender runtime thread")
            .expect("busy response sends successfully");

        assert!(busy_before_owner_send_ends);
        assert!(matches!(
            super::super::service_errors::decode(&contender_response),
            GlobalIndexError::ServiceBusy(_)
        ));
        assert_eq!(contender_sink.writes, 0);
        assert!(!runtime.busy.load(Ordering::Acquire));

        let probe = IndexServiceRequest::new(
            IndexServiceCommand::Rebuild {
                source_id: "v".into(),
            },
            Some(source()),
        );
        let probe_reservation = runtime
            .try_begin_index_request(&probe)
            .expect("slot released after terminal response attempt");
        drop(probe_reservation);
        assert!(!runtime.busy.load(Ordering::Acquire));
    }
}
#[test]
fn p1_runtime_cancel_and_shutdown_do_not_fabricate_completed_scan() {
    for shutdown in [false, true] {
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let runtime = runtime(Provider {
            gate: Some((entered.clone(), release.clone())),
            fault: None,
        });
        let request = IndexServiceRequest::new(
            IndexServiceCommand::Rebuild {
                source_id: "v".into(),
            },
            Some(source()),
        );
        let (mut client, handle) = start(runtime.clone(), request.clone());
        entered.wait();
        if shutdown {
            runtime.request_stop();
        } else {
            runtime.cancel.store(true, Ordering::Release);
        }
        release.wait();
        let mut sink = Sink::default();
        let response = finish(&request, &mut client, &mut sink).unwrap();
        assert!(matches!(
            super::super::service_errors::decode(&response),
            GlobalIndexError::Paused
        ));
        assert_eq!(sink.writes, 0);
        handle.join().unwrap().unwrap();
    }
}
#[test]
fn p1_runtime_provider_permission_and_io_keep_actual_encoded_identity() {
    for fault in [
        "provider",
        "permission",
        "io",
        "windows_io_5",
        "windows_io_1314",
    ] {
        let runtime = runtime(Provider {
            gate: None,
            fault: Some(fault),
        });
        let request = IndexServiceRequest::new(
            IndexServiceCommand::ResumeIncrementalSync {
                source_id: "v".into(),
            },
            Some(source()),
        );
        let (mut client, handle) = start(runtime, request.clone());
        let response = finish(&request, &mut client, &mut Sink::default()).unwrap();
        let error = super::super::service_errors::decode(&response);
        match fault {
            "provider" => {
                assert!(matches!(error, GlobalIndexError::Provider(_)));
                assert_eq!(response.error_code.as_deref(), Some("index_provider_error"));
            }
            "permission" => {
                assert!(matches!(error, GlobalIndexError::PermissionRequired(_)));
                assert_eq!(
                    response.error_code.as_deref(),
                    Some("index_permission_required")
                );
            }
            "windows_io_5" => {
                assert!(matches!(error, GlobalIndexError::WindowsIo(5)));
                assert_eq!(response.error_code.as_deref(), Some("index_windows_io:5"));
                assert_eq!(response.status.as_deref(), Some("permission_required"));
            }
            "windows_io_1314" => {
                assert!(matches!(error, GlobalIndexError::WindowsIo(1314)));
                assert_eq!(
                    response.error_code.as_deref(),
                    Some("index_windows_io:1314")
                );
                assert_eq!(response.status.as_deref(), Some("permission_required"));
            }
            _ => {
                assert!(matches!(error, GlobalIndexError::WindowsIo(32)));
                assert_eq!(response.error_code.as_deref(), Some("index_windows_io:32"));
            }
        }
        handle.join().unwrap().unwrap();
    }
}

#[test]
fn p1_runtime_lookup_roundtrip_binds_response_and_legacy_stale_identity() {
    let runtime = runtime(Provider {
        gate: None,
        fault: Some("lookup"),
    });
    let request = IndexServiceRequest::new(
        IndexServiceCommand::ResumeIncrementalSync {
            source_id: "v".into(),
        },
        Some(source()),
    );
    let (mut client, handle) = start(runtime, request.clone());
    let mut sink = Sink::default();
    sink.set_lookup(owned_entry());
    assert!(finish(&request, &mut client, &mut sink).unwrap().ok);
    assert_eq!(sink.writes, 3);
    handle.join().unwrap().unwrap();
}
