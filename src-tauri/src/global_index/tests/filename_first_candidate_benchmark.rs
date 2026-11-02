//! Paired native preflight for the filename-first candidate.
//!
//! Run only on a native Windows/macOS runner in Release mode. The SQLite
//! baseline and candidate alternate against one immutable fixture; update
//! samples use two pre-opened connections so the only intended difference is
//! the candidate's connection-local mutation hook.

use super::*;
use crate::db::Database;
use crate::global_index::filename_search::diagnostic_filename_index_build_sql;
use crate::global_index::models::GlobalSearchResult;
use crate::global_index::search::{
    diagnostic_filename_rank_stage_profile_on_connection,
    diagnostic_filename_trigram_candidates_sql, diagnostic_legacy_filename_trigram_join_sql,
    diagnostic_search_tier_sql, with_legacy_filename_join_candidates_for_test,
};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, StatementStatus};
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FILENAME_CANDIDATE_WARMUPS: usize = 5;
const FILENAME_CANDIDATE_SAMPLES: usize = 30;
const FILENAME_CANDIDATE_WRITE_BATCHES: [usize; 2] = [512, 5_000];

#[path = "issue494_1m_qualification.rs"]
mod million_qualification;
#[path = "issue494_writer_qualification.rs"]
mod writer_qualification;

#[derive(Clone, Copy)]
struct WriteTiming {
    total_ms: f64,
    update_ms: f64,
    commit_ms: f64,
    checkpoint_ms: f64,
}

fn timed<T>(operation: impl FnOnce() -> T) -> (T, f64) {
    let started = Instant::now();
    let result = operation();
    (result, started.elapsed().as_secs_f64() * 1000.0)
}

struct CandidateCase {
    class: &'static str,
    query: &'static str,
    offset: u32,
    expected_ids: Vec<String>,
    expected_total: u64,
}

#[derive(Clone, Copy, Debug)]
enum SearchRoute {
    ProductionSqlite,
    HistoricalJoinedCandidate,
    RowidOnlyCandidate,
}

impl SearchRoute {
    const ALL: [Self; 3] = [
        Self::ProductionSqlite,
        Self::HistoricalJoinedCandidate,
        Self::RowidOnlyCandidate,
    ];

    const fn index(self) -> usize {
        match self {
            Self::ProductionSqlite => 0,
            Self::HistoricalJoinedCandidate => 1,
            Self::RowidOnlyCandidate => 2,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::ProductionSqlite => "production_sqlite",
            Self::HistoricalJoinedCandidate => "historical_joined_candidate",
            Self::RowidOnlyCandidate => "rowid_only_candidate",
        }
    }

    const fn is_candidate(self) -> bool {
        !matches!(self, Self::ProductionSqlite)
    }
}

