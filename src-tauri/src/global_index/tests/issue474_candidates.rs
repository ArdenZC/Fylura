//! Owner-authorized test-only 100k/200k algorithm probe; never a production gate.
use super::query_cost_diagnostic::{covering_index_sql_sample, explain_details};
use super::*;
use crate::global_index::search::{
    diagnostic_search_tier, diagnostic_search_tier_sql, extension_recency_probe as probe,
};
use rusqlite::{ffi, StatementStatus};
use std::ffi::{c_void, CStr};
use std::sync::Mutex;

fn timed<T>(run: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let result = run();
    (result, start.elapsed().as_secs_f64() * 1000.)
}
fn git(args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}
unsafe extern "C" fn trace(
    mask: u32,
    context: *mut c_void,
    stmt: *mut c_void,
    _: *mut c_void,
) -> i32 {
    if mask != ffi::SQLITE_TRACE_PROFILE || context.is_null() || stmt.is_null() {
        return 0;
    }
    // SAFETY: trace_run owns the mutex and connection throughout registration.
    unsafe {
        let stmt = stmt.cast::<ffi::sqlite3_stmt>();
        let ptr = ffi::sqlite3_sql(stmt);
        if ptr.is_null() {
            return 0;
        }
        let sql = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        // Exclude nested FTS internals and transaction control. These are
        // top-level extension SELECTs; summing them includes failed probe work.
        if sql.trim_start().starts_with("SELECT") && sql.contains("global_entries ge")
            || sql == probe::RAW_SQL
        {
            (*context.cast::<Mutex<Vec<JsonValue>>>()).lock().unwrap().push(json!({
                "sql":sql, "vm_steps":ffi::sqlite3_stmt_status(stmt,ffi::SQLITE_STMTSTATUS_VM_STEP,0),
                "sort":ffi::sqlite3_stmt_status(stmt,ffi::SQLITE_STMTSTATUS_SORT,0),
                "fullscan":ffi::sqlite3_stmt_status(stmt,ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP,0)
            }));
        }
    }
    0
}
fn trace_run<T>(conn: &Connection, run: impl FnOnce() -> T) -> (T, Vec<JsonValue>) {
    struct Unregister<'a>(&'a Connection);
    impl Drop for Unregister<'_> {
        fn drop(&mut self) {
            unsafe {
                ffi::sqlite3_trace_v2(self.0.handle(), 0, None, std::ptr::null_mut());
            }
        }
    }
    let records = Mutex::new(Vec::new());
    unsafe {
        assert_eq!(
            ffi::sqlite3_trace_v2(
                conn.handle(),
                ffi::SQLITE_TRACE_PROFILE,
                Some(trace),
                (&records as *const Mutex<Vec<JsonValue>>).cast_mut().cast()
            ),
            ffi::SQLITE_OK
        );
    }
    let guard = Unregister(conn);
    let result = run();
    drop(guard);
    (result, records.into_inner().unwrap())
}
fn schema(conn: &Connection) -> Vec<(String, String)> {
    conn.prepare("SELECT name, coalesce(sql,'') FROM sqlite_schema WHERE type IN ('table','index','trigger') ORDER BY name").unwrap()
        .query_map([], |r| Ok((r.get(0)?,r.get(1)?))).unwrap().collect::<Result<_,_>>().unwrap()
}
fn sql_tier(
    conn: &Connection,
    mode: u8,
    query: &str,
    limit: u32,
    offset: u32,
) -> Result<Vec<crate::global_index::models::GlobalSearchResult>, crate::db::DbError> {
    if mode == 0 {
        diagnostic_search_tier(conn, "extension_prefix", query, limit)
    } else {
        probe::with_mode(mode, || {
            probe::run(conn, query, limit, offset == 0 && limit <= 80)
        })
    }
}
fn pair(
    db: &Database,
    conn: &Connection,
    context: &JsonValue,
    case: &str,
    query: &str,
    limit: u32,
    offset: u32,
) {
    let tier_limit = limit.saturating_add(offset).min(4096);
    let expected = sql_tier(conn, 0, query, tier_limit, offset).unwrap();
    let pooled = db.search_global_entries(query, limit, offset).unwrap();
    let mut sql_samples = [vec![], vec![], vec![]];
    let mut pool_samples = [vec![], vec![], vec![]];
    let mut fallback = 0;
    for iteration in 0..35 {
        // Rotate all three variants so the fixture/cache/environment is paired.
        for n in 0..3 {
            let mode = ((n + iteration) % 3) as u8;
            let (actual, elapsed) =
                timed(|| sql_tier(conn, mode, query, tier_limit, offset).unwrap());
            assert_eq!(actual, expected, "SQL {case} mode {mode}");
            if mode == 2 && probe::fell_back() {
                fallback += 1;
            }
            let (actual, pooled_ms) = timed(|| {
                probe::with_mode(mode, || {
                    db.search_global_entries(query, limit, offset).unwrap()
                })
            });
            assert_eq!(actual, pooled, "pooled {case} mode {mode}");
            if iteration >= 5 {
                sql_samples[mode as usize].push(elapsed);
                pool_samples[mode as usize].push(pooled_ms);
            }
        }
    }
    let counters = (0..3).map(|mode| {
        let (_, records) = trace_run(conn, || sql_tier(conn,mode,query,tier_limit,offset).unwrap());
        let plans = records.iter().map(|r| json!({"sql":r["sql"], "plan":explain_details(conn,r["sql"].as_str().unwrap(),
            if r["sql"] == probe::RAW_SQL { vec![] } else { vec![SqlValue::Text(query.into()),SqlValue::Text(format!("{}*",benchmark_escape_glob(query))),SqlValue::Integer(tier_limit as i64)] }.as_slice())})).collect::<Vec<_>>();
        json!({"mode":mode,"vm_steps":records.iter().map(|r|r["vm_steps"].as_i64().unwrap()).sum::<i64>(),
            "sort":records.iter().map(|r|r["sort"].as_i64().unwrap()).sum::<i64>(),"statements":records,"plans":plans})
    }).collect::<Vec<_>>();
    let snapshot = db
        .search_global_entries_snapshot(query, limit, offset)
        .unwrap();
    for mode in 1..3 {
        let actual = probe::with_mode(mode, || {
            db.search_global_entries_snapshot(query, limit, offset)
                .unwrap()
        });
        assert_eq!(actual.results, snapshot.results);
        assert_eq!(actual.source_health, snapshot.source_health);
        assert_eq!(actual.source_revision, snapshot.source_revision);
        assert_eq!(actual.index_status, snapshot.index_status);
    }
    emit_record(
        &json!({"record_type":"extension_pair","context":context,"case":case,"query":query,"limit":limit,"offset":offset,"sql_tier_limit":tier_limit,
        "variants":["production","unlimited_recency","bounded_1600_fallback"],
        "warmups":5,"samples":30,"sql_tier_ms":sql_samples.iter().map(|s|summarize_samples(s)).collect::<Vec<_>>(),
        "pooled_ms":pool_samples.iter().map(|s|summarize_samples(s)).collect::<Vec<_>>(),
        "sql_tier_raw_ms":sql_samples,"pooled_raw_ms":pool_samples,"bounded_sql_fallbacks_of_35":fallback,
        "all_samples_entire_results_equal":true,"snapshot_entire_objects_equal":true,
        "expected_sql_results":expected,"expected_pooled_results":pooled,"counters":counters}),
    );
}
fn fts_stages(conn: &Connection, context: &JsonValue, query: &str) {
    let phrase = format!("\"{query}\"");
    let production = diagnostic_search_tier_sql("fts");
    let expected = diagnostic_search_tier(conn, "fts", query, 80).unwrap();
    let join = "FROM global_entries_fts CROSS JOIN global_entries ge CROSS JOIN global_volumes gv WHERE global_entries_fts MATCH ?1 AND ge.rowid=global_entries_fts.rowid AND gv.id=ge.volume_id AND gv.enabled=1 AND ge.is_stale=0";
    // Diagnostic drains, not candidate result algorithms. No LIMIT before
    // filters, no rank frontier, no generic CTE rewrite. Stage times overlap;
    // do not subtract percentiles or claim these are additive CPU costs.
    let stages = [
        ("match_rowids", "SELECT rowid FROM global_entries_fts WHERE global_entries_fts MATCH ?1".into()),
        ("match_bm25_8_2_1", "SELECT rowid,bm25(global_entries_fts,8.0,2.0,1.0) FROM global_entries_fts WHERE global_entries_fts MATCH ?1".into()),
        ("match_native_rank_8_2_1", "SELECT rowid,rank FROM global_entries_fts WHERE global_entries_fts MATCH ?1 AND rank MATCH 'bm25(8.0, 2.0, 1.0)'".into()),
        ("volume_stale_association",format!("SELECT ge.rowid {join}")),
        ("association_and_bm25",format!("SELECT ge.rowid,bm25(global_entries_fts,8.0,2.0,1.0) {join}")),
        ("stable_sort_80",format!("SELECT ge.id,bm25(global_entries_fts,8.0,2.0,1.0) AS score {join} ORDER BY score ASC,ge.modified_at_fs DESC,ge.id ASC LIMIT 80")),
    ];
    let mut samples = vec![Vec::new(); stages.len() + 1];
    let mut last = Vec::new();
    for iteration in 0..35 {
        for n in 0..=stages.len() {
            let i = (n + iteration) % (stages.len() + 1);
            if i == stages.len() {
                let (rows, elapsed, _) = covering_index_sql_sample(
                    conn,
                    &production,
                    &[SqlValue::Text(phrase.clone()), SqlValue::Integer(80)],
                );
                assert_eq!(rows, expected);
                if iteration >= 5 {
                    samples[i].push(elapsed);
                }
            } else {
                let (label, sql) = &stages[i];
                let mut stmt = conn.prepare(sql).unwrap();
                let (count, elapsed) = timed(|| {
                    let columns = stmt.column_count();
                    let mut rows = stmt.query([&phrase]).unwrap();
                    let mut count = 0;
                    while let Some(row) = rows.next().unwrap() {
                        for c in 0..columns {
                            let _: SqlValue = row.get(c).unwrap();
                        }
                        count += 1;
                    }
                    count
                });
                if iteration >= 5 {
                    samples[i].push(elapsed);
                }
                if iteration == 34 {
                    last.push(json!({"stage":label,"sql":sql,"rows":count,"vm_steps":stmt.get_status(StatementStatus::VmStep),"sort":stmt.get_status(StatementStatus::Sort),"plan":explain_details(conn,sql,&[SqlValue::Text(phrase.clone())])}));
                }
            }
        }
    }
    let (rows, _, counts) = covering_index_sql_sample(
        conn,
        &production,
        &[SqlValue::Text(phrase.clone()), SqlValue::Integer(80)],
    );
    assert_eq!(rows, expected);
    // Independently check native rank has the actual BM25 values on every hit.
    let mut stmt = conn.prepare("SELECT bm25(global_entries_fts,8.0,2.0,1.0),rank FROM global_entries_fts WHERE global_entries_fts MATCH ?1 AND rank MATCH 'bm25(8.0, 2.0, 1.0)'").unwrap();
    let mut hits = stmt.query([&phrase]).unwrap();
    while let Some(row) = hits.next().unwrap() {
        assert_eq!(row.get::<_, f64>(0).unwrap(), row.get::<_, f64>(1).unwrap());
    }
    let sorted = conn
        .prepare(&stages[5].1)
        .unwrap()
        .query_map([&phrase], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        sorted,
        expected
            .iter()
            .map(|r| (r.id.clone(), r.rank))
            .collect::<Vec<_>>()
    );
    for row in &mut last {
        let index = stages
            .iter()
            .position(|(name, _)| Some(*name) == row["stage"].as_str())
            .unwrap();
        row["latency_ms"] = summarize_samples(&samples[index]);
        row["raw_ms"] = json!(samples[index]);
    }
    emit_record(
        &json!({"record_type":"fts_cost_decomposition","context":context,"query":query,"warmups":5,"samples":30,
        "stages":last,"latency_ms":samples.iter().map(|s|summarize_samples(s)).collect::<Vec<_>>(),"raw_ms":samples,
        "sample_order":"stage vector order plus actual production last; rotated each iteration",
        "actual_production_sql":production,"actual_production_plan":explain_details(conn,&production,&[SqlValue::Text(phrase),SqlValue::Integer(80)]),
        "production_counters":counts,"production_all_fields_and_rank_equal":true,"production_results":expected,
        "native_rank_bm25_all_hits_equal":true,"stage_times_are_overlapping_not_additive":true}),
    );
}

