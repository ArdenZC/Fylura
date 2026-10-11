use crate::{
    db::{Database, DbError},
    global_index::models::{
        GlobalVolume, INDEX_STATUS_INDEXING, INDEX_STATUS_READY, INDEX_STATUS_SYNCING,
        PROVIDER_WINDOWS_MFT_USN,
    },
    settings::{
        get_versioned_app_settings, save_app_settings, save_app_settings_cas,
        save_app_settings_cas_for_test, AppSettings, ScanRootSetting, SettingsError,
    },
};
use rusqlite::{Connection, Error as SqliteError, ErrorCode, OpenFlags, TransactionBehavior};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
static SETTINGS_CONTENTION_TEST_LOCK: Mutex<()> = Mutex::new(());

struct FixtureDirectory(PathBuf);

impl FixtureDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir()
            .join("zen-canvas-issue-328")
            .join(format!("{}-{}", std::process::id(), uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).expect("create isolated SQLite contention fixture");
        Self(path)
    }

    fn database_path(&self) -> PathBuf {
        self.0.join("zen-canvas.sqlite3")
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct HeldWriteLock {
    release: Option<Sender<()>>,
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl HeldWriteLock {
    fn begin(database: &Database, volume_id: &str) -> Self {
        let database = database.clone();
        let volume_id = volume_id.to_string();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
        let (release_tx, release_rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            let connection = match database.conn() {
                Ok(connection) => connection,
                Err(error) => {
                    let message = error.to_string();
                    let _ = ready_tx.send(Err(message.clone()));
                    return Err(message);
                }
            };
            let setup = (|| -> Result<(), String> {
                let journal_mode: String = connection
                    .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                    .map_err(|error| error.to_string())?;
                if !journal_mode.eq_ignore_ascii_case("wal") {
                    return Err(format!(
                        "fixture journal mode was {journal_mode}, expected WAL"
                    ));
                }
                connection
                    .execute_batch("BEGIN IMMEDIATE;")
                    .map_err(|error| error.to_string())?;
                let changed = connection
                    .execute(
                        "UPDATE global_volumes SET updated_at = updated_at + 1 WHERE id = ?1",
                        [&volume_id],
                    )
                    .map_err(|error| error.to_string())?;
                if changed != 1 {
                    return Err("lock-holder fixture volume was not updated".to_string());
                }
                Ok(())
            })();
            if let Err(message) = setup {
                let _ = ready_tx.send(Err(message.clone()));
                return Err(message);
            }
            ready_tx.send(Ok(())).map_err(|error| error.to_string())?;
            release_rx.recv().map_err(|error| error.to_string())?;
            connection
                .execute_batch("COMMIT;")
                .map_err(|error| error.to_string())
        });

        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Self {
                release: Some(release_tx),
                thread: Some(thread),
            },
            Ok(Err(message)) => panic!("failed to establish lock holder: {message}"),
            Err(error) => panic!("lock holder did not reach barrier: {error}"),
        }
    }

    fn release(mut self) -> Result<(), String> {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        self.join()
    }

    fn join(&mut self) -> Result<(), String> {
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .map_err(|_| "lock-holder thread panicked".to_string())?,
            None => Ok(()),
        }
    }
}

impl Drop for HeldWriteLock {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        let _ = self.join();
    }
}

