use crate::{
    db::{Database, DbError},
    dedupe::DedupeJobManager,
    scanner::ScanJobManager,
    watcher::{
        emit_file_watcher_error, reload_file_watcher_for_settings_with_stage, FileWatcherManager,
        SettingsWatcherReloadFailure,
    },
    window_auth::require_main_window,
};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
#[cfg(feature = "native-qa")]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};
use tauri::{AppHandle, Runtime, State, WebviewWindow};
use tauri_plugin_autostart::{AutoLaunchManager, ManagerExt};
use thiserror::Error;

pub const APP_SETTINGS_KEY: &str = "app_settings_v1";
pub const DEFAULT_SEARCH_HOTKEY: &str = "CmdOrCtrl+K";
const DEFAULT_SCAN_ROOT_CREATED_AT: &str = "1970-01-01T00:00:00.000Z";
static VERSIONED_SETTINGS_SAVE_LOCK: Mutex<()> = Mutex::new(());
#[cfg(feature = "native-qa")]
static NATIVE_QA_SETTINGS_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);

fn native_qa_settings_trace(event: &'static str, details: impl FnOnce() -> String) {
    #[cfg(feature = "native-qa")]
    {
        if std::env::var("ZC_NATIVE_QA_SETTINGS_TRACE").as_deref() != Ok("1") {
            return;
        }
        let sequence = NATIVE_QA_SETTINGS_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
        if sequence >= 64 {
            return;
        }
        let details = details().chars().take(500).collect::<String>();
        eprintln!("native_qa settings event={event} {details}");
    }
    #[cfg(not(feature = "native-qa"))]
    {
        let _ = (event, details);
    }
}

fn enabled_scan_root_count(settings: &AppSettings) -> usize {
    settings
        .default_scan_folders
        .iter()
        .filter(|root| root.enabled && !root.path.trim().is_empty())
        .count()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsSaveFailureCode {
    DatabaseFailure,
    RevisionConflict,
    WatcherRootSyncFailure,
    WatcherRuntimeFailure,
    WatcherReconciliationScheduleFailure,
    RollbackReconciliationFailure,
    UnknownFailure,
}

impl SettingsSaveFailureCode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::DatabaseFailure => "database_failure",
            Self::RevisionConflict => "revision_conflict",
            Self::WatcherRootSyncFailure => "watcher_root_sync_failure",
            Self::WatcherRuntimeFailure => "watcher_runtime_failure",
            Self::WatcherReconciliationScheduleFailure => "watcher_reconciliation_schedule_failure",
            Self::RollbackReconciliationFailure => "rollback_reconciliation_failure",
            Self::UnknownFailure => "unknown_failure",
        }
    }

    fn as_error_message(self) -> String {
        format!("settings_save_failure:{}", self.as_str())
    }

    const fn is_watcher_failure(self) -> bool {
        matches!(
            self,
            Self::WatcherRootSyncFailure
                | Self::WatcherRuntimeFailure
                | Self::WatcherReconciliationScheduleFailure
        )
    }
}

