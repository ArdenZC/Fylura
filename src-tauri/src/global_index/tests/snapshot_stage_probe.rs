//! Opt-in, thread-local instrumentation of the unchanged production Snapshot path.
//! The trace collects executed work only; EXPLAIN is performed after the timed call.
use rusqlite::{ffi, Connection};
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    ffi::{c_void, CStr},
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct Probe {
    last: Option<Instant>,
    pub stages: BTreeMap<&'static str, f64>,
    pub mapping_ms: f64,
    pub hash_ms: f64,
    pub status_sql_ms: f64,
    pub null_correction_ms: f64,
    pub revision_facts_json: Vec<u8>,
    pub exact_count_parts: Option<(i64, i64)>,
    pub statements: Vec<Value>,
    collect_statements: bool,
}
thread_local! { static PROBE: RefCell<Option<Probe>> = const { RefCell::new(None) }; }

pub(crate) fn capture<T>(collect_statements: bool, call: impl FnOnce() -> T) -> (T, Probe) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            PROBE.with(|slot| {
                slot.borrow_mut().take();
            });
        }
    }
    PROBE.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Probe {
            collect_statements,
            ..Probe::default()
        });
    });
    let _reset = Reset;
    let result = call();
    let probe = PROBE.with(|slot| slot.borrow_mut().take().unwrap());
    (result, probe)
}
pub(crate) fn start_call() {
    ENABLED_COUNT.with(|sum| sum.set(EXACT_COUNT.with(std::cell::Cell::get).then_some(0)));
    PROBE.with(|slot| {
        if let Some(probe) = slot.borrow_mut().as_mut() {
            probe.last = Some(Instant::now());
        }
    });
}
pub(crate) fn mark(stage: &'static str) {
    let now = Instant::now();
    PROBE.with(|slot| {
        if let Some(probe) = slot.borrow_mut().as_mut() {
            let elapsed = now
                .duration_since(probe.last.replace(now).unwrap())
                .as_secs_f64()
                * 1000.;
            probe.stages.insert(stage, elapsed);
        }
    });
}
pub(crate) fn add_mapping(elapsed: Duration) {
    PROBE.with(|slot| {
        if let Some(probe) = slot.borrow_mut().as_mut() {
            probe.mapping_ms += elapsed.as_secs_f64() * 1000.;
        }
    });
}
pub(crate) fn add_hash(elapsed: Duration) {
    PROBE.with(|slot| {
        if let Some(probe) = slot.borrow_mut().as_mut() {
            probe.hash_ms += elapsed.as_secs_f64() * 1000.;
        }
    });
}

// SQLite owns stmt/SQL pointers until this synchronous callback returns. Expanded
// SQL is copied before sqlite3_free; no connection re-entry or panicking work.
unsafe extern "C" fn profile(
    _event: u32,
    _context: *mut c_void,
    stmt: *mut c_void,
    _ns: *mut c_void,
) -> i32 {
    PROBE.with(|slot| {
        if let Some(probe) = slot.borrow_mut().as_mut() {
            let stmt = stmt.cast::<ffi::sqlite3_stmt>();
            let sql = ffi::sqlite3_expanded_sql(stmt);
            if !sql.is_null() {
                let sql_text = CStr::from_ptr(sql).to_string_lossy().into_owned();
                ffi::sqlite3_free(sql.cast());
                let stage = if probe.stages.contains_key("status") { "commit" }
                    else if probe.stages.contains_key("source") { "status" }
                    else if probe.stages.contains_key("search") { "source" }
                    else if probe.stages.contains_key("begin") { "search" } else { "begin" };
                probe.statements.push(json!({"stage": stage, "sql": sql_text,
                    "vm_steps": ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_VM_STEP, 0),
                    "sort_operations": ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_SORT, 0),
                    "fullscan_steps": ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP, 0)}));
            }
        }
    });
    0
}
pub(crate) struct TraceGuard(*mut ffi::sqlite3);
impl TraceGuard {
    pub(crate) fn install(conn: &Connection) -> Self {
        let enabled = PROBE.with(|slot| {
            slot.borrow()
                .as_ref()
                .is_some_and(|probe| probe.collect_statements)
        });
        if !enabled {
            return Self(std::ptr::null_mut());
        }
        // Test-only connection is exclusively held until this guard is dropped.
        let handle = unsafe { conn.handle() };
        let result = unsafe {
            ffi::sqlite3_trace_v2(
                handle,
                ffi::SQLITE_TRACE_PROFILE,
                Some(profile),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(result, ffi::SQLITE_OK);
        Self(handle)
    }
}
impl Drop for TraceGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                ffi::sqlite3_trace_v2(self.0, 0, None, std::ptr::null_mut());
            }
        }
    }
}