struct HeldReadSnapshot {
    ready: Receiver<Result<(String, i64), String>>,
    release: Option<Sender<()>>,
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl HeldReadSnapshot {
    fn begin(path: &Path) -> Self {
        let path = path.to_path_buf();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(String, i64), String>>(1);
        let (release_tx, release_rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            let connection =
                match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
                    Ok(connection) => connection,
                    Err(error) => {
                        let message = error.to_string();
                        let _ = ready_tx.send(Err(message.clone()));
                        return Err(message);
                    }
                };
            let setup = (|| -> Result<(String, i64), String> {
                connection
                    .pragma_update(None, "query_only", "ON")
                    .map_err(|error| error.to_string())?;
                let journal_mode: String = connection
                    .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                    .map_err(|error| error.to_string())?;
                let query_only: i64 = connection
                    .query_row("PRAGMA query_only", [], |row| row.get(0))
                    .map_err(|error| error.to_string())?;
                connection
                    .execute_batch("BEGIN DEFERRED;")
                    .map_err(|error| error.to_string())?;
                let _: i64 = connection
                    .query_row("SELECT COUNT(*) FROM global_volumes", [], |row| row.get(0))
                    .map_err(|error| error.to_string())?;
                Ok((journal_mode, query_only))
            })();
            let metadata = match setup {
                Ok(metadata) => metadata,
                Err(message) => {
                    let _ = ready_tx.send(Err(message.clone()));
                    return Err(message);
                }
            };
            ready_tx
                .send(Ok(metadata))
                .map_err(|error| error.to_string())?;
            release_rx.recv().map_err(|error| error.to_string())?;
            connection
                .execute_batch("ROLLBACK;")
                .map_err(|error| error.to_string())
        });
        Self {
            ready: ready_rx,
            release: Some(release_tx),
            thread: Some(thread),
        }
    }

    fn wait_until_open(&self) -> Result<(String, i64), String> {
        self.ready
            .recv_timeout(Duration::from_secs(10))
            .map_err(|error| error.to_string())?
    }

    fn release(mut self) -> Result<(), String> {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        self.join()
    }

    fn join(&mut self) -> Result<(), String> {
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .map_err(|_| "read-only observer thread panicked".to_string())?,
            None => Ok(()),
        }
    }
}

impl Drop for HeldReadSnapshot {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        let _ = self.join();
    }
}

fn fixture() -> (FixtureDirectory, Database, GlobalVolume) {
    let directory = FixtureDirectory::new();
    let database = Database::open(directory.database_path()).expect("open isolated database");
    let volume = GlobalVolume {
        id: "fixture-volume".to_string(),
        platform: "windows".to_string(),
        stable_volume_id: "fixture-volume-stable-id".to_string(),
        display_name: "Fixture volume".to_string(),
        mount_path: directory.0.to_string_lossy().into_owned(),
        filesystem_type: "NTFS".to_string(),
        drive_kind: "fixed".to_string(),
        enabled: true,
        provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
        index_status: INDEX_STATUS_INDEXING.to_string(),
        last_error: None,
        journal_id: None,
        journal_cursor: None,
        last_full_index_at: None,
        last_incremental_sync_at: None,
        entry_count: 0,
        created_at: 1,
        updated_at: 1,
    };
    database
        .upsert_global_volume(&volume)
        .expect("seed isolated Global Index volume");
    (directory, database, volume)
}

fn assert_database_configuration(database: &Database) {
    let connection = database
        .conn()
        .expect("borrow configured pooled connection");
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("read journal mode");
    let synchronous: i64 = connection
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .expect("read synchronous mode");
    let foreign_keys: i64 = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .expect("read foreign-key mode");
    let busy_timeout_ms: i64 = connection
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .expect("read configured busy timeout");

    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
    assert_eq!(synchronous, 1);
    assert_eq!(foreign_keys, 1);
    assert_eq!(busy_timeout_ms, BUSY_TIMEOUT.as_millis() as i64);
    println!(
        "issue_328_db_config journal_mode={journal_mode} synchronous={synchronous} foreign_keys={foreign_keys} busy_timeout_ms={busy_timeout_ms}"
    );
}

fn sqlite_error_codes(error: &SqliteError) -> Option<(ErrorCode, i32)> {
    match error {
        SqliteError::SqliteFailure(code, _) => Some((code.code, code.extended_code)),
        _ => None,
    }
}

fn assert_busy(error: &SqliteError) {
    match sqlite_error_codes(error) {
        Some((code, extended_code)) => {
            assert_eq!(
                code,
                ErrorCode::DatabaseBusy,
                "expected SQLITE_BUSY, got {error:?}"
            );
            assert_eq!(
                extended_code, 5,
                "expected plain SQLITE_BUSY (5), got {error:?}"
            );
        }
        None => panic!("expected SQLITE_BUSY (5), got {error:?}"),
    }
}

fn db_sqlite_error(error: &DbError) -> &SqliteError {
    match error {
        DbError::Sqlite(error) => error,
        other => panic!("expected SQLite failure, got {other:?}"),
    }
}

fn settings_sqlite_error(error: &SettingsError) -> &SqliteError {
    match error {
        SettingsError::Db(DbError::Sqlite(error)) => error,
        other => panic!("expected SQLite failure, got {other:?}"),
    }
}