fn versioned_settings_save_guard() -> MutexGuard<'static, ()> {
    VERSIONED_SETTINGS_SAVE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScanRootSetting {
    pub id: String,
    pub path: String,
    pub label: String,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchRootSetting {
    pub id: String,
    pub path: String,
    pub label: String,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrganizeRootMode {
    CurrentFolder,
    ZenCanvasFolder,
    CustomRoot,
}

fn default_organize_root_mode() -> OrganizeRootMode {
    OrganizeRootMode::CurrentFolder
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub close_behavior: String,
    pub folder_naming_language: String,
    #[serde(
        default = "default_scan_roots",
        deserialize_with = "deserialize_scan_roots"
    )]
    pub default_scan_folders: Vec<ScanRootSetting>,
    pub restore_retention_days: i64,
    pub launch_at_login: bool,
    #[serde(default = "default_background_index_on_startup")]
    pub background_index_on_startup: bool,
    #[serde(default = "default_search_hotkey")]
    pub search_hotkey: String,
    #[serde(default = "default_search_scope_mode")]
    pub search_scope_mode: String,
    #[serde(
        default = "default_search_roots",
        deserialize_with = "deserialize_search_roots"
    )]
    pub custom_search_roots: Vec<SearchRootSetting>,
    #[serde(default = "default_organize_root_mode")]
    pub organize_root_mode: OrganizeRootMode,
    #[serde(default)]
    pub organize_root_path: Option<String>,
    #[serde(default)]
    pub use_legacy_builtin_classification_rules: bool,
    #[serde(default)]
    pub use_learned_rules_as_auto_rules: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            close_behavior: "ask".to_string(),
            folder_naming_language: "en".to_string(),
            default_scan_folders: default_scan_roots(),
            restore_retention_days: 30,
            launch_at_login: false,
            background_index_on_startup: default_background_index_on_startup(),
            search_hotkey: DEFAULT_SEARCH_HOTKEY.to_string(),
            search_scope_mode: default_search_scope_mode(),
            custom_search_roots: default_search_roots(),
            organize_root_mode: OrganizeRootMode::CurrentFolder,
            organize_root_path: None,
            use_legacy_builtin_classification_rules: false,
            use_learned_rules_as_auto_rules: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionedAppSettings {
    pub settings: AppSettings,
    pub revision: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSettingsRequest {
    pub settings: AppSettings,
    pub expected_revision: i64,
}

pub fn default_settings_json() -> Result<String, DbError> {
    serde_json::to_string(&AppSettings::default()).map_err(DbError::from)
}

pub fn get_app_settings(db: &Database) -> Result<AppSettings, DbError> {
    Ok(get_versioned_app_settings(db)?.settings)
}

pub fn get_versioned_app_settings(db: &Database) -> Result<VersionedAppSettings, DbError> {
    let conn = db.conn()?;
    let row = conn
        .query_row(
            "SELECT value, revision FROM app_settings WHERE key = ?1",
            params![APP_SETTINGS_KEY],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;

    match row {
        Some((value, revision)) => Ok(VersionedAppSettings {
            settings: serde_json::from_str(&value)?,
            revision,
        }),
        None => Ok(VersionedAppSettings {
            settings: AppSettings::default(),
            revision: 0,
        }),
    }
}

pub fn save_app_settings(db: &Database, settings: &AppSettings) -> Result<(), DbError> {
    let _catalog_guard = crate::db::catalog_execution_guard();
    let mut conn = db.conn()?;
    let tx = conn.transaction()?;
    let normalized = normalized_app_settings(settings);
    let previous = tx
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![APP_SETTINGS_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .and_then(|value| serde_json::from_str::<AppSettings>(&value).ok());
    let settings_json = serde_json::to_string(&normalized)?;
    tx.execute(
        r#"
        INSERT INTO app_settings (key, value)
        VALUES (?1, ?2)
        ON CONFLICT(key) DO UPDATE SET
            value = excluded.value,
            revision = app_settings.revision + 1
        "#,
        params![APP_SETTINGS_KEY, settings_json],
    )?;
    if previous
        .as_ref()
        .is_none_or(|previous| settings_affects_rule_catalog(previous, &normalized))
    {
        crate::db::bump_catalog_revision_unconditional(&tx)?;
    }
    tx.commit()?;
    Ok(())
}

pub fn save_app_settings_cas(
    db: &Database,
    settings: &AppSettings,
    expected_revision: i64,
) -> Result<VersionedAppSettings, SettingsError> {
    save_app_settings_cas_with_behavior(
        db,
        settings,
        expected_revision,
        TransactionBehavior::Immediate,
    )
}

fn save_app_settings_cas_with_behavior(
    db: &Database,
    settings: &AppSettings,
    expected_revision: i64,
    transaction_behavior: TransactionBehavior,
) -> Result<VersionedAppSettings, SettingsError> {
    let _catalog_guard = crate::db::catalog_execution_guard();
    let mut conn = db.conn().map_err(SettingsError::Db)?;
    let tx = conn
        .transaction_with_behavior(transaction_behavior)
        .map_err(DbError::from)
        .map_err(SettingsError::Db)?;
    let normalized = normalized_app_settings(settings);
    let settings_json = serde_json::to_string(&normalized).map_err(DbError::from)?;
    let previous_json = tx
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![APP_SETTINGS_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(DbError::from)
        .map_err(SettingsError::Db)?;
    let previous = previous_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<AppSettings>(value).ok());
    let changed = tx.execute(
        "UPDATE app_settings SET value = ?1, revision = revision + 1 WHERE key = ?2 AND revision = ?3",
        params![settings_json, APP_SETTINGS_KEY, expected_revision],
    )
    .map_err(DbError::from)?;
    if changed == 0 {
        return Err(SettingsError::RevisionConflict);
    }
    if previous
        .as_ref()
        .is_none_or(|previous| settings_affects_rule_catalog(previous, &normalized))
    {
        crate::db::bump_catalog_revision_unconditional(&tx).map_err(SettingsError::Db)?;
    }
    tx.commit()
        .map_err(DbError::from)
        .map_err(SettingsError::Db)?;
    Ok(VersionedAppSettings {
        settings: normalized,
        revision: expected_revision + 1,
    })
}

#[cfg(test)]
pub(crate) fn save_app_settings_cas_for_test(
    db: &Database,
    settings: &AppSettings,
    expected_revision: i64,
    transaction_behavior: TransactionBehavior,
) -> Result<VersionedAppSettings, SettingsError> {
    save_app_settings_cas_with_behavior(db, settings, expected_revision, transaction_behavior)
}

fn settings_affects_rule_catalog(previous: &AppSettings, next: &AppSettings) -> bool {
    previous.folder_naming_language != next.folder_naming_language
        || previous.organize_root_mode != next.organize_root_mode
        || previous.organize_root_path != next.organize_root_path
        || previous.use_legacy_builtin_classification_rules
            != next.use_legacy_builtin_classification_rules
        || previous.use_learned_rules_as_auto_rules != next.use_learned_rules_as_auto_rules
}

fn deserialize_scan_roots<'de, D>(deserializer: D) -> Result<Vec<ScanRootSetting>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<Value>::deserialize(deserializer)?;
    Ok(scan_roots_from_values(values, dirs::home_dir().as_deref()))
}

fn deserialize_search_roots<'de, D>(deserializer: D) -> Result<Vec<SearchRootSetting>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<Value>::deserialize(deserializer)?;
    Ok(search_roots_from_values(values))
}

fn default_scan_roots() -> Vec<ScanRootSetting> {
    Vec::new()
}

fn default_search_roots() -> Vec<SearchRootSetting> {
    Vec::new()
}

fn default_search_hotkey() -> String {
    DEFAULT_SEARCH_HOTKEY.to_string()
}

fn default_background_index_on_startup() -> bool {
    true
}

fn default_search_scope_mode() -> String {
    "all".to_string()
}

fn scan_roots_from_values(values: Vec<Value>, home: Option<&Path>) -> Vec<ScanRootSetting> {
    let mut roots = Vec::new();

    for value in values {
        let root = match value {
            Value::String(folder) => legacy_scan_root(&folder, home, true),
            Value::Object(object) => {
                let path = object
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if path.is_empty() {
                    continue;
                }
                let label = object
                    .get("label")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| value.trim().to_string())
                    .unwrap_or_else(|| scan_root_label(&path));
                let id = object
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| value.trim().to_string())
                    .unwrap_or_else(|| scan_root_id(&path));
                let enabled = object
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let created_at = object
                    .get("createdAt")
                    .or_else(|| object.get("created_at"))
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(DEFAULT_SCAN_ROOT_CREATED_AT)
                    .to_string();
                ScanRootSetting {
                    id,
                    path: normalize_scan_root_path(&path),
                    label,
                    enabled,
                    created_at,
                }
            }
            _ => continue,
        };
        push_unique_scan_root(&mut roots, root);
    }

    roots
}

