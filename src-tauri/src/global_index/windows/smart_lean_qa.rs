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
use std::{error::Error, fs, time::Instant};

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
    if db.global_index_coverage()?.policy.mode != CoverageMode::SmartLean {
        return Err("new Windows profile did not default to Smart Lean".into());
    }
    let mut unapproved = enabled[0].clone();
    unapproved.id = format!("qa-unapproved-{}", uuid::Uuid::new_v4());
    unapproved.stable_volume_id = format!("qa-unapproved-{}", uuid::Uuid::new_v4());
    unapproved.enabled = false;
    unapproved.scope_authorization_state = "not_authorized".into();
    let new_user_profile = task.join("smart-lean-no-authorization-profile");
    fs::create_dir_all(&new_user_profile)?;
    let new_user_db = Database::open(new_user_profile.join("fylura.sqlite3"))?;
    new_user_db.upsert_global_volume(&unapproved)?;
    if new_user_db.global_index_coverage()?.policy.mode != CoverageMode::SmartLean {
        return Err("unauthorized new user profile did not default to Smart Lean".into());
    }
    let mut unauth_sink =
        crate::global_index::coordinator::database_sink_for_qualification(new_user_db.clone());
    let unauthorized_attempt = DirectWindowsGlobalIndexProvider::new().start_initial_index(
        &GlobalSourceDescriptor {
            volume: unapproved.clone(),
        },
        &mut unauth_sink,
        &AtomicBool::new(false),
    );
    if !unauthorized_attempt.err().is_some_and(|error| {
        error
            .to_string()
            .contains("global_index_source_authorization_required")
    }) {
        return Err("unauthorized new user source was not rejected before indexing".into());
    }
    let unapproved_rows: i64 = new_user_db.conn()?.query_row(
        "SELECT COUNT(*) FROM global_entries WHERE volume_id=?1",
        [&unapproved.id],
        |row| row.get(0),
    )?;
    if unapproved_rows != 0 {
        return Err("unauthorized new user source wrote index rows".into());
    }
    drop(unauth_sink);
    drop(new_user_db);
    if !included.is_file() || !excluded.is_file() {
        return Err("qualification fixtures missing".into());
    }
    if fs::metadata(included)?.len() > 4096 || fs::metadata(excluded)?.len() > 4096 {
        return Err("qualification fixtures exceed the bounded sample size".into());
    }
    let original = (fs::read(included)?, fs::read(excluded)?);
    let source_root = Path::new(&enabled[0].mount_path);
    let document = source_root.join("documents/zcsmartlean-native-document.txt");
    let photo = source_root.join("pictures/zcsmartlean-native-photo.jpg");
    let code = source_root.join("project/src/zcsmartlean-native-module.rs");
    let lookalikes = [
        source_root.join("Projects/Windows/valuable-note.txt"),
        source_root.join("AppData/important-setting.json"),
        source_root.join("build/valuable-design.pdf"),
        source_root.join("cache/indexed-report.pdf"),
        source_root.join("tmp/meeting-notes.txt"),
        source_root.join("logs/important.log"),
    ];
    let custom_exclude = source_root.join("custom-policy/private");
    let custom_excluded_file = custom_exclude.join("valuable-private-note.txt");
    let custom_include = excluded
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "node_modules"))
        .ok_or("dependency fixture is not under node_modules")?
        .join("user-preserved");
    let custom_included_file = custom_include.join("valuable-user-file.txt");
    let temporary_root = std::env::temp_dir();
    if !temporary_root.starts_with(source_root) {
        return Err(
            "native Smart Lean temporary-root fixture is outside the authorized VHD".into(),
        );
    }
    let temporary_file = temporary_root.join("zcsmartlean-native-temporary-file.txt");
    for directory in [
        document.parent().unwrap(),
        photo.parent().unwrap(),
        code.parent().unwrap(),
        custom_exclude.as_path(),
        custom_include.as_path(),
        temporary_root.as_path(),
    ] {
        fs::create_dir_all(directory)?;
    }
    fs::write(&document, b"ordinary document")?;
    fs::write(&photo, b"ordinary photo fixture")?;
    fs::write(&code, b"ordinary project source")?;
    fs::write(&custom_excluded_file, b"custom excluded")?;
    fs::write(&custom_included_file, b"custom included")?;
    fs::write(&temporary_file, b"task-owned temporary file")?;
    for path in &lookalikes {
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, b"valuable lookalike")?;
    }
    let mut phases = Vec::new();
    let mut first_searchable_us = None;
    for mode in [
        CoverageMode::SmartLean,
        CoverageMode::BroadCoverage,
        CoverageMode::Custom,
    ] {
        let source = db.get_global_volume(source_id)?.ok_or("source missing")?;
        let rules = if mode == CoverageMode::Custom {
            vec![
                bind_rule(&source, &custom_exclude.to_string_lossy(), false)?,
                bind_rule(&source, &custom_include.to_string_lossy(), true)?,
            ]
        } else {
            vec![]
        };
        db.save_global_index_coverage(db.global_index_coverage()?.policy.revision, mode, rules)?;
        let first_search_started = (mode == CoverageMode::SmartLean).then(Instant::now);
        baseline(&db, source_id)?;
        let included_visible = visible(&db, included)?;
        if let Some(started) = first_search_started {
            let elapsed_us = started.elapsed().as_micros();
            first_searchable_us = Some(elapsed_us);
            crate::global_index::qa_trace::record(&format!(
                "mft_first_searchable elapsed_us={elapsed_us} visible={included_visible}"
            ));
        }
        let excluded_visible = visible(&db, excluded)?;
        if !included_visible || excluded_visible != (mode == CoverageMode::BroadCoverage) {
            return Err(format!("direct native coverage mismatch: mode={mode:?} included={included_visible} low_value={excluded_visible}").into());
        }
        for path in [document.as_path(), photo.as_path(), code.as_path()] {
            if !visible(&db, path)? {
                return Err(format!("ordinary document/photo/source fixture was not searchable: mode={mode:?} path={}", path.display()).into());
            }
        }
        for path in &lookalikes {
            if !visible(&db, path)? {
                return Err(format!(
                    "valuable same-name folder was excluded: mode={mode:?} path={}",
                    path.display()
                )
                .into());
            }
        }
        let temporary_visible = visible(&db, &temporary_file)?;
        if temporary_visible != (mode == CoverageMode::BroadCoverage) {
            return Err(format!(
                "native OS temporary root mismatch: mode={mode:?} visible={temporary_visible}"
            )
            .into());
        }
        let custom_excluded_visible = visible(&db, &custom_excluded_file)?;
        let custom_included_visible = visible(&db, &custom_included_file)?;
        if custom_excluded_visible != (mode != CoverageMode::Custom)
            || custom_included_visible != (mode != CoverageMode::SmartLean)
        {
            return Err(format!("custom Include/Exclude mismatch: mode={mode:?} excluded={custom_excluded_visible} included={custom_included_visible}").into());
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
    let reopen_backfill_started = Instant::now();
    let reopened = Database::open(profile.join("fylura.sqlite3"))?;
    reopened.save_global_index_coverage(
        reopened.global_index_coverage()?.policy.revision,
        CoverageMode::SmartLean,
        vec![],
    )?;
    baseline(&reopened, source_id)?;
    let reopened_included_visible = visible(&reopened, included)?;
    let reopen_backfill_us = reopen_backfill_started.elapsed().as_micros();
    crate::global_index::qa_trace::record(&format!(
        "mft_database_reopen_backfill elapsed_us={reopen_backfill_us} visible={reopened_included_visible}"
    ));
    if !reopened_included_visible || visible(&reopened, excluded)? {
        return Err("Direct restart/backfill mismatch".into());
    }
    // Verify QA did not mutate either candidate; these are task-owned files.
    if (fs::read(included)?, fs::read(excluded)?) != original {
        return Err("Direct indexing mutated a controlled fixture".into());
    }
    let bytes = (fs::metadata(included)?.len(), fs::metadata(excluded)?.len());
    Ok(
        json!({"route":"Direct MFT","phases":phases,"revokeOldWriterRejected":true,"reEnableOldWriterRejected":true,"restartBackfill":true,"firstSearchableUs":first_searchable_us,"databaseReopenBackfillUs":reopen_backfill_us,"fixtureLengths":bytes,"coverage":reopened.global_index_coverage()?}),
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