#[test]
#[ignore = "Owner #474 test-only 100k/200k paired probe; never 1M or Full Validation"]
fn issue474_test_only_candidates() {
    let entries = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES")
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert!(
        matches!(entries, 100_000 | 200_000),
        "only authorized 100k/200k"
    );
    if let Ok(path) = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT") {
        if let Some(parent) = Path::new(&path).parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, "").unwrap();
    }
    let context = benchmark_context(entries);
    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).unwrap();
    db.upsert_global_volume(&test_volume()).unwrap();
    let (_, generation, population, _) = populate_synthetic_database(&db, entries);
    let conn = configured_connection(&path).unwrap();
    let before = schema(&conn);
    emit_record(
        &json!({"record_type":"candidate_environment","context":context,"head":git(&["rev-parse","HEAD"]),"tree":git(&["rev-parse","HEAD^{tree}"]),
        "git_diff":git(&["diff","--stat"]),"sqlite_version":rusqlite::version(),"rustc":std::process::Command::new("rustc").arg("-vV").output().map(|o|String::from_utf8_lossy(&o.stdout).into_owned()).unwrap(),
        "profile":"cargo test default dev/test; no desktop-runtime features; bundled rusqlite", "debug_assertions":cfg!(debug_assertions),
        "generation_ms":generation,"population_ms":population,"database":database_metrics(&conn,&path),"production_plans":capture_query_plans(&conn,&context),
        "compile_options":conn.prepare("PRAGMA compile_options").unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap()}),
    );
    for query in QUERY_MATRIX {
        for offset in [0, 80, 4016, 4096] {
            let expected = db.search_global_entries(query.query, 80, offset).unwrap();
            for mode in 1..3 {
                let candidate = probe::with_mode(mode, || {
                    db.search_global_entries(query.query, 80, offset).unwrap()
                });
                assert_eq!(
                    candidate, expected,
                    "12-class page {} offset {offset} mode {mode}",
                    query.class
                );
            }
            emit_record(
                &json!({"record_type":"twelve_class_pagination_equality","context":context,"class":query.class,"query":query.query,"offset":offset,"entire_results_equal":true,"production_results":expected}),
            );
        }
    }
    for (case, query, limit, offset) in [
        ("high_jp", "jp", 80, 0),
        ("zero", "zz_no_extension", 80, 0),
        ("sparse", "jso", 80, 0),
        ("deep_80", "jp", 80, 80),
        ("window_4096", "jp", 80, 4016),
        ("beyond_window", "jp", 80, 4096),
    ] {
        pair(&db, &conn, &context, case, query, limit, offset);
    }
    for q in ["report", "invoice"] {
        fts_stages(&conn, &context, q);
    }
    query_cost_diagnostic::insert_diagnostic_entry(
        &conn,
        "probe-managed",
        "gv_test",
        "managed-photo.jpg",
        r"C:\Probe\managed-photo.jpg",
        1_800_000_000,
    );
    query_cost_diagnostic::insert_diagnostic_entry(
        &conn,
        "probe-cross",
        "gv_test",
        "jp-crossover.jpg",
        r"C:\Probe\jp-crossover.jpg",
        1_800_000_000,
    );
    conn.execute_batch("INSERT INTO managed_scopes(id,path,global_entry_id,enabled,allow_local_ai,allow_cloud_ai,created_at,updated_at) VALUES('probe-scope','probe','probe-managed',1,1,0,1,1); INSERT INTO managed_entries(id,global_entry_id,managed_scope_id,enabled,created_at,updated_at) VALUES('probe-marker','probe-managed','probe-scope',1,1,1);").unwrap();
    pair(
        &db,
        &conn,
        &context,
        "managed_and_cross_tier_overlap",
        "jp",
        80,
        0,
    );
    // Same fixture, committed adversarial states visible through real pool.
    let mut other = test_volume();
    other.id = "gv_other".into();
    other.stable_volume_id = "probe-other".into();
    db.upsert_global_volume(&other).unwrap();
    let mut disabled = test_volume();
    disabled.id = "gv_disabled_probe".into();
    disabled.stable_volume_id = "probe-disabled".into();
    disabled.enabled = false;
    db.upsert_global_volume(&disabled).unwrap();
    conn.execute_batch("UPDATE global_entries SET volume_id='gv_other' WHERE rowid%3=0; UPDATE global_entries SET volume_id='gv_disabled_probe' WHERE rowid%7=0; UPDATE global_entries SET is_stale=1 WHERE rowid%5=0;").unwrap();
    pair(
        &db,
        &conn,
        &context,
        "stale_disabled_cross_volume",
        "jp",
        80,
        0,
    );
    conn.execute_batch("UPDATE global_entries SET extension='jp' WHERE rowid%11=0;")
        .unwrap();
    pair(&db, &conn, &context, "exclude_exact_extension", "jp", 80, 0);
    conn.execute_batch("UPDATE global_entries SET modified_at_fs=2000000000 WHERE rowid IN (SELECT rowid FROM global_entries ORDER BY modified_at_fs DESC LIMIT 1800);").unwrap();
    pair(&db, &conn, &context, "mtime_tie_over_budget", "jp", 80, 0);
    conn.execute_batch("UPDATE global_entries SET modified_at_fs=2100000000 WHERE rowid IN (SELECT rowid FROM global_entries WHERE is_stale=0 ORDER BY rowid LIMIT 1200);").unwrap();
    pair(
        &db,
        &conn,
        &context,
        "mtime_complete_tie_group",
        "jp",
        80,
        0,
    );
    // Nullable IDs and malformed SQLite types must route to the exact SQL.
    // Preserve errors as well as successful row vectors.
    for (case,sql) in [("null_id","UPDATE global_entries SET id=NULL,extension='jpg',modified_at_fs=2200000000 WHERE rowid=1"),
        ("abnormal_mtime","UPDATE global_entries SET id='probe_repaired_id',modified_at_fs='invalid-time' WHERE rowid=1")] {
        conn.execute_batch(sql).unwrap();
        for (limit,offset) in [(80,0),(4096,0)] {
            let baseline=sql_tier(&conn,0,"jp",limit,offset).map_err(|e|e.to_string());
            let candidate=sql_tier(&conn,2,"jp",limit,offset).map_err(|e|e.to_string());
            assert_eq!(candidate,baseline); assert!(probe::fell_back());
            let baseline=db.search_global_entries("jp",80,offset).map_err(|e|e.to_string());
            let candidate=probe::with_mode(2,||db.search_global_entries("jp",80,offset)).map_err(|e|e.to_string());
            assert_eq!(candidate,baseline);
            emit_record(&json!({"record_type":"abnormal_equality","case":case,"limit":limit,"offset":offset,"fallback":true,"entire_result_or_error_equal":true,"production":baseline}));
        }
    }
    assert_eq!(schema(&conn), before);
    emit_record(
        &json!({"record_type":"candidate_complete","entries":entries,"schema_indexes_triggers_unchanged":true,"production_sql_unchanged":true,"formal_30_samples_100ms_gate_unchanged":true,"one_million_run":false}),
    );
}

