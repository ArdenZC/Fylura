//! Startup migration policy. SQLite is the only recovery authority: a failed
//! upgrade rolls back; a new attempt re-reads user_version under the write lock.
use super::DbError;
use rusqlite::{ffi, Connection};
use std::{
    ffi::CStr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub type UpgradeCancellation = Arc<AtomicBool>;

#[derive(Debug, Clone)]
pub struct UpgradeResources {
    pub available_memory_bytes: u64,
    pub page_size: u64,
    pub page_count: u64,
    pub freelist_pages: u64,
    pub active_rows: u64,
    pub key_bytes: u64,
    pub facts_key_bytes: u64,
    pub database_required_bytes: u64,
    pub temporary_required_bytes: u64,
    pub database_free_bytes: u64,
    pub temporary_free_bytes: u64,
    pub temporary_directory: PathBuf,
}

// Planning headroom, not a guarantee against another process consuming space.
// Do not credit free pages: WAL can coexist with both old and new B-trees until
// checkpoint. Three payload copies cover low B-tree occupancy; two generations
// of external-sort runs are budgeted separately on SQLite's actual temp volume.
const RESERVE: u64 = 64 * 1024 * 1024;
fn budgets(name: u64, source: u64, page: u64) -> Result<(u64, u64), DbError> {
    let calculate = || {
        let indexes = name
            .checked_add(source)?
            .checked_add(page * 4)?
            .checked_mul(3)?;
        let database = indexes
            .checked_mul(2)?
            .checked_add(indexes / page * 24)?
            .checked_add(RESERVE)?;
        let temporary = name.max(source).checked_mul(2)?.checked_add(RESERVE)?;
        Some((database, temporary))
    };
    calculate().ok_or_else(|| DbError::Validation("Upgrade resource estimate overflow".into()))
}

// Facts need one B-tree row per distinct active/registered source. The streaming
// index GROUP BY below bounds memory; registered/active overlap is overcounted.
fn facts_budgets(payload: u64, page: u64) -> Result<(u64, u64), DbError> {
    budgets(payload, 0, page)
}

pub fn inspect_resources(conn: &Connection) -> Result<UpgradeResources, DbError> {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    let available_memory_bytes = system.available_memory();
    if available_memory_bytes < 128 * 1024 * 1024 {
        return Err(DbError::Validation("Not enough available memory for the database upgrade (128 MiB working reserve required). Close other applications and retry. No schema changes were committed.".into()));
    }
    let page_size: u64 = conn.query_row("PRAGMA page_size", [], |r| {
        r.get::<_, i64>(0).map(|v| v as u64)
    })?;
    let page_count = conn.query_row("PRAGMA page_count", [], |r| {
        r.get::<_, i64>(0).map(|v| v as u64)
    })?;
    let freelist_pages = conn.query_row("PRAGMA freelist_count", [], |r| {
        r.get::<_, i64>(0).map(|v| v as u64)
    })?;
    // Exact production-shaped keys, including UTF-8 byte lengths, nullable IDs,
    // integer serial types/header/rowid allowance. This read is cancellable and
    // occurs before any DROP/CREATE; it does not build a second fixture/index.
    let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let grouped_sql = if version < 43 {
        "SELECT COALESCE(SUM(n),0), COALESCE(SUM(length(CAST(volume_id AS BLOB))+64),0)
         FROM (SELECT volume_id,COUNT(*) AS n FROM global_entries INDEXED BY idx_global_entries_volume WHERE is_stale=0 GROUP BY volume_id)"
    } else {
        "SELECT COALESCE(SUM(n),0), COALESCE(SUM(length(CAST(volume_id AS BLOB))+64),0)
         FROM (SELECT volume_id,COUNT(*) AS n FROM global_entries INDEXED BY idx_global_entries_active_source_facts WHERE is_stale=0 GROUP BY volume_id)"
    };
    let (active_rows, active_fact_bytes): (u64, u64) = conn.query_row(grouped_sql, [], |r| {
        Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64))
    })?;
    let (name_bytes, source_bytes): (u64, u64) = if version < 43 {
        conn.query_row(
            "SELECT COALESCE(SUM(length(CAST(name_normalized AS BLOB)) + COALESCE(length(CAST(id AS BLOB)),0) + length(CAST(volume_id AS BLOB)) + 64),0),
             COALESCE(SUM(length(CAST(volume_id AS BLOB)) + COALESCE(length(CAST(id AS BLOB)),0) + 64),0)
             FROM global_entries WHERE is_stale=0", [], |r| Ok((r.get::<_,i64>(0)? as u64,r.get::<_,i64>(1)? as u64)))?
    } else {
        (0, 0)
    };
    let volume_bytes: u64 = conn.query_row(
        "SELECT COALESCE(SUM(length(CAST(id AS BLOB))+64),0) FROM global_volumes",
        [],
        |r| r.get::<_, i64>(0).map(|v| v as u64),
    )?;
    let facts_payload = active_fact_bytes
        .checked_add(volume_bytes)
        .ok_or_else(|| DbError::Validation("Upgrade resource estimate overflow".into()))?;
    let (mut database_required_bytes, mut temporary_required_bytes) =
        facts_budgets(facts_payload, page_size)?;
    if version < 43 {
        let (index_db, index_temp) = budgets(name_bytes, source_bytes, page_size)?;
        database_required_bytes = database_required_bytes
            .checked_add(index_db)
            .ok_or_else(|| DbError::Validation("Upgrade resource estimate overflow".into()))?;
        temporary_required_bytes = temporary_required_bytes
            .checked_add(index_temp)
            .ok_or_else(|| DbError::Validation("Upgrade resource estimate overflow".into()))?;
    }
    let database_path: String = conn.query_row(
        "SELECT file FROM pragma_database_list WHERE name='main'",
        [],
        |r| r.get(0),
    )?;
    let database_directory = Path::new(&database_path).parent().unwrap_or(Path::new("."));
    let temporary_directory = sqlite_temporary_directory(conn)?;
    let (database_free_bytes, database_device) = checked_space(database_directory)?;
    let (temporary_free_bytes, temporary_device) = checked_space(&temporary_directory)?;
    let required = if database_device == temporary_device {
        database_required_bytes.saturating_add(temporary_required_bytes)
    } else {
        database_required_bytes
    };
    if database_free_bytes < required || temporary_free_bytes < temporary_required_bytes {
        return Err(DbError::Validation(format!(
            "Not enough free space for the database upgrade. Database volume: {} MiB free, {} MiB reserved; SQLite temporary volume: {} MiB free, {} MiB reserved. Free space and retry. No schema changes were committed.",
            database_free_bytes >> 20, required >> 20, temporary_free_bytes >> 20, temporary_required_bytes >> 20)));
    }
    Ok(UpgradeResources {
        available_memory_bytes,
        page_size,
        page_count,
        freelist_pages,
        active_rows,
        key_bytes: name_bytes.saturating_add(source_bytes),
        facts_key_bytes: facts_payload,
        database_required_bytes,
        temporary_required_bytes,
        database_free_bytes,
        temporary_free_bytes,
        temporary_directory,
    })
}

