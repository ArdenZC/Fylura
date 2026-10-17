use super::super::*;
use super::analysis::{bump_dedupe_authority_tx, invalidate_analysis_findings_for_root_tx};
use super::dedupe::{invalidate_file_in_transaction, invalidate_stale_files_in_transaction};
use super::library_identity::{
    advance_root_epoch_tx, ensure_neutral_source_attribution_tx, mark_root_path_absent_tx,
    observe_source_for_root_tx, reconcile_root_membership_tx, SourceObservationContext,
};
use super::*;
use crate::ids::new_job_id;
use crate::path_filter::{
    generated_dir_variant_bases, legacy_ignored_dir_names, managed_protected_roots,
    metadata_is_link_or_reparse, ManagedPathFilter,
};
use rusqlite::{
    params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension, Row,
    Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

#[cfg(all(test, feature = "performance-test-tauri"))]
use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::Instant,
};

#[derive(Clone, Copy, Default)]
struct ManagedScanTraceCheckout {
    #[cfg(all(test, feature = "performance-test-tauri"))]
    trace_id: Option<u64>,
    #[cfg(all(test, feature = "performance-test-tauri"))]
    connection_id: Option<u64>,
    #[cfg(all(test, feature = "performance-test-tauri"))]
    pool_checkout_wait_us: Option<u128>,
}

macro_rules! checkout_managed_scan_write_connection {
    ($database:expr, $run_id:expr) => {{
        #[cfg(all(test, feature = "performance-test-tauri"))]
        {
            let run_id: Option<&str> = $run_id;
            if run_id.is_some_and(|run_id| run_id.starts_with("issue345-d4-owner-")) {
                start_d4_managed_scan_write_trace($database.path());
            }
            let trace_id = managed_scan_write_trace_for_path($database.path());
            let checkout_started_at = trace_id.map(|_| Instant::now());
            let connection = $database.conn()?;
            let checkout = match (trace_id, checkout_started_at) {
                (Some(trace_id), Some(started_at)) => ManagedScanTraceCheckout {
                    trace_id: Some(trace_id),
                    connection_id: Some(managed_scan_trace_connection_id(trace_id, &connection)),
                    pool_checkout_wait_us: Some(started_at.elapsed().as_micros()),
                },
                _ => ManagedScanTraceCheckout::default(),
            };
            Ok::<_, DbError>((connection, checkout))
        }
        #[cfg(not(all(test, feature = "performance-test-tauri")))]
        {
            let _ = $run_id;
            Ok::<_, DbError>(($database.conn()?, ManagedScanTraceCheckout::default()))
        }
    }};
}

macro_rules! begin_managed_scan_write_transaction {
    ($conn:expr, $operation:expr, $run_id:expr) => {{
        begin_managed_scan_write_transaction!(
            $conn,
            $operation,
            $run_id,
            None::<&str>,
            ManagedScanTraceCheckout::default()
        )
    }};
    ($conn:expr, $operation:expr, $run_id:expr, $issue366_diagnostic_key:expr) => {{
        begin_managed_scan_write_transaction!(
            $conn,
            $operation,
            $run_id,
            $issue366_diagnostic_key,
            ManagedScanTraceCheckout::default()
        )
    }};
    ($conn:expr, $operation:expr, $run_id:expr, $issue366_diagnostic_key:expr, $trace_checkout:expr) => {{
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let managed_scan_operation: &str = $operation;
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let managed_scan_run_id: Option<&str> = $run_id;
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let managed_scan_trace_checkout: ManagedScanTraceCheckout = $trace_checkout;
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let issue366_diagnostic_key: Option<&str> = $issue366_diagnostic_key;
        #[cfg(all(test, feature = "performance-test-tauri"))]
        if let Some(key) = issue366_diagnostic_key {
            eprintln!(
                "[issue366-begin] request_key={key} stage=before_transaction_with_behavior elapsed_from_test_start_us={:?}",
                issue366_elapsed_from_test_start_us()
            );
        }
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let issue366_thread_cpu_before = issue366_diagnostic_key
            .and_then(|_| issue366_thread_cpu_time_ns());
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let issue366_begin_wall_started =
            issue366_diagnostic_key.map(|_| std::time::Instant::now());
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let trace_transaction_attempt = start_managed_scan_trace_transaction(
            managed_scan_trace_checkout,
            managed_scan_operation,
            managed_scan_run_id,
        );
        let transaction_result = $conn.transaction_with_behavior(TransactionBehavior::Immediate);
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let trace_transaction = finish_managed_scan_trace_begin(
            trace_transaction_attempt,
            &transaction_result,
        );
        #[cfg(all(test, feature = "performance-test-tauri"))]
        if let Some(key) = issue366_diagnostic_key {
            let issue366_thread_cpu_after = issue366_thread_cpu_time_ns();
            let issue366_wall_elapsed_us = issue366_begin_wall_started
                .expect("diagnostic timer exists when a key is selected")
                .elapsed()
                .as_micros();
            let issue366_thread_cpu_elapsed_ns = issue366_thread_cpu_before
                .zip(issue366_thread_cpu_after)
                .map(|(before, after)| after.saturating_sub(before));
            let (outcome, primary_code, extended_code) = match &transaction_result {
                Ok(_) => ("success", None, None),
                Err(rusqlite::Error::SqliteFailure(code, _)) => (
                    "sqlite_error",
                    Some(code.extended_code & 0xff),
                    Some(code.extended_code),
                ),
                Err(_) => ("other_error", None, None),
            };
            eprintln!(
                "[issue366-begin] request_key={key} stage=transaction_with_behavior_returned elapsed_from_test_start_us={:?} begin_wall_us={issue366_wall_elapsed_us} begin_thread_cpu_ns={issue366_thread_cpu_elapsed_ns:?} outcome={outcome} primary_code={primary_code:?} extended_code={extended_code:?}",
                issue366_elapsed_from_test_start_us()
            );
        }
        #[cfg(all(test, feature = "performance-test-tauri"))]
        if let Err(rusqlite::Error::SqliteFailure(code, _)) = &transaction_result {
            eprintln!(
                "[issue345-sqlite] operation={} run_id={} sqlite_primary={} sqlite_extended={}",
                managed_scan_operation,
                managed_scan_run_id.unwrap_or("unavailable"),
                code.extended_code & 0xff,
                code.extended_code
            );
        }
        #[cfg(all(test, feature = "performance-test-tauri"))]
        {
            transaction_result
                .map(|transaction| {
                    ManagedScanWriteTransaction::new(transaction, trace_transaction)
                })
                .map_err(DbError::from)
        }
        #[cfg(not(all(test, feature = "performance-test-tauri")))]
        {
            let _ = $trace_checkout;
            transaction_result.map_err(DbError::from)
        }
    }};
}

#[cfg(all(test, feature = "performance-test-tauri"))]
const MANAGED_SCAN_TRACE_MAX_EVENTS: usize = 16_384;

#[cfg(all(test, feature = "performance-test-tauri"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ManagedScanWriteTraceControl {
    ScopedPressure,
    UntrackedWriterNegative,
}

#[cfg(all(test, feature = "performance-test-tauri"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagedScanTraceMode {
    ScopedPressure,
    UntrackedWriterNegative,
    D4KnownOwnerPositive,
}

#[cfg(all(test, feature = "performance-test-tauri"))]
#[derive(Default, Clone, Copy)]
pub(crate) struct ManagedScanWriteTraceSummary {
    pub(crate) transaction_count: usize,
    pub(crate) busy_count: usize,
    pub(crate) busy_without_observed_writer_count: usize,
    pub(crate) d4_positive_control_passed: bool,
    pub(crate) active_transactions_at_finish: usize,
    pub(crate) dropped_event_count: usize,
}

#[cfg(all(test, feature = "performance-test-tauri"))]
pub(crate) struct ManagedScanWriteTraceGuard {
    trace_id: u64,
    finished: bool,
}

#[cfg(all(test, feature = "performance-test-tauri"))]
impl ManagedScanWriteTraceGuard {
    pub(crate) fn enable_for_database(
        database: &Database,
        label: &str,
        control: ManagedScanWriteTraceControl,
    ) -> Self {
        let trace_id = start_managed_scan_write_trace(
            database.path(),
            label,
            match control {
                ManagedScanWriteTraceControl::ScopedPressure => {
                    ManagedScanTraceMode::ScopedPressure
                }
                ManagedScanWriteTraceControl::UntrackedWriterNegative => {
                    ManagedScanTraceMode::UntrackedWriterNegative
                }
            },
        );
        Self {
            trace_id,
            finished: false,
        }
    }

    pub(crate) fn finish(mut self) -> ManagedScanWriteTraceSummary {
        self.finished = true;
        finish_managed_scan_write_trace(self.trace_id)
    }
}

#[cfg(all(test, feature = "performance-test-tauri"))]
impl Drop for ManagedScanWriteTraceGuard {
    fn drop(&mut self) {
        if !self.finished {
            let _ = finish_managed_scan_write_trace(self.trace_id);
        }
    }
}

#[cfg(all(test, feature = "performance-test-tauri"))]
struct ManagedScanWriteTraceState {
    trace_id: u64,
    label: String,
    database_path: PathBuf,
    mode: ManagedScanTraceMode,
    started_at: Instant,
    next_event_sequence: u64,
    next_transaction_id: u64,
    next_connection_id: u64,
    connection_ids: HashMap<usize, u64>,
    active_transactions: HashMap<u64, ActiveManagedScanWriteTransaction>,
    events: Vec<String>,
    transaction_count: usize,
    busy_count: usize,
    busy_without_observed_writer_count: usize,
    dropped_event_count: usize,
    d4_owner_active_at_contender_begin: bool,
    d4_owner_commit_sequence: Option<u64>,
    d4_positive_control_passed: bool,
}

#[cfg(all(test, feature = "performance-test-tauri"))]
struct ActiveManagedScanWriteTransaction {
    transaction_id: u64,
    connection_id: u64,
    operation: String,
    run_id: Option<String>,
    acquired_at: Instant,
}

#[cfg(all(test, feature = "performance-test-tauri"))]
struct ManagedScanTraceTransactionAttempt {
    trace_id: u64,
    transaction_id: u64,
    connection_id: u64,
    operation: String,
    run_id: Option<String>,
    started_at: Instant,
    d4_owner_active_at_start: bool,
}

#[cfg(all(test, feature = "performance-test-tauri"))]
struct ManagedScanTraceTransactionContext {
    trace_id: u64,
    transaction_id: u64,
    connection_id: u64,
    operation: String,
    run_id: Option<String>,
    acquired_at: Instant,
}

#[cfg(all(test, feature = "performance-test-tauri"))]
static NEXT_MANAGED_SCAN_TRACE_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(all(test, feature = "performance-test-tauri"))]
static MANAGED_SCAN_WRITE_TRACE_ENABLED: AtomicBool = AtomicBool::new(false);

#[cfg(all(test, feature = "performance-test-tauri"))]
static MANAGED_SCAN_WRITE_TRACE: OnceLock<Mutex<Option<ManagedScanWriteTraceState>>> =
    OnceLock::new();

#[cfg(all(test, feature = "performance-test-tauri"))]
fn managed_scan_write_trace_state() -> &'static Mutex<Option<ManagedScanWriteTraceState>> {
    MANAGED_SCAN_WRITE_TRACE.get_or_init(|| Mutex::new(None))
}

#[cfg(all(test, feature = "performance-test-tauri"))]
impl ManagedScanWriteTraceState {
    fn new(trace_id: u64, database_path: &Path, label: &str, mode: ManagedScanTraceMode) -> Self {
        Self {
            trace_id,
            label: label.to_string(),
            database_path: database_path.to_path_buf(),
            mode,
            started_at: Instant::now(),
            next_event_sequence: 1,
            next_transaction_id: 1,
            next_connection_id: 1,
            connection_ids: HashMap::new(),
            active_transactions: HashMap::new(),
            events: Vec::new(),
            transaction_count: 0,
            busy_count: 0,
            busy_without_observed_writer_count: 0,
            dropped_event_count: 0,
            d4_owner_active_at_contender_begin: false,
            d4_owner_commit_sequence: None,
            d4_positive_control_passed: false,
        }
    }

    fn event(&mut self, detail: impl AsRef<str>) -> u64 {
        let sequence = self.next_event_sequence;
        self.next_event_sequence = self.next_event_sequence.saturating_add(1);
        let elapsed_us = self.started_at.elapsed().as_micros();
        if self.events.len() < MANAGED_SCAN_TRACE_MAX_EVENTS {
            self.events.push(format!(
                "seq={sequence} elapsed_us={elapsed_us} {}",
                detail.as_ref()
            ));
        } else {
            self.dropped_event_count = self.dropped_event_count.saturating_add(1);
        }
        sequence
    }

    fn active_writer_snapshot(&self) -> String {
        let mut writers = self
            .active_transactions
            .values()
            .map(|active| {
                format!(
                    "tx{}:conn{}:op={}:run_id={}:active_us={}",
                    active.transaction_id,
                    active.connection_id,
                    active.operation,
                    active.run_id.as_deref().unwrap_or("unavailable"),
                    active.acquired_at.elapsed().as_micros()
                )
            })
            .collect::<Vec<_>>();
        writers.sort_unstable();
        format!("[{}]", writers.join(";"))
    }
}

#[cfg(all(test, feature = "performance-test-tauri"))]
fn start_managed_scan_write_trace(
    database_path: &Path,
    label: &str,
    mode: ManagedScanTraceMode,
) -> u64 {
    let trace_id = NEXT_MANAGED_SCAN_TRACE_ID.fetch_add(1, Ordering::Relaxed);
    let mut current = managed_scan_write_trace_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(
        current.is_none(),
        "managed scan trace scopes must not overlap within one test process"
    );
    *current = Some(ManagedScanWriteTraceState::new(
        trace_id,
        database_path,
        label,
        mode,
    ));
    MANAGED_SCAN_WRITE_TRACE_ENABLED.store(true, Ordering::Release);
    trace_id
}

#[cfg(all(test, feature = "performance-test-tauri"))]
fn start_d4_managed_scan_write_trace(database_path: &Path) {
    let mut current = managed_scan_write_trace_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if current.is_none() {
        let trace_id = NEXT_MANAGED_SCAN_TRACE_ID.fetch_add(1, Ordering::Relaxed);
        let mut trace = ManagedScanWriteTraceState::new(
            trace_id,
            database_path,
            "issue345-d4-known-owner-positive-control",
            ManagedScanTraceMode::D4KnownOwnerPositive,
        );
        trace.event("control=d4_positive_control_started writer_registry=scan_query_begin_macro");
        *current = Some(trace);
        MANAGED_SCAN_WRITE_TRACE_ENABLED.store(true, Ordering::Release);
    }
}

#[cfg(all(test, feature = "performance-test-tauri"))]
fn managed_scan_write_trace_for_path(database_path: &Path) -> Option<u64> {
    if !MANAGED_SCAN_WRITE_TRACE_ENABLED.load(Ordering::Acquire) {
        return None;
    }
    managed_scan_write_trace_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .filter(|trace| trace.database_path == database_path)
        .map(|trace| trace.trace_id)
}

#[cfg(all(test, feature = "performance-test-tauri"))]
fn managed_scan_trace_connection_id(trace_id: u64, connection: &Connection) -> u64 {
    let connection_key = connection as *const Connection as usize;
    let mut current = managed_scan_write_trace_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let trace = current
        .as_mut()
        .filter(|trace| trace.trace_id == trace_id)
        .expect("connection checkout belongs to the active managed scan trace");
    if let Some(connection_id) = trace.connection_ids.get(&connection_key) {
        return *connection_id;
    }
    let connection_id = trace.next_connection_id;
    trace.next_connection_id = trace.next_connection_id.saturating_add(1);
    trace.connection_ids.insert(connection_key, connection_id);
    let sequence = trace.event(format!(
        "event=connection_checkout connection_id=c{connection_id} identity=opaque_sqlite_connection_address"
    ));
    let _ = sequence;
    connection_id
}

#[cfg(all(test, feature = "performance-test-tauri"))]
fn start_managed_scan_trace_transaction(
    checkout: ManagedScanTraceCheckout,
    operation: &str,
    run_id: Option<&str>,
) -> Option<ManagedScanTraceTransactionAttempt> {
    let trace_id = checkout.trace_id?;
    let connection_id = checkout.connection_id?;
    let mut current = managed_scan_write_trace_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let trace = current
        .as_mut()
        .filter(|trace| trace.trace_id == trace_id)?;
    let transaction_id = trace.next_transaction_id;
    trace.next_transaction_id = trace.next_transaction_id.saturating_add(1);
    trace.transaction_count = trace.transaction_count.saturating_add(1);
    let active_before = trace.active_writer_snapshot();
    let d4_owner_active_at_start = trace.mode == ManagedScanTraceMode::D4KnownOwnerPositive
        && operation == "persist_scan_batch"
        && run_id.is_some_and(|run_id| run_id.starts_with("issue345-d4-contender-"))
        && trace.active_transactions.values().any(|active| {
            active.operation == "persist_scan_batch"
                && active
                    .run_id
                    .as_deref()
                    .is_some_and(|active_run_id| active_run_id.starts_with("issue345-d4-owner-"))
        });
    if d4_owner_active_at_start {
        trace.d4_owner_active_at_contender_begin = true;
    }
    trace.event(format!(
        "event=before_begin transaction_id=t{transaction_id} connection_id=c{connection_id} operation={operation} run_id={} pool_checkout_wait_us={} active_instrumented_writers={active_before} writer_coverage=scan_query_begin_macro_only owner_inference=none",
        run_id.unwrap_or("unavailable"),
        checkout.pool_checkout_wait_us
            .map(|elapsed| elapsed.to_string())
            .unwrap_or_else(|| "unavailable".to_string())
    ));
    Some(ManagedScanTraceTransactionAttempt {
        trace_id,
        transaction_id,
        connection_id,
        operation: operation.to_string(),
        run_id: run_id.map(str::to_string),
        started_at: Instant::now(),
        d4_owner_active_at_start,
    })
}

#[cfg(all(test, feature = "performance-test-tauri"))]
fn finish_managed_scan_trace_begin(
    attempt: Option<ManagedScanTraceTransactionAttempt>,
    result: &rusqlite::Result<Transaction<'_>>,
) -> Option<ManagedScanTraceTransactionContext> {
    let attempt = attempt?;
    let begin_wait_us = attempt.started_at.elapsed().as_micros();
    let mut current = managed_scan_write_trace_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let trace = current
        .as_mut()
        .filter(|trace| trace.trace_id == attempt.trace_id)?;
    match result {
        Ok(_) => {
            let sequence = trace.event(format!(
                "event=acquired transaction_id=t{} connection_id=c{} operation={} run_id={} begin_writer_wait_us={begin_wait_us} pool_wait_excluded=true",
                attempt.transaction_id,
                attempt.connection_id,
                attempt.operation,
                attempt.run_id.as_deref().unwrap_or("unavailable")
            ));
            trace.active_transactions.insert(
                attempt.transaction_id,
                ActiveManagedScanWriteTransaction {
                    transaction_id: attempt.transaction_id,
                    connection_id: attempt.connection_id,
                    operation: attempt.operation.clone(),
                    run_id: attempt.run_id.clone(),
                    acquired_at: Instant::now(),
                },
            );
            if trace.mode == ManagedScanTraceMode::D4KnownOwnerPositive
                && attempt.operation == "persist_scan_batch"
                && attempt
                    .run_id
                    .as_deref()
                    .is_some_and(|run_id| run_id.starts_with("issue345-d4-contender-"))
                && attempt.d4_owner_active_at_start
                && trace
                    .d4_owner_commit_sequence
                    .is_some_and(|owner_commit| owner_commit < sequence && begin_wait_us >= 100_000)
            {
                trace.d4_positive_control_passed = true;
            }
            Some(ManagedScanTraceTransactionContext {
                trace_id: attempt.trace_id,
                transaction_id: attempt.transaction_id,
                connection_id: attempt.connection_id,
                operation: attempt.operation,
                run_id: attempt.run_id,
                acquired_at: Instant::now(),
            })
        }
        Err(error) => {
            let active_at_busy = trace.active_writer_snapshot();
            let (result_kind, primary_code, extended_code) = match error {
                rusqlite::Error::SqliteFailure(code, _) => (
                    if code.extended_code & 0xff == 5 {
                        "sqlite_busy"
                    } else {
                        "sqlite_error"
                    },
                    Some(code.extended_code & 0xff),
                    Some(code.extended_code),
                ),
                _ => ("other_error", None, None),
            };
            if result_kind == "sqlite_busy" {
                trace.busy_count = trace.busy_count.saturating_add(1);
                if trace.active_transactions.is_empty() {
                    trace.busy_without_observed_writer_count =
                        trace.busy_without_observed_writer_count.saturating_add(1);
                }
            }
            trace.event(format!(
                "event=begin_failed transaction_id=t{} connection_id=c{} operation={} run_id={} result={result_kind} primary_code={} extended_code={} begin_wait_us={begin_wait_us} pool_wait_excluded=true active_instrumented_writers_at_failure={active_at_busy} writer_coverage=scan_query_begin_macro_only owner_status=OWNER_UNKNOWN owner_inference=none",
                attempt.transaction_id,
                attempt.connection_id,
                attempt.operation,
                attempt.run_id.as_deref().unwrap_or("unavailable"),
                primary_code.map(|code| code.to_string()).unwrap_or_else(|| "none".to_string()),
                extended_code.map(|code| code.to_string()).unwrap_or_else(|| "none".to_string())
            ));
            None
        }
    }
}

#[cfg(all(test, feature = "performance-test-tauri"))]
fn finish_managed_scan_write_transaction(
    transaction: ManagedScanTraceTransactionContext,
    outcome: &str,
    release_call_us: u128,
) {
    let mut current = managed_scan_write_trace_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(trace) = current
        .as_mut()
        .filter(|trace| trace.trace_id == transaction.trace_id)
    else {
        return;
    };
    let active = trace
        .active_transactions
        .remove(&transaction.transaction_id);
    let hold_us = active
        .as_ref()
        .map(|active| active.acquired_at.elapsed().as_micros())
        .or_else(|| Some(transaction.acquired_at.elapsed().as_micros()))
        .unwrap_or_default();
    let sequence = trace.event(format!(
        "event={outcome} transaction_id=t{} connection_id=c{} operation={} run_id={} hold_us={hold_us} release_timestamp_elapsed_us={} release_call_us={release_call_us}",
        transaction.transaction_id,
        transaction.connection_id,
        transaction.operation,
        transaction.run_id.as_deref().unwrap_or("unavailable"),
        trace.started_at.elapsed().as_micros()
    ));
    if trace.mode == ManagedScanTraceMode::D4KnownOwnerPositive
        && outcome == "committed"
        && transaction.operation == "persist_scan_batch"
        && transaction
            .run_id
            .as_deref()
            .is_some_and(|run_id| run_id.starts_with("issue345-d4-owner-"))
    {
        trace.d4_owner_commit_sequence = Some(sequence);
    }
}

#[cfg(all(test, feature = "performance-test-tauri"))]
fn record_managed_scan_trace_ownership(
    transaction: &ManagedScanTraceTransactionContext,
    stage: &str,
    detail: &str,
) {
    let mut current = managed_scan_write_trace_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(trace) = current
        .as_mut()
        .filter(|trace| trace.trace_id == transaction.trace_id)
    else {
        return;
    };
    trace.event(format!(
        "event=revision_ownership transaction_id=t{} operation={} run_id={} stage={stage} {detail}",
        transaction.transaction_id,
        transaction.operation,
        transaction.run_id.as_deref().unwrap_or("unavailable")
    ));
}

#[cfg(all(test, feature = "performance-test-tauri"))]
fn record_managed_scan_trace_test_hook(run_id: &str, point: ManagedScanWriteTestPoint) {
    let mut current = managed_scan_write_trace_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(trace) = current.as_mut() else {
        return;
    };
    if trace.mode == ManagedScanTraceMode::D4KnownOwnerPositive {
        let active = trace.active_writer_snapshot();
        trace.event(format!(
            "event=d4_test_hook run_id={run_id} point={point:?} active_instrumented_writers={active} control_owner_inference=from_existing_d4_barriers"
        ));
    }
}

#[cfg(all(test, feature = "performance-test-tauri"))]
fn finish_d4_managed_scan_write_trace() {
    let trace_id = managed_scan_write_trace_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .filter(|trace| trace.mode == ManagedScanTraceMode::D4KnownOwnerPositive)
        .map(|trace| trace.trace_id);
    if let Some(trace_id) = trace_id {
        let _ = finish_managed_scan_write_trace(trace_id);
    }
}

#[cfg(all(test, feature = "performance-test-tauri"))]
fn finish_managed_scan_write_trace(trace_id: u64) -> ManagedScanWriteTraceSummary {
    let mut current = managed_scan_write_trace_state()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(mut trace) = current.take().filter(|trace| trace.trace_id == trace_id) else {
        return ManagedScanWriteTraceSummary::default();
    };
    MANAGED_SCAN_WRITE_TRACE_ENABLED.store(false, Ordering::Release);
    let summary = ManagedScanWriteTraceSummary {
        transaction_count: trace.transaction_count,
        busy_count: trace.busy_count,
        busy_without_observed_writer_count: trace.busy_without_observed_writer_count,
        d4_positive_control_passed: trace.d4_positive_control_passed,
        active_transactions_at_finish: trace.active_transactions.len(),
        dropped_event_count: trace.dropped_event_count,
    };
    let mode = match trace.mode {
        ManagedScanTraceMode::ScopedPressure => "scoped_pressure",
        ManagedScanTraceMode::UntrackedWriterNegative => "untracked_writer_negative_control",
        ManagedScanTraceMode::D4KnownOwnerPositive => "d4_known_owner_positive_control",
    };
    for event in trace.events.drain(..) {
        eprintln!(
            "[managed-scan-trace] trace={} label={} mode={} {}",
            trace.trace_id, trace.label, mode, event
        );
    }
    eprintln!(
        "[managed-scan-trace] trace={} label={} mode={} summary transactions={} busy={} busy_without_observed_writers={} d4_positive_control_passed={} active_at_finish={} dropped_events={} writer_coverage=scan_query_begin_macro_only cross_writer_coverage=partial causal_owner_claims=none",
        trace.trace_id,
        trace.label,
        mode,
        summary.transaction_count,
        summary.busy_count,
        summary.busy_without_observed_writer_count,
        summary.d4_positive_control_passed,
        summary.active_transactions_at_finish,
        summary.dropped_event_count
    );
    summary
}

#[cfg(all(test, feature = "performance-test-tauri"))]
thread_local! {
    static MANAGED_SCAN_SOURCE_IDENTITY_TRACE_SCOPE: RefCell<Option<ManagedScanSourceIdentityMetrics>> = const {
        RefCell::new(None)
    };
}

#[cfg(all(test, feature = "performance-test-tauri"))]
#[derive(Default)]
struct ManagedScanSourceIdentityMetrics {
    trace_id: u64,
    transaction_id: u64,
    calls: u64,
    elapsed_us: u128,
}

#[cfg(all(test, feature = "performance-test-tauri"))]
struct ManagedScanSourceIdentityTraceScope {
    trace_id: u64,
    transaction_id: u64,
}

#[cfg(all(test, feature = "performance-test-tauri"))]
impl ManagedScanSourceIdentityTraceScope {
    fn begin(transaction: Option<&ManagedScanTraceTransactionContext>) -> Option<Self> {
        let transaction = transaction?;
        MANAGED_SCAN_SOURCE_IDENTITY_TRACE_SCOPE.with(|scope| {
            let mut scope = scope.borrow_mut();
            if scope.is_some() {
                return None;
            }
            *scope = Some(ManagedScanSourceIdentityMetrics {
                trace_id: transaction.trace_id,
                transaction_id: transaction.transaction_id,
                ..ManagedScanSourceIdentityMetrics::default()
            });
            Some(Self {
                trace_id: transaction.trace_id,
                transaction_id: transaction.transaction_id,
            })
        })
    }
}

#[cfg(all(test, feature = "performance-test-tauri"))]
impl Drop for ManagedScanSourceIdentityTraceScope {
    fn drop(&mut self) {
        let metrics = MANAGED_SCAN_SOURCE_IDENTITY_TRACE_SCOPE.with(|scope| {
            let mut scope = scope.borrow_mut();
            if scope.as_ref().is_some_and(|metrics| {
                metrics.trace_id == self.trace_id && metrics.transaction_id == self.transaction_id
            }) {
                scope.take()
            } else {
                None
            }
        });
        if let Some(metrics) = metrics {
            let mut current = managed_scan_write_trace_state()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(trace) = current
                .as_mut()
                .filter(|trace| trace.trace_id == self.trace_id)
            {
                trace.event(format!(
                    "event=physical_identity_capture_aggregate transaction_id=t{} calls={} cumulative_native_us={}",
                    self.transaction_id, metrics.calls, metrics.elapsed_us
                ));
            }
        }
    }
}

#[cfg(all(test, feature = "performance-test-tauri"))]
fn with_managed_scan_source_identity_capture<T>(capture: impl FnOnce() -> T) -> T {
    if !MANAGED_SCAN_WRITE_TRACE_ENABLED.load(Ordering::Acquire) {
        return capture();
    }
    MANAGED_SCAN_SOURCE_IDENTITY_TRACE_SCOPE.with(|scope| {
        let mut scope = scope.borrow_mut();
        if let Some(metrics) = scope.as_mut() {
            let started_at = Instant::now();
            let result = capture();
            metrics.calls = metrics.calls.saturating_add(1);
            metrics.elapsed_us = metrics
                .elapsed_us
                .saturating_add(started_at.elapsed().as_micros());
            result
        } else {
            drop(scope);
            capture()
        }
    })
}

#[cfg(all(test, feature = "performance-test-tauri"))]
struct ManagedScanWriteTransaction<'conn> {
    transaction: Option<Transaction<'conn>>,
    trace: Option<ManagedScanTraceTransactionContext>,
}

#[cfg(all(test, feature = "performance-test-tauri"))]
impl<'conn> ManagedScanWriteTransaction<'conn> {
    fn new(
        transaction: Transaction<'conn>,
        trace: Option<ManagedScanTraceTransactionContext>,
    ) -> Self {
        Self {
            transaction: Some(transaction),
            trace,
        }
    }

    fn commit(mut self) -> rusqlite::Result<()> {
        let transaction = self
            .transaction
            .take()
            .expect("managed scan transaction is present before commit");
        let release_started_at = Instant::now();
        let result = transaction.commit();
        let release_call_us = release_started_at.elapsed().as_micros();
        if let Some(trace) = self.trace.take() {
            finish_managed_scan_write_transaction(
                trace,
                if result.is_ok() {
                    "committed"
                } else {
                    "commit_failed_released"
                },
                release_call_us,
            );
        }
        result
    }

    #[allow(dead_code)]
    fn rollback(mut self) -> rusqlite::Result<()> {
        let transaction = self
            .transaction
            .take()
            .expect("managed scan transaction is present before rollback");
        let release_started_at = Instant::now();
        let result = transaction.rollback();
        let release_call_us = release_started_at.elapsed().as_micros();
        if let Some(trace) = self.trace.take() {
            finish_managed_scan_write_transaction(
                trace,
                if result.is_ok() {
                    "rolled_back"
                } else {
                    "rollback_failed_released"
                },
                release_call_us,
            );
        }
        result
    }

    fn record_revision_ownership(&self, stage: &str, detail: &str) {
        if let Some(trace) = &self.trace {
            record_managed_scan_trace_ownership(trace, stage, detail);
        }
    }

    fn source_identity_trace_scope(&self) -> Option<ManagedScanSourceIdentityTraceScope> {
        ManagedScanSourceIdentityTraceScope::begin(self.trace.as_ref())
    }
}

#[cfg(all(test, feature = "performance-test-tauri"))]
impl<'conn> std::ops::Deref for ManagedScanWriteTransaction<'conn> {
    type Target = Transaction<'conn>;

    fn deref(&self) -> &Self::Target {
        self.transaction
            .as_ref()
            .expect("managed scan transaction remains present while borrowed")
    }
}

#[cfg(all(test, feature = "performance-test-tauri"))]
impl<'conn> std::ops::DerefMut for ManagedScanWriteTransaction<'conn> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.transaction
            .as_mut()
            .expect("managed scan transaction remains present while mutably borrowed")
    }
}

#[cfg(all(test, feature = "performance-test-tauri"))]
impl Drop for ManagedScanWriteTransaction<'_> {
    fn drop(&mut self) {
        if let Some(transaction) = self.transaction.take() {
            let release_started_at = Instant::now();
            drop(transaction);
            let release_call_us = release_started_at.elapsed().as_micros();
            if let Some(trace) = self.trace.take() {
                finish_managed_scan_write_transaction(
                    trace,
                    "rolled_back_on_drop",
                    release_call_us,
                );
            }
        }
    }
}

#[cfg(test)]
thread_local! {
    static ISSUE366_TEST_START: std::cell::Cell<Option<std::time::Instant>> = const {
        std::cell::Cell::new(None)
    };
}