fn search_roots_from_values(values: Vec<Value>) -> Vec<SearchRootSetting> {
    let mut roots = Vec::new();

    for value in values {
        let Value::Object(object) = value else {
            continue;
        };
        let path = object
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if path.is_empty() {
            continue;
        }
        let label = object
            .get("label")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_string())
            .unwrap_or_else(|| scan_root_label(&path));
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_string())
            .unwrap_or_else(|| scan_root_id(&path));
        let enabled = object
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let created_at = object
            .get("createdAt")
            .or_else(|| object.get("created_at"))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(DEFAULT_SCAN_ROOT_CREATED_AT)
            .to_string();
        push_unique_search_root(
            &mut roots,
            SearchRootSetting {
                id,
                path: normalize_scan_root_path(&path),
                label,
                enabled,
                created_at,
            },
        );
    }

    roots
}

fn normalized_app_settings(settings: &AppSettings) -> AppSettings {
    let values = settings
        .default_scan_folders
        .iter()
        .cloned()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_default();
    let mut next = settings.clone();
    if !matches!(next.close_behavior.as_str(), "ask" | "minimize" | "quit") {
        next.close_behavior = "ask".to_string();
    }
    if !matches!(next.folder_naming_language.as_str(), "en" | "zh") {
        next.folder_naming_language = "en".to_string();
    }
    next.restore_retention_days = next.restore_retention_days.clamp(1, 3650);
    next.default_scan_folders = scan_roots_from_values(values, dirs::home_dir().as_deref());
    let search_values = settings
        .custom_search_roots
        .iter()
        .cloned()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_default();
    next.custom_search_roots = search_roots_from_values(search_values);
    next.custom_search_roots
        .retain(|root| looks_absolute_path(&root.path));
    if next.search_hotkey.trim().is_empty() {
        next.search_hotkey = default_search_hotkey();
    }
    if !matches!(
        next.search_scope_mode.as_str(),
        "all" | "current_scan" | "custom_roots"
    ) {
        next.search_scope_mode = default_search_scope_mode();
    }
    next.organize_root_path = next
        .organize_root_path
        .as_deref()
        .map(normalize_scan_root_path)
        .filter(|path| !path.trim().is_empty() && looks_absolute_path(path));
    if matches!(next.organize_root_mode, OrganizeRootMode::CustomRoot)
        && next.organize_root_path.is_none()
    {
        next.organize_root_mode = OrganizeRootMode::CurrentFolder;
    }
    next
}

fn legacy_scan_root(folder: &str, home: Option<&Path>, enabled: bool) -> ScanRootSetting {
    let trimmed = folder.trim();
    let path = if looks_absolute_path(trimmed) {
        PathBuf::from(trimmed)
    } else {
        home.map(|home| home.join(trimmed))
            .unwrap_or_else(|| PathBuf::from(trimmed))
    };
    let label = scan_root_label(trimmed);
    let path = normalize_scan_root_path(&path.to_string_lossy());
    ScanRootSetting {
        id: if matches!(trimmed, "Desktop" | "Downloads" | "Documents") {
            format!("default-{}", trimmed.to_lowercase())
        } else {
            scan_root_id(&path)
        },
        path,
        label,
        enabled,
        created_at: DEFAULT_SCAN_ROOT_CREATED_AT.to_string(),
    }
}

fn push_unique_scan_root(roots: &mut Vec<ScanRootSetting>, root: ScanRootSetting) {
    let normalized_path = root.path.to_lowercase();
    if roots
        .iter()
        .any(|existing| existing.path.to_lowercase() == normalized_path)
    {
        return;
    }
    roots.push(root);
}

fn push_unique_search_root(roots: &mut Vec<SearchRootSetting>, root: SearchRootSetting) {
    let normalized_path = root.path.to_lowercase();
    if roots
        .iter()
        .any(|existing| existing.path.to_lowercase() == normalized_path)
    {
        return;
    }
    roots.push(root);
}

fn normalize_scan_root_path(path: &str) -> String {
    let normalized = path.trim().replace('\\', "/");
    let without_trailing = normalized.trim_end_matches('/');
    if without_trailing.is_empty() {
        normalized
    } else {
        without_trailing.to_string()
    }
}

fn scan_root_label(path: &str) -> String {
    let normalized = normalize_scan_root_path(path);
    normalized
        .split('/')
        .rfind(|segment| !segment.is_empty())
        .unwrap_or(&normalized)
        .to_string()
}

