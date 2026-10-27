//! One generated fixture, four controlled copies, interleaved unchanged public calls.
use super::query_cost_diagnostic::{
    covering_index_sql_sample, exact_sql_1m_tier_order, exact_sql_1m_tier_parameters,
    explain_details,
};
use super::*;
use crate::global_index::repository::diagnostic_load_production_source_health;
use crate::global_index::search::{diagnostic_search_tier, diagnostic_search_tier_sql};
use rusqlite::ffi;
use std::ffi::{c_void, CStr};
use std::sync::Mutex;

const NAME: &str = "idx_global_entries_active_name_order";
const SOURCE: &str = "idx_global_entries_active_source_facts";

fn install(conn: &Connection, variant: usize) -> f64 {
    let started = Instant::now();
    conn.execute_batch(&format!("DROP INDEX {NAME}; DROP INDEX IF EXISTS {SOURCE};
        CREATE INDEX {NAME} ON global_entries(name_normalized, modified_at_fs DESC, id ASC{}) WHERE is_stale = 0;",
        if variant & 1 != 0 { ", volume_id" } else { "" })).unwrap();
    if variant & 2 != 0 {
        conn.execute_batch(&format!("CREATE INDEX {SOURCE} ON global_entries(volume_id, last_seen_at, id) WHERE is_stale = 0;")).unwrap();
    }
    started.elapsed().as_secs_f64() * 1000.0
}

fn assert_snapshot(
    a: &crate::global_index::repository::GlobalSearchSnapshot,
    b: &crate::global_index::repository::GlobalSearchSnapshot,
) {
    assert_eq!(a.results, b.results);
    assert_eq!(a.source_health, b.source_health);
    assert_eq!(a.source_revision.as_bytes(), b.source_revision.as_bytes());
    assert_eq!(a.index_status, b.index_status);
}

// Latency runs have tracing disabled. This separate real Snapshot observation
// reads counters at PROFILE, while statements still exist. Inner FTS statements
// are labelled and retained; never add their work to parent statement totals.
unsafe extern "C" fn profile_callback(
    mask: u32,
    context: *mut c_void,
    statement: *mut c_void,
    _elapsed: *mut c_void,
) -> i32 {
    if mask != ffi::SQLITE_TRACE_PROFILE || context.is_null() || statement.is_null() {
        return 0;
    }
    // SAFETY: trace_snapshot keeps this Mutex and all registered handles alive
    // until tracing is unregistered; SQLite supplies a live sqlite3_stmt here.
    unsafe {
        let stmt = statement.cast::<ffi::sqlite3_stmt>();
        let sql_ptr = ffi::sqlite3_expanded_sql(stmt);
        if sql_ptr.is_null() {
            return 0;
        }
        let sql = CStr::from_ptr(sql_ptr).to_string_lossy().into_owned();
        ffi::sqlite3_free(sql_ptr.cast());
        let record = json!({"sql": sql,
            "vm_steps": ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_VM_STEP, 0),
            "fullscan_steps": ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP, 0),
            "sort_operations": ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_SORT, 0)});
        if let Ok(mut records) = (*context.cast::<Mutex<Vec<JsonValue>>>()).lock() {
            records.push(record);
        }
    }
    0
}

pub(super) fn trace_snapshot(db: &Database, conn: &Connection, query: &str) -> Vec<JsonValue> {
    let records = Mutex::new(Vec::<JsonValue>::new());
    // Pool max_size is eight. Hold every handle while registering/unregistering.
    let handles = (0..8).map(|_| db.conn().unwrap()).collect::<Vec<_>>();
    for handle in &handles {
        // SAFETY: callback state outlives registration and no other caller uses
        // these test-owned databases. All callbacks are removed before return.
        unsafe {
            assert_eq!(
                ffi::sqlite3_trace_v2(
                    handle.handle(),
                    ffi::SQLITE_TRACE_PROFILE,
                    Some(profile_callback),
                    (&records as *const Mutex<Vec<JsonValue>>).cast_mut().cast()
                ),
                ffi::SQLITE_OK
            );
        }
    }
    drop(handles);
    let result = db.search_global_entries_snapshot(query, SEARCH_RESULT_LIMIT, 0);
    let handles = (0..8).map(|_| db.conn().unwrap()).collect::<Vec<_>>();
    for handle in &handles {
        // SAFETY: unregister before releasing callback state, including errors.
        unsafe {
            ffi::sqlite3_trace_v2(handle.handle(), 0, None, std::ptr::null_mut());
        }
    }
    drop(handles);
    result.unwrap();
    let mut rows = records.into_inner().unwrap();
    for row in &mut rows {
        let sql = row["sql"].as_str().unwrap();
        // Only top-level source/status/search SELECTs can be explained here.
        if sql.contains("global_entry_source_facts")
            || sql.contains("global_entries_fts.rank")
            || sql.contains("CASE WHEN")
            || sql.contains("active_volume_facts")
            || sql.contains("SELECT COUNT(*)\n             FROM global_entries entry")
        {
            row["query_plan"] = json!(explain_details(conn, sql, &[]));
        }
    }
    rows
}