#[cfg(test)]
fn issue366_set_test_start(started_at: std::time::Instant) {
    ISSUE366_TEST_START.with(|origin| origin.set(Some(started_at)));
}

#[cfg(all(test, feature = "performance-test-tauri"))]
fn issue366_elapsed_from_test_start_us() -> Option<u128> {
    ISSUE366_TEST_START.with(|origin| {
        origin
            .get()
            .map(|started_at| started_at.elapsed().as_micros())
    })
}

#[cfg(test)]
fn issue366_thread_cpu_time_ns() -> Option<u128> {
    #[cfg(target_os = "macos")]
    {
        let mut value = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let result = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut value) };
        if result == 0 && value.tv_sec >= 0 && value.tv_nsec >= 0 {
            return Some(value.tv_sec as u128 * 1_000_000_000 + value.tv_nsec as u128);
        }
        None
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(all(test, feature = "performance-test-tauri"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ManagedScanWriteTestPoint {
    BeforeTransaction,
    TransactionAcquired,
    AfterCommit,
}

#[cfg(all(test, feature = "performance-test-tauri"))]
pub(crate) type ManagedScanWriteTestHook =
    std::sync::Arc<dyn Fn(&str, ManagedScanWriteTestPoint) + Send + Sync + 'static>;

#[cfg(all(test, feature = "performance-test-tauri"))]
static MANAGED_SCAN_WRITE_TEST_HOOK: std::sync::OnceLock<
    std::sync::Mutex<Option<ManagedScanWriteTestHook>>,
> = std::sync::OnceLock::new();

#[cfg(all(test, feature = "performance-test-tauri"))]
pub(crate) fn set_managed_scan_write_test_hook(hook: Option<ManagedScanWriteTestHook>) {
    let clearing = hook.is_none();
    *MANAGED_SCAN_WRITE_TEST_HOOK
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = hook;
    #[cfg(all(test, feature = "performance-test-tauri"))]
    if clearing {
        finish_d4_managed_scan_write_trace();
    }
    #[cfg(not(all(test, feature = "performance-test-tauri")))]
    let _ = clearing;
}

#[cfg(all(test, feature = "performance-test-tauri"))]
fn invoke_managed_scan_write_test_hook(run_id: &str, point: ManagedScanWriteTestPoint) {
    // The dedicated D4 observation uses this prefix. Workspace Foundation's
    // latency measurements never take the hook lock or execute a callback.
    if !run_id.starts_with("issue345-d4-") {
        return;
    }
    #[cfg(all(test, feature = "performance-test-tauri"))]
    record_managed_scan_trace_test_hook(run_id, point);
    let hook = MANAGED_SCAN_WRITE_TEST_HOOK
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if let Some(hook) = hook {
        hook(run_id, point);
    }
}

const WATCHER_RECONCILIATION_MAX_AUTOMATIC_ATTEMPTS: usize = 3;
const WATCHER_RECONCILIATION_RETRY_DELAYS_SECONDS: [i64; 2] = [2, 10];

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedScanRequest {
    pub roots: Vec<String>,
    #[serde(default)]
    pub request_key: Option<String>,
    #[serde(default)]
    pub dedupe: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRootDto {
    pub id: String,
    pub normalized_path: String,
    pub display_name: String,
    pub source_kind: String,
    pub enabled: bool,
    pub health_status: String,
    pub current_generation: i64,
    pub active_run_id: Option<String>,
    pub active_generation: Option<i64>,
    pub revision: i64,
    pub last_successful_generation: Option<i64>,
    pub last_full_scan_at: Option<i64>,
    pub needs_reconciliation: bool,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub watcher_revision: i64,
    pub watcher_applied_revision: i64,
    pub watcher_last_event_at: Option<i64>,
    pub watcher_last_applied_at: Option<i64>,
    pub watcher_last_error_code: Option<String>,
    pub watcher_last_error_message: Option<String>,
    pub watcher_rule_recovery_required: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSessionRootDto {
    pub session_id: String,
    pub requested_index: i64,
    pub requested_path: String,
    pub normalized_requested_path: String,
    pub resolution: String,
    pub effective_root_id: Option<String>,
    pub effective_path: Option<String>,
    pub effective_index: Option<i64>,
    pub run_id: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRunDto {
    pub id: String,
    pub scan_root_id: String,
    pub root_path: String,
    pub generation: i64,
    pub parent_session_id: Option<String>,
    pub status: String,
    pub phase: String,
    pub scanned_files: i64,
    pub scanned_directories: i64,
    pub processed_bytes: i64,
    pub warnings_count: i64,
    pub errors_count: i64,
    pub metadata_error_count: i64,
    pub coverage_error_count: i64,
    pub coverage_complete: bool,
    pub stale_reconciliation_allowed: bool,
    pub cancel_requested: bool,
    pub revision: i64,
    pub session_revision: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub last_checkpoint_at: Option<i64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub result_json: Option<String>,
    pub watcher_revision_at_start: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSessionDto {
    pub id: String,
    pub request_key: Option<String>,
    pub canonical_request_hash: Option<String>,
    pub status: String,
    pub phase: String,
    pub cancel_requested: bool,
    pub requested_root_count: i64,
    pub effective_root_count: i64,
    pub completed_root_count: i64,
    pub failed_root_count: i64,
    pub cancelled_root_count: i64,
    pub covered_root_count: i64,
    pub unstarted_root_count: i64,
    pub dedupe_requested: bool,
    pub dedupe_dispatch_state: String,
    pub dedupe_attempt_count: i64,
    pub dedupe_job_id: Option<String>,
    pub dedupe_last_error: Option<String>,
    pub scanned_files: i64,
    pub scanned_directories: i64,
    pub warnings_count: i64,
    pub errors_count: i64,
    pub revision: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub last_checkpoint_at: Option<i64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub result_json: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub roots: Vec<ScanSessionRootDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedScanStartDto {
    pub session: ScanSessionDto,
    pub runs: Vec<ScanRunDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedScanSnapshotDto {
    pub session: ScanSessionDto,
    pub runs: Vec<ScanRunDto>,
}

#[derive(Debug, Clone)]
pub(crate) struct ScanAdmissionOptions {
    pub request: ManagedScanRequest,
    pub run_id_override: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ScanAdmission {
    pub session: ScanSessionDto,
    pub runs: Vec<ScanRunDto>,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WatcherReconciliationAdmission {
    Start { request_key: String, attempt: usize },
    Active,
    Backoff { retry_at: i64 },
    Exhausted { attempts: usize },
}

#[derive(Debug, Clone)]
pub(crate) struct ScanRunRecord {
    pub dto: ScanRunDto,
    pub lease_token: String,
    pub root_revision: i64,
    pub root_active_run_id: Option<String>,
    pub root_active_generation: Option<i64>,
    pub current_watcher_revision: i64,
    pub session_revision: i64,
    pub session_status: String,
}

#[derive(Debug, Clone)]
pub(crate) struct WatcherRootConfig {
    pub id: String,
    pub path: String,
}

pub(crate) fn settings_owned_watcher_roots(
    conn: &Connection,
) -> Result<Vec<WatcherRootConfig>, DbError> {
    let settings_json = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [crate::settings::APP_SETTINGS_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let settings = settings_json
        .map(|value| serde_json::from_str::<crate::settings::AppSettings>(&value))
        .transpose()?
        .unwrap_or_default();
    let configured_paths =
        crate::watcher::watch_paths_from_default_scan_folders(&settings.default_scan_folders)
            .into_iter()
            .map(|path| normalize_scan_root_path(&path.to_string_lossy()))
            .filter(|path| !path.is_empty())
            .map(|path| root_identity_key(&path))
            .collect::<HashSet<_>>();
    if configured_paths.is_empty() {
        return Ok(Vec::new());
    }

    let mut statement = conn.prepare(
        "SELECT id, normalized_path FROM scan_roots WHERE enabled = 1 AND source_kind = 'file_library' ORDER BY length(normalized_path) DESC, normalized_path",
    )?;
    let roots = statement
        .query_map([], |row| {
            Ok(WatcherRootConfig {
                id: row.get(0)?,
                path: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|root| {
            configured_paths.contains(&root_identity_key(&normalize_scan_root_path(&root.path)))
        })
        .collect();
    Ok(roots)
}

#[derive(Debug, Clone)]
pub(crate) struct WatcherRevisionStart {
    pub watcher_revision: i64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WatcherMutationResult {
    pub upserted_paths: Vec<String>,
    pub reconciliation_required: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ScanErrorInput {
    pub path: Option<String>,
    pub error_code: String,
    pub error_message: String,
    pub affects_coverage: bool,
    pub metadata_error: bool,
}

#[derive(Debug)]
pub(crate) struct ScanBatchInput<'a> {
    pub entries: &'a [InsertFileRequest],
    pub errors: &'a [ScanErrorInput],
    pub scanned_files: i64,
    pub scanned_directories: i64,
    pub processed_bytes: i64,
    pub warnings: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct ScanFinalizeInput {
    pub terminal_status: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub allow_stale_reconciliation: bool,
    pub rule_recovery_succeeded: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ScanFinalization {
    pub run: ScanRunRecord,
    pub session: ScanSessionDto,
    pub dedupe_pending: bool,
}

#[derive(Debug, Clone)]
struct ScanRootSeed {
    id: String,
    current_generation: i64,
    revision: i64,
    active_run_id: Option<String>,
}

#[derive(Debug, Clone)]
struct ResolvedRequestedRoot {
    requested_index: i64,
    requested_path: String,
    normalized_path: String,
    key: String,
    resolution: &'static str,
    effective_key: Option<String>,
    effective_path: Option<String>,
    effective_index: Option<i64>,
    status: &'static str,
    reason: Option<String>,
}

#[derive(Debug, Clone)]
struct EffectiveRoot {
    key: String,
    path: String,
    first_index: i64,
}

impl Database {
    pub(crate) fn admit_managed_scan(
        &self,
        options: &ScanAdmissionOptions,
    ) -> Result<ScanAdmission, DbError> {
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let issue366_diagnostic_key = options
            .request
            .request_key
            .as_deref()
            .map(str::trim)
            .filter(|value| value.starts_with("issue366-"));
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let issue366_admission_started = issue366_diagnostic_key.map(|_| std::time::Instant::now());
        #[cfg(all(test, feature = "performance-test-tauri"))]
        if let Some(key) = issue366_diagnostic_key {
            eprintln!(
                "[issue366-admission] request_key={key} stage=method_entered elapsed_from_test_start_us={:?}",
                issue366_elapsed_from_test_start_us()
            );
        }

        #[cfg(all(test, feature = "performance-test-tauri"))]
        let issue366_roots_started = issue366_diagnostic_key.map(|_| std::time::Instant::now());
        let resolved = resolve_requested_roots(&options.request.roots);
        #[cfg(all(test, feature = "performance-test-tauri"))]
        if let Some(key) = issue366_diagnostic_key {
            eprintln!(
                "[issue366-admission] request_key={key} stage=roots_resolved elapsed_from_test_start_us={:?} roots_resolution_wall_us={}",
                issue366_elapsed_from_test_start_us(),
                issue366_roots_started
                    .expect("diagnostic timer exists when a key is selected")
                    .elapsed()
                    .as_micros()
            );
        }

        #[cfg(all(test, feature = "performance-test-tauri"))]
        let issue366_hash_started = issue366_diagnostic_key.map(|_| std::time::Instant::now());
        let canonical_hash = canonical_request_hash(&resolved, options.request.dedupe)?;
        #[cfg(all(test, feature = "performance-test-tauri"))]
        if let Some(key) = issue366_diagnostic_key {
            eprintln!(
                "[issue366-admission] request_key={key} stage=request_hash_ready elapsed_from_test_start_us={:?} request_hash_wall_us={}",
                issue366_elapsed_from_test_start_us(),
                issue366_hash_started
                    .expect("diagnostic timer exists when a key is selected")
                    .elapsed()
                    .as_micros()
            );
        }

        let request_key = options
            .request
            .request_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        #[cfg(all(test, feature = "performance-test-tauri"))]
        let managed_trace_id = managed_scan_write_trace_for_path(self.path());
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let managed_trace_checkout_started = managed_trace_id.map(|_| Instant::now());
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let issue366_pool_started = issue366_diagnostic_key.map(|_| std::time::Instant::now());
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let issue366_pool_thread_cpu_before =
            issue366_diagnostic_key.and_then(|_| issue366_thread_cpu_time_ns());
        let pool_result = self.conn();
        #[cfg(all(test, feature = "performance-test-tauri"))]
        if let Some(key) = issue366_diagnostic_key {
            let pool_elapsed_us = issue366_pool_started
                .expect("diagnostic timer exists when a key is selected")
                .elapsed()
                .as_micros();
            let pool_thread_cpu_elapsed_ns = issue366_pool_thread_cpu_before
                .zip(issue366_thread_cpu_time_ns())
                .map(|(before, after)| after.saturating_sub(before));
            eprintln!(
                "[issue366-admission] request_key={key} stage=pool_connection_result elapsed_from_test_start_us={:?} pool_wait_wall_us={pool_elapsed_us} pool_wait_thread_cpu_ns={pool_thread_cpu_elapsed_ns:?} result={}",
                issue366_elapsed_from_test_start_us(),
                if pool_result.is_ok() { "ok" } else { "error" }
            );
        }
        let mut conn = pool_result?;
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let managed_trace_checkout = match (managed_trace_id, managed_trace_checkout_started) {
            (Some(trace_id), Some(started_at)) => ManagedScanTraceCheckout {
                trace_id: Some(trace_id),
                connection_id: Some(managed_scan_trace_connection_id(trace_id, &conn)),
                pool_checkout_wait_us: Some(started_at.elapsed().as_micros()),
            },
            _ => ManagedScanTraceCheckout::default(),
        };
        #[cfg(not(all(test, feature = "performance-test-tauri")))]
        let managed_trace_checkout = ManagedScanTraceCheckout::default();
        #[cfg(all(test, feature = "performance-test-tauri"))]
        if let Some(key) = issue366_diagnostic_key {
            let pragma_started = std::time::Instant::now();
            let configured_busy_timeout_ms = conn
                .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                .unwrap_or(-1);
            eprintln!(
                "[issue366-admission] request_key={key} stage=pool_connection_acquired elapsed_from_test_start_us={:?} busy_timeout_ms={configured_busy_timeout_ms} pragma_read_wall_us={}",
                issue366_elapsed_from_test_start_us(),
                pragma_started.elapsed().as_micros()
            );
            eprintln!(
                "[issue366-admission] request_key={key} stage=before_begin_immediate elapsed_from_test_start_us={:?} admission_method_wall_us={}",
                issue366_elapsed_from_test_start_us(),
                issue366_admission_started
                    .as_ref()
                    .expect("diagnostic timer exists when a key is selected")
                    .elapsed()
                    .as_micros()
            );
        }
        let tx = begin_managed_scan_write_transaction!(
            &mut conn,
            "admit_managed_scan",
            None,
            issue366_diagnostic_key,
            managed_trace_checkout
        )?;

        if let Some(request_key) = request_key.as_deref() {
            if let Some((session_id, existing_hash)) = tx
                .query_row(
                    "SELECT id, canonical_request_hash FROM scan_sessions WHERE request_key = ?1",
                    params![request_key],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .optional()?
            {
                if existing_hash.as_deref() != Some(canonical_hash.as_str()) {
                    return Err(DbError::Validation(format!(
                        "Scan request key already belongs to a different canonical request: {request_key}"
                    )));
                }
                let admission = load_admission_tx(&tx, &session_id, false)?;
                tx.commit()?;
                self.wake_automation_triggers();
                return Ok(admission);
            }
        }

        let mut roots_by_key = HashMap::new();
        for effective in effective_roots(&resolved) {
            let seed = ensure_scan_root_tx(&tx, &effective.path)?;
            if seed.active_run_id.is_some() {
                return Err(DbError::Validation(format!(
                    "Scan root already has an active run: {}",
                    effective.path
                )));
            }
            roots_by_key.insert(effective.key.clone(), seed);
        }

        let active_roots = {
            let mut statement = tx.prepare(
                "SELECT normalized_path, active_run_id FROM scan_roots WHERE active_run_id IS NOT NULL",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        for effective in effective_roots(&resolved) {
            if let Some((active_path, active_run_id)) = active_roots
                .iter()
                .find(|(active_path, _)| root_paths_overlap(active_path, &effective.path))
            {
                return Err(DbError::Validation(format!(
                    "Scan root overlaps an active root lease: {} overlaps {} ({active_run_id})",
                    effective.path, active_path
                )));
            }
        }

        let now = current_unix_seconds();
        let session_id = new_job_id("scan-session");
        tx.execute(
            r#"
            INSERT INTO scan_sessions (
                id, request_key, canonical_request_hash, status, phase,
                requested_root_count, effective_root_count, dedupe_requested,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, 'queued', 'preparing', ?4, ?5, ?6, ?7, ?7)
            "#,
            params![
                session_id,
                request_key,
                canonical_hash,
                resolved.len() as i64,
                roots_by_key.len() as i64,
                bool_to_i64(options.request.dedupe),
                now,
            ],
        )?;

        for mapping in &resolved {
            let root_seed = mapping
                .effective_key
                .as_ref()
                .and_then(|key| roots_by_key.get(key));
            tx.execute(
                r#"
                INSERT INTO scan_session_roots (
                    session_id, requested_index, requested_path, normalized_requested_path,
                    resolution, effective_root_id, effective_path, effective_index,
                    status, reason, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
                "#,
                params![
                    session_id,
                    mapping.requested_index,
                    mapping.requested_path,
                    mapping.normalized_path,
                    mapping.resolution,
                    root_seed.map(|seed| seed.id.as_str()),
                    mapping.effective_path,
                    mapping.effective_index,
                    mapping.status,
                    mapping.reason,
                    now,
                ],
            )?;
        }

        for effective in effective_roots(&resolved) {
            let root_seed = roots_by_key
                .get(&effective.key)
                .expect("effective root was inserted into the admission map");
            let generation = root_seed.current_generation + 1;
            let run_id = if effective.first_index == 0 {
                options
                    .run_id_override
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| new_job_id("scan-run"))
            } else {
                new_job_id("scan-run")
            };
            let lease_token = new_job_id("scan-lease");
            tx.execute(
                r#"
                INSERT INTO scan_runs (
                    id, scan_root_id, generation, parent_session_id, lease_token,
                    status, phase, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, 'queued', 'preparing', ?6, ?6)
                "#,
                params![
                    run_id,
                    root_seed.id,
                    generation,
                    session_id,
                    lease_token,
                    now,
                ],
            )?;
            let changed = tx.execute(
                r#"
                UPDATE scan_roots
                SET current_generation = ?1,
                    active_run_id = ?2,
                    active_generation = ?1,
                    health_status = 'scanning',
                    revision = revision + 1,
                    updated_at = ?3
                WHERE id = ?4
                  AND revision = ?5
                  AND active_run_id IS NULL
                  AND active_generation IS NULL
                "#,
                params![generation, run_id, now, root_seed.id, root_seed.revision],
            )?;
            if changed != 1 {
                return Err(DbError::Validation(format!(
                    "Scan root lease admission CAS failed: {}",
                    effective.path
                )));
            }
            tx.execute(
                r#"
                UPDATE scan_session_roots
                SET run_id = ?1, updated_at = ?2
                WHERE session_id = ?3 AND effective_root_id = ?4
                "#,
                params![run_id, now, session_id, root_seed.id],
            )?;
        }

        if roots_by_key.is_empty() {
            let _ = update_session_projection_tx(&tx, &session_id, 0, now)?;
        }

        let admission = load_admission_tx(&tx, &session_id, true)?;
        tx.commit()?;
        self.wake_automation_triggers();
        Ok(admission)
    }

    pub(crate) fn get_scan_run_record(&self, run_id: &str) -> Result<ScanRunRecord, DbError> {
        let conn = self.conn()?;
        load_scan_run_record(&conn, run_id)
    }

    pub(crate) fn get_scan_session(&self, session_id: &str) -> Result<ScanSessionDto, DbError> {
        let conn = self.conn()?;
        load_session(&conn, session_id)
    }

    pub(crate) fn get_managed_scan_snapshot(
        &self,
        session_id: &str,
    ) -> Result<ManagedScanSnapshotDto, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let session = load_session(&tx, session_id)?;
        let mut statement = tx.prepare(&format!(
            "{SCAN_RUN_SELECT} WHERE run.parent_session_id = ?1 ORDER BY run.created_at, run.id"
        ))?;
        let runs = statement
            .query_map(params![session_id], scan_run_dto_from_row)
            .map_err(DbError::from)?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        tx.commit()?;
        Ok(ManagedScanSnapshotDto { session, runs })
    }

    pub(crate) fn list_scan_runs(
        &self,
        session_id: Option<&str>,
        root_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ScanRunDto>, DbError> {
        let conn = self.conn()?;
        let limit = i64::try_from(limit.clamp(1, 500)).unwrap_or(500);
        let mut statement = conn.prepare(&format!(
            "{SCAN_RUN_SELECT} WHERE (?1 IS NULL OR run.parent_session_id = ?1) \
             AND (?2 IS NULL OR run.scan_root_id = ?2) \
             ORDER BY run.created_at DESC LIMIT ?3"
        ))?;
        let rows = statement
            .query_map(params![session_id, root_id, limit], scan_run_dto_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub(crate) fn list_scan_roots(&self) -> Result<Vec<ScanRootDto>, DbError> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            r#"
            SELECT id, normalized_path, display_name, source_kind, enabled, health_status,
                   current_generation, active_run_id, active_generation, revision,
                   last_successful_generation, last_full_scan_at, needs_reconciliation,
                   last_error_code, last_error_message, watcher_revision,
                   watcher_applied_revision, watcher_last_event_at, watcher_last_applied_at,
                   watcher_last_error_code, watcher_last_error_message,
                   watcher_rule_recovery_required, created_at, updated_at
            FROM scan_roots
            ORDER BY normalized_path
            "#,
        )?;
        let rows = statement
            .query_map([], scan_root_from_row)
            .map_err(DbError::from)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub(crate) fn next_watcher_reconciliation_admission(
        &self,
        root_id: &str,
        watcher_revision: i64,
        now: i64,
    ) -> Result<WatcherReconciliationAdmission, DbError> {
        let conn = self.conn()?;
        let base_key = format!("watcher-reconcile:{root_id}:{watcher_revision}");
        let attempt_prefix = format!("{base_key}:attempt:");
        let like_pattern = format!("{attempt_prefix}%");
        let mut statement = conn.prepare(
            "SELECT request_key, status, updated_at
             FROM scan_sessions
             WHERE request_key = ?1 OR request_key LIKE ?2
             ORDER BY created_at DESC, id DESC",
        )?;
        let mut latest: Option<(usize, String, i64)> = None;
        let mut max_attempt = None;
        for row in statement.query_map(params![base_key, like_pattern], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })? {
            let (request_key, status, updated_at) = row?;
            let attempt = if request_key == base_key {
                Some(0)
            } else {
                request_key
                    .strip_prefix(&attempt_prefix)
                    .and_then(|value| value.parse::<usize>().ok())
            };
            let Some(attempt) = attempt else {
                continue;
            };
            max_attempt = Some(max_attempt.map_or(attempt, |current: usize| current.max(attempt)));
            if matches!(status.as_str(), "queued" | "running" | "cancelling") {
                return Ok(WatcherReconciliationAdmission::Active);
            }
            if latest
                .as_ref()
                .is_none_or(|(latest_attempt, _, _)| attempt > *latest_attempt)
            {
                latest = Some((attempt, status, updated_at));
            }
        }

        let Some((latest_attempt, latest_status, updated_at)) = latest else {
            return Ok(WatcherReconciliationAdmission::Start {
                request_key: base_key,
                attempt: 0,
            });
        };
        let retryable = matches!(
            latest_status.as_str(),
            "failed"
                | "interrupted"
                | "requires_reconciliation"
                | "completed_with_warnings"
                | "completed"
        );
        if !retryable {
            return Ok(WatcherReconciliationAdmission::Exhausted {
                attempts: max_attempt.map_or(1, |attempt| attempt + 1),
            });
        }

        let attempts = max_attempt.map_or(1, |attempt| attempt + 1);
        if attempts >= WATCHER_RECONCILIATION_MAX_AUTOMATIC_ATTEMPTS {
            return Ok(WatcherReconciliationAdmission::Exhausted { attempts });
        }
        let delay_index = latest_attempt.min(WATCHER_RECONCILIATION_RETRY_DELAYS_SECONDS.len() - 1);
        let retry_at = updated_at + WATCHER_RECONCILIATION_RETRY_DELAYS_SECONDS[delay_index];
        if now < retry_at {
            return Ok(WatcherReconciliationAdmission::Backoff { retry_at });
        }

        let attempt = latest_attempt + 1;
        Ok(WatcherReconciliationAdmission::Start {
            request_key: format!("{base_key}:attempt:{attempt}"),
            attempt,
        })
    }

    pub(crate) fn get_scan_root_health(
        &self,
        root_id: Option<&str>,
        path: Option<&str>,
    ) -> Result<ScanRootDto, DbError> {
        let conn = self.conn()?;
        let normalized_path = path.map(normalize_scan_root_path);
        let path_clause = if cfg!(windows) {
            "lower(normalized_path) = lower(?2)"
        } else {
            "normalized_path = ?2"
        };
        conn.query_row(
            &format!(
                r#"
                SELECT id, normalized_path, display_name, source_kind, enabled, health_status,
                       current_generation, active_run_id, active_generation, revision,
                       last_successful_generation, last_full_scan_at, needs_reconciliation,
                       last_error_code, last_error_message, watcher_revision,
                       watcher_applied_revision, watcher_last_event_at, watcher_last_applied_at,
                       watcher_last_error_code, watcher_last_error_message,
                       watcher_rule_recovery_required, created_at, updated_at
                FROM scan_roots
                WHERE (?1 IS NOT NULL AND id = ?1)
                   OR (?2 IS NOT NULL AND {path_clause})
                ORDER BY id
                LIMIT 1
                "#
            ),
            params![root_id, normalized_path],
            scan_root_from_row,
        )
        .map_err(DbError::from)
    }

    pub fn sync_file_library_watcher_roots(
        &self,
        roots: &[crate::settings::ScanRootSetting],
    ) -> Result<(), DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let desired = roots
            .iter()
            .filter_map(|root| {
                let path = normalize_scan_root_path(&root.path);
                (!path.is_empty()).then_some((path, root.enabled, root.label.trim().to_string()))
            })
            .collect::<Vec<_>>();
        let desired_keys = desired
            .iter()
            .map(|(path, _, _)| root_identity_key(path))
            .collect::<HashSet<_>>();
        let mut previous_enabled = HashMap::<String, bool>::new();
        let mut root_cursor_transitions = HashMap::<String, Option<i64>>::new();

        {
            let mut statement = tx.prepare(
                "SELECT id, normalized_path, enabled FROM scan_roots WHERE source_kind = 'file_library'",
            )?;
            let existing = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)? != 0,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (id, path, enabled) in existing {
                let key = root_identity_key(&normalize_scan_root_path(&path));
                previous_enabled
                    .entry(key.clone())
                    .and_modify(|was_enabled| *was_enabled |= enabled)
                    .or_insert(enabled);
                if !desired_keys.contains(&key) {
                    if enabled {
                        root_cursor_transitions.insert(id.clone(), None);
                    }
                    tx.execute(
                        "UPDATE scan_roots SET enabled = 0, updated_at = ?1 WHERE id = ?2",
                        params![current_unix_seconds(), id],
                    )?;
                }
            }
        }

        for (path, enabled, label) in desired {
            let seed = ensure_scan_root_tx(&tx, &path)?;
            let key = root_identity_key(&path);
            let was_enabled = previous_enabled.get(&key).copied().unwrap_or(false);
            if enabled && !was_enabled {
                let revision = tx.query_row(
                    "SELECT library_change_revision FROM scan_roots WHERE id = ?1",
                    [&seed.id],
                    |row| row.get(0),
                )?;
                root_cursor_transitions.insert(seed.id.clone(), Some(revision));
            } else if !enabled && was_enabled {
                root_cursor_transitions.insert(seed.id.clone(), None);
            }
            let display_name = if label.is_empty() {
                scan_root_display_name(&path)
            } else {
                label
            };
            tx.execute(
                r#"
                UPDATE scan_roots
                SET display_name = ?1,
                    source_kind = 'file_library',
                    enabled = ?2,
                    health_status = CASE
                        WHEN ?2 = 1 AND health_status IN ('missing', 'permission_required')
                        THEN 'unknown'
                        ELSE health_status
                    END,
                    updated_at = ?3
                WHERE id = ?4
                "#,
                params![
                    display_name,
                    bool_to_i64(enabled),
                    current_unix_seconds(),
                    seed.id
                ],
            )?;
        }
        tx.execute(
            r#"
            UPDATE duplicate_groups
            SET status = 'stale',
                revision = revision + 1,
                updated_at = ?1
            WHERE status = 'active'
              AND id IN (
                  SELECT DISTINCT member.group_id
                  FROM duplicate_group_members AS member
                  JOIN files AS file_row ON file_row.id = member.file_id
                  JOIN scan_roots AS root ON root.source_kind = 'file_library'
                  WHERE root.enabled = 0
                    AND (
                        file_row.path = root.normalized_path
                        OR substr(file_row.path, 1, length(root.normalized_path) + 1) = root.normalized_path || '/'
                        OR substr(file_row.path, 1, length(root.normalized_path) + 1) = root.normalized_path || '\\'
                    )
              )
            "#,
            params![current_unix_seconds()],
        )?;
        let disabled_roots = tx
            .prepare(
                "SELECT normalized_path FROM scan_roots WHERE source_kind = 'file_library' AND enabled = 0",
            )?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for root_path in &disabled_roots {
            invalidate_analysis_findings_for_root_tx(&tx, root_path)?;
        }
        if !disabled_roots.is_empty() {
            bump_dedupe_authority_tx(&tx, "rebuild_required")?;
        }
        let root_cursor_transitions = root_cursor_transitions.into_iter().collect::<Vec<_>>();
        crate::db::apply_watcher_root_transitions(
            &tx,
            &root_cursor_transitions,
            current_unix_seconds(),
        )?;
        tx.commit()?;
        self.wake_automation_triggers();
        Ok(())
    }

    pub(crate) fn list_watcher_root_configs(&self) -> Result<Vec<WatcherRootConfig>, DbError> {
        let conn = self.conn()?;
        settings_owned_watcher_roots(&conn)
    }

    pub(crate) fn begin_watcher_revision(
        &self,
        root_id: &str,
    ) -> Result<Option<WatcherRevisionStart>, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = current_unix_seconds();
        let changed = tx.execute(
            r#"
            UPDATE scan_roots
            SET watcher_revision = watcher_revision + 1,
                watcher_last_event_at = ?1,
                watcher_last_error_code = NULL,
                watcher_last_error_message = NULL,
                revision = revision + 1,
                updated_at = ?1
            WHERE id = ?2 AND enabled = 1 AND source_kind = 'file_library'
            "#,
            params![now, root_id],
        )?;
        if changed == 0 {
            tx.commit()?;
            self.wake_automation_triggers();
            return Ok(None);
        }
        // A new watcher revision makes the previously published global
        // duplicate universe incomplete until the batch is applied.
        bump_dedupe_authority_tx(&tx, "rebuild_required")?;
        let result = tx.query_row(
            "SELECT id, normalized_path, watcher_revision, revision, active_run_id FROM scan_roots WHERE id = ?1",
            params![root_id],
            |row| {
                Ok(WatcherRevisionStart {
                    watcher_revision: row.get(2)?,
                })
            },
        )?;
        tx.commit()?;
        self.wake_automation_triggers();
        Ok(Some(result))
    }

    pub(crate) fn complete_watcher_revision(
        &self,
        root_id: &str,
        batch_revision: i64,
    ) -> Result<bool, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = current_unix_seconds();
        let changed = tx.execute(
            r#"
            UPDATE scan_roots
            SET watcher_applied_revision = ?1,
                watcher_last_applied_at = ?2,
                watcher_last_error_code = NULL,
                watcher_last_error_message = NULL,
                health_status = CASE
                    WHEN watcher_rule_recovery_required = 1 THEN 'reconciliation_required'
                    ELSE health_status
                END,
                needs_reconciliation = CASE
                    WHEN watcher_rule_recovery_required = 1 THEN 1
                    ELSE needs_reconciliation
                END,
                revision = revision + 1,
                updated_at = ?2
            WHERE id = ?3
              AND enabled = 1
              AND watcher_applied_revision < ?1
              AND watcher_revision >= ?1
            "#,
            params![batch_revision, now, root_id],
        )?;
        if changed > 0 {
            bump_dedupe_authority_tx(&tx, "rebuild_required")?;
        }
        tx.commit()?;
        self.wake_automation_triggers();
        Ok(changed == 1)
    }

    pub(crate) fn mark_watcher_reconciliation(
        &self,
        root_id: &str,
        error_code: &str,
        error_message: &str,
    ) -> Result<bool, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = current_unix_seconds();
        let changed = tx.execute(
            r#"
            UPDATE scan_roots
            SET needs_reconciliation = 1,
                health_status = CASE
                    WHEN health_status IN ('missing', 'permission_required') THEN health_status
                    ELSE 'reconciliation_required'
                END,
                watcher_rule_recovery_required = CASE
                    WHEN ?1 IN ('watcher_rule_failure', 'watcher_rule_retry_exhausted') THEN 1
                    ELSE watcher_rule_recovery_required
                END,
                watcher_last_error_code = ?1,
                watcher_last_error_message = ?2,
                revision = revision + 1,
                updated_at = ?3
            WHERE id = ?4 AND source_kind = 'file_library'
            "#,
            params![error_code, error_message, now, root_id],
        )?;
        if changed > 0 {
            bump_dedupe_authority_tx(&tx, "rebuild_required")?;
        }
        tx.commit()?;
        self.wake_automation_triggers();
        Ok(changed == 1)
    }

    pub(crate) fn record_watcher_warning(
        &self,
        root_id: &str,
        error_code: &str,
        error_message: &str,
    ) -> Result<bool, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            r#"
            UPDATE scan_roots
            SET watcher_last_error_code = ?1,
                watcher_last_error_message = ?2,
                watcher_rule_recovery_required = CASE
                    WHEN ?1 IN ('watcher_rule_failure', 'watcher_rule_retry_exhausted') THEN 1
                    ELSE watcher_rule_recovery_required
                END,
                health_status = CASE
                    WHEN health_status IN ('missing', 'permission_required', 'reconciliation_required')
                    THEN health_status
                    ELSE 'degraded'
                END,
                revision = revision + 1,
                updated_at = ?3
            WHERE id = ?4 AND source_kind = 'file_library'
            "#,
            params![error_code, error_message, current_unix_seconds(), root_id],
        )?;
        if changed > 0 {
            bump_dedupe_authority_tx(&tx, "rebuild_required")?;
        }
        tx.commit()?;
        self.wake_automation_triggers();
        Ok(changed == 1)
    }

    pub(crate) fn mark_watcher_root_missing(
        &self,
        root_id: &str,
        error_code: &str,
        error_message: &str,
    ) -> Result<bool, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            r#"
            UPDATE scan_roots
            SET health_status = ?1,
                needs_reconciliation = 1,
                watcher_last_error_code = ?2,
                watcher_last_error_message = ?3,
                revision = revision + 1,
                updated_at = ?4
            WHERE id = ?5 AND source_kind = 'file_library'
              AND (
                  health_status IS NOT ?1
                  OR needs_reconciliation != 1
                  OR watcher_last_error_code IS NOT ?2
                  OR watcher_last_error_message IS NOT ?3
              )
            "#,
            params![
                error_code,
                error_code,
                error_message,
                current_unix_seconds(),
                root_id
            ],
        )?;
        if changed > 0 {
            bump_dedupe_authority_tx(&tx, "rebuild_required")?;
        }
        tx.commit()?;
        self.wake_automation_triggers();
        Ok(changed == 1)
    }

    pub(crate) fn apply_watcher_exact_mutations(
        &self,
        root_id: &str,
        expected_watcher_revision: i64,
        paths: &[String],
        directory_paths: &HashSet<String>,
    ) -> Result<WatcherMutationResult, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (root_path, actual_watcher_revision): (String, i64) = tx.query_row(
            "SELECT normalized_path, watcher_revision FROM scan_roots WHERE id = ?1 AND enabled = 1 AND source_kind = 'file_library'",
            params![root_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if actual_watcher_revision != expected_watcher_revision {
            return Err(DbError::Validation(
                "Watcher revision is stale; source claims were not applied.".to_string(),
            ));
        }
        let observed_at = current_unix_seconds();
        let mut files = Vec::new();
        let mut stale_paths = Vec::new();
        let mut upserted_paths = Vec::new();
        let mut reconciliation_required = false;
        let mut warning = None;
        let path_filter = ManagedPathFilter::new(Path::new(&root_path));

        for raw_path in paths
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            let normalized = normalize_scan_root_path(raw_path);
            if normalized.is_empty() || !watcher_path_within_root(&root_path, &normalized) {
                reconciliation_required = true;
                warning = Some("Watcher path did not map to exactly one managed root.".to_string());
                continue;
            }
            let path = PathBuf::from(&normalized);
            if path_filter.excludes(&path, path.is_dir()) {
                continue;
            }
            if !path_filter.direct_path_has_no_links(&path)? {
                reconciliation_required = true;
                warning =
                    Some("Linked ancestor in watcher path requires reconciliation.".to_string());
                continue;
            }
            if directory_paths.contains(&normalized) {
                reconciliation_required = true;
            }
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata_is_link_or_reparse(&metadata) => {
                    reconciliation_required = true;
                    warning = Some(
                        "Symlink or reparse-point watcher event requires reconciliation."
                            .to_string(),
                    );
                }
                Ok(metadata) => {
                    if metadata.is_dir() {
                        reconciliation_required = true;
                    }
                    files.push(insert_request_from_metadata(
                        normalized.clone(),
                        &path,
                        &metadata,
                    ));
                    if !metadata.is_dir() {
                        upserted_paths.push(normalized);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    stale_paths.push(normalized);
                }
                Err(error) => return Err(DbError::Io(error)),
            }
        }

        let root_epoch = if !files.is_empty() || !stale_paths.is_empty() {
            advance_root_epoch_tx(
                &tx,
                root_id,
                "watcher",
                expected_watcher_revision,
                observed_at,
            )?
        } else {
            0
        };
        let mut filesystem_changed = upsert_file_rows_tx(
            &tx,
            &files,
            observed_at,
            root_id,
            root_epoch,
            "watcher",
            expected_watcher_revision,
        )?;
        for path in &stale_paths {
            if !directory_paths.contains(path) {
                let absent_claims = mark_root_path_absent_tx(
                    &tx,
                    root_id,
                    path,
                    root_epoch,
                    "watcher",
                    expected_watcher_revision,
                    observed_at,
                )?;
                filesystem_changed |= absent_claims > 0;
            }
            for candidate in path_lookup_candidates(path, path) {
                filesystem_changed |= tx.execute(
                    r#"
                    UPDATE files AS file_row SET is_stale = 1
                    WHERE file_row.is_stale = 0
                      AND (file_row.id = ?1 OR file_row.path = ?1)
                      AND NOT EXISTS (
                          SELECT 1 FROM library_source_root_claims AS claim
                          WHERE claim.observed_path = file_row.path
                            AND claim.root_id != ?2
                            AND claim.claim_state = 'present'
                      )
                    "#,
                    params![candidate, root_id],
                )? > 0;
            }
        }
        invalidate_stale_files_in_transaction(&tx)?;
        if !files.is_empty() || !stale_paths.is_empty() {
            super::library::bump_library_query_revision_in_transaction(&tx)?;
        }
        if filesystem_changed {
            bump_root_change_revision(&tx, root_id)?;
        }
        tx.commit()?;
        if filesystem_changed {
            self.wake_automation_triggers();
        }
        Ok(WatcherMutationResult {
            upserted_paths,
            reconciliation_required,
            warning,
        })
    }

    pub(crate) fn claim_queued_scan_run(&self, run_id: &str) -> Result<ScanRunRecord, DbError> {
        let (mut conn, trace_checkout) =
            checkout_managed_scan_write_connection!(self, Some(run_id))?;
        let tx = begin_managed_scan_write_transaction!(
            &mut conn,
            "claim_queued_scan_run",
            Some(run_id),
            None::<&str>,
            trace_checkout
        )?;
        let record = load_scan_run_record(&tx, run_id)?;
        if record.dto.status != "queued" {
            return Ok(record);
        }
        if record.root_active_run_id.as_deref() != Some(run_id)
            || record.root_active_generation != Some(record.dto.generation)
        {
            return Err(DbError::Validation(
                "Queued scan run no longer owns its root lease.".to_string(),
            ));
        }
        if record.session_status == "cancelling" || record.dto.cancel_requested {
            return Err(DbError::Validation(
                "Scan run was cancelled before start.".to_string(),
            ));
        }

        let now = current_unix_seconds();
        let changed = tx.execute(
            r#"
            UPDATE scan_runs
            SET status = 'running', phase = 'discovering',
                started_at = COALESCE(started_at, ?1),
                last_checkpoint_at = ?1,
                watcher_revision_at_start = ?2,
                revision = revision + 1, updated_at = ?1
            WHERE id = ?3 AND status = 'queued' AND revision = ?4
              AND lease_token = ?5 AND cancel_requested = 0
            "#,
            params![
                now,
                record.current_watcher_revision,
                run_id,
                record.dto.revision,
                record.lease_token
            ],
        )?;
        if changed != 1 {
            return Err(DbError::Validation(
                "Queued scan run claim CAS failed.".to_string(),
            ));
        }
        let root_changed = tx.execute(
            r#"
            UPDATE scan_roots
            SET health_status = 'scanning', revision = revision + 1, updated_at = ?1
            WHERE id = ?2 AND revision = ?3
              AND active_run_id = ?4 AND active_generation = ?5
            "#,
            params![
                now,
                record.dto.scan_root_id,
                record.root_revision,
                run_id,
                record.dto.generation
            ],
        )?;
        if root_changed != 1 {
            return Err(DbError::Validation(
                "Root lease claim CAS failed.".to_string(),
            ));
        }
        let session_id =
            record.dto.parent_session_id.as_deref().ok_or_else(|| {
                DbError::Validation("Scan run has no parent session.".to_string())
            })?;
        let session_changed = tx.execute(
            r#"
            UPDATE scan_sessions
            SET status = 'running',
                phase = CASE WHEN phase IN ('finalizing', 'completed') THEN phase ELSE 'running' END,
                started_at = COALESCE(started_at, ?1),
                last_checkpoint_at = ?1, revision = revision + 1, updated_at = ?1
            WHERE id = ?2 AND revision = ?3
              AND cancel_requested = 0
              AND status IN ('queued', 'running')
            "#,
            params![now, session_id, record.session_revision],
        )?;
        if session_changed != 1 {
            return Err(DbError::Validation(
                "Scan session claim CAS failed.".to_string(),
            ));
        }
        tx.execute(
            "UPDATE scan_session_roots SET status = 'running', updated_at = ?1 WHERE run_id = ?2 AND status = 'queued'",
            params![now, run_id],
        )?;

        let claimed = load_scan_run_record(&tx, run_id)?;
        tx.commit()?;
        self.wake_automation_triggers();
        Ok(claimed)
    }

    pub(crate) fn persist_scan_batch(
        &self,
        run_id: &str,
        expected_run_revision: i64,
        expected_root_revision: i64,
        expected_session_revision: i64,
        batch: &ScanBatchInput<'_>,
    ) -> Result<ScanRunRecord, DbError> {
        let (mut conn, trace_checkout) =
            checkout_managed_scan_write_connection!(self, Some(run_id))?;
        #[cfg(all(test, feature = "performance-test-tauri"))]
        invoke_managed_scan_write_test_hook(run_id, ManagedScanWriteTestPoint::BeforeTransaction);
        let tx = begin_managed_scan_write_transaction!(
            &mut conn,
            "persist_scan_batch",
            Some(run_id),
            None::<&str>,
            trace_checkout
        )?;
        #[cfg(all(test, feature = "performance-test-tauri"))]
        invoke_managed_scan_write_test_hook(run_id, ManagedScanWriteTestPoint::TransactionAcquired);
        let record = load_scan_run_record(&tx, run_id)?;
        let ownership_result = validate_worker_ownership(
            &record,
            run_id,
            expected_run_revision,
            expected_root_revision,
            expected_session_revision,
            true,
        );
        #[cfg(all(test, feature = "performance-test-tauri"))]
        tx.record_revision_ownership(
            "worker_batch",
            &format!(
                "expected_run_revision={expected_run_revision} actual_run_revision={} expected_root_revision={expected_root_revision} actual_root_revision={} expected_session_revision={expected_session_revision} actual_session_revision={} ownership_valid={}",
                record.dto.revision,
                record.root_revision,
                record.session_revision,
                ownership_result.is_ok()
            ),
        );
        ownership_result?;

        let observed_at = current_unix_seconds();
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let source_identity_trace_scope = tx.source_identity_trace_scope();
        let filesystem_changed = upsert_scan_files_tx(
            &tx,
            run_id,
            batch.entries,
            observed_at,
            &record.dto.scan_root_id,
            expected_root_revision,
        )?;
        #[cfg(all(test, feature = "performance-test-tauri"))]
        drop(source_identity_trace_scope);
        insert_scan_errors_tx(&tx, run_id, batch.errors, observed_at)?;

        let metadata_errors = batch
            .errors
            .iter()
            .filter(|error| error.metadata_error)
            .count() as i64;
        let coverage_errors = batch
            .errors
            .iter()
            .filter(|error| error.affects_coverage)
            .count() as i64;
        let errors = batch.errors.len() as i64;
        let now = current_unix_seconds();
        let run_changed = tx.execute(
            r#"
            UPDATE scan_runs
            SET phase = 'persisting',
                scanned_files = scanned_files + ?1,
                scanned_directories = scanned_directories + ?2,
                processed_bytes = processed_bytes + ?3,
                warnings_count = warnings_count + ?4,
                errors_count = errors_count + ?5,
                metadata_error_count = metadata_error_count + ?6,
                coverage_error_count = coverage_error_count + ?7,
                coverage_complete = 0,
                stale_reconciliation_allowed = 0,
                last_checkpoint_at = ?8,
                revision = revision + 1,
                updated_at = ?8
            WHERE id = ?9 AND status = 'running' AND cancel_requested = 0
              AND revision = ?10 AND lease_token = ?11
            "#,
            params![
                batch.scanned_files,
                batch.scanned_directories,
                batch.processed_bytes,
                batch.warnings,
                errors,
                metadata_errors,
                coverage_errors,
                now,
                run_id,
                expected_run_revision,
                record.lease_token,
            ],
        )?;
        if run_changed != 1 {
            return Err(DbError::Validation(
                "Scan batch run revision CAS failed.".to_string(),
            ));
        }

        let session_id =
            record.dto.parent_session_id.as_deref().ok_or_else(|| {
                DbError::Validation("Scan run has no parent session.".to_string())
            })?;
        let session_changed = tx.execute(
            r#"
            UPDATE scan_sessions
            SET status = 'running',
                phase = CASE WHEN phase IN ('finalizing', 'completed') THEN phase ELSE 'running' END,
                scanned_files = scanned_files + ?1,
                scanned_directories = scanned_directories + ?2,
                warnings_count = warnings_count + ?3,
                errors_count = errors_count + ?4,
                started_at = COALESCE(started_at, ?5),
                last_checkpoint_at = ?5,
                revision = revision + 1,
                updated_at = ?5
            WHERE id = ?6 AND revision = ?7
              AND cancel_requested = 0
              AND status IN ('queued', 'running')
            "#,
            params![
                batch.scanned_files,
                batch.scanned_directories,
                batch.warnings,
                errors,
                now,
                session_id,
                expected_session_revision,
            ],
        )?;
        if session_changed != 1 {
            return Err(DbError::Validation(
                "Scan batch session revision CAS failed.".to_string(),
            ));
        }

        if !batch.entries.is_empty() {
            super::library::bump_library_query_revision_in_transaction(&tx)?;
        }

        if filesystem_changed {
            bump_root_change_revision(&tx, &record.dto.scan_root_id)?;
        }
        let updated = load_scan_run_record(&tx, run_id)?;
        tx.commit()?;
        #[cfg(all(test, feature = "performance-test-tauri"))]
        invoke_managed_scan_write_test_hook(run_id, ManagedScanWriteTestPoint::AfterCommit);
        if filesystem_changed {
            self.wake_automation_triggers();
        }
        Ok(updated)
    }

    pub(crate) fn reconcile_missing(
        &self,
        run_id: &str,
        expected_run_revision: i64,
        expected_root_revision: i64,
        expected_session_revision: i64,
    ) -> Result<ScanRunRecord, DbError> {
        let (mut conn, trace_checkout) =
            checkout_managed_scan_write_connection!(self, Some(run_id))?;
        let tx = begin_managed_scan_write_transaction!(
            &mut conn,
            "reconcile_missing",
            Some(run_id),
            None::<&str>,
            trace_checkout
        )?;
        let record = load_scan_run_record(&tx, run_id)?;
        validate_worker_ownership(
            &record,
            run_id,
            expected_run_revision,
            expected_root_revision,
            expected_session_revision,
            true,
        )?;
        if record.dto.coverage_error_count > 0 {
            return Err(DbError::Validation(
                "Scan coverage is incomplete; stale reconciliation is forbidden.".to_string(),
            ));
        }
        let started_at = record
            .dto
            .started_at
            .ok_or_else(|| DbError::Validation("Scan run has no start timestamp.".to_string()))?;
        let now = current_unix_seconds();
        let watcher_stable =
            record.current_watcher_revision == record.dto.watcher_revision_at_start;
        let mut root_membership_changed = false;
        let changed = if watcher_stable {
            let root = &record.dto.root_path;
            let (root_slash, root_slash_descendant, root_backslash, root_backslash_descendant) =
                root_patterns(root);
            let ignored_patterns = ignored_subtree_like_patterns(root);
            let ignored_clauses = ignored_patterns
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    let parameter_index = 8 + index;
                    format!("LOWER(f.path) NOT LIKE LOWER(?{parameter_index}) ESCAPE '~'")
                })
                .collect::<Vec<_>>()
                .join(" AND ");
            let mut values = vec![
                SqlValue::Integer(started_at),
                SqlValue::Text(root_slash),
                SqlValue::Text(root_slash_descendant),
                SqlValue::Text(root_backslash),
                SqlValue::Text(root_backslash_descendant),
                SqlValue::Text(run_id.to_string()),
                SqlValue::Text(record.dto.scan_root_id.clone()),
            ];
            values.extend(ignored_patterns.into_iter().map(SqlValue::Text));
            let root_epoch = advance_root_epoch_tx(
                &tx,
                &record.dto.scan_root_id,
                "scan",
                record.dto.generation,
                now,
            )?;
            let claims_changed = reconcile_root_membership_tx(
                &tx,
                &record.dto.scan_root_id,
                run_id,
                root_epoch,
                record.dto.generation,
                now,
            )?;
            root_membership_changed = claims_changed > 0;
            let stale_changed = tx.execute(
                &format!(
                    r#"
                    UPDATE files AS f
                    SET is_stale = 1
                    WHERE f.is_stale = 0
                      AND f.last_seen_at < ?1
                      AND (
                          f.path = ?2 OR f.path LIKE ?3 ESCAPE '~'
                          OR f.path = ?4 OR f.path LIKE ?5 ESCAPE '~'
                      )
                      AND NOT EXISTS (
                          SELECT 1 FROM scan_seen AS seen
                          WHERE seen.run_id = ?6
                            AND (seen.file_id = f.id OR seen.observed_path = f.path)
                      )
                      AND NOT EXISTS (
                          SELECT 1 FROM library_source_root_claims AS claim
                          WHERE claim.observed_path = f.path
                            AND claim.root_id != ?7
                            AND claim.claim_state = 'present'
                      )
                      AND {ignored_clauses}
                    "#
                ),
                params_from_iter(values),
            )?;
            stale_changed
        } else {
            0
        };
        if changed > 0 || root_membership_changed {
            bump_root_change_revision(&tx, &record.dto.scan_root_id)?;
            if changed > 0 {
                invalidate_stale_files_in_transaction(&tx)?;
            }
            super::library::bump_library_query_revision_in_transaction(&tx)?;
        }
        let run_changed = tx.execute(
            r#"
            UPDATE scan_runs
            SET phase = 'reconciling_missing', coverage_complete = 1,
                stale_reconciliation_allowed = ?2, last_checkpoint_at = ?1,
                revision = revision + 1, updated_at = ?1
            WHERE id = ?3 AND status = 'running' AND cancel_requested = 0
              AND coverage_error_count = 0 AND revision = ?4
              AND lease_token = ?5
            "#,
            params![
                now,
                bool_to_i64(watcher_stable),
                run_id,
                expected_run_revision,
                record.lease_token
            ],
        )?;
        if run_changed != 1 {
            return Err(DbError::Validation(
                "Stale reconciliation run CAS failed; stale update rolled back.".to_string(),
            ));
        }
        if !watcher_stable {
            tx.execute(
                r#"
                UPDATE scan_roots
                SET needs_reconciliation = 1,
                    health_status = CASE
                        WHEN health_status IN ('missing', 'permission_required') THEN health_status
                        ELSE 'reconciliation_required'
                    END,
                    last_error_code = 'watcher_changed_during_scan',
                    last_error_message = 'Filesystem changes arrived while the scan was running; missing reconciliation was skipped.',
                    revision = revision + 1,
                    updated_at = ?1
                WHERE id = ?2 AND active_run_id = ?3 AND active_generation = ?4
                "#,
                params![now, record.dto.scan_root_id, run_id, record.dto.generation],
            )?;
        }
        let updated = load_scan_run_record(&tx, run_id)?;
        let _ = changed;
        tx.commit()?;
        self.wake_automation_triggers();
        Ok(updated)
    }

    pub(crate) fn transition_scan_run_phase(
        &self,
        run_id: &str,
        expected_run_revision: i64,
        expected_root_revision: i64,
        phase: &str,
    ) -> Result<ScanRunRecord, DbError> {
        if !matches!(
            phase,
            "preparing"
                | "discovering"
                | "persisting"
                | "reconciling_missing"
                | "optimizing_search"
                | "finalizing"
                | "completed"
        ) {
            return Err(DbError::Validation(format!(
                "Invalid scan run phase: {phase}"
            )));
        }
        let (mut conn, trace_checkout) =
            checkout_managed_scan_write_connection!(self, Some(run_id))?;
        let tx = begin_managed_scan_write_transaction!(
            &mut conn,
            "transition_scan_run_phase",
            Some(run_id),
            None::<&str>,
            trace_checkout
        )?;
        let record = load_scan_run_record(&tx, run_id)?;
        if record.dto.revision != expected_run_revision
            || !root_revision_owned_after_watcher_change(&record, expected_root_revision)
            || record.root_active_run_id.as_deref() != Some(run_id)
            || record.root_active_generation != Some(record.dto.generation)
        {
            return Err(DbError::Validation(
                "Scan phase transition lost root lease or revision ownership.".to_string(),
            ));
        }
        let now = current_unix_seconds();
        let changed = tx.execute(
            r#"
            UPDATE scan_runs
            SET phase = ?1, last_checkpoint_at = ?2, revision = revision + 1, updated_at = ?2
            WHERE id = ?3 AND status = 'running' AND cancel_requested = 0
              AND revision = ?4 AND lease_token = ?5
            "#,
            params![
                phase,
                now,
                run_id,
                expected_run_revision,
                record.lease_token
            ],
        )?;
        if changed != 1 {
            return Err(DbError::Validation(
                "Scan phase transition CAS failed.".to_string(),
            ));
        }

        if phase == "finalizing" {
            let session_id = record.dto.parent_session_id.as_deref().ok_or_else(|| {
                DbError::Validation("Scan run has no parent session.".to_string())
            })?;
            let other_mappings_terminal = {
                let mut statement = tx.prepare(
                    "SELECT status FROM scan_session_roots WHERE session_id = ?1 AND (run_id IS NULL OR run_id != ?2)",
                )?;
                let statuses = statement
                    .query_map(params![session_id, run_id], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                statuses.iter().all(|status| is_mapping_terminal(status))
            };
            if other_mappings_terminal {
                let session = load_session(&tx, session_id)?;
                if session.revision != record.session_revision
                    || session.status != "running"
                    || session.cancel_requested
                {
                    return Err(DbError::Validation(
                        "Scan session finalizing transition lost durable revision ownership."
                            .to_string(),
                    ));
                }
                if session.phase == "running" {
                    let session_changed = tx.execute(
                        r#"
                        UPDATE scan_sessions
                        SET phase = 'finalizing', last_checkpoint_at = ?1,
                            revision = revision + 1, updated_at = ?1
                        WHERE id = ?2 AND revision = ?3
                          AND status = 'running' AND cancel_requested = 0
                          AND phase = 'running'
                        "#,
                        params![now, session_id, record.session_revision],
                    )?;
                    if session_changed != 1 {
                        return Err(DbError::Validation(
                            "Scan session finalizing transition CAS failed.".to_string(),
                        ));
                    }
                } else if session.phase != "finalizing" {
                    return Err(DbError::Validation(
                        "Scan session phase cannot advance to finalizing from its durable state."
                            .to_string(),
                    ));
                }
            }
        }
        let updated = load_scan_run_record(&tx, run_id)?;
        tx.commit()?;
        Ok(updated)
    }

    pub(crate) fn finalize_scan_run(
        &self,
        run_id: &str,
        expected_run_revision: i64,
        expected_root_revision: i64,
        expected_session_revision: i64,
        input: &ScanFinalizeInput,
    ) -> Result<ScanFinalization, DbError> {
        if !matches!(
            input.terminal_status.as_str(),
            "cancelled"
                | "completed"
                | "completed_with_warnings"
                | "failed"
                | "interrupted"
                | "requires_reconciliation"
        ) {
            return Err(DbError::Validation(format!(
                "Invalid scan terminal status: {}",
                input.terminal_status
            )));
        }
        let success = matches!(
            input.terminal_status.as_str(),
            "completed" | "completed_with_warnings"
        );
        let (mut conn, trace_checkout) =
            checkout_managed_scan_write_connection!(self, Some(run_id))?;
        let tx = begin_managed_scan_write_transaction!(
            &mut conn,
            "finalize_scan_run",
            Some(run_id),
            None::<&str>,
            trace_checkout
        )?;
        let record = load_scan_run_record(&tx, run_id)?;
        #[cfg(all(test, feature = "performance-test-tauri"))]
        tx.record_revision_ownership(
            "finalize_snapshot",
            &format!(
                "expected_run_revision={expected_run_revision} actual_run_revision={} expected_root_revision={expected_root_revision} actual_root_revision={} expected_session_revision={expected_session_revision} actual_session_revision={} root_active_generation={:?} run_generation={} session_id_available={}",
                record.dto.revision,
                record.root_revision,
                record.session_revision,
                record.root_active_generation,
                record.dto.generation,
                record.dto.parent_session_id.is_some()
            ),
        );
        let (
            root_watcher_error_code,
            root_watcher_error_message,
            root_rule_recovery_required,
        ): (Option<String>, Option<String>, bool) =
            tx.query_row(
                "SELECT watcher_last_error_code, watcher_last_error_message, watcher_rule_recovery_required FROM scan_roots WHERE id = ?1",
                params![record.dto.scan_root_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? != 0)),
            )?;
        if record.dto.status == input.terminal_status {
            let repeated_ownership_matches = record.dto.revision == expected_run_revision
                && record.root_revision == expected_root_revision
                && record.session_revision == expected_session_revision;
            #[cfg(all(test, feature = "performance-test-tauri"))]
            tx.record_revision_ownership(
                "finalize_idempotent_check",
                &format!("ownership_valid={repeated_ownership_matches}"),
            );
            if !repeated_ownership_matches {
                return Err(DbError::Validation(
                    "Repeated scan finalization lost durable revision ownership.".to_string(),
                ));
            }
            let session_id = record.dto.parent_session_id.as_deref().ok_or_else(|| {
                DbError::Validation("Scan run has no parent session.".to_string())
            })?;
            let session = load_session(&tx, session_id)?;
            tx.commit()?;
            self.wake_automation_triggers();
            return Ok(ScanFinalization {
                run: record,
                dedupe_pending: session.dedupe_dispatch_state == "pending",
                session,
            });
        }
        if !matches!(record.dto.status.as_str(), "running" | "cancelling") {
            return Err(DbError::Validation(
                "Terminal scan finalization can only claim a running run.".to_string(),
            ));
        }
        if success && record.dto.coverage_error_count > 0 {
            return Err(DbError::Validation(
                "A coverage-breaking scan cannot finalize successfully.".to_string(),
            ));
        }
        if success && record.dto.cancel_requested {
            return Err(DbError::Validation(
                "A scan with a durable cancellation request cannot finalize successfully."
                    .to_string(),
            ));
        }
        let revision_ownership_matches = record.dto.revision == expected_run_revision
            && root_revision_owned_after_watcher_change(&record, expected_root_revision)
            && record.session_revision == expected_session_revision
            && record.root_active_run_id.as_deref() == Some(run_id)
            && record.root_active_generation == Some(record.dto.generation);
        #[cfg(all(test, feature = "performance-test-tauri"))]
        tx.record_revision_ownership(
            "finalize_claim_check",
            &format!("ownership_valid={revision_ownership_matches}"),
        );
        if !revision_ownership_matches {
            return Err(DbError::Validation(
                "Scan finalization lost run, root lease, generation, or session revision."
                    .to_string(),
            ));
        }
        let watcher_changed_during_scan =
            success && record.current_watcher_revision != record.dto.watcher_revision_at_start;
        if success
            && input.allow_stale_reconciliation
            && !record.dto.stale_reconciliation_allowed
            && !watcher_changed_during_scan
        {
            return Err(DbError::Validation(
                "Successful finalization requires the stale reconciliation gate.".to_string(),
            ));
        }

        let rule_recovery_succeeded = input.rule_recovery_succeeded && !watcher_changed_during_scan;
        let rule_failure_pending = !rule_recovery_succeeded
            && (matches!(
                root_watcher_error_code.as_deref(),
                Some("watcher_rule_failure" | "watcher_rule_retry_exhausted")
            ) || root_rule_recovery_required
                || input.error_code.as_deref() == Some("watcher_rule_failure"));
        let effective_terminal_status = if watcher_changed_during_scan || rule_failure_pending {
            "completed_with_warnings"
        } else {
            input.terminal_status.as_str()
        };
        let durable_success = success && !watcher_changed_during_scan && !rule_failure_pending;
        let final_error_code = if watcher_changed_during_scan {
            Some("watcher_changed_during_scan")
        } else if rule_failure_pending {
            Some("watcher_rule_failure")
        } else {
            input.error_code.as_deref()
        };
        let final_error_message = if watcher_changed_during_scan {
            Some("Filesystem changes arrived while the scan was running; a follow-up reconciliation is required.")
        } else if rule_failure_pending {
            input
                .error_message
                .as_deref()
                .or(root_watcher_error_message.as_deref())
                .or(Some(
                    "Watcher rule execution failed; retry or manual recovery is required.",
                ))
        } else {
            input.error_message.as_deref()
        };

        let now = current_unix_seconds();
        let result_json = serde_json::json!({
            "generation": record.dto.generation,
            "coverageComplete": record.dto.coverage_error_count == 0,
            "staleReconciliation": record.dto.stale_reconciliation_allowed && durable_success,
            "watcherRevisionAtStart": record.dto.watcher_revision_at_start,
            "watcherRevisionAtFinalize": record.current_watcher_revision,
            "watcherChangedDuringScan": watcher_changed_during_scan,
            "ruleFailurePending": rule_failure_pending,
        })
        .to_string();
        let changed = tx.execute(
            r#"
            UPDATE scan_runs
            SET status = ?1, phase = 'completed',
                coverage_complete = CASE WHEN ?2 = 1 AND coverage_error_count = 0 THEN 1 ELSE coverage_complete END,
                stale_reconciliation_allowed = CASE WHEN ?3 = 1 THEN stale_reconciliation_allowed ELSE 0 END,
                finished_at = ?4, last_checkpoint_at = ?4,
                error_code = COALESCE(?5, error_code),
                error_message = COALESCE(?6, error_message),
                result_json = ?7,
                revision = revision + 1, updated_at = ?4
            WHERE id = ?8 AND status IN ('running', 'cancelling')
              AND revision = ?9 AND lease_token = ?10
            "#,
            params![
                effective_terminal_status,
                bool_to_i64(success),
                bool_to_i64(input.allow_stale_reconciliation && durable_success),
                now,
                final_error_code,
                final_error_message,
                result_json,
                run_id,
                expected_run_revision,
                record.lease_token,
            ],
        )?;
        if changed != 1 {
            return Err(DbError::Validation(
                "Scan terminal run CAS failed.".to_string(),
            ));
        }

        let health = if watcher_changed_during_scan || rule_failure_pending {
            "reconciliation_required"
        } else if success {
            if effective_terminal_status == "completed_with_warnings" {
                "degraded"
            } else {
                "healthy"
            }
        } else if matches!(input.error_code.as_deref(), Some("root_missing")) {
            "missing"
        } else if matches!(
            input.error_code.as_deref(),
            Some("permission_denied" | "root_permission")
        ) {
            "permission_required"
        } else {
            "reconciliation_required"
        };
        let last_successful_generation = if durable_success {
            Some(record.dto.generation)
        } else {
            None
        };
        let last_full_scan_at = if durable_success { Some(now) } else { None };
        let root_changed = tx.execute(
            r#"
            UPDATE scan_roots
            SET active_run_id = NULL,
                active_generation = NULL,
                health_status = ?1,
                watcher_applied_revision = CASE
                    WHEN ?2 = 1 AND watcher_applied_revision < ?12 THEN ?12
                    ELSE watcher_applied_revision
                END,
                watcher_last_applied_at = CASE
                    WHEN ?2 = 1 AND watcher_revision >= ?12 THEN ?7
                    ELSE watcher_last_applied_at
                END,
                watcher_last_error_code = CASE
                    WHEN ?14 = 1 THEN NULL
                    WHEN ?13 = 1 THEN COALESCE(?5, watcher_last_error_code)
                    WHEN ?2 = 1 AND watcher_revision >= ?12 THEN NULL
                    ELSE watcher_last_error_code
                END,
                watcher_last_error_message = CASE
                    WHEN ?14 = 1 THEN NULL
                    WHEN ?13 = 1 THEN COALESCE(?6, watcher_last_error_message)
                    WHEN ?2 = 1 AND watcher_revision >= ?12 THEN NULL
                    ELSE watcher_last_error_message
                END,
                watcher_rule_recovery_required = CASE
                    WHEN ?14 = 1 THEN 0
                    WHEN ?15 = 1 THEN 1
                    ELSE watcher_rule_recovery_required
                END,
                last_successful_generation = CASE WHEN ?2 = 1 THEN ?3 ELSE last_successful_generation END,
                last_full_scan_at = CASE WHEN ?2 = 1 THEN ?4 ELSE last_full_scan_at END,
                needs_reconciliation = CASE
                    WHEN ?2 = 1 AND ?13 = 0 THEN 0
                    ELSE 1
                END,
                last_error_code = ?5,
                last_error_message = ?6,
                revision = revision + 1,
                updated_at = ?7
            WHERE id = ?8 AND revision = ?9
              AND active_run_id = ?10 AND active_generation = ?11
            "#,
            params![
                health,
                bool_to_i64(durable_success),
                last_successful_generation,
                last_full_scan_at,
                final_error_code,
                final_error_message,
                now,
                record.dto.scan_root_id,
                record.root_revision,
                run_id,
                record.dto.generation,
                record.current_watcher_revision,
                bool_to_i64(rule_failure_pending),
                bool_to_i64(rule_recovery_succeeded),
                bool_to_i64(rule_failure_pending),
            ],
        )?;
        if root_changed != 1 {
            return Err(DbError::Validation(
                "Scan terminal root lease CAS failed.".to_string(),
            ));
        }

        let mapping_status = if success {
            if effective_terminal_status == "completed_with_warnings" {
                "completed_with_warnings"
            } else {
                "completed"
            }
        } else {
            effective_terminal_status
        };
        tx.execute(
            r#"
            UPDATE scan_session_roots
            SET status = CASE
                    WHEN ?1 = 1 THEN CASE WHEN resolution = 'effective' THEN ?2 ELSE 'covered' END
                    ELSE ?3
                END,
                updated_at = ?4
            WHERE run_id = ?5
            "#,
            params![
                bool_to_i64(success),
                mapping_status,
                mapping_status,
                now,
                run_id,
            ],
        )?;

        let session_id =
            record.dto.parent_session_id.as_deref().ok_or_else(|| {
                DbError::Validation("Scan run has no parent session.".to_string())
            })?;
        let projection =
            update_session_projection_tx(&tx, session_id, expected_session_revision, now)?;
        let updated = load_scan_run_record(&tx, run_id)?;
        let session = load_session(&tx, session_id)?;
        let dedupe_pending = projection.dedupe_pending;
        tx.commit()?;
        self.wake_automation_triggers();
        Ok(ScanFinalization {
            run: updated,
            session,
            dedupe_pending,
        })
    }

    pub(crate) fn record_scan_warning(
        &self,
        run_id: &str,
        expected_run_revision: i64,
        expected_root_revision: i64,
        expected_session_revision: i64,
        warning_code: &str,
        warning_message: &str,
    ) -> Result<ScanRunRecord, DbError> {
        let (mut conn, trace_checkout) =
            checkout_managed_scan_write_connection!(self, Some(run_id))?;
        let tx = begin_managed_scan_write_transaction!(
            &mut conn,
            "record_scan_warning",
            Some(run_id),
            None::<&str>,
            trace_checkout
        )?;
        let record = load_scan_run_record(&tx, run_id)?;
        validate_worker_ownership(
            &record,
            run_id,
            expected_run_revision,
            expected_root_revision,
            expected_session_revision,
            true,
        )?;
        let now = current_unix_seconds();
        let run_changed = tx.execute(
            r#"
            UPDATE scan_runs
            SET warnings_count = warnings_count + 1,
                error_code = COALESCE(error_code, ?1),
                error_message = COALESCE(error_message, ?2),
                last_checkpoint_at = ?3, revision = revision + 1, updated_at = ?3
            WHERE id = ?4 AND status = 'running' AND cancel_requested = 0
              AND revision = ?5 AND lease_token = ?6
            "#,
            params![
                warning_code,
                warning_message,
                now,
                run_id,
                expected_run_revision,
                record.lease_token,
            ],
        )?;
        if run_changed != 1 {
            return Err(DbError::Validation(
                "Scan warning run revision CAS failed.".to_string(),
            ));
        }
        let session_id =
            record.dto.parent_session_id.as_deref().ok_or_else(|| {
                DbError::Validation("Scan run has no parent session.".to_string())
            })?;
        let session_changed = tx.execute(
            r#"
            UPDATE scan_sessions
            SET warnings_count = warnings_count + 1,
                last_checkpoint_at = ?1, revision = revision + 1, updated_at = ?1
            WHERE id = ?2 AND revision = ?3 AND cancel_requested = 0
              AND status IN ('running', 'queued')
            "#,
            params![now, session_id, expected_session_revision],
        )?;
        if session_changed != 1 {
            return Err(DbError::Validation(
                "Scan warning session revision CAS failed.".to_string(),
            ));
        }
        let updated = load_scan_run_record(&tx, run_id)?;
        tx.commit()?;
        Ok(updated)
    }

    pub(crate) fn request_scan_cancellation(&self, run_id: &str) -> Result<ScanRunRecord, DbError> {
        let (mut conn, trace_checkout) =
            checkout_managed_scan_write_connection!(self, Some(run_id))?;
        let tx = begin_managed_scan_write_transaction!(
            &mut conn,
            "request_scan_cancellation",
            Some(run_id),
            None::<&str>,
            trace_checkout
        )?;
        let record = load_scan_run_record(&tx, run_id)?;
        if is_terminal_status(&record.dto.status) {
            #[cfg(all(test, feature = "performance-test-tauri"))]
            tx.record_revision_ownership(
                "cancellation_snapshot_terminal",
                &format!(
                    "run_revision={} root_revision={} session_revision={} status={} cancel_requested={}",
                    record.dto.revision,
                    record.root_revision,
                    record.session_revision,
                    record.dto.status,
                    record.dto.cancel_requested
                ),
            );
            tx.commit()?;
            self.wake_automation_triggers();
            return Ok(record);
        }
        if !record.dto.cancel_requested
            && matches!(
                record.dto.phase.as_str(),
                "reconciling_missing" | "optimizing_search" | "finalizing"
            )
        {
            // Once stale reconciliation has committed, cancellation must not
            // turn a run that already changed stale state into a cancelled run.
            tx.commit()?;
            self.wake_automation_triggers();
            return Ok(record);
        }
        let session_id =
            record.dto.parent_session_id.as_deref().ok_or_else(|| {
                DbError::Validation("Scan run has no parent session.".to_string())
            })?;
        let session = load_session(&tx, session_id)?;
        #[cfg(all(test, feature = "performance-test-tauri"))]
        tx.record_revision_ownership(
            "cancellation_snapshot",
            &format!(
                "run_revision={} root_revision={} session_revision={} loaded_session_revision={} status={} phase={} cancel_requested={}",
                record.dto.revision,
                record.root_revision,
                record.session_revision,
                session.revision,
                record.dto.status,
                record.dto.phase,
                record.dto.cancel_requested
            ),
        );
        let now = current_unix_seconds();
        let session_changed = tx.execute(
            r#"
            UPDATE scan_sessions
            SET cancel_requested = 1, status = 'cancelling',
                revision = revision + 1, updated_at = ?1
            WHERE id = ?2 AND revision = ?3
              AND status NOT IN ('cancelled', 'completed', 'completed_with_warnings',
                                 'failed', 'interrupted', 'requires_reconciliation')
            "#,
            params![now, session_id, session.revision],
        )?;
        if session_changed != 1 {
            return Err(DbError::Validation(
                "Scan session cancellation CAS failed.".to_string(),
            ));
        }
        let session_revision = session.revision + 1;
        let queued_runs = {
            let mut statement = tx.prepare(
                "SELECT id FROM scan_runs WHERE parent_session_id = ?1 AND status = 'queued'",
            )?;
            let rows = statement
                .query_map(params![session_id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        for queued_run_id in queued_runs {
            let queued = load_scan_run_record(&tx, &queued_run_id)?;
            let run_changed = tx.execute(
                r#"
                UPDATE scan_runs
                SET status = 'cancelled', phase = 'completed', cancel_requested = 1,
                    finished_at = ?1, last_checkpoint_at = ?1,
                    error_code = 'cancelled_before_start',
                    error_message = 'Scan was cancelled before this root started.',
                    revision = revision + 1, updated_at = ?1
                WHERE id = ?2 AND status = 'queued' AND revision = ?3
                  AND lease_token = ?4
                "#,
                params![now, queued_run_id, queued.dto.revision, queued.lease_token],
            )?;
            if run_changed != 1 {
                return Err(DbError::Validation(
                    "Queued scan cancellation CAS failed.".to_string(),
                ));
            }
            release_root_lease_tx(&tx, &queued, now, "cancelled_before_start")?;
            tx.execute(
                "UPDATE scan_session_roots SET status = 'cancelled_not_started', updated_at = ?1 WHERE run_id = ?2 AND status = 'queued'",
                params![now, queued_run_id],
            )?;
        }
        if record.dto.status == "running" || record.dto.status == "cancelling" {
            let changed = tx.execute(
                r#"
                UPDATE scan_runs
                SET status = 'cancelling', cancel_requested = 1,
                    last_checkpoint_at = ?1, revision = revision + 1, updated_at = ?1
                WHERE id = ?2 AND status IN ('running', 'cancelling')
                  AND revision = ?3 AND lease_token = ?4
                "#,
                params![now, run_id, record.dto.revision, record.lease_token],
            )?;
            if changed != 1 {
                return Err(DbError::Validation(
                    "Running scan cancellation CAS failed.".to_string(),
                ));
            }
        }
        let _ = update_session_projection_tx(&tx, session_id, session_revision, now)?;
        let updated = load_scan_run_record(&tx, run_id)?;
        #[cfg(all(test, feature = "performance-test-tauri"))]
        tx.record_revision_ownership(
            "cancellation_cas_result",
            &format!(
                "session_revision_before={} session_revision_after={session_revision} run_revision_before={} run_revision_after={} cancel_requested={}",
                session.revision,
                record.dto.revision,
                updated.dto.revision,
                updated.dto.cancel_requested
            ),
        );
        tx.commit()?;
        self.wake_automation_triggers();
        Ok(updated)
    }

    pub(crate) fn recover_interrupted_scan_runs(&self) -> Result<usize, DbError> {
        let (mut conn, trace_checkout) =
            checkout_managed_scan_write_connection!(self, None::<&str>)?;
        let tx = begin_managed_scan_write_transaction!(
            &mut conn,
            "recover_interrupted_scan_runs",
            None,
            None::<&str>,
            trace_checkout
        )?;
        let ids = {
            let mut statement = tx.prepare(
                "SELECT id FROM scan_runs WHERE status IN ('queued', 'running', 'cancelling') ORDER BY created_at",
            )?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let now = current_unix_seconds();
        let mut recovered = 0;
        for run_id in ids {
            let record = load_scan_run_record(&tx, &run_id)?;
            let run_changed = tx.execute(
                r#"
                UPDATE scan_runs
                SET status = 'interrupted', phase = 'completed',
                    finished_at = ?1, last_checkpoint_at = ?1,
                    error_code = 'process_interrupted',
                    error_message = 'The application stopped before this scan finalized.',
                    revision = revision + 1, updated_at = ?1
                WHERE id = ?2 AND status IN ('queued', 'running', 'cancelling')
                  AND revision = ?3 AND lease_token = ?4
                "#,
                params![now, run_id, record.dto.revision, record.lease_token],
            )?;
            if run_changed != 1 {
                continue;
            }
            release_root_lease_tx(&tx, &record, now, "process_interrupted")?;
            tx.execute(
                "UPDATE scan_session_roots SET status = 'interrupted', updated_at = ?1 WHERE run_id = ?2 AND status IN ('queued', 'running')",
                params![now, run_id],
            )?;
            if let Some(session_id) = record.dto.parent_session_id.as_deref() {
                let current = load_session(&tx, session_id)?;
                let _ = update_session_projection_tx(&tx, session_id, current.revision, now)?;
            }
            recovered += 1;
        }
        tx.execute(
            r#"
            UPDATE scan_sessions
            SET dedupe_dispatch_state = 'unknown',
                dedupe_last_error = 'The process stopped while dispatching dedupe.',
                revision = revision + 1, updated_at = ?1
            WHERE dedupe_dispatch_state = 'dispatching'
              AND status IN ('completed', 'completed_with_warnings')
            "#,
            params![now],
        )?;
        tx.commit()?;
        self.wake_automation_triggers();
        Ok(recovered)
    }

    pub(crate) fn list_dedupe_dispatch_candidates(
        &self,
        limit: usize,
    ) -> Result<Vec<ScanSessionDto>, DbError> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            r#"
            SELECT id
            FROM scan_sessions
            WHERE dedupe_requested = 1
              AND status IN ('completed', 'completed_with_warnings')
              AND dedupe_dispatch_state IN ('pending', 'unknown', 'failed')
            ORDER BY updated_at, id
            LIMIT ?1
            "#,
        )?;
        let ids = statement
            .query_map(params![limit.min(1000) as i64], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|session_id| load_session(&conn, &session_id))
            .collect()
    }

    pub(crate) fn claim_dedupe_dispatch(
        &self,
        session_id: &str,
    ) -> Result<Option<ScanSessionDto>, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let session = load_session(&tx, session_id)?;
        if !session.dedupe_requested
            || !matches!(
                session.status.as_str(),
                "completed" | "completed_with_warnings"
            )
            || !matches!(
                session.dedupe_dispatch_state.as_str(),
                "pending" | "unknown" | "failed"
            )
        {
            tx.commit()?;
            return Ok(None);
        }
        let now = current_unix_seconds();
        let changed = tx.execute(
            r#"
            UPDATE scan_sessions
            SET dedupe_dispatch_state = 'dispatching',
                dedupe_attempt_count = dedupe_attempt_count + 1,
                dedupe_last_error = NULL, revision = revision + 1, updated_at = ?1
            WHERE id = ?2 AND revision = ?3
              AND dedupe_dispatch_state IN ('pending', 'unknown', 'failed')
            "#,
            params![now, session_id, session.revision],
        )?;
        if changed != 1 {
            return Ok(None);
        }
        let claimed = load_session(&tx, session_id)?;
        tx.commit()?;
        Ok(Some(claimed))
    }

    pub(crate) fn record_dedupe_dispatch(
        &self,
        session_id: &str,
        expected_revision: i64,
        job_id: Option<&str>,
        error: Option<&str>,
    ) -> Result<ScanSessionDto, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = if job_id.is_some() {
            "dispatched"
        } else {
            "failed"
        };
        let now = current_unix_seconds();
        let changed = tx.execute(
            r#"
            UPDATE scan_sessions
            SET dedupe_dispatch_state = ?1, dedupe_job_id = ?2,
                dedupe_last_error = ?3, revision = revision + 1, updated_at = ?4
            WHERE id = ?5 AND revision = ?6 AND dedupe_dispatch_state = 'dispatching'
            "#,
            params![state, job_id, error, now, session_id, expected_revision],
        )?;
        if changed != 1 {
            return Err(DbError::Validation(
                "Dedupe dispatch result CAS failed.".to_string(),
            ));
        }
        let session = load_session(&tx, session_id)?;
        tx.commit()?;
        Ok(session)
    }

    pub(crate) fn prune_scan_observations(&self) -> Result<usize, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = current_unix_seconds();
        let mut candidates = Vec::<(String, String)>::new();
        {
            let mut statement = tx.prepare(
                r#"
                SELECT id, scan_root_id, status, COALESCE(finished_at, created_at)
                FROM scan_runs
                WHERE status NOT IN ('queued', 'running', 'cancelling')
                ORDER BY scan_root_id, created_at DESC
                "#,
            )?;
            let mut newest_by_root = HashMap::<String, usize>::new();
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?;
            for row in rows {
                let (run_id, root_id, status, finished_at) = row?;
                let newest = newest_by_root.entry(root_id).or_default();
                let keep_newest = *newest < 2;
                *newest += 1;
                let retention =
                    if matches!(status.as_str(), "completed" | "completed_with_warnings") {
                        7 * 24 * 60 * 60
                    } else {
                        30 * 24 * 60 * 60
                    };
                if !keep_newest && finished_at <= now.saturating_sub(retention) {
                    candidates.push((run_id, status));
                }
            }
        }
        let mut deleted = 0usize;
        for (run_id, status) in candidates {
            if deleted >= 1000 {
                break;
            }
            let remaining = 1000 - deleted;
            let changed = tx.execute(
                &format!(
                    "DELETE FROM scan_seen WHERE rowid IN (SELECT rowid FROM scan_seen WHERE run_id = ?1 LIMIT {remaining})"
                ),
                params![run_id],
            )?;
            deleted += changed;
            if deleted >= 1000 {
                break;
            }
            let remaining = 1000 - deleted;
            let changed = tx.execute(
                &format!(
                    "DELETE FROM scan_run_errors WHERE rowid IN (SELECT rowid FROM scan_run_errors WHERE run_id = ?1 LIMIT {remaining})"
                ),
                params![run_id],
            )?;
            deleted += changed;
            let _ = status;
        }
        tx.commit()?;
        Ok(deleted)
    }
}

#[derive(Debug, Clone, Copy)]
struct SessionProjection {
    dedupe_pending: bool,
}

fn is_terminal_status(status: &str) -> bool {
    matches!(
        status,
        "cancelled"
            | "completed"
            | "completed_with_warnings"
            | "failed"
            | "interrupted"
            | "requires_reconciliation"
    )
}

fn release_root_lease_tx(
    tx: &Transaction<'_>,
    record: &ScanRunRecord,
    now: i64,
    error_code: &str,
) -> Result<(), DbError> {
    let changed = tx.execute(
        r#"
        UPDATE scan_roots
        SET active_run_id = NULL, active_generation = NULL,
            health_status = 'reconciliation_required', needs_reconciliation = 1,
            last_error_code = ?1,
            last_error_message = 'Scan did not complete and requires reconciliation.',
            revision = revision + 1, updated_at = ?2
        WHERE id = ?3 AND revision = ?4
          AND active_run_id = ?5 AND active_generation = ?6
        "#,
        params![
            error_code,
            now,
            record.dto.scan_root_id,
            record.root_revision,
            record.dto.id,
            record.dto.generation,
        ],
    )?;
    if changed != 1 {
        return Err(DbError::Validation(
            "Root lease release CAS failed.".to_string(),
        ));
    }
    Ok(())
}

fn update_session_projection_tx(
    tx: &Transaction<'_>,
    session_id: &str,
    expected_revision: i64,
    now: i64,
) -> Result<SessionProjection, DbError> {
    let session = load_session(tx, session_id)?;
    if session.revision != expected_revision {
        return Err(DbError::Validation(
            "Scan session projection revision CAS failed.".to_string(),
        ));
    }
    let statuses = {
        let mut statement = tx.prepare(
            "SELECT status FROM scan_session_roots WHERE session_id = ?1 ORDER BY requested_index",
        )?;
        let rows = statement
            .query_map(params![session_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let terminal = statuses.iter().all(|status| is_mapping_terminal(status));
    let has_requires = statuses
        .iter()
        .any(|status| status == "requires_reconciliation");
    let has_interrupted = statuses.iter().any(|status| status == "interrupted");
    let has_failed = statuses
        .iter()
        .any(|status| status == "failed" || status == "invalid");
    let has_cancelled = statuses
        .iter()
        .any(|status| status == "cancelled" || status == "cancelled_not_started");
    let has_warning = statuses
        .iter()
        .any(|status| status == "completed_with_warnings");
    let status = if statuses.is_empty() {
        "failed"
    } else if !terminal {
        if session.cancel_requested {
            "cancelling"
        } else {
            "running"
        }
    } else if has_requires {
        "requires_reconciliation"
    } else if has_interrupted {
        "interrupted"
    } else if has_failed {
        "failed"
    } else if has_cancelled {
        "cancelled"
    } else if has_warning {
        "completed_with_warnings"
    } else {
        "completed"
    };
    let phase = if terminal {
        "completed"
    } else {
        match session.phase.as_str() {
            "finalizing" => "finalizing",
            "completed" => "completed",
            _ => "running",
        }
    };
    let completed_root_count = statuses
        .iter()
        .filter(|status| matches!(status.as_str(), "completed" | "completed_with_warnings"))
        .count() as i64;
    let failed_root_count = statuses
        .iter()
        .filter(|status| matches!(status.as_str(), "failed" | "invalid"))
        .count() as i64;
    let cancelled_root_count = statuses
        .iter()
        .filter(|status| matches!(status.as_str(), "cancelled" | "cancelled_not_started"))
        .count() as i64;
    let covered_root_count = statuses
        .iter()
        .filter(|status| matches!(status.as_str(), "covered" | "duplicate" | "nested"))
        .count() as i64;
    let unstarted_root_count = statuses
        .iter()
        .filter(|status| matches!(status.as_str(), "queued" | "cancelled_not_started"))
        .count() as i64;
    // Duplicate detection runs over rows that were successfully indexed; its
    // correctness does not depend on the index being *complete*.  A run that ended in
    // `requires_reconciliation` still persisted every file it managed to observe, so
    // suppressing dedupe there would drop a user-requested result for a reason that
    // does not affect it.  Only a run that indexed nothing is ineligible.
    let dedupe_eligible_root_count = statuses
        .iter()
        .filter(|status| {
            matches!(
                status.as_str(),
                "completed" | "completed_with_warnings" | "requires_reconciliation"
            )
        })
        .count() as i64;
    let dedupe_pending = terminal
        && matches!(
            status,
            "completed" | "completed_with_warnings" | "requires_reconciliation"
        )
        && session.dedupe_requested
        && dedupe_eligible_root_count > 0
        && matches!(
            session.dedupe_dispatch_state.as_str(),
            "not_requested" | "pending" | "unknown" | "failed"
        );
    let dedupe_state = if dedupe_pending {
        "pending"
    } else {
        &session.dedupe_dispatch_state
    };
    let changed = tx.execute(
        r#"
        UPDATE scan_sessions
        SET status = ?1, phase = ?2,
            completed_root_count = ?3, failed_root_count = ?4,
            cancelled_root_count = ?5, covered_root_count = ?6,
            unstarted_root_count = ?7,
            dedupe_dispatch_state = ?8,
            finished_at = CASE WHEN ?9 = 1 THEN ?10 ELSE finished_at END,
            last_checkpoint_at = ?10, revision = revision + 1, updated_at = ?10
        WHERE id = ?11 AND revision = ?12
        "#,
        params![
            status,
            phase,
            completed_root_count,
            failed_root_count,
            cancelled_root_count,
            covered_root_count,
            unstarted_root_count,
            dedupe_state,
            bool_to_i64(terminal),
            now,
            session_id,
            expected_revision,
        ],
    )?;
    if changed != 1 {
        return Err(DbError::Validation(
            "Scan session projection update affected no rows.".to_string(),
        ));
    }
    Ok(SessionProjection { dedupe_pending })
}

fn is_mapping_terminal(status: &str) -> bool {
    matches!(
        status,
        "completed"
            | "completed_with_warnings"
            | "failed"
            | "cancelled"
            | "interrupted"
            | "requires_reconciliation"
            | "covered"
            | "duplicate"
            | "nested"
            | "invalid"
            | "cancelled_not_started"
    )
}

fn validate_worker_ownership(
    record: &ScanRunRecord,
    run_id: &str,
    expected_run_revision: i64,
    expected_root_revision: i64,
    expected_session_revision: i64,
    require_running: bool,
) -> Result<(), DbError> {
    if record.dto.id != run_id
        || record.dto.revision != expected_run_revision
        || !root_revision_owned_after_watcher_change(record, expected_root_revision)
        || record.session_revision != expected_session_revision
        || record.lease_token.is_empty()
        || record.root_active_run_id.as_deref() != Some(run_id)
        || record.root_active_generation != Some(record.dto.generation)
    {
        return Err(DbError::Validation(
            "Scan worker lost run, root lease, generation, or revision ownership.".to_string(),
        ));
    }
    if require_running && (record.dto.status != "running" || record.dto.cancel_requested) {
        return Err(DbError::Validation(
            "Scan worker cannot persist after cancellation or terminal transition.".to_string(),
        ));
    }
    if record.session_status != "running" {
        return Err(DbError::Validation(
            "Scan session is not writable by the current worker.".to_string(),
        ));
    }
    Ok(())
}

fn root_revision_owned_after_watcher_change(
    record: &ScanRunRecord,
    expected_root_revision: i64,
) -> bool {
    record.root_revision == expected_root_revision
        || record.current_watcher_revision != record.dto.watcher_revision_at_start
}

pub(super) fn watcher_path_within_root(root: &str, path: &str) -> bool {
    let root = root_identity_key(&normalize_scan_root_path(root));
    let path = root_identity_key(&normalize_scan_root_path(path));
    if root == "/" {
        return path.starts_with('/');
    }
    path == root
        || path
            .strip_prefix(&root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn upsert_file_rows_tx(
    tx: &Transaction<'_>,
    files: &[InsertFileRequest],
    observed_at: i64,
    root_id: &str,
    root_epoch: i64,
    owner_kind: &str,
    owner_revision: i64,
) -> Result<bool, DbError> {
    if files.is_empty() {
        return Ok(false);
    }
    let mut statement = tx.prepare(
        r#"
        INSERT INTO files (
            id, path, name, extension, size, mtime, ctime, is_dir, state_code,
            file_type, suggested_name, classification_status, is_stale, last_seen_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0, ?13)
        ON CONFLICT(id) DO UPDATE SET
            path = excluded.path,
            name = excluded.name,
            extension = excluded.extension,
            size = excluded.size,
            mtime = excluded.mtime,
            ctime = excluded.ctime,
            is_dir = excluded.is_dir,
            state_code = excluded.state_code,
            file_type = excluded.file_type,
            suggested_name = CASE
                WHEN files.suggested_name = '' OR files.suggested_name = files.name
                THEN excluded.suggested_name
                ELSE files.suggested_name
            END,
            content_hash = CASE
                WHEN files.path != excluded.path
                  OR files.size != excluded.size
                  OR files.mtime != excluded.mtime
                  OR files.is_dir != excluded.is_dir
                THEN ''
                ELSE files.content_hash
            END,
            is_stale = 0,
            last_seen_at = excluded.last_seen_at
        "#,
    )?;
    let mut invalidations = Vec::new();
    let mut filesystem_changed = false;
    for observed_file in files {
        // `InsertFileRequest.id` is normally the path. Once a legacy row has
        // been parked, the live row has an opaque SourceUID-based id instead.
        // Resolve by path first so repeated Scan/Watcher observations update
        // that live row rather than mistaking the parked legacy id for it.
        let row_at_path = tx
            .query_row(
                "SELECT id FROM files WHERE path = ?1",
                params![observed_file.path],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let mut file = observed_file.clone();
        if let Some(id) = row_at_path.as_ref() {
            file.id = id.clone();
        }
        // Observe identity only inside the existing filesystem publication
        // owner. This is read-only and never feeds trigger-state file IDs.
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let physical = with_managed_scan_source_identity_capture(|| {
            crate::fs_safety::capture_source_object_identity(std::path::Path::new(&file.path)).ok()
        });
        #[cfg(not(all(test, feature = "performance-test-tauri")))]
        let physical =
            crate::fs_safety::capture_source_object_identity(std::path::Path::new(&file.path)).ok();
        let source = observe_source_for_root_tx(
            tx,
            &file,
            physical.as_ref(),
            SourceObservationContext {
                root_id,
                root_epoch,
                owner_kind,
                owner_revision,
                legacy_row_exists: row_at_path.is_some(),
                observed_at,
            },
        )?;

        if source.block_legacy_row {
            let verified = source.verification_state == "verified";
            let reusable_unverified_neutral = if !verified {
                if let Some(previous_id) = row_at_path
                    .as_deref()
                    .filter(|id| neutral_source_row_matches_source(id, &source.source_uid, false))
                {
                    !neutral_source_row_has_attached_facts_tx(tx, previous_id)?
                } else {
                    false
                }
            } else {
                false
            };
            let neutral_id = if reusable_unverified_neutral {
                row_at_path
                    .as_ref()
                    .expect("reusable neutral row is present")
                    .clone()
            } else {
                neutral_source_file_id(&source.source_uid, root_epoch, verified)
            };
            let already_neutral = row_at_path.as_deref() == Some(neutral_id.as_str());
            if let Some(previous_id) = row_at_path.as_deref().filter(|_| !already_neutral) {
                let stale_changed = tx.execute(
                    "UPDATE files SET is_stale = 1, suggested_action = 'Keep', requires_confirmation = 1 WHERE id = ?1 AND is_stale = 0",
                    params![previous_id],
                )? > 0;
                if stale_changed {
                    invalidate_file_in_transaction(tx, previous_id, "stale")?;
                }
                let previous_path: String = tx.query_row(
                    "SELECT path FROM files WHERE id = ?1",
                    params![previous_id],
                    |row| row.get(0),
                )?;
                let archive_path = legacy_unassigned_archive_path(tx, previous_id, &previous_path)?;
                tx.execute(
                    "UPDATE files SET path = ?2, is_stale = 1, content_hash = '', filesystem_observation_key = NULL WHERE id = ?1",
                    params![previous_id, archive_path],
                )?;
                filesystem_changed = true;
            }
            file.id = neutral_id;
            ensure_neutral_source_attribution_tx(
                tx,
                &file.id,
                &file.path,
                &source.source_uid,
                physical.as_ref(),
                source.verification_state == "verified",
                observed_at,
            )?;
            filesystem_changed |= source.changed || row_at_path.is_none();
        }

        let previous = tx
            .query_row(
                "SELECT path, size, mtime, is_dir, is_stale, ctime, filesystem_observation_key FROM files WHERE id = ?1",
                params![file.id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .optional()?;
        let identity = if file.is_dir || physical.is_some() {
            // Retain the last filesystem publication independently of metadata
            // rows later updated by existing user operation owners. Only this
            // owner can compare/publish the root change clock.
            let bytes = serde_json::to_vec(&(
                &file.path,
                file.size,
                file.mtime,
                file.ctime,
                file.is_dir,
                physical
                    .as_ref()
                    .map(|identity| (identity.physical_key.as_deref(), identity.modified_ns)),
            ))?;
            Some(blake3::hash(&bytes).to_hex().to_string())
        } else {
            None
        };
        filesystem_changed |= previous.as_ref().is_some_and(|previous| {
            matches!((&previous.6, &identity), (Some(before), Some(after)) if before != after)
        });
        filesystem_changed |=
            previous
                .as_ref()
                .is_none_or(|(path, size, mtime, is_dir, stale, ctime, _identity)| {
                    path != &file.path
                        || *size != file.size
                        || *mtime != file.mtime
                        || *is_dir != bool_to_i64(file.is_dir)
                        || *stale != 0
                        || *ctime != file.ctime
                });
        statement.execute(params![
            file.id,
            file.path,
            file.name,
            file.extension,
            file.size,
            file.mtime,
            file.ctime,
            bool_to_i64(file.is_dir),
            file.state_code,
            infer_file_type(&file.extension, file.is_dir),
            file.name,
            CLASSIFICATION_STATUS_UNCLASSIFIED,
            observed_at,
        ])?;
        if source.block_legacy_row && file.id.starts_with("zen-canvas-source:") {
            // Neutral rows remain browseable without inheriting old metadata
            // or gaining an operation suggestion while source proof is absent.
            tx.execute(
                "UPDATE files SET purpose = 'Unknown', lifecycle = 'Inbox', context = '', risk_level = 'Normal', suggested_action = 'Keep', suggested_target_path = '', confidence = 0.0, classification_reason = 'Legacy facts remain unassigned to this current filesystem entry.', matched_rules = '[]', requires_confirmation = 1, classification_status = ?2 WHERE id = ?1",
                params![file.id, CLASSIFICATION_STATUS_UNCLASSIFIED],
            )?;
        }
        if let Some(identity) = identity {
            tx.execute(
                "UPDATE files SET filesystem_observation_key=?2 WHERE id=?1 AND filesystem_observation_key IS NOT ?2",
                params![file.id, identity],
            )?;
        }
        if let Some((
            old_path,
            old_size,
            old_mtime,
            old_is_dir,
            old_is_stale,
            _old_ctime,
            _old_identity,
        )) = previous
        {
            if old_path != file.path
                || old_size != file.size
                || old_mtime != file.mtime
                || old_is_dir != bool_to_i64(file.is_dir)
            {
                invalidations.push((
                    file.id.clone(),
                    if old_is_stale != 0 {
                        "missing"
                    } else {
                        "stale"
                    },
                ));
            }
        }
    }
    drop(statement);
    for (file_id, stale_status) in invalidations {
        invalidate_file_in_transaction(tx, &file_id, stale_status)?;
    }
    Ok(filesystem_changed)
}

fn neutral_source_file_id(source_uid: &str, root_epoch: i64, verified: bool) -> String {
    if verified {
        format!("zen-canvas-source:{source_uid}")
    } else {
        // Unverified entries may be reused only while they have no attached
        // facts. If facts were added, a new per-observation id preserves them
        // on the parked row and gives the current object a clean browse row.
        format!("zen-canvas-source:{source_uid}:epoch:{root_epoch}")
    }
}

fn neutral_source_row_matches_source(file_id: &str, source_uid: &str, verified: bool) -> bool {
    let source_file_id = format!("zen-canvas-source:{source_uid}");
    if verified {
        file_id == source_file_id
    } else {
        file_id
            .strip_prefix(&format!("{source_file_id}:epoch:"))
            .is_some_and(|epoch| epoch.parse::<i64>().is_ok())
    }
}

fn neutral_source_row_has_attached_facts_tx(
    tx: &Transaction<'_>,
    file_id: &str,
) -> Result<bool, DbError> {
    tx.query_row(
        r#"
        SELECT
            file.classification_status <> ?2
            OR file.purpose <> 'Unknown'
            OR file.lifecycle <> 'Inbox'
            OR file.context <> ''
            OR file.risk_level <> 'Normal'
            OR file.suggested_action <> 'Keep'
            OR file.suggested_target_path <> ''
            OR (file.suggested_name <> '' AND file.suggested_name <> file.name)
            OR file.confidence <> 0.0
            OR file.classification_reason <> 'Legacy facts remain unassigned to this current filesystem entry.'
            OR file.matched_rules <> '[]'
            OR file.requires_confirmation <> 1
            OR file.last_classified_at IS NOT NULL
            OR file.content_hash <> ''
            OR EXISTS (SELECT 1 FROM file_user_tags WHERE file_id = file.id)
            OR EXISTS (SELECT 1 FROM file_fingerprints WHERE file_id = file.id)
            OR EXISTS (SELECT 1 FROM duplicate_group_members WHERE file_id = file.id)
        FROM files AS file
        WHERE file.id = ?1
        "#,
        params![file_id, CLASSIFICATION_STATUS_UNCLASSIFIED],
        |row| row.get(0),
    )
    .optional()
    .map(|value| value.unwrap_or(false))
    .map_err(DbError::from)
}

fn legacy_unassigned_archive_path(
    tx: &Transaction<'_>,
    file_id: &str,
    original_path: &str,
) -> Result<String, DbError> {
    let digest = blake3::hash(format!("{file_id}\0{original_path}").as_bytes())
        .to_hex()
        .to_string();
    let mut suffix = 0_u32;
    loop {
        let candidate = if suffix == 0 {
            format!("{}{digest}", super::LEGACY_UNASSIGNED_ARCHIVE_PATH_PREFIX)
        } else {
            format!(
                "{}{digest}-{suffix}",
                super::LEGACY_UNASSIGNED_ARCHIVE_PATH_PREFIX
            )
        };
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM files WHERE path = ?1)",
            params![candidate],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(candidate);
        }
        suffix = suffix.saturating_add(1);
    }
}

fn upsert_scan_files_tx(
    tx: &Transaction<'_>,
    run_id: &str,
    files: &[InsertFileRequest],
    observed_at: i64,
    root_id: &str,
    owner_revision: i64,
) -> Result<bool, DbError> {
    if files.is_empty() {
        return Ok(false);
    }
    let root_epoch = advance_root_epoch_tx(tx, root_id, "scan", owner_revision, observed_at)?;
    let filesystem_changed = upsert_file_rows_tx(
        tx,
        files,
        observed_at,
        root_id,
        root_epoch,
        "scan",
        owner_revision,
    )?;

    let mut seen_statement = tx.prepare(
        r#"
        INSERT INTO scan_seen (run_id, file_id, observed_path, observed_at)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(run_id, file_id) DO UPDATE SET
            observed_path = excluded.observed_path,
            observed_at = excluded.observed_at
        "#,
    )?;
    for file in files {
        seen_statement.execute(params![run_id, file.id, file.path, observed_at])?;
    }
    Ok(filesystem_changed)
}

fn insert_scan_errors_tx(
    tx: &Transaction<'_>,
    run_id: &str,
    errors: &[ScanErrorInput],
    created_at: i64,
) -> Result<(), DbError> {
    if errors.is_empty() {
        return Ok(());
    }
    let mut statement = tx.prepare(
        r#"
        INSERT INTO scan_run_errors (
            id, run_id, path, error_code, error_message, affects_coverage, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )?;
    for error in errors {
        statement.execute(params![
            new_job_id("scan-error"),
            run_id,
            error.path,
            error.error_code,
            error.error_message,
            bool_to_i64(error.affects_coverage),
            created_at,
        ])?;
    }
    Ok(())
}

fn root_patterns(root: &str) -> (String, String, String, String) {
    let slash = root.replace('\\', "/");
    let backslash = slash.replace('/', "\\");
    let escaped_slash = escape_like_pattern_local(&slash);
    let escaped_backslash = escape_like_pattern_local(&backslash);
    (
        slash.clone(),
        format!("{escaped_slash}/%"),
        backslash.clone(),
        format!("{escaped_backslash}\\%"),
    )
}

fn ignored_subtree_like_patterns(root: &str) -> Vec<String> {
    let slash_root = escape_like_pattern_local(&root.replace('\\', "/"));
    let backslash_root = escape_like_pattern_local(&root.replace('\\', "/").replace('/', "\\"));
    let mut patterns = Vec::new();

    // Keep legacy exclusions as an additional lossless historical guard.
    // Current ingress eligibility is shared; broad repair of never-observed
    // historical rows is deliberately not authorized by this correction.
    for name in legacy_ignored_dir_names() {
        let name = escape_like_pattern_local(name);
        patterns.extend([
            format!("{slash_root}/{name}"),
            format!("{slash_root}/{name}/%"),
            format!("{slash_root}/%/{name}"),
            format!("{slash_root}/%/{name}/%"),
            format!("{backslash_root}\\{name}"),
            format!("{backslash_root}\\{name}\\%"),
            format!("{backslash_root}\\%\\{name}"),
            format!("{backslash_root}\\%\\{name}\\%"),
        ]);
    }
    for base in generated_dir_variant_bases() {
        for marker in ['.', '-', '_'] {
            let variant = escape_like_pattern_local(&format!("{base}{marker}"));
            patterns.extend([
                format!("{slash_root}/{variant}%"),
                format!("{slash_root}/{variant}%/%"),
                format!("{slash_root}/%/{variant}%"),
                format!("{slash_root}/%/{variant}%/%"),
                format!("{backslash_root}\\{variant}%"),
                format!("{backslash_root}\\{variant}%\\%"),
                format!("{backslash_root}\\%\\{variant}%"),
                format!("{backslash_root}\\%\\{variant}%\\%"),
            ]);
        }
    }
    let root_filter = ManagedPathFilter::new(Path::new(root));
    let root_allowed = !root_filter.excludes(Path::new(root), true);
    for protected in managed_protected_roots() {
        if root_allowed && watcher_path_within_root(&protected.to_string_lossy(), root) {
            continue; // Preserve the platform owner's current-temp exception.
        }
        let slash = escape_like_pattern_local(&normalize_path_text(&protected.to_string_lossy()));
        let backslash = escape_like_pattern_local(
            &normalize_path_text(&protected.to_string_lossy()).replace('/', "\\"),
        );
        patterns.extend([
            slash.clone(),
            format!("{slash}/%"),
            backslash.clone(),
            format!("{backslash}\\%"),
        ]);
    }
    patterns
}

fn escape_like_pattern_local(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '~' | '%' | '_') {
            escaped.push('~');
        }
        escaped.push(ch);
    }
    escaped
}

fn ensure_scan_root_tx(tx: &Transaction<'_>, path: &str) -> Result<ScanRootSeed, DbError> {
    let normalized_path = normalize_scan_root_path(path);
    if normalized_path.is_empty() {
        return Err(DbError::Validation(
            "Scan root path cannot be empty.".to_string(),
        ));
    }
    let existing = if cfg!(windows) {
        tx.query_row(
            r#"
            SELECT id, current_generation, revision, active_run_id
            FROM scan_roots WHERE lower(normalized_path) = lower(?1)
            "#,
            params![normalized_path],
            |row| {
                Ok(ScanRootSeed {
                    id: row.get(0)?,
                    current_generation: row.get(1)?,
                    revision: row.get(2)?,
                    active_run_id: row.get(3)?,
                })
            },
        )
        .optional()?
    } else {
        tx.query_row(
            r#"
            SELECT id, current_generation, revision, active_run_id
            FROM scan_roots WHERE normalized_path = ?1
            "#,
            params![normalized_path],
            |row| {
                Ok(ScanRootSeed {
                    id: row.get(0)?,
                    current_generation: row.get(1)?,
                    revision: row.get(2)?,
                    active_run_id: row.get(3)?,
                })
            },
        )
        .optional()?
    };
    if let Some(seed) = existing {
        return Ok(seed);
    }

    let now = current_unix_seconds();
    let id = new_job_id("scan-root");
    let display_name = scan_root_display_name(&normalized_path);
    tx.execute(
        r#"
        INSERT INTO scan_roots (
            id, normalized_path, display_name, source_kind, created_at, updated_at
        ) VALUES (?1, ?2, ?3, 'file_library', ?4, ?4)
        "#,
        params![id, normalized_path, display_name, now],
    )?;
    Ok(ScanRootSeed {
        id,
        current_generation: 0,
        revision: 0,
        active_run_id: None,
    })
}

fn load_admission_tx(
    tx: &Transaction<'_>,
    session_id: &str,
    created: bool,
) -> Result<ScanAdmission, DbError> {
    let session = load_session(tx, session_id)?;
    let mut statement = tx.prepare(&format!(
        "{SCAN_RUN_SELECT} WHERE run.parent_session_id = ?1 ORDER BY run.created_at, run.id"
    ))?;
    let runs = statement
        .query_map(params![session_id], scan_run_dto_from_row)
        .map_err(DbError::from)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ScanAdmission {
        session,
        runs,
        created,
    })
}

fn load_session(conn: &Connection, session_id: &str) -> Result<ScanSessionDto, DbError> {
    let session = conn.query_row(
        r#"
        SELECT id, request_key, canonical_request_hash, status, phase, cancel_requested,
               requested_root_count, effective_root_count, completed_root_count,
               failed_root_count, cancelled_root_count, covered_root_count,
               unstarted_root_count, dedupe_requested, dedupe_dispatch_state,
               dedupe_attempt_count, dedupe_job_id, dedupe_last_error, scanned_files,
               scanned_directories, warnings_count, errors_count, revision, started_at,
               finished_at, last_checkpoint_at, error_code, error_message, result_json,
               created_at, updated_at
        FROM scan_sessions WHERE id = ?1
        "#,
        params![session_id],
        scan_session_from_row,
    )?;
    let mut mappings_statement = conn.prepare(
        r#"
        SELECT session_id, requested_index, requested_path, normalized_requested_path,
               resolution, effective_root_id, effective_path, effective_index, run_id,
               status, reason, created_at, updated_at
        FROM scan_session_roots
        WHERE session_id = ?1
        ORDER BY requested_index
        "#,
    )?;
    let roots = mappings_statement
        .query_map(params![session_id], scan_session_root_from_row)
        .map_err(DbError::from)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ScanSessionDto { roots, ..session })
}

fn load_scan_run_record(conn: &Connection, run_id: &str) -> Result<ScanRunRecord, DbError> {
    conn.query_row(
        &format!("{SCAN_RUN_SELECT} WHERE run.id = ?1"),
        params![run_id],
        scan_run_record_from_row,
    )
    .map_err(DbError::from)
}

const SCAN_RUN_SELECT: &str = r#"
    SELECT run.id, run.scan_root_id, root.normalized_path, run.generation,
           run.parent_session_id, run.lease_token, run.status, run.phase,
           run.scanned_files, run.scanned_directories, run.processed_bytes,
           run.warnings_count, run.errors_count, run.metadata_error_count,
           run.coverage_error_count, run.coverage_complete,
           run.stale_reconciliation_allowed, run.cancel_requested, run.revision,
           run.started_at, run.finished_at, run.last_checkpoint_at, run.error_code,
           run.error_message, run.result_json, run.watcher_revision_at_start,
           run.created_at, run.updated_at,
           root.revision, root.active_run_id, root.active_generation, root.health_status,
           root.watcher_revision, session.revision, session.status
    FROM scan_runs AS run
    JOIN scan_roots AS root ON root.id = run.scan_root_id
    LEFT JOIN scan_sessions AS session ON session.id = run.parent_session_id
"#;

fn scan_run_dto_from_row(row: &Row<'_>) -> rusqlite::Result<ScanRunDto> {
    Ok(scan_run_record_from_row(row)?.dto)
}

fn scan_run_record_from_row(row: &Row<'_>) -> rusqlite::Result<ScanRunRecord> {
    Ok(ScanRunRecord {
        dto: ScanRunDto {
            id: row.get(0)?,
            scan_root_id: row.get(1)?,
            root_path: row.get(2)?,
            generation: row.get(3)?,
            parent_session_id: row.get(4)?,
            status: row.get(6)?,
            phase: row.get(7)?,
            scanned_files: row.get(8)?,
            scanned_directories: row.get(9)?,
            processed_bytes: row.get(10)?,
            warnings_count: row.get(11)?,
            errors_count: row.get(12)?,
            metadata_error_count: row.get(13)?,
            coverage_error_count: row.get(14)?,
            coverage_complete: row.get::<_, i64>(15)? != 0,
            stale_reconciliation_allowed: row.get::<_, i64>(16)? != 0,
            cancel_requested: row.get::<_, i64>(17)? != 0,
            revision: row.get(18)?,
            session_revision: row.get::<_, Option<i64>>(33)?.unwrap_or_default(),
            started_at: row.get(19)?,
            finished_at: row.get(20)?,
            last_checkpoint_at: row.get(21)?,
            error_code: row.get(22)?,
            error_message: row.get(23)?,
            result_json: row.get(24)?,
            watcher_revision_at_start: row.get(25)?,
            created_at: row.get(26)?,
            updated_at: row.get(27)?,
        },
        lease_token: row.get(5)?,
        root_revision: row.get(28)?,
        root_active_run_id: row.get(29)?,
        root_active_generation: row.get(30)?,
        current_watcher_revision: row.get(32)?,
        session_revision: row.get::<_, Option<i64>>(33)?.unwrap_or_default(),
        session_status: row
            .get::<_, Option<String>>(34)?
            .unwrap_or_else(|| "failed".to_string()),
    })
}

fn scan_root_from_row(row: &Row<'_>) -> rusqlite::Result<ScanRootDto> {
    Ok(ScanRootDto {
        id: row.get(0)?,
        normalized_path: row.get(1)?,
        display_name: row.get(2)?,
        source_kind: row.get(3)?,
        enabled: row.get::<_, i64>(4)? != 0,
        health_status: row.get(5)?,
        current_generation: row.get(6)?,
        active_run_id: row.get(7)?,
        active_generation: row.get(8)?,
        revision: row.get(9)?,
        last_successful_generation: row.get(10)?,
        last_full_scan_at: row.get(11)?,
        needs_reconciliation: row.get::<_, i64>(12)? != 0,
        last_error_code: row.get(13)?,
        last_error_message: row.get(14)?,
        watcher_revision: row.get(15)?,
        watcher_applied_revision: row.get(16)?,
        watcher_last_event_at: row.get(17)?,
        watcher_last_applied_at: row.get(18)?,
        watcher_last_error_code: row.get(19)?,
        watcher_last_error_message: row.get(20)?,
        watcher_rule_recovery_required: row.get::<_, i64>(21)? != 0,
        created_at: row.get(22)?,
        updated_at: row.get(23)?,
    })
}

fn scan_session_from_row(row: &Row<'_>) -> rusqlite::Result<ScanSessionDto> {
    Ok(ScanSessionDto {
        id: row.get(0)?,
        request_key: row.get(1)?,
        canonical_request_hash: row.get(2)?,
        status: row.get(3)?,
        phase: row.get(4)?,
        cancel_requested: row.get::<_, i64>(5)? != 0,
        requested_root_count: row.get(6)?,
        effective_root_count: row.get(7)?,
        completed_root_count: row.get(8)?,
        failed_root_count: row.get(9)?,
        cancelled_root_count: row.get(10)?,
        covered_root_count: row.get(11)?,
        unstarted_root_count: row.get(12)?,
        dedupe_requested: row.get::<_, i64>(13)? != 0,
        dedupe_dispatch_state: row.get(14)?,
        dedupe_attempt_count: row.get(15)?,
        dedupe_job_id: row.get(16)?,
        dedupe_last_error: row.get(17)?,
        scanned_files: row.get(18)?,
        scanned_directories: row.get(19)?,
        warnings_count: row.get(20)?,
        errors_count: row.get(21)?,
        revision: row.get(22)?,
        started_at: row.get(23)?,
        finished_at: row.get(24)?,
        last_checkpoint_at: row.get(25)?,
        error_code: row.get(26)?,
        error_message: row.get(27)?,
        result_json: row.get(28)?,
        created_at: row.get(29)?,
        updated_at: row.get(30)?,
        roots: Vec::new(),
    })
}

fn scan_session_root_from_row(row: &Row<'_>) -> rusqlite::Result<ScanSessionRootDto> {
    Ok(ScanSessionRootDto {
        session_id: row.get(0)?,
        requested_index: row.get(1)?,
        requested_path: row.get(2)?,
        normalized_requested_path: row.get(3)?,
        resolution: row.get(4)?,
        effective_root_id: row.get(5)?,
        effective_path: row.get(6)?,
        effective_index: row.get(7)?,
        run_id: row.get(8)?,
        status: row.get(9)?,
        reason: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn resolve_requested_roots(paths: &[String]) -> Vec<ResolvedRequestedRoot> {
    let mut resolved = Vec::with_capacity(paths.len());
    let mut first_by_key = HashMap::<String, (i64, String)>::new();
    for (index, requested_path) in paths.iter().enumerate() {
        let requested_path = requested_path.trim().to_string();
        let normalized_path = normalize_scan_root_path(&requested_path);
        let key = root_identity_key(&normalized_path);
        let invalid = normalized_path.is_empty();
        if !invalid {
            first_by_key
                .entry(key.clone())
                .or_insert((index as i64, normalized_path.clone()));
        }
        resolved.push(ResolvedRequestedRoot {
            requested_index: index as i64,
            requested_path,
            normalized_path,
            key,
            resolution: if invalid { "invalid" } else { "effective" },
            effective_key: None,
            effective_path: None,
            effective_index: None,
            status: if invalid { "invalid" } else { "queued" },
            reason: invalid.then(|| "empty_root_path".to_string()),
        });
    }

    let unique = first_by_key
        .iter()
        .map(|(key, (first_index, path))| EffectiveRoot {
            key: key.clone(),
            path: path.clone(),
            first_index: *first_index,
        })
        .collect::<Vec<_>>();
    let effective = unique
        .iter()
        .filter(|candidate| {
            !unique.iter().any(|other| {
                other.key != candidate.key && is_nested_root(&other.path, &candidate.path)
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut effective = effective;
    effective.sort_by_key(|root| root.first_index);
    let effective_by_key = effective
        .iter()
        .enumerate()
        .map(|(index, root)| (root.key.clone(), (root.path.clone(), index as i64)))
        .collect::<HashMap<_, _>>();

    for item in &mut resolved {
        if item.resolution == "invalid" {
            continue;
        }
        let first_index = first_by_key
            .get(&item.key)
            .map(|value| value.0)
            .unwrap_or(item.requested_index);
        let is_duplicate = first_index != item.requested_index;
        let effective_key = effective_by_key
            .get(&item.key)
            .map(|_| item.key.clone())
            .or_else(|| {
                effective
                    .iter()
                    .filter(|root| is_nested_root(&root.path, &item.normalized_path))
                    .max_by_key(|root| root.path.len())
                    .map(|root| root.key.clone())
            });
        let Some(effective_key) = effective_key else {
            item.resolution = "invalid";
            item.status = "invalid";
            item.reason = Some("no_effective_root".to_string());
            continue;
        };
        let (effective_path, effective_index) = effective_by_key
            .get(&effective_key)
            .expect("effective mapping was computed from the effective set");
        item.effective_key = Some(effective_key);
        item.effective_path = Some(effective_path.clone());
        item.effective_index = Some(*effective_index);
        if is_duplicate {
            item.resolution = "duplicate_requested";
            item.status = "duplicate";
            item.reason = Some("same_normalized_root_requested_more_than_once".to_string());
        } else if first_by_key.contains_key(&item.key) && effective_by_key.contains_key(&item.key) {
            item.resolution = "effective";
            item.status = "queued";
        } else {
            item.resolution = "nested_under_effective";
            item.status = "nested";
            item.reason = Some("covered_by_effective_ancestor".to_string());
        }
    }
    resolved
}

fn effective_roots(resolved: &[ResolvedRequestedRoot]) -> Vec<EffectiveRoot> {
    let mut roots = resolved
        .iter()
        .filter(|item| item.resolution == "effective")
        .map(|item| EffectiveRoot {
            key: item.key.clone(),
            path: item.normalized_path.clone(),
            first_index: item.requested_index,
        })
        .collect::<Vec<_>>();
    roots.sort_by_key(|root| root.first_index);
    let mut seen = HashSet::new();
    roots.retain(|root| seen.insert(root.key.clone()));
    roots
}

fn canonical_request_hash(
    resolved: &[ResolvedRequestedRoot],
    dedupe: bool,
) -> Result<String, DbError> {
    let roots = resolved
        .iter()
        .map(|root| root.normalized_path.clone())
        .collect::<Vec<_>>();
    let canonical = serde_json::json!({ "roots": roots, "dedupe": dedupe });
    Ok(blake3::hash(serde_json::to_string(&canonical)?.as_bytes())
        .to_hex()
        .to_string())
}

pub(crate) fn normalize_scan_root_path(path: &str) -> String {
    let trimmed = trim_trailing_path_separators(path.trim());
    normalize_path_text(trimmed)
}

fn root_identity_key(path: &str) -> String {
    if cfg!(windows) {
        path.to_ascii_lowercase()
    } else {
        path.to_string()
    }
}

fn is_nested_root(parent: &str, child: &str) -> bool {
    let parent = root_identity_key(&normalize_scan_root_path(parent));
    let child = root_identity_key(&normalize_scan_root_path(child));
    child != parent
        && child
            .strip_prefix(&parent)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn root_paths_overlap(left: &str, right: &str) -> bool {
    let left = normalize_scan_root_path(left);
    let right = normalize_scan_root_path(right);
    root_identity_key(&left) == root_identity_key(&right)
        || is_nested_root(&left, &right)
        || is_nested_root(&right, &left)
}

fn scan_root_display_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn bump_root_change_revision(conn: &Connection, root_id: &str) -> Result<(), DbError> {
    conn.execute(
        "UPDATE scan_roots SET library_change_revision=library_change_revision+1 WHERE id=?1",
        [root_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::{
            atomic::{AtomicU64, Ordering},
            mpsc::{self, Receiver, Sender},
        },
        thread::JoinHandle,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    static TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_db(label: &str) -> Database {
        let sequence = TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        Database::open(std::env::temp_dir().join(format!(
            "zen-canvas-scan-{label}-{}-{timestamp}-{sequence}.sqlite3",
            std::process::id()
        )))
        .expect("open scan test database")
    }

    fn request(root: &str, request_key: &str) -> ScanAdmissionOptions {
        ScanAdmissionOptions {
            request: ManagedScanRequest {
                roots: vec![root.to_string()],
                request_key: Some(request_key.to_string()),
                dedupe: false,
            },
            run_id_override: None,
        }
    }

    fn persist_default_scan_folders(db: &Database, roots: Vec<crate::settings::ScanRootSetting>) {
        let mut settings = crate::settings::get_app_settings(db).expect("read app settings");
        settings.default_scan_folders = roots;
        crate::settings::save_app_settings(db, &settings).expect("persist app settings");
    }

    fn sync_watcher_roots(db: &Database, paths: &[PathBuf]) -> HashMap<String, String> {
        let roots = paths
            .iter()
            .enumerate()
            .map(|(index, path)| crate::settings::ScanRootSetting {
                id: format!("source-identity-root-{index}"),
                path: normalize_scan_root_path(&path.to_string_lossy()),
                label: format!("Source identity root {index}"),
                enabled: true,
                created_at: "2026-10-15T00:00:00.000Z".to_string(),
            })
            .collect::<Vec<_>>();
        persist_default_scan_folders(db, roots.clone());
        db.sync_file_library_watcher_roots(&roots)
            .expect("sync source identity roots");
        let configs = db
            .list_watcher_root_configs()
            .expect("list source identity roots");
        paths
            .iter()
            .map(|path| {
                let normalized = normalize_scan_root_path(&path.to_string_lossy());
                let root_id = configs
                    .iter()
                    .find(|config| config.path == normalized)
                    .expect("configured source identity root")
                    .id
                    .clone();
                (normalized, root_id)
            })
            .collect()
    }

    fn watcher_root_id(db: &Database, path: &Path) -> String {
        let normalized = normalize_scan_root_path(&path.to_string_lossy());
        sync_watcher_roots(db, std::slice::from_ref(&path.to_path_buf()))
            .get(&normalized)
            .expect("configured watcher root")
            .clone()
    }

    struct HeldManagedScanWriter {
        ready: Receiver<Result<Instant, String>>,
        release: Option<Sender<()>>,
        worker: Option<JoinHandle<Result<(), String>>>,
    }

    impl HeldManagedScanWriter {
        fn start(db: Database) -> Self {
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let (release_tx, release_rx) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                let connection = match db.conn() {
                    Ok(connection) => connection,
                    Err(error) => {
                        let message = error.to_string();
                        let _ = ready_tx.send(Err(message.clone()));
                        return Err(message);
                    }
                };
                if let Err(error) = connection.execute_batch("BEGIN IMMEDIATE") {
                    let message = error.to_string();
                    let _ = ready_tx.send(Err(message.clone()));
                    return Err(message);
                }
                let writer_lock_acquired_at = Instant::now();
                let _ = ready_tx.send(Ok(writer_lock_acquired_at));
                match release_rx.recv_timeout(Duration::from_secs(20)) {
                    Ok(()) => connection
                        .execute_batch("COMMIT")
                        .map_err(|error| error.to_string()),
                    Err(error) => {
                        let _ = connection.execute_batch("ROLLBACK");
                        Err(format!("writer release was not received: {error}"))
                    }
                }
            });
            Self {
                ready: ready_rx,
                release: Some(release_tx),
                worker: Some(worker),
            }
        }

        fn wait_until_held(&self) -> Instant {
            self.ready
                .recv_timeout(Duration::from_secs(5))
                .expect("writer lock holder reaches its barrier")
                .expect("writer lock holder begins a real SQLite transaction")
        }

        fn release_and_join(mut self) {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
            self.worker
                .take()
                .expect("writer lock holder thread")
                .join()
                .expect("writer lock holder does not panic")
                .expect("writer lock holder commits cleanly");
        }
    }

    impl Drop for HeldManagedScanWriter {
        fn drop(&mut self) {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    fn sqlite_failure_codes(error: &DbError) -> Option<(rusqlite::ErrorCode, i32)> {
        match error {
            DbError::Sqlite(rusqlite::Error::SqliteFailure(code, _)) => {
                Some((code.code, code.extended_code))
            }
            _ => None,
        }
    }

    fn assert_plain_sqlite_busy(error: &DbError) -> (i32, i32) {
        let (primary, extended) = sqlite_failure_codes(error)
            .unwrap_or_else(|| panic!("expected SQLite failure with result code, got {error:?}"));
        assert_eq!(primary, rusqlite::ErrorCode::DatabaseBusy, "{error:?}");
        let primary_code = extended & 0xff;
        assert_eq!(primary_code, 5, "expected SQLITE_BUSY (5), got {error:?}");
        assert_eq!(extended, 5, "expected plain SQLITE_BUSY (5), got {error:?}");
        (primary_code, extended)
    }

    #[test]
    #[ignore = "Issue #345 deterministic SQLite writer contention evidence"]
    fn managed_scan_admission_waits_for_a_short_real_writer_lock() {
        let db = test_db("issue345-admission-short-lock");
        let holder = HeldManagedScanWriter::start(db.clone());
        holder.wait_until_held();

        let root = normalize_scan_root_path(
            &std::env::temp_dir()
                .join(format!(
                    "zen-canvas-issue345-admission-{}",
                    new_job_id("root")
                ))
                .to_string_lossy(),
        );
        let options = request(&root, &new_job_id("issue345-admission-short"));
        let contender_db = db.clone();
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let started_at = Instant::now();
        let contender = std::thread::spawn(move || {
            let _ = started_tx.send(());
            let result = contender_db.admit_managed_scan(&options);
            let _ = result_tx.send(result);
        });
        started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("admission contender reaches the call boundary");

        match result_rx.recv_timeout(Duration::from_millis(250)) {
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Ok(_) => panic!("admission completed while the writer slot was held"),
            Err(error) => panic!("admission result channel closed unexpectedly: {error}"),
        }
        holder.release_and_join();

        let admission = result_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("admission completes within the existing busy timeout")
            .expect("short writer contention does not reject admission");
        let elapsed = started_at.elapsed();
        assert!(elapsed >= Duration::from_millis(250));
        assert!(admission.created);
        assert_eq!(admission.runs.len(), 1);
        assert_eq!(admission.runs[0].status, "queued");
        println!(
            "issue345_d1 admission_waited=true success=true elapsed_ms={} primary_code=none extended_code=none",
            elapsed.as_millis()
        );
        contender.join().expect("admission contender joins");
    }

    #[test]
    #[ignore = "Issue #345 deterministic SQLite writer contention evidence"]
    fn managed_scan_admission_fails_closed_after_busy_timeout_without_partial_authority() {
        let test_started_at = Instant::now();
        let request_key = new_job_id("issue366-admission-timeout");
        let db = test_db("issue366-admission-timeout-lock");
        const TEST_BUSY_TIMEOUT_MS: u64 = 500;
        db.set_test_busy_timeout_for_all_connections(
            5_000,
            Duration::from_millis(TEST_BUSY_TIMEOUT_MS),
        )
        .expect("verify production timeout then bound the test fixture timeout");
        eprintln!(
            "[issue366-test] request_key={request_key} stage=test_busy_timeout_configured production_default_ms=5000 fixture_busy_timeout_ms={TEST_BUSY_TIMEOUT_MS}"
        );
        let holder = HeldManagedScanWriter::start(db.clone());
        let writer_lock_acquired_at = holder.wait_until_held();
        eprintln!(
            "[issue366-test] request_key={request_key} stage=writer_begin_immediate_returned elapsed_from_test_start_us={} ",
            writer_lock_acquired_at.duration_since(test_started_at).as_micros()
        );
        let root = normalize_scan_root_path(
            &std::env::temp_dir()
                .join(format!(
                    "zen-canvas-issue366-admission-timeout-{}",
                    new_job_id("root")
                ))
                .to_string_lossy(),
        );
        let options = request(&root, &request_key);
        let contender_db = db.clone();
        let worker_request_key = request_key.clone();
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let started_at = test_started_at;
        let admission_watch_started_at = Instant::now();
        let contender = std::thread::spawn(move || {
            issue366_set_test_start(started_at);
            let start_signal_sent_at = Instant::now();
            let start_signal_sent = started_tx.send(start_signal_sent_at).is_ok();
            eprintln!(
                "[issue366-test] request_key={worker_request_key} stage=start_signal_sent elapsed_from_test_start_us={} send_ok={start_signal_sent}",
                start_signal_sent_at.duration_since(started_at).as_micros()
            );
            let admission_call_started_at = Instant::now();
            let admission_thread_cpu_started_ns = issue366_thread_cpu_time_ns();
            eprintln!(
                "[issue366-test] request_key={worker_request_key} stage=admit_call_started elapsed_from_test_start_us={} ",
                admission_call_started_at.duration_since(started_at).as_micros()
            );
            let result = contender_db.admit_managed_scan(&options);
            let admission_call_returned_at = Instant::now();
            let admission_thread_cpu_elapsed_ns = admission_thread_cpu_started_ns
                .zip(issue366_thread_cpu_time_ns())
                .map(|(before, after)| after.saturating_sub(before));
            let result_label = match &result {
                Ok(_) => "success".to_string(),
                Err(DbError::Sqlite(rusqlite::Error::SqliteFailure(code, _))) => format!(
                    "sqlite_error_primary_{}_extended_{}",
                    code.extended_code & 0xff,
                    code.extended_code
                ),
                Err(_) => "other_error".to_string(),
            };
            eprintln!(
                "[issue366-test] request_key={worker_request_key} stage=admit_call_returned elapsed_from_test_start_us={} admission_call_wall_us={} admission_thread_cpu_ns={admission_thread_cpu_elapsed_ns:?} result={result_label}",
                admission_call_returned_at
                    .duration_since(started_at)
                    .as_micros(),
                admission_call_returned_at
                    .duration_since(admission_call_started_at)
                    .as_micros()
            );
            let result_send_started_at = Instant::now();
            let sent = result_tx
                .send((
                    result,
                    admission_call_started_at,
                    admission_call_returned_at,
                    admission_thread_cpu_elapsed_ns,
                    result_send_started_at,
                ))
                .is_ok();
            let result_send_returned_at = Instant::now();
            eprintln!(
                "[issue366-test] request_key={worker_request_key} stage=result_send_returned elapsed_from_test_start_us={} send_wall_us={} send_ok={sent}",
                result_send_returned_at.duration_since(started_at).as_micros(),
                result_send_returned_at
                    .duration_since(result_send_started_at)
                    .as_micros()
            );
        });
        let start_receiver_wait_at = Instant::now();
        let start_signal_sent_at = started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("admission contender reaches the call boundary");
        let start_signal_received_at = Instant::now();
        eprintln!(
            "[issue366-test] request_key={request_key} stage=start_signal_received signal_elapsed_from_test_start_us={} signal_delivery_us={} receiver_wait_wall_us={} ",
            start_signal_sent_at.duration_since(started_at).as_micros(),
            start_signal_received_at
                .duration_since(start_signal_sent_at)
                .as_micros(),
            start_signal_received_at
                .duration_since(start_receiver_wait_at)
                .as_micros()
        );
        // This D2 fixture verified the pool's production 5 s default, then
        // overrides its connections to 500 ms. Keep hosted-runner scheduling
        // variance room while still failing well before the 20 s holder release.
        let channel_wait_started_at = Instant::now();
        eprintln!(
            "[issue366-test] request_key={request_key} stage=result_channel_wait_started elapsed_from_test_start_us={} receiver_deadline_ms=10000",
            channel_wait_started_at.duration_since(started_at).as_micros()
        );
        let received = result_rx.recv_timeout(Duration::from_secs(10));
        let channel_wait_returned_at = Instant::now();
        let error = match received {
            Ok((Err(error), call_started_at, call_returned_at, thread_cpu_ns, sent_at)) => {
                let result_received_at = channel_wait_returned_at;
                eprintln!(
                    "[issue366-test] request_key={request_key} stage=result_received elapsed_from_test_start_us={} channel_wait_wall_us={} call_started_elapsed_us={} call_returned_elapsed_us={} thread_cpu_ns={thread_cpu_ns:?} send_to_receive_us={} ",
                    result_received_at.duration_since(started_at).as_micros(),
                    result_received_at
                        .duration_since(channel_wait_started_at)
                        .as_micros(),
                    call_started_at.duration_since(started_at).as_micros(),
                    call_returned_at.duration_since(started_at).as_micros(),
                    result_received_at.duration_since(sent_at).as_micros()
                );
                error
            }
            Ok((Ok(_), _, _, _, _)) => {
                panic!("admission unexpectedly passed under a held writer")
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                eprintln!(
                    "[issue366-test] request_key={request_key} stage=result_channel_timeout elapsed_from_test_start_us={} channel_wait_wall_us={} writer_lock=still_held",
                    channel_wait_returned_at.duration_since(started_at).as_micros(),
                    channel_wait_returned_at
                        .duration_since(channel_wait_started_at)
                        .as_micros()
                );
                eprintln!(
                    "[issue366-test] request_key={request_key} stage=writer_release_requested elapsed_from_test_start_us={} ",
                    Instant::now().duration_since(started_at).as_micros()
                );
                holder.release_and_join();
                eprintln!(
                    "[issue366-test] request_key={request_key} stage=writer_release_joined elapsed_from_test_start_us={} ",
                    Instant::now().duration_since(started_at).as_micros()
                );
                let after_release = result_rx.recv_timeout(Duration::from_secs(3));
                let after_release_received_at = Instant::now();
                let outcome = match &after_release {
                    Ok((Ok(admission), _, _, _, _)) => format!(
                        "admitted(created={}, runs={})",
                        admission.created,
                        admission.runs.len()
                    ),
                    Ok((Err(error), _, _, _, _)) => format!("error({error:?})"),
                    Err(error) => format!("no_result({error})"),
                };
                let contender_joined = if after_release.is_ok() {
                    let joined = contender.join().is_ok();
                    eprintln!(
                        "[issue366-test] request_key={request_key} stage=contender_joined elapsed_from_test_start_us={} joined={joined}",
                        Instant::now().duration_since(started_at).as_micros()
                    );
                    joined
                } else {
                    false
                };
                eprintln!(
                    "[issue366-test] request_key={request_key} stage=after_writer_release elapsed_from_test_start_us={} post_release_wait_us={} contender_result={} contender_joined={contender_joined}",
                    after_release_received_at.duration_since(started_at).as_micros(),
                    after_release_received_at
                        .duration_since(channel_wait_returned_at)
                        .as_micros(),
                    outcome
                );
                panic!(
                    "admission did not fail boundedly at the busy timeout: timed out waiting on channel; after_writer_release={outcome}; contender_joined={contender_joined}"
                );
            }
            Err(error) => panic!("admission result channel closed unexpectedly: {error}"),
        };
        let elapsed = admission_watch_started_at.elapsed();
        let (primary_code, extended_code) = assert_plain_sqlite_busy(&error);

        let conn = db.conn().expect("inspect admission transaction state");
        let root_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM scan_roots WHERE normalized_path = ?1",
                [&root],
                |row| row.get(0),
            )
            .expect("read root authority count");
        let session_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM scan_sessions WHERE request_key = ?1",
                [&request_key],
                |row| row.get(0),
            )
            .expect("read session authority count");
        let run_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM scan_runs WHERE parent_session_id IN (SELECT id FROM scan_sessions WHERE request_key = ?1)",
                [&request_key],
                |row| row.get(0),
            )
            .expect("read run authority count");
        drop(conn);
        holder.release_and_join();
        contender.join().expect("admission contender joins");

        assert!(
            elapsed >= Duration::from_millis(TEST_BUSY_TIMEOUT_MS / 2),
            "busy timeout returned too early: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "busy timeout was not bounded: {elapsed:?}"
        );
        assert_eq!((root_count, session_count, run_count), (0, 0, 0));
        println!(
            "issue345_d2 admission_waited=true success=false elapsed_ms={} fixture_busy_timeout_ms={} primary_code={} extended_code={} partial_authority=false",
            elapsed.as_millis(), TEST_BUSY_TIMEOUT_MS, primary_code, extended_code
        );
    }

    #[test]
    #[ignore = "Issue #345 deterministic SQLite writer contention evidence"]
    fn managed_scan_batch_write_returns_busy_atomically_and_succeeds_after_release() {
        let db = test_db("issue345-batch-timeout-lock");
        let root =
            std::env::temp_dir().join(format!("zen-canvas-issue345-batch-{}", new_job_id("root")));
        fs::create_dir_all(&root).expect("create managed scan root");
        let root_text = root.to_string_lossy().replace('\\', "/");
        let admission = db
            .admit_managed_scan(&request(&root_text, &new_job_id("issue345-batch")))
            .expect("admit durable batch fixture");
        let claimed = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim durable batch fixture");
        let path = root.join("issue345-batch-entry.txt");
        let path_text = path.to_string_lossy().replace('\\', "/");
        let entry_id = format!("issue345-test:{path_text}");
        let expected_file_id = entry_id.clone();
        let batch_entry = InsertFileRequest {
            id: entry_id,
            path: path_text.clone(),
            name: "issue345-batch-entry.txt".to_string(),
            extension: "txt".to_string(),
            size: 17,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        };
        const TEST_BUSY_TIMEOUT_MS: u64 = 500;
        db.set_test_busy_timeout_for_all_connections(
            5_000,
            Duration::from_millis(TEST_BUSY_TIMEOUT_MS),
        )
        .expect("verify production timeout then bound the test fixture timeout");
        #[cfg(feature = "performance-test-tauri")]
        let trace = ManagedScanWriteTraceGuard::enable_for_database(
            &db,
            "issue345-d3-untracked-writer-negative-control",
            ManagedScanWriteTraceControl::UntrackedWriterNegative,
        );
        println!(
            "[issue366-d3-test] stage=test_busy_timeout_configured production_default_ms=5000 fixture_busy_timeout_ms={TEST_BUSY_TIMEOUT_MS}"
        );
        let holder = HeldManagedScanWriter::start(db.clone());
        holder.wait_until_held();

        let contender_db = db.clone();
        let run_id = claimed.dto.id.clone();
        let run_revision = claimed.dto.revision;
        let root_revision = claimed.root_revision;
        let session_revision = claimed.session_revision;
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let started_at = Instant::now();
        let contender = std::thread::spawn(move || {
            let _ = started_tx.send(());
            let result = contender_db.persist_scan_batch(
                &run_id,
                run_revision,
                root_revision,
                session_revision,
                &ScanBatchInput {
                    entries: &[batch_entry],
                    errors: &[],
                    scanned_files: 1,
                    scanned_directories: 0,
                    processed_bytes: 17,
                    warnings: 0,
                },
            );
            let _ = result_tx.send(result);
        });
        started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("batch writer reaches the real Database API");
        // Keep the production busy timeout unchanged; hosted Apple runners need
        // extra scheduling margin, but this remains bounded before the holder's
        // 20 s fail-safe release.
        let error = match result_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Err(error)) => error,
            Ok(Ok(_)) => panic!("batch persistence unexpectedly passed under a held writer"),
            Err(error) => panic!("batch persistence did not fail boundedly: {error}"),
        };
        let elapsed = started_at.elapsed();
        let (primary_code, extended_code) = assert_plain_sqlite_busy(&error);
        let after_failure = db
            .get_scan_run_record(&claimed.dto.id)
            .expect("read run after rolled-back batch");
        let session_after_failure = db
            .get_scan_session(&admission.session.id)
            .expect("read session after rolled-back batch");
        let conn = db.conn().expect("inspect batch rollback state");
        let persisted_file_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE id = ?1",
                [&expected_file_id],
                |row| row.get(0),
            )
            .expect("read file ledger after failed batch");
        let seen_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM scan_seen WHERE run_id = ?1",
                [&claimed.dto.id],
                |row| row.get(0),
            )
            .expect("read scan_seen after failed batch");
        drop(conn);
        holder.release_and_join();
        contender.join().expect("batch writer contender joins");

        assert!(
            elapsed >= Duration::from_millis(TEST_BUSY_TIMEOUT_MS / 2),
            "batch BUSY returned too early: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "batch BUSY result was not bounded: {elapsed:?}"
        );
        assert_eq!(after_failure.dto.revision, claimed.dto.revision);
        assert_eq!(after_failure.dto.scanned_files, 0);
        assert_eq!(session_after_failure.revision, claimed.session_revision);
        assert_eq!(session_after_failure.scanned_files, 0);
        assert_eq!((persisted_file_count, seen_count), (0, 0));

        let entry = InsertFileRequest {
            id: format!("issue345-test:{path_text}"),
            path: path_text,
            name: "issue345-batch-entry.txt".to_string(),
            extension: "txt".to_string(),
            size: 17,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        };
        let persisted = db
            .persist_scan_batch(
                &claimed.dto.id,
                claimed.dto.revision,
                claimed.root_revision,
                claimed.session_revision,
                &ScanBatchInput {
                    entries: &[entry],
                    errors: &[],
                    scanned_files: 1,
                    scanned_directories: 0,
                    processed_bytes: 17,
                    warnings: 0,
                },
            )
            .expect("same durable write succeeds after lock release");
        let finalizing = db
            .transition_scan_run_phase(
                &persisted.dto.id,
                persisted.dto.revision,
                persisted.root_revision,
                "finalizing",
            )
            .expect("enter finalization after retrying test operation");
        db.finalize_scan_run(
            &finalizing.dto.id,
            finalizing.dto.revision,
            finalizing.root_revision,
            finalizing.session_revision,
            &ScanFinalizeInput {
                terminal_status: "completed".to_string(),
                error_code: None,
                error_message: None,
                allow_stale_reconciliation: false,
                rule_recovery_succeeded: false,
            },
        )
        .expect("settle the test run");
        #[cfg(feature = "performance-test-tauri")]
        {
            let trace_summary = trace.finish();
            assert_eq!(trace_summary.busy_count, 1);
            assert_eq!(trace_summary.busy_without_observed_writer_count, 1);
            assert_eq!(trace_summary.active_transactions_at_finish, 0);
        }
        println!(
            "issue345_d3 operation=persist_scan_batch transaction=BEGIN_IMMEDIATE elapsed_ms={} production_default_ms=5000 fixture_busy_timeout_ms={} primary_code={} extended_code={} rollback_atomic=true after_release_success=true",
            elapsed.as_millis(), TEST_BUSY_TIMEOUT_MS, primary_code, extended_code
        );
        fs::remove_dir_all(root).expect("remove managed scan root fixture");
    }

    #[test]
    fn ad_hoc_scan_admission_does_not_join_settings_watcher_roots() {
        let db = test_db("ad-hoc-watcher-authority");
        let mut settings = crate::settings::get_app_settings(&db).expect("default settings");
        settings.default_scan_folders.clear();
        crate::settings::save_app_settings(&db, &settings).expect("persist empty watcher settings");

        let path = std::env::temp_dir().join(format!("zen-canvas-ad-hoc-{}", new_job_id("root")));
        fs::create_dir_all(&path).expect("create ad-hoc scan root");
        let normalized = normalize_scan_root_path(&path.to_string_lossy());
        db.admit_managed_scan(&request(&normalized, "ad-hoc-watcher-authority"))
            .expect("admit ordinary managed scan");

        let watcher_roots = db
            .list_watcher_root_configs()
            .expect("read settings-owned watcher roots");
        assert!(
            watcher_roots.iter().all(|root| root.path != normalized),
            "ordinary scan admission must not grant watcher ownership"
        );

        drop(db);
        fs::remove_dir_all(path).expect("remove ad-hoc scan root");
    }

    #[test]
    fn watcher_membership_uses_enabled_default_folders_only() {
        let fixture = std::env::temp_dir().join(format!(
            "zen-canvas-watcher-membership-{}",
            uuid::Uuid::new_v4()
        ));
        let enabled_path = fixture.join("enabled-default");
        let disabled_path = fixture.join("disabled-default");
        let custom_search_path = fixture.join("custom-search");
        let global_scope_path = fixture.join("global-index-scope");
        for path in [
            &enabled_path,
            &disabled_path,
            &custom_search_path,
            &global_scope_path,
        ] {
            fs::create_dir_all(path).expect("create isolated watcher-membership root");
        }

        let setting =
            |id: &str, path: &std::path::Path, enabled| crate::settings::ScanRootSetting {
                id: id.to_string(),
                path: path.to_string_lossy().into_owned(),
                label: id.to_string(),
                enabled,
                created_at: "2026-10-02T00:00:00.000Z".to_string(),
            };
        let enabled_default = setting("enabled-default", &enabled_path, true);
        let disabled_default = setting("disabled-default", &disabled_path, false);
        let db = test_db("watcher-membership");
        let mut settings = crate::settings::get_app_settings(&db).expect("default settings");
        settings.default_scan_folders = vec![enabled_default.clone(), disabled_default.clone()];
        settings.custom_search_roots = vec![crate::settings::SearchRootSetting {
            id: "custom-search".to_string(),
            path: custom_search_path.to_string_lossy().into_owned(),
            label: "Custom search only".to_string(),
            enabled: true,
            created_at: "2026-10-02T00:00:00.000Z".to_string(),
        }];
        crate::settings::save_app_settings(&db, &settings).expect("persist scoped settings");
        db.sync_file_library_watcher_roots(&settings.default_scan_folders)
            .expect("sync default File Library roots");

        db.admit_managed_scan(&request(
            &normalize_scan_root_path(&custom_search_path.to_string_lossy()),
            "custom-search-ad-hoc-scan",
        ))
        .expect("create durable root under custom-search-only path");
        db.add_managed_scope(crate::global_index::AddManagedScopeRequest {
            path: global_scope_path.to_string_lossy().into_owned(),
            global_entry_id: None,
            enabled: true,
            allow_local_ai: true,
            allow_cloud_ai: false,
        })
        .expect("add independent Global Index managed scope");
        db.admit_managed_scan(&request(
            &normalize_scan_root_path(&global_scope_path.to_string_lossy()),
            "global-scope-ad-hoc-scan",
        ))
        .expect("create separate ad-hoc File Library root under Global Index path");

        let watcher_roots = db
            .list_watcher_root_configs()
            .expect("read settings-owned watcher roots");
        let watcher_paths = watcher_roots
            .iter()
            .map(|root| normalize_scan_root_path(&root.path))
            .collect::<HashSet<_>>();
        assert_eq!(watcher_paths.len(), 1);
        assert!(watcher_paths.contains(&normalize_scan_root_path(&enabled_path.to_string_lossy())));
        assert!(
            !watcher_paths.contains(&normalize_scan_root_path(&disabled_path.to_string_lossy()))
        );
        assert!(!watcher_paths.contains(&normalize_scan_root_path(
            &custom_search_path.to_string_lossy()
        )));
        assert!(!watcher_paths.contains(&normalize_scan_root_path(
            &global_scope_path.to_string_lossy()
        )));

        drop(db);
        fs::remove_dir_all(fixture).expect("remove isolated watcher-membership fixture");
    }

    #[test]
    fn requested_root_resolution_preserves_order_and_distinguishes_duplicates_and_nested_roots() {
        let resolved = resolve_requested_roots(&[
            "/library".to_string(),
            "/library/child".to_string(),
            "/library".to_string(),
            "".to_string(),
        ]);

        assert_eq!(resolved[0].resolution, "effective");
        assert_eq!(resolved[1].resolution, "nested_under_effective");
        assert_eq!(resolved[2].resolution, "duplicate_requested");
        assert_eq!(resolved[3].resolution, "invalid");
        assert_eq!(effective_roots(&resolved).len(), 1);
        assert_eq!(resolved[1].effective_index, Some(0));
    }

    #[test]
    fn admission_is_idempotent_and_rejects_a_second_active_run_for_the_same_root() {
        let db = test_db("admission");
        let root = format!("/tmp/zen-canvas-scan-admission-{}", new_job_id("root"));
        let first = db
            .admit_managed_scan(&request(&root, "request-1"))
            .expect("admit first");
        assert!(first.created);
        assert_eq!(first.runs[0].generation, 1);

        let duplicate = db
            .admit_managed_scan(&request(&root, "request-1"))
            .expect("repeat request is idempotent");
        assert!(!duplicate.created);
        assert_eq!(duplicate.session.id, first.session.id);
        assert_eq!(duplicate.runs[0].id, first.runs[0].id);

        let conflict = db.admit_managed_scan(&request(&root, "request-2"));
        assert!(conflict.is_err());
        assert_eq!(db.list_scan_roots().expect("list roots").len(), 1);
    }

    #[test]
    fn watcher_reconciliation_retries_each_terminal_failure_status_with_a_new_key() {
        let db = test_db("watcher-reconciliation-retry-statuses");

        for (index, terminal_status) in ["failed", "interrupted", "requires_reconciliation"]
            .into_iter()
            .enumerate()
        {
            let root = format!(
                "/tmp/zen-canvas-watcher-reconciliation-status-{index}-{}",
                new_job_id("root")
            );
            let seed_key = format!("watcher-reconciliation-seed-{index}");
            let seed = db
                .admit_managed_scan(&request(&root, &seed_key))
                .expect("admit seed run");
            let root_id = seed.runs[0].scan_root_id.clone();
            let seed_run = db
                .claim_queued_scan_run(&seed.runs[0].id)
                .expect("claim seed run");
            db.finalize_scan_run(
                &seed_run.dto.id,
                seed_run.dto.revision,
                seed_run.root_revision,
                seed_run.session_revision,
                &ScanFinalizeInput {
                    terminal_status: "failed".to_string(),
                    error_code: Some("seed_failure".to_string()),
                    error_message: Some("seed failure".to_string()),
                    allow_stale_reconciliation: false,
                    rule_recovery_succeeded: false,
                },
            )
            .expect("finalize seed run");
            db.mark_watcher_reconciliation(
                &root_id,
                "watcher_test_failure",
                "watcher test requires reconciliation",
            )
            .expect("mark root dirty");

            let base_key = format!("watcher-reconcile:{root_id}:0");
            let base = db
                .admit_managed_scan(&ScanAdmissionOptions {
                    request: ManagedScanRequest {
                        roots: vec![root.clone()],
                        request_key: Some(base_key.clone()),
                        dedupe: false,
                    },
                    run_id_override: None,
                })
                .expect("admit base reconciliation");
            let base_run = db
                .claim_queued_scan_run(&base.runs[0].id)
                .expect("claim base reconciliation");
            let base_final = db
                .finalize_scan_run(
                    &base_run.dto.id,
                    base_run.dto.revision,
                    base_run.root_revision,
                    base_run.session_revision,
                    &ScanFinalizeInput {
                        terminal_status: terminal_status.to_string(),
                        error_code: Some("watcher_test_failure".to_string()),
                        error_message: Some("watcher reconciliation attempt failed".to_string()),
                        allow_stale_reconciliation: false,
                        rule_recovery_succeeded: false,
                    },
                )
                .expect("finalize base reconciliation");
            assert_eq!(base_final.run.dto.status, terminal_status);

            match db
                .next_watcher_reconciliation_admission(&root_id, 0, current_unix_seconds() + 60)
                .expect("next reconciliation admission")
            {
                WatcherReconciliationAdmission::Start {
                    request_key,
                    attempt,
                } => {
                    assert_eq!(attempt, 1);
                    assert_eq!(request_key, format!("{base_key}:attempt:1"));
                }
                other => panic!("terminal status must be retryable, got {other:?}"),
            }
        }
    }

    #[test]
    fn watcher_reconciliation_retry_is_durable_across_restart_and_old_key_does_not_block_manual_retry(
    ) {
        let db = test_db("watcher-reconciliation-retry-restart");
        let path = db.path().to_path_buf();
        let root = format!(
            "/tmp/zen-canvas-watcher-reconciliation-restart-{}",
            new_job_id("root")
        );
        let seed = db
            .admit_managed_scan(&request(&root, "watcher-reconciliation-restart-seed"))
            .expect("admit seed run");
        let root_id = seed.runs[0].scan_root_id.clone();
        let seed_run = db
            .claim_queued_scan_run(&seed.runs[0].id)
            .expect("claim seed run");
        db.finalize_scan_run(
            &seed_run.dto.id,
            seed_run.dto.revision,
            seed_run.root_revision,
            seed_run.session_revision,
            &ScanFinalizeInput {
                terminal_status: "failed".to_string(),
                error_code: Some("seed_failure".to_string()),
                error_message: Some("seed failure".to_string()),
                allow_stale_reconciliation: false,
                rule_recovery_succeeded: false,
            },
        )
        .expect("finalize seed run");
        db.mark_watcher_reconciliation(
            &root_id,
            "watcher_test_failure",
            "watcher test requires reconciliation",
        )
        .expect("mark root dirty");

        let base_key = format!("watcher-reconcile:{root_id}:0");
        let base = db
            .admit_managed_scan(&ScanAdmissionOptions {
                request: ManagedScanRequest {
                    roots: vec![root.clone()],
                    request_key: Some(base_key.clone()),
                    dedupe: false,
                },
                run_id_override: None,
            })
            .expect("admit base reconciliation");
        let base_run = db
            .claim_queued_scan_run(&base.runs[0].id)
            .expect("claim base reconciliation");
        db.finalize_scan_run(
            &base_run.dto.id,
            base_run.dto.revision,
            base_run.root_revision,
            base_run.session_revision,
            &ScanFinalizeInput {
                terminal_status: "failed".to_string(),
                error_code: Some("watcher_test_failure".to_string()),
                error_message: Some("watcher reconciliation attempt failed".to_string()),
                allow_stale_reconciliation: false,
                rule_recovery_succeeded: false,
            },
        )
        .expect("finalize base reconciliation");
        drop(db);

        let db = Database::open(&path).expect("reopen scan database");
        let retry_key = match db
            .next_watcher_reconciliation_admission(&root_id, 0, current_unix_seconds() + 60)
            .expect("next retry after restart")
        {
            WatcherReconciliationAdmission::Start {
                request_key,
                attempt,
            } => {
                assert_eq!(attempt, 1);
                request_key
            }
            other => panic!("failed reconciliation must be retryable after restart, got {other:?}"),
        };

        let old_request = db
            .admit_managed_scan(&ScanAdmissionOptions {
                request: ManagedScanRequest {
                    roots: vec![root.clone()],
                    request_key: Some(base_key),
                    dedupe: false,
                },
                run_id_override: None,
            })
            .expect("old request key remains an idempotent lookup");
        assert!(!old_request.created);
        assert_eq!(old_request.session.status, "failed");

        let retry = db
            .admit_managed_scan(&ScanAdmissionOptions {
                request: ManagedScanRequest {
                    roots: vec![root.clone()],
                    request_key: Some(retry_key),
                    dedupe: false,
                },
                run_id_override: None,
            })
            .expect("admit controlled retry");
        assert!(retry.created);
        let retry_run = db
            .claim_queued_scan_run(&retry.runs[0].id)
            .expect("claim controlled retry");
        db.finalize_scan_run(
            &retry_run.dto.id,
            retry_run.dto.revision,
            retry_run.root_revision,
            retry_run.session_revision,
            &ScanFinalizeInput {
                terminal_status: "failed".to_string(),
                error_code: Some("watcher_test_failure".to_string()),
                error_message: Some("retry failed".to_string()),
                allow_stale_reconciliation: false,
                rule_recovery_succeeded: false,
            },
        )
        .expect("finalize controlled retry");

        let manual = db
            .admit_managed_scan(&request(&root, "watcher-reconciliation-manual-retry"))
            .expect("manual retry uses a fresh request key");
        assert!(manual.created);
    }

    #[test]
    fn active_parent_rejects_child_but_allows_a_sibling_root() {
        let db = test_db("root-overlap-parent");
        let parent = format!("/tmp/zen-canvas-overlap-parent-{}", new_job_id("root"));
        let child = format!("{parent}/child");
        let sibling = format!("/tmp/zen-canvas-overlap-sibling-{}", new_job_id("root"));

        db.admit_managed_scan(&request(&parent, "overlap-parent"))
            .expect("admit active parent");
        let child_error = db.admit_managed_scan(&request(&child, "overlap-child"));
        assert!(child_error.is_err());
        assert!(db
            .admit_managed_scan(&request(&sibling, "overlap-sibling"))
            .is_ok());
    }

    #[test]
    fn active_child_rejects_parent_and_root_overlap_normalizes_separators() {
        let db = test_db("root-overlap-child");
        let parent = format!("/tmp/zen-canvas-overlap-child-{}", new_job_id("root"));
        let child = format!("{parent}/child");

        db.admit_managed_scan(&request(&child, "overlap-child-active"))
            .expect("admit active child");
        let parent_error = db.admit_managed_scan(&request(&parent, "overlap-parent-late"));
        assert!(parent_error.is_err());
        assert!(root_paths_overlap("/tmp/library", "/tmp/library\\child"));
        #[cfg(windows)]
        assert!(root_paths_overlap("C:\\Library", "c:/library/child"));
    }

    #[test]
    fn generation_is_only_successful_after_terminal_cas_and_then_advances() {
        let db = test_db("generation");
        let root = format!("/tmp/zen-canvas-scan-generation-{}", new_job_id("root"));
        let first = db
            .admit_managed_scan(&request(&root, "generation-1"))
            .expect("admit");
        let run = db.claim_queued_scan_run(&first.runs[0].id).expect("claim");
        let finalized = db
            .finalize_scan_run(
                &run.dto.id,
                run.dto.revision,
                run.root_revision,
                run.session_revision,
                &ScanFinalizeInput {
                    terminal_status: "completed".to_string(),
                    error_code: None,
                    error_message: None,
                    allow_stale_reconciliation: false,
                    rule_recovery_succeeded: false,
                },
            )
            .expect("finalize");
        assert_eq!(finalized.run.dto.generation, 1);
        assert_eq!(finalized.session.status, "completed");
        assert!(db
            .finalize_scan_run(
                &run.dto.id,
                run.dto.revision,
                run.root_revision,
                run.session_revision,
                &ScanFinalizeInput {
                    terminal_status: "completed".to_string(),
                    error_code: None,
                    error_message: None,
                    allow_stale_reconciliation: false,
                    rule_recovery_succeeded: false,
                },
            )
            .is_err());

        let second = db
            .admit_managed_scan(&request(&root, "generation-2"))
            .expect("admit second");
        assert_eq!(second.runs[0].generation, 2);
    }

    #[test]
    fn metadata_errors_write_error_ledger_without_seen_or_stale_reconciliation() {
        let db = test_db("metadata-error");
        let root = format!("/tmp/zen-canvas-scan-metadata-{}", new_job_id("root"));
        let old_path = format!("{root}/old.txt");
        db.insert_file(InsertFileRequest {
            id: old_path.clone(),
            path: old_path.clone(),
            name: "old.txt".to_string(),
            extension: "txt".to_string(),
            size: 1,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("insert old file");
        {
            let conn = db.conn().expect("db connection");
            conn.execute(
                "UPDATE files SET last_seen_at = 0 WHERE id = ?1",
                params![old_path],
            )
            .expect("age old file");
        }

        let admission = db
            .admit_managed_scan(&request(&root, "metadata-error-1"))
            .expect("admit");
        let claimed = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim");
        let entry = InsertFileRequest {
            id: format!("{root}/new.txt"),
            path: format!("{root}/new.txt"),
            name: "new.txt".to_string(),
            extension: "txt".to_string(),
            size: 2,
            mtime: 2,
            ctime: 2,
            is_dir: false,
            state_code: 0,
        };
        let errors = vec![ScanErrorInput {
            path: Some(format!("{root}/unreadable")),
            error_code: "metadata_error".to_string(),
            error_message: "permission denied".to_string(),
            affects_coverage: true,
            metadata_error: true,
        }];
        let updated = db
            .persist_scan_batch(
                &claimed.dto.id,
                claimed.dto.revision,
                claimed.root_revision,
                claimed.session_revision,
                &ScanBatchInput {
                    entries: std::slice::from_ref(&entry),
                    errors: &errors,
                    scanned_files: 1,
                    scanned_directories: 0,
                    processed_bytes: 2,
                    warnings: 0,
                },
            )
            .expect("persist metadata error batch");
        assert_eq!(updated.dto.metadata_error_count, 1);
        assert_eq!(updated.dto.coverage_error_count, 1);

        let conn = db.conn().expect("db connection");
        let seen_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM scan_seen WHERE run_id = ?1",
                params![claimed.dto.id],
                |row| row.get(0),
            )
            .expect("seen count");
        let error_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM scan_run_errors WHERE run_id = ?1",
                params![claimed.dto.id],
                |row| row.get(0),
            )
            .expect("error count");
        let old_stale: i64 = conn
            .query_row(
                "SELECT is_stale FROM files WHERE id = ?1",
                params![old_path],
                |row| row.get(0),
            )
            .expect("old stale state");
        assert_eq!(seen_count, 1);
        assert_eq!(error_count, 1);
        assert_eq!(old_stale, 0);
        drop(conn);
        assert!(db
            .reconcile_missing(
                &updated.dto.id,
                updated.dto.revision,
                updated.root_revision,
                updated.session_revision,
            )
            .is_err());
    }

    #[test]
    fn stale_reconciliation_uses_the_authoritative_ignore_contract_at_any_depth() {
        let db = test_db("stale-ignore-contract");
        let root = format!("/tmp/zen-canvas-scan-ignore-{}", new_job_id("root"));
        let fixtures = [
            (format!("{root}/node_modules/cache.txt"), false),
            (format!("{root}/project/.git-worktree/data.txt"), false),
            (format!("{root}/project/Node_Modules.CACHE/data.txt"), false),
            (format!("{root}\\project\\__PYcache__\\entry.txt"), false),
            (format!("{root}/project/Library/old.txt"), true),
        ];
        for (path, _) in &fixtures {
            db.insert_file(InsertFileRequest {
                id: path.clone(),
                path: path.clone(),
                name: "entry.txt".to_string(),
                extension: "txt".to_string(),
                size: 1,
                mtime: 1,
                ctime: 1,
                is_dir: false,
                state_code: 0,
            })
            .expect("insert ignore fixture");
        }
        db.conn()
            .expect("db connection")
            .execute("UPDATE files SET last_seen_at = 0", [])
            .expect("age ignore fixtures");

        let admission = db
            .admit_managed_scan(&request(&root, "stale-ignore-contract-1"))
            .expect("admit ignore fixture");
        let run = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim ignore fixture");
        let reconciled = db
            .reconcile_missing(
                &run.dto.id,
                run.dto.revision,
                run.root_revision,
                run.session_revision,
            )
            .expect("reconcile ignore fixture");
        assert!(reconciled.dto.stale_reconciliation_allowed);

        let conn = db.conn().expect("db connection");
        for (path, should_be_stale) in fixtures {
            let stale: i64 = conn
                .query_row(
                    "SELECT is_stale FROM files WHERE id = ?1",
                    params![path],
                    |row| row.get(0),
                )
                .expect("read ignore fixture state");
            assert_eq!(stale, i64::from(should_be_stale), "fixture path: {path}");
        }
    }

    #[test]
    fn scanner_publication_clock_ignores_same_filesystem_state_and_counts_size_mtime() {
        let db = test_db("automation-publication-clock");
        let root =
            std::env::temp_dir().join(format!("zen-canvas-trigger-scan-{}", new_job_id("root")));
        fs::create_dir_all(&root).expect("create scanner clock fixture root");
        let path = root.join("file.txt");
        fs::write(&path, b"same").expect("create scanner clock fixture file");
        let normalized_path = normalize_scan_root_path(&path.to_string_lossy());
        let normalized_root = normalize_scan_root_path(&root.to_string_lossy());
        let admission = db
            .admit_managed_scan(&request(&normalized_root, "automation-clock"))
            .unwrap();
        let mut claimed = db.claim_queued_scan_run(&admission.runs[0].id).unwrap();
        let metadata = fs::metadata(&path).expect("read scanner clock fixture metadata");
        let mut entry = insert_request_from_metadata(normalized_path.clone(), &path, &metadata);
        let mut first_observation_revision = None;
        for observation_index in 0..3 {
            if observation_index == 2 {
                fs::write(&path, b"changed size").expect("change fixture filesystem size");
                let metadata = fs::metadata(&path).expect("read changed fixture metadata");
                entry = insert_request_from_metadata(normalized_path.clone(), &path, &metadata);
                // Preserve an explicit mtime delta even on filesystems whose
                // timestamp precision is coarser than this test's runtime.
                entry.mtime = entry.mtime.saturating_add(1);
            }
            claimed = db
                .persist_scan_batch(
                    &claimed.dto.id,
                    claimed.dto.revision,
                    claimed.root_revision,
                    claimed.session_revision,
                    &ScanBatchInput {
                        entries: std::slice::from_ref(&entry),
                        errors: &[],
                        scanned_files: 1,
                        scanned_directories: 0,
                        processed_bytes: entry.size,
                        warnings: 0,
                    },
                )
                .unwrap();
            let current_revision = db
                .conn()
                .unwrap()
                .query_row(
                    "SELECT library_change_revision FROM scan_roots WHERE id=?1",
                    [&claimed.dto.scan_root_id],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap();
            match observation_index {
                0 => first_observation_revision = Some(current_revision),
                1 => assert_eq!(current_revision, first_observation_revision.unwrap()),
                2 => assert_eq!(current_revision, first_observation_revision.unwrap() + 1),
                _ => unreachable!(),
            }
            entry.state_code = 1; // Classification alone is not filesystem truth.
        }
        fs::remove_dir_all(root).expect("remove scanner clock fixture");
    }

    #[test]
    fn successful_metadata_and_scan_seen_commit_together_and_old_worker_cas_is_rejected() {
        let db = test_db("batch-cas");
        let root = format!("/tmp/zen-canvas-scan-batch-{}", new_job_id("root"));
        let admission = db
            .admit_managed_scan(&request(&root, "batch-cas-1"))
            .expect("admit");
        let claimed = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim");
        let entry = InsertFileRequest {
            id: format!("{root}/ok.txt"),
            path: format!("{root}/ok.txt"),
            name: "ok.txt".to_string(),
            extension: "txt".to_string(),
            size: 4,
            mtime: 4,
            ctime: 4,
            is_dir: false,
            state_code: 0,
        };
        let _updated = db
            .persist_scan_batch(
                &claimed.dto.id,
                claimed.dto.revision,
                claimed.root_revision,
                claimed.session_revision,
                &ScanBatchInput {
                    entries: std::slice::from_ref(&entry),
                    errors: &[],
                    scanned_files: 1,
                    scanned_directories: 0,
                    processed_bytes: 4,
                    warnings: 0,
                },
            )
            .expect("persist successful metadata");
        let conn = db.conn().expect("db connection");
        let seen: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM scan_seen WHERE run_id = ?1 AND file_id = ?2",
                params![claimed.dto.id, entry.id],
                |row| row.get(0),
            )
            .expect("scan seen row");
        assert_eq!(seen, 1);
        assert_eq!(
            conn.query_row(
                "SELECT library_change_revision FROM scan_roots WHERE id=?1",
                [&claimed.dto.scan_root_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        drop(conn);

        let late_entry = InsertFileRequest {
            id: format!("{root}/late.txt"),
            path: format!("{root}/late.txt"),
            name: "late.txt".to_string(),
            extension: "txt".to_string(),
            size: 2,
            mtime: 2,
            ctime: 2,
            is_dir: false,
            state_code: 0,
        };
        assert!(db
            .persist_scan_batch(
                &claimed.dto.id,
                claimed.dto.revision,
                claimed.root_revision,
                claimed.session_revision,
                &ScanBatchInput {
                    entries: std::slice::from_ref(&late_entry),
                    errors: &[],
                    scanned_files: 1,
                    scanned_directories: 0,
                    processed_bytes: 2,
                    warnings: 0,
                },
            )
            .is_err());
        let conn = db.conn().expect("db connection");
        let late_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE id = ?1",
                params![late_entry.id],
                |row| row.get(0),
            )
            .expect("late file count");
        assert_eq!(late_count, 0);
    }

    #[test]
    fn complete_reconciliation_stales_only_unseen_rows_and_advances_success_generation() {
        let db = test_db("stale-success");
        let root = format!("/tmp/zen-canvas-scan-stale-{}", new_job_id("root"));
        let old_path = format!("{root}/old.txt");
        db.insert_file(InsertFileRequest {
            id: old_path.clone(),
            path: old_path.clone(),
            name: "old.txt".to_string(),
            extension: "txt".to_string(),
            size: 1,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("insert old file");
        db.conn()
            .expect("db connection")
            .execute(
                "UPDATE files SET last_seen_at = 0 WHERE id = ?1",
                params![old_path],
            )
            .expect("age old file");

        let admission = db
            .admit_managed_scan(&request(&root, "stale-success-1"))
            .expect("admit");
        let claimed = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim");
        let entry = InsertFileRequest {
            id: format!("{root}/new.txt"),
            path: format!("{root}/new.txt"),
            name: "new.txt".to_string(),
            extension: "txt".to_string(),
            size: 2,
            mtime: 2,
            ctime: 2,
            is_dir: false,
            state_code: 0,
        };
        let updated = db
            .persist_scan_batch(
                &claimed.dto.id,
                claimed.dto.revision,
                claimed.root_revision,
                claimed.session_revision,
                &ScanBatchInput {
                    entries: std::slice::from_ref(&entry),
                    errors: &[],
                    scanned_files: 1,
                    scanned_directories: 0,
                    processed_bytes: 2,
                    warnings: 0,
                },
            )
            .expect("persist");
        let reconciled = db
            .reconcile_missing(
                &updated.dto.id,
                updated.dto.revision,
                updated.root_revision,
                updated.session_revision,
            )
            .expect("reconcile");
        assert!(reconciled.dto.stale_reconciliation_allowed);
        let finalization = db
            .finalize_scan_run(
                &reconciled.dto.id,
                reconciled.dto.revision,
                reconciled.root_revision,
                reconciled.session_revision,
                &ScanFinalizeInput {
                    terminal_status: "completed".to_string(),
                    error_code: None,
                    error_message: None,
                    allow_stale_reconciliation: true,
                    rule_recovery_succeeded: false,
                },
            )
            .expect("finalize");
        assert_eq!(finalization.session.status, "completed");
        let conn = db.conn().expect("db connection");
        let stale: i64 = conn
            .query_row(
                "SELECT is_stale FROM files WHERE id = ?1",
                params![old_path],
                |row| row.get(0),
            )
            .expect("stale state");
        let successful_generation: i64 = conn
            .query_row(
                "SELECT last_successful_generation FROM scan_roots WHERE id = ?1",
                params![finalization.run.dto.scan_root_id],
                |row| row.get(0),
            )
            .expect("successful generation");
        assert_eq!(stale, 1);
        assert_eq!(successful_generation, 1);
    }

    #[test]
    fn cancelled_finalization_never_stales_unseen_files() {
        let db = test_db("cancel-stale");
        let root = format!("/tmp/zen-canvas-scan-cancel-{}", new_job_id("root"));
        let old_path = format!("{root}/old.txt");
        db.insert_file(InsertFileRequest {
            id: old_path.clone(),
            path: old_path.clone(),
            name: "old.txt".to_string(),
            extension: "txt".to_string(),
            size: 1,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("insert old file");
        let admission = db
            .admit_managed_scan(&request(&root, "cancel-stale-1"))
            .expect("admit");
        let claimed = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim");
        let cancelling = db
            .request_scan_cancellation(&claimed.dto.id)
            .expect("request cancellation");
        let finalization = db
            .finalize_scan_run(
                &cancelling.dto.id,
                cancelling.dto.revision,
                cancelling.root_revision,
                cancelling.session_revision,
                &ScanFinalizeInput {
                    terminal_status: "cancelled".to_string(),
                    error_code: Some("cancelled".to_string()),
                    error_message: Some("cancelled by test".to_string()),
                    allow_stale_reconciliation: false,
                    rule_recovery_succeeded: false,
                },
            )
            .expect("finalize cancellation");
        assert_eq!(finalization.run.dto.status, "cancelled");
        let stale: i64 = db
            .conn()
            .expect("db connection")
            .query_row(
                "SELECT is_stale FROM files WHERE id = ?1",
                params![old_path],
                |row| row.get(0),
            )
            .expect("stale state");
        assert_eq!(stale, 0);
    }

    #[test]
    fn multi_root_projection_preserves_mapping_and_terminal_priority() {
        let db = test_db("multi-root");
        let parent = format!("/tmp/zen-canvas-scan-multi-{}", new_job_id("root"));
        let child = format!("{parent}/child");
        let sibling = format!("/tmp/zen-canvas-scan-sibling-{}", new_job_id("root"));
        let admission = db
            .admit_managed_scan(&ScanAdmissionOptions {
                request: ManagedScanRequest {
                    roots: vec![parent.clone(), child, "".to_string(), sibling],
                    request_key: Some("multi-root-1".to_string()),
                    dedupe: true,
                },
                run_id_override: None,
            })
            .expect("admit multi-root request");
        assert_eq!(admission.runs.len(), 2);
        assert_eq!(admission.session.requested_root_count, 4);
        assert_eq!(admission.session.effective_root_count, 2);
        assert_eq!(admission.session.roots[0].status, "queued");
        assert_eq!(admission.session.roots[1].status, "nested");
        assert_eq!(admission.session.roots[2].status, "invalid");
        assert_eq!(admission.session.roots[3].status, "queued");

        let snapshot = db
            .get_managed_scan_snapshot(&admission.session.id)
            .expect("durable session snapshot");
        assert_eq!(snapshot.session.id, admission.session.id);
        assert_eq!(snapshot.session.requested_root_count, 4);
        assert_eq!(snapshot.session.roots.len(), 4);
        assert_eq!(snapshot.runs.len(), 2);
        assert_eq!(
            snapshot
                .runs
                .iter()
                .map(|run| run.id.as_str())
                .collect::<HashSet<_>>(),
            admission
                .runs
                .iter()
                .map(|run| run.id.as_str())
                .collect::<HashSet<_>>()
        );

        let first = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim first root");
        let first_finalizing = db
            .transition_scan_run_phase(
                &first.dto.id,
                first.dto.revision,
                first.root_revision,
                "finalizing",
            )
            .expect("finalize first root");
        assert_eq!(
            db.get_scan_session(&admission.session.id)
                .expect("session after first finalizing")
                .phase,
            "running"
        );
        let first_final = db
            .finalize_scan_run(
                &first_finalizing.dto.id,
                first_finalizing.dto.revision,
                first_finalizing.root_revision,
                first_finalizing.session_revision,
                &ScanFinalizeInput {
                    terminal_status: "completed".to_string(),
                    error_code: None,
                    error_message: None,
                    allow_stale_reconciliation: false,
                    rule_recovery_succeeded: false,
                },
            )
            .expect("complete first root");
        assert_eq!(first_final.session.status, "running");

        let second = db
            .claim_queued_scan_run(&admission.runs[1].id)
            .expect("claim second root");
        let second_finalizing = db
            .transition_scan_run_phase(
                &second.dto.id,
                second.dto.revision,
                second.root_revision,
                "finalizing",
            )
            .expect("finalize last root");
        assert_eq!(
            db.get_scan_session(&admission.session.id)
                .expect("session after last finalizing")
                .phase,
            "finalizing"
        );
        let finalization = db
            .finalize_scan_run(
                &second_finalizing.dto.id,
                second_finalizing.dto.revision,
                second_finalizing.root_revision,
                second_finalizing.session_revision,
                &ScanFinalizeInput {
                    terminal_status: "failed".to_string(),
                    error_code: Some("test_failure".to_string()),
                    error_message: Some("partial failure".to_string()),
                    allow_stale_reconciliation: false,
                    rule_recovery_succeeded: false,
                },
            )
            .expect("fail second root");
        assert_eq!(finalization.session.status, "failed");
        assert_eq!(finalization.session.completed_root_count, 1);
        assert_eq!(finalization.session.failed_root_count, 2);
        assert_eq!(finalization.session.covered_root_count, 1);
        assert_eq!(finalization.session.phase, "completed");
        assert_eq!(finalization.session.dedupe_dispatch_state, "not_requested");
    }

    #[test]
    fn cancellation_marks_queued_roots_not_started_and_preserves_failure_priority() {
        let db = test_db("cancel-queued");
        let root = format!("/tmp/zen-canvas-scan-cancel-queued-{}", new_job_id("root"));
        let second_root = format!("/tmp/zen-canvas-scan-cancel-queued-{}", new_job_id("root"));
        let admission = db
            .admit_managed_scan(&ScanAdmissionOptions {
                request: ManagedScanRequest {
                    roots: vec![root, second_root, "".to_string()],
                    request_key: Some("cancel-queued-1".to_string()),
                    dedupe: false,
                },
                run_id_override: None,
            })
            .expect("admit");
        let cancelled = db
            .request_scan_cancellation(&admission.runs[0].id)
            .expect("cancel queued run");
        assert_eq!(cancelled.dto.status, "cancelled");
        let session = db.get_scan_session(&admission.session.id).expect("session");
        assert_eq!(session.status, "failed");
        assert!(session
            .roots
            .iter()
            .filter(|root| root.resolution == "effective")
            .all(|root| root.status == "cancelled_not_started"));
        assert_eq!(
            session
                .roots
                .iter()
                .filter(|root| root.resolution == "invalid")
                .count(),
            1
        );
    }

    #[test]
    fn dedupe_dispatch_claim_and_retry_are_durable_at_least_once() {
        let db = test_db("dedupe-dispatch");
        let root = format!("/tmp/zen-canvas-scan-dedupe-{}", new_job_id("root"));
        let admission = db
            .admit_managed_scan(&ScanAdmissionOptions {
                request: ManagedScanRequest {
                    roots: vec![root],
                    request_key: Some("dedupe-dispatch-1".to_string()),
                    dedupe: true,
                },
                run_id_override: None,
            })
            .expect("admit");
        let run = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim");
        let finalization = db
            .finalize_scan_run(
                &run.dto.id,
                run.dto.revision,
                run.root_revision,
                run.session_revision,
                &ScanFinalizeInput {
                    terminal_status: "completed".to_string(),
                    error_code: None,
                    error_message: None,
                    allow_stale_reconciliation: false,
                    rule_recovery_succeeded: false,
                },
            )
            .expect("complete");
        assert!(finalization.dedupe_pending);
        let dispatching = db
            .claim_dedupe_dispatch(&admission.session.id)
            .expect("claim dispatch")
            .expect("pending dispatch");
        assert_eq!(dispatching.dedupe_dispatch_state, "dispatching");
        assert_eq!(dispatching.dedupe_attempt_count, 1);
        assert!(db
            .claim_dedupe_dispatch(&admission.session.id)
            .expect("second claim")
            .is_none());
        let failed = db
            .record_dedupe_dispatch(
                &admission.session.id,
                dispatching.revision,
                None,
                Some("manager unavailable"),
            )
            .expect("record failed dispatch");
        assert_eq!(failed.dedupe_dispatch_state, "failed");
        let retry = db
            .claim_dedupe_dispatch(&admission.session.id)
            .expect("retry claim")
            .expect("retry pending");
        assert_eq!(retry.dedupe_attempt_count, 2);
    }

    #[test]
    fn cancellation_is_rejected_after_stale_reconciliation_has_committed() {
        let db = test_db("cancel-finalizing");
        let root = format!("/tmp/zen-canvas-scan-finalizing-{}", new_job_id("root"));
        let admission = db
            .admit_managed_scan(&request(&root, "cancel-finalizing-1"))
            .expect("admit");
        let claimed = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim");
        let reconciling = db
            .transition_scan_run_phase(
                &claimed.dto.id,
                claimed.dto.revision,
                claimed.root_revision,
                "reconciling_missing",
            )
            .expect("phase");
        let unchanged = db
            .request_scan_cancellation(&reconciling.dto.id)
            .expect("late cancel");
        assert_eq!(unchanged.dto.status, "running");
        assert!(!unchanged.dto.cancel_requested);
    }

    #[test]
    fn missing_root_finalization_preserves_reconciliation_health() {
        let db = test_db("root-health");
        let root = format!("/tmp/zen-canvas-scan-health-{}", new_job_id("root"));
        let admission = db
            .admit_managed_scan(&request(&root, "root-health-1"))
            .expect("admit");
        let claimed = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim");
        let finalization = db
            .finalize_scan_run(
                &claimed.dto.id,
                claimed.dto.revision,
                claimed.root_revision,
                claimed.session_revision,
                &ScanFinalizeInput {
                    terminal_status: "requires_reconciliation".to_string(),
                    error_code: Some("root_missing".to_string()),
                    error_message: Some("root disappeared".to_string()),
                    allow_stale_reconciliation: false,
                    rule_recovery_succeeded: false,
                },
            )
            .expect("finalize missing root");
        let health = db
            .get_scan_root_health(Some(&finalization.run.dto.scan_root_id), None)
            .expect("root health");
        assert_eq!(health.health_status, "missing");
        assert!(health.needs_reconciliation);
        assert_eq!(health.last_successful_generation, None);
    }

    #[test]
    fn historical_interrupted_and_requires_reconciliation_observations_follow_bounded_retention() {
        let db = test_db("retention-bounded");
        let root = format!("/tmp/zen-canvas-scan-retention-{}", new_job_id("root"));
        let statuses = [
            "interrupted",
            "requires_reconciliation",
            "interrupted",
            "requires_reconciliation",
        ];
        let mut run_ids = Vec::new();
        for (index, terminal_status) in statuses.iter().enumerate() {
            let admission = db
                .admit_managed_scan(&request(&root, &format!("retention-bounded-{index}")))
                .expect("admit historical run");
            let claimed = db
                .claim_queued_scan_run(&admission.runs[0].id)
                .expect("claim historical run");
            let entry = InsertFileRequest {
                id: format!("{root}/observed-{index}.txt"),
                path: format!("{root}/observed-{index}.txt"),
                name: format!("observed-{index}.txt"),
                extension: "txt".to_string(),
                size: 1,
                mtime: 1,
                ctime: 1,
                is_dir: false,
                state_code: 0,
            };
            let errors = vec![ScanErrorInput {
                path: Some(format!("{root}/unreadable-{index}")),
                error_code: "metadata_error".to_string(),
                error_message: "unreadable".to_string(),
                affects_coverage: true,
                metadata_error: true,
            }];
            let updated = db
                .persist_scan_batch(
                    &claimed.dto.id,
                    claimed.dto.revision,
                    claimed.root_revision,
                    claimed.session_revision,
                    &ScanBatchInput {
                        entries: std::slice::from_ref(&entry),
                        errors: &errors,
                        scanned_files: 1,
                        scanned_directories: 0,
                        processed_bytes: 1,
                        warnings: 0,
                    },
                )
                .expect("persist historical run");
            let finalization = db
                .finalize_scan_run(
                    &updated.dto.id,
                    updated.dto.revision,
                    updated.root_revision,
                    updated.session_revision,
                    &ScanFinalizeInput {
                        terminal_status: (*terminal_status).to_string(),
                        error_code: Some("coverage_incomplete".to_string()),
                        error_message: Some("metadata error".to_string()),
                        allow_stale_reconciliation: false,
                        rule_recovery_succeeded: false,
                    },
                )
                .expect("finalize historical run");
            run_ids.push(finalization.run.dto.id);
        }

        let now = current_unix_seconds();
        let conn = db.conn().expect("db connection");
        for (index, run_id) in run_ids.iter().enumerate() {
            let timestamp = if index < 2 { 0 } else { now + index as i64 };
            conn.execute(
                "UPDATE scan_runs SET finished_at = ?1, created_at = ?1 WHERE id = ?2",
                params![timestamp, run_id],
            )
            .expect("age historical run");
        }
        drop(conn);

        db.prune_scan_observations().expect("bounded prune");
        let conn = db.conn().expect("db connection");
        for (index, run_id) in run_ids.iter().enumerate() {
            let seen_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM scan_seen WHERE run_id = ?1",
                    params![run_id],
                    |row| row.get(0),
                )
                .expect("seen count");
            let error_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM scan_run_errors WHERE run_id = ?1",
                    params![run_id],
                    |row| row.get(0),
                )
                .expect("error count");
            if index < 2 {
                assert_eq!(seen_count, 0, "old run {run_id} scan_seen");
                assert_eq!(error_count, 0, "old run {run_id} errors");
            } else {
                assert_eq!(seen_count, 1, "newest run {run_id} scan_seen");
                assert_eq!(error_count, 1, "newest run {run_id} errors");
            }
        }
    }

    #[test]
    #[ignore = "bounded performance fixture; invoked by npm run test:performance"]
    fn performance_100k_scan_seen_missing_reconcile_and_wal_reader() {
        const FILE_COUNT: usize = 100_000;
        let db = test_db("performance-100k");
        let seen_root = format!(
            "/tmp/zen-canvas-scan-performance-seen-{}",
            new_job_id("root")
        );
        let seen_admission = db
            .admit_managed_scan(&request(&seen_root, "performance-seen-1"))
            .expect("admit seen fixture");
        let seen_run = db
            .claim_queued_scan_run(&seen_admission.runs[0].id)
            .expect("claim seen fixture");
        let seen_entries = (0..FILE_COUNT)
            .map(|index| {
                let path = format!("{seen_root}/seen-{index:06}.txt");
                InsertFileRequest {
                    id: path.clone(),
                    path,
                    name: format!("seen-{index:06}.txt"),
                    extension: "txt".to_string(),
                    size: 1,
                    mtime: 1,
                    ctime: 1,
                    is_dir: false,
                    state_code: 0,
                }
            })
            .collect::<Vec<_>>();
        let insert_started = Instant::now();
        let seen_updated = db
            .persist_scan_batch(
                &seen_run.dto.id,
                seen_run.dto.revision,
                seen_run.root_revision,
                seen_run.session_revision,
                &ScanBatchInput {
                    entries: &seen_entries,
                    errors: &[],
                    scanned_files: FILE_COUNT as i64,
                    scanned_directories: 0,
                    processed_bytes: FILE_COUNT as i64,
                    warnings: 0,
                },
            )
            .expect("insert 100k scan_seen rows");
        let insert_elapsed = insert_started.elapsed();
        let seen_final = db
            .finalize_scan_run(
                &seen_updated.dto.id,
                seen_updated.dto.revision,
                seen_updated.root_revision,
                seen_updated.session_revision,
                &ScanFinalizeInput {
                    terminal_status: "completed".to_string(),
                    error_code: None,
                    error_message: None,
                    allow_stale_reconciliation: false,
                    rule_recovery_succeeded: false,
                },
            )
            .expect("finalize seen fixture");

        let missing_root = format!(
            "/tmp/zen-canvas-scan-performance-missing-{}",
            new_job_id("root")
        );
        {
            let mut conn = db.conn().expect("db connection");
            let tx = conn
                .transaction()
                .expect("seed missing fixture transaction");
            let mut statement = tx
                .prepare(
                    "INSERT INTO files (id, path, name, extension, size, mtime, ctime, is_dir, state_code, file_type, suggested_name, classification_status, is_stale, last_seen_at) VALUES (?1, ?1, ?2, 'txt', 1, 1, 1, 0, 0, 'Other', ?2, 'unclassified', 0, 0)",
                )
                .expect("prepare missing fixture");
            for index in 0..FILE_COUNT {
                let name = format!("missing-{index:06}.txt");
                let path = format!("{missing_root}/{name}");
                statement
                    .execute(params![path, name])
                    .expect("seed missing file");
            }
            drop(statement);
            tx.commit().expect("commit missing fixture");
        }
        let missing_admission = db
            .admit_managed_scan(&request(&missing_root, "performance-missing-1"))
            .expect("admit missing fixture");
        let missing_run = db
            .claim_queued_scan_run(&missing_admission.runs[0].id)
            .expect("claim missing fixture");
        let reconcile_started = Instant::now();
        let reconciled = db
            .reconcile_missing(
                &missing_run.dto.id,
                missing_run.dto.revision,
                missing_run.root_revision,
                missing_run.session_revision,
            )
            .expect("reconcile 100k missing rows");
        let reconcile_elapsed = reconcile_started.elapsed();
        assert!(reconciled.dto.stale_reconciliation_allowed);
        let missing_final = db
            .finalize_scan_run(
                &reconciled.dto.id,
                reconciled.dto.revision,
                reconciled.root_revision,
                reconciled.session_revision,
                &ScanFinalizeInput {
                    terminal_status: "completed".to_string(),
                    error_code: None,
                    error_message: None,
                    allow_stale_reconciliation: true,
                    rule_recovery_succeeded: false,
                },
            )
            .expect("finalize missing fixture");

        let writer = db.conn().expect("writer connection");
        writer
            .execute_batch("BEGIN IMMEDIATE; UPDATE scan_roots SET updated_at = updated_at;")
            .expect("hold WAL writer transaction");
        let reader_started = Instant::now();
        let roots = db.list_scan_roots().expect("read roots during writer");
        let reader_elapsed = reader_started.elapsed();
        writer
            .execute_batch("ROLLBACK")
            .expect("rollback writer transaction");
        assert!(roots.len() >= 2);

        let prune_started = Instant::now();
        db.prune_scan_observations().expect("bounded prune");
        let prune_elapsed = prune_started.elapsed();
        assert!(seen_final.session.status == "completed");
        assert_eq!(missing_final.session.status, "completed");
        assert!(insert_elapsed < Duration::from_secs(120));
        assert!(reconcile_elapsed < Duration::from_secs(120));
        assert!(reader_elapsed < Duration::from_secs(10));
        assert!(prune_elapsed < Duration::from_secs(30));
        eprintln!(
            "scan performance: scan_seen_insert={:?}, missing_reconcile={:?}, wal_reader={:?}, prune={:?}",
            insert_elapsed, reconcile_elapsed, reader_elapsed, prune_elapsed
        );
    }

    #[test]
    fn startup_recovery_interrupts_active_runs_and_releases_the_root_lease() {
        let db = test_db("recovery");
        let root = format!("/tmp/zen-canvas-scan-recovery-{}", new_job_id("root"));
        let admission = db
            .admit_managed_scan(&request(&root, "recovery-1"))
            .expect("admit");
        let run = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim");
        assert_eq!(db.recover_interrupted_scan_runs().expect("recover"), 1);
        let recovered = db.get_scan_run_record(&run.dto.id).expect("recovered run");
        assert_eq!(recovered.dto.status, "interrupted");
        assert!(recovered.root_active_run_id.is_none());
        assert_eq!(
            db.get_scan_session(&admission.session.id)
                .expect("session")
                .status,
            "interrupted"
        );
    }

    #[test]
    fn watcher_exact_mutation_uses_durable_revision_without_scan_seen_or_generation() {
        let db = test_db("watcher-exact");
        let root_path =
            std::env::temp_dir().join(format!("zen-canvas-watcher-root-{}", new_job_id("root")));
        fs::create_dir_all(&root_path).expect("create watcher root");
        let file_path = root_path.join("new.txt");
        fs::write(&file_path, b"watcher").expect("create watcher file");
        let root = root_path.to_string_lossy().into_owned();
        let watcher_roots = vec![crate::settings::ScanRootSetting {
            id: "settings-root".to_string(),
            path: root.clone(),
            label: "Watcher root".to_string(),
            enabled: true,
            created_at: "2026-07-27T00:00:00.000Z".to_string(),
        }];
        persist_default_scan_folders(&db, watcher_roots.clone());
        db.sync_file_library_watcher_roots(&watcher_roots)
            .expect("sync watcher root");
        let root_id = db
            .list_watcher_root_configs()
            .expect("watcher configs")
            .into_iter()
            .find(|config| config.path == normalize_scan_root_path(&root))
            .expect("managed watcher root")
            .id;
        let batch = db
            .begin_watcher_revision(&root_id)
            .expect("begin watcher revision")
            .expect("enabled root revision");
        assert_eq!(batch.watcher_revision, 1);
        let normalized_file = normalize_scan_root_path(&file_path.to_string_lossy());
        let result = db
            .apply_watcher_exact_mutations(
                &root_id,
                batch.watcher_revision,
                std::slice::from_ref(&normalized_file),
                &HashSet::new(),
            )
            .expect("apply exact watcher mutation");
        assert_eq!(result.upserted_paths, vec![normalized_file.clone()]);
        assert!(!result.reconciliation_required);
        assert!(db
            .complete_watcher_revision(&root_id, batch.watcher_revision)
            .expect("complete watcher revision"));

        let conn = db.conn().expect("db connection");
        let file_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE id = ?1",
                params![normalized_file],
                |row| row.get(0),
            )
            .expect("watcher file count");
        let seen_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM scan_seen", [], |row| row.get(0))
            .expect("watcher scan seen count");
        let generation: i64 = conn
            .query_row(
                "SELECT current_generation FROM scan_roots WHERE id = ?1",
                params![root_id],
                |row| row.get(0),
            )
            .expect("watcher generation");
        assert_eq!(file_count, 1);
        assert_eq!(seen_count, 0);
        assert_eq!(generation, 0);
        drop(conn);
        fs::remove_dir_all(root_path).expect("remove watcher fixture");
    }

    #[test]
    fn schema39_tagged_file_stays_browseable_after_upgrade_and_real_scan() {
        let db = test_db("schema39-library-compatibility");
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-schema39-library-{}",
            new_job_id("root")
        ));
        fs::create_dir_all(&root).expect("create migration root");
        let path = root.join("legacy.txt");
        fs::write(&path, b"unchanged legacy object").expect("write legacy file");
        // Keep the pre-existing Schema39 root in the active File Library
        // scope so the test exercises the same reader used by users.
        watcher_root_id(&db, &root);
        let normalized_path = normalize_scan_root_path(&path.to_string_lossy());
        let metadata = fs::metadata(&path).expect("read legacy metadata");
        let legacy_file = insert_request_from_metadata(normalized_path.clone(), &path, &metadata);
        db.insert_file(legacy_file)
            .expect("insert Schema39 legacy file");
        let tag = db
            .create_user_tag(crate::db::CreateUserTagRequest {
                display_name: "Legacy tag remains unassigned".to_string(),
                color_token: "blue".to_string(),
            })
            .expect("create legacy tag");
        db.mutate_file_user_tags(crate::db::MutateFileUserTagsRequest {
            selection: crate::db::LibrarySelectionV1::Explicit {
                file_ids: vec![normalized_path.clone()],
            },
            tag_ids: vec![tag.id.clone()],
            operation: crate::db::UserTagMutationOperation::Add,
            expected_count: Some(1),
        })
        .expect("tag Schema39 legacy file");

        let database_path = db.path().to_path_buf();
        {
            let conn = db.conn().expect("open Schema39 fixture connection");
            crate::db::drop_source_writer_schema_for_test(&conn);
            conn.execute_batch("PRAGMA user_version = 39;")
                .expect("restore the fixture to Schema39 shape");
        }
        drop(db);

        // Reopening executes the production 39 -> 41 migration path.
        let db = Database::open(&database_path).expect("upgrade Schema39 database");
        let root_text = normalize_scan_root_path(&root.to_string_lossy());
        let admission = db
            .admit_managed_scan(&request(&root_text, "schema39-first-scan"))
            .expect("admit first post-upgrade scan");
        let claimed = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim first post-upgrade scan");
        let entry = insert_request_from_metadata(
            normalized_path.clone(),
            &path,
            &fs::metadata(&path).expect("read unchanged file metadata"),
        );
        let persisted = db
            .persist_scan_batch(
                &claimed.dto.id,
                claimed.dto.revision,
                claimed.root_revision,
                claimed.session_revision,
                &ScanBatchInput {
                    entries: std::slice::from_ref(&entry),
                    errors: &[],
                    scanned_files: 1,
                    scanned_directories: 0,
                    processed_bytes: entry.size,
                    warnings: 0,
                },
            )
            .expect("publish the real upgraded file observation");
        let current_file_id: String = db
            .conn()
            .expect("inspect current upgraded file")
            .query_row(
                "SELECT id FROM files WHERE path = ?1 AND is_stale = 0",
                params![normalized_path],
                |row| row.get(0),
            )
            .expect("current upgraded file row");

        let query = || {
            db.query_file_library_v2(crate::db::queries::library::FileQueryRequestV2 {
                version: 2,
                request_id: "schema39-upgrade-visible-file".to_string(),
                query: crate::db::queries::library::FileQuerySpecV2 {
                    scope: crate::db::queries::library::FileLibraryScopeV2::AllEnabledRoots,
                    text: None,
                    filters: Default::default(),
                    sort: Default::default(),
                },
                page_size: 50,
                cursor: None,
            })
            .expect("query the active legacy File Library reader")
        };

        // Repeating the same Scan observation must update the current neutral
        // row in place and must not make the quarantined legacy facts visible.
        let repeated = db
            .persist_scan_batch(
                &persisted.dto.id,
                persisted.dto.revision,
                persisted.root_revision,
                persisted.session_revision,
                &ScanBatchInput {
                    entries: std::slice::from_ref(&entry),
                    errors: &[],
                    scanned_files: 1,
                    scanned_directories: 0,
                    processed_bytes: entry.size,
                    warnings: 0,
                },
            )
            .expect("repeat the upgraded file observation");
        let reconciled = db
            .reconcile_missing(
                &repeated.dto.id,
                repeated.dto.revision,
                repeated.root_revision,
                repeated.session_revision,
            )
            .expect("reconcile the repeated upgraded scan");
        assert!(reconciled.dto.stale_reconciliation_allowed);
        db.finalize_scan_run(
            &reconciled.dto.id,
            reconciled.dto.revision,
            reconciled.root_revision,
            reconciled.session_revision,
            &ScanFinalizeInput {
                terminal_status: "completed".to_string(),
                error_code: None,
                error_message: None,
                allow_stale_reconciliation: true,
                rule_recovery_succeeded: false,
            },
        )
        .expect("finish the reconciled compatibility scan");
        let first_query = query();
        let first_visible = first_query
            .files
            .iter()
            .find(|file| file.id == current_file_id)
            .expect("unchanged upgraded file remains visible");
        assert_ne!(first_visible.id, normalized_path);
        assert_eq!(first_visible.name, "legacy.txt");
        assert_eq!(first_visible.tag_count, 0);
        assert!(first_visible.tags.is_empty());
        assert!(first_visible.requires_review);
        let second_query = query();
        let visible_rows = second_query
            .files
            .iter()
            .filter(|file| file.id == current_file_id)
            .collect::<Vec<_>>();
        assert_eq!(visible_rows.len(), 1);
        assert_eq!(visible_rows[0].id, first_visible.id);
        assert_eq!(visible_rows[0].tag_count, 0);
        assert!(visible_rows[0].tags.is_empty());
        assert!(repeated.dto.revision > persisted.dto.revision);

        let conn = db.conn().expect("inspect retained legacy facts");
        let (legacy_stale, legacy_tag_count): (i64, i64) = conn
            .query_row(
                "SELECT is_stale, (SELECT COUNT(*) FROM file_user_tags WHERE file_id = files.id AND tag_id = ?2) FROM files WHERE id = ?1",
                params![normalized_path, tag.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read quarantined Schema39 facts");
        assert_eq!(legacy_stale, 1);
        assert_eq!(legacy_tag_count, 1);
        let unresolved_legacy_claims: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM library_legacy_attributions WHERE legacy_file_id = ?1 AND claim_state = 'unresolved' AND source_uid IS NULL",
                params![normalized_path],
                |row| row.get(0),
            )
            .expect("check legacy attribution stayed unresolved");
        assert_eq!(unresolved_legacy_claims, 1);
        drop(conn);
        fs::remove_dir_all(root).expect("remove Schema39 migration fixture");
    }

    #[test]
    fn source_uid_replacement_keeps_old_tag_and_does_not_attach_it_to_new_object() {
        let db = test_db("source-replacement-keeps-tag");
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-source-replacement-{}",
            new_job_id("root")
        ));
        fs::create_dir_all(&root).expect("create root");
        let path = root.join("kept.txt");
        let moved_old_object = root.join("old-object-preserved.txt");
        fs::write(&path, b"first physical object").expect("write original object");
        let root_id = watcher_root_id(&db, &root);
        let normalized_path = normalize_scan_root_path(&path.to_string_lossy());

        let initial_revision = db
            .begin_watcher_revision(&root_id)
            .expect("begin initial watcher revision")
            .expect("enabled initial root")
            .watcher_revision;
        db.apply_watcher_exact_mutations(
            &root_id,
            initial_revision,
            std::slice::from_ref(&normalized_path),
            &HashSet::new(),
        )
        .expect("publish original object");
        assert!(db
            .complete_watcher_revision(&root_id, initial_revision)
            .expect("complete initial watcher revision"));
        // Establish the ordinary managed-scan lifecycle before testing the
        // File Library reader. Watcher publication alone leaves a freshly
        // registered root unknown, which is intentionally not browseable.
        let root_text = normalize_scan_root_path(&root.to_string_lossy());
        let admission = db
            .admit_managed_scan(&request(&root_text, "replacement-initial-scan"))
            .expect("admit initial managed scan");
        let claimed = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim initial managed scan");
        let entry = insert_request_from_metadata(
            normalized_path.clone(),
            &path,
            &fs::metadata(&path).expect("read original metadata"),
        );
        let persisted = db
            .persist_scan_batch(
                &claimed.dto.id,
                claimed.dto.revision,
                claimed.root_revision,
                claimed.session_revision,
                &ScanBatchInput {
                    entries: std::slice::from_ref(&entry),
                    errors: &[],
                    scanned_files: 1,
                    scanned_directories: 0,
                    processed_bytes: entry.size,
                    warnings: 0,
                },
            )
            .expect("publish original through Managed Scan");
        let reconciled = db
            .reconcile_missing(
                &persisted.dto.id,
                persisted.dto.revision,
                persisted.root_revision,
                persisted.session_revision,
            )
            .expect("reconcile initial managed scan");
        db.finalize_scan_run(
            &reconciled.dto.id,
            reconciled.dto.revision,
            reconciled.root_revision,
            reconciled.session_revision,
            &ScanFinalizeInput {
                terminal_status: "completed".to_string(),
                error_code: None,
                error_message: None,
                allow_stale_reconciliation: true,
                rule_recovery_succeeded: false,
            },
        )
        .expect("finalize initial managed scan");
        let original_file_id: String = db
            .conn()
            .expect("inspect original library row")
            .query_row(
                "SELECT id FROM files WHERE path = ?1 AND is_stale = 0",
                params![normalized_path],
                |row| row.get(0),
            )
            .expect("original library row");
        let original_uid: String = db
            .conn()
            .expect("open original uid connection")
            .query_row(
                "SELECT source_uid FROM library_source_root_claims WHERE root_id = ?1 AND observed_path = ?2 AND claim_state = 'present'",
                params![root_id, normalized_path],
                |row| row.get(0),
            )
            .expect("original source uid");
        let tag = db
            .create_user_tag(crate::db::CreateUserTagRequest {
                display_name: "Keep with original object".to_string(),
                color_token: "blue".to_string(),
            })
            .expect("create retained tag");
        let tagged = db
            .mutate_file_user_tags(crate::db::MutateFileUserTagsRequest {
                selection: crate::db::LibrarySelectionV1::Explicit {
                    file_ids: vec![original_file_id.clone()],
                },
                tag_ids: vec![tag.id.clone()],
                operation: crate::db::UserTagMutationOperation::Add,
                expected_count: Some(1),
            })
            .expect("tag original object");
        assert_eq!(tagged.applied_count, 1);

        fs::rename(&path, &moved_old_object).expect("preserve original physical object");
        fs::write(&path, b"replacement physical object").expect("write replacement object");
        let replacement_revision = db
            .begin_watcher_revision(&root_id)
            .expect("begin replacement watcher revision")
            .expect("enabled replacement root")
            .watcher_revision;
        db.apply_watcher_exact_mutations(
            &root_id,
            replacement_revision,
            std::slice::from_ref(&normalized_path),
            &HashSet::new(),
        )
        .expect("observe replacement object");
        assert!(db
            .complete_watcher_revision(&root_id, replacement_revision)
            .expect("complete replacement watcher revision"));

        let repeated_revision = db
            .begin_watcher_revision(&root_id)
            .expect("begin repeated replacement revision")
            .expect("enabled repeated replacement root")
            .watcher_revision;
        db.apply_watcher_exact_mutations(
            &root_id,
            repeated_revision,
            std::slice::from_ref(&normalized_path),
            &HashSet::new(),
        )
        .expect("repeat replacement observation");
        assert!(db
            .complete_watcher_revision(&root_id, repeated_revision)
            .expect("complete repeated watcher revision"));

        let current_replacement_id: String = db
            .conn()
            .expect("inspect current replacement row")
            .query_row(
                "SELECT id FROM files WHERE path = ?1 AND is_stale = 0",
                params![normalized_path],
                |row| row.get(0),
            )
            .expect("current replacement row");

        let library = db
            .query_file_library_v2(crate::db::queries::library::FileQueryRequestV2 {
                version: 2,
                request_id: "replacement-neutral-row".to_string(),
                query: crate::db::queries::library::FileQuerySpecV2 {
                    scope: crate::db::queries::library::FileLibraryScopeV2::AllEnabledRoots,
                    text: None,
                    filters: Default::default(),
                    sort: Default::default(),
                },
                page_size: 50,
                cursor: None,
            })
            .expect("query replacement through the active File Library reader");
        let visible_replacement = library
            .files
            .iter()
            .find(|file| file.id == current_replacement_id)
            .expect("replacement remains browseable after repeated observation");
        assert_eq!(visible_replacement.name, "kept.txt");
        assert_eq!(visible_replacement.tag_count, 0);
        assert!(visible_replacement.tags.is_empty());
        assert!(visible_replacement.requires_review);

        let conn = db.conn().expect("inspect replacement state");
        let (stale, retained_tag_count): (i64, i64) = conn
            .query_row(
                "SELECT file.is_stale, (SELECT COUNT(*) FROM file_user_tags WHERE file_id = file.id AND tag_id = ?2) FROM files AS file WHERE file.id = ?1",
                params![original_file_id, tag.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read old user facts");
        let replacement_uid: String = conn
            .query_row(
                "SELECT source_uid FROM library_source_root_claims WHERE root_id = ?1 AND observed_path = ?2 AND claim_state = 'present'",
                params![root_id, normalized_path],
                |row| row.get(0),
            )
            .expect("replacement source uid");
        let replacement_attribution_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM library_legacy_attributions WHERE legacy_file_id = ?1 AND source_uid = ?2 AND claim_state IN ('verified','user_confirmed')",
                params![normalized_path, replacement_uid],
                |row| row.get(0),
            )
            .expect("count replacement facts attribution");
        assert_eq!(stale, 1, "legacy row must remain non-actionable");
        assert_eq!(retained_tag_count, 1, "old tag remains on the old row");
        assert_ne!(
            replacement_uid, original_uid,
            "replacement receives a new UID"
        );
        assert_eq!(
            replacement_attribution_count, 0,
            "old tag is not attributed to replacement"
        );
        let neutral_id: String = conn
            .query_row(
                "SELECT id FROM files WHERE path = ?1 AND is_stale = 0",
                params![normalized_path],
                |row| row.get(0),
            )
            .expect("read current neutral row id");
        assert_eq!(neutral_id, visible_replacement.id);
        let (suggested_action, requires_confirmation, classification_status): (
            String,
            i64,
            String,
        ) = conn
            .query_row(
                "SELECT suggested_action, requires_confirmation, classification_status FROM files WHERE id = ?1",
                params![neutral_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read neutral row authority defaults");
        assert_eq!(suggested_action, "Keep");
        assert_eq!(requires_confirmation, 1);
        assert_eq!(classification_status, CLASSIFICATION_STATUS_UNCLASSIFIED);
        drop(conn);
        fs::remove_dir_all(root).expect("remove replacement fixture");
    }

    #[test]
    fn source_uid_same_object_rename_keeps_the_physical_uid() {
        let db = test_db("source-same-object-rename");
        let root =
            std::env::temp_dir().join(format!("zen-canvas-source-rename-{}", new_job_id("root")));
        fs::create_dir_all(&root).expect("create root");
        let old_path = root.join("before.txt");
        let new_path = root.join("after.txt");
        fs::write(&old_path, b"same physical object").expect("write rename fixture");
        let root_id = watcher_root_id(&db, &root);
        let normalized_old = normalize_scan_root_path(&old_path.to_string_lossy());
        let normalized_new = normalize_scan_root_path(&new_path.to_string_lossy());
        let first_revision = db
            .begin_watcher_revision(&root_id)
            .expect("begin initial rename revision")
            .expect("enabled initial rename root")
            .watcher_revision;
        db.apply_watcher_exact_mutations(
            &root_id,
            first_revision,
            std::slice::from_ref(&normalized_old),
            &HashSet::new(),
        )
        .expect("publish original rename path");
        let original_uid: String = db
            .conn()
            .expect("open original rename uid connection")
            .query_row(
                "SELECT source_uid FROM library_source_root_claims WHERE root_id = ?1 AND observed_path = ?2 AND claim_state = 'present'",
                params![root_id, normalized_old],
                |row| row.get(0),
            )
            .expect("read original rename uid");

        fs::rename(&old_path, &new_path).expect("rename same physical object");
        let rename_revision = db
            .begin_watcher_revision(&root_id)
            .expect("begin rename watcher revision")
            .expect("enabled rename root")
            .watcher_revision;
        db.apply_watcher_exact_mutations(
            &root_id,
            rename_revision,
            &[normalized_old.clone(), normalized_new.clone()],
            &HashSet::new(),
        )
        .expect("publish both rename paths");

        let conn = db.conn().expect("inspect rename claims");
        let (new_uid, new_state): (String, String) = conn
            .query_row(
                "SELECT source_uid, claim_state FROM library_source_root_claims WHERE root_id = ?1 AND observed_path = ?2",
                params![root_id, normalized_new],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read new rename claim");
        let old_state: String = conn
            .query_row(
                "SELECT claim_state FROM library_source_root_claims WHERE root_id = ?1 AND source_uid = ?2 AND observed_path = ?3",
                params![root_id, original_uid, normalized_old],
                |row| row.get(0),
            )
            .expect("read old rename claim");
        assert_eq!(new_uid, original_uid, "same object keeps its SourceUID");
        assert_eq!(new_state, "present");
        assert_eq!(old_state, "absent");
        drop(conn);
        fs::remove_dir_all(root).expect("remove rename fixture");
    }

    #[test]
    fn overlapping_root_absence_does_not_stale_a_surviving_root_claim() {
        let db = test_db("source-overlapping-roots");
        let parent =
            std::env::temp_dir().join(format!("zen-canvas-source-overlap-{}", new_job_id("root")));
        let child = parent.join("child");
        fs::create_dir_all(&child).expect("create overlapping roots");
        let path = child.join("shared.txt");
        fs::write(&path, b"shared path in two roots").expect("write shared file");
        let roots = sync_watcher_roots(&db, &[parent.clone(), child.clone()]);
        let parent_id = roots
            .get(&normalize_scan_root_path(&parent.to_string_lossy()))
            .expect("parent root")
            .clone();
        let child_id = roots
            .get(&normalize_scan_root_path(&child.to_string_lossy()))
            .expect("child root")
            .clone();
        let normalized_path = normalize_scan_root_path(&path.to_string_lossy());
        for root_id in [&parent_id, &child_id] {
            let revision = db
                .begin_watcher_revision(root_id)
                .expect("begin overlapping-root observation")
                .expect("enabled overlapping root")
                .watcher_revision;
            db.apply_watcher_exact_mutations(
                root_id,
                revision,
                std::slice::from_ref(&normalized_path),
                &HashSet::new(),
            )
            .expect("observe shared path in root");
        }
        fs::remove_file(&path).expect("remove file from both roots");
        let parent_revision = db
            .begin_watcher_revision(&parent_id)
            .expect("begin parent absence revision")
            .expect("enabled parent root")
            .watcher_revision;
        db.apply_watcher_exact_mutations(
            &parent_id,
            parent_revision,
            std::slice::from_ref(&normalized_path),
            &HashSet::new(),
        )
        .expect("record parent-only absence");

        let conn = db.conn().expect("inspect overlapping root claims");
        let file_stale: i64 = conn
            .query_row(
                "SELECT is_stale FROM files WHERE id = ?1",
                params![normalized_path],
                |row| row.get(0),
            )
            .expect("read shared file visibility");
        let child_claim: String = conn
            .query_row(
                "SELECT claim_state FROM library_source_root_claims WHERE root_id = ?1 AND observed_path = ?2",
                params![child_id, normalized_path],
                |row| row.get(0),
            )
            .expect("read surviving child claim");
        assert_eq!(
            file_stale, 0,
            "one root's absence cannot hide the surviving claim"
        );
        assert_eq!(child_claim, "present");
        drop(conn);
        fs::remove_dir_all(parent).expect("remove overlapping roots fixture");
    }

    #[test]
    fn stale_watcher_revision_cannot_publish_source_claims() {
        let db = test_db("source-stale-watcher-revision");
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-source-stale-watcher-{}",
            new_job_id("root")
        ));
        fs::create_dir_all(&root).expect("create stale watcher root");
        let path = root.join("event.txt");
        fs::write(&path, b"stale watcher event").expect("write stale watcher file");
        let normalized_path = normalize_scan_root_path(&path.to_string_lossy());
        let root_id = watcher_root_id(&db, &root);
        let stale_revision = db
            .begin_watcher_revision(&root_id)
            .expect("begin stale event revision")
            .expect("enabled stale root")
            .watcher_revision;
        let current_revision = db
            .begin_watcher_revision(&root_id)
            .expect("advance watcher revision")
            .expect("enabled current root")
            .watcher_revision;
        assert!(current_revision > stale_revision);
        assert!(db
            .apply_watcher_exact_mutations(
                &root_id,
                stale_revision,
                std::slice::from_ref(&normalized_path),
                &HashSet::new(),
            )
            .is_err());
        let conn = db.conn().expect("inspect stale watcher result");
        let stale_claim_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM library_source_root_claims WHERE root_id = ?1 AND observed_path = ?2",
                params![root_id, normalized_path],
                |row| row.get(0),
            )
            .expect("count stale claims");
        let file_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE path = ?1",
                params![normalized_path],
                |row| row.get(0),
            )
            .expect("count stale indexed rows");
        assert_eq!(stale_claim_count, 0);
        assert_eq!(file_count, 0);
        drop(conn);
        fs::remove_dir_all(root).expect("remove stale watcher fixture");
    }

    #[test]
    fn moved_directory_old_descendants_are_removed_only_by_full_reconciliation() {
        let db = test_db("watcher-cross-root-directory-rename");
        let old_root_path = std::env::temp_dir().join(format!(
            "zen-canvas-watcher-cross-root-old-{}",
            new_job_id("root")
        ));
        let new_root_path = std::env::temp_dir().join(format!(
            "zen-canvas-watcher-cross-root-new-{}",
            new_job_id("root")
        ));
        let old_directory = old_root_path.join("old-folder");
        fs::create_dir_all(&old_directory).expect("create old directory");
        fs::create_dir_all(&new_root_path).expect("create new root");
        let old_root = normalize_scan_root_path(&old_root_path.to_string_lossy());
        let new_root = normalize_scan_root_path(&new_root_path.to_string_lossy());
        let old_directory = normalize_scan_root_path(&old_directory.to_string_lossy());
        let old_descendant = format!("{old_directory}/child.txt");

        db.insert_file(InsertFileRequest {
            id: old_descendant.clone(),
            path: old_descendant.clone(),
            name: "child.txt".to_string(),
            extension: "txt".to_string(),
            size: 1,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("seed old directory descendant");
        let conn = db.conn().expect("open old descendant fixture");
        conn.execute(
            "UPDATE files SET last_seen_at = 0 WHERE path = ?1",
            params![old_descendant],
        )
        .expect("age old descendant before reconciliation");
        drop(conn);
        let watcher_roots = vec![
            crate::settings::ScanRootSetting {
                id: "cross-root-old".to_string(),
                path: old_root.clone(),
                label: "Old root".to_string(),
                enabled: true,
                created_at: "2026-07-27T00:00:00.000Z".to_string(),
            },
            crate::settings::ScanRootSetting {
                id: "cross-root-new".to_string(),
                path: new_root,
                label: "New root".to_string(),
                enabled: true,
                created_at: "2026-07-27T00:00:00.000Z".to_string(),
            },
        ];
        persist_default_scan_folders(&db, watcher_roots.clone());
        db.sync_file_library_watcher_roots(&watcher_roots)
            .expect("sync cross-root watcher roots");
        fs::remove_dir_all(&old_directory).expect("simulate move out of old root");

        let old_root_id = db
            .list_watcher_root_configs()
            .expect("watcher configs")
            .into_iter()
            .find(|root_config| root_config.path == old_root)
            .expect("old managed root")
            .id;
        let batch = db
            .begin_watcher_revision(&old_root_id)
            .expect("begin old root rename revision")
            .expect("old root revision");
        let directory_paths = std::iter::once(old_directory.clone()).collect::<HashSet<_>>();
        let mutation = db
            .apply_watcher_exact_mutations(
                &old_root_id,
                batch.watcher_revision,
                std::slice::from_ref(&old_directory),
                &directory_paths,
            )
            .expect("apply old side of directory rename");
        assert!(mutation.reconciliation_required);
        assert!(db
            .mark_watcher_reconciliation(
                &old_root_id,
                "watcher_directory_rename",
                "old and new directory sides require full reconciliation",
            )
            .expect("mark old side reconciliation"));
        assert_eq!(batch.watcher_revision, 1);

        let admission = db
            .admit_managed_scan(&request(&old_root, "cross-root-directory-reconcile"))
            .expect("admit old root reconciliation");
        let run = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim old root reconciliation");
        let reconciled = db
            .reconcile_missing(
                &run.dto.id,
                run.dto.revision,
                run.root_revision,
                run.session_revision,
            )
            .expect("reconcile old directory descendants");
        assert!(reconciled.dto.stale_reconciliation_allowed);
        db.finalize_scan_run(
            &reconciled.dto.id,
            reconciled.dto.revision,
            reconciled.root_revision,
            reconciled.session_revision,
            &ScanFinalizeInput {
                terminal_status: "completed".to_string(),
                error_code: None,
                error_message: None,
                allow_stale_reconciliation: true,
                rule_recovery_succeeded: false,
            },
        )
        .expect("finalize old directory reconciliation");

        let conn = db.conn().expect("inspect old descendant");
        let stale: i64 = conn
            .query_row(
                "SELECT is_stale FROM files WHERE path = ?1",
                params![old_descendant],
                |row| row.get(0),
            )
            .expect("old descendant stale state");
        assert_eq!(stale, 1);
        drop(conn);
        fs::remove_dir_all(old_root_path).expect("remove old root fixture");
        fs::remove_dir_all(new_root_path).expect("remove new root fixture");
    }

    #[test]
    fn watcher_revision_change_during_scan_skips_stale_and_finalizes_with_warning() {
        let db = test_db("watcher-active-scan");
        let root = format!("/tmp/zen-canvas-watcher-active-{}", new_job_id("root"));
        let admission = db
            .admit_managed_scan(&request(&root, "watcher-active-scan-1"))
            .expect("admit scan");
        let claimed = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim scan");
        db.begin_watcher_revision(&claimed.dto.scan_root_id)
            .expect("begin watcher event")
            .expect("watcher root");
        let latest = db
            .get_scan_run_record(&claimed.dto.id)
            .expect("reload scan after watcher event");
        let reconciled = db
            .reconcile_missing(
                &latest.dto.id,
                latest.dto.revision,
                latest.root_revision,
                latest.session_revision,
            )
            .expect("safe reconcile gate");
        assert!(!reconciled.dto.stale_reconciliation_allowed);
        db.begin_watcher_revision(&claimed.dto.scan_root_id)
            .expect("second watcher event")
            .expect("watcher root for second event");
        let finalization = db
            .finalize_scan_run(
                &reconciled.dto.id,
                reconciled.dto.revision,
                reconciled.root_revision,
                reconciled.session_revision,
                &ScanFinalizeInput {
                    terminal_status: "completed".to_string(),
                    error_code: None,
                    error_message: None,
                    allow_stale_reconciliation: true,
                    rule_recovery_succeeded: false,
                },
            )
            .expect("warning finalization");
        assert_eq!(finalization.run.dto.status, "completed_with_warnings");
        let root_health = db
            .get_scan_root_health(Some(&claimed.dto.scan_root_id), None)
            .expect("root health");
        assert!(root_health.needs_reconciliation);
        assert_eq!(root_health.health_status, "reconciliation_required");
        assert_eq!(root_health.last_successful_generation, None);
    }

    #[test]
    fn successful_full_scan_advances_watcher_applied_revision() {
        let db = test_db("watcher-full-reconcile");
        let root = format!("/tmp/zen-canvas-watcher-full-{}", new_job_id("root"));
        let admission = db
            .admit_managed_scan(&request(&root, "watcher-full-reconcile-1"))
            .expect("admit scan");
        let root_id = admission.runs[0].scan_root_id.clone();
        db.begin_watcher_revision(&root_id)
            .expect("begin watcher gap")
            .expect("watcher root");
        let claimed = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim reconciliation scan");
        assert_eq!(claimed.dto.watcher_revision_at_start, 1);
        let reconciled = db
            .reconcile_missing(
                &claimed.dto.id,
                claimed.dto.revision,
                claimed.root_revision,
                claimed.session_revision,
            )
            .expect("reconcile missing files");
        let finalization = db
            .finalize_scan_run(
                &reconciled.dto.id,
                reconciled.dto.revision,
                reconciled.root_revision,
                reconciled.session_revision,
                &ScanFinalizeInput {
                    terminal_status: "completed".to_string(),
                    error_code: None,
                    error_message: None,
                    allow_stale_reconciliation: true,
                    rule_recovery_succeeded: false,
                },
            )
            .expect("finalize reconciliation scan");
        let root_health = db
            .get_scan_root_health(Some(&root_id), None)
            .expect("root health");

        assert_eq!(finalization.run.dto.status, "completed");
        assert_eq!(root_health.watcher_revision, 1);
        assert_eq!(root_health.watcher_applied_revision, 1);
        assert!(!root_health.needs_reconciliation);
        assert_eq!(root_health.health_status, "healthy");
    }

    #[test]
    fn watcher_rule_failure_survives_scan_until_rule_recovery_succeeds() {
        let db = test_db("watcher-rule-failure-recovery");
        let path = db.path().to_path_buf();
        let root = format!(
            "/tmp/zen-canvas-watcher-rule-failure-{}",
            new_job_id("root")
        );
        let first = db
            .admit_managed_scan(&request(&root, "watcher-rule-failure-first"))
            .expect("admit first recovery run");
        let root_id = first.runs[0].scan_root_id.clone();
        db.mark_watcher_reconciliation(
            &root_id,
            "watcher_rule_failure",
            "rule execution failed for a newly observed file",
        )
        .expect("persist rule failure");
        let first_run = db
            .claim_queued_scan_run(&first.runs[0].id)
            .expect("claim first recovery run");
        let first_final = db
            .finalize_scan_run(
                &first_run.dto.id,
                first_run.dto.revision,
                first_run.root_revision,
                first_run.session_revision,
                &ScanFinalizeInput {
                    terminal_status: "completed_with_warnings".to_string(),
                    error_code: Some("watcher_rule_failure".to_string()),
                    error_message: Some("bounded rule recovery did not succeed".to_string()),
                    allow_stale_reconciliation: false,
                    rule_recovery_succeeded: false,
                },
            )
            .expect("finalize unresolved rule failure");
        assert_eq!(first_final.run.dto.status, "completed_with_warnings");
        let failed_health = db
            .get_scan_root_health(Some(&root_id), None)
            .expect("failed rule health");
        assert!(failed_health.needs_reconciliation);
        assert_eq!(
            failed_health.watcher_last_error_code.as_deref(),
            Some("watcher_rule_failure")
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                first_final.run.dto.result_json.as_deref().unwrap_or("{}")
            )
            .expect("result json")["ruleFailurePending"],
            true
        );
        drop(db);

        let db = Database::open(&path).expect("reopen rule recovery database");
        let persisted_health = db
            .get_scan_root_health(Some(&root_id), None)
            .expect("persisted rule health");
        assert!(persisted_health.needs_reconciliation);
        assert_eq!(
            persisted_health.watcher_last_error_code.as_deref(),
            Some("watcher_rule_failure")
        );

        let second = db
            .admit_managed_scan(&request(&root, "watcher-rule-failure-recovered"))
            .expect("admit recovery retry");
        let second_run = db
            .claim_queued_scan_run(&second.runs[0].id)
            .expect("claim recovery retry");
        let second_final = db
            .finalize_scan_run(
                &second_run.dto.id,
                second_run.dto.revision,
                second_run.root_revision,
                second_run.session_revision,
                &ScanFinalizeInput {
                    terminal_status: "completed".to_string(),
                    error_code: None,
                    error_message: None,
                    allow_stale_reconciliation: false,
                    rule_recovery_succeeded: true,
                },
            )
            .expect("finalize successful rule recovery");
        assert_eq!(second_final.run.dto.status, "completed");
        let recovered_health = db
            .get_scan_root_health(Some(&root_id), None)
            .expect("recovered rule health");
        assert!(!recovered_health.needs_reconciliation);
        assert!(recovered_health.watcher_last_error_code.is_none());
        assert_eq!(recovered_health.health_status, "healthy");
    }

    #[test]
    fn schema_26_upgrade_creates_ledger_without_fabricating_observations() {
        let path = {
            let sequence = TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
            std::env::temp_dir().join(format!(
                "zen-canvas-scan-schema-26-{}-{sequence}.sqlite3",
                std::process::id()
            ))
        };
        let db = Database::open(&path).expect("create current fixture");
        db.insert_file(InsertFileRequest {
            id: "/tmp/legacy.txt".to_string(),
            path: "/tmp/legacy.txt".to_string(),
            name: "legacy.txt".to_string(),
            extension: "txt".to_string(),
            size: 1,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("insert legacy file");
        drop(db);
        let conn = Connection::open(&path).expect("open fixture");
        crate::db::drop_source_writer_schema_for_test(&conn);
        conn.execute_batch(
            r#"
            DROP TABLE scan_seen;
            DROP TABLE scan_run_errors;
            DROP TABLE scan_session_roots;
            DROP TABLE scan_runs;
            DROP TABLE scan_sessions;
            DROP TABLE scan_roots;
            DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; PRAGMA user_version = 26;
            "#,
        )
        .expect("downgrade ledger tables for schema 26 fixture");
        drop(conn);

        let migrated = Database::open(&path).expect("migrate schema 26 fixture");
        let conn = migrated.conn().expect("inspect migrated fixture");
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("schema version");
        let file_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE id = '/tmp/legacy.txt'",
                [],
                |row| row.get(0),
            )
            .expect("legacy file");
        let seen_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM scan_seen", [], |row| row.get(0))
            .expect("scan seen count");
        let watcher_defaults: (i64, i64) = conn
            .query_row(
                "SELECT COALESCE(MAX(watcher_revision), 0), COALESCE(MAX(watcher_applied_revision), 0) FROM scan_roots",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("watcher defaults");
        assert_eq!(version, 41);
        assert_eq!(file_count, 1);
        assert_eq!(seen_count, 0);
        assert_eq!(watcher_defaults, (0, 0));
    }
}