fn sqlite_temporary_directory(conn: &Connection) -> Result<PathBuf, DbError> {
    let mut filename: *mut std::ffi::c_char = std::ptr::null_mut();
    // SQLite allocates the VFS-selected candidate; this does not create a file
    // or change process-global temp_store_directory / environment variables.
    let rc = unsafe {
        ffi::sqlite3_file_control(
            conn.handle(),
            c"main".as_ptr(),
            ffi::SQLITE_FCNTL_TEMPFILENAME,
            (&mut filename as *mut *mut std::ffi::c_char).cast(),
        )
    };
    if rc != ffi::SQLITE_OK || filename.is_null() {
        return Err(DbError::Validation("Cannot determine SQLite temporary storage. Check that the system temporary directory is available, then retry.".into()));
    }
    let bytes = unsafe { CStr::from_ptr(filename).to_bytes().to_vec() };
    unsafe { ffi::sqlite3_free(filename.cast()) };
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_vec(bytes))
    };
    #[cfg(not(unix))]
    let path = PathBuf::from(
        String::from_utf8(bytes)
            .map_err(|_| DbError::Validation("Invalid SQLite temporary path".into()))?,
    );
    path.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| DbError::Validation("SQLite temporary path has no parent".into()))
}

#[cfg(test)]
thread_local! { static TEST_CAPACITY: std::cell::Cell<Option<u64>> = const {std::cell::Cell::new(None)}; }
#[cfg(test)]
pub(crate) fn with_test_capacity<T>(bytes: u64, run: impl FnOnce() -> T) -> T {
    struct Reset(Option<u64>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_CAPACITY.with(|v| v.set(self.0));
        }
    }
    let _reset = Reset(TEST_CAPACITY.with(|v| v.replace(Some(bytes))));
    run()
}
fn checked_space(path: &Path) -> Result<(u64, u64), DbError> {
    let (free, device) = free_space(path)?;
    #[cfg(test)]
    let free = TEST_CAPACITY.with(|v| v.get()).unwrap_or(free);
    Ok((free, device))
}

#[cfg(test)]
pub(crate) fn storage_free_bytes(path: &Path) -> Result<u64, DbError> {
    free_space(path).map(|(free, _)| free)
}

