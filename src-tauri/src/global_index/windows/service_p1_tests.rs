//! Deterministic tests of the production client exchange and admission validator.
use super::*;
use crate::global_index::coordinator::{GlobalIndexError, GlobalIndexSink};
use crate::global_index::models::*;
use std::collections::VecDeque;

pub(crate) fn source() -> GlobalVolume {
    serde_json::from_value(serde_json::json!({
        "id":"v", "platform":"windows", "stableVolumeId":"native-v", "displayName":"test", "mountPath":"Z:\\",
        "filesystemType":"NTFS", "driveKind":"fixed", "enabled":true, "scopeAuthorizationState":"authorized", "provider":PROVIDER_WINDOWS_MFT_USN,
        "indexStatus":INDEX_STATUS_READY, "lastError":null, "journalId":null, "journalCursor":null,
        "lastFullIndexAt":null, "lastIncrementalSyncAt":null, "entryCount":0, "createdAt":0, "updatedAt":0
    })).unwrap()
}
pub(crate) fn owned_entry() -> GlobalEntry {
    serde_json::from_value(serde_json::json!({
        "id":"owned-id", "volumeId":"v", "platformFileId":"f", "parentPlatformFileId":"p", "name":"name", "nameNormalized":"name",
        "path":"Z:\\name", "pathNormalized":"z:/name", "extension":"", "isDirectory":false, "size":0, "createdAtFs":null, "modifiedAtFs":null,
        "fileAttributes":0, "isHidden":false, "isSystem":false, "isStale":false, "sourceProvider":PROVIDER_WINDOWS_MFT_USN, "lastSeenAt":0
    })).unwrap()
}
fn request(full: bool) -> IndexServiceRequest {
    IndexServiceRequest::new(
        if full {
            IndexServiceCommand::Rebuild {
                source_id: "v".into(),
            }
        } else {
            IndexServiceCommand::ResumeIncrementalSync {
                source_id: "v".into(),
            }
        },
        Some(source()),
    )
}
pub(super) fn response(request: &IndexServiceRequest, code: Option<&str>) -> IndexServiceResponse {
    IndexServiceResponse {
        protocol_version: super::IPC_PROTOCOL_VERSION,
        request_id: request.request_id.clone(),
        ok: code.is_none(),
        error_code: code.map(str::to_string),
        message: None,
        status: None,
    }
}
fn marker() -> IndexServiceEvent {
    IndexServiceEvent::Sources {
        sources: vec![source()],
    }
}
fn provider() -> IndexServiceEvent {
    IndexServiceEvent::SourceProvider {
        volume_id: "v".into(),
        provider: PROVIDER_WINDOWS_MFT_USN.into(),
    }
}
fn stale() -> IndexServiceEvent {
    IndexServiceEvent::VolumeEntriesStale {
        volume_id: "v".into(),
    }
}
fn checkpoint() -> IndexServiceEvent {
    IndexServiceEvent::Checkpoint {
        volume_id: "v".into(),
        journal_id: Some("7".into()),
        journal_cursor: Some("42".into()),
    }
}
fn event_frame(request: &IndexServiceRequest, event: IndexServiceEvent) -> IndexServiceFrame {
    IndexServiceFrame::Event {
        request_id: request.request_id.clone(),
        event,
    }
}
#[derive(Default)]
pub(crate) struct Sink {
    pub(crate) writes: usize,
    pub(crate) provider_writes: usize,
    pub(crate) volume_stale_writes: usize,
    pub(crate) batch_writes: usize,
    fail: Option<GlobalIndexError>,
    lookup: Option<GlobalEntry>,
}
impl Sink {
    pub(crate) fn set_lookup(&mut self, entry: GlobalEntry) {
        self.lookup = Some(entry);
    }
    fn write(&mut self) -> Result<(), GlobalIndexError> {
        if let Some(error) = self.fail.take() {
            return Err(error);
        }
        self.writes += 1;
        Ok(())
    }
}
impl GlobalIndexSink for Sink {
    fn write_batch(&mut self, entries: &[GlobalEntryInput]) -> Result<usize, GlobalIndexError> {
        self.write()?;
        self.batch_writes += 1;
        Ok(entries.len())
    }
    fn mark_entry_stale(&mut self, _: &str) -> Result<(), GlobalIndexError> {
        self.write()
    }
    fn checkpoint(
        &mut self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<(), GlobalIndexError> {
        self.write()
    }
    fn set_source_state(
        &mut self,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> Result<(), GlobalIndexError> {
        self.write()
    }
    fn set_source_provider(&mut self, _: &str, _: &str) -> Result<(), GlobalIndexError> {
        self.write()?;
        self.provider_writes += 1;
        Ok(())
    }
    fn resolve_parent_path(
        &mut self,
        _: &str,
        _: &str,
    ) -> Result<Option<String>, GlobalIndexError> {
        Ok(None)
    }
    fn find_entry_by_identity(
        &mut self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Option<GlobalEntry>, GlobalIndexError> {
        Ok(self.lookup.clone())
    }
    fn mark_volume_entries_stale(&mut self, _: &str) -> Result<(), GlobalIndexError> {
        self.write()?;
        self.volume_stale_writes += 1;
        Ok(())
    }
}
struct Io {
    frames: VecDeque<IndexServiceFrame>,
    sent: bool,
    replies: usize,
    send_failure: bool,
}
impl IndexServiceIo for Io {
    fn send_request(&mut self, _: &IndexServiceRequest) -> Result<(), IndexServiceCallError> {
        self.sent = true;
        if self.send_failure {
            Err(IndexServiceCallError::UnknownOutcome(
                "partial write".into(),
            ))
        } else {
            Ok(())
        }
    }
    fn read_frame(&mut self) -> Result<IndexServiceFrame, IndexServiceCallError> {
        self.frames
            .pop_front()
            .ok_or_else(|| IndexServiceCallError::UnknownOutcome("disconnect".into()))
    }
    fn send_lookup(&mut self, _: IndexServiceLookupResponse) -> Result<(), String> {
        self.replies += 1;
        Ok(())
    }
}
fn run(
    request: &IndexServiceRequest,
    events: Vec<IndexServiceEvent>,
    terminal: Option<IndexServiceResponse>,
    sink: &mut Sink,
) -> Result<IndexServiceResponse, IndexServiceCallError> {
    let mut frames: VecDeque<_> = events
        .into_iter()
        .map(|event| event_frame(request, event))
        .collect();
    if let Some(response) = terminal {
        frames.push_back(IndexServiceFrame::Response { response });
    }
    let mut io = Io {
        frames,
        sent: false,
        replies: 0,
        send_failure: false,
    };
    let mut admission = IndexServiceAdmission::new(request);
    let response = exchange(request, &mut io, |event| admission.apply(sink, event))?;
    admission.finish(&response)?;
    Ok(response)
}
#[test]
fn p1_t01_busy_before_admission_has_zero_sink_writes() {
    let request = request(true);
    let mut sink = Sink::default();
    let response = run(
        &request,
        vec![],
        Some(response(&request, Some("index_service_busy"))),
        &mut sink,
    )
    .unwrap();
    assert!(matches!(
        super::super::service_errors::decode(&response),
        GlobalIndexError::ServiceBusy(_)
    ));
    assert_eq!(sink.writes, 0);
}
#[test]
fn p1_t02_wrong_response_or_event_identity_and_version_are_rejected_before_callback() {
    let request = request(true);
    for frame in [
        IndexServiceFrame::Response {
            response: IndexServiceResponse {
                protocol_version: super::IPC_PROTOCOL_VERSION + 1,
                ..response(&request, None)
            },
        },
        IndexServiceFrame::Response {
            response: IndexServiceResponse {
                request_id: "old".into(),
                ..response(&request, None)
            },
        },
        IndexServiceFrame::Event {
            request_id: "old".into(),
            event: marker(),
        },
    ] {
        let mut io = Io {
            frames: VecDeque::from([frame]),
            sent: false,
            replies: 0,
            send_failure: false,
        };
        let mut callbacks = 0;
        assert!(matches!(
            exchange(&request, &mut io, |_| {
                callbacks += 1;
                Ok(None)
            }),
            Err(IndexServiceCallError::Protocol(_))
        ));
        assert_eq!(callbacks, 0);
    }
}
#[test]
fn p1_t03_foreign_events_and_unproven_legacy_stale_never_write() {
    let request = request(false);
    for event in [
        IndexServiceEvent::SourceState {
            volume_id: "foreign".into(),
            status: INDEX_STATUS_ERROR.into(),
            error: None,
        },
        IndexServiceEvent::VolumeEntriesStale {
            volume_id: "foreign".into(),
        },
        IndexServiceEvent::EntryStale {
            entry_id: "foreign-id".into(),
        },
    ] {
        let mut sink = Sink::default();
        let mut admission = IndexServiceAdmission::new(&request);
        admission.apply(&mut sink, marker()).unwrap();
        admission.apply(&mut sink, provider()).unwrap();
        let before = sink.writes;
        assert!(matches!(
            admission.apply(&mut sink, event),
            Err(IndexServiceCallError::Protocol(_))
        ));
        assert_eq!(before, sink.writes);
    }
}
#[test]
fn p1_t04_duplicate_missing_and_late_admission_are_rejected() {
    let request = request(true);
    for events in [
        vec![provider()],
        vec![marker(), marker()],
        vec![checkpoint()],
        vec![marker(), checkpoint()],
    ] {
        assert!(matches!(
            run(
                &request,
                events,
                Some(response(&request, None)),
                &mut Sink::default()
            ),
            Err(IndexServiceCallError::Protocol(_))
        ));
    }
}
#[test]
fn p1_t05_real_fault_states_and_error_codes_keep_distinct_identity() {
    let request = request(false);
    for (status, code) in [
        (
            INDEX_STATUS_PERMISSION_REQUIRED,
            "index_permission_required",
        ),
        (INDEX_STATUS_REBUILD_REQUIRED, "index_rebuild_required"),
        (INDEX_STATUS_ERROR, "index_provider_error"),
    ] {
        let mut sink = Sink::default();
        let response = run(
            &request,
            vec![
                marker(),
                provider(),
                IndexServiceEvent::SourceState {
                    volume_id: "v".into(),
                    status: status.into(),
                    error: Some("same message".into()),
                },
            ],
            Some(response(&request, Some(code))),
            &mut sink,
        )
        .unwrap();
        assert_eq!(
            super::super::service_errors::classify(&super::super::service_errors::decode(
                &response
            ))
            .1,
            status
        );
    }
}
#[test]
fn p1_t06_initial_and_rebuild_require_real_full_completion_boundaries() {
    for command in [
        IndexServiceCommand::StartInitialIndex {
            source_id: "v".into(),
        },
        IndexServiceCommand::Rebuild {
            source_id: "v".into(),
        },
    ] {
        let request = IndexServiceRequest::new(command, Some(source()));
        run(
            &request,
            vec![marker(), provider(), stale(), checkpoint()],
            Some(response(&request, None)),
            &mut Sink::default(),
        )
        .unwrap();
        assert!(matches!(
            run(
                &request,
                vec![marker(), provider(), checkpoint()],
                Some(response(&request, None)),
                &mut Sink::default()
            ),
            Err(IndexServiceCallError::Protocol(_))
        ));
    }
}
#[test]
fn p1_t07_empty_scan_needs_completion_not_entries_or_terminal_ok() {
    for full in [true, false] {
        let request = request(full);
        let mut events = vec![marker(), provider()];
        if full {
            events.push(stale());
        }
        events.push(checkpoint());
        run(
            &request,
            events,
            Some(response(&request, None)),
            &mut Sink::default(),
        )
        .unwrap();
        assert!(matches!(
            run(
                &request,
                vec![],
                Some(response(&request, None)),
                &mut Sink::default()
            ),
            Err(IndexServiceCallError::Protocol(_))
        ));
    }
}
#[test]
fn p1_t08_partial_stream_disconnect_is_transport_unknown_not_provider() {
    let request = request(true);
    let mut sink = Sink::default();
    let error = run(&request, vec![marker(), provider()], None, &mut sink).unwrap_err();
    assert!(matches!(
        error.into_global(),
        GlobalIndexError::ServiceOutcomeUnknown(_)
    ));
    assert_eq!(sink.writes, 1);
}
#[test]
fn p1_t09_actual_sqlite_busy_survives_client_sink_boundary() {
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let path = std::env::temp_dir().join(format!("zen380-p1-client-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&path).unwrap();
    let directory = Directory(path);
    let writer = rusqlite::Connection::open(directory.0.join("test.sqlite")).unwrap();
    writer
        .execute_batch(
            "CREATE TABLE sample(id INTEGER); INSERT INTO sample VALUES(1); BEGIN IMMEDIATE;",
        )
        .unwrap();
    let contender = rusqlite::Connection::open(directory.0.join("test.sqlite")).unwrap();
    contender.busy_timeout(std::time::Duration::ZERO).unwrap();
    let error = contender.execute("UPDATE sample SET id=2", []).unwrap_err();
    assert!(matches!(&error,rusqlite::Error::SqliteFailure(code,_) if code.extended_code==5));
    let request = request(false);
    let mut sink = Sink {
        fail: Some(GlobalIndexError::Database(error.into())),
        ..Default::default()
    };
    let error = run(&request, vec![marker(), provider()], None, &mut sink)
        .unwrap_err()
        .into_global();
    assert!(
        matches!(error,GlobalIndexError::Database(crate::db::DbError::Sqlite(rusqlite::Error::SqliteFailure(code,_))) if code.extended_code==5)
    );
    writer.execute_batch("ROLLBACK").unwrap();
}
#[test]
fn p1_t10_duplicate_cross_request_lookup_and_unsolicited_reply_are_rejected() {
    let request = request(false);
    let lookup = IndexServiceEvent::ResolveParentPath {
        lookup_id: "lookup".into(),
        volume_id: "v".into(),
        parent_platform_file_id: "p".into(),
    };
    assert!(matches!(
        run(
            &request,
            vec![marker(), provider(), lookup.clone(), lookup],
            None,
            &mut Sink::default()
        ),
        Err(IndexServiceCallError::Protocol(_))
    ));
    let mut io = Io {
        frames: VecDeque::from([IndexServiceFrame::LookupResponse {
            response: IndexServiceLookupResponse::ParentPath {
                lookup_id: "old".into(),
                path: None,
            },
        }]),
        sent: false,
        replies: 0,
        send_failure: false,
    };
    assert!(matches!(
        exchange(&request, &mut io, |_| Ok(None)),
        Err(IndexServiceCallError::Protocol(_))
    ));
    assert_eq!(io.replies, 0);
}
#[test]
fn p1_t11_unknown_remote_code_is_protocol_not_transport_unknown() {
    let request = request(false);
    assert!(matches!(
        run(
            &request,
            vec![],
            Some(response(&request, Some("index_outcome_unknown"))),
            &mut Sink::default()
        ),
        Err(IndexServiceCallError::Protocol(_))
    ));
}
#[test]
fn p1_t12_attempted_partial_send_is_never_not_sent() {
    let request = request(false);
    let mut io = Io {
        frames: VecDeque::new(),
        sent: false,
        replies: 0,
        send_failure: true,
    };
    assert!(matches!(
        exchange(&request, &mut io, |_| Ok(None)),
        Err(IndexServiceCallError::UnknownOutcome(_))
    ));
    assert!(io.sent);
}
#[test]
fn p1_t13_no_unadmitted_state_or_lookup_side_effects() {
    let request = request(false);
    for event in [
        IndexServiceEvent::SourceState {
            volume_id: "v".into(),
            status: INDEX_STATUS_ERROR.into(),
            error: None,
        },
        IndexServiceEvent::ResolveParentPath {
            lookup_id: "l".into(),
            volume_id: "v".into(),
            parent_platform_file_id: "p".into(),
        },
    ] {
        let mut sink = Sink::default();
        assert!(run(&request, vec![event], None, &mut sink).is_err());
        assert_eq!(sink.writes, 0);
    }
}
#[test]
fn p1_lookup_result_ownership_and_nonempty_entries_are_checked_before_effects() {
    let request = request(false);
    let lookup = IndexServiceEvent::FindEntryByIdentity {
        lookup_id: "l".into(),
        volume_id: "v".into(),
        platform_file_id: "f".into(),
        parent_platform_file_id: "p".into(),
        name: "name".into(),
    };
    let mut sink = Sink {
        lookup: Some(owned_entry()),
        ..Default::default()
    };
    run(
        &request,
        vec![
            marker(),
            provider(),
            lookup.clone(),
            IndexServiceEvent::EntryStale {
                entry_id: "owned-id".into(),
            },
            checkpoint(),
        ],
        Some(response(&request, None)),
        &mut sink,
    )
    .unwrap();
    assert_eq!(sink.writes, 3);
    let mut foreign = owned_entry();
    foreign.volume_id = "foreign".into();
    let mut sink = Sink {
        lookup: Some(foreign),
        ..Default::default()
    };
    assert!(matches!(
        run(
            &request,
            vec![marker(), provider(), lookup.clone()],
            None,
            &mut sink
        ),
        Err(IndexServiceCallError::Protocol(_))
    ));
    assert_eq!(sink.writes, 1);
    let mut input = serde_json::to_value(owned_entry()).unwrap();
    input["volumeId"] = "foreign".into();
    let entry: GlobalEntryInput = serde_json::from_value(input).unwrap();
    let mut sink = Sink::default();
    assert!(matches!(
        run(
            &request,
            vec![
                marker(),
                provider(),
                IndexServiceEvent::Entries {
                    entries: vec![entry]
                }
            ],
            None,
            &mut sink
        ),
        Err(IndexServiceCallError::Protocol(_))
    ));
    assert_eq!(sink.writes, 1);
    let mut io = Io {
        frames: VecDeque::from([IndexServiceFrame::Event {
            request_id: "previous-request".into(),
            event: lookup,
        }]),
        sent: false,
        replies: 0,
        send_failure: false,
    };
    assert!(matches!(
        exchange(&request, &mut io, |_| panic!(
            "replayed lookup must never reach callback"
        )),
        Err(IndexServiceCallError::Protocol(_))
    ));
    assert_eq!(io.replies, 0);
}

#[test]
fn p1_t14_v3_shape_and_checkpoint_type_remain_unchanged() {
    let json = serde_json::to_value(IndexServiceEvent::EntryStale {
        entry_id: "id".into(),
    })
    .unwrap();
    assert_eq!(json.as_object().unwrap().len(), 2);
    assert!(json.get("volume_id").is_none());
    assert!(serde_json::from_value::<IndexServiceFrame>(serde_json::json!({"kind":"response","response":{"protocol_version":3,"request_id":"r","ok":true,"error_code":null,"message":null,"status":null,"extra":true}})).is_err());
    let request = request(false);
    run(
        &request,
        vec![marker(), provider(), checkpoint()],
        Some(response(&request, None)),
        &mut Sink::default(),
    )
    .unwrap();
}

#[test]
fn p1_malformed_checkpoint_text_is_not_scan_completion() {
    let request = request(false);
    for (journal, cursor) in [("junk", "42"), ("7", "junk"), ("7", "-1"), ("07", "42")] {
        let mut sink = Sink::default();
        let event = IndexServiceEvent::Checkpoint {
            volume_id: "v".into(),
            journal_id: Some(journal.into()),
            journal_cursor: Some(cursor.into()),
        };
        assert!(matches!(
            run(
                &request,
                vec![marker(), provider(), event],
                Some(response(&request, None)),
                &mut sink
            ),
            Err(IndexServiceCallError::Protocol(_))
        ));
        assert_eq!(sink.writes, 1);
    }
}

#[test]
fn p1_resume_checkpoint_never_changes_journal_or_rewinds_snapshot() {
    for (journal, cursor) in [("8", "43"), ("7", "41")] {
        let mut volume = source();
        volume.journal_id = Some("7".into());
        volume.journal_cursor = Some("42".into());
        let request = IndexServiceRequest::new(
            IndexServiceCommand::ResumeIncrementalSync {
                source_id: "v".into(),
            },
            Some(volume.clone()),
        );
        let mut sink = Sink::default();
        assert!(matches!(
            run(
                &request,
                vec![
                    IndexServiceEvent::Sources {
                        sources: vec![volume]
                    },
                    provider(),
                    IndexServiceEvent::Checkpoint {
                        volume_id: "v".into(),
                        journal_id: Some(journal.into()),
                        journal_cursor: Some(cursor.into())
                    }
                ],
                Some(response(&request, None)),
                &mut sink
            ),
            Err(IndexServiceCallError::Protocol(_))
        ));
        assert_eq!(sink.writes, 1);
    }
}
