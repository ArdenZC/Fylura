//! #474 test-only alternate retrieval contract: recent-first substring matching.
//! NEVER activate from production. This deliberately does not preserve BM25 order.
use super::*;
use std::collections::HashSet;

// The existing modified-time index drives a bounded recency-first query.
// The ASCII substring predicate is deliberately not claimed to be equivalent
// to an arbitrary FTS5 trigram phrase, even if these two fixture terms agree.
const RECENT_FIRST_SQL: &str = r#"
    SELECT ge.id, ge.modified_at_fs
    FROM global_entries ge INDEXED BY idx_global_entries_modified_at
    CROSS JOIN global_volumes gv
    WHERE gv.id=ge.volume_id
      AND gv.enabled=1 AND ge.is_stale=0
      AND (instr(ge.name_normalized, ?1)>0
           OR instr(ge.path_normalized, ?1)>0
           OR instr(lower(ge.extension), ?1)>0)
    ORDER BY ge.modified_at_fs DESC, ge.id ASC
    LIMIT ?2
"#;
const RECENT_SET_SQL: &str = r#"
    SELECT ge.id
    FROM global_entries ge INDEXED BY idx_global_entries_modified_at
    CROSS JOIN global_volumes gv
    WHERE gv.id=ge.volume_id
      AND gv.enabled=1 AND ge.is_stale=0
      AND (instr(ge.name_normalized, ?1)>0
           OR instr(ge.path_normalized, ?1)>0
           OR instr(lower(ge.extension), ?1)>0)
"#;
const FTS_SET_SQL: &str = r#"
    SELECT ge.id FROM global_entries_fts
    CROSS JOIN global_entries ge
    CROSS JOIN global_volumes gv
    WHERE global_entries_fts MATCH ?1
      AND ge.rowid=global_entries_fts.rowid
      AND gv.id=ge.volume_id AND gv.enabled=1 AND ge.is_stale=0
"#;

fn recent_first(conn: &Connection, query: &str, limit: u32) -> Vec<(String, Option<i64>)> {
    let mut stmt = conn
        .prepare(RECENT_FIRST_SQL)
        .expect("prepare recent-first SQL");
    stmt.query_map(params![query, limit], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?))
    })
    .expect("query recent-first rows")
    .collect::<rusqlite::Result<Vec<_>>>()
    .expect("collect recent-first rows")
}

fn all_ids(conn: &Connection, sql: &str, query: &str) -> HashSet<String> {
    let arg = if sql == FTS_SET_SQL {
        format!("\"{}\"", query.replace('"', "\"\""))
    } else {
        query.to_string()
    };
    let mut stmt = conn.prepare(sql).expect("prepare full membership probe");
    stmt.query_map([arg], |row| row.get::<_, String>(0))
        .expect("query membership probe")
        .collect::<rusqlite::Result<HashSet<_>>>()
        .expect("collect full membership probe")
}

fn sql_plan(conn: &Connection, query: &str) -> Vec<String> {
    conn.prepare(&format!("EXPLAIN QUERY PLAN {RECENT_FIRST_SQL}"))
        .expect("prepare recent-first plan")
        .query_map(params![query, SEARCH_RESULT_LIMIT], |row| {
            row.get::<_, String>(3)
        })
        .expect("execute recent-first plan")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect recent-first plan")
}