#[test]
#[ignore = "Issue #494 paired native Release qualification; set scale to 100k, 200k, or 1M"]
fn issue494_filename_first_native_preflight() {
    let entries = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES")
        .expect("set an authorized fixture scale")
        .parse::<u64>()
        .expect("fixture scale is numeric");
    assert!(
        matches!(entries, 100_000 | 200_000 | 1_000_000),
        "only 100k/200k preflight and one 1M qualification are authorized"
    );
    assert!(
        !std::hint::black_box(cfg!(debug_assertions)),
        "native preflight must run in Release mode"
    );
    if cfg!(target_os = "windows") {
        assert_eq!(
            std::hint::black_box(std::env::consts::ARCH),
            "x86_64",
            "Windows qualification requires native x86_64"
        );
    } else if cfg!(target_os = "macos") {
        assert_eq!(
            std::hint::black_box(std::env::consts::ARCH),
            "aarch64",
            "macOS qualification requires native Apple Silicon"
        );
    } else {
        panic!("Issue #494 qualification requires native Windows or macOS");
    }

    if let Ok(output) = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT") {
        let output = PathBuf::from(output);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).expect("create candidate evidence directory");
        }
        fs::write(output, "").expect("start fresh candidate JSONL evidence");
    }

    let context = benchmark_context(entries);
    if entries == 1_000_000 {
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "issue494_1m_native_qualification_started",
            "context": context,
            "required_native_contract": "Windows x86_64 and Apple Silicon macOS; exact 1M synthetic fixture; no production search-code changes"
        }));
    }
    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open paired Global Index fixture");
    db.upsert_global_volume(&test_volume())
        .expect("insert authorized synthetic source");
    let (expectations, generation_ms, population_ms, population_batch_ms) =
        populate_synthetic_database(&db, entries);

    let baseline_conn = configured_connection(&path).expect("open query plan reader");
    let source_row_count: i64 = baseline_conn
        .query_row("SELECT COUNT(*) FROM global_entries", [], |row| row.get(0))
        .expect("count fixture rows");
    assert_eq!(source_row_count, entries as i64);
    let schema_objects = persistent_schema_objects(&baseline_conn);
    let db_before_index = database_metrics(&baseline_conn, &path);

    let (process_before_index, process_after_index, index_build_peak_sampling) =
        if entries == 1_000_000 {
            million_qualification::rebuild_index_with_sampled_peak(&db)
        } else {
            rebuild_index_with_low_scale_sampled_peak(&db)
        };
    let rss_before_index = process_before_index.and_then(|sample| sample.rss_bytes);
    let rss_after_index = process_after_index.and_then(|sample| sample.rss_bytes);
    let index_diagnostics = db.filename_search.diagnostics();
    assert!(
        index_diagnostics.ready,
        "candidate index must be ready before sampling"
    );
    assert_eq!(index_diagnostics.index_entries, entries as usize);
    let index_build_memory_profile = db
        .filename_search
        .build_memory_profile_for_test()
        .expect("capture build-phase memory profile");

    let fts_sql = diagnostic_search_tier_sql("fts");
    let trigram_sql = diagnostic_filename_trigram_candidates_sql();
    let joined_trigram_sql = diagnostic_legacy_filename_trigram_join_sql();
    let mut plan_records = vec![
        plan_record(
            &baseline_conn,
            &context,
            "legacy_sqlite_fts_ranked_candidates",
            "report",
            fts_sql,
            vec![SqlValue::Text("\"report\"".into()), SqlValue::Integer(80)],
        ),
        plan_record(
            &baseline_conn,
            &context,
            "historical_joined_candidate_trigram_and_extension_filter",
            "report",
            joined_trigram_sql.to_string(),
            vec![
                SqlValue::Text("\"report\"".into()),
                SqlValue::Text("report".into()),
            ],
        ),
        plan_record(
            &baseline_conn,
            &context,
            "rowid_only_candidate_trigram_postings",
            "report",
            trigram_sql.to_string(),
            vec![SqlValue::Text("\"report\"".into())],
        ),
    ];
    let mut index_build_plan = plan_record(
        &baseline_conn,
        &context,
        "compact_filename_projection_build",
        "index build",
        diagnostic_filename_index_build_sql().to_string(),
        Vec::new(),
    );
    index_build_plan["plan_sql_source"] = json!("filename_search.rs::FILENAME_INDEX_BUILD_SQL");
    let index_build_details = index_build_plan["details"]
        .as_array()
        .expect("index build plan details")
        .iter()
        .filter_map(|detail| detail.as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    plan_records.push(index_build_plan);

    let mut candidate_cases = vec![
        CandidateCase {
            class: "single_name_prefix",
            query: "Annual Budget 2026",
            offset: 0,
            expected_ids: vec!["ge_bench_0000000".into()],
            expected_total: 1,
        },
        CandidateCase {
            class: "one_filename_substring",
            query: "budget",
            offset: 0,
            expected_ids: vec!["ge_bench_0000000".into()],
            expected_total: 1,
        },
        CandidateCase {
            class: "high_filename_substring",
            query: "report",
            offset: 0,
            expected_ids: expectations[3]
                .expected_page(0, 80)
                .into_iter()
                .map(str::to_owned)
                .collect(),
            expected_total: expectations[3].total_matches,
        },
        CandidateCase {
            class: "high_filename_substring_deep_window",
            query: "report",
            offset: 4016,
            expected_ids: expectations[3]
                .expected_page(4016, 80)
                .into_iter()
                .map(str::to_owned)
                .collect(),
            expected_total: expectations[3].total_matches,
        },
        CandidateCase {
            class: "zero_filename_match",
            query: "zzznomatchtoken",
            offset: 0,
            expected_ids: Vec::new(),
            expected_total: 0,
        },
    ];
    if entries == 1_000_000 {
        candidate_cases.push(million_qualification::sparse_filename_case(&baseline_conn));
        assert_eq!(
            candidate_cases[2].expected_total, 50_000,
            "the 1M high-hit query must contain exactly 50,000 eligible filename matches"
        );
    }

    let candidate_sql_metrics = candidate_cases
        .iter()
        .map(|case| {
            json!({
                "query_class": case.class,
                "query": case.query,
                "routes": SearchRoute::ALL
                    .into_iter()
                    .map(|route| measure_candidate_sql(&baseline_conn, route, case.query))
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();

    let mut search_records = Vec::new();
    for case in &candidate_cases {
        let mut search_record = measure_search_pair(&db, &context, case);
        if entries == 1_000_000 {
            search_record["record_type"] = json!("issue494_1m_search_pair");
            emit_record(&search_record);
        }
        search_records.push(search_record);
        let mut snapshot_record = measure_snapshot_pair(&db, &context, case);
        if entries == 1_000_000 {
            snapshot_record["record_type"] = json!("issue494_1m_snapshot_pair");
            emit_record(&snapshot_record);
        }
        search_records.push(snapshot_record);
    }
    if entries == 1_000_000 {
        let window_record = million_qualification::verify_complete_candidate_window(
            &db,
            &expectations[3],
            &context,
        );
        emit_record(&window_record);
    }

    let process_after_queries = native_process_sample();
    let candidate_reader_sqlite_memory = db.filename_search.with_reader_for_test(|connection| {
        json!({
            "dbstatus_cache_used_bytes": sqlite_cache_used_bytes(connection),
            "effective_mmap_size_bytes": connection
                .query_row("PRAGMA mmap_size", [], |row| row.get::<_, i64>(0))
                .ok()
        })
    });
    let idle_seconds = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_IDLE_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    assert!(
        matches!(idle_seconds, 0 | 1_800),
        "idle attribution is either disabled or one 30-minute window"
    );
    let idle_resources = measure_idle_resources(idle_seconds);
    let process_after_idle = native_process_sample();
    let diagnostics_after_search = db.filename_search.diagnostics();
    db.filename_search.set_enabled_for_test(true);
    let rank_component_records = candidate_cases
        .iter()
        .filter(|case| case.class.starts_with("high_filename_substring"))
        .map(|case| measure_rank_component_replay(&db, &context, case))
        .collect::<Vec<_>>();
    let process_after_rank_replay = native_process_sample();

    let paired_batch_writes = FILENAME_CANDIDATE_WRITE_BATCHES
        .into_iter()
        .map(|batch_size| {
            let (baseline_timings, candidate_timings, pair_order) =
                measure_mutation_hook_write_cost(&db, &path, entries as usize, batch_size);
            paired_write_record(
                batch_size,
                &baseline_timings,
                &candidate_timings,
                pair_order,
            )
        })
        .collect::<Vec<_>>();

    let post_write_metrics = database_metrics(&baseline_conn, &path);
    let process_after_queries_and_writes = native_process_sample();
    let rss_after_queries_and_writes =
        process_after_queries_and_writes.and_then(|sample| sample.rss_bytes);
    let diagnostics_after = diagnostics_after_search;
    if entries == 1_000_000 {
        let source_authorization_record =
            million_qualification::verify_revoked_source_fallback(&db, &context);
        emit_record(&source_authorization_record);
    }
    emit_record(&json!({
        "schema_version": 1,
        "record_type": if entries == 1_000_000 {
            "issue494_filename_first_1m_native_qualification"
        } else {
            "issue494_filename_first_native_preflight"
        },
        "context": context,
        "fixture": {
            "entry_count": entries,
            "name_source": "existing global_search_benchmark.rs deterministic generator; unchanged",
            "population_batch_size": INSERT_BATCH_SIZE,
            "production_triggers_preserved": assert_entry_triggers_enabled(&baseline_conn),
            "source_rows_verified": source_row_count,
            "query_candidate_limit": SEARCH_CANDIDATE_LIMIT,
            "full_result_contract": "exact candidate IDs and order checked against the independent generated-file oracle; complete result fields compared between paired routes, rank ignored because BM25 compatibility is explicitly not required"
        },
        "construction": {
            "dataset_generation_ms": round_ms(generation_ms),
            "sqlite_population_ms": round_ms(population_ms),
            "population_transaction_ms": summarize_samples(&population_batch_ms),
            "index_build_ms": index_diagnostics.last_build_micros as f64 / 1000.0,
            "index_entries": index_diagnostics.index_entries,
            "distinct_filename_count": index_diagnostics.distinct_filename_count,
            "estimated_retained_index_bytes": index_diagnostics.index_private_bytes,
            "index_build_peak_bytes": index_diagnostics.index_build_peak_bytes,
            "sampled_process_index_build_peak": index_build_peak_sampling,
            "build_phase_memory_profile": index_build_memory_profile_json(&index_build_memory_profile),
            "rss_before_index_build_bytes": rss_before_index,
            "rss_after_index_build_bytes": rss_after_index,
            "process_before_index_build": native_sample_json(process_before_index),
            "process_after_index_build": native_sample_json(process_after_index),
            "persistent_search_index_bytes": 0,
            "persistent_schema_objects": schema_objects,
            "database_before_index": db_before_index
        },
        "candidate_diagnostics": {
            "enabled_during_measurement": diagnostics_after.enabled,
            "ready": diagnostics_after.ready,
            "rebuild_in_progress": diagnostics_after.rebuild_in_progress,
            "rebuilds": diagnostics_after.rebuilds,
            "accepted_queries": diagnostics_after.accepted_queries,
            "last_error": diagnostics_after.last_error,
            "fallback_not_ready": diagnostics_after.fallback_not_ready,
            "fallback_busy": diagnostics_after.fallback_busy,
            "fallback_untracked_change": diagnostics_after.fallback_untracked_change,
            "fallback_source_change": diagnostics_after.fallback_source_change,
            "fallback_overlay_limit": diagnostics_after.fallback_overlay_limit
        },
        "candidate_query": {
            "retrieval": "the new candidate asks FTS5 for rowids only; compact immutable projection supplies mtime, stable-id, filename and extension keys; a bounded Rust heap keeps the complete top 4096; path-only matches use the existing indexed trigram table after filename candidates",
            "index_overlay_rows_last_query": diagnostics_after.overlay_rows,
            "sqlite_vm_steps_and_candidate_row_counts_by_query": candidate_sql_metrics,
            "plans": plan_records,
            "projection_build_plan_details": index_build_details
        },
        "paired_search_and_snapshot": search_records,
        "paired_batch_writes": paired_batch_writes,
        "write_cost_semantics": "the 1M harness records paired 512/5,000-row forced-FULL-checkpoint stress costs; production WAL/autocheckpoint qualification remains the already completed, separately reported 200k writer evidence",
        "post_write_database": post_write_metrics,
        "rss_after_queries_and_writes_bytes": rss_after_queries_and_writes,
        "resource_attribution": {
            "process_after_search_before_rank_replay": native_sample_json(process_after_queries),
            "process_after_idle_before_rank_replay": native_sample_json(process_after_idle),
            "process_after_rank_component_replay": native_sample_json(process_after_rank_replay),
            "process_after_queries_and_writes": native_sample_json(process_after_queries_and_writes),
            "private_bytes_delta_after_queries": sample_private_delta(process_after_index, process_after_queries),
            "working_set_delta_after_queries": sample_rss_delta(process_after_index, process_after_queries),
            "private_bytes_delta_index_build": sample_private_delta(process_before_index, process_after_index),
            "working_set_delta_index_build": sample_rss_delta(process_before_index, process_after_index),
            "private_bytes_delta_after_queries_and_writes": sample_private_delta(process_after_index, process_after_queries_and_writes),
            "private_bytes_delta_before_index_to_after_queries_and_writes": sample_private_delta(process_before_index, process_after_queries_and_writes),
            "working_set_delta_after_queries_and_writes": sample_rss_delta(process_after_index, process_after_queries_and_writes),
            "candidate_reader_sqlite": candidate_reader_sqlite_memory,
            "estimated_retained_filename_index_heap_bytes": diagnostics_after.index_private_bytes,
            "estimated_filename_index_build_peak_heap_bytes": diagnostics_after.index_build_peak_bytes,
            "mmap_resident_bytes": null,
            "mmap_residency_limit": "effective mmap window and process working-set/private footprint are reported; the OS does not expose per-SQLite-mapping resident bytes through this benchmark API",
            "idle_30m": idle_resources,
            "rank_component_replay_records": rank_component_records.len(),
            "rank_replay_private_delta_bytes": sample_private_delta(process_after_idle, process_after_rank_replay),
            "process_kind": "native Release Rust test harness using public Database and Repository Snapshot paths; not installed Tauri GUI"
        },
        "fallback_counters": {
            "not_ready": diagnostics_after.fallback_not_ready,
            "busy": diagnostics_after.fallback_busy,
            "untracked_change": diagnostics_after.fallback_untracked_change,
            "source_change": diagnostics_after.fallback_source_change,
            "overlay_limit": diagnostics_after.fallback_overlay_limit
        },
        "correctness": "every candidate sample matched the independent filename-first ID order; fields for IDs shared by the two routes matched except rank; baseline order differences are recorded rather than rejected; Repository Snapshot source health, source revision and index status matched pairwise",
        "limitations": [
            "process-private accounting uses Windows PrivateUsage and macOS phys_footprint; both include the Rust test harness and allocator, not only filename-index allocations",
            "SQLite cache bytes and effective mmap window are separated; per-mapping OS resident bytes are not isolated by this benchmark",
            "this preflight is synthetic and does not measure installed-app GUI or live filesystem indexing",
            "the in-memory candidate rebuilds after process restart because no persistent sidecar is used"
        ]
    }));
    for record in rank_component_records {
        emit_record(&record);
    }
}

fn measure_search_pair(db: &Database, context: &JsonValue, case: &CandidateCase) -> JsonValue {
    let expected = SearchRoute::ALL.map(|route| run_search_route(db, route, case));
    assert_shared_result_fields_equal_except_rank(&expected[0], &expected[1]);
    assert_shared_result_fields_equal_except_rank(&expected[0], &expected[2]);
    assert_result_ids(&expected[1], case);
    assert_result_ids(&expected[2], case);
    let mut samples: [Vec<f64>; 3] =
        std::array::from_fn(|_| Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES));
    const ROUTE_ORDERS: [[SearchRoute; 3]; 6] = [
        [
            SearchRoute::ProductionSqlite,
            SearchRoute::HistoricalJoinedCandidate,
            SearchRoute::RowidOnlyCandidate,
        ],
        [
            SearchRoute::ProductionSqlite,
            SearchRoute::RowidOnlyCandidate,
            SearchRoute::HistoricalJoinedCandidate,
        ],
        [
            SearchRoute::HistoricalJoinedCandidate,
            SearchRoute::ProductionSqlite,
            SearchRoute::RowidOnlyCandidate,
        ],
        [
            SearchRoute::HistoricalJoinedCandidate,
            SearchRoute::RowidOnlyCandidate,
            SearchRoute::ProductionSqlite,
        ],
        [
            SearchRoute::RowidOnlyCandidate,
            SearchRoute::ProductionSqlite,
            SearchRoute::HistoricalJoinedCandidate,
        ],
        [
            SearchRoute::RowidOnlyCandidate,
            SearchRoute::HistoricalJoinedCandidate,
            SearchRoute::ProductionSqlite,
        ],
    ];
    let total_iterations = FILENAME_CANDIDATE_WARMUPS + FILENAME_CANDIDATE_SAMPLES;
    for iteration in 0..total_iterations {
        for route in ROUTE_ORDERS[iteration % ROUTE_ORDERS.len()] {
            let (actual, elapsed) = timed(|| run_search_route(db, route, case));
            assert_shared_result_fields_equal_except_rank(&expected[0], &actual);
            if route.is_candidate() {
                assert_result_ids(&actual, case);
            }
            if iteration >= FILENAME_CANDIDATE_WARMUPS {
                samples[route.index()].push(elapsed);
            }
        }
    }
    json!({
        "context": context,
        "operation": "public Database::search_global_entries full route",
        "query_class": case.class,
        "query": case.query,
        "limit": SEARCH_RESULT_LIMIT,
        "offset": case.offset,
        "expected_total_matches": case.expected_total,
        "routes": {
            "production_sqlite": summarize_samples(&samples[0]),
            "historical_joined_candidate": summarize_samples(&samples[1]),
            "rowid_only_candidate": summarize_samples(&samples[2])
        },
        "route_raw_ms": {
            "production_sqlite": samples[0],
            "historical_joined_candidate": samples[1],
            "rowid_only_candidate": samples[2]
        },
        "sqlite_page_ids": result_ids(&expected[0]),
        "historical_joined_page_ids": result_ids(&expected[1]),
        "rowid_only_page_ids": result_ids(&expected[2]),
        "independent_oracle_page_ids": case.expected_ids,
        "shared_result_fields_equal_except_rank": true,
        "sqlite_and_rowid_page_id_sets_equal": same_result_id_set(&expected[0], &expected[2]),
        "sqlite_order_matches_filename_oracle": result_ids(&expected[0]) == case.expected_ids,
        "both_candidate_routes_match_independent_oracle_ids_and_order": true,
        "ranking_contract": "both candidates are checked against the independent filename-first oracle; production SQLite order is recorded without assuming BM25 compatibility",
        "pair_order": "three routes run per sample; all six route orders are rotated, with five warmups and thirty measured samples per route",
        "stage_profile": measure_candidate_stage_profile(db, case, false)
    })
}

fn measure_snapshot_pair(db: &Database, context: &JsonValue, case: &CandidateCase) -> JsonValue {
    let expected = SearchRoute::ALL.map(|route| run_snapshot_route(db, route, case));
    for route in SearchRoute::ALL.into_iter().skip(1) {
        assert_snapshot_equal(&expected[0], &expected[route.index()]);
        assert_result_ids(&expected[route.index()].results, case);
    }
    let mut samples: [Vec<f64>; 3] =
        std::array::from_fn(|_| Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES));
    const ROUTE_ORDERS: [[SearchRoute; 3]; 6] = [
        [
            SearchRoute::ProductionSqlite,
            SearchRoute::HistoricalJoinedCandidate,
            SearchRoute::RowidOnlyCandidate,
        ],
        [
            SearchRoute::ProductionSqlite,
            SearchRoute::RowidOnlyCandidate,
            SearchRoute::HistoricalJoinedCandidate,
        ],
        [
            SearchRoute::HistoricalJoinedCandidate,
            SearchRoute::ProductionSqlite,
            SearchRoute::RowidOnlyCandidate,
        ],
        [
            SearchRoute::HistoricalJoinedCandidate,
            SearchRoute::RowidOnlyCandidate,
            SearchRoute::ProductionSqlite,
        ],
        [
            SearchRoute::RowidOnlyCandidate,
            SearchRoute::ProductionSqlite,
            SearchRoute::HistoricalJoinedCandidate,
        ],
        [
            SearchRoute::RowidOnlyCandidate,
            SearchRoute::HistoricalJoinedCandidate,
            SearchRoute::ProductionSqlite,
        ],
    ];
    let total_iterations = FILENAME_CANDIDATE_WARMUPS + FILENAME_CANDIDATE_SAMPLES;
    for iteration in 0..total_iterations {
        for route in ROUTE_ORDERS[iteration % ROUTE_ORDERS.len()] {
            let (actual, elapsed) = timed(|| run_snapshot_route(db, route, case));
            assert_snapshot_equal(&expected[0], &actual);
            if route.is_candidate() {
                assert_result_ids(&actual.results, case);
            }
            if iteration >= FILENAME_CANDIDATE_WARMUPS {
                samples[route.index()].push(elapsed);
            }
        }
    }
    json!({
        "context": context,
        "operation": "Database::search_global_entries_snapshot full repository path",
        "query_class": case.class,
        "query": case.query,
        "limit": SEARCH_RESULT_LIMIT,
        "offset": case.offset,
        "expected_total_matches": case.expected_total,
        "routes": {
            "production_sqlite": summarize_samples(&samples[0]),
            "historical_joined_candidate": summarize_samples(&samples[1]),
            "rowid_only_candidate": summarize_samples(&samples[2])
        },
        "route_raw_ms": {
            "production_sqlite": samples[0],
            "historical_joined_candidate": samples[1],
            "rowid_only_candidate": samples[2]
        },
        "production_sqlite_ids": result_ids(&expected[0].results),
        "historical_joined_ids": result_ids(&expected[1].results),
        "rowid_only_ids": result_ids(&expected[2].results),
        "independent_oracle_page_ids": case.expected_ids,
        "all_source_health_revision_and_index_status_equal": true,
        "both_candidate_routes_match_independent_oracle_ids_and_order": true,
        "pair_order": "three routes run per sample; all six route orders are rotated, with five warmups and thirty measured samples per route",
        "stage_profile": measure_candidate_stage_profile(db, case, true)
    })
}

fn measure_candidate_stage_profile(
    db: &Database,
    case: &CandidateCase,
    snapshot: bool,
) -> JsonValue {
    db.filename_search.set_enabled_for_test(true);
    let expected_search =
        (!snapshot).then(|| run_search_route(db, SearchRoute::RowidOnlyCandidate, case));
    let expected_snapshot =
        snapshot.then(|| run_snapshot_route(db, SearchRoute::RowidOnlyCandidate, case));
    let mut full_route_samples = Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES);
    let mut stage_samples = HashMap::<&'static str, Vec<f64>>::new();

    for iteration in 0..(FILENAME_CANDIDATE_WARMUPS + FILENAME_CANDIDATE_SAMPLES) {
        let started = Instant::now();
        let ((actual_search, actual_snapshot), stages) =
            crate::global_index::filename_search::capture_candidate_search_stages(|| {
                if snapshot {
                    (
                        None,
                        Some(run_snapshot_route(
                            db,
                            SearchRoute::RowidOnlyCandidate,
                            case,
                        )),
                    )
                } else {
                    (
                        Some(run_search_route(db, SearchRoute::RowidOnlyCandidate, case)),
                        None,
                    )
                }
            });
        let profiled_full_route_ms = started.elapsed().as_secs_f64() * 1000.0;
        match (
            actual_search,
            actual_snapshot,
            &expected_search,
            &expected_snapshot,
        ) {
            (Some(actual), None, Some(expected), None) => {
                assert_shared_result_fields_equal_except_rank(expected, &actual);
                assert_result_ids(&actual, case);
            }
            (None, Some(actual), None, Some(expected)) => {
                assert_snapshot_equal(expected, &actual);
                assert_result_ids(&actual.results, case);
            }
            _ => panic!("candidate stage profile must follow exactly one full route"),
        }
        if iteration >= FILENAME_CANDIDATE_WARMUPS {
            full_route_samples.push(profiled_full_route_ms);
            for (stage, duration_ns) in stages {
                stage_samples
                    .entry(stage)
                    .or_default()
                    .push(duration_ns as f64 / 1_000_000.0);
            }
        }
    }

    let stage_distributions = stage_samples
        .iter()
        .map(|(stage, samples)| (*stage, summarize_samples(samples)))
        .collect::<HashMap<_, _>>();
    json!({
        "instrumentation": "cfg(test)-only timers; paired full-route samples above were measured without stage capture",
        "operation": if snapshot { "candidate Repository Snapshot" } else { "candidate Global Search" },
        "warmups": FILENAME_CANDIDATE_WARMUPS,
        "samples": FILENAME_CANDIDATE_SAMPLES,
        "instrumented_full_route_ms": summarize_samples(&full_route_samples),
        "instrumented_stage_ms": stage_distributions,
        "instrumented_stage_raw_ms": stage_samples,
        "conditional_stages_not_exercised": ["source_state_revalidation; immutable fixture leaves authorization unchanged"],
        "full_result_contract_checked": true
    })
}

fn measure_rank_component_replay(
    db: &Database,
    context: &JsonValue,
    case: &CandidateCase,
) -> JsonValue {
    db.filename_search.set_enabled_for_test(true);
    let mut replay_ms = Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES);
    let mut identity_setup_ms = Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES);
    let mut lookup_ms = Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES);
    let mut top_k_ms = Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES);
    let mut sort_ms = Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES);
    let mut accepted_candidates = Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES);
    let mut overlay_rows = Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES);

    for iteration in 0..(FILENAME_CANDIDATE_WARMUPS + FILENAME_CANDIDATE_SAMPLES) {
        let (profile, elapsed_ms) = timed(|| {
            db.filename_search
                .try_with_snapshot(|transaction, index, changes| {
                    let profile = diagnostic_filename_rank_stage_profile_on_connection(
                        transaction,
                        case.query,
                        SEARCH_RESULT_LIMIT,
                        case.offset,
                        index,
                        changes,
                    )?;
                    let Some(profile) = profile else {
                        return Ok(None);
                    };
                    Ok(Some(profile))
                })
                .expect("run source-consistent rank component replay")
                .expect("candidate index remains ready for rank replay")
        });
        if iteration >= FILENAME_CANDIDATE_WARMUPS {
            replay_ms.push(elapsed_ms);
            identity_setup_ms.push(profile.identity_setup_ns as f64 / 1_000_000.0);
            lookup_ms.push(profile.lookup_and_classification_ns as f64 / 1_000_000.0);
            top_k_ms.push(profile.top_k_heap_ns as f64 / 1_000_000.0);
            sort_ms.push(profile.heap_sort_ns as f64 / 1_000_000.0);
            accepted_candidates.push(profile.accepted_candidates as f64);
            overlay_rows.push(profile.overlay_rows as f64);
        }
    }

    json!({
        "schema_version": 1,
        "record_type": "issue494_filename_rank_component_replay",
        "context": context,
        "query_class": case.class,
        "query": case.query,
        "offset": case.offset,
        "hit_count": case.expected_total,
        "warmups": FILENAME_CANDIDATE_WARMUPS,
        "samples": FILENAME_CANDIDATE_SAMPLES,
        "full_replay_wall_ms": summarize_samples(&replay_ms),
        "overlay_identity_setup_ms": summarize_samples(&identity_setup_ms),
        "rowid_lookup_and_match_classification_ms": summarize_samples(&lookup_ms),
        "bounded_top_k_heap_insertion_ms": summarize_samples(&top_k_ms),
        "final_heap_sort_ms": summarize_samples(&sort_ms),
        "accepted_rankable_candidate_count": summarize_samples(&accepted_candidates),
        "overlay_rows_at_profile_time": summarize_samples(&overlay_rows),
        "rowid_lookup_and_match_classification_raw_ms": lookup_ms,
        "bounded_top_k_heap_insertion_raw_ms": top_k_ms,
        "final_heap_sort_raw_ms": sort_ms,
        "method": "separate native replay using the same rowid lookup/classification helpers and Top-K insertion as the real route; replay result is asserted equal to the interleaved route; replay allocates a temporary rank-key Vec and is excluded from end-to-end Search p95 and resource snapshots",
        "uses_authoritative_source_snapshot": true
    })
}