fn scan_root_id(path: &str) -> String {
    let slug = normalize_scan_root_path(path)
        .to_lowercase()
        .trim_start_matches(|character: char| character.is_ascii_alphabetic() || character == ':')
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(&character) {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();

    let normalized = normalize_scan_root_path(path).to_lowercase();
    let digest = blake3::hash(normalized.as_bytes()).to_hex().to_string();
    format!(
        "scan-root-{}-{}",
        if slug.is_empty() { "root" } else { &slug },
        &digest[..8]
    )
}

fn looks_absolute_path(path: &str) -> bool {
    Path::new(path).is_absolute()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.as_bytes().get(0..3).is_some_and(|prefix| {
            prefix[0].is_ascii_alphabetic()
                && prefix[1] == b':'
                && (prefix[2] == b'/' || prefix[2] == b'\\')
        })
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("autostart error: {0}")]
    Autostart(String),
    #[error("settings autostart rollback failed")]
    AutostartRollback,
    #[error("settings_revision_conflict")]
    RevisionConflict,
    #[error("settings side-effect reconciliation failed")]
    SideEffectReconciliation,
}

pub trait LaunchAtLoginController {
    fn enable(&self) -> Result<(), String>;
    fn disable(&self) -> Result<(), String>;
    fn is_enabled(&self) -> Result<bool, String>;
}

impl LaunchAtLoginController for AutoLaunchManager {
    fn enable(&self) -> Result<(), String> {
        AutoLaunchManager::enable(self).map_err(|error| error.to_string())
    }

    fn disable(&self) -> Result<(), String> {
        AutoLaunchManager::disable(self).map_err(|error| error.to_string())
    }

    fn is_enabled(&self) -> Result<bool, String> {
        AutoLaunchManager::is_enabled(self).map_err(|error| error.to_string())
    }
}

pub fn save_app_settings_with_launch_at_login(
    db: &Database,
    settings: &AppSettings,
    launch_at_login: &impl LaunchAtLoginController,
) -> Result<AppSettings, SettingsError> {
    let current_settings = get_app_settings(db)?;
    let launch_changed = current_settings.launch_at_login != settings.launch_at_login;
    if launch_changed {
        if settings.launch_at_login {
            launch_at_login.enable().map_err(SettingsError::Autostart)?;
        } else {
            launch_at_login
                .disable()
                .map_err(SettingsError::Autostart)?;
        }
    }

    let normalized = normalized_app_settings(settings);
    if let Err(error) = save_app_settings(db, &normalized) {
        if launch_changed {
            let rollback = if current_settings.launch_at_login {
                launch_at_login.enable()
            } else {
                launch_at_login.disable()
            };
            if let Err(rollback_error) = rollback {
                return Err(SettingsError::Autostart(format!(
                    "database save failed: {error}; autostart rollback failed: {rollback_error}"
                )));
            }
        }
        return Err(SettingsError::Db(error));
    }
    Ok(normalized)
}

pub fn save_versioned_app_settings_with_launch_at_login(
    db: &Database,
    request: &SaveSettingsRequest,
    launch_at_login: &impl LaunchAtLoginController,
) -> Result<VersionedAppSettings, SettingsError> {
    let _save_guard = versioned_settings_save_guard();
    save_versioned_app_settings_with_launch_at_login_locked(db, request, launch_at_login)
}

fn save_versioned_app_settings_with_launch_at_login_locked(
    db: &Database,
    request: &SaveSettingsRequest,
    launch_at_login: &impl LaunchAtLoginController,
) -> Result<VersionedAppSettings, SettingsError> {
    let current = get_versioned_app_settings(db)?;
    let launch_changed = current.settings.launch_at_login != request.settings.launch_at_login;
    if launch_changed {
        if request.settings.launch_at_login {
            launch_at_login.enable().map_err(SettingsError::Autostart)?;
        } else {
            launch_at_login
                .disable()
                .map_err(SettingsError::Autostart)?;
        }
    }
    match save_app_settings_cas(db, &request.settings, request.expected_revision) {
        Ok(saved) => Ok(saved),
        Err(error) => {
            if launch_changed {
                let rollback = if current.settings.launch_at_login {
                    launch_at_login.enable()
                } else {
                    launch_at_login.disable()
                };
                rollback.map_err(|_| SettingsError::AutostartRollback)?;
            }
            Err(error)
        }
    }
}

pub fn reconcile_versioned_settings_side_effect_failure(
    db: &Database,
    previous: &VersionedAppSettings,
    failed_save: &VersionedAppSettings,
    launch_at_login: &impl LaunchAtLoginController,
    restore_other_side_effects: impl FnOnce(&AppSettings) -> Result<(), String>,
) -> Result<VersionedAppSettings, SettingsError> {
    let _save_guard = versioned_settings_save_guard();
    reconcile_versioned_settings_side_effect_failure_locked(
        db,
        previous,
        failed_save,
        launch_at_login,
        restore_other_side_effects,
    )
}

fn reconcile_versioned_settings_side_effect_failure_locked(
    db: &Database,
    previous: &VersionedAppSettings,
    failed_save: &VersionedAppSettings,
    launch_at_login: &impl LaunchAtLoginController,
    restore_other_side_effects: impl FnOnce(&AppSettings) -> Result<(), String>,
) -> Result<VersionedAppSettings, SettingsError> {
    let rollback = save_versioned_app_settings_with_launch_at_login_locked(
        db,
        &SaveSettingsRequest {
            settings: previous.settings.clone(),
            expected_revision: failed_save.revision,
        },
        launch_at_login,
    )
    .inspect_err(|error| {
        native_qa_settings_trace("rollback_settings_save_failed", || {
            format!(
                "previous_revision={} failed_revision={} failure={}",
                previous.revision,
                failed_save.revision,
                settings_error_diagnostic_code(error)
            )
        });
    })?;
    native_qa_settings_trace("rollback_settings_persisted", || {
        format!(
            "revision={} enabled_scan_roots={}",
            rollback.revision,
            enabled_scan_root_count(&rollback.settings)
        )
    });
    if restore_other_side_effects(&previous.settings).is_err() {
        native_qa_settings_trace("rollback_runtime_restore_failed", || {
            format!(
                "revision={} enabled_scan_roots={}",
                rollback.revision,
                enabled_scan_root_count(&previous.settings)
            )
        });
        return Err(SettingsError::SideEffectReconciliation);
    }
    native_qa_settings_trace("rollback_runtime_restore_succeeded", || {
        format!(
            "revision={} enabled_scan_roots={}",
            rollback.revision,
            enabled_scan_root_count(&previous.settings)
        )
    });
    Ok(rollback)
}

