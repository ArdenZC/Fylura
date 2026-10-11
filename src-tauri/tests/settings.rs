use rusqlite::{params, Connection};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
    time::{SystemTime, UNIX_EPOCH},
};

static TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);
use zen_canvas_tauri::{
    db::Database,
    settings::{
        get_app_settings, get_versioned_app_settings,
        reconcile_versioned_settings_side_effect_failure, save_app_settings, save_app_settings_cas,
        save_app_settings_with_launch_at_login, save_versioned_app_settings_with_launch_at_login,
        sync_launch_at_login_from_system, AppSettings, LaunchAtLoginController,
        SaveSettingsRequest, ScanRootSetting, APP_SETTINGS_KEY,
    },
};

#[test]
fn new_database_creates_default_app_settings_row() {
    let db = Database::open(test_db_path()).expect("open test database");
    let conn = Connection::open(db.path()).expect("open migrated database");

    let version: i64 = conn
        .query_row("SELECT user_version FROM pragma_user_version", [], |row| {
            row.get(0)
        })
        .expect("schema version");
    let value: String = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![APP_SETTINGS_KEY],
            |row| row.get(0),
        )
        .expect("default app settings row");
    let settings: AppSettings = serde_json::from_str(&value).expect("deserialize settings");

    assert_eq!(version, 37);
    assert_eq!(settings.close_behavior, "ask");
    assert_eq!(settings.folder_naming_language, "en");
    assert!(settings.default_scan_folders.is_empty());
    assert_eq!(settings.restore_retention_days, 30);
    assert!(!settings.launch_at_login);
    assert_eq!(settings.search_hotkey, "CmdOrCtrl+K");
}

#[test]
fn schema_7_database_migrates_to_settings_without_losing_existing_rows() {
    let path = test_db_path();
    create_schema_7_database(&path);

    let db = Database::open(&path).expect("migrate schema 7 database");
    let conn = Connection::open(db.path()).expect("open migrated database");

    let version: i64 = conn
        .query_row("SELECT user_version FROM pragma_user_version", [], |row| {
            row.get(0)
        })
        .expect("schema version");
    let file_name: String = conn
        .query_row(
            "SELECT name FROM files WHERE id = 'file-legacy'",
            [],
            |row| row.get(0),
        )
        .expect("legacy file");
    let rule_name: String = conn
        .query_row(
            "SELECT name FROM rules WHERE id = 'rule-legacy'",
            [],
            |row| row.get(0),
        )
        .expect("legacy rule");
    let default_settings = get_app_settings(&db).expect("default settings");

    assert_eq!(version, 37);
    assert_eq!(file_name, "legacy.pdf");
    assert_eq!(rule_name, "Legacy Rule");
    assert!(default_settings.default_scan_folders.is_empty());
    assert_eq!(default_settings.search_hotkey, "CmdOrCtrl+K");
}

#[test]
fn app_settings_roundtrip_persists_single_json_row() {
    let db = Database::open(test_db_path()).expect("open test database");
    let settings = AppSettings {
        close_behavior: "quit".to_string(),
        folder_naming_language: "zh".to_string(),
        default_scan_folders: vec![scan_root(
            "downloads",
            "/Users/zen/Downloads",
            "Downloads",
            true,
        )],
        restore_retention_days: 90,
        launch_at_login: true,
        search_hotkey: "Alt+Space".to_string(),
        ..AppSettings::default()
    };

    save_app_settings(&db, &settings).expect("save settings");
    let loaded = get_app_settings(&db).expect("load settings");
    let conn = Connection::open(db.path()).expect("open migrated database");
    let row_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM app_settings", [], |row| row.get(0))
        .expect("settings row count");

    assert_eq!(loaded.close_behavior, "quit");
    assert_eq!(loaded.folder_naming_language, "zh");
    assert_eq!(
        loaded.default_scan_folders,
        vec![scan_root(
            "downloads",
            "/Users/zen/Downloads",
            "Downloads",
            true
        )]
    );
    assert_eq!(loaded.restore_retention_days, 90);
    assert!(loaded.launch_at_login);
    assert_eq!(loaded.search_hotkey, "Alt+Space");
    assert_eq!(row_count, 1);
}