#[test]
#[ignore = "100k-only experimental recency-first FTS alternative; changes BM25 ordering"]
fn experimental_recent_first_substring_vs_exact_bm25() {
    let entries = configured_entries();
    assert_eq!(
        entries, 100_000,
        "manual alternate-ranking preflight is fixed to 100k"
    );
    let context = benchmark_context(entries);
    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open baseline synthetic database");
    db.upsert_global_volume(&test_volume())
        .expect("insert synthetic benchmark volume");
    let (expectations, generation_ms, population_ms, _) = populate_synthetic_database(&db, entries);
    let conn = configured_connection(&path).expect("open alternate-ranking read connection");
    assert_entry_triggers_enabled(&conn);

    emit_record(&json!({
        "schema_version": 1,
        "record_type": "fts_recency_fixture",
        "context": context,
        "generation_ms": generation_ms,
        "population_ms": population_ms,
        "scope": "test-only ranking-contract comparison; not production Search; no index or schema changes",
        "release_build": !cfg!(debug_assertions)
    }));

    for query in ["report", "invoice"] {
        let query_class = if query == "report" {
            "fts_substring_report"
        } else {
            "fts_substring_invoice"
        };
        let baseline = db
            .search_global_entries(query, SEARCH_RESULT_LIMIT, 0)
            .expect("search exact baseline through pooled production entrypoint");
        let expected_index = if query == "report" { 3 } else { 4 };
        let expected_ids =
            expectations[expected_index].expected_page(0, SEARCH_RESULT_LIMIT as usize);
        assert_eq!(
            baseline
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            expected_ids,
            "exact BM25 production baseline must meet historical result oracle"
        );

        // Membership may differ for a general phrase, normalization or mixed
        // Unicode case. Record the full set difference; don't hide it by
        // comparing only the first 80 hits.
        let fts_set = all_ids(&conn, FTS_SET_SQL, query);
        let recent_set = all_ids(&conn, RECENT_SET_SQL, query);
        let only_fts = fts_set.difference(&recent_set).count();
        let only_recent = recent_set.difference(&fts_set).count();
        let same_membership = only_fts == 0 && only_recent == 0;

        let mut timings = Vec::<f64>::with_capacity(30);
        let mut page = Vec::new();
        for iteration in 0..35 {
            let started = Instant::now();
            let selected = recent_first(&conn, query, SEARCH_RESULT_LIMIT);
            let latency_ms = started.elapsed().as_secs_f64() * 1_000.0;
            if iteration >= 5 {
                timings.push(latency_ms);
            }
            if iteration == 0 {
                page = selected;
            } else {
                assert_eq!(page, selected, "recency-first query is nondeterministic");
            }
        }
        let selected_ids = page.iter().map(|row| row.0.as_str()).collect::<Vec<_>>();
        let selected_set = selected_ids.iter().copied().collect::<HashSet<_>>();
        assert_eq!(
            selected_set.len(),
            page.len(),
            "duplicate recency candidates"
        );
        assert!(selected_ids.iter().all(|id| recent_set.contains(*id)));
        let bm25_ids = baseline
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>();
        let bm25_set = bm25_ids.iter().copied().collect::<HashSet<_>>();
        let overlap = selected_set.intersection(&bm25_set).count();
        let same_top_80_order = selected_ids == bm25_ids;
        let same_top_80_set = selected_set == bm25_set;
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "fts_recency_alternative",
            "context": context,
            "query_class": query_class,
            "query": query,
            "strategy": "global_entries.modified_at_fs_index_scan + ASCII normalized substring; recency-first",
            "semantic_contract": "EXPERIMENTAL_BM25_ORDER_CHANGE_NOT_PRODUCT_EQUIVALENT",
            "production_bm25_full_order_preserved": false,
            "fts_matched_count": fts_set.len(),
            "recent_substring_matched_count": recent_set.len(),
            "same_eligible_membership_on_fixture": same_membership,
            "fts_only_rows": only_fts,
            "recency_only_rows": only_recent,
            "production_top80_count": baseline.len(),
            "recent_top80_count": page.len(),
            "top80_overlap": overlap,
            "top80_same_set": same_top_80_set,
            "top80_same_order": same_top_80_order,
            "top80_production_ids": bm25_ids,
            "top80_recency_ids": selected_ids,
            "recency_latency_ms": summarize_samples(&timings),
            "recency_explain": sql_plan(&conn, query),
            "rows": entries,
            "sample_policy": "5 warmups + 30 measured, connection reused, excludes fixture generation and full membership oracles",
            "ranking_warning": "do not ship as a drop-in BM25 optimization; requires explicit UX/product decision and Unicode/phrase correctness contract"
        }));
    }
    profile_sparse_and_zero_tail(&conn, &db, &context);
}

/// A controlled counterexample. A newer path hit need not outrank an older,
/// highly relevant filename hit in FTS5's weighted BM25 ordering.
#[test]
fn recent_first_ranking_diverges_from_weighted_bm25() {
    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open small rank-order fixture");
    db.upsert_global_volume(&test_volume())
        .expect("insert enabled volume");
    let mut disabled_volume = test_volume();
    disabled_volume.id = "gv_disabled".to_string();
    disabled_volume.stable_volume_id = "disabled-volume".to_string();
    disabled_volume.enabled = false;
    db.upsert_global_volume(&disabled_volume)
        .expect("insert disabled volume");

    let mut old = test_entry(
        r"C:\Global\Old\archived report report report report.txt",
        "archived report report report report.txt",
        false,
    );
    old.modified_at_fs = Some(10);
    let mut recent = test_entry(
        // Deliberately lowercase: normalize_path() only folds paths on Windows.
        r"C:\Global\reports\recent_notes.txt",
        "recent_notes.txt",
        false,
    );
    recent.modified_at_fs = Some(200);
    let mut stale = test_entry(
        r"C:\Global\Report\stale_report.txt",
        "stale_report.txt",
        false,
    );
    stale.modified_at_fs = Some(300);
    let mut disabled = test_entry(
        r"C:\Disabled\Report\hidden_report.txt",
        "hidden_report.txt",
        false,
    );
    disabled.volume_id = disabled_volume.id;
    disabled.modified_at_fs = Some(400);

    let old_id = old.entry_id();
    let recent_id = recent.entry_id();
    let stale_id = stale.entry_id();
    let disabled_id = disabled.entry_id();
    db.upsert_global_entries_batch(&[old, recent, stale, disabled])
        .expect("insert adversarial FTS corpus");
    let conn = db.conn().expect("get adversarial connection");
    conn.execute(
        "UPDATE global_entries SET is_stale=1 WHERE id=?1",
        [stale_id.as_str()],
    )
    .expect("retire newer hit");

    let recent_ids = recent_first(&conn, "report", 80)
        .into_iter()
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    let exact_ids = conn
        .prepare(
            "SELECT ge.id FROM global_entries_fts
             CROSS JOIN global_entries ge
             CROSS JOIN global_volumes gv
             WHERE global_entries_fts MATCH ?1
               AND ge.rowid=global_entries_fts.rowid
               AND gv.id=ge.volume_id AND gv.enabled=1 AND ge.is_stale=0
             ORDER BY bm25(global_entries_fts,8.0,2.0,1.0) ASC,
                      ge.modified_at_fs DESC,ge.id ASC LIMIT 80",
        )
        .expect("prepare weighted BM25 ranking")
        .query_map(["\"report\""], |row| row.get::<_, String>(0))
        .expect("query weighted BM25 ranking")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect weighted BM25 ids");
    assert_eq!(recent_ids, vec![recent_id.clone(), old_id.clone()]);
    assert_eq!(exact_ids, vec![old_id, recent_id]);
    assert!(!recent_ids.contains(&stale_id));
    assert!(!recent_ids.contains(&disabled_id));
}