fn db_error_diagnostic_code(error: &DbError) -> String {
    match error {
        DbError::Sqlite(rusqlite::Error::SqliteFailure(code, _)) => format!(
            "sqlite_primary_{}_extended_{}",
            code.extended_code & 0xff,
            code.extended_code
        ),
        DbError::Sqlite(_) => "sqlite_failure".to_string(),
        DbError::Pool(_) => "database_pool_failure".to_string(),
        DbError::Io(_) => "database_io_failure".to_string(),
        DbError::Json(_) => "database_json_failure".to_string(),
        DbError::Validation(_) => "database_validation_failure".to_string(),
    }
}

fn settings_error_diagnostic_code(error: &SettingsError) -> String {
    match error {
        SettingsError::Db(error) => db_error_diagnostic_code(error),
        SettingsError::RevisionConflict => "revision_conflict".to_string(),
        SettingsError::AutostartRollback => "autostart_rollback_failure".to_string(),
        SettingsError::SideEffectReconciliation => "runtime_restore_failure".to_string(),
        SettingsError::Autostart(_) => "unknown_failure".to_string(),
    }
}

pub fn sync_launch_at_login_from_system(
    db: &Database,
    settings: &AppSettings,
    launch_at_login: &impl LaunchAtLoginController,
) -> Result<AppSettings, SettingsError> {
    let system_launch_at_login = launch_at_login
        .is_enabled()
        .map_err(SettingsError::Autostart)?;
    if settings.launch_at_login == system_launch_at_login {
        return Ok(settings.clone());
    }

    let mut synced_settings = settings.clone();
    synced_settings.launch_at_login = system_launch_at_login;
    save_app_settings(db, &synced_settings)?;
    Ok(synced_settings)
}

#[tauri::command]
pub fn get_settings(db: State<'_, Database>) -> Result<VersionedAppSettings, String> {
    get_versioned_app_settings(&db).map_err(|error| error.to_string())
}

fn settings_save_failure_code(error: &SettingsError) -> SettingsSaveFailureCode {
    match error {
        SettingsError::Db(_) => SettingsSaveFailureCode::DatabaseFailure,
        SettingsError::RevisionConflict => SettingsSaveFailureCode::RevisionConflict,
        SettingsError::AutostartRollback | SettingsError::SideEffectReconciliation => {
            SettingsSaveFailureCode::RollbackReconciliationFailure
        }
        SettingsError::Autostart(_) => SettingsSaveFailureCode::UnknownFailure,
    }
}

fn watcher_save_failure_code(error: SettingsWatcherReloadFailure) -> SettingsSaveFailureCode {
    match error {
        SettingsWatcherReloadFailure::RootSynchronization => {
            SettingsSaveFailureCode::WatcherRootSyncFailure
        }
        SettingsWatcherReloadFailure::RuntimeRestart => {
            SettingsSaveFailureCode::WatcherRuntimeFailure
        }
        SettingsWatcherReloadFailure::ReconciliationScheduling => {
            SettingsSaveFailureCode::WatcherReconciliationScheduleFailure
        }
    }
}

fn save_settings_with_watcher_reload(
    db: &Database,
    request: &SaveSettingsRequest,
    launch_at_login: &impl LaunchAtLoginController,
    reload_watcher: impl Fn(&AppSettings) -> Result<bool, SettingsWatcherReloadFailure>,
) -> Result<VersionedAppSettings, SettingsSaveFailureCode> {
    // Keep the versioned settings operation serialized through watcher reload
    // and any compensation. This keeps the rollback snapshot authoritative and
    // prevents another Settings CAS from being overwritten by stale settings.
    let _save_guard = versioned_settings_save_guard();
    let previous = get_versioned_app_settings(db).map_err(|error| {
        native_qa_settings_trace("save_previous_settings_read_failed", || {
            format!("failure={}", db_error_diagnostic_code(&error))
        });
        SettingsSaveFailureCode::DatabaseFailure
    })?;
    native_qa_settings_trace("save_begin", || {
        format!(
            "previous_revision={} expected_revision={} requested_enabled_scan_roots={}",
            previous.revision,
            request.expected_revision,
            enabled_scan_root_count(&request.settings)
        )
    });
    let saved =
        save_versioned_app_settings_with_launch_at_login_locked(db, request, launch_at_login)
            .map_err(|error| {
                let failure = settings_save_failure_code(&error);
                native_qa_settings_trace("settings_persist_failed", || {
                    format!("failure={}", failure.as_str())
                });
                failure
            })?;
    native_qa_settings_trace("settings_persisted", || {
        format!(
            "revision={} enabled_scan_roots={}",
            saved.revision,
            enabled_scan_root_count(&saved.settings)
        )
    });

    if let Err(watcher_error) = reload_watcher(&saved.settings) {
        let failure_code = watcher_save_failure_code(watcher_error);
        native_qa_settings_trace("watcher_reload_failed", || {
            format!(
                "revision={} failure={}",
                saved.revision,
                failure_code.as_str()
            )
        });
        let rollback = reconcile_versioned_settings_side_effect_failure_locked(
            db,
            &previous,
            &saved,
            launch_at_login,
            |settings| {
                reload_watcher(settings)
                    .map(|_| ())
                    .map_err(|error| error.as_support_code().to_string())
            },
        );
        if rollback.is_err() {
            native_qa_settings_trace("save_failed_after_rollback", || {
                format!(
                    "failure={}",
                    SettingsSaveFailureCode::RollbackReconciliationFailure.as_str()
                )
            });
            return Err(SettingsSaveFailureCode::RollbackReconciliationFailure);
        }
        native_qa_settings_trace("save_failed_after_successful_rollback", || {
            format!("failure={}", failure_code.as_str())
        });
        return Err(failure_code);
    }

    native_qa_settings_trace("save_succeeded", || format!("revision={}", saved.revision));
    Ok(saved)
}