#[test]
fn clean_first_run_scan_scope_cas_syncs_one_enabled_root_and_survives_reload() {
    let database_path = test_db_path();
    let root_path = std::env::temp_dir().join(format!(
        "zen-canvas-onboarding-root-{}-{}",
        std::process::id(),
        TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root_path).expect("create permitted existing scan root");

    let db = Database::open(&database_path).expect("open clean settings database");
    let initial = get_versioned_app_settings(&db).expect("load clean default settings");
    assert!(initial.settings.default_scan_folders.is_empty());
    let root = scan_root(
        "onboarding-fixture",
        &root_path.to_string_lossy(),
        "fixture",
        true,
    );
    let mut next = initial.settings.clone();
    next.default_scan_folders = vec![root];
    let saved = save_versioned_app_settings_with_launch_at_login(
        &db,
        &SaveSettingsRequest {
            settings: next,
            expected_revision: initial.revision,
        },
        &RecordingLaunchAtLoginController::new(false),
    )
    .expect("CAS-persist first-run scan scope");
    db.sync_file_library_watcher_roots(&saved.settings.default_scan_folders)
        .expect("synchronize the settings-owned watcher root");
    drop(db);

    let db = Database::open(&database_path).expect("reopen clean settings database");
    let reloaded = get_versioned_app_settings(&db).expect("reload authoritative settings");
    assert_eq!(reloaded.revision, saved.revision);
    assert_eq!(
        reloaded.settings.default_scan_folders,
        saved.settings.default_scan_folders
    );
    let conn = Connection::open(db.path()).expect("open synchronized root database");
    let normalized_path = root_path.to_string_lossy().replace('\\', "/");
    let roots = conn
        .prepare(
            "SELECT normalized_path, enabled, source_kind FROM scan_roots WHERE lower(normalized_path) = lower(?1)",
        )
        .expect("prepare synchronized root lookup")
        .query_map(params![normalized_path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? != 0,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("query synchronized root")
        .collect::<Result<Vec<_>, _>>()
        .expect("read synchronized root rows");

    assert_eq!(roots.len(), 1);
    assert!(roots[0].1);
    assert_eq!(roots[0].2, "file_library");
    std::fs::remove_dir_all(&root_path).expect("remove disposable scan root");
}

#[test]
fn versioned_settings_return_backend_normalized_scan_root_fields() {
    let db = Database::open(test_db_path()).expect("open clean settings database");
    let initial = get_versioned_app_settings(&db).expect("load clean default settings");
    let mut next = initial.settings.clone();
    let mut root = scan_root(" ", "  C:\\OwnerQualification\\fixture\\  ", " ", true);
    root.created_at = " ".to_string();
    next.default_scan_folders = vec![root];

    let saved = save_app_settings_cas(&db, &next, initial.revision)
        .expect("save scan root using backend canonicalization");
    let normalized = &saved.settings.default_scan_folders[0];

    assert_eq!(normalized.path, "C:/OwnerQualification/fixture");
    assert_eq!(normalized.label, "fixture");
    assert_eq!(normalized.created_at, "1970-01-01T00:00:00.000Z");
    assert!(!normalized.id.trim().is_empty());
}

#[test]
fn side_effect_rollback_restores_settings_and_disables_the_synchronized_root() {
    let database_path = test_db_path();
    let root_path = std::env::temp_dir().join(format!(
        "zen-canvas-onboarding-rollback-root-{}-{}",
        std::process::id(),
        TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root_path).expect("create disposable scan root");

    let db = Database::open(&database_path).expect("open clean settings database");
    let previous = get_versioned_app_settings(&db).expect("load clean default settings");
    let root = scan_root(
        "onboarding-rollback",
        &root_path.to_string_lossy(),
        "fixture",
        true,
    );
    let mut next = previous.settings.clone();
    next.default_scan_folders = vec![root];
    let launch_controller = RecordingLaunchAtLoginController::new(false);
    let saved = save_versioned_app_settings_with_launch_at_login(
        &db,
        &SaveSettingsRequest {
            settings: next,
            expected_revision: previous.revision,
        },
        &launch_controller,
    )
    .expect("persist selected scan scope before watcher side effect");
    db.sync_file_library_watcher_roots(&saved.settings.default_scan_folders)
        .expect("synchronize selected root before injected watcher failure");

    let error = reconcile_versioned_settings_side_effect_failure(
        &db,
        &previous,
        &saved,
        &launch_controller,
        |restored_settings| {
            db.sync_file_library_watcher_roots(&restored_settings.default_scan_folders)
                .map_err(|error| error.to_string())?;
            Err("injected watcher restart failure".to_string())
        },
    )
    .expect_err("watcher side-effect failure remains visible after rollback");

    assert_eq!(
        error.to_string(),
        "settings side-effect reconciliation failed"
    );
    let reloaded = get_versioned_app_settings(&db).expect("reload rolled-back settings");
    assert!(reloaded.settings.default_scan_folders.is_empty());
    let conn = Connection::open(db.path()).expect("open synchronized root database");
    let enabled: i64 = conn
        .query_row(
            "SELECT enabled FROM scan_roots WHERE lower(normalized_path) = lower(?1)",
            params![root_path.to_string_lossy().replace('\\', "/")],
            |row| row.get(0),
        )
        .expect("read root after watcher failure rollback");
    assert_eq!(enabled, 0);
    std::fs::remove_dir_all(&root_path).expect("remove disposable scan root");
}

#[test]
fn stale_settings_revision_cannot_overwrite_a_newer_save() {
    let db = Database::open(test_db_path()).expect("open test database");
    let initial = get_versioned_app_settings(&db).expect("initial versioned settings");
    let mut newer = initial.settings.clone();
    newer.restore_retention_days = 90;
    let saved = save_app_settings_cas(&db, &newer, initial.revision).expect("save newer settings");

    let mut stale = initial.settings;
    stale.close_behavior = "quit".to_string();
    let error = save_app_settings_cas(&db, &stale, initial.revision)
        .expect_err("stale revision must conflict");

    assert_eq!(error.to_string(), "settings_revision_conflict");
    let loaded = get_versioned_app_settings(&db).expect("load winner");
    assert_eq!(loaded.revision, saved.revision);
    assert_eq!(loaded.settings.restore_retention_days, 90);
    assert_eq!(loaded.settings.close_behavior, newer.close_behavior);
}

#[test]
fn failed_runtime_side_effect_restores_database_and_autostart_intent() {
    let db = Database::open(test_db_path()).expect("open test database");
    let initial = get_versioned_app_settings(&db).expect("initial settings");
    let mut desired = initial.settings.clone();
    desired.launch_at_login = true;
    let controller = RecordingLaunchAtLoginController::new(false);
    let saved = save_versioned_app_settings_with_launch_at_login(
        &db,
        &SaveSettingsRequest {
            settings: desired,
            expected_revision: initial.revision,
        },
        &controller,
    )
    .expect("persist desired settings");

    let rollback = reconcile_versioned_settings_side_effect_failure(
        &db,
        &initial,
        &saved,
        &controller,
        |_| Ok(()),
    )
    .expect("reconcile failed watcher reload");

    assert_eq!(rollback.revision, initial.revision + 2);
    assert!(!rollback.settings.launch_at_login);
    assert_eq!(controller.calls(), vec!["enable", "disable"]);
    let loaded = get_versioned_app_settings(&db).expect("load rolled back settings");
    assert_eq!(loaded.revision, rollback.revision);
    assert!(!loaded.settings.launch_at_login);
}

#[test]
fn settings_cas_serializes_autostart_side_effects() {
    let db = Database::open(test_db_path()).expect("open test database");
    let initial = get_versioned_app_settings(&db).expect("initial settings");
    let mut desired = initial.settings.clone();
    desired.launch_at_login = true;
    let request = SaveSettingsRequest {
        settings: desired,
        expected_revision: initial.revision,
    };
    let controller = Arc::new(ConcurrentLaunchAtLoginController::default());
    let (entered_tx, entered_rx) = mpsc::channel();

    let first_db = db.clone();
    let first_request = request.clone();
    let first_controller = Arc::clone(&controller);
    let first = thread::spawn(move || {
        first_controller.mark_first_enable(entered_tx);
        save_versioned_app_settings_with_launch_at_login(
            &first_db,
            &first_request,
            first_controller.as_ref(),
        )
    });
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("first autostart mutation started");

    let second_db = db.clone();
    let second_controller = Arc::clone(&controller);
    let second = thread::spawn(move || {
        save_versioned_app_settings_with_launch_at_login(
            &second_db,
            &request,
            second_controller.as_ref(),
        )
    });

    let first_result = first.join().expect("first save thread");
    let second_result = second.join().expect("second save thread");
    let loaded = get_versioned_app_settings(&db).expect("load final settings");

    assert_ne!(first_result.is_ok(), second_result.is_ok());
    assert!(loaded.settings.launch_at_login);
    assert!(controller.is_enabled().expect("controller state"));
}

#[test]
fn legacy_string_default_scan_folders_load_as_absolute_scan_roots() {
    let db = Database::open(test_db_path()).expect("open test database");
    let conn = Connection::open(db.path()).expect("open migrated database");
    conn.execute(
        r#"
        INSERT INTO app_settings (key, value)
        VALUES (?1, ?2)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value
        "#,
        params![
            APP_SETTINGS_KEY,
            r#"{
              "closeBehavior":"ask",
              "folderNamingLanguage":"en",
              "defaultScanFolders":["Desktop","Downloads","Documents"],
              "restoreRetentionDays":30,
              "launchAtLogin":false
            }"#
        ],
    )
    .expect("insert legacy settings");

    let loaded = get_app_settings(&db).expect("load migrated legacy settings");

    assert_legacy_scan_roots(&loaded.default_scan_folders);
    assert_eq!(loaded.search_hotkey, "CmdOrCtrl+K");
}

#[test]
fn prune_operation_logs_removes_expired_logs_and_orphan_batches() {
    let db = Database::open(test_db_path()).expect("open test database");
    let conn = Connection::open(db.path()).expect("open migrated database");
    let recent_timestamp = current_timestamp_ms();

    insert_batch(&conn, "batch-old", 0);
    insert_batch(&conn, "batch-recent", recent_timestamp);
    insert_batch(&conn, "batch-orphan", 0);
    insert_operation_log(&conn, "log-old", "batch-old", 0);
    insert_operation_log(&conn, "log-recent", "batch-recent", recent_timestamp);

    db.prune_operation_logs(30).expect("prune operation logs");

    let log_ids = string_column_values(&conn, "SELECT id FROM operation_logs ORDER BY id");
    let batch_ids = string_column_values(&conn, "SELECT id FROM operation_batches ORDER BY id");

    assert_eq!(log_ids, vec!["log-recent"]);
    assert_eq!(batch_ids, vec!["batch-recent"]);
}

#[test]
fn save_settings_with_launch_at_login_enables_autostart_before_persisting() {
    let db = Database::open(test_db_path()).expect("open test database");
    let controller = RecordingLaunchAtLoginController::new(false);
    let settings = AppSettings {
        launch_at_login: true,
        ..AppSettings::default()
    };

    save_app_settings_with_launch_at_login(&db, &settings, &controller).expect("save settings");
    let loaded = get_app_settings(&db).expect("load settings");

    assert!(loaded.launch_at_login);
    assert_eq!(controller.calls(), vec!["enable"]);
}

#[test]
fn save_settings_with_launch_at_login_disables_autostart_before_persisting() {
    let db = Database::open(test_db_path()).expect("open test database");
    let controller = RecordingLaunchAtLoginController::new(true);
    let mut settings = AppSettings {
        launch_at_login: true,
        ..AppSettings::default()
    };
    save_app_settings(&db, &settings).expect("save enabled settings");
    settings.launch_at_login = false;

    save_app_settings_with_launch_at_login(&db, &settings, &controller).expect("save settings");
    let loaded = get_app_settings(&db).expect("load settings");

    assert!(!loaded.launch_at_login);
    assert_eq!(controller.calls(), vec!["disable"]);
}

#[test]
fn save_settings_with_launch_at_login_does_not_persist_when_autostart_sync_fails() {
    let db = Database::open(test_db_path()).expect("open test database");
    let controller = RecordingLaunchAtLoginController::new(false).fail_enable();
    let settings = AppSettings {
        launch_at_login: true,
        ..AppSettings::default()
    };

    let result = save_app_settings_with_launch_at_login(&db, &settings, &controller);
    let loaded = get_app_settings(&db).expect("load settings");

    assert!(result.is_err());
    assert!(!loaded.launch_at_login);
    assert_eq!(controller.calls(), vec!["enable"]);
}

#[test]
fn startup_launch_at_login_sync_uses_system_truth() {
    let db = Database::open(test_db_path()).expect("open test database");
    let controller = RecordingLaunchAtLoginController::new(false);
    let settings = AppSettings {
        launch_at_login: true,
        ..AppSettings::default()
    };
    save_app_settings(&db, &settings).expect("save stale settings");

    let synced =
        sync_launch_at_login_from_system(&db, &settings, &controller).expect("sync settings");
    let loaded = get_app_settings(&db).expect("load settings");

    assert!(!synced.launch_at_login);
    assert!(!loaded.launch_at_login);
    assert_eq!(controller.calls(), vec!["is_enabled"]);
}

fn create_schema_7_database(path: &Path) {
    let conn = Connection::open(path).expect("open schema 7 database");
    conn.execute_batch(
        r#"
        CREATE TABLE files (
            id TEXT PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            extension TEXT NOT NULL DEFAULT '',
            size INTEGER NOT NULL DEFAULT 0,
            mtime INTEGER NOT NULL DEFAULT 0,
            is_dir INTEGER NOT NULL DEFAULT 0 CHECK (is_dir IN (0, 1)),
            state_code INTEGER NOT NULL DEFAULT 0,
            file_type TEXT NOT NULL DEFAULT 'Other',
            purpose TEXT NOT NULL DEFAULT 'Unknown',
            lifecycle TEXT NOT NULL DEFAULT 'Inbox',
            context TEXT NOT NULL DEFAULT '',
            risk_level TEXT NOT NULL DEFAULT 'Normal',
            suggested_action TEXT NOT NULL DEFAULT 'Keep',
            suggested_target_path TEXT NOT NULL DEFAULT '',
            suggested_name TEXT NOT NULL DEFAULT '',
            confidence REAL NOT NULL DEFAULT 0.5,
            classification_reason TEXT NOT NULL DEFAULT 'Indexed by Zen Canvas Tauri backend.',
            matched_rules TEXT NOT NULL DEFAULT '[]',
            requires_confirmation INTEGER NOT NULL DEFAULT 0,
            ctime INTEGER NOT NULL DEFAULT 0,
            is_stale INTEGER NOT NULL DEFAULT 0,
            last_seen_at INTEGER NOT NULL DEFAULT 0,
            last_classified_at INTEGER NOT NULL DEFAULT 0,
            classified_rule_version TEXT NOT NULL DEFAULT '',
            last_classified_mtime INTEGER NOT NULL DEFAULT 0,
            last_classified_size INTEGER NOT NULL DEFAULT 0
        );
        INSERT INTO files (id, path, name, extension, size)
        VALUES ('file-legacy', '/legacy/legacy.pdf', 'legacy.pdf', 'pdf', 2048);
        CREATE TABLE rules (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL
        );
        INSERT INTO rules (id, name) VALUES ('rule-legacy', 'Legacy Rule');
        PRAGMA user_version = 7;
        "#,
    )
    .expect("create schema 7 database");
}

fn insert_batch(conn: &Connection, id: &str, created_at: i64) {
    conn.execute(
        "INSERT INTO operation_batches (id, created_at, status) VALUES (?1, ?2, 'success')",
        params![id, created_at],
    )
    .expect("insert operation batch");
}

fn insert_operation_log(conn: &Connection, id: &str, batch_id: &str, created_at: i64) {
    conn.execute(
        r#"
        INSERT INTO operation_logs (
            id,
            batch_id,
            operation_type,
            source_path,
            target_path,
            old_name,
            new_name,
            status,
            error_message,
            created_at,
            can_undo,
            path_before,
            path_after,
            name_before,
            name_after,
            can_restore,
            restored_at,
            restore_status,
            restore_error
        )
        VALUES (?1, ?2, 'move', '/tmp/source.txt', '/tmp/target.txt', 'source.txt', 'target.txt', 'success', NULL, ?3, 1, '/tmp/source.txt', '/tmp/target.txt', 'source.txt', 'target.txt', 1, NULL, 'not_restored', NULL)
        "#,
        params![id, batch_id, created_at],
    )
    .expect("insert operation log");
}

fn scan_root(id: &str, path: &str, label: &str, enabled: bool) -> ScanRootSetting {
    ScanRootSetting {
        id: id.to_string(),
        path: path.to_string(),
        label: label.to_string(),
        enabled,
        created_at: "2026-06-22T00:00:00.000Z".to_string(),
    }
}

fn assert_legacy_scan_roots(roots: &[ScanRootSetting]) {
    assert_eq!(roots.len(), 3);
    for label in ["Desktop", "Downloads", "Documents"] {
        let root = roots
            .iter()
            .find(|root| root.label == label)
            .unwrap_or_else(|| panic!("missing {label} root"));
        assert!(root.enabled);
        assert!(!root.id.is_empty());
        assert!(!root.created_at.is_empty());
        assert!(
            root.path.replace('\\', "/").ends_with(&format!("/{label}")),
            "{} did not end with /{label}",
            root.path
        );
    }
}

fn string_column_values(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).expect("prepare query");
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query rows");
    rows.map(|row| row.expect("row value")).collect()
}

fn current_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).expect("timestamp fits i64"))
        .expect("clock")
}

