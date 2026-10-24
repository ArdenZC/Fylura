//! Issue #364: observations of real pooled requests and separately labelled SQL
//! controls. Reuses the benchmark's sole fixture; never populates another one.
use super::query_cost_diagnostic::{
    exact_sql_1m_tier_parameters, explain_details, map_search_result,
};
use super::*;
use crate::global_index::models::GlobalSearchResult;
use crate::global_index::search::diagnostic_search_tier_sql;
use rusqlite::{ffi, StatementStatus};
use std::cell::RefCell;
use std::ffi::{c_void, CStr};

thread_local! {
    static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) };
}
struct Trace {
    origin: Instant,
    active: Option<(String, f64, Option<f64>, usize)>,
    events: Vec<JsonValue>,
}
fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

// No Rust references cross the C boundary. State is thread-local, the callback
// never panics, and only a test-owned pooled connection is instrumented.
unsafe extern "C" fn trace_callback(
    event: u32,
    _: *mut c_void,
    statement: *mut c_void,
    _: *mut c_void,
) -> std::ffi::c_int {
    let _ = TRACE.try_with(|slot| {
        let Ok(mut slot) = slot.try_borrow_mut() else { return };
        let Some(trace) = slot.as_mut() else { return };
        let stmt = statement.cast::<ffi::sqlite3_stmt>();
        if event == ffi::SQLITE_TRACE_STMT {
            let ptr = unsafe { ffi::sqlite3_sql(stmt) };
            if ptr.is_null() { return; }
            let sql = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned();
            // FTS issues nested internal SELECTs. They are already included in
            // the parent duration; do not double count or overwrite its state.
            let top = sql.contains("SELECT ge.id,") || sql.trim() == "SELECT 1";
            if top && trace.active.is_none() {
                trace.active = Some((sql, ms(trace.origin), None, 0));
            }
        } else if event == ffi::SQLITE_TRACE_ROW {
            if let Some((sql, _, first, count)) = trace.active.as_mut() {
                let ptr = unsafe { ffi::sqlite3_sql(stmt) };
                if !ptr.is_null() && unsafe { CStr::from_ptr(ptr) }.to_bytes() == sql.as_bytes() {
                    first.get_or_insert(ms(trace.origin));
                    *count += 1;
                }
            }
        } else if event == ffi::SQLITE_TRACE_PROFILE {
            let ptr = unsafe { ffi::sqlite3_sql(stmt) };
            let matches = trace.active.as_ref().is_some_and(|(sql, ..)| {
                !ptr.is_null() && unsafe { CStr::from_ptr(ptr) }.to_bytes() == sql.as_bytes()
            });
            if matches {
                if let Some((sql, begin, first, count)) = trace.active.take() {
                    let end = ms(trace.origin);
                    trace.events.push(json!({
                        "sql": sql, "start_ms": begin, "profile_ms": end,
                        "statement_interval_ms": end - begin,
                        "first_row_ms": first, "rows": count,
                        "vm_steps": unsafe { ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_VM_STEP, 0) },
                        "sorts": unsafe { ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_SORT, 0) },
                        "fullscan_steps": unsafe { ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP, 0) },
                        "reprepare": unsafe { ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_REPREPARE, 0) }
                    }));
                }
            }
        }
    });
    0
}

fn actual_request(db: &Database, query: &str, offset: u32) -> (Vec<GlobalSearchResult>, JsonValue) {
    actual_request_with_control(db, query, offset, None)
}
fn actual_request_with_control(
    db: &Database,
    query: &str,
    offset: u32,
    sql_control: Option<(&str, &[SqlValue])>,
) -> (Vec<GlobalSearchResult>, JsonValue) {
    // Hold seven of the eight connections so the actual public production
    // method checks out the instrumented eighth. No shared pool is touched.
    let held = (0..7).map(|_| db.conn().unwrap()).collect::<Vec<_>>();
    let conn = db.conn().unwrap();
    unsafe {
        assert_eq!(
            ffi::sqlite3_trace_v2(
                conn.handle(),
                ffi::SQLITE_TRACE_STMT | ffi::SQLITE_TRACE_ROW | ffi::SQLITE_TRACE_PROFILE,
                Some(trace_callback),
                std::ptr::null_mut()
            ),
            ffi::SQLITE_OK
        );
    }
    drop(conn);
    let origin = Instant::now();
    TRACE.with(|slot| {
        *slot.borrow_mut() = Some(Trace {
            origin,
            active: None,
            events: vec![],
        })
    });
    let results = db
        .search_global_entries(query, SEARCH_RESULT_LIMIT, offset)
        .unwrap();
    let total = ms(origin);
    let trace = TRACE.with(|slot| slot.borrow_mut().take().unwrap());
    let conn = db.conn().unwrap();
    unsafe {
        assert_eq!(
            ffi::sqlite3_trace_v2(conn.handle(), 0, None, std::ptr::null_mut()),
            ffi::SQLITE_OK
        );
    }
    let same_handle_control = sql_control.map(|(sql, values)| split_sql(&conn, sql, values).1);
    drop(conn);
    drop(held);
    let event_ms: f64 = trace
        .events
        .iter()
        .map(|e| e["statement_interval_ms"].as_f64().unwrap())
        .sum();
    (
        results,
        json!({
            "total_ms": total, "events": trace.events, "same_pooled_handle_sql_control":same_handle_control,
            "outside_statement_intervals_ms": total - event_ms,
            "scope": "actual Database::search_global_entries; intervals include SQLite steps and intervening Rust row mapping; remainder includes pool validation/checkout, SQL construction/prepare, reset/finalize, dedup and paging; no percentile subtraction",
            "pool": "test-owned 8-connection production pool; 7 held to force known traced handle; uncontended checkout only"
        }),
    )
}