fn run_search_route(
    db: &Database,
    route: SearchRoute,
    case: &CandidateCase,
) -> Vec<GlobalSearchResult> {
    db.filename_search
        .set_enabled_for_test(route.is_candidate());
    let run = || {
        db.search_global_entries(case.query, SEARCH_RESULT_LIMIT, case.offset)
            .expect("run paired public Global Search route")
    };
    match route {
        SearchRoute::HistoricalJoinedCandidate => {
            with_legacy_filename_join_candidates_for_test(run)
        }
        SearchRoute::ProductionSqlite | SearchRoute::RowidOnlyCandidate => run(),
    }
}

fn run_snapshot_route(
    db: &Database,
    route: SearchRoute,
    case: &CandidateCase,
) -> crate::global_index::repository::GlobalSearchSnapshot {
    db.filename_search
        .set_enabled_for_test(route.is_candidate());
    let run = || {
        db.search_global_entries_snapshot(case.query, SEARCH_RESULT_LIMIT, case.offset)
            .expect("run paired complete Repository Snapshot route")
    };
    match route {
        SearchRoute::HistoricalJoinedCandidate => {
            with_legacy_filename_join_candidates_for_test(run)
        }
        SearchRoute::ProductionSqlite | SearchRoute::RowidOnlyCandidate => run(),
    }
}

