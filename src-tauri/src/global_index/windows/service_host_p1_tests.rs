//! Actual ServiceRuntime -> PipeSink -> production exchange/admission path.
//! Controlled providers are test doubles, not native MFT or released binaries.
use super::super::service::p1_tests::{owned_entry, source, Sink};
use super::super::service::{
    exchange, IndexServiceAdmission, IndexServiceCallError, IndexServiceFrame, IndexServiceIo,
};
use super::*;
use std::sync::{mpsc, Barrier};

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
                _ => GlobalIndexError::Provider("same message".into()),
            });
        }
        if full {
            sink.mark_volume_entries_stale(&source.volume.id)?;
        }
        sink.checkpoint(&source.volume.id, Some("7"), Some("42"))
    }
}
struct Connection {
    id: String,
    frames: mpsc::Sender<IndexServiceFrame>,
    lookups: mpsc::Receiver<IndexServiceLookupResponse>,
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
}
impl IndexServiceIo for Client {
    fn send_request(&mut self, _: &IndexServiceRequest) -> Result<(), IndexServiceCallError> {
        Ok(())
    }
    fn read_frame(&mut self) -> Result<IndexServiceFrame, IndexServiceCallError> {
        self.frames
            .recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|e| IndexServiceCallError::UnknownOutcome(e.to_string()))
    }
    fn send_lookup(&mut self, response: IndexServiceLookupResponse) -> Result<(), String> {
        self.lookups.send(response).map_err(|e| e.to_string())
    }
}
fn start(
    runtime: Arc<ServiceRuntime>,
    request: IndexServiceRequest,
) -> (Client, std::thread::JoinHandle<Result<(), String>>) {
    let (tx, rx) = mpsc::channel();
    let (ltx, lrx) = mpsc::channel();
    let mut connection = Connection {
        id: request.request_id.clone(),
        frames: tx,
        lookups: lrx,
    };
    let handle = std::thread::spawn(move || runtime.handle_request(request, &mut connection));
    (
        Client {
            frames: rx,
            lookups: ltx,
        },
        handle,
    )
}
fn runtime(provider: Provider) -> Arc<ServiceRuntime> {
    let mut runtime = ServiceRuntime::new();
    runtime.provider = Box::new(provider);
    Arc::new(runtime)
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
    for fault in ["provider", "permission", "io"] {
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
        assert!(match fault {
            "provider" => matches!(error, GlobalIndexError::Provider(_)),
            "permission" => matches!(error, GlobalIndexError::PermissionRequired(_)),
            _ => matches!(error, GlobalIndexError::WindowsIo(32)),
        });
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