pub(crate) struct MappingGuard(pub Instant);
impl Drop for MappingGuard {
    fn drop(&mut self) {
        add_mapping(self.0.elapsed());
    }
}

thread_local! { static SEQUENTIAL_SOURCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
pub(crate) fn with_sequential_source<T>(call: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            SEQUENTIAL_SOURCE.with(|slot| slot.set(self.0));
        }
    }
    let _reset = Reset(SEQUENTIAL_SOURCE.with(|slot| slot.replace(true)));
    call()
}
pub(crate) fn source_sql(original: &'static str) -> &'static str {
    static SEQUENTIAL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if SEQUENTIAL_SOURCE.with(std::cell::Cell::get) {
        SEQUENTIAL
            .get_or_init(|| {
                original.replacen("FROM global_entries", "FROM global_entries NOT INDEXED", 1)
            })
            .as_str()
    } else {
        original
    }
}

#[test]
fn sequential_source_preserves_nullable_orphan_disabled_empty_facts() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE global_volumes(id TEXT PRIMARY KEY, enabled INTEGER, provider TEXT, index_status TEXT, last_error TEXT, updated_at INTEGER);
        CREATE TABLE global_entries(id TEXT PRIMARY KEY, volume_id TEXT, is_stale INTEGER, last_seen_at INTEGER);
        CREATE INDEX idx_global_entries_volume ON global_entries(volume_id, is_stale);
        INSERT INTO global_volumes VALUES ('a',1,'p','ready',NULL,7),('b',0,'q','error','denied',8),('c',1,'p','paused',NULL,9);
        INSERT INTO global_entries VALUES ('1','a',0,10),(NULL,'a',0,30),('2','a',1,99),('3','b',0,NULL),('4','orphan',0,100);").unwrap();
    let tx = conn.transaction().unwrap();
    let reference = super::load_global_search_source_health_candidate(
        &tx,
        super::GlobalSearchSourceHealthQueryCandidate::NarrowAggregate,
    )
    .unwrap();
    let actual =
        with_sequential_source(|| super::load_global_search_sources_from_connection(&tx)).unwrap();
    assert_eq!(actual.0, reference.source_health);
    assert_eq!(actual.1, reference.source_revision);
    let facts: Value = serde_json::from_slice(&reference.revision_facts_json).unwrap();
    assert_eq!(facts[0]["active_entry_count"], 1);
    assert_eq!(facts[0]["max_last_seen_at"], 30);
    assert_eq!(facts[1]["active_entry_count"], 1);
    assert!(facts[1]["max_last_seen_at"].is_null());
    assert_eq!(facts[2]["active_entry_count"], 0);
    assert_eq!(facts.as_array().unwrap().len(), 3);
    let correction = null_correction_evidence(&tx);
    assert_eq!(correction["canonical_enabled_count_id_sum"], 1);
    assert_eq!(correction["null_id_correction"], 1);
    tx.commit().unwrap();
}

pub(crate) fn status_sql(elapsed: Duration) {
    PROBE.with(|slot| {
        if let Some(probe) = slot.borrow_mut().as_mut() {
            probe.status_sql_ms = elapsed.as_secs_f64() * 1000.;
        }
    });
}

/// Process-wide CPU and lifetime RSS high-water mark, outside the latency timer.
/// RSS cannot be attributed to one variant after the shared fixture is built.
#[cfg(target_os = "linux")]
pub(crate) fn linux_process_sample() -> Option<(f64, u64)> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    let usage = unsafe { usage.assume_init() };
    let cpu_ms = (usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) as f64 * 1000.
        + (usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) as f64 / 1000.;
    Some((cpu_ms, usage.ru_maxrss as u64 * 1024))
}
#[cfg(not(target_os = "linux"))]
pub(crate) fn linux_process_sample() -> Option<(f64, u64)> {
    None
}

