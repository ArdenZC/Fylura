//! Task-owned native MFT qualification for the disposable filename-first path.
//! The caller supplies an isolated profile and a task-created NTFS fixture.

use super::mft::{open_volume, query_journal};
use super::service::{IndexServiceCommand, IndexServiceRequest};
use super::WindowsGlobalIndexProvider;
use crate::db::{Database, DbError, InsertFileRequest};
use crate::global_index::coordinator::{GlobalIndexProvider, GlobalIndexSink};
use crate::global_index::models::{
    normalize_path, unix_now, INDEX_STATUS_READY, SCOPE_AUTHORIZATION_AUTHORIZED,
};
use crate::global_index::GlobalSourceDescriptor;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::CloseHandle;

/// Disposable qualification probe that sends the desktop process's mode to
/// the SCM-installed service over the production named-pipe protocol.
pub fn run_service_mode_probe(args: &[String]) -> Result<(), Box<dyn Error>> {
    if args.len() != 3 {
        return Err("usage: --global-index-filename-first-service-mode-qa <report-path> <profile-root> <command>".into());
    }
    let report_path = PathBuf::from(&args[0]);
    let profile_root = PathBuf::from(&args[1]);
    if !report_path.is_absolute() || !profile_root.is_absolute() {
        return Err("service mode QA report and profile paths must be absolute".into());
    }
    validate_task_profile_boundary(&profile_root)?;
    let db = Database::open(profile_root.join("fylura.sqlite3"))?;
    let enabled_sources = db
        .list_global_volumes()?
        .into_iter()
        .filter(|volume| volume.enabled)
        .collect::<Vec<_>>();
    if enabled_sources.len() != 1 {
        return Err("service mode QA requires exactly one enabled task-owned source".into());
    }
    let volume = enabled_sources
        .into_iter()
        .next()
        .expect("one enabled source");
    if volume.scope_authorization_state != SCOPE_AUTHORIZATION_AUTHORIZED {
        return Err("service mode QA source must retain explicit index authorization".into());
    }
    let source_id = volume.id.clone();
    validate_task_profile_source(&source_id, &db)?;
    let command_name = args[2].as_str();
    let (command, source) = match command_name {
        "status" => (IndexServiceCommand::Status, None),
        "pause" => (IndexServiceCommand::Pause, None),
        "start_initial_index" => (
            IndexServiceCommand::StartInitialIndex {
                source_id: source_id.clone(),
            },
            Some(volume.clone()),
        ),
        "rebuild" => (
            IndexServiceCommand::Rebuild {
                source_id: source_id.clone(),
            },
            Some(volume.clone()),
        ),
        "resume_incremental_sync" => (
            IndexServiceCommand::ResumeIncrementalSync {
                source_id: source_id.clone(),
            },
            Some(volume.clone()),
        ),
        _ => return Err(format!("unsupported service mode QA command: {command_name}").into()),
    };
    let before_volume = db
        .get_global_volume(&source_id)?
        .ok_or("task-owned QA source disappeared before request")?;
    let (before_total_entries, before_active_entries) =
        db.native_qa_global_entry_counts(&source_id)?;
    let request = IndexServiceRequest::new(command, source);
    let response = super::service::call_index_service(&request)
        .map_err(|error| format!("installed Global Index Service mode probe failed: {error}"))?;
    let after_volume = db
        .get_global_volume(&source_id)?
        .ok_or("task-owned QA source disappeared after request")?;
    let (after_total_entries, after_active_entries) =
        db.native_qa_global_entry_counts(&source_id)?;
    let source_snapshot_unchanged = before_volume == after_volume;
    let entry_counts_unchanged = before_total_entries == after_total_entries
        && before_active_entries == after_active_entries;
    let report = json!({
        "command": command_name,
        "desktopFilenameFirstEnabled": request.filename_first_enabled,
        "responseOk": response.ok,
        "errorCode": response.error_code,
        "serviceStatus": response.status,
        "protocolVersion": response.protocol_version,
        "sourceSnapshotUnchanged": source_snapshot_unchanged,
        "entryCountsUnchanged": entry_counts_unchanged,
        "sideEffectFree": source_snapshot_unchanged && entry_counts_unchanged,
        "entryCountBefore": before_total_entries,
        "activeEntryCountBefore": before_active_entries,
        "entryCountAfter": after_total_entries,
        "activeEntryCountAfter": after_active_entries,
    });
    std::fs::write(report_path, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}

#[derive(Clone, Copy)]
struct DiagnosticScenario {
    name: &'static str,
    search_polling: bool,
    ordinary_writes: bool,
}

fn diagnostic_scenario() -> Result<DiagnosticScenario, Box<dyn Error>> {
    let name = std::env::var("ZC_GLOBAL_INDEX_QA_SCENARIO").unwrap_or_else(|_| "D".into());
    match name.as_str() {
        "A" => Ok(DiagnosticScenario {
            name: "A",
            search_polling: false,
            ordinary_writes: false,
        }),
        "B" => Ok(DiagnosticScenario {
            name: "B",
            search_polling: true,
            ordinary_writes: false,
        }),
        "C" => Ok(DiagnosticScenario {
            name: "C",
            search_polling: false,
            ordinary_writes: true,
        }),
        "D" => Ok(DiagnosticScenario {
            name: "D",
            search_polling: true,
            ordinary_writes: true,
        }),
        _ => Err(format!("unknown filename-first QA scenario: {name}").into()),
    }
}

fn verify_task_profile(profile: &Path, source_id: &str) -> Result<Database, Box<dyn Error>> {
    validate_task_profile_boundary(profile)?;
    let db = Database::open(profile.join("fylura.sqlite3"))?;
    validate_task_profile_source(source_id, &db)?;
    Ok(db)
}

fn validate_task_profile_boundary(profile: &Path) -> Result<(), Box<dyn Error>> {
    let task = profile.parent().ok_or("missing task profile boundary")?;
    let runner = std::env::var_os("RUNNER_TEMP").ok_or("RUNNER_TEMP required")?;
    if !profile.is_absolute()
        || !task.starts_with(Path::new(&runner))
        || !task
            .join("qualification-volume-created-by-task.txt")
            .is_file()
    {
        return Err(
            "native filename qualification requires a task-owned NTFS VHD and isolated profile"
                .into(),
        );
    }
    Ok(())
}

fn validate_task_profile_source(source_id: &str, db: &Database) -> Result<(), Box<dyn Error>> {
    let enabled = db
        .list_global_volumes()?
        .into_iter()
        .filter(|volume| volume.enabled)
        .collect::<Vec<_>>();
    if enabled.len() != 1
        || enabled[0].id != source_id
        || enabled[0].provider != crate::global_index::models::PROVIDER_WINDOWS_MFT_USN
        || !enabled[0].filesystem_type.eq_ignore_ascii_case("NTFS")
    {
        return Err(
            "filename qualification must use exactly one explicitly authorized fixed NTFS source"
                .into(),
        );
    }
    Ok(())
}

fn descriptor(db: &Database, source_id: &str) -> Result<GlobalSourceDescriptor, Box<dyn Error>> {
    Ok(GlobalSourceDescriptor {
        volume: db
            .get_global_volume(source_id)?
            .ok_or("authorized qualification source disappeared")?,
    })
}

fn live_result_visible(
    db: &Database,
    query: &str,
    expected_path: &str,
) -> Result<bool, Box<dyn Error>> {
    let Some(result) = db
        .search_global_entries(query, 50, 0)?
        .into_iter()
        .find(|result| normalize_path(&result.path) == expected_path)
    else {
        return Ok(false);
    };
    Ok(crate::global_index::commands::revalidated_global_entry(db, &result.id).is_ok())
}

fn durable_source_fts_ready(
    conn: &Connection,
    source_id: &str,
    expected_fixture_file_count: usize,
) -> rusqlite::Result<bool> {
    let _transaction_trace = crate::global_index::qa_trace::TransactionTrace::start(
        "read",
        "qa.durable_fts_readiness_probe",
    );
    let result = conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM global_volumes
            WHERE id=?1 AND entry_count >= ?2
              AND fts_bulk_build=0 AND fts_bulk_cursor=0
        )",
        rusqlite::params![source_id, expected_fixture_file_count as i64],
        |row| row.get(0),
    );
    if let Err(error) = &result {
        crate::global_index::qa_trace::record_sqlite_error("qa.durable_fts_readiness_probe", error);
    }
    result
}