fn assert_busy_wait(elapsed: Duration) {
    assert!(
        elapsed >= Duration::from_millis(4_500),
        "operation returned before the configured busy timeout: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(20),
        "operation exceeded the configured busy timeout by an unexpected amount: {elapsed:?}"
    );
}

#[test]
fn global_index_state_write_reproduces_busy_and_recovers_after_holder_release() {
    let (_directory, database, volume) = fixture();
    assert_database_configuration(&database);
    let holder = HeldWriteLock::begin(&database, &volume.id);

    let started = Instant::now();
    let error = database
        .update_global_volume_state(
            &volume.id,
            INDEX_STATUS_SYNCING,
            Some("fixture contender"),
            None,
            None,
            None,
            None,
        )
        .expect_err("a second writer must wait, then fail while the holder owns WAL write access");
    let elapsed = started.elapsed();
    assert_busy(db_sqlite_error(&error));
    assert_busy_wait(elapsed);

    let status_while_locked = database
        .global_index_status()
        .expect("WAL status reader should see the previous committed state");
    assert_eq!(status_while_locked.status, INDEX_STATUS_INDEXING);
    assert_eq!(
        database
            .get_global_volume(&volume.id)
            .expect("read durable volume while writer is active")
            .expect("seeded volume exists")
            .index_status,
        INDEX_STATUS_INDEXING,
        "failed update must not publish an uncommitted status"
    );

    let release_result = holder.release();
    assert!(
        release_result.is_ok(),
        "lock holder should commit: {release_result:?}"
    );
    database
        .update_global_volume_state(
            &volume.id,
            INDEX_STATUS_READY,
            None,
            None,
            None,
            Some(2),
            None,
        )
        .expect("the same Global Index status write should recover after release");
    let recovered = database
        .global_index_status()
        .expect("read recovered Global Index status");
    assert_eq!(recovered.status, INDEX_STATUS_READY);
    assert_eq!(recovered.last_error, None);
    println!(
        "issue_328_reproducer holder=second_pooled_connection transaction=BEGIN_IMMEDIATE table=global_volumes contender=Database::update_global_volume_state stage=UPDATE result=SQLITE_BUSY(5) extended_code=5 elapsed_ms={} released_retry=success final_status={}",
        elapsed.as_millis(), recovered.status
    );
}

#[test]
fn read_only_wal_observer_does_not_block_global_index_writer() {
    let (directory, database, volume) = fixture();
    let observer = HeldReadSnapshot::begin(&directory.database_path());
    let observer_metadata = observer
        .wait_until_open()
        .expect("read-only observer establishes a WAL snapshot");
    assert_eq!(observer_metadata.0.to_ascii_lowercase(), "wal");
    assert_eq!(
        observer_metadata.1, 1,
        "observer query_only must be enabled"
    );

    let started = Instant::now();
    database
        .update_global_volume_state(
            &volume.id,
            INDEX_STATUS_READY,
            None,
            None,
            None,
            Some(2),
            None,
        )
        .expect("a read-only WAL snapshot should not own the writer lock");
    let write_elapsed = started.elapsed();
    assert!(
        write_elapsed < BUSY_TIMEOUT,
        "read-only WAL observer delayed a writer unexpectedly: {write_elapsed:?}"
    );
    assert_eq!(
        database
            .global_index_status()
            .expect("read current state from another connection")
            .status,
        INDEX_STATUS_READY
    );
    let release_result = observer.release();
    assert!(
        release_result.is_ok(),
        "read-only observer should close cleanly: {release_result:?}"
    );

    println!(
        "issue_328_observer simulation=open_flags=SQLITE_OPEN_READ_ONLY query_only=1 journal_mode={} held_read_transaction=true writer_elapsed_ms={} classification=readers_do_not_block_wal_writer",
        observer_metadata.0,
        write_elapsed.as_millis()
    );
}

