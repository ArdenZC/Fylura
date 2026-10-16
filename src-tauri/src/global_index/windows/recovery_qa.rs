//! Native-qa-only installed service regression on the qualification VHD/profile.
use super::WindowsGlobalIndexProvider;
use crate::db::Database;
use crate::global_index::coordinator::{
    run_service_recovery_qa_cycle, GlobalIndexError, GlobalIndexProvider, GlobalIndexSink,
};
use crate::global_index::models::*;
use crate::global_index::windows::PauseObservation;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};

struct ProbeProvider {
    inner: WindowsGlobalIndexProvider,
    database: Database,
    source_id: String,
    pause_once: AtomicBool,
    outcome: Mutex<&'static str>,
    rebuilds: AtomicUsize,
    events: Mutex<Vec<serde_json::Value>>,
}
impl ProbeProvider {
    fn record_event(&self, mut event: serde_json::Value) {
        if let Ok(mut events) = self.events.lock() {
            if let Some(object) = event.as_object_mut() {
                object.insert("sequence".into(), serde_json::json!(events.len() + 1));
            }
            events.push(event);
        }
    }

    fn trigger_pause_callback(&self, source_id: &str) -> Result<(), GlobalIndexError> {
        if !self.pause_once.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        self.record_event(serde_json::json!({
            "event": "pause_callback_started",
            "source_id": source_id,
            "trigger": "service_sources_event_before_admission_commit",
        }));
        let result = self.inner.pause_for_native_qa(|observation| {
            let event = match observation {
                PauseObservation::Request {
                    protocol_version,
                    request_id,
                } => serde_json::json!({
                    "event": "typed_pause_request",
                    "command": "pause",
                    "protocol_version": protocol_version,
                    "request_id": request_id,
                }),
                PauseObservation::Response(response) => serde_json::json!({
                    "event": "typed_pause_response",
                    "protocol_version": response.protocol_version,
                    "request_id": response.request_id,
                    "ok": response.ok,
                    "status": response.status,
                    "error_code": response.error_code,
                    "message_omitted": response.message.is_some(),
                }),
                PauseObservation::TransportError { kind } => serde_json::json!({
                    "event": "typed_pause_transport_error",
                    "kind": kind,
                }),
            };
            self.record_event(event);
        });
        self.record_event(serde_json::json!({
            "event": "pause_callback_result",
            "succeeded": result.is_ok(),
        }));
        result
    }

    fn operation_state_snapshot(&self) -> serde_json::Value {
        match self.database.global_index_operation_state(&self.source_id) {
            Ok(state) => operation_state_value(state.as_ref()),
            Err(_) => serde_json::json!({"read_error": "database_error"}),
        }
    }

    fn record_operation_state(&self, event: &'static str) {
        self.record_event(serde_json::json!({
            "event": event,
            "source_operation": self.operation_state_snapshot(),
        }));
    }

    fn events_snapshot(&self) -> Vec<serde_json::Value> {
        self.events
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default()
    }

    fn outcome(&self) -> &'static str {
        self.outcome
            .lock()
            .map(|outcome| *outcome)
            .unwrap_or("unavailable")
    }

    fn record(&self, result: Result<(), GlobalIndexError>) -> Result<(), GlobalIndexError> {
        let outcome = match &result {
            Ok(()) => "ready",
            Err(GlobalIndexError::Paused) => "paused",
            Err(GlobalIndexError::RebuildRequired(_)) => "rebuild_required",
            _ => "failure",
        };
        *self.outcome.lock().unwrap() = outcome;
        let terminal_error_code = match &result {
            Ok(()) => None,
            Err(GlobalIndexError::Paused) => Some("index_paused"),
            Err(GlobalIndexError::RebuildRequired(_)) => Some("rebuild_required"),
            Err(GlobalIndexError::PermissionRequired(_)) => Some("permission_required"),
            Err(GlobalIndexError::ServiceBusy(_)) => Some("index_service_busy"),
            Err(GlobalIndexError::SourceBusy(_)) => Some("source_operation_busy"),
            Err(GlobalIndexError::ServiceProtocol(_)) => Some("invalid_service_protocol"),
            Err(GlobalIndexError::ServiceOutcomeUnknown(_)) => Some("service_outcome_unknown"),
            Err(GlobalIndexError::WindowsIo(_)) => Some("windows_io_error"),
            Err(GlobalIndexError::Database(_)) => Some("database_error"),
            Err(GlobalIndexError::Provider(_)) => Some("provider_error"),
        };
        self.record_event(serde_json::json!({
            "event": "service_index_stream_terminal",
            "ok": result.is_ok(),
            "error_code": terminal_error_code,
            "outcome": outcome,
            "service_available": self.inner.service_available.load(Ordering::Acquire),
            "observation": "decoded by the installed service client after consuming the stream",
        }));
        self.record_event(serde_json::json!({
            "event": "provider_operation_result",
            "outcome": outcome,
            "service_available": self.inner.service_available.load(Ordering::Acquire),
        }));
        if !self.inner.service_available.load(Ordering::Acquire) {
            return Err(GlobalIndexError::Provider(
                "QA refuses direct-provider fallback".into(),
            ));
        }
        result
    }
}

