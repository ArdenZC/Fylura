//! One-time, non-destructive database handoff from the unreleased Zen Canvas
//! development identity to Fylura. The old database remains untouched as a
//! rollback copy; SQLite's backup API folds any committed WAL state into the
//! new database instead of copying live sidecars.

use rusqlite::{backup::Backup, Connection};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    time::Duration,
};
use sysinfo::{ProcessesToUpdate, System};

const LEGACY_APP_IDENTIFIER: &str = "com.startlan.zencanvas";
const CURRENT_APP_IDENTIFIER: &str = "com.startlan.fylura";
const LEGACY_DATABASE_NAME: &str = "zen-canvas.sqlite3";
const CURRENT_DATABASE_NAME: &str = "fylura.sqlite3";
const SQLITE_SIDECAR_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];
#[cfg(target_os = "windows")]
const LEGACY_WINDOWS_RUN_VALUE_NAME: &str = "Zen Canvas";
#[cfg(target_os = "windows")]
const CURRENT_WINDOWS_RUN_VALUE_NAME: &str = "Fylura";
#[cfg(any(target_os = "windows", test))]
const LEGACY_WINDOWS_BINARY_NAME: &str = "zen-canvas.exe";
#[cfg(target_os = "windows")]
const LEGACY_WINDOWS_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(target_os = "windows")]
const WINDOWS_STARTUP_APPROVED_RUN_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
#[cfg(target_os = "macos")]
const LEGACY_MACOS_APP_NAME: &str = "Zen Canvas";
#[cfg(target_os = "macos")]
const LEGACY_MACOS_BINARY_NAME: &str = "zen-canvas";

pub(crate) fn legacy_app_data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|data_root| legacy_app_data_dir_under(&data_root))
}

fn legacy_app_data_dir_under(data_root: &Path) -> PathBuf {
    data_root.join(LEGACY_APP_IDENTIFIER)
}

pub(crate) fn is_legacy_app_data_path(path: &Path) -> bool {
    legacy_app_data_dir().is_some_and(|legacy_dir| is_same_or_child(path, &legacy_dir))
}

pub(crate) fn migrate_legacy_database(
    data_root: &Path,
    current_app_data_dir: &Path,
) -> Result<(), String> {
    migrate_legacy_database_with_process_state(data_root, current_app_data_dir, || {
        legacy_app_process_is_running()
    })
}

fn migrate_legacy_database_with_process_state(
    data_root: &Path,
    current_app_data_dir: &Path,
    legacy_app_running: impl FnOnce() -> bool,
) -> Result<(), String> {
    let expected_current_dir = data_root.join(CURRENT_APP_IDENTIFIER);
    if !same_path(&expected_current_dir, current_app_data_dir) {
        return Err(format!(
            "identity_migration_refused: expected Fylura data directory {:?}, got {:?}",
            expected_current_dir, current_app_data_dir
        ));
    }

    let legacy_database = legacy_app_data_dir_under(data_root).join(LEGACY_DATABASE_NAME);
    let current_database = current_app_data_dir.join(CURRENT_DATABASE_NAME);

    // An existing Fylura database is authoritative. Never replace or merge it
    // with an older development database.
    match fs::symlink_metadata(&current_database) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => return Ok(()),
        Ok(_) => {
            return Err(
                "identity_migration_refused: existing Fylura database is not a regular file"
                    .to_string(),
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "identity_migration_could_not_inspect_fylura_database: {error}"
            ));
        }
    }

    refuse_orphan_sqlite_sidecars(&current_database, "Fylura")?;

    let legacy_metadata = match fs::symlink_metadata(&legacy_database) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            refuse_orphan_sqlite_sidecars(&legacy_database, "legacy Zen Canvas")?;
            return Ok(());
        }
        Err(error) => {
            return Err(format!(
                "identity_migration_could_not_inspect_legacy_database: {error}"
            ));
        }
    };
    if !legacy_metadata.is_file() || legacy_metadata.file_type().is_symlink() {
        return Err(
            "identity_migration_refused: legacy database is not a regular file".to_string(),
        );
    }
    validate_existing_sqlite_sidecars(&legacy_database)?;

    if legacy_app_running() {
        return Err(format!(
            "identity_migration_blocked: close the older Zen Canvas app before first Fylura launch; the legacy database at {:?} was not changed",
            legacy_database
        ));
    }

    fs::create_dir_all(current_app_data_dir)
        .map_err(|error| format!("identity_migration_could_not_create_fylura_data_dir: {error}"))?;

    let temporary_database = current_app_data_dir.join(format!(
        ".{CURRENT_DATABASE_NAME}.migration-{}",
        uuid::Uuid::new_v4()
    ));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_database)
        .map_err(|error| {
            format!("identity_migration_could_not_create_temporary_database: {error}")
        })?;

    let result = create_verified_snapshot(&legacy_database, &temporary_database)
        .and_then(|()| publish_without_replacing(&temporary_database, &current_database));
    remove_owned_temporary_files(&temporary_database);
    result?;

    eprintln!(
        "database_identity_migration completed source={:?} target={:?} legacy_preserved=true",
        legacy_database, current_database
    );
    Ok(())
}