fn measure_candidate_sql(conn: &Connection, route: SearchRoute, query: &str) -> JsonValue {
    let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
    let mut statement = match route {
        SearchRoute::ProductionSqlite => conn
            .prepare(&diagnostic_search_tier_sql("fts"))
            .expect("prepare production SQLite FTS tier"),
        SearchRoute::HistoricalJoinedCandidate => conn
            .prepare(diagnostic_legacy_filename_trigram_join_sql())
            .expect("prepare historical joined candidate SQL"),
        SearchRoute::RowidOnlyCandidate => conn
            .prepare(diagnostic_filename_trigram_candidates_sql())
            .expect("prepare rowid-only candidate SQL"),
    };
    let mut rows = match route {
        SearchRoute::ProductionSqlite => statement
            .query(rusqlite::params![fts_query, 80_i64])
            .expect("execute production SQLite FTS tier"),
        SearchRoute::HistoricalJoinedCandidate => statement
            .query(rusqlite::params![fts_query, query])
            .expect("execute historical joined candidate SQL"),
        SearchRoute::RowidOnlyCandidate => statement
            .query([fts_query])
            .expect("execute rowid-only candidate SQL"),
    };
    let mut rows_returned = 0_u64;
    while rows.next().expect("step candidate SQL").is_some() {
        rows_returned += 1;
    }
    drop(rows);
    json!({
        "route": route.label(),
        "row_count_returned": rows_returned,
        "sqlite_vm_steps": statement.get_status(StatementStatus::VmStep),
        "sqlite_sort_operations": statement.get_status(StatementStatus::Sort),
        "sqlite_fullscan_steps": statement.get_status(StatementStatus::FullscanStep),
        "statement_memory_bytes": statement.get_status(StatementStatus::MemUsed)
    })
}