#[tauri::command]
pub fn save_settings<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    watcher_manager: State<'_, FileWatcherManager>,
    scan_jobs: State<'_, ScanJobManager>,
    dedupe_jobs: State<'_, DedupeJobManager>,
    request: SaveSettingsRequest,
) -> Result<VersionedAppSettings, String> {
    require_main_window(&window)
        .map_err(|_| SettingsSaveFailureCode::UnknownFailure.as_error_message())?;
    let launch_at_login = app.autolaunch();
    save_settings_with_watcher_reload(&db, &request, &*launch_at_login, |settings| {
        reload_file_watcher_for_settings_with_stage(
            app.clone(),
            &watcher_manager,
            &db,
            &scan_jobs,
            &dedupe_jobs,
            settings,
        )
    })
    .map_err(|failure| {
        let message = failure.as_error_message();
        if failure.is_watcher_failure()
            || failure == SettingsSaveFailureCode::RollbackReconciliationFailure
        {
            emit_file_watcher_error(&app, message.clone());
        }
        message
    })
}

#[cfg(test)]
mod settings_save_tests {
    use super::*;
    use crate::watcher::SettingsWatcherReloadFailure;
    use rusqlite::ErrorCode;
    use std::{cell::Cell, fs, path::PathBuf};

    struct TestDatabaseDirectory(PathBuf);

    impl TestDatabaseDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join("zen-canvas-issue-329-settings-v2")
                .join(format!("{}-{}", std::process::id(), uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).expect("create isolated settings database directory");
            Self(path)
        }

