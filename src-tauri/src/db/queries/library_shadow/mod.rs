//! Dormant #417 F1-2 materialization. No runtime caller or publication authority.
//!
//! `Verified` means internally consistent captured bytes, never current-source
//! qualification. The deliberately narrow capture accepts completed scan-owned
//! coverage; other writer histories fail closed until their proof is implemented.
#![allow(dead_code)] // Deliberately compiled without a production entry point.

mod query;
mod slot;
mod snapshot;
#[cfg(test)]
mod tests;

use crate::{
    db::DbError,
    file_workspace::WorkClass,
    scheduler::{CancellationToken, ResourceHints, WorkRequest, WorkScheduler},
};
use rusqlite::{ffi, Connection};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[allow(unused_imports)] // Dormant crate-private surface, exercised by direct tests.
pub(crate) use slot::{
    open_shadow, recover_candidate, retire_candidate, verify, Candidate, Shadow,
};

static BUILD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
struct BuildPermit;
fn admit_single() -> Result<BuildPermit, DbError> {
    use std::sync::atomic::Ordering::Acquire;
    BUILD
        .compare_exchange(false, true, Acquire, std::sync::atomic::Ordering::Relaxed)
        .map_err(|_| unavailable("builder_busy"))?;
    Ok(BuildPermit)
}
impl Drop for BuildPermit {
    fn drop(&mut self) {
        BUILD.store(false, std::sync::atomic::Ordering::Release);
    }
}
const FORMAT: u32 = 1;
const RESERVE: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct BuildBudget {
    /// Conservative charged snapshot + SQLite allocator exposure, not process RSS.
    pub peak_ram_bytes: u64,
    /// Sum of all workspace files, including staging, manifests and journals.
    pub private_bytes: u64,
    pub staging_bytes: u64,
    pub main_read_age: Duration,
    /// Absolute observed WAL ceiling (also bounds growth from zero).
    pub main_wal_bytes: u64,
    pub work_age: Duration,
}

pub(crate) struct SnapshotReader<'a> {
    pub database: &'a Path,
    pub scheduler: Arc<WorkScheduler>,
}

pub(crate) fn unavailable(reason: &str) -> DbError {
    DbError::Validation(format!("UNAVAILABLE: {reason}"))
}