fn assert_result_ids(results: &[GlobalSearchResult], case: &CandidateCase) {
    let ids = result_ids(results);
    assert_eq!(
        ids, case.expected_ids,
        "class={} offset={}",
        case.class, case.offset
    );
}

fn result_ids(results: &[GlobalSearchResult]) -> Vec<String> {
    results.iter().map(|row| row.id.clone()).collect()
}

fn same_result_id_set(left: &[GlobalSearchResult], right: &[GlobalSearchResult]) -> bool {
    let left = left
        .iter()
        .map(|row| row.id.as_str())
        .collect::<HashSet<_>>();
    let right = right
        .iter()
        .map(|row| row.id.as_str())
        .collect::<HashSet<_>>();
    left == right
}

fn assert_shared_result_fields_equal_except_rank(
    left: &[GlobalSearchResult],
    right: &[GlobalSearchResult],
) {
    let right_by_id = right
        .iter()
        .map(|result| (result.id.as_str(), result))
        .collect::<HashMap<_, _>>();
    for left in left {
        let Some(right) = right_by_id.get(left.id.as_str()) else {
            continue;
        };
        let mut left = left.clone();
        let mut right = (*right).clone();
        left.rank = 0.0;
        right.rank = 0.0;
        assert_eq!(left, right);
    }
}