// Companion boundary control executes the exact operations of the public
// wrapper, with clocks around checkout/production route/release in this call.
// It is not a replacement implementation of the search route.
fn checkout_route_control(db: &Database, query: &str) -> (Vec<GlobalSearchResult>, JsonValue) {
    let origin = Instant::now();
    let conn = db.conn().unwrap();
    let checkout = ms(origin);
    let start = Instant::now();
    let results = crate::global_index::search::search_global_entries_on_connection(
        &conn,
        query,
        SEARCH_RESULT_LIMIT,
        0,
    )
    .unwrap();
    let route = ms(start);
    let start = Instant::now();
    drop(conn);
    let release = ms(start);
    (
        results,
        json!({"checkout_ms":checkout,"production_route_ms":route,
        "release_ms":release,"total_ms":ms(origin),
        "scope":"same-call boundary control of the public wrapper operations; actual production on-connection search; no trace callbacks or tier replay"}),
    )
}

fn split_sql(
    conn: &Connection,
    sql: &str,
    values: &[SqlValue],
) -> (Vec<GlobalSearchResult>, JsonValue) {
    let origin = Instant::now();
    let mut statement = conn.prepare(sql).unwrap();
    let prepare = ms(origin);
    let mut result = Vec::new();
    let mut step = 0.0;
    let mut mapping = 0.0;
    let mut first_step = 0.0;
    let bind_start = Instant::now();
    let mut rows = statement.query(params_from_iter(values.iter())).unwrap();
    let bind = ms(bind_start);
    loop {
        let start = Instant::now();
        let row = rows.next().unwrap();
        let elapsed = ms(start);
        if result.is_empty() {
            first_step += elapsed;
        }
        step += elapsed;
        let Some(row) = row else { break };
        let start = Instant::now();
        result.push(map_search_result(row).unwrap());
        mapping += ms(start);
    }
    let start = Instant::now();
    drop(rows);
    let rows_drop = ms(start);
    let counters = json!({ "vm_steps": statement.get_status(StatementStatus::VmStep),
        "sorts": statement.get_status(StatementStatus::Sort),
        "fullscan_steps": statement.get_status(StatementStatus::FullscanStep) });
    let before_finalize = ms(origin);
    let start = Instant::now();
    drop(statement);
    let finalize = ms(start);
    let total = ms(origin);
    (
        result,
        json!({ "prepare_ms": prepare, "bind_ms": bind,
        "step_ms": step, "first_step_ms": first_step, "mapping_ms": mapping,
        "rows_drop_ms": rows_drop, "before_finalize_ms": before_finalize,
        "finalize_ms": finalize, "total_ms": total, "counters": counters,
        "scope": "one SQL control execution, including explicit statement drop; not an instrumented production Rust mapper" }),
    )
}

fn candidate_sql(tier: &str, original: &str) -> String {
    match tier {
        // Existing recency index, no new index/schema. Exact same predicates,
        // order, bound limit and full outer projection. Low-hit scan risk.
        "name_prefix" => original.replace(
            "idx_global_entries_active_name_order",
            "idx_global_entries_modified_at",
        ),
        // Preserve FTS-first, weighted BM25 and both total orders. Let SQLite
        // consider a coroutine rather than require a materialized temp table.
        "fts" => original.replacen("AS MATERIALIZED", "AS NOT MATERIALIZED", 1),
        _ => original.to_string(),
    }
}