fn time<T>(run: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let value = run();
    (value, start.elapsed().as_secs_f64() * 1000.0)
}

#[test]
#[ignore = "bounded A0/A1/A2/A3 covering-index qualification; one fixture per scale"]
fn global_search_covering_index_pair() {
    let entries = configured_entries();
    assert!([100_000, 1_000_000].contains(&entries));
    let mut context = benchmark_context(entries);
    context["source_tree"] = json!(env::var("ZEN_CANVAS_BENCHMARK_SOURCE_TREE").ok());
    context["build_profile"] = json!(if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    });
    let paths = (0..4).map(|_| test_db_path()).collect::<Vec<_>>();
    let _cleanup = paths
        .iter()
        .cloned()
        .map(BenchmarkCleanup)
        .collect::<Vec<_>>();
    let base = Database::open(&paths[0]).unwrap();
    base.upsert_global_volume(&test_volume()).unwrap();
    install(&base.conn().unwrap(), 0);
    let (expectations, generation_ms, population_ms, batches) =
        populate_synthetic_database(&base, entries);
    let inspect = configured_connection(&paths[0]).unwrap();
    assert_entry_triggers_enabled(&inspect);
    inspect
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    drop(inspect);
    drop(base);
    // Closed/checkpointed SQLite main file is the controlled-copy authority.
    for path in &paths[1..] {
        fs::copy(&paths[0], path).unwrap();
    }
    let dbs = paths
        .iter()
        .map(|p| Database::open(p).unwrap())
        .collect::<Vec<_>>();
    let conns = paths
        .iter()
        .map(|p| configured_connection(p).unwrap())
        .collect::<Vec<_>>();
    emit_record(&json!({"record_type":"covering_fixture", "context":context,
        "fixture_generations":1,"controlled_copies":3,"generation_ms":generation_ms,
        "population_ms":population_ms,"batch_ms":summarize_samples(&batches),
        "limit":80,"candidate_window":4096,"warmups":WARMUP_SAMPLES,"samples":WARM_SAMPLES,
        "historical_p95_target_ms":WARM_QUERY_P95_LIMIT_MS}));
    for (variant, conn) in conns.iter().enumerate() {
        let build_ms = install(conn, variant);
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let writes = write_samples(conn, entries);
        // dbstat exposes exact per-index B-tree bytes, including interior pages.
        let mut statement = conn
            .prepare("SELECT name, SUM(pgsize) FROM dbstat GROUP BY name ORDER BY name")
            .unwrap();
        let sizes = statement
            .query_map([], |r| {
                Ok(json!({"name":r.get::<_,String>(0)?,"bytes":r.get::<_,i64>(1)?}))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        emit_record(
            &json!({"record_type":"covering_index_cost", "context":context,"variant":variant,
            "index_ddl_ms":build_ms,"db":database_metrics(conn,&paths[variant]),"btree_sizes":sizes,
            "write_256_rows_rollback_ms":summarize_samples(&writes),
            "write_scope":"5 transaction-only 256-row INSERT samples with production FTS/count triggers; rolled back; not sustained ingestion"}),
        );
    }
    for (q, case) in QUERY_MATRIX.iter().copied().enumerate() {
        let oracle = dbs[0]
            .search_global_entries_snapshot(case.query, 80, 0)
            .unwrap();
        assert_eq!(
            oracle.results,
            assert_search_results(&conns[0], case, &expectations[q], 80, 0, entries)
        );
        let health = diagnostic_load_production_source_health(&conns[0]).unwrap();
        let mut samples = vec![vec![Vec::<f64>::new(); 5]; 4];
        for round in 0..WARMUP_SAMPLES + WARM_SAMPLES {
            for slot in 0..4 {
                let v = (slot + round) % 4;
                let db = &dbs[v];
                let conn = &conns[v];
                let (sql, a) =
                    time(|| search_global_entries_on_connection(conn, case.query, 80, 0).unwrap());
                let (pooled, b) = time(|| db.search_global_entries(case.query, 80, 0).unwrap());
                let (source, c) = time(|| diagnostic_load_production_source_health(conn).unwrap());
                let (status, d) = time(|| db.global_index_status().unwrap());
                let (snapshot, e) = time(|| {
                    db.search_global_entries_snapshot(case.query, 80, 0)
                        .unwrap()
                });
                assert_eq!(sql, oracle.results);
                assert_eq!(pooled, oracle.results);
                assert_eq!(source, health);
                assert_eq!(status, oracle.index_status);
                assert_snapshot(&oracle, &snapshot);
                if round >= WARMUP_SAMPLES {
                    for (stage, value) in [a, b, c, d, e].iter().enumerate() {
                        samples[v][stage].push(*value);
                    }
                    emit_record(&json!({"record_type":"covering_sample","context":context,
                        "query_class":case.class,"variant":v,"round":round-WARMUP_SAMPLES,
                        "sql_route_ms":a,"pooled_ms":b,"source_health_ms":c,"index_status_ms":d,"snapshot_ms":e}));
                }
            }
        }
        for v in 0..4 {
            for (stage, name) in [
                "sql_route",
                "pooled",
                "source_health",
                "index_status",
                "snapshot",
            ]
            .iter()
            .enumerate()
            {
                emit_record(
                    &json!({"record_type":"covering_summary","context":context,"query_class":case.class,
                    "variant":v,"stage":name,"latency_ms":summarize_samples(&samples[v][stage]),
                    "pooled_original_100ms_pass":if stage==1 {json!(summarize_samples(&samples[v][stage])["p95"].as_f64().unwrap()<100.0)} else {JsonValue::Null}}),
                );
            }
            for (limit, offset) in [(1, 0), (80, 40), (200, 4016), (80, 4096)] {
                assert_snapshot(
                    &dbs[0]
                        .search_global_entries_snapshot(case.query, limit, offset)
                        .unwrap(),
                    &dbs[v]
                        .search_global_entries_snapshot(case.query, limit, offset)
                        .unwrap(),
                );
            }
            for tier in exact_sql_1m_tier_order(case.query) {
                let sql = diagnostic_search_tier_sql(tier);
                let values = exact_sql_1m_tier_parameters(tier, case.query, 80);
                let expected = diagnostic_search_tier(&conns[0], tier, case.query, 80).unwrap();
                let mut times = Vec::new();
                let mut counters = JsonValue::Null;
                for round in 0..WARMUP_SAMPLES + WARM_SAMPLES {
                    let (rows, elapsed, work) = covering_index_sql_sample(&conns[v], &sql, &values);
                    assert_eq!(rows, expected);
                    if round >= WARMUP_SAMPLES {
                        times.push(elapsed);
                        counters = work;
                    }
                }
                let plan = explain_details(&conns[v], &sql, &values);
                if v & 1 != 0 && tier == "name_prefix" {
                    assert!(plan
                        .iter()
                        .any(|line| line
                            .contains("COVERING INDEX idx_global_entries_active_name_order")));
                }
                emit_record(
                    &json!({"record_type":"covering_sql_tier","context":context,"query_class":case.class,
                    "variant":v,"tier":tier,"sql":sql,"parameters":format!("{values:?}"),
                    "plan":plan,"counters":counters,"latency_ms":summarize_samples(&times)}),
                );
            }
            let trace = trace_snapshot(&dbs[v], &conns[v], case.query);
            if v & 2 != 0 {
                assert!(trace
                    .iter()
                    .any(|row| row["query_plan"]
                        .as_array()
                        .is_some_and(|plan| plan
                            .iter()
                            .any(|p| p.as_str().unwrap_or("").contains(
                                "COVERING INDEX idx_global_entries_active_source_facts"
                            )))));
            }
            emit_record(
                &json!({"record_type":"covering_snapshot_trace","context":context,
                "query_class":case.class,"variant":v,"statements":trace}),
            );
        }
        eprintln!(
            "covering_pair_progress rows={entries} query={} correctness=PASS",
            case.class
        );
    }
    emit_record(
        &json!({"record_type":"covering_terminal","context":context,"correctness_classes":12,
        "full_fields_bm25_pages_source_revision_status_equal":true,"status":"PASS"}),
    );
}

#[test]
fn covering_indexes_preserve_hostile_source_and_search_contracts() {
    use crate::global_index::repository::{
        load_global_search_source_health_candidate,
        GlobalSearchSourceHealthQueryCandidate as Candidate,
    };
    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).unwrap();
    db.upsert_global_volume(&test_volume()).unwrap();
    for (id, enabled) in [("disabled", false), ("empty", true)] {
        let mut volume = test_volume();
        volume.id = id.into();
        volume.stable_volume_id = id.into();
        volume.enabled = enabled;
        db.upsert_global_volume(&volume).unwrap();
    }
    for i in 0..100 {
        let mut entry = super::super::test_entry(
            &format!("C:\\Global\\probe-{i}.txt"),
            &format!("probe-{i}.txt"),
            false,
        );
        if i % 3 == 0 {
            entry.volume_id = "disabled".into();
        }
        db.upsert_global_entries_batch(&[entry]).unwrap();
    }
    let mut conn = configured_connection(&path).unwrap();
    conn.execute_batch(
        "UPDATE global_entries SET id=NULL WHERE rowid=1;
        UPDATE global_entries SET is_stale=1 WHERE rowid%5=0;
        UPDATE global_entries SET modified_at_fs=20;",
    )
    .unwrap();
    install(&conn, 0);
    let facts =
        load_global_search_source_health_candidate(&conn, Candidate::NarrowAggregate).unwrap();
    // NULL is in the disabled source; enabled search keeps normal nullable-ID behavior.
    let oracle = db.search_global_entries_snapshot("probe", 80, 0).unwrap();
    for v in 0..4 {
        install(&conn, v);
        assert_eq!(
            load_global_search_source_health_candidate(&conn, Candidate::NarrowAggregate).unwrap(),
            facts
        );
        assert_snapshot(
            &oracle,
            &db.search_global_entries_snapshot("probe", 80, 0).unwrap(),
        );
        let tx = conn.transaction().unwrap();
        let pinned = diagnostic_load_production_source_health(&tx).unwrap();
        db.conn()
            .unwrap()
            .execute(
                "UPDATE global_volumes SET enabled=1-enabled WHERE id='empty'",
                [],
            )
            .unwrap();
        assert_eq!(
            diagnostic_load_production_source_health(&tx).unwrap(),
            pinned
        );
        tx.commit().unwrap();
        db.conn()
            .unwrap()
            .execute(
                "UPDATE global_volumes SET enabled=1-enabled WHERE id='empty'",
                [],
            )
            .unwrap();
    }
    // Compare every source/status projection with COUNT(id) nullable semantics,
    // disabled sources, all-disabled and empty volumes retained in the same DB.
    for status in [
        "discovered",
        "indexing",
        "syncing",
        "rebuild_required",
        "ready",
        "spotlight_not_indexed",
        "permission_required",
        "spotlight_unavailable",
        "spotlight_external_not_indexed",
        "fsevents_unavailable",
        "unavailable",
        "error",
        "paused",
    ] {
        conn.execute("UPDATE global_volumes SET index_status=?1, last_error='fixture error' WHERE id='gv_test'",[status]).unwrap();
        for enabled in [0, 1] {
            conn.execute("UPDATE global_volumes SET enabled=?1", [enabled])
                .unwrap();
            install(&conn, 0);
            let original = db
                .search_global_entries_snapshot("zzznomatchtoken", 80, 0)
                .unwrap();
            let original_facts =
                load_global_search_source_health_candidate(&conn, Candidate::NarrowAggregate)
                    .unwrap();
            for v in 1..4 {
                install(&conn, v);
                assert_snapshot(
                    &original,
                    &db.search_global_entries_snapshot("zzznomatchtoken", 80, 0)
                        .unwrap(),
                );
                assert_eq!(
                    load_global_search_source_health_candidate(&conn, Candidate::NarrowAggregate)
                        .unwrap(),
                    original_facts
                );
            }
        }
    }
}