fn operation_state_value(
    state: Option<&crate::global_index::repository::SourceOperationState>,
) -> serde_json::Value {
    match state {
        Some(state) => serde_json::json!({
            "revision": state.revision,
            "owner_id": state.owner_id,
            "operation_state": state.operation_state,
            "operation_route": state.operation_route,
            "operation_kind": state.operation_kind,
            "rebuild_intent_revision": state.rebuild_intent_revision,
            "rebuild_completed_revision": state.rebuild_completed_revision,
        }),
        None => serde_json::json!({"present": false}),
    }
}

fn first_event(events: &[serde_json::Value], name: &str) -> serde_json::Value {
    events
        .iter()
        .find(|event| event.get("event").and_then(|event| event.as_str()) == Some(name))
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

fn write_report(
    report: &std::path::Path,
    value: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::write(report, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
impl GlobalIndexProvider for ProbeProvider {
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        let sources = self.inner.discover_sources()?;
        if !self.inner.service_available.load(Ordering::Acquire) {
            return Err(GlobalIndexError::Provider(
                "installed pipe unavailable".into(),
            ));
        }
        Ok(sources
            .into_iter()
            .filter(|source| source.volume.id == self.source_id)
            .collect())
    }
    fn start_initial_index(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        let mut sink = PauseSink {
            sink,
            provider: self,
        };
        self.record(self.inner.start_initial_index(source, &mut sink, cancel))
    }
    fn resume_incremental_sync(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.record(self.inner.resume_incremental_sync(source, sink, cancel))
    }
    fn rebuild(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.rebuilds.fetch_add(1, Ordering::AcqRel);
        self.record(self.inner.rebuild(source, sink, cancel))
    }
    fn pause(&self) -> Result<(), GlobalIndexError> {
        self.inner.pause()
    }
    fn status(&self) -> Result<String, GlobalIndexError> {
        self.inner.status()
    }
    fn shutdown(&self) -> Result<(), GlobalIndexError> {
        self.inner.shutdown()
    }
    fn has_external_source_operation_admission(&self) -> bool {
        self.inner.has_external_source_operation_admission()
    }
}
struct PauseSink<'a> {
    sink: &'a mut dyn GlobalIndexSink,
    provider: &'a ProbeProvider,
}
impl GlobalIndexSink for PauseSink<'_> {
    fn admit_source_operation(
        &mut self,
        source: &GlobalVolume,
        operation_id: &str,
        kind: crate::global_index::SourceOperationKind,
        route: crate::global_index::SourceOperationRoute,
    ) -> Result<(), GlobalIndexError> {
        self.provider.record_event(serde_json::json!({
            "event": "source_operation_admission_started",
            "source_id": source.id,
            "operation_id": operation_id,
            "kind": format!("{kind:?}"),
            "route": format!("{route:?}"),
        }));
        // The Service has accepted this index request and emitted Sources, but
        // has not yet begun provider work. Issue typed Pause at this boundary;
        // doing so later from set_source_provider blocks the stream consumer
        // while the independent Pause pipe is being read.
        self.provider.trigger_pause_callback(&source.id)?;
        let result = self
            .sink
            .admit_source_operation(source, operation_id, kind, route);
        self.provider.record_event(serde_json::json!({
            "event": "source_operation_admission_result",
            "source_id": source.id,
            "committed": result.is_ok(),
        }));
        result?;
        self.provider
            .record_operation_state("source_operation_admission_committed");
        Ok(())
    }
    fn source_operation_admitted(&self, volume_id: &str) -> bool {
        self.sink.source_operation_admitted(volume_id)
    }
    fn mark_source_operation_unknown(&mut self, volume_id: &str) -> Result<(), GlobalIndexError> {
        self.provider.record_event(serde_json::json!({
            "event": "source_operation_unknown_started",
            "source_id": volume_id,
        }));
        let result = self.sink.mark_source_operation_unknown(volume_id);
        self.provider.record_event(serde_json::json!({
            "event": "source_operation_unknown_result",
            "source_id": volume_id,
            "committed": result.is_ok(),
        }));
        result?;
        self.provider
            .record_operation_state("source_operation_unknown_committed");
        Ok(())
    }
    fn finish_source_operation(
        &mut self,
        volume_id: &str,
        status: &str,
        error: Option<&str>,
        full_index_at: Option<i64>,
        incremental_sync_at: Option<i64>,
        complete_rebuild_intent: bool,
    ) -> Result<(), GlobalIndexError> {
        self.provider.record_event(serde_json::json!({
            "event": "source_operation_finish_started",
            "source_id": volume_id,
            "status": status,
            "last_error": error,
            "complete_rebuild_intent": complete_rebuild_intent,
        }));
        let result = self.sink.finish_source_operation(
            volume_id,
            status,
            error,
            full_index_at,
            incremental_sync_at,
            complete_rebuild_intent,
        );
        self.provider.record_event(serde_json::json!({
            "event": "source_operation_finish_result",
            "source_id": volume_id,
            "committed": result.is_ok(),
        }));
        result?;
        self.provider
            .record_operation_state("source_operation_finish_committed");
        Ok(())
    }
    fn set_source_provider(&mut self, id: &str, provider: &str) -> Result<(), GlobalIndexError> {
        self.provider.record_event(serde_json::json!({
            "event": "source_provider_write_started",
            "source_id": id,
            "provider": provider,
        }));
        let result = self.sink.set_source_provider(id, provider);
        self.provider.record_event(serde_json::json!({
            "event": "source_provider_write_result",
            "source_id": id,
            "committed": result.is_ok(),
        }));
        result?;
        Ok(())
    }
    fn write_batch(&mut self, entries: &[GlobalEntryInput]) -> Result<usize, GlobalIndexError> {
        self.sink.write_batch(entries)
    }
    fn mark_entry_stale(&mut self, entry_id: &str) -> Result<(), GlobalIndexError> {
        self.sink.mark_entry_stale(entry_id)
    }
    fn checkpoint(
        &mut self,
        volume_id: &str,
        journal_id: Option<&str>,
        journal_cursor: Option<&str>,
    ) -> Result<(), GlobalIndexError> {
        self.provider.record_event(serde_json::json!({
            "event": "checkpoint_write_started",
            "source_id": volume_id,
            "journal_id": journal_id,
            "journal_cursor": journal_cursor,
        }));
        let result = self.sink.checkpoint(volume_id, journal_id, journal_cursor);
        self.provider.record_event(serde_json::json!({
            "event": "checkpoint_write_result",
            "source_id": volume_id,
            "committed": result.is_ok(),
        }));
        result
    }
    fn set_source_state(
        &mut self,
        volume_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), GlobalIndexError> {
        self.provider.record_event(serde_json::json!({
            "event": "source_state_write_started",
            "source_id": volume_id,
            "status": status,
            "last_error": error,
        }));
        let result = self.sink.set_source_state(volume_id, status, error);
        self.provider.record_event(serde_json::json!({
            "event": "source_state_write_result",
            "source_id": volume_id,
            "committed": result.is_ok(),
        }));
        result?;
        self.provider
            .record_operation_state("source_state_write_committed");
        Ok(())
    }
    fn resolve_parent_path(
        &mut self,
        volume_id: &str,
        parent_platform_file_id: &str,
    ) -> Result<Option<String>, GlobalIndexError> {
        self.sink
            .resolve_parent_path(volume_id, parent_platform_file_id)
    }
    fn find_entry_by_identity(
        &mut self,
        volume_id: &str,
        platform_file_id: &str,
        parent_platform_file_id: &str,
        name: &str,
    ) -> Result<Option<GlobalEntry>, GlobalIndexError> {
        self.sink
            .find_entry_by_identity(volume_id, platform_file_id, parent_platform_file_id, name)
    }
    fn mark_volume_entries_stale(&mut self, volume_id: &str) -> Result<(), GlobalIndexError> {
        self.sink.mark_volume_entries_stale(volume_id)
    }
}

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Err(
            "usage: --global-index-recovery-qa <isolated-profile> <source-id> <report-path>".into(),
        );
    }
    let root = PathBuf::from(&args[0]);
    let report = PathBuf::from(&args[2]);
    if !root.is_absolute() || !report.is_absolute() {
        return Err("QA paths must be absolute".into());
    }
    let db = Database::open(root.join("zen-canvas.sqlite3"))?;
    let source = db
        .get_global_volume(&args[1])?
        .ok_or("isolated source absent")?;
    if !source.enabled
        || source.provider != PROVIDER_WINDOWS_MFT_USN
        || source.last_full_index_at.is_some()
        || db
            .list_global_volumes()?
            .iter()
            .filter(|v| v.enabled)
            .count()
            != 1
    {
        return Err("fresh isolated single-source profile required".into());
    }
    let provider = ProbeProvider {
        inner: WindowsGlobalIndexProvider::with_wake(
            std::sync::Arc::new(crate::global_index::wake::GlobalIndexWakeSlot::default()),
            root.join("zen-canvas.sqlite3"),
        ),
        database: db.clone(),
        source_id: source.id.clone(),
        pause_once: AtomicBool::new(true),
        outcome: Mutex::new("none"),
        rebuilds: AtomicUsize::new(0),
        events: Mutex::new(Vec::new()),
    };
    let owner_before_cycle = db.global_index_operation_state(&source.id)?;
    provider.record_event(serde_json::json!({
        "event": "pause_recovery_cycle_started",
        "durable_index_status": source.index_status,
        "durable_last_error": source.last_error,
        "source_operation": operation_state_value(owner_before_cycle.as_ref()),
    }));
    let first_cycle_result = run_service_recovery_qa_cycle(&provider, &db);
    provider.record_event(serde_json::json!({
        "event": "pause_recovery_cycle_returned",
        "succeeded": first_cycle_result.is_ok(),
    }));
    let paused_result = db.get_global_volume(&source.id);
    let owner_after_cycle_result = db.global_index_operation_state(&source.id);
    let actual_source = paused_result
        .as_ref()
        .ok()
        .and_then(|source| source.as_ref());
    let actual_index_status = actual_source.map(|source| source.index_status.as_str());
    let actual_last_error = actual_source.and_then(|source| source.last_error.as_deref());
    let source_state_read_error = paused_result.as_ref().err().map(|_| "database_error");
    let actual_outcome = provider.outcome();
    let owner_after_cycle = match owner_after_cycle_result.as_ref() {
        Ok(owner) => operation_state_value(owner.as_ref()),
        Err(_) => serde_json::json!({"read_error": "database_error"}),
    };
    provider.record_event(serde_json::json!({
        "event": "durable_source_state_observed_after_cycle",
        "source_present": actual_source.is_some(),
        "source_state_read_error": source_state_read_error,
        "index_status": actual_index_status,
        "last_error": actual_last_error,
        "source_operation": owner_after_cycle.clone(),
    }));
    let events = provider.events_snapshot();
    let pause_request = first_event(&events, "typed_pause_request");
    let pause_response = first_event(&events, "typed_pause_response");
    let pause_diagnostic = serde_json::json!({
        "index_status": actual_index_status,
        "last_error": actual_last_error,
        "provider_outcome": actual_outcome,
        "cycle_succeeded": first_cycle_result.is_ok(),
        "service_available": provider.inner.service_available.load(Ordering::Acquire),
        "source_state_read_error": source_state_read_error,
        "source_present": actual_source.is_some(),
        "source_operation_before_cycle": operation_state_value(owner_before_cycle.as_ref()),
        "source_operation_after_cycle": owner_after_cycle,
        "pause_request": pause_request,
        "pause_response": pause_response,
        "service_stream_terminal": first_event(&events, "service_index_stream_terminal"),
        "event_sequence_scope": "client callback observation order; it does not establish send order across independent named pipes",
        "source_state_commit_order": events,
        "strict_assertions": {
            "index_status_is_paused": actual_index_status == Some(INDEX_STATUS_PAUSED),
            "last_error_is_null": actual_last_error.is_none(),
            "provider_outcome_is_paused": actual_outcome == "paused",
        },
    });
    let partial_report = serde_json::json!({
        "protocol": 3,
        "directFallbackAllowed": false,
        "pausedViaPipe": pause_response.get("ok").and_then(|value| value.as_bool()).unwrap_or(false),
        "pauseDiagnostic": pause_diagnostic,
    });
    write_report(&report, &partial_report)?;
    if let Err(error) = first_cycle_result {
        return Err(
            format!("Pause recovery cycle failed after diagnostic capture: {error}").into(),
        );
    }
    let paused = paused_result?.ok_or("source absent")?;
    let owner_after_cycle = owner_after_cycle_result?;
    let provider_outcome = provider.outcome();
    if paused.index_status != INDEX_STATUS_PAUSED
        || paused.last_error.is_some()
        || provider_outcome != "paused"
    {
        return Err(format!(
            "typed pipe Pause / durable paused assertion failed: index_status={}, last_error={:?}, provider_outcome={}, owner_revision={:?}, owner_id={:?}, operation_state={:?}",
            paused.index_status,
            paused.last_error,
            provider_outcome,
            owner_after_cycle.as_ref().map(|state| state.revision),
            owner_after_cycle
                .as_ref()
                .and_then(|state| state.owner_id.as_deref()),
            owner_after_cycle
                .as_ref()
                .and_then(|state| state.operation_state.as_deref()),
        )
        .into());
    }
    // Controlled metadata fixture only: no journal/volume manipulation.
    db.update_global_volume_state(
        &source.id,
        INDEX_STATUS_READY,
        None,
        Some("18446744073709551615"),
        Some("9223372036854775807"),
        Some(1),
        None,
    )?;
    run_service_recovery_qa_cycle(&provider, &db)?;
    let required = db.get_global_volume(&source.id)?.ok_or("source absent")?;
    let required_operation = db.global_index_operation_state(&source.id)?;
    if required.index_status != INDEX_STATUS_REBUILD_REQUIRED
        || *provider.outcome.lock().unwrap() != "rebuild_required"
        || provider.rebuilds.load(Ordering::Acquire) != 0
    {
        return Err(format!(
            "typed rebuild-required / first-cycle durable assertion failed: status={}, last_error={:?}, outcome={}, rebuilds={}, operation={:?}",
            required.index_status,
            required.last_error,
            *provider.outcome.lock().unwrap(),
            provider.rebuilds.load(Ordering::Acquire),
            required_operation,
        )
        .into());
    }
    run_service_recovery_qa_cycle(&provider, &db)?;
    let rebuilt = db.get_global_volume(&source.id)?.ok_or("source absent")?;
    if rebuilt.index_status != INDEX_STATUS_READY
        || rebuilt.last_full_index_at.unwrap_or(0) <= 1
        || rebuilt.entry_count == 0
        || provider.rebuilds.load(Ordering::Acquire) != 1
    {
        return Err("next admitted service MFT rebuild did not reach ready".into());
    }
    std::fs::write(
        report,
        serde_json::to_vec_pretty(
            &serde_json::json!({"protocol":3,"directFallbackAllowed":false,"pausedViaPipe":true,"rebuildRequiredViaPipe":true,"firstCycleStatus":required.index_status,"nextAdmittedRebuildCount":1,"finalStatus":rebuilt.index_status,"lastFullIndexAt":rebuilt.last_full_index_at,"seededJournalId":"18446744073709551615","seededCursor":"9223372036854775807","automaticUserFileMutationAuthorityAdded":0,"pauseDiagnostic":pause_diagnostic}),
        )?,
    )?;
    Ok(())
}