fn create_verified_snapshot(source_path: &Path, target_path: &Path) -> Result<(), String> {
    let source = Connection::open_with_flags(
        source_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|error| format!("identity_migration_could_not_open_legacy_database: {error}"))?;
    source
        .busy_timeout(Duration::from_secs(30))
        .map_err(|error| format!("identity_migration_could_not_set_busy_timeout: {error}"))?;
    let mut target = Connection::open(target_path).map_err(|error| {
        format!("identity_migration_could_not_open_temporary_database: {error}")
    })?;

    {
        let backup = Backup::new(&source, &mut target)
            .map_err(|error| format!("identity_migration_backup_could_not_start: {error}"))?;
        backup
            .run_to_completion(128, Duration::from_millis(25), None)
            .map_err(|error| format!("identity_migration_sqlite_backup_failed: {error}"))?;
    }

    let integrity: String = target
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| format!("identity_migration_integrity_check_failed: {error}"))?;
    if integrity != "ok" {
        return Err(format!(
            "identity_migration_integrity_check_rejected_snapshot: {integrity}"
        ));
    }

    let required_tables: i64 = target
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('files', 'operation_logs', 'app_settings')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("identity_migration_schema_check_failed: {error}"))?;
    if required_tables != 3 {
        return Err("identity_migration_refused: legacy database schema is incomplete".to_string());
    }

    let source_user_version: i64 = source
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| format!("identity_migration_source_version_read_failed: {error}"))?;
    let target_user_version: i64 = target
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| format!("identity_migration_target_version_read_failed: {error}"))?;
    if source_user_version != target_user_version {
        return Err(
            "identity_migration_refused: SQLite user_version did not survive backup".to_string(),
        );
    }

    drop(target);
    File::open(target_path)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("identity_migration_snapshot_sync_failed: {error}"))?;
    Ok(())
}

fn publish_without_replacing(
    temporary_database: &Path,
    current_database: &Path,
) -> Result<(), String> {
    fs::hard_link(temporary_database, current_database)
        .map_err(|error| format!("identity_migration_publish_failed_without_overwrite: {error}"))
}

fn sqlite_sidecar_path(database_path: &Path, suffix: &str) -> PathBuf {
    let mut path = OsString::from(database_path.as_os_str());
    path.push(suffix);
    PathBuf::from(path)
}

fn refuse_orphan_sqlite_sidecars(database_path: &Path, identity: &str) -> Result<(), String> {
    for suffix in SQLITE_SIDECAR_SUFFIXES {
        let sidecar = sqlite_sidecar_path(database_path, suffix);
        match fs::symlink_metadata(&sidecar) {
            Ok(_) => {
                return Err(format!(
                    "identity_migration_refused: {identity} SQLite sidecar exists without its primary database at {:?}; all files were preserved",
                    sidecar
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "identity_migration_could_not_inspect_sqlite_sidecar: {error}"
                ));
            }
        }
    }
    Ok(())
}