fn write_samples(conn: &Connection, entries: u64) -> Vec<f64> {
    let rows = (0..256)
        .map(|offset| super::generated_entry(entries + 10_000 + offset))
        .collect::<Vec<_>>();
    let mut timings = Vec::with_capacity(5);
    for _ in 0..5 {
        let transaction = conn
            .unchecked_transaction()
            .expect("begin rollback-only index maintenance sample");
        let mut statement = transaction
            .prepare(INSERT_SYNTHETIC_ENTRY)
            .expect("prepare rollback-only index maintenance insert");
        let started = Instant::now();
        for entry in &rows {
            statement
                .execute(params![
                    entry.id,
                    "gv_test",
                    entry.platform_file_id,
                    entry.parent_platform_file_id,
                    entry.name,
                    entry.name_normalized,
                    entry.path,
                    entry.path_normalized,
                    entry.extension,
                    0_i64,
                    entry.size,
                    entry.created_at_fs,
                    entry.modified_at_fs,
                    entry.file_attributes,
                    i64::from(entry.is_hidden),
                    i64::from(entry.is_system),
                    0_i64,
                    "issue352_index_write_cost",
                    1_700_600_000_i64 + entry.index as i64,
                ])
                .expect("insert rollback-only index maintenance row");
        }
        timings.push(started.elapsed().as_secs_f64() * 1_000.0);
        drop(statement);
        transaction
            .rollback()
            .expect("rollback index maintenance sample rows");
    }
    timings
}