/// Research only: exact COUNT(*) = enabled canonical COUNT(id) + active enabled
/// NULL-id rows. Unlike #433, adds no COUNT(*) accumulator to the full source scan.
/// Untimed count-identity probe retained beside the test-only full Snapshot A/B.
pub(crate) fn null_correction_evidence(conn: &Connection) -> Value {
    let facts = super::load_global_search_source_health_candidate(
        conn,
        super::GlobalSearchSourceHealthQueryCandidate::NarrowAggregate,
    )
    .unwrap();
    let facts: Value = serde_json::from_slice(&facts.revision_facts_json).unwrap();
    let nonnull: i64 = facts
        .as_array()
        .unwrap()
        .iter()
        .filter(|fact| fact["enabled"].as_bool().unwrap())
        .map(|fact| fact["active_entry_count"].as_i64().unwrap())
        .sum();
    let sql = NULL_CORRECTION_SQL;
    let mut statement = conn.prepare(sql).unwrap();
    let null_rows: i64 = statement.query_row([], |row| row.get(0)).unwrap();
    let reference: i64 = conn.query_row("SELECT COUNT(*) FROM global_entries entry JOIN global_volumes volume ON volume.id = entry.volume_id WHERE entry.is_stale = 0 AND volume.enabled = 1",[],|row|row.get(0)).unwrap();
    assert_eq!(nonnull + null_rows, reference);
    let plans = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map([], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    json!({"sql":sql,"query_plan":plans,"canonical_enabled_count_id_sum":nonnull,"null_id_correction":null_rows,
        "original_status_count_star":reference,"exact_count_equal":true,
        "vm_steps":statement.get_status(rusqlite::StatementStatus::VmStep),
        "sort_operations":statement.get_status(rusqlite::StatementStatus::Sort),
        "fullscan_steps":statement.get_status(rusqlite::StatementStatus::FullscanStep),
        "scope":"one untimed correction query; no full Snapshot A/B or latency-win claim"})
}

// These slots exist only in the repository's cfg(test) module. The count
// override is armed only around Snapshot status, never standalone Status reads.
thread_local! {
    static EXACT_COUNT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static ENABLED_COUNT: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
    static STATUS_COUNT: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
}
pub(crate) fn with_exact_count<T>(call: impl FnOnce() -> T) -> T {
    struct Reset(bool, Option<i64>);
    impl Drop for Reset {
        fn drop(&mut self) {
            EXACT_COUNT.with(|slot| slot.set(self.0));
            ENABLED_COUNT.with(|slot| slot.set(self.1));
        }
    }
    let _reset = Reset(
        EXACT_COUNT.with(|slot| slot.replace(true)),
        ENABLED_COUNT.with(std::cell::Cell::get),
    );
    call()
}
pub(crate) fn observe_source_count(enabled: bool, count: i64) {
    if enabled {
        ENABLED_COUNT.with(|slot| {
            if let Some(sum) = slot.get() {
                slot.set(Some(
                    sum.checked_add(count).expect("exact source count fits i64"),
                ));
            }
        });
    }
}
pub(crate) fn source_facts(bytes: &[u8]) {
    PROBE.with(|slot| {
        if let Some(probe) = slot.borrow_mut().as_mut() {
            probe.revision_facts_json = bytes.to_vec();
        }
    });
}

const NULL_CORRECTION_SQL: &str = "SELECT COUNT(*) FROM global_entries entry JOIN global_volumes volume ON volume.id = entry.volume_id WHERE entry.id IS NULL AND entry.is_stale = 0 AND volume.enabled = 1";
const ORIGINAL_STATUS_COUNT: &str = "(SELECT COUNT(*)
             FROM global_entries entry
             JOIN global_volumes volume ON volume.id = entry.volume_id
             WHERE entry.is_stale = 0 AND volume.enabled = 1)";

pub(crate) struct StatusCountGuard(Option<i64>);
impl Drop for StatusCountGuard {
    fn drop(&mut self) {
        STATUS_COUNT.with(|slot| slot.set(self.0));
    }
}
pub(crate) fn prepare_snapshot_status(
    conn: &Connection,
) -> Result<StatusCountGuard, crate::db::DbError> {
    let previous = STATUS_COUNT.with(std::cell::Cell::get);
    if EXACT_COUNT.with(std::cell::Cell::get) {
        let started = Instant::now();
        let null_rows: i64 = conn.query_row(NULL_CORRECTION_SQL, [], |row| row.get(0))?;
        let elapsed = started.elapsed().as_secs_f64() * 1000.;
        let nonnull = ENABLED_COUNT
            .with(std::cell::Cell::get)
            .expect("Snapshot initializes canonical count, including zero");
        let total = nonnull
            .checked_add(null_rows)
            .expect("exact status count fits i64");
        STATUS_COUNT.with(|slot| slot.set(Some(total)));
        PROBE.with(|slot| {
            if let Some(probe) = slot.borrow_mut().as_mut() {
                probe.null_correction_ms = elapsed;
                probe.exact_count_parts = Some((nonnull, null_rows));
            }
        });
    }
    Ok(StatusCountGuard(previous))
}