#[cfg(unix)]
#[allow(clippy::unnecessary_cast)] // statvfs field widths differ between Unix targets.
fn free_space(path: &Path) -> Result<(u64, u64), DbError> {
    use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
    let path_c = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| DbError::Validation("Invalid storage path".into()))?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path_c.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let stat = unsafe { stat.assume_init() };
    Ok((
        (stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64),
        std::fs::metadata(path)?.dev(),
    ))
}

#[cfg(windows)]
fn free_space(path: &Path) -> Result<(u64, u64), DbError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
    };
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0;
    let mut mount = [0u16; 32768];
    let mut volume = [0u16; 1024];
    if unsafe {
        GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
        || unsafe { GetVolumePathNameW(path.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) } == 0
        || unsafe {
            GetVolumeNameForVolumeMountPointW(
                mount.as_ptr(),
                volume.as_mut_ptr(),
                volume.len() as u32,
            )
        } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    // Only an equality key is needed, not a persisted volume identity.
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    volume.hash(&mut hasher);
    Ok((free, hasher.finish()))
}

pub(crate) fn check_cancelled(cancel: &AtomicBool) -> Result<(), DbError> {
    if cancel.load(Ordering::Acquire) {
        Err(DbError::Validation(
            "Upgrade cancelled. No schema changes were committed. Start Fylura again to retry."
                .into(),
        ))
    } else {
        Ok(())
    }
}

unsafe extern "C" fn progress(context: *mut std::ffi::c_void) -> std::ffi::c_int {
    let cancel = unsafe { &*(context.cast::<AtomicBool>()) };
    i32::from(cancel.load(Ordering::Acquire))
}

/// Owns the callback allocation and connection settings until rollback/commit.
/// It must outlive the transaction. Restore settings before returning to pool.
pub(crate) struct UpgradeGuard<'a> {
    conn: &'a Connection,
    cancel: UpgradeCancellation,
    temp: i64,
    cache: i64,
    mmap: i64,
    threads: i64,
    bounded: bool,
}
impl<'a> UpgradeGuard<'a> {
    pub(crate) fn new(
        conn: &'a Connection,
        cancel: UpgradeCancellation,
        bounded: bool,
    ) -> Result<Self, DbError> {
        let value = |name: &str| -> rusqlite::Result<i64> {
            conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))
        };
        let guard = Self {
            conn,
            cancel,
            temp: value("temp_store")?,
            cache: value("cache_size")?,
            mmap: conn
                .query_row("PRAGMA mmap_size", [], |r| r.get(0))
                .unwrap_or(0),
            threads: value("threads")?,
            bounded,
        };
        if bounded {
            conn.execute_batch("PRAGMA temp_store=FILE; PRAGMA cache_size=-8192; PRAGMA mmap_size=0; PRAGMA threads=0;")?;
        }
        unsafe {
            ffi::sqlite3_progress_handler(
                conn.handle(),
                1000,
                Some(progress),
                Arc::as_ptr(&guard.cancel).cast_mut().cast(),
            )
        };
        guard.check_cancelled()?;
        Ok(guard)
    }
    pub(crate) fn check_cancelled(&self) -> Result<(), DbError> {
        check_cancelled(&self.cancel)
    }
}
impl Drop for UpgradeGuard<'_> {
    fn drop(&mut self) {
        unsafe { ffi::sqlite3_progress_handler(self.conn.handle(), 0, None, std::ptr::null_mut()) };
        if !self.bounded {
            return;
        }
        let _ = self.conn.execute_batch(&format!(
            "PRAGMA temp_store={}; PRAGMA cache_size={}; PRAGMA mmap_size={}; PRAGMA threads={};",
            self.temp, self.cache, self.mmap, self.threads
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budgets_are_checked_and_do_not_credit_freelist() {
        let (db, tmp) = budgets(1000, 2000, 4096).unwrap();
        assert!(db > RESERVE + 18000 && tmp >= RESERVE + 4000);
        assert!(budgets(u64::MAX, 1, 4096).is_err());
    }
    #[test]
    fn cancellation_interrupts_sql_and_restores_connection() {
        let conn = Connection::open_in_memory().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let original: i64 = conn
            .query_row("PRAGMA temp_store", [], |r| r.get(0))
            .unwrap();
        {
            let _guard = UpgradeGuard::new(&conn, cancel.clone(), true).unwrap();
            cancel.store(true, Ordering::Release);
            let result=conn.execute_batch("WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<100000) SELECT sum(x) FROM n");
            assert!(result.is_err());
        }
        assert_eq!(
            original,
            conn.query_row::<i64, _, _>("PRAGMA temp_store", [], |r| r.get(0))
                .unwrap()
        );
        conn.execute_batch("SELECT 1").unwrap();
    }
}