// Test-only route replay; every SQL control is compared with the real method.
// Only the selected tier is replaced. Limits, dedup and paging mirror search.rs.
fn candidate_route(
    conn: &Connection,
    query: &str,
    scope: &str,
    offset: u32,
) -> Vec<GlobalSearchResult> {
    use std::collections::HashSet;
    let query = query.trim();
    if query.is_empty() || offset >= 4096 {
        return Vec::new();
    }
    let target = (offset + SEARCH_RESULT_LIMIT).min(4096);
    let mut results = Vec::new();
    let mut seen = HashSet::new();
    let mut tiers = vec![
        "exact_name",
        "name_prefix",
        "exact_extension",
        "extension_prefix",
    ];
    if query.chars().count() >= 3 {
        tiers.push(
            if query
                .chars()
                .all(|c| c.is_alphanumeric() || c.is_whitespace())
            {
                "fts"
            } else {
                "punctuation_prefix"
            },
        );
    }
    for tier in tiers {
        if results.len() >= target as usize {
            break;
        }
        let limit = (target + seen.len() as u32).clamp(1, 4096);
        let sql = diagnostic_search_tier_sql(tier);
        let sql = if tier == scope {
            candidate_sql(tier, &sql)
        } else {
            sql
        };
        let values = exact_sql_1m_tier_parameters(tier, query, limit);
        for result in split_sql(conn, &sql, &values).0 {
            if seen.insert(result.id.clone()) {
                results.push(result);
            }
            if results.len() >= target as usize {
                break;
            }
        }
    }
    results
        .into_iter()
        .skip(offset as usize)
        .take(SEARCH_RESULT_LIMIT as usize)
        .collect()
}

pub(super) fn profile_existing_fixture(db: &Database, conn: &Connection, context: &JsonValue) {
    let mut cases = QUERY_MATRIX
        .iter()
        .copied()
        .map(|case| (case, None))
        .collect::<Vec<_>>();
    let missing = QUERY_MATRIX
        .iter()
        .copied()
        .find(|case| case.class == "no_result")
        .unwrap();
    cases.extend([(missing, Some("name_prefix")), (missing, Some("fts"))]);
    adversarial_equivalence(conn);
    emit_record(
        &json!({"record_type":"search_path_adversarial", "context":context,
        "full_objects_equal":true, "cases":12, "candidate_scopes":["name_prefix","fts"],
        "offsets":[0,40,4016,4096], "filters":"stale/disabled before limit", "managed":"enabled and disabled entries/scopes", "ties":true}),
    );
    for (case, forced_tier) in cases {
        let tier = forced_tier.unwrap_or(match case.class {
            "name_prefix"
            | "common_prefix_high_fanout"
            | "chinese_prefix"
            | "punctuation_prefix"
            | "unicode_accented_prefix" => "name_prefix",
            "fts_substring_report" | "fts_substring_invoice" => "fts",
            "extension_prefix" => "extension_prefix",
            _ => "exact_name",
        });
        let original = diagnostic_search_tier_sql(tier);
        let candidate = candidate_sql(tier, &original);
        let values = exact_sql_1m_tier_parameters(tier, case.query, SEARCH_RESULT_LIMIT);
        let mut originals = Vec::new();
        let mut candidates = Vec::new();
        let mut actual = Vec::new();
        for sample in 0..10 {
            let (results, request) =
                actual_request_with_control(db, case.query, 0, Some((&original, &values)));
            let (baseline, checkout_control) = checkout_route_control(db, case.query);
            assert_eq!(results, baseline);
            let (a, b) = if sample % 2 == 0 {
                (
                    split_sql(conn, &original, &values),
                    split_sql(conn, &candidate, &values),
                )
            } else {
                let b = split_sql(conn, &candidate, &values);
                (split_sql(conn, &original, &values), b)
            };
            assert_eq!(
                a.0, b.0,
                "full objects including BM25/managed: {}",
                case.class
            );
            originals.push(a.1["total_ms"].as_f64().unwrap());
            candidates.push(b.1["total_ms"].as_f64().unwrap());
            actual.push(request["total_ms"].as_f64().unwrap());
            emit_record(
                &json!({ "record_type": "search_path_same_call_sample", "context": context,
                "query_class": case.class, "sample": sample, "tier": tier,
                "actual_request": request, "checkout_route_control":checkout_control, "sql_control": a.1, "sql_candidate": b.1,
                "candidate_is_identity_control": original == candidate,
                "full_objects_equal": true }),
            );
        }
        for limit in [1, 80, 200, 4096] {
            let values = exact_sql_1m_tier_parameters(tier, case.query, limit);
            let original_window = split_sql(conn, &original, &values);
            let candidate_window = split_sql(conn, &candidate, &values);
            assert_eq!(original_window.0, candidate_window.0);
            emit_record(
                &json!({"record_type":"search_path_window_control", "context":context,
                "query_class":case.class,"tier":tier,"sql_limit":limit,
                "original":original_window.1,"candidate":candidate_window.1,
                "result_count":original_window.0.len(),"full_objects_equal":true,
                "policy":"one exploratory execution per bound limit; no tail claim"}),
            );
        }
        for offset in [0, 40, 4016, 4096] {
            let (observed, request) = actual_request(db, case.query, offset);
            emit_record(
                &json!({"record_type":"search_path_page_trace", "context":context,
                "query_class":case.class,"candidate_scope":tier,"offset":offset,
                "actual_request":request,"result_count":observed.len()}),
            );
            assert_eq!(
                observed,
                candidate_route(conn, case.query, tier, offset),
                "candidate full route and page: {}",
                case.class
            );
            assert_eq!(
                observed,
                db.search_global_entries(case.query, SEARCH_RESULT_LIMIT, offset)
                    .unwrap()
            );
        }
        emit_record(
            &json!({ "record_type": "search_path_candidate_summary", "context": context,
            "query_class": case.class, "tier": tier, "original_sql": original, "candidate_sql": candidate,
            "parameters": values.iter().map(|v| format!("{v:?}")).collect::<Vec<_>>(),
            "original_eqp": explain_details(conn, &original, &values),
            "candidate_eqp": explain_details(conn, &candidate, &values),
            "original_including_finalize_ms": summarize_samples(&originals),
            "candidate_including_finalize_ms": summarize_samples(&candidates),
            "actual_pooled_ms": summarize_samples(&actual),
            "policy": "10 exploratory paired samples, alternating SQL order; no performance qualification; complete raw stage samples retained",
            "extension_policy": "identity only; historical extension CTE never selected" }),
        );
    }
}

