use super::schema::migrate;
use super::*;
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
        Arc, Mutex,
    },
};

const LIBRARY_COUNT_CACHE_MAX_ENTRIES: usize = 32;

struct LibraryCountCacheEntry {
    revision: i64,
    membership_fingerprint: String,
    total_count: i64,
}

#[derive(Clone, Default)]
struct ManagedAiWakeSlot {
    sender: Arc<Mutex<Option<SyncSender<()>>>>,
    work_wakes_armed: Arc<AtomicBool>,
}

impl ManagedAiWakeSlot {
    fn set(&self, sender: SyncSender<()>) {
        self.set_work_wakes_armed(false);
        *self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(sender);
    }

    fn set_work_wakes_armed(&self, armed: bool) {
        self.work_wakes_armed.store(armed, Ordering::Release);
    }

    fn notify_control(&self) {
        self.send_wake();
    }

    fn notify_work(&self) {
        if self.work_wakes_armed.load(Ordering::Acquire) {
            self.send_wake();
        }
    }

    fn send_wake(&self) {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(sender) = sender {
            // A full one-slot channel already contains a wake. The durable SQLite
            // queue remains the source of work; this signal only asks the worker
            // to inspect it.
            let _ = sender.try_send(());
        }
    }
}

#[derive(Clone)]
pub struct Database {
    path: PathBuf,
    pool: Pool<SqliteConnectionManager>,
    library_count_cache: Arc<Mutex<VecDeque<LibraryCountCacheEntry>>>,
    managed_ai_waker: ManagedAiWakeSlot,
    pub(crate) automation_wake: Arc<Mutex<Option<std::sync::Weak<AutomationWake>>>>,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let manager = SqliteConnectionManager::file(&path).with_init(configure_connection);
        let pool = Pool::builder()
            .max_size(8)
            .min_idle(Some(1))
            .build(manager)?;
        {
            let conn = pool.get()?;
            migrate(&conn)?;
        }

        Ok(Self {
            path,
            pool,
            library_count_cache: Arc::new(Mutex::new(VecDeque::new())),
            managed_ai_waker: ManagedAiWakeSlot::default(),
            automation_wake: Arc::new(Mutex::new(None)),
        })
    }

    pub(crate) fn set_managed_ai_waker(&self, sender: SyncSender<()>) {
        self.managed_ai_waker.set(sender);
    }

    /// Force the worker to re-read durable settings or policy state.
    pub(crate) fn notify_managed_ai_control_wake(&self) {
        self.managed_ai_waker.notify_control();
    }

    pub(crate) fn set_managed_ai_work_wakes_armed(&self, armed: bool) {
        self.managed_ai_waker.set_work_wakes_armed(armed);
    }

    /// Signal newly eligible durable work only while the worker is interested.
    pub(crate) fn notify_managed_ai_work(&self) {
        self.managed_ai_waker.notify_work();
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn init(&self) -> Result<(), DbError> {
        let conn = self.conn()?;
        migrate(&conn)
    }

    pub(crate) fn conn(&self) -> Result<PooledConnection<SqliteConnectionManager>, DbError> {
        self.pool.get().map_err(DbError::from)
    }

    /// Verify the normal timeout, then set the test fixture timeout on every pooled connection.
    #[cfg(test)]
    pub(crate) fn set_test_busy_timeout_for_all_connections(
        &self,
        expected_current_ms: i64,
        timeout: std::time::Duration,
    ) -> Result<(), DbError> {
        let pool_size = self.pool.max_size();
        let mut connections = Vec::with_capacity(pool_size as usize);
        for _ in 0..pool_size {
            connections.push(self.conn()?);
        }

        let timeout_ms = timeout.as_millis() as i64;
        for connection in &mut connections {
            let current_ms =
                connection.query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))?;
            if current_ms != expected_current_ms {
                return Err(DbError::Validation(format!(
                    "test database busy timeout was {current_ms}ms, expected {expected_current_ms}ms before override"
                )));
            }

            connection.busy_timeout(timeout)?;
            let configured_ms =
                connection.query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))?;
            if configured_ms != timeout_ms {
                return Err(DbError::Validation(format!(
                    "test database busy timeout override was {configured_ms}ms, expected {timeout_ms}ms"
                )));
            }
        }

        Ok(())
    }

    pub(crate) fn cached_library_count(
        &self,
        revision: i64,
        membership_fingerprint: &str,
    ) -> Option<i64> {
        let mut cache = self
            .library_count_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let index = cache.iter().position(|entry| {
            entry.revision == revision && entry.membership_fingerprint == membership_fingerprint
        })?;
        let entry = cache.remove(index)?;
        let total_count = entry.total_count;
        cache.push_back(entry);
        Some(total_count)
    }

    pub(crate) fn cache_library_count(
        &self,
        revision: i64,
        membership_fingerprint: String,
        total_count: i64,
    ) {
        let mut cache = self
            .library_count_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.retain(|entry| {
            entry.revision != revision || entry.membership_fingerprint != membership_fingerprint
        });
        cache.push_back(LibraryCountCacheEntry {
            revision,
            membership_fingerprint,
            total_count,
        });
        while cache.len() > LIBRARY_COUNT_CACHE_MAX_ENTRIES {
            cache.pop_front();
        }
    }
}