/// The chronological index scan can traverse all 100k entries on an empty
/// term or a very old rare hit. No p95 result here qualifies production.
fn profile_sparse_and_zero_tail(conn: &Connection, db: &Database, context: &JsonValue) {
    for (case, query) in [
        ("no_hit", "zzznomatchtoken"),
        ("old_sparse_hit", "annual budget 2026"),
    ] {
        let eligible = all_ids(conn, RECENT_SET_SQL, query);
        let mut recency_samples = Vec::with_capacity(30);
        let mut pooled_samples = Vec::with_capacity(30);
        let mut recency_results = Vec::new();
        let mut pooled_results = Vec::new();
        for iteration in 0..35 {
            let start = Instant::now();
            let recency = recent_first(conn, query, SEARCH_RESULT_LIMIT);
            let recency_ms = start.elapsed().as_secs_f64() * 1_000.0;
            let start = Instant::now();
            let pooled = db
                .search_global_entries(query, SEARCH_RESULT_LIMIT, 0)
                .expect("full production pooled control");
            let pooled_ms = start.elapsed().as_secs_f64() * 1_000.0;
            if iteration >= 5 {
                recency_samples.push(recency_ms);
                pooled_samples.push(pooled_ms);
            }
            recency_results = recency;
            pooled_results = pooled;
        }
        let fast_ids = recency_results
            .iter()
            .map(|row| row.0.as_str())
            .collect::<Vec<_>>();
        let prod_ids = pooled_results
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>();
        assert!(fast_ids.iter().all(|id| eligible.contains(*id)));
        if case == "no_hit" {
            assert!(fast_ids.is_empty());
            assert!(prod_ids.is_empty());
        } else {
            assert_eq!(eligible.len(), 1, "sparse term must have one eligible hit");
        }
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "fts_recency_tail_risk",
            "context": context,
            "case": case,
            "query": query,
            "eligible_matches": eligible.len(),
            "recency_latency_ms": summarize_samples(&recency_samples),
            "production_pooled_latency_ms": summarize_samples(&pooled_samples),
            "recency_ids": fast_ids,
            "production_ids": prod_ids,
            "same_id_order": fast_ids == prod_ids,
            "samples": 30,
            "method": "sequential same-fixture control, 5 warmups and 30 samples; stage timings are not paired total-pipeline qualification",
            "warning": "LIMIT 80 bounds outputs, not number of rows scanned in recency index"
        }));
    }
}

/// Document an existing path-casing mismatch in the *experimental* instr()
/// matcher: Windows normalizes the path to lowercase; macOS does not. FTS5
/// trigram matching is configured independently of this naive normalization.
#[test]
fn recency_experiment_path_case_is_platform_sensitive() {
    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open path-case fixture");
    db.upsert_global_volume(&test_volume())
        .expect("insert enabled volume");
    let entry = test_entry(r"C:\Global\Reports\case_only.txt", "case_only.txt", false);
    let id = entry.entry_id();
    db.upsert_global_entries_batch(&[entry])
        .expect("insert mixed-case-path entry");
    let conn = db.conn().expect("get path-case connection");
    let stored: String = conn
        .query_row(
            "SELECT path_normalized FROM global_entries WHERE id = ?1",
            [id.as_str()],
            |row| row.get(0),
        )
        .expect("read actual platform-normalized path");
    let matched = recent_first(&conn, "report", 80)
        .into_iter()
        .any(|row| row.0 == id);
    assert_eq!(matched, stored.contains("report"));
    assert_eq!(matched, cfg!(windows));
}
