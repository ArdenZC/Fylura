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
    diagnostic_filename_trigram_candidates_sql, diagnostic_search_tier_sql,
};
use rusqlite::types::Value as SqlValue;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::env;
use std::time::Instant;

const FILENAME_CANDIDATE_WARMUPS: usize = 5;
const FILENAME_CANDIDATE_SAMPLES: usize = 30;
const FILENAME_CANDIDATE_WRITE_BATCHES: [usize; 2] = [512, 5_000];

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

    let rss_before_index = process_resident_bytes();
    db.filename_search.enable_for_test();
    db.filename_search
        .rebuild_now_for_test()
        .expect("synchronously build filename candidate for paired measurement");
    let rss_after_index = process_resident_bytes();
    let index_diagnostics = db.filename_search.diagnostics();
    assert!(
        index_diagnostics.ready,
        "candidate index must be ready before sampling"
    );
    assert_eq!(index_diagnostics.index_entries, entries as usize);

    let fts_sql = diagnostic_search_tier_sql("fts");
    let trigram_sql = diagnostic_filename_trigram_candidates_sql();
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
            "candidate_trigram_rowids_and_extension_match",
            "report",
            trigram_sql.to_string(),
            vec![
                SqlValue::Text("\"report\"".into()),
                SqlValue::Text("report".into()),
            ],
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
        .filter_map(|detail| detail.as_str())
        .collect::<Vec<_>>();
    assert!(
        index_build_details
            .iter()
            .any(|detail| detail.contains("COVERING INDEX idx_global_entries_active_name_order")),
        "compact projection builder must scan the existing covering name-order index: {index_build_details:?}"
    );
    plan_records.push(index_build_plan);

    let candidate_cases = vec![
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

    let mut search_records = Vec::new();
    for case in &candidate_cases {
        search_records.push(measure_search_pair(&db, &context, case));
        search_records.push(measure_snapshot_pair(&db, &context, case));
    }

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
    let rss_after_queries_and_writes = process_resident_bytes();
    db.filename_search.set_enabled_for_test(true);
    let diagnostics_after = db.filename_search.diagnostics();
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "issue494_filename_first_native_preflight",
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
            "estimated_retained_index_bytes": index_diagnostics.index_private_bytes,
            "index_build_peak_bytes": index_diagnostics.index_build_peak_bytes,
            "rss_before_index_build_bytes": rss_before_index,
            "rss_after_index_build_bytes": rss_after_index,
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
            "retrieval": "SQLite FTS5 trigram postings return rowids only; compact immutable projection supplies mtime and stable-id keys; a bounded Rust heap keeps the complete top 4096; path-only matches use the existing indexed trigram table after filename candidates",
            "ranked_fuzzy_hit_count_last_query": diagnostics_after.trigram_candidate_rows,
            "index_overlay_rows_last_query": diagnostics_after.overlay_rows,
            "plans": plan_records
        },
        "paired_search_and_snapshot": search_records,
        "paired_batch_writes": paired_batch_writes,
        "post_write_database": post_write_metrics,
        "rss_after_queries_and_writes_bytes": rss_after_queries_and_writes,
        "fallback_counters": {
            "not_ready": diagnostics_after.fallback_not_ready,
            "busy": diagnostics_after.fallback_busy,
            "untracked_change": diagnostics_after.fallback_untracked_change,
            "source_change": diagnostics_after.fallback_source_change,
            "overlay_limit": diagnostics_after.fallback_overlay_limit
        },
        "correctness": "every candidate sample matched the independent filename-first ID order; fields for IDs shared by the two routes matched except rank; baseline order differences are recorded rather than rejected; Repository Snapshot source health, source revision and index status matched pairwise",
        "limitations": [
            "process RSS includes the Rust test harness and SQLite page cache; it is not private-bytes accounting",
            "this preflight is synthetic and does not measure installed-app GUI or live filesystem indexing",
            "the in-memory candidate rebuilds after process restart because no persistent sidecar is used"
        ]
    }));
}