fn validate_existing_sqlite_sidecars(database_path: &Path) -> Result<(), String> {
    for suffix in SQLITE_SIDECAR_SUFFIXES {
        let sidecar = sqlite_sidecar_path(database_path, suffix);
        match fs::symlink_metadata(&sidecar) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(format!(
                    "identity_migration_refused: legacy SQLite sidecar is not a regular file at {:?}; all files were preserved",
                    sidecar
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "identity_migration_could_not_inspect_legacy_sqlite_sidecar: {error}"
                ));
            }
        }
    }
    Ok(())
}

fn remove_owned_temporary_files(path: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let candidate = if suffix.is_empty() {
            path.to_path_buf()
        } else {
            sqlite_sidecar_path(path, suffix)
        };
        if let Err(error) = fs::remove_file(&candidate) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "database_identity_migration temporary_cleanup_failed path={:?} error={error}",
                    candidate
                );
            }
        }
    }
}

fn legacy_app_process_is_running() -> bool {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    system.processes().values().any(|process| {
        let executable_name = process
            .exe()
            .and_then(Path::file_name)
            .unwrap_or_else(|| process.name());
        is_legacy_executable_name(executable_name)
    })
}

fn is_legacy_executable_name(name: &OsStr) -> bool {
    let name = name.to_string_lossy();
    [
        "zen-canvas",
        "zen-canvas.exe",
        "zen canvas",
        "zen canvas.exe",
    ]
    .iter()
    .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

/// Identify only the command shape written by the legacy Tauri autostart
/// plugin: an absolute old app executable followed by the single background
/// argument. The registry value name is checked separately before removal.
#[cfg(any(target_os = "windows", test))]
fn is_legacy_windows_autostart_command(command: &str) -> bool {
    let Some(executable) = command.strip_suffix(" --background") else {
        return false;
    };
    let executable = executable
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(executable);
    if executable.contains('"') {
        return false;
    }

    let normalized = executable.replace('/', "\\");
    let bytes = normalized.as_bytes();
    let is_drive_absolute =
        bytes.len() >= 4 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    if !is_drive_absolute {
        return false;
    }
    normalized
        .rsplit('\\')
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case(LEGACY_WINDOWS_BINARY_NAME))
}

#[cfg(any(target_os = "windows", test))]
fn windows_startup_approval_enabled(bytes: Option<&[u8]>) -> Option<bool> {
    let Some(bytes) = bytes else {
        // Windows treats a missing StartupApproved record as enabled.
        return Some(true);
    };
    let state = bytes.get(..4)?;
    match u32::from_le_bytes(state.try_into().ok()?) {
        0x02 | 0x06 => Some(true),
        0x03 | 0x07 => Some(false),
        _ => None,
    }
}