/// Reuses the original Status row mapper and state precedence without a test
/// copy. Only Snapshot's first count expression changes; every other SQL byte
/// and every standalone invocation retains the original behavior.
pub(crate) struct StatusConnection<'a> {
    conn: &'a Connection,
}
impl<'a> StatusConnection<'a> {
    pub(crate) fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
    pub(crate) fn query_row<T, P, F>(
        &self,
        sql: &str,
        parameters: P,
        mapper: F,
    ) -> rusqlite::Result<T>
    where
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        if let Some(total) = STATUS_COUNT.with(std::cell::Cell::get) {
            assert_eq!(sql.matches(ORIGINAL_STATUS_COUNT).count(), 1);
            let sql = sql.replacen(ORIGINAL_STATUS_COUNT, "?1", 1);
            self.conn.query_row(&sql, [total], mapper)
        } else {
            self.conn.query_row(sql, parameters, mapper)
        }
    }
}

/// Same database helpers on a caller-owned transaction, for pinned-reader and
/// nullable shadow-table tests. Performance samples use the real Repository API.
pub(crate) fn snapshot_on_connection(
    conn: &Connection,
    query: &str,
) -> Result<super::GlobalSearchSnapshot, crate::db::DbError> {
    start_call();
    let results =
        crate::global_index::search::search_global_entries_on_connection(conn, query, 80, 0)?;
    let (source_health, source_revision) = super::load_snapshot_source_health(conn)?;
    let _override = prepare_snapshot_status(conn)?;
    let index_status = super::global_index_status_from_connection(conn)?;
    Ok(super::GlobalSearchSnapshot {
        results,
        source_health,
        source_revision,
        index_status,
    })
}

#[test]
fn exact_count_override_preserves_standalone_status_and_zero_counts() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE global_volumes(id TEXT PRIMARY KEY, enabled INTEGER, provider TEXT, index_status TEXT, last_error TEXT, updated_at INTEGER, last_incremental_sync_at INTEGER);
        CREATE TABLE global_entries(id TEXT PRIMARY KEY, volume_id TEXT, is_stale INTEGER, last_seen_at INTEGER);
        INSERT INTO global_volumes VALUES ('a',1,'p','ready',NULL,1,NULL);
        INSERT INTO global_entries VALUES ('1','a',0,10),(NULL,'a',0,30);").unwrap();
    let reference = super::global_index_status_from_connection(&conn).unwrap();
    assert_eq!(reference.total_entries, 2);
    with_exact_count(|| {
        start_call();
        super::load_global_search_sources_from_connection(&conn).unwrap();
        {
            let _scope = prepare_snapshot_status(&conn).unwrap();
            assert_eq!(STATUS_COUNT.with(std::cell::Cell::get), Some(2));
            assert_eq!(
                super::global_index_status_from_connection(&conn).unwrap(),
                reference
            );
        }
        conn.execute("INSERT INTO global_entries VALUES ('2','a',0,50)", [])
            .unwrap();
        // Still inside the candidate opt-in, but outside Snapshot status scope:
        // a standalone call must see fresh SQL COUNT(*) rather than cached 2.
        assert_eq!(
            super::global_index_status_from_connection(&conn)
                .unwrap()
                .total_entries,
            3
        );
        conn.execute("DELETE FROM global_entries", []).unwrap();
        start_call();
        super::load_global_search_sources_from_connection(&conn).unwrap();
        let _scope = prepare_snapshot_status(&conn).unwrap();
        assert_eq!(STATUS_COUNT.with(std::cell::Cell::get), Some(0));
        assert_eq!(
            super::global_index_status_from_connection(&conn)
                .unwrap()
                .total_entries,
            0
        );
    });
    assert_eq!(STATUS_COUNT.with(std::cell::Cell::get), None);
    assert!(!EXACT_COUNT.with(std::cell::Cell::get));
}