#[test]
fn deferred_settings_cas_reproduces_the_immediate_upgrade_failure_baseline() {
    let _test_guard = SETTINGS_CONTENTION_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (_directory, database, volume) = fixture();
    assert_database_configuration(&database);
    save_app_settings(&database, &AppSettings::default()).expect("seed default app settings");
    let previous = get_versioned_app_settings(&database).expect("load initial settings revision");
    let mut next = previous.settings.clone();
    next.background_index_on_startup = !previous.settings.background_index_on_startup;

    let holder = HeldWriteLock::begin(&database, &volume.id);
    let release_thread = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        holder.release()
    });
    let cas_started = Instant::now();
    let cas_error = save_app_settings_cas_for_test(
        &database,
        &next,
        previous.revision,
        TransactionBehavior::Deferred,
    )
    .expect_err("deferred read-to-write upgrade should fail while the writer is active");
    let cas_elapsed = cas_started.elapsed();
    assert_busy(settings_sqlite_error(&cas_error));
    assert!(
        cas_elapsed < Duration::from_millis(200),
        "deferred upgrade should reproduce the immediate SQLITE_BUSY failure: {cas_elapsed:?}"
    );
    let release_result = release_thread
        .join()
        .expect("deferred baseline writer release thread should not panic");
    assert!(
        release_result.is_ok(),
        "deferred baseline writer should commit: {release_result:?}"
    );
    let persisted = get_versioned_app_settings(&database).expect("reload baseline settings");
    assert_eq!(persisted.revision, previous.revision);
    assert_eq!(
        serde_json::to_value(&persisted.settings).expect("serialize persisted settings"),
        serde_json::to_value(&previous.settings).expect("serialize previous settings")
    );
    println!(
        "issue_329_correlation baseline=deferred_settings_cas stage=read_to_write_upgrade result=SQLITE_BUSY(5)/extended=5 elapsed_ms={} configured_busy_timeout_ms={} correlation=POSSIBLE_SHARED_CONTENTION",
        cas_elapsed.as_millis(),
        BUSY_TIMEOUT.as_millis()
    );
}

#[test]
fn settings_cas_waits_for_a_short_writer_and_saves_one_revision() {
    let _test_guard = SETTINGS_CONTENTION_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (directory, database, volume) = fixture();
    assert_database_configuration(&database);
    save_app_settings(&database, &AppSettings::default()).expect("seed default app settings");
    let previous = get_versioned_app_settings(&database).expect("load initial settings revision");

    let fixture_root = directory.0.join("short-contention-root");
    fs::create_dir_all(&fixture_root).expect("create permitted scan-root fixture");
    let mut next = previous.settings.clone();
    next.default_scan_folders = vec![ScanRootSetting {
        id: "fixture-short-contention-root".to_string(),
        path: fixture_root.to_string_lossy().into_owned(),
        label: "Fixture root".to_string(),
        enabled: true,
        created_at: "2026-10-08T00:00:00.000Z".to_string(),
    }];

    let holder = HeldWriteLock::begin(&database, &volume.id);
    let release_thread = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        holder.release()
    });
    let cas_started = Instant::now();
    let saved = save_app_settings_cas(&database, &next, previous.revision)
        .expect("CAS should wait for a short competing writer and save");
    let cas_elapsed = cas_started.elapsed();
    let release_result = release_thread
        .join()
        .expect("short writer release thread should not panic");
    assert!(
        release_result.is_ok(),
        "short writer should commit: {release_result:?}"
    );
    assert!(
        cas_elapsed >= Duration::from_millis(200),
        "CAS should overlap the held writer: {cas_elapsed:?}"
    );
    assert!(
        cas_elapsed < BUSY_TIMEOUT,
        "short contention should resolve inside the configured timeout: {cas_elapsed:?}"
    );

    let persisted = get_versioned_app_settings(&database).expect("reload persisted settings");
    assert_eq!(saved.revision, previous.revision + 1);
    assert_eq!(persisted.revision, previous.revision + 1);
    let mut expected_scan_root = next.default_scan_folders[0].clone();
    expected_scan_root.path = expected_scan_root.path.replace('\\', "/");
    assert_eq!(
        persisted.settings.default_scan_folders,
        vec![expected_scan_root]
    );
    println!(
        "issue_329_correlation holder=synthetic_second_pooled_connection_BEGIN_IMMEDIATE cas_operation=save_app_settings_cas stage=BEGIN_IMMEDIATE_acquisition result=success elapsed_ms={} timeout_ms={} correlation=POSSIBLE_SHARED_CONTENTION",
        cas_elapsed.as_millis(),
        BUSY_TIMEOUT.as_millis()
    );
}