/// Preserve the old Windows login preference across the product-name change.
/// Only the exact `Zen Canvas` Run value pointing at the old executable is
/// eligible for removal; all foreign or ambiguous values are retained.
#[cfg(target_os = "windows")]
pub fn migrate_legacy_windows_autostart(
    enable_current: impl FnOnce() -> Result<(), String>,
    disable_current: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    use winreg::{
        enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE},
        RegKey,
    };

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run_key = match hkcu
        .open_subkey_with_flags(LEGACY_WINDOWS_RUN_KEY, KEY_QUERY_VALUE | KEY_SET_VALUE)
    {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("legacy_windows_autostart_open_failed: {error}")),
    };
    let legacy_command = match run_key.get_value::<String, _>(LEGACY_WINDOWS_RUN_VALUE_NAME) {
        Ok(command) => command,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("legacy_windows_autostart_read_failed: {error}")),
    };
    if !is_legacy_windows_autostart_command(&legacy_command) {
        return Err(
            "legacy_windows_autostart_refused: the Zen Canvas Run value does not match the old executable command; it was preserved"
                .to_string(),
        );
    }
    if legacy_app_process_is_running() {
        return Err(
            "legacy_windows_autostart_blocked: close the older Zen Canvas app before changing its login item"
                .to_string(),
        );
    }

    let current_entry_present = match run_key.get_value::<String, _>(CURRENT_WINDOWS_RUN_VALUE_NAME)
    {
        Ok(command) => {
            let current_executable = std::env::current_exe()
                .map_err(|error| format!("current_windows_executable_unavailable: {error}"))?;
            let expected_command = format!("{} --background", current_executable.display());
            if !command.eq_ignore_ascii_case(&expected_command) {
                return Err(
                    "legacy_windows_autostart_refused: the Fylura Run value points to another executable; both entries were preserved"
                        .to_string(),
                );
            }
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(format!("current_windows_autostart_read_failed: {error}"));
        }
    };

    // If this Fylura build already has a Run entry, preserve its Startup Apps
    // state and let the regular settings sync read it after legacy cleanup.
    // Otherwise copy the old app's Run + StartupApproved state exactly.
    let legacy_enabled = if current_entry_present {
        false
    } else {
        let approved_key =
            match hkcu.open_subkey_with_flags(WINDOWS_STARTUP_APPROVED_RUN_KEY, KEY_QUERY_VALUE) {
                Ok(key) => Some(key),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(format!(
                        "legacy_windows_startup_approval_open_failed: {error}"
                    ));
                }
            };
        let approval_bytes = match approved_key {
            Some(key) => match key.get_raw_value(LEGACY_WINDOWS_RUN_VALUE_NAME) {
                Ok(value) => Some(value.bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(format!(
                        "legacy_windows_startup_approval_read_failed: {error}"
                    ));
                }
            },
            None => None,
        };
        windows_startup_approval_enabled(approval_bytes.as_deref()).ok_or_else(|| {
            "legacy_windows_autostart_refused: the StartupApproved state is unknown; the old Run entry was preserved".to_string()
        })?
    };

    if !current_entry_present && legacy_enabled {
        enable_current().map_err(|error| {
            format!("legacy_windows_autostart_could_not_enable_fylura_entry: {error}")
        })?;
    }
    if let Err(error) = run_key.delete_value(LEGACY_WINDOWS_RUN_VALUE_NAME) {
        let rollback_detail = if current_entry_present || !legacy_enabled {
            String::new()
        } else {
            disable_current()
                .err()
                .map(|rollback_error| format!("; Fylura entry rollback failed: {rollback_error}"))
                .unwrap_or_default()
        };
        return Err(format!(
            "legacy_windows_autostart_could_not_remove_old_entry: {error}{rollback_detail}"
        ));
    }

    eprintln!(
        "identity_migration windows_autostart migrated=true legacy_entry_removed=true current_entry_present={current_entry_present} legacy_launch_at_login={legacy_enabled}"
    );
    Ok(!current_entry_present && legacy_enabled)
}

/// Remove only the exact LaunchAgent shape written by the old app. The caller
/// re-enables Fylura first when this entry existed, so the saved login behavior
/// survives the change from `Zen Canvas.plist` to `Fylura.plist`.
#[cfg(target_os = "macos")]
pub fn migrate_legacy_macos_autostart(
    enable_current: impl FnOnce() -> Result<(), String>,
    disable_current: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    let home = dirs::home_dir()
        .ok_or_else(|| "legacy_macos_autostart_home_directory_unavailable".to_string())?;
    let legacy_agent = home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LEGACY_MACOS_APP_NAME}.plist"));
    let metadata = match fs::symlink_metadata(&legacy_agent) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("legacy_macos_autostart_inspection_failed: {error}")),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("legacy_macos_autostart_refused: entry is not a regular file".to_string());
    }
    if !is_legacy_macos_launch_agent(&legacy_agent)? {
        return Err(
            "legacy_macos_autostart_refused: LaunchAgent does not match the old Zen Canvas login item; it was preserved"
                .to_string(),
        );
    }
    if legacy_app_process_is_running() {
        return Err(
            "legacy_macos_autostart_blocked: close the older Zen Canvas app before changing its login item"
                .to_string(),
        );
    }

    enable_current().map_err(|error| {
        format!("legacy_macos_autostart_could_not_enable_fylura_entry: {error}")
    })?;
    if let Err(error) = fs::remove_file(&legacy_agent) {
        let rollback = disable_current();
        let rollback_detail = rollback
            .err()
            .map(|rollback_error| format!("; Fylura entry rollback failed: {rollback_error}"))
            .unwrap_or_default();
        return Err(format!(
            "legacy_macos_autostart_could_not_remove_old_entry: {error}{rollback_detail}"
        ));
    }

    eprintln!(
        "identity_migration macos_autostart migrated=true legacy_entry_removed=true current_entry_enabled=true"
    );
    Ok(true)
}

