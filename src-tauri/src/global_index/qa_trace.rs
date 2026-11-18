//! Narrow diagnostics for the disposable hosted Global Index qualification.
//! This is inert unless the binary was explicitly built with `native-qa` and
//! the runner supplies a task-owned trace path.

#[cfg(feature = "native-qa")]
static TRACE_FILE_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

#[cfg(feature = "native-qa")]
thread_local! {
    static FILE_UPSERT_LOCK_RETRY_SAMPLE: std::cell::Cell<u64> = std::cell::Cell::new(0);
}

#[cfg(feature = "native-qa")]
pub(crate) fn record(event: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;

    let Some(path) = std::env::var_os("ZC_GLOBAL_INDEX_QA_TRACE") else {
        return;
    };
    // The Service and IPC probe share the same configured trace base path.
    // Give each process its own file to avoid cross-process append interleaving;
    // serialize threads within a process so each diagnostic event remains one
    // parseable line. Both behaviors exist only in native-qa builds.
    let pid = std::process::id();
    let mut process_path = path;
    process_path.push(format!(".pid-{pid}.log"));
    let _guard = TRACE_FILE_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(process_path)
    else {
        return;
    };
    let timestamp_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();
    let line = format!("timestamp_us={timestamp_us} pid={pid} {event}\n");
    let _ = file.write_all(line.as_bytes());
}

#[cfg(feature = "native-qa")]
pub(crate) fn is_enabled() -> bool {
    std::env::var_os("ZC_GLOBAL_INDEX_QA_TRACE").is_some()
}

#[cfg(feature = "native-qa")]
/// The filename-first qualification brackets each synthetic `files` UPSERT
/// with reset/take so its transaction retries stay attributable to that call.
pub(crate) fn reset_file_upsert_lock_retry_sample() {
    FILE_UPSERT_LOCK_RETRY_SAMPLE.with(|sample| sample.set(0));
}

#[cfg(feature = "native-qa")]
pub(crate) fn take_file_upsert_lock_retry_sample() -> u64 {
    FILE_UPSERT_LOCK_RETRY_SAMPLE.with(|sample| sample.replace(0))
}

#[cfg(feature = "native-qa")]
pub(crate) fn record_file_upsert_lock_retry(attempt: usize) {
    FILE_UPSERT_LOCK_RETRY_SAMPLE.with(|sample| {
        sample.set(sample.get().saturating_add(1));
    });
    if is_enabled() {
        record(&format!(
            "sqlite_lock_retry source=files.insert_files attempt={attempt}"
        ));
    }
}

#[cfg(feature = "native-qa")]
static ACTIVE_TRANSACTIONS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A coarse transaction scope used only by native qualification. It records
/// transaction lifetimes, never individual statements or indexed rows.
pub(crate) struct TransactionTrace {
    #[cfg(feature = "native-qa")]
    source: &'static str,
    #[cfg(feature = "native-qa")]
    kind: &'static str,
    #[cfg(feature = "native-qa")]
    started: std::time::Instant,
    #[cfg(feature = "native-qa")]
    enabled: bool,
}

impl TransactionTrace {
    #[inline]
    pub(crate) fn start(kind: &'static str, source: &'static str) -> Self {
        #[cfg(feature = "native-qa")]
        {
            let enabled = is_enabled();
            if enabled {
                ACTIVE_TRANSACTIONS.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                record(&format!(
                    "sqlite_transaction phase=begin kind={kind} source={source} active_transactions={}",
                    ACTIVE_TRANSACTIONS.load(std::sync::atomic::Ordering::Acquire),
                ));
            }
            Self {
                source,
                kind,
                started: std::time::Instant::now(),
                enabled,
            }
        }
        #[cfg(not(feature = "native-qa"))]
        {
            let _ = (kind, source);
            Self {}
        }
    }
}