fn assert_snapshot_equal(
    left: &crate::global_index::repository::GlobalSearchSnapshot,
    right: &crate::global_index::repository::GlobalSearchSnapshot,
) {
    assert_shared_result_fields_equal_except_rank(&left.results, &right.results);
    assert_eq!(left.source_health, right.source_health);
    assert_eq!(left.source_revision, right.source_revision);
    assert_eq!(left.index_status, right.index_status);
}

fn measure_mutation_hook_write_cost(
    db: &Database,
    path: &Path,
    entries: usize,
    batch_size: usize,
) -> (Vec<WriteTiming>, Vec<WriteTiming>, Vec<&'static str>) {
    let mut baseline = configured_connection(path).expect("open no-hook SQLite writer");
    let mut candidate = configured_connection(path).expect("open candidate-hook SQLite writer");
    baseline
        .pragma_update(None, "wal_autocheckpoint", 0)
        .expect("disable implicit checkpoint for paired write sampling");
    candidate
        .pragma_update(None, "wal_autocheckpoint", 0)
        .expect("disable implicit checkpoint for paired write sampling");
    db.filename_search
        .install_mutation_hooks(&candidate)
        .expect("install same connection-local hooks used by candidate Database pool");

    let samples_count = FILENAME_CANDIDATE_WARMUPS + FILENAME_CANDIDATE_SAMPLES;
    let mut samples = [
        Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES),
        Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES),
    ];
    let mut pair_order = Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES);
    for iteration in 0..samples_count {
        let candidate_first = iteration.is_multiple_of(2);
        for position in 0..2 {
            let is_candidate = (position == 0) == candidate_first;
            let connection = if is_candidate {
                &mut candidate
            } else {
                &mut baseline
            };
            let timing = timed_write_batch(connection, entries, iteration, batch_size);
            if iteration >= FILENAME_CANDIDATE_WARMUPS {
                samples[usize::from(is_candidate)].push(timing);
            }
        }
        if iteration >= FILENAME_CANDIDATE_WARMUPS {
            pair_order.push(if candidate_first {
                "candidate_first"
            } else {
                "baseline_first"
            });
        }
    }
    (samples[0].clone(), samples[1].clone(), pair_order)
}

fn paired_write_record(
    batch_size: usize,
    baseline_timings: &[WriteTiming],
    candidate_timings: &[WriteTiming],
    pair_order: Vec<&'static str>,
) -> JsonValue {
    let baseline_ms = baseline_timings
        .iter()
        .map(|sample| sample.total_ms)
        .collect::<Vec<_>>();
    let candidate_ms = candidate_timings
        .iter()
        .map(|sample| sample.total_ms)
        .collect::<Vec<_>>();
    let baseline_latency = summarize_samples(&baseline_ms);
    let candidate_latency = summarize_samples(&candidate_ms);
    let baseline_p95 = baseline_latency["p95"].as_f64().unwrap();
    let candidate_p95 = candidate_latency["p95"].as_f64().unwrap();
    let p95_overhead_percent = if baseline_p95 > 0.0 {
        ((candidate_p95 / baseline_p95) - 1.0) * 100.0
    } else {
        0.0
    };

    json!({
        "warmups": FILENAME_CANDIDATE_WARMUPS,
        "samples": FILENAME_CANDIDATE_SAMPLES,
        "batch_size": batch_size,
        "operation": format!("{batch_size} UPDATEs of modified_at_fs and last_seen_at in one transaction on a pre-opened SQLite connection; production global_entries schema and triggers remain enabled"),
        "wal_checkpoint_policy": "wal_autocheckpoint=0 for both writers; a FULL checkpoint runs after every transaction and is included in total latency so checkpoint work is measured without being assigned to whichever route happened to cross the automatic threshold",
        "baseline_connection_hooks": "none",
        "candidate_connection_hooks": "candidate's connection-local SQLite update/commit/rollback hooks",
        "baseline_latency_ms": baseline_latency,
        "candidate_latency_ms": candidate_latency,
        "baseline_raw_ms": baseline_ms,
        "candidate_raw_ms": candidate_ms,
        "baseline_update_raw_ms": baseline_timings.iter().map(|sample| sample.update_ms).collect::<Vec<_>>(),
        "candidate_update_raw_ms": candidate_timings.iter().map(|sample| sample.update_ms).collect::<Vec<_>>(),
        "baseline_commit_raw_ms": baseline_timings.iter().map(|sample| sample.commit_ms).collect::<Vec<_>>(),
        "candidate_commit_raw_ms": candidate_timings.iter().map(|sample| sample.commit_ms).collect::<Vec<_>>(),
        "baseline_checkpoint_raw_ms": baseline_timings.iter().map(|sample| sample.checkpoint_ms).collect::<Vec<_>>(),
        "candidate_checkpoint_raw_ms": candidate_timings.iter().map(|sample| sample.checkpoint_ms).collect::<Vec<_>>(),
        "pair_order": pair_order,
        "candidate_p95_over_baseline_percent": round_ms(p95_overhead_percent),
        "sample_pairing": format!("the same {batch_size} rowids are updated by both connections in each iteration; first writer alternates by iteration")
    })
}