#[cfg(target_os = "macos")]
fn is_legacy_macos_launch_agent(path: &Path) -> Result<bool, String> {
    let document = plist::Value::from_file(path)
        .map_err(|error| format!("legacy_macos_autostart_plist_read_failed: {error}"))?;
    let Some(dictionary) = document.as_dictionary() else {
        return Ok(false);
    };
    if dictionary.get("Label").and_then(plist::Value::as_string) != Some(LEGACY_MACOS_APP_NAME) {
        return Ok(false);
    }
    let Some(arguments) = dictionary
        .get("ProgramArguments")
        .and_then(plist::Value::as_array)
    else {
        return Ok(false);
    };
    let Some(arguments) = arguments
        .iter()
        .map(plist::Value::as_string)
        .collect::<Option<Vec<_>>>()
    else {
        return Ok(false);
    };
    if arguments.len() != 2 || arguments[1] != "--background" {
        return Ok(false);
    }

    let executable = Path::new(arguments[0]);
    let has_legacy_bundle = executable
        .ancestors()
        .any(|ancestor| ancestor.file_name() == Some(OsStr::new("Zen Canvas.app")));
    Ok(has_legacy_bundle && executable.file_name() == Some(OsStr::new(LEGACY_MACOS_BINARY_NAME)))
}