fn durable_fts_marker_visible(
    conn: &Connection,
    source_id: &str,
    complete_query: &str,
    complete_expected_path: &str,
) -> rusqlite::Result<bool> {
    let _transaction_trace = crate::global_index::qa_trace::TransactionTrace::start(
        "read",
        "qa.durable_fts_marker_probe",
    );
    let result = conn.query_row(
        "SELECT EXISTS(
            SELECT 1
            FROM global_entries_fts
            JOIN global_entries ge ON ge.rowid=global_entries_fts.rowid
            WHERE global_entries_fts MATCH ?1
              AND ge.volume_id=?2
              AND ge.path_normalized=?3
              AND ge.is_stale=0
        )",
        rusqlite::params![complete_query, source_id, complete_expected_path],
        |row| row.get(0),
    );
    if let Err(error) = &result {
        crate::global_index::qa_trace::record_sqlite_error("qa.durable_fts_marker_probe", error);
    }
    result
}

/// Measures the lock wait, statement execution, and commit separately for
/// ordinary QA writes. Explicit IMMEDIATE transactions preserve the same
/// SQLite writer-slot semantics as autocommit DML while exposing those phases.
fn run_diagnostic_write(
    db: &Database,
    source: &'static str,
    operation: impl FnOnce(&rusqlite::Transaction<'_>) -> rusqlite::Result<usize>,
) -> Result<(), DbError> {
    let connection_started = Instant::now();
    let mut connection = db.conn()?;
    let connection_acquisition_us = connection_started.elapsed().as_micros();
    let lock_started = Instant::now();
    let transaction = match connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
    {
        Ok(transaction) => transaction,
        Err(error) => {
            crate::global_index::qa_trace::record_sqlite_error(&format!("{source}.begin"), &error);
            return Err(error.into());
        }
    };
    let lock_wait_us = lock_started.elapsed().as_micros();
    let _transaction_trace =
        crate::global_index::qa_trace::TransactionTrace::start("write", source);
    let execution_started = Instant::now();
    let changed_rows = match operation(&transaction) {
        Ok(changed_rows) => changed_rows,
        Err(error) => {
            crate::global_index::qa_trace::record_sqlite_error(
                &format!("{source}.execute"),
                &error,
            );
            drop(transaction);
            return Err(error.into());
        }
    };
    let execution_us = execution_started.elapsed().as_micros();
    let commit_started = Instant::now();
    if let Err(error) = transaction.commit() {
        crate::global_index::qa_trace::record_sqlite_error(&format!("{source}.commit"), &error);
        return Err(error.into());
    }
    let commit_us = commit_started.elapsed().as_micros();
    crate::global_index::qa_trace::record(&format!(
        "sqlite_write source={source} connection_acquisition_us={connection_acquisition_us} lock_wait_us={lock_wait_us} execution_us={execution_us} commit_us={commit_us} rows={changed_rows}"
    ));
    Ok(())
}

pub fn run_initial(
    profile: &Path,
    source_id: &str,
    query: &str,
    expected_path: &Path,
    complete_query: &str,
    complete_expected_path: &Path,
    expected_fixture_file_count: usize,
) -> Result<Value, Box<dyn Error>> {
    let db = verify_task_profile(profile, source_id)?;
    let source = descriptor(&db, source_id)?;
    let scenario = diagnostic_scenario()?;
    let expected_path = normalize_path(&expected_path.to_string_lossy());
    let complete_expected_path = normalize_path(&complete_expected_path.to_string_lossy());
    if expected_fixture_file_count == 0 {
        return Err("filename qualification requires a non-empty task fixture".into());
    }
    let fixture_file_pattern = expected_path
        .rsplit_once('/')
        .and_then(|(bucket_path, _)| bucket_path.rsplit_once('/'))
        .map(|(fixture_root, _)| format!("{fixture_root}/bucket-%/filename-first-item-______.txt"))
        .ok_or("qualification fixture path is not nested under a bucket directory")?;
    let fixture_root_prefix = expected_path
        .rsplit_once('/')
        .and_then(|(bucket_path, _)| bucket_path.rsplit_once('/'))
        .map(|(fixture_root, _)| format!("{fixture_root}/"))
        .ok_or("qualification fixture path is not nested under a bucket directory")?;
    let durable_connection = db.conn()?;
    let database_path = profile.join("fylura.sqlite3");
    let filename_first = crate::global_index::filename_first_enabled();
    let started = Instant::now();
    let concurrent_writer_stop = Arc::new(AtomicBool::new(false));
    let concurrent_writer_db = db.clone();
    let concurrent_writer_db_path = db.path().to_path_buf();
    let concurrent_writer_source = source_id.to_owned();
    let concurrent_writer_stop_thread = Arc::clone(&concurrent_writer_stop);
    let concurrent_writer = scenario.ordinary_writes.then(|| thread::spawn(move || {
        let mut file_latencies_us = Vec::new();
        let mut journal_latencies_us = Vec::new();
        let mut settings_latencies_us = Vec::new();
        let mut errors = Vec::new();
        let mut index = 0_u64;
        let mut peak_wal_bytes = 0_u64;
        while !concurrent_writer_stop_thread.load(Ordering::Acquire) {
            peak_wal_bytes =
                peak_wal_bytes.max(file_size(&sidecar(&concurrent_writer_db_path, "-wal")));
            let file_started = Instant::now();
            let file_result = concurrent_writer_db.insert_file(InsertFileRequest {
                id: format!("filename-first-concurrent-{index}"),
                path: format!("Z:\\fylura-native-qa-concurrent\\{index}.txt"),
                name: format!("concurrent-{index}.txt"),
                extension: "txt".into(),
                size: index as i64,
                mtime: unix_now(),
                ctime: unix_now(),
                is_dir: false,
                state_code: 0,
            });
            file_latencies_us.push(file_started.elapsed().as_micros() as u64);
            if let Err(error) = file_result {
                crate::global_index::qa_trace::record_db_error(
                    "qa.files.insert_file",
                    &error,
                );
                errors.push(format!("files:{error:?}"));
            }

            let journal_started = Instant::now();
            let journal_result = run_diagnostic_write(
                &concurrent_writer_db,
                "qa.journal_update",
                |transaction| {
                    transaction.execute(
                        "UPDATE global_volumes
                         SET journal_id=journal_id, journal_cursor=journal_cursor, updated_at=updated_at
                         WHERE id=?1",
                        [&concurrent_writer_source],
                    )
                },
            );
            journal_latencies_us.push(journal_started.elapsed().as_micros() as u64);
            if let Err(error) = journal_result {
                crate::global_index::qa_trace::record_db_error("qa.journal_update", &error);
                errors.push(format!("journal:{error:?}"));
            }

            let settings_started = Instant::now();
            let setting_value = index.to_string();
            let settings_result = run_diagnostic_write(
                &concurrent_writer_db,
                "qa.settings_upsert",
                |transaction| {
                    transaction.execute(
                        "INSERT INTO app_settings(key,value) VALUES('filename_first_concurrent_qa',?1)
                         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                        [&setting_value],
                    )
                },
            );
            settings_latencies_us.push(settings_started.elapsed().as_micros() as u64);
            if let Err(error) = settings_result {
                crate::global_index::qa_trace::record_db_error("qa.settings_upsert", &error);
                errors.push(format!("settings:{error:?}"));
            }
            index = index.saturating_add(1);
            thread::sleep(std::time::Duration::from_millis(100));
        }
        peak_wal_bytes =
            peak_wal_bytes.max(file_size(&sidecar(&concurrent_writer_db_path, "-wal")));
        let sqlite_busy_errors = errors
            .iter()
            .filter(|error| {
                error.to_ascii_lowercase().contains("busy")
                    || error.to_ascii_lowercase().contains("locked")
            })
            .count();
        json!({
            "cycles": index,
            "peakWalBytes": peak_wal_bytes,
            "files": latency_summary(&file_latencies_us),
            "journal": latency_summary(&journal_latencies_us),
            "settings": latency_summary(&settings_latencies_us),
            "errors": errors,
            "sqliteBusyErrors": sqlite_busy_errors,
        })
    }));
    let search_poller_stop = Arc::new(AtomicBool::new(false));
    let search_poller = scenario.search_polling.then(|| {
        let poller_db = db.clone();
        let poller_stop = Arc::clone(&search_poller_stop);
        let poller_query = query.to_owned();
        let poller_expected_path = expected_path.clone();
        let poller_index_started = started.clone();
        thread::spawn(move || {
            let mut latencies_us = Vec::new();
            let mut errors = Vec::new();
            let mut query_count = 0_u64;
            let mut visible_count = 0_u64;
            let mut first_search = None;
            while !poller_stop.load(Ordering::Acquire) {
                let search_started = Instant::now();
                let result = poller_db.search_global_entries_snapshot(&poller_query, 50, 0);
                let elapsed_us = search_started.elapsed().as_micros() as u64;
                latencies_us.push(elapsed_us);
                query_count = query_count.saturating_add(1);
                match result {
                    Ok(snapshot) => {
                        let matching_result = snapshot
                            .results
                            .iter()
                            .find(|result| normalize_path(&result.path) == poller_expected_path);
                        let visible = matching_result.is_some_and(|result| {
                            crate::global_index::commands::revalidated_global_entry(
                                &poller_db, &result.id,
                            )
                            .is_ok()
                        });
                        if visible {
                            visible_count = visible_count.saturating_add(1);
                            if first_search.is_none() {
                                first_search = Some(json!({
                                    "elapsedUs": poller_index_started.elapsed().as_micros(),
                                    "collectionComplete": snapshot.index_status.collection_complete,
                                    "processedEntries": snapshot.index_status.processed_entries,
                                    "totalEntries": snapshot.index_status.total_entries,
                                }));
                            }
                        }
                    }
                    Err(error) => {
                        crate::global_index::qa_trace::record_db_error(
                            "qa.search_poll.snapshot",
                            &error,
                        );
                        errors.push(format!("{error:?}"));
                    }
                }
                thread::sleep(Duration::from_millis(100));
            }
            json!({
                "enabled": true,
                "queryCount": query_count,
                "visibleQueryCount": visible_count,
                "latency": latency_summary(&latencies_us),
                "errors": errors,
                "firstVisible": first_search,
            })
        })
    });
    let worker_db = db.clone();
    let worker_source = source.clone();
    let worker = std::thread::spawn(move || -> Result<(), String> {
        let mut sink =
            crate::global_index::coordinator::database_sink_for_qualification(worker_db.clone());
        let provider = WindowsGlobalIndexProvider::with_wake(
            Arc::new(super::GlobalIndexWakeSlot::default()),
            worker_db.path().to_path_buf(),
        );
        provider
            .start_initial_index(&worker_source, &mut sink, &AtomicBool::new(false))
            .map_err(|error| error.to_string())?;
        let completed_at = unix_now();
        sink.finish_source_operation(
            &worker_source.volume.id,
            INDEX_STATUS_READY,
            None,
            Some(completed_at),
            Some(completed_at),
            true,
        )
        .map_err(|error| error.to_string())?;
        provider.shutdown().map_err(|error| error.to_string())?;
        Ok(())
    });

    let mut first_searchable_us = None;
    let mut first_search_complete = None;
    let mut first_search_processed_entries = None;
    let mut first_search_total_entries = None;
    let mut durable_filename_index_us = None;
    let mut transient_catalog_ready_us = None;
    let mut transient_catalog_entry_count = None;
    let mut transient_catalog_estimated_bytes = None;
    let mut transient_fixture_file_entries = None;
    let mut complete_transient_search_visible = None;
    let mut complete_transient_search_before_durable_fts = None;
    let mut collection_complete_when_transient_ready = None;
    let mut peak_working_set_bytes = 0u64;
    let mut peak_private_bytes = 0u64;
    let mut peak_database_bytes_with_wal =
        file_size(&database_path).saturating_add(file_size(&sidecar(&database_path, "-wal")));
    let mut next_durable_fts_probe = Instant::now();
    let mut next_activity_probe = Instant::now();
    let mut sqlite_activity_samples = Vec::new();
    let mut durable_readiness_probe_count = 0u64;
    let mut durable_fts_match_probe_count = 0u64;
    loop {
        if scenario.search_polling
            && durable_filename_index_us.is_none()
            && Instant::now() >= next_durable_fts_probe
        {
            // Do not issue an FTS MATCH every 10 ms while the FTS bulk writer
            // is changing the same database. Poll the small source checkpoint
            // every 250 ms, then validate the final marker through FTS only
            // after the source has published its completed cursor.
            next_durable_fts_probe = Instant::now() + Duration::from_millis(250);
            durable_readiness_probe_count = durable_readiness_probe_count.saturating_add(1);
            let source_fts_ready = durable_source_fts_ready(
                &durable_connection,
                source_id,
                expected_fixture_file_count,
            )?;
            if source_fts_ready {
                durable_fts_match_probe_count = durable_fts_match_probe_count.saturating_add(1);
                let final_marker_visible = durable_fts_marker_visible(
                    &durable_connection,
                    source_id,
                    complete_query,
                    &complete_expected_path,
                )?;
                if final_marker_visible {
                    durable_filename_index_us = Some(started.elapsed().as_micros());
                }
            }
        }
        if transient_catalog_ready_us.is_none() {
            if let Some((true, catalog_entries, fixture_entries)) =
                db.filename_search.transient_catalog_qa_counts(
                    source_id,
                    &fixture_root_prefix,
                    "filename-first-item-",
                )
            {
                transient_catalog_ready_us = Some(started.elapsed().as_micros());
                transient_catalog_entry_count = Some(catalog_entries);
                transient_catalog_estimated_bytes = db
                    .filename_search
                    .transient_catalog_estimated_bytes(source_id);
                transient_fixture_file_entries = Some(fixture_entries);
                if scenario.search_polling {
                    complete_transient_search_before_durable_fts =
                        Some(durable_filename_index_us.is_none());
                    complete_transient_search_visible = Some(live_result_visible(
                        &db,
                        complete_query,
                        &complete_expected_path,
                    )?);
                    collection_complete_when_transient_ready =
                        Some(db.global_index_status()?.collection_complete);
                }
            }
        }
        if Instant::now() >= next_activity_probe {
            // Sample the WAL at 10 Hz without adding a SQLite query to the
            // indexing path; r2d2 state and process counters are in-memory.
            next_activity_probe = Instant::now() + Duration::from_millis(100);
            let elapsed_us = started.elapsed().as_micros();
            let main_bytes = file_size(&database_path);
            let wal_bytes = file_size(&sidecar(&database_path, "-wal"));
            peak_database_bytes_with_wal =
                peak_database_bytes_with_wal.max(main_bytes.saturating_add(wal_bytes));
            if let Some((working_set, private_bytes)) = process_memory_sample() {
                peak_working_set_bytes = peak_working_set_bytes.max(working_set);
                peak_private_bytes = peak_private_bytes.max(private_bytes);
            }
            let (connections, idle_connections, active_connections) =
                db.diagnostic_connection_counts();
            let active_transactions = crate::global_index::qa_trace::active_transaction_count();
            let sample = json!({
                "elapsedUs": elapsed_us,
                "databaseBytes": main_bytes,
                "walBytes": wal_bytes,
                "databaseAndWalBytes": main_bytes.saturating_add(wal_bytes),
                "connections": connections,
                "idleConnections": idle_connections,
                "activeConnections": active_connections,
                "activeTransactions": active_transactions,
            });
            crate::global_index::qa_trace::record(&format!(
                "sqlite_activity elapsed_us={elapsed_us} database_bytes={main_bytes} wal_bytes={wal_bytes} connections={connections} idle_connections={idle_connections} active_connections={active_connections} active_transactions={active_transactions}"
            ));
            sqlite_activity_samples.push(sample);
        }
        if worker.is_finished() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let worker_result = worker.join();
    concurrent_writer_stop.store(true, Ordering::Release);
    search_poller_stop.store(true, Ordering::Release);
    let concurrent_database_writes = if let Some(writer) = concurrent_writer {
        writer
            .join()
            .map_err(|_| "concurrent ordinary database writer panicked")?
    } else {
        json!({
            "cycles": 0,
            "peakWalBytes": 0,
            "files": latency_summary(&[]),
            "journal": latency_summary(&[]),
            "settings": latency_summary(&[]),
            "errors": [],
            "sqliteBusyErrors": 0,
        })
    };
    let search_polling = if let Some(poller) = search_poller {
        poller
            .join()
            .map_err(|_| "search polling thread panicked")?
    } else {
        json!({
            "enabled": false,
            "queryCount": 0,
            "visibleQueryCount": 0,
            "latency": latency_summary(&[]),
            "errors": [],
            "firstVisible": null,
        })
    };
    worker_result
        .map_err(|_| "native MFT qualification worker panicked")?
        .map_err(|error| format!("native MFT qualification failed: {error}"))?;
    let full_index_us = started.elapsed().as_micros();
    // The worker can finish between two 250 ms readiness polls. Recheck the
    // durable marker after joining it so a successful complete FTS build is
    // not reported as missing merely because the final poll was skipped.
    if scenario.search_polling && durable_filename_index_us.is_none() {
        durable_readiness_probe_count = durable_readiness_probe_count.saturating_add(1);
        if durable_source_fts_ready(&durable_connection, source_id, expected_fixture_file_count)? {
            durable_fts_match_probe_count = durable_fts_match_probe_count.saturating_add(1);
            if durable_fts_marker_visible(
                &durable_connection,
                source_id,
                complete_query,
                &complete_expected_path,
            )? {
                durable_filename_index_us = Some(started.elapsed().as_micros());
            }
        }
    }
    if !scenario.search_polling {
        durable_readiness_probe_count = durable_readiness_probe_count.saturating_add(1);
        if durable_source_fts_ready(&durable_connection, source_id, expected_fixture_file_count)? {
            durable_fts_match_probe_count = durable_fts_match_probe_count.saturating_add(1);
            if durable_fts_marker_visible(
                &durable_connection,
                source_id,
                complete_query,
                &complete_expected_path,
            )? {
                // With polling disabled this is a terminal upper bound, not
                // the instant the marker first became visible.
                durable_filename_index_us = Some(full_index_us);
            }
        }
    }
    let final_pre_checkpoint_db_bytes = file_size(&database_path);
    let final_pre_checkpoint_wal_bytes = file_size(&sidecar(&database_path, "-wal"));
    peak_database_bytes_with_wal = peak_database_bytes_with_wal
        .max(final_pre_checkpoint_db_bytes.saturating_add(final_pre_checkpoint_wal_bytes));
    if let Some((working_set, private_bytes)) = process_memory_sample() {
        peak_working_set_bytes = peak_working_set_bytes.max(working_set);
        peak_private_bytes = peak_private_bytes.max(private_bytes);
    }
    if let Some(first_visible) = search_polling.get("firstVisible") {
        if !first_visible.is_null() {
            first_searchable_us = first_visible
                .get("elapsedUs")
                .and_then(Value::as_u64)
                .map(u128::from);
            first_search_complete = first_visible
                .get("collectionComplete")
                .and_then(Value::as_bool);
            first_search_processed_entries = first_visible
                .get("processedEntries")
                .and_then(Value::as_u64)
                .map(u128::from);
            first_search_total_entries = first_visible
                .get("totalEntries")
                .and_then(Value::as_u64)
                .map(u128::from);
        }
    }
    if first_searchable_us.is_none() && live_result_visible(&db, query, &expected_path)? {
        let status = db.global_index_status()?;
        first_searchable_us = Some(full_index_us);
        first_search_complete = Some(status.collection_complete);
        first_search_processed_entries = u128::try_from(status.processed_entries).ok();
        first_search_total_entries = u128::try_from(status.total_entries).ok();
    }
    let first_searchable_us =
        first_searchable_us.ok_or("target filename never became searchable")?;
    if peak_working_set_bytes == 0 || peak_private_bytes == 0 {
        return Err("Windows process memory counters were unavailable during qualification".into());
    }
    let durable_filename_index_us = durable_filename_index_us.ok_or_else(|| {
        let (entry_count, fts_bulk_build, fts_bulk_cursor, marker_visible): (i64, bool, i64, bool) =
            durable_connection
                .query_row(
                    "SELECT entry_count, fts_bulk_build, fts_bulk_cursor,
                         EXISTS(
                             SELECT 1
                             FROM global_entries_fts
                             JOIN global_entries ge ON ge.rowid=global_entries_fts.rowid
                             WHERE global_entries_fts MATCH ?2
                               AND ge.volume_id=global_volumes.id
                               AND ge.path_normalized=?3
                               AND ge.is_stale=0
                         )
                     FROM global_volumes WHERE id=?1",
                    rusqlite::params![source_id, complete_query, complete_expected_path],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap_or((-1, true, -1, false));
        format!(
            "complete task fixture was not visible through durable FTS after worker completion: entry_count={entry_count} fts_bulk_build={fts_bulk_build} fts_bulk_cursor={fts_bulk_cursor} marker_visible={marker_visible}"
        )
    })?;
    let peak_wal_bytes = sqlite_activity_samples
        .iter()
        .filter_map(|sample| sample.get("walBytes").and_then(Value::as_u64))
        .max()
        .unwrap_or(final_pre_checkpoint_wal_bytes);
    let indexed_source = db
        .get_global_volume(source_id)?
        .ok_or("authorized source disappeared after indexing")?;
    let saved_journal_id = indexed_source
        .journal_id
        .as_deref()
        .and_then(|value| value.parse::<u64>().ok());
    let saved_journal_cursor = indexed_source
        .journal_cursor
        .as_deref()
        .and_then(|value| value.parse::<i64>().ok());
    let journal_handle = open_volume(&indexed_source.mount_path)?;
    let current_journal = query_journal(journal_handle);
    unsafe { CloseHandle(journal_handle) };
    let current_journal = current_journal?;
    let journal_checkpoint_valid = saved_journal_id == Some(current_journal.UsnJournalID)
        && saved_journal_cursor.is_some_and(|cursor| {
            cursor >= current_journal.FirstUsn && cursor <= current_journal.NextUsn
        });
    if !journal_checkpoint_valid {
        return Err(format!(
            "initial indexing did not preserve a usable USN checkpoint: saved_journal_id={saved_journal_id:?} current_journal_id={} saved_cursor={saved_journal_cursor:?} available={}..={}",
            current_journal.UsnJournalID,
            current_journal.FirstUsn,
            current_journal.NextUsn,
        )
        .into());
    }
    if filename_first
        && scenario.search_polling
        && (transient_catalog_ready_us.is_none()
            || transient_fixture_file_entries != Some(expected_fixture_file_count)
            || complete_transient_search_visible != Some(true)
            || complete_transient_search_before_durable_fts != Some(true)
            || collection_complete_when_transient_ready != Some(false))
    {
        return Err(format!(
            "complete transient filename catalogue was not searchable before durable FTS: ready_us={transient_catalog_ready_us:?} fixture_entries={transient_fixture_file_entries:?} expected={expected_fixture_file_count} final_marker_visible={complete_transient_search_visible:?} before_fts={complete_transient_search_before_durable_fts:?} collection_complete={collection_complete_when_transient_ready:?}"
        )
        .into());
    }
    let complete_coverage_search_visible = db
        .search_global_entries(complete_query, 50, 0)?
        .iter()
        .any(|result| normalize_path(&result.path) == complete_expected_path);
    if !complete_coverage_search_visible {
        return Err(
            "a filename from the final MFT records was missing after indexing completed".into(),
        );
    }
    let (database_total_entries, database_active_entries) =
        db.native_qa_global_entry_counts(source_id)?;
    let (source_entry_count, source_fact_active_rows, source_fact_active_ids): (i64, i64, i64) =
        db.conn()?.query_row(
            "SELECT gv.entry_count, facts.active_rows, facts.active_ids
             FROM global_volumes gv
             JOIN global_entry_source_facts facts ON facts.volume_id=gv.id
             WHERE gv.id=?1",
            [source_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    if source_entry_count != database_active_entries
        || source_fact_active_rows != database_active_entries
        || source_fact_active_ids > source_fact_active_rows
    {
        return Err(format!(
            "source statistics disagree with active entries: entry_count={source_entry_count} facts_rows={source_fact_active_rows} facts_ids={source_fact_active_ids} active_entries={database_active_entries}"
        )
        .into());
    }
    let fixture_file_entries = db.conn()?.query_row(
        "SELECT COUNT(*) FROM global_entries WHERE volume_id=?1 AND is_stale=0 AND path_normalized LIKE ?2",
        rusqlite::params![source_id, fixture_file_pattern],
        |row| row.get::<_, i64>(0),
    )?;
    let integrity_conn = db.conn()?;
    let (fts_bulk_build, fts_bulk_cursor): (i64, i64) = integrity_conn.query_row(
        "SELECT fts_bulk_build, fts_bulk_cursor FROM global_volumes WHERE id=?1",
        [source_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if fts_bulk_build != 0 || fts_bulk_cursor != 0 {
        return Err(format!(
            "source FTS bulk build did not finish: fts_bulk_build={fts_bulk_build} cursor={fts_bulk_cursor}"
        )
        .into());
    }
    integrity_conn.execute(
        "INSERT INTO global_entries_fts(global_entries_fts,rank) VALUES('integrity-check',1)",
        [],
    )?;
    let fts_trigger_count = integrity_conn.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type='trigger' AND name IN ('global_entries_ai','global_entries_ad','global_entries_au')",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if fts_trigger_count != 3 {
        return Err(
            format!("expected all FTS triggers restored, found {fts_trigger_count}").into(),
        );
    }
    let database_footprint_after_index = database_footprint(&database_path, Some(&integrity_conn));
    let database_bytes = database_footprint_after_index["mainBytes"]
        .as_u64()
        .unwrap_or_default()
        .saturating_add(
            database_footprint_after_index["walBytes"]
                .as_u64()
                .unwrap_or_default(),
        );
    drop(integrity_conn);
    let final_status = db.global_index_status()?;
    let source_status = db
        .get_global_volume(source_id)?
        .ok_or("qualification source disappeared before completion check")?;
    if source_status.index_status != crate::global_index::models::INDEX_STATUS_READY {
        return Err(format!(
            "native index worker exited without a ready source: global={final_status:?} source_status={}",
            source_status.index_status
        )
        .into());
    }
    let coverage = db.global_index_coverage()?;
    let source_coverage = coverage
        .sources
        .iter()
        .find(|source| source.source_id == source_id)
        .ok_or("qualification source has no coverage progress")?;
    let undetermined_entries = {
        let connection = db.conn()?;
        let mut statement = connection.prepare(
            "SELECT name,path,file_attributes,is_directory FROM global_entries WHERE volume_id=?1 AND is_stale=0 AND scope_decision='undetermined' ORDER BY path",
        )?;
        let entries = statement
            .query_map([source_id], |row| {
                Ok(json!({
                    "name": row.get::<_, String>(0)?,
                    "path": row.get::<_, String>(1)?,
                    "fileAttributes": row.get::<_, i64>(2)?,
                    "isDirectory": row.get::<_, i64>(3)? != 0,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        entries
    };

    drop(durable_connection);
    drop(db);
    let database_footprint_after_close = database_footprint(&database_path, None);
    let reopened = Database::open(profile.join("fylura.sqlite3"))?;
    let restarted_search_visible = reopened
        .search_global_entries(query, 50, 0)?
        .iter()
        .any(|result| normalize_path(&result.path) == expected_path);
    if !restarted_search_visible {
        return Err("durable filename result was missing after database reopen".into());
    }
    let restarted_complete_coverage_visible = reopened
        .search_global_entries(complete_query, 50, 0)?
        .iter()
        .any(|result| normalize_path(&result.path) == complete_expected_path);
    if !restarted_complete_coverage_visible {
        return Err("the final MFT filename was missing after database reopen".into());
    }
    let reopen_connection = reopened.conn()?;
    let database_footprint_after_reopen =
        database_footprint(&database_path, Some(&reopen_connection));
    drop(reopen_connection);
    drop(reopened);
    let checkpoint_connection = Connection::open(&database_path)?;
    checkpoint_connection.busy_timeout(Duration::ZERO)?;
    let wal_bytes_before_passive = file_size(&sidecar(&database_path, "-wal"));
    let passive_started = Instant::now();
    let passive_checkpoint: rusqlite::Result<(i64, i64, i64)> =
        checkpoint_connection.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        });
    let (passive_busy, passive_log_frames, passive_checkpointed_frames) = passive_checkpoint
        .map_err(|error| {
            crate::global_index::qa_trace::record_sqlite_error(
                "qa.final_checkpoint.passive",
                &error,
            );
            error
        })?;
    let passive_elapsed_us = passive_started.elapsed().as_micros();
    let wal_bytes_after_passive = file_size(&sidecar(&database_path, "-wal"));
    crate::global_index::qa_trace::record(&format!(
        "sqlite_checkpoint mode=passive busy={passive_busy} log_frames={passive_log_frames} checkpointed_frames={passive_checkpointed_frames} elapsed_us={passive_elapsed_us} wal_before_bytes={wal_bytes_before_passive} wal_after_bytes={wal_bytes_after_passive}"
    ));
    let wal_bytes_before_restart = file_size(&sidecar(&database_path, "-wal"));
    let restart_started = Instant::now();
    let restart_checkpoint: rusqlite::Result<(i64, i64, i64)> =
        checkpoint_connection.query_row("PRAGMA wal_checkpoint(RESTART)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        });
    let (restart_busy, restart_log_frames, restart_checkpointed_frames) = restart_checkpoint
        .map_err(|error| {
            crate::global_index::qa_trace::record_sqlite_error(
                "qa.final_checkpoint.restart",
                &error,
            );
            error
        })?;
    let restart_elapsed_us = restart_started.elapsed().as_micros();
    let wal_bytes_after_restart = file_size(&sidecar(&database_path, "-wal"));
    crate::global_index::qa_trace::record(&format!(
        "sqlite_checkpoint mode=restart busy={restart_busy} log_frames={restart_log_frames} checkpointed_frames={restart_checkpointed_frames} elapsed_us={restart_elapsed_us} wal_before_bytes={wal_bytes_before_restart} wal_after_bytes={wal_bytes_after_restart}"
    ));
    let wal_bytes_before_truncate = file_size(&sidecar(&database_path, "-wal"));
    let truncate_checkpoint: rusqlite::Result<(i64, i64, i64)> =
        checkpoint_connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        });
    let (checkpoint_busy, checkpoint_log_frames, checkpointed_frames) = truncate_checkpoint
        .map_err(|error| {
            crate::global_index::qa_trace::record_sqlite_error(
                "qa.final_checkpoint.truncate",
                &error,
            );
            error
        })?;
    let wal_bytes_after_truncate = file_size(&sidecar(&database_path, "-wal"));
    drop(checkpoint_connection);
    let database_footprint_after_checkpoint_close = database_footprint(&database_path, None);
    let reopened_after_checkpoint = Database::open(&database_path)?;
    let checkpoint_reopen_connection = reopened_after_checkpoint.conn()?;
    let database_footprint_after_checkpoint =
        database_footprint(&database_path, Some(&checkpoint_reopen_connection));
    let checkpointed_search_visible = reopened_after_checkpoint
        .search_global_entries(query, 50, 0)?
        .iter()
        .any(|result| normalize_path(&result.path) == expected_path);
    if !checkpointed_search_visible {
        return Err("filename search was missing after WAL checkpoint and reopen".into());
    }
    let search_evidence = json!({
        "diagnosticScenario": scenario.name,
        "searchPollingEnabled": scenario.search_polling,
        "filenameFirstEnabled": filename_first,
        "firstSearchObservationMode": if scenario.search_polling { "100ms_poll" } else { "post_index_probe" },
        "durableFtsObservationMode": if scenario.search_polling { "250ms_poll" } else { "post_index_terminal_probe" },
        "query": query,
        "expectedPath": expected_path,
        "firstSearchableUs": first_searchable_us,
        "durableFilenameIndexUs": durable_filename_index_us,
        "firstSearchBeforeComplete": first_search_complete == Some(false),
        "firstSearchProcessedEntries": first_search_processed_entries,
        "firstSearchTotalEntries": first_search_total_entries,
        "durableReadinessProbeCount": durable_readiness_probe_count,
        "durableFtsMatchProbeCount": durable_fts_match_probe_count,
        "searchPolling": search_polling,
        "transientCatalogReadyUs": transient_catalog_ready_us,
        "transientCatalogEntryCount": transient_catalog_entry_count,
        "transientCatalogEstimatedBytes": transient_catalog_estimated_bytes,
        "transientFixtureFileEntries": transient_fixture_file_entries,
        "completeTransientSearchVisible": complete_transient_search_visible,
        "completeTransientSearchBeforeDurableFts": complete_transient_search_before_durable_fts,
        "collectionCompleteWhenTransientReady": collection_complete_when_transient_ready,
        "completeCoverageSearchVisible": complete_coverage_search_visible,
    });
    let persistence_evidence = json!({
        "fullIndexUs": full_index_us,
        "peakWorkingSetBytes": peak_working_set_bytes,
        "peakPrivateBytes": peak_private_bytes,
        "peakWalBytes": peak_wal_bytes,
        "concurrentDatabaseWrites": concurrent_database_writes,
        "databaseTotalEntries": database_total_entries,
        "databaseActiveEntries": database_active_entries,
        "sourceEntryCount": source_entry_count,
        "sourceFactActiveRows": source_fact_active_rows,
        "sourceFactActiveIds": source_fact_active_ids,
        "fixtureFileEntries": fixture_file_entries,
        "databaseBytesWithWal": database_bytes,
        "peakDatabaseBytesWithWal": peak_database_bytes_with_wal,
        "databaseBytesImmediatelyBeforeFinalCheckpoint": final_pre_checkpoint_db_bytes,
        "walBytesImmediatelyBeforeFinalCheckpoint": final_pre_checkpoint_wal_bytes,
        "sqliteActivitySamples": sqlite_activity_samples,
        "databaseFootprintAfterIndex": database_footprint_after_index,
        "databaseFootprintAfterClose": database_footprint_after_close,
        "databaseFootprintAfterReopen": database_footprint_after_reopen,
        "databaseFootprintAfterCheckpointClose": database_footprint_after_checkpoint_close,
        "databaseFootprintAfterCheckpoint": database_footprint_after_checkpoint,
        "walCheckpoint": json!({
            "mode": "truncate",
            "busy": checkpoint_busy,
            "logFrames": checkpoint_log_frames,
            "checkpointedFrames": checkpointed_frames,
            "walBytesBefore": wal_bytes_before_truncate,
            "walBytesAfter": wal_bytes_after_truncate,
            "passive": {
                "busy": passive_busy,
                "logFrames": passive_log_frames,
                "checkpointedFrames": passive_checkpointed_frames,
                "elapsedUs": passive_elapsed_us,
                "walBytesBefore": wal_bytes_before_passive,
                "walBytesAfter": wal_bytes_after_passive,
            },
            "restart": {
                "busy": restart_busy,
                "logFrames": restart_log_frames,
                "checkpointedFrames": restart_checkpointed_frames,
                "elapsedUs": restart_elapsed_us,
                "walBytesBefore": wal_bytes_before_restart,
                "walBytesAfter": wal_bytes_after_restart,
            },
        }),
        "checkpointedSearchVisible": checkpointed_search_visible,
        "ftsIntegrityVerified": true,
        "ftsTriggerCount": fts_trigger_count,
        "ftsBulkBuildComplete": fts_bulk_build == 0 && fts_bulk_cursor == 0,
        "restartedSearchVisible": restarted_search_visible,
        "restartedCompleteCoverageSearchVisible": restarted_complete_coverage_visible,
        "indexComplete": final_status.collection_complete,
        "sourceIndexStatus": source_status.index_status,
        "savedJournalId": saved_journal_id,
        "savedJournalCursor": saved_journal_cursor,
        "journalCheckpointValid": journal_checkpoint_valid,
    });
    let coverage_evidence = json!({
        "coverageUndetermined": source_coverage.undetermined,
        "coveragePending": source_coverage.pending,
        "undeterminedEntries": undetermined_entries,
    });
    let mut result = serde_json::Map::new();
    result.extend(
        search_evidence
            .as_object()
            .ok_or("search evidence is not an object")?
            .clone(),
    );
    result.extend(
        persistence_evidence
            .as_object()
            .ok_or("persistence evidence is not an object")?
            .clone(),
    );
    result.extend(
        coverage_evidence
            .as_object()
            .ok_or("coverage evidence is not an object")?
            .clone(),
    );
    Ok(Value::Object(result))
}

fn process_memory_sample() -> Option<(u64, u64)> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let result = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    (result != 0).then_some((counters.WorkingSetSize as u64, counters.PrivateUsage as u64))
}

fn latency_summary(samples: &[u64]) -> Value {
    if samples.is_empty() {
        return json!({"count": 0, "maxUs": 0, "p95Us": 0});
    }
    let mut ordered = samples.to_vec();
    ordered.sort_unstable();
    let p95_index = ordered
        .len()
        .saturating_mul(95)
        .saturating_add(99)
        .checked_div(100)
        .unwrap_or_default()
        .saturating_sub(1);
    json!({
        "count": ordered.len(),
        "maxUs": ordered.last().copied().unwrap_or_default(),
        "p95Us": ordered[p95_index.min(ordered.len() - 1)],
    })
}

pub fn run_incremental(
    profile: &Path,
    source_id: &str,
    added_path: &Path,
    renamed_from: &Path,
    renamed_to: &Path,
    moved_from: &Path,
    moved_to: &Path,
    deleted_path: &Path,
) -> Result<Value, Box<dyn Error>> {
    let db = verify_task_profile(profile, source_id)?;
    let source = descriptor(&db, source_id)?;
    let mut sink = crate::global_index::coordinator::database_sink_for_qualification(db.clone());
    let provider = WindowsGlobalIndexProvider::with_wake(
        Arc::new(super::GlobalIndexWakeSlot::default()),
        db.path().to_path_buf(),
    );
    provider.resume_incremental_sync(&source, &mut sink, &AtomicBool::new(false))?;
    sink.finish_source_operation(
        source_id,
        INDEX_STATUS_READY,
        None,
        None,
        Some(unix_now()),
        false,
    )?;
    provider.shutdown()?;

    let added_visible = search_path(&db, added_path)?;
    let renamed_visible = search_path(&db, renamed_to)?;
    let old_rename_hidden = !search_path(&db, renamed_from)?;
    let moved_visible = search_path(&db, moved_to)?;
    let old_move_hidden = !search_path(&db, moved_from)?;
    let deleted_hidden = !search_path(&db, deleted_path)?;
    if !(added_visible
        && renamed_visible
        && old_rename_hidden
        && moved_visible
        && old_move_hidden
        && deleted_hidden)
    {
        return Err(format!(
            "incremental filename correctness failed: added={added_visible} renamed={renamed_visible} old_rename_hidden={old_rename_hidden} moved={moved_visible} old_move_hidden={old_move_hidden} deleted_hidden={deleted_hidden}"
        )
        .into());
    }

    drop(db);
    let reopened = Database::open(profile.join("fylura.sqlite3"))?;
    let restart_correct = search_path(&reopened, added_path)?
        && search_path(&reopened, renamed_to)?
        && !search_path(&reopened, renamed_from)?
        && search_path(&reopened, moved_to)?
        && !search_path(&reopened, moved_from)?
        && !search_path(&reopened, deleted_path)?;
    if !restart_correct {
        return Err("incremental filename changes were not durable after database reopen".into());
    }
    Ok(json!({
        "addedVisible": added_visible,
        "renamedVisible": renamed_visible,
        "oldRenameHidden": old_rename_hidden,
        "movedVisible": moved_visible,
        "oldMoveHidden": old_move_hidden,
        "deletedHidden": deleted_hidden,
        "restartCorrect": restart_correct,
    }))
}

fn search_path(db: &Database, path: &Path) -> Result<bool, Box<dyn Error>> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("qualification path has no UTF-8 filename")?;
    let expected_path = normalize_path(&path.to_string_lossy());
    Ok(db
        .search_global_entries(name, 25, 0)?
        .iter()
        .any(|result| normalize_path(&result.path) == expected_path))
}

fn database_footprint(path: &Path, connection: Option<&Connection>) -> Value {
    let fts_bytes = connection.and_then(|connection| {
        connection
            .query_row(
                "SELECT COALESCE(SUM(pgsize),0) FROM dbstat WHERE name LIKE 'global_entries_fts%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .ok()
            .map(|bytes| bytes.max(0) as u64)
    });
    json!({
        "mainBytes": file_size(path),
        "walBytes": file_size(&sidecar(path, "-wal")),
        "shmBytes": file_size(&sidecar(path, "-shm")),
        "ftsBytes": fts_bytes,
        "dbstatAvailable": fts_bytes.is_some(),
    })
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}
