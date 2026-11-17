//! Narrow diagnostics for the disposable hosted Global Index qualification.
//! This is inert unless the binary was explicitly built with `native-qa` and
//! the runner supplies a task-owned trace path.

#[cfg(feature = "native-qa")]
pub(crate) fn record(event: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;

    let Some(path) = std::env::var_os("ZC_GLOBAL_INDEX_QA_TRACE") else {
        return;
    };
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "{event}");
}

#[cfg(feature = "native-qa")]
pub(crate) fn is_enabled() -> bool {
    std::env::var_os("ZC_GLOBAL_INDEX_QA_TRACE").is_some()
}

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
pub(crate) fn take_page_cache_counters(_connection: &rusqlite::Connection) -> Option<(i32, i32)> {
    None
}
