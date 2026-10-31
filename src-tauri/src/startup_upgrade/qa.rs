//! Native widget probe, compiled only with native-qa. A deliberate 1.5-second
//! worker delay makes cancellation/retry observable without a large fixture.
//! This delay is never used for the migration cost benchmark.
use super::*;
use std::sync::{atomic::AtomicUsize, Mutex};
static ACTIVE: AtomicBool = AtomicBool::new(false);
static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static TICKS: AtomicUsize = AtomicUsize::new(0);
static CANCELLED: AtomicBool = AtomicBool::new(false);
static RETRIED: AtomicBool = AtomicBool::new(false);
static VISIBLE: AtomicBool = AtomicBool::new(true);
static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
pub(super) fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}
pub(super) fn event(name: &str) {
    if active() {
        EVENTS.lock().unwrap().push(name.into());
    }
}
pub(super) fn displayed(visible: bool) {
    if active() {
        VISIBLE.fetch_and(visible, Ordering::AcqRel);
        event("native_window_visible_before_worker");
    }
}
pub(super) fn starting() {
    if active() {
        ATTEMPTS.fetch_add(1, Ordering::AcqRel);
        event("worker_start");
    }
}
pub(super) fn delay_worker() {
    if active() {
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
}
pub(super) fn cancel_on_tick() -> bool {
    if !active() {
        return false;
    }
    let ticks = TICKS.fetch_add(1, Ordering::AcqRel) + 1;
    event("native_timer_tick");
    ticks >= 2 && ATTEMPTS.load(Ordering::Acquire) == 1 && !CANCELLED.swap(true, Ordering::AcqRel)
}
pub(super) fn retried() {
    RETRIED.store(true, Ordering::Release);
    event("native_retry_button");
}

fn verify_background_deferral(source: &std::path::Path, installed: i64) -> Result<(), String> {
    let profile = source.with_extension("background-profile");
    std::fs::create_dir(&profile).map_err(|e| e.to_string())?;
    let path = profile.join("fylura.sqlite3");
    std::fs::copy(source, &path).map_err(|e| e.to_string())?;
    let conn = rusqlite::Connection::open(&path).map_err(|e| e.to_string())?;
    historical_schema(&conn, installed)?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .map_err(|e| e.to_string())?;
    drop(conn);
    let mut child = std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .arg("--background")
        .env("ZC_NATIVE_QA_PROFILE_ROOT", &profile)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Background deferral did not exit within 30 seconds".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let logs = String::from_utf8_lossy(&output.stderr);
    if !output.status.success()
        || !logs.contains("startup_checkpoint=tauri_setup_entered")
        || logs.contains("startup_checkpoint=database_ready")
        || logs.contains("native_dialog_displayed")
    {
        return Err(format!(
            "Background deferral failed: status={} log={logs}",
            output.status
        ));
    }
    let conn = rusqlite::Connection::open(&path).map_err(|e| e.to_string())?;
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if version != installed {
        return Err(format!(
            "Background launch unexpectedly migrated schema to {version}"
        ));
    }
    drop(conn);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    std::fs::remove_dir(profile).map_err(|e| e.to_string())?;
    Ok(())
}

fn historical_schema(conn: &rusqlite::Connection, installed: i64) -> Result<(), String> {
    // Test fixture only. Production never lowers a schema version.
    conn.execute_batch(
        "DROP TRIGGER global_entry_source_facts_ai;
        DROP TRIGGER global_entry_source_facts_ad;
        DROP TRIGGER global_entry_source_facts_au;
        DROP TRIGGER global_entry_source_facts_volume_ai;
        DROP TRIGGER global_entry_source_facts_volume_au;
        DROP TABLE global_entry_source_facts;",
    )
    .map_err(|e| e.to_string())?;
    if installed == 42 {
        conn.execute_batch("DROP INDEX idx_global_entries_active_name_order; DROP INDEX idx_global_entries_active_source_facts; CREATE INDEX idx_global_entries_active_name_order ON global_entries(name_normalized,modified_at_fs DESC,id ASC) WHERE is_stale=0;").map_err(|e|e.to_string())?;
    }
    conn.pragma_update(None, "user_version", installed)
        .map_err(|e| e.to_string())
}

pub(super) fn run() -> Result<(), String> {
    let mut routes = Vec::new();
    for installed in [42, 43] {
        let path = std::env::temp_dir().join(format!(
            "zen-canvas-upgrade-ui-{}-{installed}.db",
            std::process::id()
        ));
        if path.exists() {
            return Err("Native QA requires a fresh task-owned database path".into());
        }
        drop(Database::open(&path).map_err(|e| e.to_string())?);
        let conn = rusqlite::Connection::open(&path).map_err(|e| e.to_string())?;
        historical_schema(&conn, installed)?;
        drop(conn);
        ATTEMPTS.store(0, Ordering::Release);
        TICKS.store(0, Ordering::Release);
        CANCELLED.store(false, Ordering::Release);
        RETRIED.store(false, Ordering::Release);
        VISIBLE.store(true, Ordering::Release);
        EVENTS.lock().unwrap().clear();
        ACTIVE.store(true, Ordering::Release);
        let result = open(path.clone(), false);
        ACTIVE.store(false, Ordering::Release);
        let db = result?;
        let conn = rusqlite::Connection::open(&path).map_err(|e| e.to_string())?;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let events = EVENTS.lock().unwrap().clone();
        let starts = events
            .iter()
            .enumerate()
            .filter(|(_, e)| e.as_str() == "worker_start")
            .all(|(i, _)| {
                events[..i]
                    .iter()
                    .filter(|e| e.as_str() == "native_window_visible_before_worker")
                    .count()
                    > events[..i]
                        .iter()
                        .filter(|e| e.as_str() == "worker_start")
                        .count()
            });
        if version != i64::from(fylura_tauri::db::SUPPORTED_SCHEMA_VERSION)
            || ATTEMPTS.load(Ordering::Acquire) != 2
            || TICKS.load(Ordering::Acquire) < 4
            || !CANCELLED.load(Ordering::Acquire)
            || !RETRIED.load(Ordering::Acquire)
            || !VISIBLE.load(Ordering::Acquire)
            || !starts
        {
            return Err(format!("Native UI probe failed: installed={installed}, version={version}, events={events:?}"));
        }
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .map_err(|e| e.to_string())?;
        verify_background_deferral(&path, installed)?;
        routes.push(serde_json::json!({"installed_schema":installed,"schema":version,"visible_before_worker":true,"attempts":2,"native_timer_ticks":TICKS.load(Ordering::Acquire),"cancel_and_retry_native_buttons":true,"background_deferral_real_tauri_process":true,"background_exit_code":0,"background_schema":installed,"events":events}));
        drop(conn);
        drop(db);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }
    let report = serde_json::json!({"record_type":"native_upgrade_ui","platform":std::env::consts::OS,"status":"PASS","routes":routes,"scope":"Actual native widgets/message loop with deliberate worker delay; synthetic button clicks. Human keyboard/screen-reader/contrast, packaged first launch and Owner Windows acceptance remain unverified."});
    let output = std::env::var_os("ZC_UPGRADE_UI_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("schema-upgrade-ui.json"));
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap())
        .map_err(|e| e.to_string())?;
    println!("{report}");
    Ok(())
}
