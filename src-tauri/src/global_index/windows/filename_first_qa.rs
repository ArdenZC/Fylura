//! Task-owned native MFT qualification for the disposable filename-first path.
//! The caller supplies an isolated profile and a task-created NTFS fixture.

use super::DirectWindowsGlobalIndexProvider;
use crate::db::Database;
use crate::global_index::coordinator::{GlobalIndexProvider, GlobalIndexSink};
use crate::global_index::models::{normalize_path, unix_now, INDEX_STATUS_READY};
use crate::global_index::{GlobalSourceDescriptor, SourceOperationKind, SourceOperationRoute};
use serde_json::{json, Value};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

fn verify_task_profile(profile: &Path, source_id: &str) -> Result<Database, Box<dyn Error>> {
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
    let db = Database::open(profile.join("fylura.sqlite3"))?;
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
    Ok(db)
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

pub fn run_initial(
    profile: &Path,
    source_id: &str,
    query: &str,
    expected_path: &Path,
    complete_query: &str,
    complete_expected_path: &Path,
) -> Result<Value, Box<dyn Error>> {
    let db = verify_task_profile(profile, source_id)?;
    let source = descriptor(&db, source_id)?;
    let expected_path = normalize_path(&expected_path.to_string_lossy());
    let complete_expected_path = normalize_path(&complete_expected_path.to_string_lossy());
    let filename_first = std::env::var("ZC_GLOBAL_INDEX_FILENAME_FIRST")
        .map(|value| value != "0")
        .unwrap_or(true);
    let started = Instant::now();
    let worker_db = db.clone();
    let worker_source = source.clone();
    let worker = std::thread::spawn(move || -> Result<(), String> {
        let mut sink =
            crate::global_index::coordinator::database_sink_for_qualification(worker_db.clone());
        let operation_id = uuid::Uuid::new_v4().to_string();
        sink.admit_source_operation(
            &worker_source.volume,
            &operation_id,
            SourceOperationKind::Rebuild,
            SourceOperationRoute::Direct,
        )
        .map_err(|error| error.to_string())?;
        DirectWindowsGlobalIndexProvider::new()
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
        Ok(())
    });

    let mut first_searchable_us = None;
    let mut first_search_complete = None;
    let mut first_search_processed_entries = None;
    let mut first_search_total_entries = None;
    loop {
        if first_searchable_us.is_none() {
            if live_result_visible(&db, query, &expected_path)? {
                let status = db.global_index_status()?;
                first_searchable_us = Some(started.elapsed().as_micros());
                first_search_complete = Some(status.collection_complete);
                first_search_processed_entries = Some(status.processed_entries);
                first_search_total_entries = Some(status.total_entries);
            }
        }
        if worker.is_finished() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    worker
        .join()
        .map_err(|_| "native MFT qualification worker panicked")?
        .map_err(|error| format!("native MFT qualification failed: {error}"))?;
    let full_index_us = started.elapsed().as_micros();
    if first_searchable_us.is_none() {
        if live_result_visible(&db, query, &expected_path)? {
            let status = db.global_index_status()?;
            first_searchable_us = Some(full_index_us);
            first_search_complete = Some(status.collection_complete);
            first_search_processed_entries = Some(status.processed_entries);
            first_search_total_entries = Some(status.total_entries);
        }
    }
    let first_searchable_us =
        first_searchable_us.ok_or("target filename never became searchable")?;
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
    let database_path = profile.join("fylura.sqlite3");
    let database_bytes = file_size(&database_path)
        .saturating_add(file_size(&sidecar(&database_path, "-wal")))
        .saturating_add(file_size(&sidecar(&database_path, "-shm")));
    let final_status = db.global_index_status()?;
    if !final_status.collection_complete {
        return Err("native index worker exited without a complete source status".into());
    }

    drop(db);
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
    Ok(json!({
        "filenameFirstEnabled": filename_first,
        "query": query,
        "expectedPath": expected_path,
        "firstSearchableUs": first_searchable_us,
        "firstSearchBeforeComplete": first_search_complete == Some(false),
        "firstSearchProcessedEntries": first_search_processed_entries,
        "firstSearchTotalEntries": first_search_total_entries,
        "completeCoverageSearchVisible": complete_coverage_search_visible,
        "fullIndexUs": full_index_us,
        "databaseTotalEntries": database_total_entries,
        "databaseActiveEntries": database_active_entries,
        "databaseBytesWithWal": database_bytes,
        "restartedSearchVisible": restarted_search_visible,
        "restartedCompleteCoverageSearchVisible": restarted_complete_coverage_visible,
        "indexComplete": final_status.collection_complete,
    }))
}

pub fn run_incremental(
    profile: &Path,
    source_id: &str,
    added_path: &Path,
    renamed_from: &Path,
    renamed_to: &Path,
    deleted_path: &Path,
) -> Result<Value, Box<dyn Error>> {
    let db = verify_task_profile(profile, source_id)?;
    let source = descriptor(&db, source_id)?;
    let mut sink = crate::global_index::coordinator::database_sink_for_qualification(db.clone());
    let operation_id = uuid::Uuid::new_v4().to_string();
    sink.admit_source_operation(
        &source.volume,
        &operation_id,
        SourceOperationKind::Incremental,
        SourceOperationRoute::Direct,
    )?;
    DirectWindowsGlobalIndexProvider::new().resume_incremental_sync(
        &source,
        &mut sink,
        &AtomicBool::new(false),
    )?;
    sink.finish_source_operation(
        source_id,
        INDEX_STATUS_READY,
        None,
        None,
        Some(unix_now()),
        false,
    )?;

    let added_visible = search_path(&db, added_path)?;
    let renamed_visible = search_path(&db, renamed_to)?;
    let old_rename_hidden = !search_path(&db, renamed_from)?;
    let deleted_hidden = !search_path(&db, deleted_path)?;
    if !(added_visible && renamed_visible && old_rename_hidden && deleted_hidden) {
        return Err(format!(
            "incremental filename correctness failed: added={added_visible} renamed={renamed_visible} old_rename_hidden={old_rename_hidden} deleted_hidden={deleted_hidden}"
        )
        .into());
    }

    drop(db);
    let reopened = Database::open(profile.join("fylura.sqlite3"))?;
    let restart_correct = search_path(&reopened, added_path)?
        && search_path(&reopened, renamed_to)?
        && !search_path(&reopened, renamed_from)?
        && !search_path(&reopened, deleted_path)?;
    if !restart_correct {
        return Err("incremental filename changes were not durable after database reopen".into());
    }
    Ok(json!({
        "addedVisible": added_visible,
        "renamedVisible": renamed_visible,
        "oldRenameHidden": old_rename_hidden,
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
