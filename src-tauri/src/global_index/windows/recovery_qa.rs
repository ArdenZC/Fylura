//! Native-qa-only installed service regression on the qualification VHD/profile.
use super::WindowsGlobalIndexProvider;
use crate::db::Database;
use crate::global_index::coordinator::{
    run_service_recovery_qa_cycle, GlobalIndexError, GlobalIndexProvider, GlobalIndexSink,
};
use crate::global_index::models::*;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};

struct ProbeProvider {
    inner: WindowsGlobalIndexProvider,
    source_id: String,
    pause_once: AtomicBool,
    outcome: Mutex<&'static str>,
    rebuilds: AtomicUsize,
}
impl ProbeProvider {
    fn record(&self, result: Result<(), GlobalIndexError>) -> Result<(), GlobalIndexError> {
        *self.outcome.lock().unwrap() = match &result {
            Ok(()) => "ready",
            Err(GlobalIndexError::Paused) => "paused",
            Err(GlobalIndexError::RebuildRequired(_)) => "rebuild_required",
            _ => "failure",
        };
        if !self.inner.service_available.load(Ordering::Acquire) {
            return Err(GlobalIndexError::Provider(
                "QA refuses direct-provider fallback".into(),
            ));
        }
        result
    }
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
        self.sink
            .admit_source_operation(source, operation_id, kind, route)
    }
    fn source_operation_admitted(&self, volume_id: &str) -> bool {
        self.sink.source_operation_admitted(volume_id)
    }
    fn mark_source_operation_unknown(&mut self, volume_id: &str) -> Result<(), GlobalIndexError> {
        self.sink.mark_source_operation_unknown(volume_id)
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
        self.sink.finish_source_operation(
            volume_id,
            status,
            error,
            full_index_at,
            incremental_sync_at,
            complete_rebuild_intent,
        )
    }
    fn set_source_provider(&mut self, id: &str, provider: &str) -> Result<(), GlobalIndexError> {
        self.sink.set_source_provider(id, provider)?;
        // Pause after the real service begins streaming, before its MFT read.
        if self.provider.pause_once.swap(false, Ordering::AcqRel) {
            self.provider.inner.pause()?;
        }
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
        self.sink.checkpoint(volume_id, journal_id, journal_cursor)
    }
    fn set_source_state(
        &mut self,
        volume_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), GlobalIndexError> {
        self.sink.set_source_state(volume_id, status, error)
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
        source_id: source.id.clone(),
        pause_once: AtomicBool::new(true),
        outcome: Mutex::new("none"),
        rebuilds: AtomicUsize::new(0),
    };
    run_service_recovery_qa_cycle(&provider, &db)?;
    let paused = db.get_global_volume(&source.id)?.ok_or("source absent")?;
    if paused.index_status != INDEX_STATUS_PAUSED
        || paused.last_error.is_some()
        || *provider.outcome.lock().unwrap() != "paused"
    {
        return Err("typed pipe Pause / durable paused assertion failed".into());
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
            &serde_json::json!({"protocol":3,"directFallbackAllowed":false,"pausedViaPipe":true,"rebuildRequiredViaPipe":true,"firstCycleStatus":required.index_status,"nextAdmittedRebuildCount":1,"finalStatus":rebuilt.index_status,"lastFullIndexAt":rebuilt.last_full_index_at,"seededJournalId":"18446744073709551615","seededCursor":"9223372036854775807","automaticUserFileMutationAuthorityAdded":0}),
        )?,
    )?;
    Ok(())
}