fn same_path(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn is_same_or_child(path: &Path, parent: &Path) -> bool {
    let path = path.to_string_lossy();
    let parent = parent.to_string_lossy();
    let separator = std::path::MAIN_SEPARATOR;
    #[cfg(windows)]
    let (path, parent) = (path.to_ascii_lowercase(), parent.to_ascii_lowercase());
    #[cfg(not(windows))]
    let (path, parent) = (path.to_string(), parent.to_string());
    path == parent
        || path
            .strip_prefix(&parent)
            .is_some_and(|suffix| suffix.starts_with(separator))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let counter = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "fylura-identity-migration-{}-{timestamp}-{counter}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create migration fixture");
            Self(path)
        }

        fn data_root(&self) -> PathBuf {
            self.0.join("data")
        }

        fn legacy_database(&self) -> PathBuf {
            self.data_root()
                .join(LEGACY_APP_IDENTIFIER)
                .join(LEGACY_DATABASE_NAME)
        }

        fn current_dir(&self) -> PathBuf {
            self.data_root().join(CURRENT_APP_IDENTIFIER)
        }

        fn current_database(&self) -> PathBuf {
            self.current_dir().join(CURRENT_DATABASE_NAME)
        }

        fn write_legacy_database(&self) -> Connection {
            let path = self.legacy_database();
            fs::create_dir_all(path.parent().expect("legacy parent"))
                .expect("create legacy data directory");
            let connection = Connection::open(path).expect("create legacy database");
            connection
                .pragma_update(None, "journal_mode", "WAL")
                .expect("enable WAL");
            connection
                .execute_batch(
                    "CREATE TABLE files(id TEXT PRIMARY KEY, path TEXT NOT NULL);
                     CREATE TABLE operation_logs(id TEXT PRIMARY KEY, status TEXT NOT NULL);
                     CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                     PRAGMA user_version=47;
                     INSERT INTO files(id,path) VALUES('file-1','/safe/file.txt');
                     INSERT INTO operation_logs(id,status) VALUES('restore-1','restored');
                     INSERT INTO app_settings(key,value) VALUES('global_index_sources','authorized-roots-preserved');",
                )
                .expect("write legacy database fixture");
            connection
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn copies_a_wal_consistent_snapshot_without_modifying_legacy_data() {
        let fixture = Fixture::new();
        let legacy_writer = fixture.write_legacy_database();
        assert!(fixture
            .legacy_database()
            .with_extension("sqlite3-wal")
            .exists());

        migrate_legacy_database_with_process_state(
            &fixture.data_root(),
            &fixture.current_dir(),
            || false,
        )
        .expect("migrate legacy database");

        let copied = Connection::open(fixture.current_database()).expect("open Fylura copy");
        let (path, status, scope): (String, String, String) = copied
            .query_row(
                "SELECT files.path, operation_logs.status, app_settings.value FROM files, operation_logs, app_settings WHERE files.id='file-1' AND operation_logs.id='restore-1' AND app_settings.key='global_index_sources'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read copied file, History/Restore, and index settings");
        assert_eq!(path, "/safe/file.txt");
        assert_eq!(status, "restored");
        assert_eq!(scope, "authorized-roots-preserved");
        assert_eq!(
            copied
                .query_row::<String, _, _>("PRAGMA integrity_check", [], |row| row.get(0))
                .expect("integrity check"),
            "ok"
        );
        assert!(fixture.legacy_database().exists());

        let legacy_value: String = legacy_writer
            .query_row(
                "SELECT value FROM app_settings WHERE key='global_index_sources'",
                [],
                |row| row.get(0),
            )
            .expect("legacy database remains readable");
        assert_eq!(legacy_value, "authorized-roots-preserved");
    }

    #[test]
    fn does_not_replace_an_existing_fylura_database() {
        let fixture = Fixture::new();
        let _legacy_writer = fixture.write_legacy_database();
        fs::create_dir_all(fixture.current_dir()).expect("create current data directory");
        fs::write(fixture.current_database(), b"existing Fylura state")
            .expect("write existing target");

        migrate_legacy_database_with_process_state(
            &fixture.data_root(),
            &fixture.current_dir(),
            || false,
        )
        .expect("existing Fylura state stays authoritative");

        assert_eq!(
            fs::read(fixture.current_database()).expect("read existing target"),
            b"existing Fylura state"
        );
        assert!(fixture.legacy_database().exists());
    }

    #[test]
    fn refuses_to_migrate_over_orphaned_fylura_sqlite_sidecars() {
        let fixture = Fixture::new();
        let _legacy_writer = fixture.write_legacy_database();
        fs::create_dir_all(fixture.current_dir()).expect("create Fylura data directory");
        let wal_path = sqlite_sidecar_path(&fixture.current_database(), "-wal");
        fs::write(&wal_path, b"preserve orphaned Fylura WAL").expect("write orphaned WAL");

        let error = migrate_legacy_database_with_process_state(
            &fixture.data_root(),
            &fixture.current_dir(),
            || false,
        )
        .expect_err("orphaned sidecars must block migration");

        assert!(error.contains("Fylura SQLite sidecar"));
        assert!(!fixture.current_database().exists());
        assert_eq!(
            fs::read(&wal_path).expect("read preserved WAL"),
            b"preserve orphaned Fylura WAL"
        );
        assert!(fixture.legacy_database().is_file());
    }

    #[test]
    fn refuses_to_ignore_legacy_sqlite_sidecars_without_the_primary_database() {
        let fixture = Fixture::new();
        let legacy_wal = sqlite_sidecar_path(&fixture.legacy_database(), "-wal");
        fs::create_dir_all(legacy_wal.parent().expect("legacy parent"))
            .expect("create legacy directory");
        fs::write(&legacy_wal, b"preserve legacy WAL").expect("write legacy WAL");

        let error = migrate_legacy_database_with_process_state(
            &fixture.data_root(),
            &fixture.current_dir(),
            || false,
        )
        .expect_err("legacy orphaned sidecar must block first-run database creation");

        assert!(error.contains("legacy Zen Canvas SQLite sidecar"));
        assert_eq!(
            fs::read(&legacy_wal).expect("read preserved legacy WAL"),
            b"preserve legacy WAL"
        );
        assert!(!fixture.current_database().exists());
    }

    #[test]
    fn refuses_to_snapshot_while_the_legacy_application_is_running() {
        let fixture = Fixture::new();
        let _legacy_writer = fixture.write_legacy_database();

        let error = migrate_legacy_database_with_process_state(
            &fixture.data_root(),
            &fixture.current_dir(),
            || true,
        )
        .expect_err("avoid concurrent old and new database writers");

        assert!(error.contains("close the older Zen Canvas app"));
        assert!(!fixture.current_database().exists());
        assert!(fixture.legacy_database().exists());
    }

    #[test]
    fn detects_only_the_legacy_main_binary_names() {
        assert!(is_legacy_executable_name(OsStr::new("zen-canvas.exe")));
        assert!(is_legacy_executable_name(OsStr::new("Zen Canvas")));
        assert!(!is_legacy_executable_name(OsStr::new("fylura.exe")));
    }

    #[test]
    fn accepts_only_absolute_legacy_windows_autostart_commands() {
        assert!(is_legacy_windows_autostart_command(
            r"C:\Program Files\Zen Canvas\zen-canvas.exe --background"
        ));
        assert!(is_legacy_windows_autostart_command(
            r#""C:\Custom\zen-canvas.exe" --background"#
        ));
        assert!(!is_legacy_windows_autostart_command(
            r"C:\Program Files\Fylura\fylura.exe --background"
        ));
        assert!(!is_legacy_windows_autostart_command(
            r"zen-canvas.exe --background"
        ));
        assert!(!is_legacy_windows_autostart_command(
            r"C:\Program Files\Zen Canvas\zen-canvas.exe --background --open C:\secret.txt"
        ));
        assert!(!is_legacy_windows_autostart_command(
            r"C:\Other App\other.exe --background"
        ));
    }

    #[test]
    fn reads_windows_startup_approval_state_and_fails_closed_for_unknown_data() {
        assert_eq!(windows_startup_approval_enabled(None), Some(true));
        assert_eq!(
            windows_startup_approval_enabled(Some(&[0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,])),
            Some(true)
        );
        assert_eq!(
            windows_startup_approval_enabled(Some(&[
                0x03, 0, 0, 0, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
            ])),
            Some(false)
        );
        assert_eq!(
            windows_startup_approval_enabled(Some(&[0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,])),
            Some(true)
        );
        assert_eq!(
            windows_startup_approval_enabled(Some(&[
                0x07, 0, 0, 0, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
            ])),
            Some(false)
        );
        assert_eq!(
            windows_startup_approval_enabled(Some(&[0x09, 0, 0, 0])),
            None
        );
        assert_eq!(windows_startup_approval_enabled(Some(&[0x02])), None);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn recognizes_only_the_owned_legacy_macos_launch_agent_shape() {
        let fixture = Fixture::new();
        let old_agent = fixture.0.join("Zen Canvas.plist");
        fs::write(
            &old_agent,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>Zen Canvas</string>
<key>ProgramArguments</key><array>
<string>/Applications/Zen Canvas.app/Contents/MacOS/zen-canvas</string>
<string>--background</string>
</array></dict></plist>"#,
        )
        .expect("write launch-agent fixture");
        assert!(is_legacy_macos_launch_agent(&old_agent).expect("parse old launch agent"));

        fs::write(
            &old_agent,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>Label</key><string>Zen Canvas</string>
<key>ProgramArguments</key><array>
<string>/Applications/Other.app/Contents/MacOS/zen-canvas</string>
<string>--background</string>
</array></dict></plist>"#,
        )
        .expect("write non-owned fixture");
        assert!(!is_legacy_macos_launch_agent(&old_agent).expect("parse foreign launch agent"));
    }

    #[test]
    fn legacy_app_data_protection_covers_the_old_database_and_caches() {
        let root = Path::new("/profile/Application Support");
        let legacy = legacy_app_data_dir_under(root);
        assert!(is_same_or_child(
            &legacy.join(LEGACY_DATABASE_NAME),
            &legacy
        ));
        assert!(is_same_or_child(
            &legacy.join("quick-look/cache.png"),
            &legacy
        ));
        assert!(!is_same_or_child(
            &root
                .join(CURRENT_APP_IDENTIFIER)
                .join(CURRENT_DATABASE_NAME),
            &legacy
        ));
    }
}