#[derive(Default)]
struct RecordingLaunchAtLoginController {
    enabled: bool,
    fail_enable: bool,
    calls: std::cell::RefCell<Vec<&'static str>>,
}

impl RecordingLaunchAtLoginController {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            fail_enable: false,
            calls: std::cell::RefCell::new(Vec::new()),
        }
    }

    fn fail_enable(mut self) -> Self {
        self.fail_enable = true;
        self
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls.borrow().clone()
    }
}

impl LaunchAtLoginController for RecordingLaunchAtLoginController {
    fn enable(&self) -> Result<(), String> {
        self.calls.borrow_mut().push("enable");
        if self.fail_enable {
            Err("enable failed".to_string())
        } else {
            Ok(())
        }
    }

    fn disable(&self) -> Result<(), String> {
        self.calls.borrow_mut().push("disable");
        Ok(())
    }

    fn is_enabled(&self) -> Result<bool, String> {
        self.calls.borrow_mut().push("is_enabled");
        Ok(self.enabled)
    }
}

#[derive(Default)]
struct ConcurrentLaunchAtLoginController {
    state: Mutex<ConcurrentLaunchState>,
}

#[derive(Default)]
struct ConcurrentLaunchState {
    enabled: bool,
    first_enable_sender: Option<mpsc::Sender<()>>,
    enable_calls: usize,
}

impl ConcurrentLaunchAtLoginController {
    fn mark_first_enable(&self, sender: mpsc::Sender<()>) {
        self.state
            .lock()
            .expect("controller state")
            .first_enable_sender = Some(sender);
    }
}

impl LaunchAtLoginController for ConcurrentLaunchAtLoginController {
    fn enable(&self) -> Result<(), String> {
        let sender = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "state poisoned".to_string())?;
            state.enabled = true;
            state.enable_calls += 1;
            if state.enable_calls == 1 {
                state.first_enable_sender.take()
            } else {
                None
            }
        };
        if let Some(sender) = sender {
            let _ = sender.send(());
            thread::sleep(Duration::from_millis(250));
        }
        Ok(())
    }

    fn disable(&self) -> Result<(), String> {
        self.state
            .lock()
            .map_err(|_| "state poisoned".to_string())?
            .enabled = false;
        Ok(())
    }

    fn is_enabled(&self) -> Result<bool, String> {
        self.state
            .lock()
            .map(|state| state.enabled)
            .map_err(|_| "state poisoned".to_string())
    }
}

fn test_db_path() -> PathBuf {
    let counter = TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "zen-canvas-settings-test-{}-{counter}-{nonce}.sqlite3",
        std::process::id()
    ))
}