/// Explicit callers only. One admitted builder per process; no thread or wake loop.
pub(crate) fn prepare(
    reader: SnapshotReader<'_>,
    workspace: &Path,
    budget: BuildBudget,
    cancellation: CancellationToken,
) -> Result<Candidate, DbError> {
    if cancellation.is_cancelled() {
        return Err(unavailable("cancelled"));
    }
    let _single = admit_single()?;
    if budget.peak_ram_bytes <= RESERVE * 2
        || budget.staging_bytes == 0
        || budget.private_bytes == 0
        || budget.main_read_age.is_zero()
        || budget.work_age.is_zero()
    {
        return Err(unavailable("budget_missing"));
    }
    if budget.private_bytes <= RESERVE {
        return Err(unavailable("disk_budget"));
    }
    let request = WorkRequest::new(
        format!("dormant-shadow-{}", uuid::Uuid::new_v4()),
        WorkClass::Background,
        ResourceHints::cpu_io(1, 1),
    )
    .with_cancellation(cancellation.clone());
    let _lease = reader
        .scheduler
        .try_acquire(request)
        .map_err(|_| unavailable("resource_deferred"))?;
    let _qos = crate::resource_governor::scope_thread_qos(WorkClass::Background);
    let guard = Guard::new(workspace, reader.database, budget, cancellation);
    guard.check()?;
    // Require a caller-owned empty private workspace; never scavenge unknown files.
    if !workspace.is_dir() || fs::read_dir(workspace)?.next().is_some() {
        return Err(unavailable("workspace_not_empty"));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let directory = workspace.join(&id);
    fs::create_dir(&directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    }
    let mut candidate = Candidate::new(
        directory,
        id,
        guard.budget.clone(),
        reader.scheduler.clone(),
    );
    candidate.usage = guard.usage.clone();
    slot::state(&candidate, "STAGING")?;
    let result = (|| {
        let captured = snapshot::capture(reader.database, &guard)?;
        slot::build(&candidate, captured, &guard)?;
        slot::bind(candidate.clone())
    })();
    if result.is_err() {
        // Best effort only; status is never trusted by recovery or verify.
        let _ = slot::state(&candidate, "UNAVAILABLE");
    }
    result
}

#[derive(Default, Debug)]
pub(super) struct Usage {
    private_bytes: std::sync::atomic::AtomicU64,
    sqlite_bytes: std::sync::atomic::AtomicU64,
    charged_ram: std::sync::atomic::AtomicU64,
    wal_bytes: std::sync::atomic::AtomicU64,
    read_micros: std::sync::atomic::AtomicU64,
}
impl Usage {
    fn sample(&self) -> [u64; 5] {
        use std::sync::atomic::Ordering::Relaxed;
        [
            self.private_bytes.load(Relaxed),
            self.sqlite_bytes.load(Relaxed),
            self.charged_ram.load(Relaxed),
            self.wal_bytes.load(Relaxed),
            self.read_micros.load(Relaxed),
        ]
    }
}

#[derive(Clone)]
struct Guard {
    workspace: PathBuf,
    wal: PathBuf,
    budget: BuildBudget,
    cancellation: CancellationToken,
    start: Instant,
    read_start: Option<Instant>,
    charged: u64,
    usage: Arc<Usage>,
}

impl Guard {
    fn new(
        workspace: &Path,
        database: &Path,
        budget: BuildBudget,
        cancellation: CancellationToken,
    ) -> Self {
        let mut wal = database.as_os_str().to_os_string();
        wal.push("-wal");
        Self {
            workspace: workspace.into(),
            wal: wal.into(),
            budget,
            cancellation,
            start: Instant::now(),
            read_start: None,
            charged: 0,
            usage: Arc::new(Usage::default()),
        }
    }

    fn check(&self) -> Result<(), DbError> {
        if self.cancellation.is_cancelled() {
            return Err(unavailable("cancelled"));
        }
        if self.start.elapsed() >= self.budget.work_age {
            return Err(unavailable("work_deadline"));
        }
        use std::sync::atomic::Ordering::Relaxed;
        if let Some(start) = self.read_start {
            self.usage.read_micros.fetch_max(
                start.elapsed().as_micros().try_into().unwrap_or(u64::MAX),
                Relaxed,
            );
            if start.elapsed() >= self.budget.main_read_age {
                return Err(unavailable("read_deadline"));
            }
            let bytes = match fs::metadata(&self.wal) {
                Ok(m) => m.len(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
                Err(e) => return Err(e.into()),
            };
            self.usage.wal_bytes.fetch_max(bytes, Relaxed);
            if bytes > self.budget.main_wal_bytes {
                return Err(unavailable("wal_exposure"));
            }
        }
        let private = disk_bytes(&self.workspace)?;
        self.usage.private_bytes.fetch_max(private, Relaxed);
        if private > self.budget.private_bytes.saturating_sub(RESERVE) {
            return Err(unavailable("disk_budget"));
        }
        // Global allocator usage is intentionally conservative: other SQLite
        // users can cause rejection, never an extra global heap-limit setting.
        let sqlite_bytes = unsafe { ffi::sqlite3_memory_used() }.max(0) as u64;
        self.usage.sqlite_bytes.fetch_max(sqlite_bytes, Relaxed);
        self.usage.charged_ram.fetch_max(
            sqlite_bytes
                .saturating_add(self.charged)
                .saturating_add(RESERVE),
            Relaxed,
        );
        if sqlite_bytes
            .saturating_add(self.charged)
            .saturating_add(RESERVE)
            > self.budget.peak_ram_bytes
        {
            return Err(unavailable("ram_budget"));
        }
        Ok(())
    }
}

fn disk_bytes(path: &Path) -> Result<u64, DbError> {
    let mut total = 0u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.file_type()?;
        if meta.is_symlink() {
            return Err(unavailable("workspace_symlink"));
        }
        let bytes = if meta.is_dir() {
            disk_bytes(&entry.path())?
        } else {
            entry.metadata()?.len()
        };
        total = total
            .checked_add(bytes)
            .ok_or_else(|| unavailable("disk_overflow"))?;
    }
    Ok(total)
}

/// RAII SQLite interruption without a timer thread or a Cargo feature change.
/// The callback borrows only its own boxed guard; it never accesses the connection.
struct Progress<'a> {
    conn: &'a Connection,
    context: Box<Guard>,
}
impl<'a> Progress<'a> {
    fn install(conn: &'a Connection, guard: &Guard) -> Self {
        unsafe extern "C" fn check(p: *mut std::ffi::c_void) -> i32 {
            // Do not unwind across SQLite's C ABI, including on allocation panic.
            std::panic::catch_unwind(|| unsafe { (&*(p as *const Guard)).check().is_err() as i32 })
                .unwrap_or(1)
        }
        let mut context = Box::new(guard.clone());
        unsafe {
            ffi::sqlite3_progress_handler(
                conn.handle(),
                100,
                Some(check),
                (&mut *context as *mut Guard).cast(),
            );
        }
        Self { conn, context }
    }
}
impl Drop for Progress<'_> {
    fn drop(&mut self) {
        unsafe {
            ffi::sqlite3_progress_handler(self.conn.handle(), 0, None, std::ptr::null_mut());
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
enum Cell {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}
impl rusqlite::types::ToSql for Cell {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        use rusqlite::types::{ToSqlOutput, ValueRef};
        Ok(match self {
            Self::Null => ToSqlOutput::Borrowed(ValueRef::Null),
            Self::Integer(v) => ToSqlOutput::Borrowed(ValueRef::Integer(*v)),
            Self::Real(v) => ToSqlOutput::Borrowed(ValueRef::Real(*v)),
            Self::Text(v) => ToSqlOutput::Borrowed(ValueRef::Text(v.as_bytes())),
            Self::Blob(v) => ToSqlOutput::Borrowed(ValueRef::Blob(v)),
        })
    }
}
fn cell(row: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<Cell> {
    use rusqlite::types::ValueRef;
    Ok(match row.get_ref(i)? {
        ValueRef::Null => Cell::Null,
        ValueRef::Integer(v) => Cell::Integer(v),
        ValueRef::Real(v) if v.is_finite() => Cell::Real(v),
        ValueRef::Real(_) => return Err(rusqlite::Error::InvalidQuery),
        ValueRef::Text(v) => Cell::Text(
            std::str::from_utf8(v)
                .map_err(|_| rusqlite::Error::InvalidQuery)?
                .to_string(),
        ),
        ValueRef::Blob(v) => Cell::Blob(v.into()),
    })
}
fn digest<T: Serialize>(value: &T) -> Result<String, DbError> {
    Ok(blake3::hash(&serde_json::to_vec(value)?)
        .to_hex()
        .to_string())
}
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