#[test]
fn settings_cas_and_watcher_root_sync_share_the_database_writer_lock() {
    let _test_guard = SETTINGS_CONTENTION_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (directory, database, volume) = fixture();
    assert_database_configuration(&database);
    let initial = AppSettings::default();
    save_app_settings(&database, &initial).expect("seed default app settings");
    let previous = get_versioned_app_settings(&database).expect("load initial settings revision");

    let fixture_root = directory.0.join("scan-root");
    fs::create_dir_all(&fixture_root).expect("create permitted scan-root fixture");
    let mut next = previous.settings.clone();
    next.default_scan_folders = vec![ScanRootSetting {
        id: "fixture-scan-root".to_string(),
        path: fixture_root.to_string_lossy().into_owned(),
        label: "Fixture scan root".to_string(),
        enabled: true,
        created_at: "2026-10-08T00:00:00.000Z".to_string(),
    }];

    let holder = HeldWriteLock::begin(&database, &volume.id);
    let cas_started = Instant::now();
    let cas_error = save_app_settings_cas(&database, &next, previous.revision)
        .expect_err("settings CAS must surface the active SQLite writer conflict");
    let cas_elapsed = cas_started.elapsed();
    assert_busy(settings_sqlite_error(&cas_error));
    assert_busy_wait(cas_elapsed);

    // save_settings stops after the failed CAS. Exercise the next real boundary
    // separately to classify sync_file_library_watcher_roots' BEGIN IMMEDIATE.
    let sync_started = Instant::now();
    let sync_error = database
        .sync_file_library_watcher_roots(&next.default_scan_folders)
        .expect_err("root synchronization must surface the active SQLite writer conflict");
    let sync_elapsed = sync_started.elapsed();
    assert_busy(db_sqlite_error(&sync_error));
    assert_busy_wait(sync_elapsed);

    let persisted_while_locked =
        get_versioned_app_settings(&database).expect("read settings while writer is active");
    assert_eq!(persisted_while_locked.revision, previous.revision);
    assert!(persisted_while_locked
        .settings
        .default_scan_folders
        .is_empty());
    let root_count_while_locked: i64 = database
        .conn()
        .expect("borrow pooled connection for pre-commit root assertion")
        .query_row(
            "SELECT COUNT(*) FROM scan_roots WHERE source_kind = 'file_library' AND normalized_path = ?1",
            [fixture_root.to_string_lossy().as_ref()],
            |row| row.get(0),
        )
        .expect("read roots after both begin-time failures");
    assert_eq!(root_count_while_locked, 0);

    let release_result = holder.release();
    assert!(
        release_result.is_ok(),
        "lock holder should commit: {release_result:?}"
    );
    let saved = save_app_settings_cas(&database, &next, previous.revision)
        .expect("settings CAS should succeed after lock release");
    database
        .sync_file_library_watcher_roots(&saved.settings.default_scan_folders)
        .expect("watcher-root synchronization should succeed after lock release");

    let persisted = get_versioned_app_settings(&database).expect("reload persisted settings");
    assert_eq!(persisted.revision, previous.revision + 1);
    assert_eq!(persisted.settings.default_scan_folders.len(), 1);
    let connection = database
        .conn()
        .expect("borrow pooled connection for root assertion");
    let enabled_roots: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM scan_roots WHERE source_kind = 'file_library' AND enabled = 1",
            [],
            |row| row.get(0),
        )
        .expect("read synchronized root count");
    assert_eq!(enabled_roots, 1);

    println!(
        "issue_329_correlation holder=synthetic_second_pooled_connection_BEGIN_IMMEDIATE cas_operation=save_app_settings_cas stage=BEGIN_IMMEDIATE_acquisition result=SQLITE_BUSY(5)/extended=5 elapsed_ms={} root_sync_operation=sync_file_library_watcher_roots stage=BEGIN_IMMEDIATE_acquisition result=SQLITE_BUSY(5)/extended=5 elapsed_ms={} no_mutations_before_begin=true released_retry=cas_and_root_sync_success watcher_reload=not_exercised correlation=POSSIBLE_SHARED_CONTENTION",
        cas_elapsed.as_millis(),
        sync_elapsed.as_millis()
    );
}