fn adversarial_equivalence(conn: &Connection) {
    conn.execute_batch("SAVEPOINT search_path_adversarial;
        INSERT INTO global_volumes(id, platform, stable_volume_id, display_name, mount_path,
            filesystem_type, drive_kind, enabled, provider, created_at, updated_at)
        VALUES('stage_disabled','synthetic','stage_disabled','disabled','/disabled',
            'synthetic','fixed',0,'synthetic',1,1);
        UPDATE global_entries SET volume_id='stage_disabled' WHERE rowid % 11 = 0;
        UPDATE global_entries SET is_stale=1 WHERE rowid % 13 = 0;
        UPDATE global_entries SET modified_at_fs=42 WHERE rowid % 7 = 0;
        INSERT INTO managed_scopes(id,path,enabled,created_at,updated_at)
        VALUES('stage_enabled','/stage_enabled',1,1,1),('stage_disabled','/stage_disabled',0,1,1);
        INSERT INTO managed_entries(id,global_entry_id,managed_scope_id,enabled,created_at,updated_at)
        SELECT 'stage_a_' || id,id,'stage_enabled',rowid % 2,1,1
        FROM global_entries WHERE rowid % 17 = 0;
        INSERT INTO managed_entries(id,global_entry_id,managed_scope_id,enabled,created_at,updated_at)
        SELECT 'stage_b_' || id,id,'stage_disabled',1,1,1
        FROM global_entries WHERE rowid % 19 = 0;").unwrap();
    for case in QUERY_MATRIX {
        for scope in ["name_prefix", "fts"] {
            for offset in [0, 40, 4016, 4096] {
                let original = crate::global_index::search::search_global_entries_on_connection(
                    conn,
                    case.query,
                    SEARCH_RESULT_LIMIT,
                    offset,
                )
                .unwrap();
                assert_eq!(
                    original,
                    candidate_route(conn, case.query, scope, offset),
                    "adversarial full result/order/rank/managed/page: {} / {scope}",
                    case.class
                );
            }
        }
    }
    conn.execute_batch("ROLLBACK TO search_path_adversarial; RELEASE search_path_adversarial;")
        .unwrap();
}

#[test]
fn search_path_trace_and_candidates_preserve_adversarial_results() {
    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).unwrap();
    db.upsert_global_volume(&test_volume()).unwrap();
    populate_synthetic_database(&db, 512);
    let conn = configured_connection(&path).unwrap();
    adversarial_equivalence(&conn);
    let (actual, record) = actual_request(&db, "quarterly", 0);
    assert_eq!(
        actual,
        db.search_global_entries("quarterly", 80, 0).unwrap()
    );
    let events = record["events"].as_array().unwrap();
    let search_events = events
        .iter()
        .filter(|e| e["sql"].as_str().unwrap().contains("SELECT ge.id,"))
        .collect::<Vec<_>>();
    assert_eq!(
        search_events.len(),
        5,
        "real low-hit route runs all five tiers once"
    );
    assert!(search_events
        .iter()
        .all(|e| e["vm_steps"].as_i64().unwrap() > 0));
}