#[test]
fn bounded_probe_snapshot_fallback_and_window_contract() {
    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).unwrap();
    db.upsert_global_volume(&test_volume()).unwrap();
    populate_synthetic_database(&db, 3000);
    let conn = configured_connection(&path).unwrap();
    for (limit, offset) in [(1, 0), (80, 0), (80, 1), (200, 0), (80, 4016), (80, 4096)] {
        let expected = db.search_global_entries("jp", limit, offset).unwrap();
        let actual = probe::with_mode(2, || db.search_global_entries("jp", limit, offset).unwrap());
        assert_eq!(actual, expected);
    }
    for tier_limit in [80, 160, 4096] {
        let expected = sql_tier(&conn, 0, "jp", tier_limit, 0).unwrap();
        assert_eq!(sql_tier(&conn, 2, "jp", tier_limit, 0).unwrap(), expected);
        if tier_limit > 160 {
            assert!(probe::fell_back());
        }
    }
    // Zero-hit must run fallback after the raw prefix. A writer inserts a
    // matching extension after raw rows were drained; the internal transaction
    // must retain the pre-write zero result, then the next read sees the write.
    let writer = configured_connection(&path).unwrap();
    probe::after_raw(move || {
        writer
            .execute(
                "UPDATE global_entries SET extension='zzprobe' WHERE rowid=1",
                [],
            )
            .unwrap();
    });
    let rows = sql_tier(&conn, 2, "zzp", 80, 0).unwrap();
    assert!(rows.is_empty());
    assert!(probe::fell_back());
    assert_eq!(sql_tier(&conn, 0, "zzp", 80, 0).unwrap().len(), 1);
    // Existing Repository/read transaction is reused rather than nested.
    conn.execute_batch("BEGIN DEFERRED").unwrap();
    let before = sql_tier(&conn, 0, "zzp", 80, 0).unwrap();
    let writer = configured_connection(&path).unwrap();
    writer
        .execute("UPDATE global_entries SET is_stale=1 WHERE rowid=1", [])
        .unwrap();
    assert_eq!(sql_tier(&conn, 2, "zzp", 80, 0).unwrap(), before);
    conn.execute_batch("ROLLBACK").unwrap();
    assert!(sql_tier(&conn, 0, "zzp", 80, 0).unwrap().is_empty());
}