impl Drop for TransactionTrace {
    fn drop(&mut self) {
        #[cfg(feature = "native-qa")]
        if self.enabled {
            let active = ACTIVE_TRANSACTIONS
                .fetch_sub(1, std::sync::atomic::Ordering::AcqRel)
                .saturating_sub(1);
            record(&format!(
                "sqlite_transaction phase=end kind={} source={} elapsed_us={} active_transactions={active}",
                self.kind,
                self.source,
                self.started.elapsed().as_micros(),
            ));
        }
    }
}

#[cfg(feature = "native-qa")]
pub(crate) fn active_transaction_count() -> usize {
    ACTIVE_TRANSACTIONS.load(std::sync::atomic::Ordering::Acquire)
}

#[cfg(not(feature = "native-qa"))]
pub(crate) fn active_transaction_count() -> usize {
    0
}

#[cfg(feature = "native-qa")]
pub(crate) fn record_db_error(source: &str, error: &crate::db::DbError) {
    use rusqlite::ErrorCode;

    let Some((code, message)) = (match error {
        crate::db::DbError::Sqlite(rusqlite::Error::SqliteFailure(code, message))
            if matches!(
                code.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            ) =>
        {
            Some((code.extended_code, message.as_deref().unwrap_or_default()))
        }
        _ => None,
    }) else {
        return;
    };
    record(&format!(
        "sqlite_busy source={source} extended_code={code} message={message:?}"
    ));
}

#[cfg(not(feature = "native-qa"))]
pub(crate) fn record_db_error(_source: &str, _error: &crate::db::DbError) {}

#[cfg(feature = "native-qa")]
pub(crate) fn record_sqlite_error(source: &str, error: &rusqlite::Error) {
    let rusqlite::Error::SqliteFailure(code, message) = error else {
        return;
    };
    if !matches!(
        code.code,
        rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
    ) {
        return;
    }
    record(&format!(
        "sqlite_busy source={source} extended_code={} message={:?}",
        code.extended_code,
        message.as_deref().unwrap_or_default(),
    ));
}

#[cfg(not(feature = "native-qa"))]
pub(crate) fn record_sqlite_error(_source: &str, _error: &rusqlite::Error) {}

/// Returns SQLite pager write and cache-spill counters since the previous
/// sample. This is used only by the disposable qualification trace so the
/// production writer does not pay for diagnostic SQLite calls.
#[cfg(feature = "native-qa")]
pub(crate) fn take_page_cache_counters(connection: &rusqlite::Connection) -> Option<(i32, i32)> {
    use rusqlite::ffi;

    // SAFETY: only read/reset connection-local status counters while the
    // caller exclusively owns this Connection lease; no raw handle escapes.
    let handle = unsafe { connection.handle() };
    let read_counter = |operation| {
        let mut current = 0;
        let mut highwater = 0;
        // SAFETY: SQLite writes two valid c_int outputs and the handle above
        // remains alive for the duration of this call.
        let result =
            unsafe { ffi::sqlite3_db_status(handle, operation, &mut current, &mut highwater, 1) };
        (result == ffi::SQLITE_OK).then_some(current)
    };

    Some((
        read_counter(ffi::SQLITE_DBSTATUS_CACHE_SPILL)?,
        read_counter(ffi::SQLITE_DBSTATUS_CACHE_WRITE)?,
    ))
}

#[cfg(not(feature = "native-qa"))]
pub(crate) fn record(_event: &str) {}

#[cfg(not(feature = "native-qa"))]
pub(crate) fn is_enabled() -> bool {
    false
}

#[cfg(not(feature = "native-qa"))]
pub(crate) fn reset_file_upsert_lock_retry_sample() {}

#[cfg(not(feature = "native-qa"))]
pub(crate) fn take_file_upsert_lock_retry_sample() -> u64 {
    0
}

#[cfg(not(feature = "native-qa"))]
pub(crate) fn record_file_upsert_lock_retry(_attempt: usize) {}

#[cfg(not(feature = "native-qa"))]
pub(crate) fn take_page_cache_counters(_connection: &rusqlite::Connection) -> Option<(i32, i32)> {
    None
}
