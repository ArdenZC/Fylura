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
                ffi::SQLITE_TRACE_PROFILE as u32,
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
/// This is not wired into status or claimed as a qualified Snapshot candidate.
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
    let sql = "SELECT COUNT(*) FROM global_entries entry JOIN global_volumes volume ON volume.id = entry.volume_id WHERE entry.id IS NULL AND entry.is_stale = 0 AND volume.enabled = 1";
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