fn measure_search_pair(db: &Database, context: &JsonValue, case: &CandidateCase) -> JsonValue {
    let baseline_expected = run_search_route(db, false, case);
    let candidate_expected = run_search_route(db, true, case);
    assert_shared_result_fields_equal_except_rank(&baseline_expected, &candidate_expected);
    assert_result_ids(&candidate_expected, case);
    let baseline_candidate_order_equal = result_ids(&baseline_expected) == case.expected_ids;
    let paired_result_id_sets_equal = same_result_id_set(&baseline_expected, &candidate_expected);
    let candidate_diagnostics = db.filename_search.diagnostics();

    let mut samples = [
        Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES),
        Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES),
    ];
    for iteration in 0..(FILENAME_CANDIDATE_WARMUPS + FILENAME_CANDIDATE_SAMPLES) {
        for position in 0..2 {
            let candidate = (iteration + position) % 2 == 0;
            let (actual, elapsed) = timed(|| run_search_route(db, candidate, case));
            assert_shared_result_fields_equal_except_rank(&baseline_expected, &actual);
            if candidate {
                assert_result_ids(&actual, case);
            }
            if iteration >= FILENAME_CANDIDATE_WARMUPS {
                samples[usize::from(candidate)].push(elapsed);
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
        "baseline_sqlite": summarize_samples(&samples[0]),
        "candidate": summarize_samples(&samples[1]),
        "baseline_raw_ms": samples[0],
        "candidate_raw_ms": samples[1],
        "candidate_trigram_rows": candidate_diagnostics.trigram_candidate_rows,
        "shared_result_fields_equal_except_rank": true,
        "paired_result_id_sets_equal": paired_result_id_sets_equal,
        "baseline_candidate_order_equal": baseline_candidate_order_equal,
        "candidate_independent_oracle_ids_and_order_match": true,
        "ranking_contract": "only the candidate is compared with the filename-first independent oracle; baseline order and the two page ID sets may differ",
        "pair_order": "alternated by sample"
    })
}

fn measure_snapshot_pair(db: &Database, context: &JsonValue, case: &CandidateCase) -> JsonValue {
    let baseline_expected = run_snapshot_route(db, false, case);
    let candidate_expected = run_snapshot_route(db, true, case);
    assert_snapshot_equal(&baseline_expected, &candidate_expected);
    assert_result_ids(&candidate_expected.results, case);
    let baseline_candidate_order_equal =
        result_ids(&baseline_expected.results) == case.expected_ids;
    let paired_result_id_sets_equal =
        same_result_id_set(&baseline_expected.results, &candidate_expected.results);
    let candidate_diagnostics = db.filename_search.diagnostics();

    let mut samples = [
        Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES),
        Vec::with_capacity(FILENAME_CANDIDATE_SAMPLES),
    ];
    for iteration in 0..(FILENAME_CANDIDATE_WARMUPS + FILENAME_CANDIDATE_SAMPLES) {
        for position in 0..2 {
            let candidate = (iteration + position) % 2 == 0;
            let (actual, elapsed) = timed(|| run_snapshot_route(db, candidate, case));
            assert_snapshot_equal(&baseline_expected, &actual);
            if candidate {
                assert_result_ids(&actual.results, case);
            }
            if iteration >= FILENAME_CANDIDATE_WARMUPS {
                samples[usize::from(candidate)].push(elapsed);
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
        "baseline_sqlite": summarize_samples(&samples[0]),
        "candidate": summarize_samples(&samples[1]),
        "baseline_raw_ms": samples[0],
        "candidate_raw_ms": samples[1],
        "candidate_trigram_rows": candidate_diagnostics.trigram_candidate_rows,
        "shared_result_fields_equal_except_rank": true,
        "paired_result_id_sets_equal": paired_result_id_sets_equal,
        "baseline_candidate_order_equal": baseline_candidate_order_equal,
        "source_health_equal": true,
        "source_revision_equal": true,
        "index_status_equal": true,
        "candidate_independent_oracle_ids_and_order_match": true,
        "ranking_contract": "only the candidate is compared with the filename-first independent oracle; baseline order and the two page ID sets may differ",
        "pair_order": "alternated by sample"
    })
}

fn run_search_route(
    db: &Database,
    candidate: bool,
    case: &CandidateCase,
) -> Vec<GlobalSearchResult> {
    db.filename_search.set_enabled_for_test(candidate);
    db.search_global_entries(case.query, SEARCH_RESULT_LIMIT, case.offset)
        .expect("run paired public Global Search route")
}

fn run_snapshot_route(
    db: &Database,
    candidate: bool,
    case: &CandidateCase,
) -> crate::global_index::repository::GlobalSearchSnapshot {
    db.filename_search.set_enabled_for_test(candidate);
    db.search_global_entries_snapshot(case.query, SEARCH_RESULT_LIMIT, case.offset)
        .expect("run paired complete Repository Snapshot route")
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

fn process_resident_bytes() -> Option<u64> {
    let pid = sysinfo::get_current_pid().ok()?;
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).map(sysinfo::Process::memory)
}
