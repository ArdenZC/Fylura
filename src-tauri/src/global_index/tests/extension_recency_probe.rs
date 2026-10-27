//! Test-only extension algorithms. Release builds never compile this module.
use super::*;
use rusqlite::types::Value;

std::thread_local! {
    static MODE: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    static FIRST_PAGE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static AFTER_RAW: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static FALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
pub(crate) fn with_mode<T>(mode: u8, run: impl FnOnce() -> T) -> T {
    struct Reset(u8);
    impl Drop for Reset {
        fn drop(&mut self) {
            MODE.with(|v| v.set(self.0));
        }
    }
    let _reset = Reset(MODE.with(|v| v.replace(mode)));
    run()
}
pub(crate) fn page(limit: u32, offset: u32) {
    FIRST_PAGE.with(|v| v.set(offset == 0 && limit <= 80));
}
pub(crate) fn mode() -> u8 {
    MODE.with(std::cell::Cell::get)
}
pub(crate) fn after_raw(hook: impl FnOnce() + 'static) {
    AFTER_RAW.with(|v| *v.borrow_mut() = Some(Box::new(hook)));
}
pub(crate) fn fell_back() -> bool {
    FALLBACK.with(std::cell::Cell::get)
}
pub(crate) const RAW_SQL: &str = "SELECT rowid, id, modified_at_fs FROM global_entries INDEXED BY idx_global_entries_modified_at ORDER BY modified_at_fs DESC LIMIT 1601";
pub(crate) fn unlimited_sql() -> String {
    candidate_sql(
        "global_entries ge INDEXED BY idx_global_entries_modified_at CROSS JOIN global_volumes gv",
        "gv.id = ge.volume_id AND gv.enabled = 1 AND ge.is_stale = 0 AND ge.extension GLOB ?2 AND ge.extension <> lower(?1)",
        "ge.modified_at_fs DESC, ge.id ASC", "1.0", "?3")
}
fn execute(
    conn: &Connection,
    sql: &str,
    query: &str,
    limit: u32,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    collect_candidates(
        &mut conn.prepare(sql)?,
        params![query, format!("{}*", escape_glob(query)), limit],
    )
}
pub(crate) fn run(
    conn: &Connection,
    query: &str,
    limit: u32,
    first_page: bool,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    // Both standalone pooled calls and an already active Repository snapshot
    // retain one SQLite read view across raw keys, hydration, and fallback.
    let transaction = if conn.is_autocommit() {
        Some(conn.unchecked_transaction()?)
    } else {
        None
    };
    let read = transaction.as_deref().unwrap_or(conn);
    FALLBACK.with(|v| v.set(false));
    if mode() == 1 {
        return execute(read, &unlimited_sql(), query, limit);
    }
    let fallback = || {
        FALLBACK.with(|v| v.set(true));
        execute(
            read,
            &diagnostic_search_tier_sql("extension_prefix"),
            query,
            limit,
        )
    };
    if !first_page || limit > 160 {
        return fallback();
    }
    let raw = read
        .prepare(RAW_SQL)?
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Value>(1)?,
                row.get::<_, Value>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    AFTER_RAW.with(|v| {
        if let Some(hook) = v.borrow_mut().take() {
            hook();
        }
    });
    // Dynamic SQLite types, nullable legacy IDs and NULL mtimes are not a
    // total order this experiment can certify. Preserve the production path,
    // including its error behavior on a selected NULL ID.
    if raw
        .iter()
        .any(|(_, id, mtime)| !matches!(id, Value::Text(_)) || !matches!(mtime, Value::Integer(_)))
    {
        return fallback();
    }
    // The existing index has no id suffix. Read one lookahead and discard its
    // entire mtime group BEFORE sorting by id. This avoids unbounded tie-block
    // sorting by SQLite and gives a provably complete prefix in (mtime,id).
    // If a huge tie covers the budget, the certified prefix is empty: fallback.
    let boundary = if raw.len() == 1601 {
        Some(&raw[1600].2)
    } else {
        None
    };
    let keys = raw
        .iter()
        .take(1600)
        .filter(|(_, _, t)| boundary != Some(t))
        .map(|(rowid, _, _)| rowid.to_string())
        .collect::<Vec<_>>();
    if keys.is_empty() {
        return fallback();
    }
    let sql = candidate_sql("global_entries ge CROSS JOIN global_volumes gv",
        &format!("ge.rowid IN ({}) AND gv.id = ge.volume_id AND gv.enabled = 1 AND ge.is_stale = 0 AND ge.extension GLOB ?2 AND ge.extension <> lower(?1)", keys.join(",")),
        "ge.modified_at_fs DESC, ge.id ASC", "1.0", "?3");
    let rows = execute(read, &sql, query, limit)?;
    if rows.len() == limit as usize {
        Ok(rows)
    } else {
        fallback()
    }
}
pub(crate) fn dispatch(
    conn: &Connection,
    query: &str,
    limit: u32,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    run(conn, query, limit, FIRST_PAGE.with(std::cell::Cell::get))
}