fn configure_connection(conn: &mut Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "mmap_size", 3_000_000_000_i64)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(())
}

#[cfg(test)]
mod pool_tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDatabaseDirectory(PathBuf);

    impl TestDatabaseDirectory {
        fn new() -> Self {
            let counter = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(".tmp-tests")
                .join("zb-01-pool-tests")
                .join(format!("{}-{nonce}-{counter}", std::process::id()));
            fs::create_dir_all(&path).expect("create test database directory");
            Self(path)
        }

        fn database_path(&self) -> PathBuf {
            self.0.join("database.sqlite3")
        }
    }

    impl Drop for TestDatabaseDirectory {
        fn drop(&mut self) {
            let mut last_error = None;
            for attempt in 0..20 {
                match fs::remove_dir_all(&self.0) {
                    Ok(()) => return,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                    Err(error) => last_error = Some(error),
                }
                if attempt < 19 {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            if let Some(error) = last_error {
                eprintln!(
                    "failed to remove test database directory {:?}: {error}",
                    self.0
                );
            }
        }
    }

    #[test]
    fn managed_ai_wake_slot_buffers_and_coalesces_wakes() {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let wake_slot = ManagedAiWakeSlot::default();
        wake_slot.set(sender);

        // Disarmed ordinary work signals do not wake a worker that has read that
        // AI is disabled.
        wake_slot.notify_work();
        assert_eq!(
            receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        );

        // A producer can signal after the worker's last queue check but before it
        // begins waiting; the buffered token makes that transition race-safe.
        wake_slot.set_work_wakes_armed(true);
        wake_slot.notify_work();
        wake_slot.notify_work();
        assert_eq!(receiver.try_recv(), Ok(()));
        assert_eq!(
            receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        );

        // Separate clones share the same bounded, payload-free notification slot.
        let cloned_slot = wake_slot.clone();
        cloned_slot.set_work_wakes_armed(false);
        cloned_slot.notify_work();
        assert_eq!(
            receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        );
        cloned_slot.notify_control();
        assert_eq!(receiver.try_recv(), Ok(()));
    }

    #[test]
    fn database_open_uses_minimum_idle_configuration_and_runs_migrations() {
        let test_dir = TestDatabaseDirectory::new();
        let db = Database::open(test_dir.database_path()).expect("open and migrate database");
        let state = db.pool.state();

        eprintln!(
            "pool state after Database::open: connections={}, idle_connections={}, max_size={}, min_idle={:?}",
            state.connections,
            state.idle_connections,
            db.pool.max_size(),
            db.pool.min_idle()
        );
        assert_eq!(db.pool.max_size(), 8);
        assert_eq!(db.pool.min_idle(), Some(1));

        assert!((1..=2).contains(&state.connections));
        assert_eq!(state.idle_connections, state.connections);
        assert!(state.connections < db.pool.max_size());

        let conn = db.conn().expect("get connection after migration");
        let files_table_count = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'files'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("query migrated schema");
        assert_eq!(files_table_count, 1);
        let query_result = conn
            .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .expect("run ordinary query");
        assert_eq!(query_result, 1);
    }

    #[test]
    fn database_pool_grows_for_multiple_leases_and_reuses_returned_connections() {
        let test_dir = TestDatabaseDirectory::new();
        let db = Database::open(test_dir.database_path()).expect("open and migrate database");

        let first = db.conn().expect("lease first connection");
        let second = db.conn().expect("lease second connection");
        let third = db.conn().expect("lease third connection");
        let held_state = db.pool.state();

        eprintln!(
            "pool state with three leases held: connections={}, idle_connections={}, max_size={}, min_idle={:?}",
            held_state.connections,
            held_state.idle_connections,
            db.pool.max_size(),
            db.pool.min_idle()
        );
        assert_eq!(db.pool.max_size(), 8);
        assert_eq!(db.pool.min_idle(), Some(1));
        assert!(held_state.connections >= 3);
        assert!(held_state.connections <= db.pool.max_size());
        assert!(held_state.idle_connections < held_state.connections);

        drop((first, second, third));
        let returned_state = db.pool.state();
        eprintln!(
            "pool state after returning three leases: connections={}, idle_connections={}",
            returned_state.connections, returned_state.idle_connections
        );
        assert!(returned_state.idle_connections >= 1);
        assert!(returned_state.connections <= db.pool.max_size());

        let conn = db.conn().expect("lease connection after returning leases");
        let query_result = conn
            .query_row("SELECT 42", [], |row| row.get::<_, i64>(0))
            .expect("query using returned pool connection");
        assert_eq!(query_result, 42);
    }
}