fn timed_write_batch(
    connection: &mut Connection,
    entries: usize,
    sample: usize,
    batch_size: usize,
) -> WriteTiming {
    assert!(batch_size < entries, "write batch must fit within fixture");
    let start = Instant::now();
    let transaction = connection.transaction().expect("begin paired batch update");
    let update_start = Instant::now();
    {
        let mut statement = transaction
            .prepare(
                "UPDATE global_entries
                 SET modified_at_fs=COALESCE(modified_at_fs,0)+1,
                     last_seen_at=last_seen_at+1
                 WHERE id=?1",
            )
            .expect("prepare paired batch update");
        let first = (sample * batch_size) % (entries - batch_size);
        for index in first..(first + batch_size) {
            let id = format!("ge_bench_{index:07}");
            assert_eq!(
                statement.execute([id]).expect("update one fixture row"),
                1,
                "fixture id must exist"
            );
        }
    }
    let update_ms = update_start.elapsed().as_secs_f64() * 1000.0;
    let commit_start = Instant::now();
    transaction.commit().expect("commit paired batch update");
    let commit_ms = commit_start.elapsed().as_secs_f64() * 1000.0;
    let checkpoint_start = Instant::now();
    let (busy, log_frames, checkpointed_frames): (i64, i64, i64) = connection
        .query_row("PRAGMA wal_checkpoint(FULL)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("checkpoint paired write batch");
    assert_eq!(busy, 0, "paired write checkpoint must not be blocked");
    assert_eq!(log_frames, checkpointed_frames);
    let checkpoint_ms = checkpoint_start.elapsed().as_secs_f64() * 1000.0;
    WriteTiming {
        total_ms: start.elapsed().as_secs_f64() * 1000.0,
        update_ms,
        commit_ms,
        checkpoint_ms,
    }
}

fn persistent_schema_objects(connection: &Connection) -> Vec<(String, String)> {
    connection
        .prepare("SELECT type,name FROM sqlite_schema WHERE type IN ('table','index','trigger') ORDER BY type,name")
        .expect("prepare persistent schema inventory")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query persistent schema inventory")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect persistent schema inventory")
}

#[derive(Clone, Copy)]
struct NativeProcessSample {
    rss_bytes: Option<u64>,
    private_bytes: Option<u64>,
    cpu_time_ns: Option<u64>,
    pageins: Option<u64>,
}

#[derive(Clone, Copy, Default)]
struct BuildProcessPeak {
    samples: u64,
    private_bytes: Option<u64>,
    rss_bytes: Option<u64>,
}

impl BuildProcessPeak {
    fn observe(&mut self, sample: Option<NativeProcessSample>) {
        let Some(sample) = sample else {
            return;
        };
        self.samples = self.samples.saturating_add(1);
        self.private_bytes = max_optional_u64(self.private_bytes, sample.private_bytes);
        self.rss_bytes = max_optional_u64(self.rss_bytes, sample.rss_bytes);
    }
}

fn max_optional_u64(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    }
}

fn spawn_build_peak_sampler(
    stop: Arc<AtomicBool>,
    peak: Arc<Mutex<BuildProcessPeak>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || loop {
        peak.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .observe(native_process_sample());
        if stop.load(Ordering::Acquire) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    })
}

fn rebuild_index_with_low_scale_sampled_peak(
    db: &Database,
) -> (
    Option<NativeProcessSample>,
    Option<NativeProcessSample>,
    JsonValue,
) {
    let before = native_process_sample();
    let stop = Arc::new(AtomicBool::new(false));
    let peak = Arc::new(Mutex::new(BuildProcessPeak::default()));
    let sampler = spawn_build_peak_sampler(Arc::clone(&stop), Arc::clone(&peak));

    db.filename_search.enable_for_test();
    let build_started = Instant::now();
    let result = db.filename_search.rebuild_now_for_test();
    let build_wall_ms = build_started.elapsed().as_secs_f64() * 1_000.0;
    stop.store(true, Ordering::Release);
    sampler.join().expect("join native build peak sampler");
    result.expect("synchronously build filename candidate for paired measurement");
    let after = native_process_sample();
    let peak = *peak
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(
        peak.samples > 0,
        "native process sampler must observe the build"
    );
    let delta = |high: Option<u64>, base: Option<u64>| Some(high? as i64 - base? as i64);
    let sample_control = measure_build_sampler_overhead_control();
    let record = json!({
        "measured": true,
        "sampling_interval_ms": 5,
        "sample_count": peak.samples,
        "build_wall_ms": round_ms(build_wall_ms),
        "peak_private_bytes": peak.private_bytes,
        "peak_private_delta_from_prebuild_bytes": delta(peak.private_bytes, before.and_then(|sample| sample.private_bytes)),
        "peak_working_set_bytes": peak.rss_bytes,
        "peak_working_set_delta_from_prebuild_bytes": delta(peak.rss_bytes, before.and_then(|sample| sample.rss_bytes)),
        "prebuild": native_sample_json(before),
        "postbuild": native_sample_json(after),
        "sampling_method": if cfg!(target_os = "windows") {
            "5ms polling of Windows PrivateUsage and working set"
        } else {
            "5ms polling of macOS phys_footprint and resident size"
        },
        "sampler_overhead_control": sample_control
    });
    (before, after, record)
}