        fn database_path(&self) -> PathBuf {
            self.0.join("settings.sqlite3")
        }
    }

    impl Drop for TestDatabaseDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct NoopAutostart;

    impl LaunchAtLoginController for NoopAutostart {
        fn enable(&self) -> Result<(), String> {
            Ok(())
        }

        fn disable(&self) -> Result<(), String> {
            Ok(())
        }

        fn is_enabled(&self) -> Result<bool, String> {
            Ok(false)
        }
    }

    fn open_database() -> (TestDatabaseDirectory, Database) {
        let directory = TestDatabaseDirectory::new();
        let database =
            Database::open(directory.database_path()).expect("open isolated settings database");
        (directory, database)
    }

    fn catalog_revision(database: &Database) -> i64 {
        database
            .conn()
            .expect("borrow pooled connection")
            .query_row(
                "SELECT revision FROM rule_catalog_state WHERE singleton_id = 1",
                [],
                |row| row.get(0),
            )
            .expect("read catalog revision")
    }

    fn enabled_library_root(database: &Database, path: &str) -> i64 {
        let normalized = normalize_scan_root_path(path);
        database
            .conn()
            .expect("borrow pooled connection")
            .query_row(
                "SELECT COALESCE(MAX(enabled), 0) FROM scan_roots WHERE source_kind = 'file_library' AND normalized_path = ?1",
                [normalized],
                |row| row.get(0),
            )
            .expect("read file library root enablement")
    }

    #[test]
    fn stale_settings_cas_conflicts_without_lost_update_or_duplicate_revision() {
        let (_directory, database) = open_database();
        save_app_settings(&database, &AppSettings::default()).expect("seed app settings");
        let initial = get_versioned_app_settings(&database).expect("load seeded settings");
        let catalog_before = catalog_revision(&database);

        let mut winning_settings = initial.settings.clone();
        winning_settings.folder_naming_language = "zh".to_string();
        let winner = save_app_settings_cas(&database, &winning_settings, initial.revision)
            .expect("current settings revision should save");
        assert_eq!(winner.revision, initial.revision + 1);

        let mut stale_settings = initial.settings.clone();
        stale_settings.organize_root_mode = OrganizeRootMode::ZenCanvasFolder;
        let conflict = save_app_settings_cas(&database, &stale_settings, initial.revision)
            .expect_err("stale expected revision must fail closed");
        assert!(matches!(conflict, SettingsError::RevisionConflict));

        let persisted = get_versioned_app_settings(&database).expect("reload persisted settings");
        assert_eq!(persisted.revision, initial.revision + 1);
        assert_eq!(persisted.settings.folder_naming_language, "zh");
        assert_eq!(
            persisted.settings.organize_root_mode,
            initial.settings.organize_root_mode
        );
        assert_eq!(catalog_revision(&database), catalog_before + 1);
    }

    #[test]
    fn settings_command_rolls_back_after_root_sync_mutation_failure_and_returns_safe_stage_code() {
        let (_directory, database) = open_database();
        save_app_settings(&database, &AppSettings::default()).expect("seed app settings");
        let initial = get_versioned_app_settings(&database).expect("load seeded settings");
        let root_path = std::env::temp_dir().join(format!(
            "zen-canvas-issue-329-root-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root_path).expect("create valid disposable root");

        let mut requested_settings = initial.settings.clone();
        requested_settings.default_scan_folders = vec![ScanRootSetting {
            id: "issue-329-fixture-root".to_string(),
            path: root_path.to_string_lossy().into_owned(),
            label: "Fixture root".to_string(),
            enabled: true,
            created_at: "2026-10-08T00:00:00.000Z".to_string(),
        }];

        database
            .conn()
            .expect("borrow pooled connection")
            .execute_batch(
                "CREATE TRIGGER issue_329_fail_root_update
                 BEFORE UPDATE ON scan_roots
                 WHEN NEW.source_kind = 'file_library'
                 BEGIN
                     SELECT RAISE(ABORT, 'injected root synchronization failure');
                 END;",
            )
            .expect("install transaction-local root sync fault");

        let actual_sync_error = Cell::new(None);
        let result = save_settings_with_watcher_reload(
            &database,
            &SaveSettingsRequest {
                settings: requested_settings,
                expected_revision: initial.revision,
            },
            &NoopAutostart,
            |settings| {
                database
                    .sync_file_library_watcher_roots(&settings.default_scan_folders)
                    .map(|()| true)
                    .map_err(|error| {
                        if let DbError::Sqlite(rusqlite::Error::SqliteFailure(code, _)) = error {
                            actual_sync_error.set(Some((code.code, code.extended_code)));
                        }
                        SettingsWatcherReloadFailure::RootSynchronization
                    })
            },
        );
        assert!(matches!(
            result,
            Err(SettingsSaveFailureCode::WatcherRootSyncFailure)
        ));

        let persisted = get_versioned_app_settings(&database).expect("reload reconciled settings");
        assert_eq!(persisted.revision, initial.revision + 2);
        assert_eq!(
            persisted.settings.default_scan_folders,
            initial.settings.default_scan_folders
        );
        let root_count: i64 = database
            .conn()
            .expect("borrow pooled connection")
            .query_row(
                "SELECT COUNT(*) FROM scan_roots WHERE normalized_path = ?1",
                [root_path.to_string_lossy().replace('\\', "/")],
                |row| row.get(0),
            )
            .expect("verify failed root sync rolled back its inserted root");
        assert_eq!(root_count, 0);
        assert_eq!(
            actual_sync_error.get(),
            Some((ErrorCode::ConstraintViolation, 1811)),
            "capture the real SQLite constraint-trigger code from root synchronization"
        );
        assert_eq!(
            SettingsSaveFailureCode::WatcherRootSyncFailure.as_error_message(),
            "settings_save_failure:watcher_root_sync_failure"
        );
        let _ = fs::remove_dir_all(root_path);
    }

    #[test]
    fn watcher_rollback_reconciliation_failure_is_reported_and_settings_stay_rolled_back() {
        let (_directory, database) = open_database();
        save_app_settings(&database, &AppSettings::default()).expect("seed app settings");
        let initial = get_versioned_app_settings(&database).expect("load seeded settings");
        let mut requested_settings = initial.settings.clone();
        requested_settings.folder_naming_language = "zh".to_string();
        let reload_calls = Cell::new(0);

        let result = save_settings_with_watcher_reload(
            &database,
            &SaveSettingsRequest {
                settings: requested_settings,
                expected_revision: initial.revision,
            },
            &NoopAutostart,
            |_| {
                reload_calls.set(reload_calls.get() + 1);
                Err(SettingsWatcherReloadFailure::RuntimeRestart)
            },
        );

        assert!(matches!(
            result,
            Err(SettingsSaveFailureCode::RollbackReconciliationFailure)
        ));
        assert_eq!(reload_calls.get(), 2, "initial reload and rollback reload");
        let persisted = get_versioned_app_settings(&database).expect("read compensated settings");
        assert_eq!(persisted.revision, initial.revision + 2);
        assert_eq!(
            serde_json::to_value(&persisted.settings).expect("serialize compensated settings"),
            serde_json::to_value(&initial.settings).expect("serialize initial settings")
        );
    }

    #[test]
    fn reconciliation_scheduling_failure_rolls_back_settings_and_returns_its_stage_code() {
        let (_directory, database) = open_database();
        save_app_settings(&database, &AppSettings::default()).expect("seed app settings");
        let initial = get_versioned_app_settings(&database).expect("load seeded settings");
        let mut requested_settings = initial.settings.clone();
        requested_settings.background_index_on_startup =
            !initial.settings.background_index_on_startup;
        let reload_calls = Cell::new(0);

        let result = save_settings_with_watcher_reload(
            &database,
            &SaveSettingsRequest {
                settings: requested_settings,
                expected_revision: initial.revision,
            },
            &NoopAutostart,
            |_| {
                reload_calls.set(reload_calls.get() + 1);
                if reload_calls.get() == 1 {
                    Err(SettingsWatcherReloadFailure::ReconciliationScheduling)
                } else {
                    Ok(true)
                }
            },
        );

        assert!(matches!(
            result,
            Err(SettingsSaveFailureCode::WatcherReconciliationScheduleFailure)
        ));
        assert_eq!(reload_calls.get(), 2, "initial reload and rollback reload");
        let persisted = get_versioned_app_settings(&database).expect("read rolled-back settings");
        assert_eq!(persisted.revision, initial.revision + 2);
        assert_eq!(
            serde_json::to_value(&persisted.settings).expect("serialize rolled-back settings"),
            serde_json::to_value(&initial.settings).expect("serialize initial settings")
        );
    }

    #[test]
    fn runtime_restore_failure_reports_rollback_error_with_settings_and_root_state_observable() {
        let (_directory, database) = open_database();
        save_app_settings(&database, &AppSettings::default()).expect("seed app settings");
        let initial = get_versioned_app_settings(&database).expect("load seeded settings");
        let root_path = std::env::temp_dir().join(format!(
            "zen-canvas-issue-329-runtime-rollback-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root_path).expect("create valid disposable root");
        let root_path_text = root_path.to_string_lossy().into_owned();
        let mut requested_settings = initial.settings.clone();
        requested_settings.default_scan_folders = vec![ScanRootSetting {
            id: "issue-329-runtime-rollback-root".to_string(),
            path: root_path_text.clone(),
            label: "Fixture root".to_string(),
            enabled: true,
            created_at: "2026-10-10T00:00:00.000Z".to_string(),
        }];
        let reload_calls = Cell::new(0);

        let result = save_settings_with_watcher_reload(
            &database,
            &SaveSettingsRequest {
                settings: requested_settings,
                expected_revision: initial.revision,
            },
            &NoopAutostart,
            |settings| {
                database
                    .sync_file_library_watcher_roots(&settings.default_scan_folders)
                    .map_err(|_| SettingsWatcherReloadFailure::RootSynchronization)?;
                reload_calls.set(reload_calls.get() + 1);
                Err(SettingsWatcherReloadFailure::RuntimeRestart)
            },
        );

        assert!(matches!(
            result,
            Err(SettingsSaveFailureCode::RollbackReconciliationFailure)
        ));
        assert_eq!(reload_calls.get(), 2, "initial reload and rollback reload");
        let persisted = get_versioned_app_settings(&database).expect("read rolled-back settings");
        assert_eq!(persisted.revision, initial.revision + 2);
        assert_eq!(
            serde_json::to_value(&persisted.settings).expect("serialize persisted settings"),
            serde_json::to_value(&initial.settings).expect("serialize original settings")
        );
        assert_eq!(enabled_library_root(&database, &root_path_text), 0);
        let _ = fs::remove_dir_all(root_path);
    }

    #[test]
    fn reconciliation_schedule_failure_restores_real_settings_owned_root_state() {
        let (_directory, database) = open_database();
        save_app_settings(&database, &AppSettings::default()).expect("seed app settings");
        let initial = get_versioned_app_settings(&database).expect("load seeded settings");
        let root_path = std::env::temp_dir().join(format!(
            "zen-canvas-issue-329-schedule-rollback-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root_path).expect("create valid disposable root");
        let root_path_text = root_path.to_string_lossy().into_owned();
        let mut requested_settings = initial.settings.clone();
        requested_settings.default_scan_folders = vec![ScanRootSetting {
            id: "issue-329-schedule-rollback-root".to_string(),
            path: root_path_text.clone(),
            label: "Fixture root".to_string(),
            enabled: true,
            created_at: "2026-10-10T00:00:00.000Z".to_string(),
        }];
        let reload_calls = Cell::new(0);

        let result = save_settings_with_watcher_reload(
            &database,
            &SaveSettingsRequest {
                settings: requested_settings,
                expected_revision: initial.revision,
            },
            &NoopAutostart,
            |settings| {
                database
                    .sync_file_library_watcher_roots(&settings.default_scan_folders)
                    .map_err(|_| SettingsWatcherReloadFailure::RootSynchronization)?;
                reload_calls.set(reload_calls.get() + 1);
                if reload_calls.get() == 1 {
                    Err(SettingsWatcherReloadFailure::ReconciliationScheduling)
                } else {
                    Ok(true)
                }
            },
        );

        assert!(matches!(
            result,
            Err(SettingsSaveFailureCode::WatcherReconciliationScheduleFailure)
        ));
        assert_eq!(reload_calls.get(), 2, "initial reload and rollback reload");
        let persisted = get_versioned_app_settings(&database).expect("read rolled-back settings");
        assert_eq!(persisted.revision, initial.revision + 2);
        assert_eq!(
            serde_json::to_value(&persisted.settings).expect("serialize persisted settings"),
            serde_json::to_value(&initial.settings).expect("serialize original settings")
        );
        assert_eq!(enabled_library_root(&database, &root_path_text), 0);
        let _ = fs::remove_dir_all(root_path);
    }

    #[test]
    fn settings_save_codes_are_stage_only_and_include_unknown_and_rollback() {
        assert_eq!(
            settings_save_failure_code(&SettingsError::Db(DbError::Validation(
                "private detail".to_string()
            )))
            .as_error_message(),
            "settings_save_failure:database_failure"
        );
        assert_eq!(
            settings_save_failure_code(&SettingsError::RevisionConflict).as_error_message(),
            "settings_save_failure:revision_conflict"
        );
        assert_eq!(
            settings_save_failure_code(&SettingsError::AutostartRollback).as_error_message(),
            "settings_save_failure:rollback_reconciliation_failure"
        );
        assert_eq!(
            settings_save_failure_code(&SettingsError::Autostart("private detail".to_string()))
                .as_error_message(),
            "settings_save_failure:unknown_failure"
        );
        assert_eq!(
            watcher_save_failure_code(SettingsWatcherReloadFailure::RootSynchronization)
                .as_error_message(),
            "settings_save_failure:watcher_root_sync_failure"
        );
        assert_eq!(
            watcher_save_failure_code(SettingsWatcherReloadFailure::RuntimeRestart)
                .as_error_message(),
            "settings_save_failure:watcher_runtime_failure"
        );
        assert_eq!(
            watcher_save_failure_code(SettingsWatcherReloadFailure::ReconciliationScheduling)
                .as_error_message(),
            "settings_save_failure:watcher_reconciliation_schedule_failure"
        );
    }
}
