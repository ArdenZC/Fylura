//! Native qualification only: the caller must positively control the disposable
//! NTFS VHD and isolated profile (qualifyWindowsGlobalIndexService.ps1).
use super::*;
use crate::{
    db::Database,
    global_index::{
        coverage::{bind_rule, CoverageMode},
        GlobalEntryInput,
    },
};
use serde_json::{json, Value};
use std::{error::Error, fs};

pub fn run(
    profile: &Path,
    source_id: &str,
    included: &Path,
    excluded: &Path,
) -> Result<Value, Box<dyn Error>> {
    let task = profile.parent().ok_or("missing task profile boundary")?;
    let runner = std::env::var_os("RUNNER_TEMP").ok_or("RUNNER_TEMP required")?;
    if !profile.is_absolute()
        || !task.starts_with(Path::new(&runner))
        || !task
            .join("qualification-volume-created-by-task.txt")
            .is_file()
    {
        return Err(
            "direct qualification requires the task-owned VHD marker and runner profile".into(),
        );
    }
    let db = Database::open(profile.join("fylura.sqlite3"))?;
    let enabled = db
        .list_global_volumes()?
        .into_iter()
        .filter(|v| v.enabled)
        .collect::<Vec<_>>();
    if enabled.len() != 1
        || enabled[0].id != source_id
        || !included.starts_with(&enabled[0].mount_path)
        || !excluded.starts_with(&enabled[0].mount_path)
    {
        return Err("only the explicitly authorized qualification volume is permitted".into());
    }
    if !included.is_file() || !excluded.is_file() {
        return Err("qualification fixtures missing".into());
    }
    if fs::metadata(included)?.len() > 4096 || fs::metadata(excluded)?.len() > 4096 {
        return Err("qualification fixtures exceed the bounded sample size".into());
    }
    let original = (fs::read(included)?, fs::read(excluded)?);
    let mut phases = Vec::new();
    for mode in [
        CoverageMode::SmartLean,
        CoverageMode::BroadCoverage,
        CoverageMode::Custom,
    ] {
        let source = db.get_global_volume(source_id)?.ok_or("source missing")?;
        let rules = if mode == CoverageMode::Custom {
            vec![bind_rule(
                &source,
                &excluded
                    .parent()
                    .ok_or("excluded parent missing")?
                    .to_string_lossy(),
                true,
            )?]
        } else {
            vec![]
        };
        db.save_global_index_coverage(db.global_index_coverage()?.policy.revision, mode, rules)?;
        baseline(&db, source_id)?;
        let included_visible = visible(&db, included)?;
        let excluded_visible = visible(&db, excluded)?;
        if !included_visible || excluded_visible != (mode != CoverageMode::SmartLean) {
            return Err(format!("direct native coverage mismatch: mode={mode:?} included={included_visible} low_value={excluded_visible}").into());
        }
        phases.push(json!({"mode":mode,"includedVisible":included_visible,"lowValueVisible":excluded_visible}));
    }
    // Revoke and re-enable fence a still-live Direct operation, not just a UI projection.
    let source = db.get_global_volume(source_id)?.ok_or("source missing")?;
    let mut sink = crate::global_index::coordinator::database_sink_for_qualification(db.clone());
    sink.admit_source_operation(
        &source,
        &uuid::Uuid::new_v4().to_string(),
        SourceOperationKind::Incremental,
        SourceOperationRoute::Direct,
    )?;
    db.set_global_volume_enabled(source_id, false)?;
    let input = GlobalEntryInput::from_path(source_id, included, PROVIDER_WINDOWS_MFT_USN);
    if sink.write_batch(std::slice::from_ref(&input)).is_ok() || visible(&db, included)? {
        return Err("revoked Direct writer remained active".into());
    }
    db.set_global_volume_enabled(source_id, true)?;
    if sink.write_batch(std::slice::from_ref(&input)).is_ok() {
        return Err("re-enabled source accepted old Direct writer".into());
    }
    drop(sink);
    drop(db);
    let reopened = Database::open(profile.join("fylura.sqlite3"))?;
    reopened.save_global_index_coverage(
        reopened.global_index_coverage()?.policy.revision,
        CoverageMode::SmartLean,
        vec![],
    )?;
    baseline(&reopened, source_id)?;
    if !visible(&reopened, included)? || visible(&reopened, excluded)? {
        return Err("Direct restart/backfill mismatch".into());
    }
    // Verify QA did not mutate either candidate; these are task-owned files.
    if (fs::read(included)?, fs::read(excluded)?) != original {
        return Err("Direct indexing mutated a controlled fixture".into());
    }
    let bytes = (fs::metadata(included)?.len(), fs::metadata(excluded)?.len());
    Ok(
        json!({"route":"Direct MFT","phases":phases,"revokeOldWriterRejected":true,"reEnableOldWriterRejected":true,"restartBackfill":true,"fixtureLengths":bytes,"coverage":reopened.global_index_coverage()?}),
    )
}
fn baseline(db: &Database, id: &str) -> Result<(), Box<dyn Error>> {
    let source = GlobalSourceDescriptor {
        volume: db.get_global_volume(id)?.ok_or("source missing")?,
    };
    let mut sink = crate::global_index::coordinator::database_sink_for_qualification(db.clone());
    sink.admit_source_operation(
        &source.volume,
        &uuid::Uuid::new_v4().to_string(),
        SourceOperationKind::Rebuild,
        SourceOperationRoute::Direct,
    )?;
    DirectWindowsGlobalIndexProvider::new().start_initial_index(
        &source,
        &mut sink,
        &AtomicBool::new(false),
    )?;
    let now = crate::global_index::models::unix_now();
    sink.finish_source_operation(id, "ready", None, Some(now), Some(now), true)?;
    Ok(())
}
fn visible(db: &Database, path: &Path) -> Result<bool, Box<dyn Error>> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("invalid fixture name")?;
    let expected = crate::global_index::models::normalize_path(&path.to_string_lossy());
    Ok(db
        .search_global_entries(name, 25, 0)?
        .iter()
        .any(|entry| crate::global_index::models::normalize_path(&entry.path) == expected))
}