fn measure_build_sampler_overhead_control() -> JsonValue {
    const WINDOW: Duration = Duration::from_millis(250);

    let no_sampler_before = native_process_sample();
    std::thread::sleep(WINDOW);
    let no_sampler_after = native_process_sample();

    let sampled_before = native_process_sample();
    let stop = Arc::new(AtomicBool::new(false));
    let peak = Arc::new(Mutex::new(BuildProcessPeak::default()));
    let sampler = spawn_build_peak_sampler(Arc::clone(&stop), Arc::clone(&peak));
    std::thread::sleep(WINDOW);
    stop.store(true, Ordering::Release);
    sampler.join().expect("join sampler overhead control");
    let sampled_after = native_process_sample();
    let peak = *peak
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let cpu_delta = |before: Option<NativeProcessSample>, after: Option<NativeProcessSample>| {
        Some(after?.cpu_time_ns? as i64 - before?.cpu_time_ns? as i64)
    };
    let no_sampler_cpu_ns = cpu_delta(no_sampler_before, no_sampler_after);
    let sampled_cpu_ns = cpu_delta(sampled_before, sampled_after);
    let incremental_cpu_ns = sampled_cpu_ns
        .zip(no_sampler_cpu_ns)
        .map(|(sampled, no_sampler)| sampled - no_sampler);
    let sampler_peak_private_delta = peak
        .private_bytes
        .zip(sampled_before.and_then(|sample| sample.private_bytes))
        .map(|(peak, before)| peak as i64 - before as i64);
    let sampler_peak_rss_delta = peak
        .rss_bytes
        .zip(sampled_before.and_then(|sample| sample.rss_bytes))
        .map(|(peak, before)| peak as i64 - before as i64);
    json!({
        "window_ms_per_control": WINDOW.as_millis(),
        "control_order": ["no_sampler", "5ms_process_sampler"],
        "no_sampler_process_cpu_delta_ns": no_sampler_cpu_ns,
        "sampler_process_cpu_delta_ns": sampled_cpu_ns,
        "estimated_incremental_sampler_cpu_ns": incremental_cpu_ns,
        "sampler_cpu_overhead_percent_of_window": incremental_cpu_ns
            .map(|value| value as f64 / (WINDOW.as_nanos() as f64) * 100.0),
        "sampler_control_sample_count": peak.samples,
        "sampler_control_peak_private_delta_bytes": sampler_peak_private_delta,
        "sampler_control_peak_working_set_delta_bytes": sampler_peak_rss_delta,
        "measurement_note": "paired 250ms idle windows after build; process-wide CPU deltas include scheduler noise and estimate only the added 5ms polling cost"
    })
}

fn index_build_memory_profile_json(
    profile: &crate::global_index::filename_search::FilenameIndexBuildMemoryProfile,
) -> JsonValue {
    serde_json::to_value(profile).expect("serialize build memory profile")
}

#[cfg(target_os = "windows")]
fn native_process_sample() -> Option<NativeProcessSample> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    let process = unsafe { GetCurrentProcess() };
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let memory_ok = unsafe {
        GetProcessMemoryInfo(
            process,
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let cpu_ok =
        unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) };
    if memory_ok == 0 || cpu_ok == 0 {
        return None;
    }
    let filetime =
        |value: FILETIME| (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime);
    let cpu_time_100ns = filetime(kernel).saturating_add(filetime(user));
    Some(NativeProcessSample {
        rss_bytes: Some(counters.WorkingSetSize as u64),
        private_bytes: Some(counters.PrivateUsage as u64),
        cpu_time_ns: Some(cpu_time_100ns.saturating_mul(100)),
        pageins: None,
    })
}

#[cfg(target_os = "macos")]
fn native_process_sample() -> Option<NativeProcessSample> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v4>::zeroed();
    let result = unsafe {
        libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V4,
            usage.as_mut_ptr().cast(),
        )
    };
    if result != 0 {
        return None;
    }
    let usage = unsafe { usage.assume_init() };
    Some(NativeProcessSample {
        rss_bytes: Some(usage.ri_resident_size),
        private_bytes: Some(usage.ri_phys_footprint),
        cpu_time_ns: Some(usage.ri_user_time.saturating_add(usage.ri_system_time)),
        pageins: Some(usage.ri_pageins),
    })
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn native_process_sample() -> Option<NativeProcessSample> {
    None
}

fn native_sample_json(sample: Option<NativeProcessSample>) -> JsonValue {
    let private_metric = if cfg!(target_os = "windows") {
        "PROCESS_MEMORY_COUNTERS_EX::PrivateUsage"
    } else if cfg!(target_os = "macos") {
        "rusage_info_v4::ri_phys_footprint"
    } else {
        "unavailable"
    };
    json!({
        "working_set_or_resident_bytes": sample.and_then(|value| value.rss_bytes),
        "private_committed_or_footprint_bytes": sample.and_then(|value| value.private_bytes),
        "private_metric": private_metric,
        "cumulative_process_cpu_ns": sample.and_then(|value| value.cpu_time_ns),
        "pageins": sample.and_then(|value| value.pageins)
    })
}

fn sample_private_delta(
    before: Option<NativeProcessSample>,
    after: Option<NativeProcessSample>,
) -> Option<i64> {
    Some(after?.private_bytes? as i64 - before?.private_bytes? as i64)
}

fn sample_rss_delta(
    before: Option<NativeProcessSample>,
    after: Option<NativeProcessSample>,
) -> Option<i64> {
    Some(after?.rss_bytes? as i64 - before?.rss_bytes? as i64)
}

fn sqlite_cache_used_bytes(connection: &Connection) -> Option<u64> {
    let mut current = 0;
    let mut highwater = 0;
    let result = unsafe {
        rusqlite::ffi::sqlite3_db_status(
            connection.handle(),
            rusqlite::ffi::SQLITE_DBSTATUS_CACHE_USED,
            &mut current,
            &mut highwater,
            0,
        )
    };
    (result == rusqlite::ffi::SQLITE_OK).then_some(current.max(0) as u64)
}

fn measure_idle_resources(seconds: u64) -> JsonValue {
    if seconds == 0 {
        return json!({ "measured": false, "reason": "30-minute idle capture was not requested for this fixture scale" });
    }
    assert_eq!(seconds, 1_800);
    let started = Instant::now();
    let baseline = native_process_sample().expect("native idle process metrics available");
    let mut samples = Vec::with_capacity(30);
    for minute in 1..=30 {
        std::thread::sleep(std::time::Duration::from_secs(60));
        let sample = native_process_sample().expect("native idle process metrics remain available");
        samples.push(json!({
            "minute": minute,
            "working_set_or_resident_bytes": sample.rss_bytes,
            "private_committed_or_footprint_bytes": sample.private_bytes,
            "cumulative_process_cpu_ns": sample.cpu_time_ns,
            "pageins": sample.pageins
        }));
        println!("Issue #494 idle resource sample {minute}/30");
    }
    let elapsed_seconds = started.elapsed().as_secs_f64();
    let final_sample = native_process_sample().expect("native final idle metrics available");
    let cpu_delta_ns = final_sample
        .cpu_time_ns
        .unwrap_or_default()
        .saturating_sub(baseline.cpu_time_ns.unwrap_or_default());
    json!({
        "measured": true,
        "requested_seconds": seconds,
        "actual_wall_seconds": round_ms(elapsed_seconds * 1000.0) / 1000.0,
        "normalized_one_core_cpu_percent": round_ms(cpu_delta_ns as f64 / (elapsed_seconds * 10_000_000.0)),
        "cpu_time_delta_ns": cpu_delta_ns,
        "baseline": native_sample_json(Some(baseline)),
        "final": native_sample_json(Some(final_sample)),
        "minute_samples": samples,
        "sampling_method": "native process CPU and memory counters once per minute; test thread sleeps between samples"
    })
}
