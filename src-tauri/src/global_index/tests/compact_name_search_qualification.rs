use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;

#[cfg(feature = "compact-name-search-candidate")]
#[derive(Clone)]
struct CompactOracleRow {
    id: String,
    class: u8,
    modified_at: Option<i64>,
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_oracle_rank(name: &str, extension: &str, path: &str, query: &str) -> Option<u8> {
    let bounded = name.match_indices(query).any(|(start, matched)| {
        let before = name[..start].chars().next_back();
        let after = name[start + matched.len()..].chars().next();
        !before.is_some_and(|character| character.is_alphanumeric() || character == '_')
            && !after.is_some_and(|character| character.is_alphanumeric() || character == '_')
    });
    if name.contains(query) {
        Some(if bounded { 0 } else { 1 })
    } else if extension.contains(query) {
        Some(1)
    } else if path.contains(query) {
        Some(2)
    } else {
        None
    }
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_oracle_window(conn: &Connection, query: &str) -> (Vec<String>, u64) {
    let query = query.to_lowercase();
    let mut statement = conn
        .prepare(
            "SELECT ge.id, ge.name_normalized, ge.extension, ge.path_normalized,
                    ge.modified_at_fs
             FROM global_entries ge
             JOIN global_volumes gv ON gv.id=ge.volume_id
             WHERE gv.enabled=1 AND gv.coverage_pending=0 AND ge.is_stale=0
               AND (instr(ge.name_normalized, ?1)>0
                    OR instr(ge.extension, ?1)>0
                    OR instr(ge.path_normalized, ?1)>0)",
        )
        .expect("prepare complete compact-search oracle");
    let rows = statement
        .query_map([&query], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })
        .expect("read compact-search oracle candidates")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect compact-search oracle candidates");
    let mut ranked = rows
        .into_iter()
        .filter_map(|(id, name, extension, path, modified_at)| {
            compact_oracle_rank(&name, &extension, &path, &query).map(|class| CompactOracleRow {
                id,
                class,
                modified_at,
            })
        })
        .collect::<Vec<_>>();
    let total = ranked.len() as u64;
    ranked.sort_by(|left, right| {
        left.class
            .cmp(&right.class)
            .then_with(|| right.modified_at.cmp(&left.modified_at))
            .then_with(|| left.id.cmp(&right.id))
    });
    ranked.truncate(SEARCH_CANDIDATE_LIMIT);
    (
        ranked.into_iter().map(|row| row.id).collect::<Vec<_>>(),
        total,
    )
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_update_ids(conn: &Connection, count: usize) -> Vec<String> {
    let mut statement = conn
        .prepare(
            "SELECT id FROM global_entries
             WHERE volume_id='gv_test' ORDER BY id COLLATE BINARY LIMIT ?1",
        )
        .expect("prepare modification fixture IDs");
    statement
        .query_map([count as i64], |row| row.get::<_, String>(0))
        .expect("read modification fixture IDs")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect modification fixture IDs")
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_update_samples(db: &Database, ids: &[String], repetitions: usize) -> Vec<f64> {
    let encoded_ids = serde_json::to_string(ids).expect("serialize modification fixture IDs");
    let mut samples = Vec::with_capacity(repetitions);
    for _ in 0..repetitions {
        let mut conn = db.conn().expect("lease modification connection");
        let transaction = conn
            .transaction()
            .expect("begin modification sample transaction");
        let started = Instant::now();
        let changed = transaction
            .execute(
                "UPDATE global_entries
                 SET modified_at_fs=COALESCE(modified_at_fs,0)+1
                 WHERE id IN (SELECT value FROM json_each(?1))",
                [&encoded_ids],
            )
            .expect("update modification sample rows");
        transaction
            .commit()
            .expect("commit modification sample transaction");
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        assert_eq!(changed, ids.len());
    }
    samples
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_relative_overhead_percent(
    baseline: &[f64],
    candidate: &[f64],
    quantile_value: f64,
) -> f64 {
    let mut baseline = baseline.to_vec();
    baseline.sort_by(f64::total_cmp);
    let mut candidate = candidate.to_vec();
    candidate.sort_by(f64::total_cmp);
    ((quantile(&candidate, quantile_value) / quantile(&baseline, quantile_value).max(0.001)) - 1.0)
        * 100.0
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_process_memory_bytes() -> Option<u64> {
    let pid = sysinfo::get_current_pid().ok()?;
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).map(sysinfo::Process::memory)
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_start_rss_peak_sampler() -> (Arc<AtomicBool>, Arc<AtomicU64>, std::thread::JoinHandle<()>)
{
    let pid = sysinfo::get_current_pid().expect("read current process ID");
    let stop = Arc::new(AtomicBool::new(false));
    let peak = Arc::new(AtomicU64::new(compact_process_memory_bytes().unwrap_or(0)));
    let worker_stop = Arc::clone(&stop);
    let worker_peak = Arc::clone(&peak);
    let worker = std::thread::Builder::new()
        .name("compact-search-rss-sampler".into())
        .spawn(move || {
            let mut system = sysinfo::System::new();
            while !worker_stop.load(AtomicOrdering::Acquire) {
                system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
                if let Some(process) = system.process(pid) {
                    worker_peak.fetch_max(process.memory(), AtomicOrdering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        })
        .expect("start process RSS sampler");
    (stop, peak, worker)
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_measure_idle_cpu(duration: Duration) -> JsonValue {
    let pid = sysinfo::get_current_pid().expect("read current process ID");
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    let sample_count = duration.as_secs().max(1);
    let mut samples = Vec::with_capacity(sample_count as usize);
    for _ in 0..sample_count {
        std::thread::sleep(Duration::from_secs(1));
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        if let Some(process) = system.process(pid) {
            samples.push(f64::from(process.cpu_usage()));
        }
    }
    let average = samples.iter().sum::<f64>() / samples.len().max(1) as f64;
    let maximum = samples.iter().copied().fold(0.0f64, f64::max);
    json!({
        "record_type": "compact_name_search_idle_cpu",
        "duration_seconds": duration.as_secs(),
        "sample_count": samples.len(),
        "average_process_cpu_percent": average,
        "maximum_sample_process_cpu_percent": maximum,
        "measurement_scope": "release-test-process-with-mapped-index"
    })
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_sidecar_disk_bytes(database_path: &Path) -> u64 {
    let sidecar = PathBuf::from(format!("{}.compact-name-search", database_path.display()));
    fs::read_dir(sidecar)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

#[cfg(feature = "compact-name-search-candidate")]
#[test]
#[ignore = "native Release qualification; set ZC_COMPACT_NAME_SEARCH_QUALIFICATION_ENTRIES and output path"]
fn global_search_compact_name_search_qualification() {
    assert!(
        std::hint::black_box(!cfg!(debug_assertions)),
        "compact-name-search qualification must run in a native Release test binary"
    );
    let entries = env::var("ZC_COMPACT_NAME_SEARCH_QUALIFICATION_ENTRIES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(100_000);
    assert!(
        [100_000, 200_000, 1_000_000].contains(&entries),
        "qualification entries must be 100000, 200000, or 1000000"
    );
    let context = benchmark_context(entries);
    let output_path = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT").ok();
    if let Some(path) = output_path.as_ref() {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create candidate evidence directory");
        }
        fs::write(path, "").expect("start candidate JSONL artifact");
    }

    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open compact-search qualification database");
    assert!(
        !db.compact_name_search().is_enabled(),
        "leave ZC_COMPACT_NAME_SEARCH unset; the test enables the candidate after fixture setup"
    );
    db.upsert_global_volume(&test_volume())
        .expect("insert qualification volume");
    let (_, generation_ms, population_ms, baseline_insert_samples) =
        populate_synthetic_database(&db, entries);
    let population_conn = db.conn().expect("inspect qualification database");
    assert_eq!(
        population_conn
            .query_row(
                "SELECT COUNT(*) FROM global_entries WHERE volume_id='gv_test'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("count complete candidate fixture") as u64,
        entries
    );
    let modify_512 = compact_update_ids(&population_conn, 512);
    let modify_5000 = compact_update_ids(&population_conn, 5_000);
    drop(population_conn);

    let baseline_512_samples = compact_update_samples(&db, &modify_512, WARM_SAMPLES);
    let baseline_5000_samples = compact_update_samples(&db, &modify_5000, WARM_SAMPLES);
    db.enable_compact_name_search_for_test()
        .expect("install opt-in compact change triggers");
    let candidate_512_samples = compact_update_samples(&db, &modify_512, WARM_SAMPLES);
    let candidate_5000_samples = compact_update_samples(&db, &modify_5000, WARM_SAMPLES);
    let memory_before_index_bytes = compact_process_memory_bytes();
    let (rss_sampler_stop, rss_peak_during_index, rss_sampler) = compact_start_rss_peak_sampler();
    let index_build_started = Instant::now();
    db.rebuild_compact_name_search_for_test()
        .expect("build complete compact filename index");
    let index_build_ms = index_build_started.elapsed().as_secs_f64() * 1_000.0;
    rss_sampler_stop.store(true, AtomicOrdering::Release);
    rss_sampler.join().expect("join process RSS sampler");
    let process_rss_peak_during_index_bytes = rss_peak_during_index.load(AtomicOrdering::Relaxed);
    let memory_after_index_bytes = compact_process_memory_bytes();
    let index_disk_bytes = compact_sidecar_disk_bytes(&path);
    db.compact_name_search().drop_mapping_for_test();
    let cold_load_started = Instant::now();
    let cold_load_results = db
        .search_global_entries("report", SEARCH_RESULT_LIMIT, 0)
        .expect("re-map compact index after dropping the process mapping");
    let cold_remap_search_ms = cold_load_started.elapsed().as_secs_f64() * 1_000.0;
    assert_eq!(cold_load_results.len(), SEARCH_RESULT_LIMIT as usize);

    let overlay_ids = compact_update_ids(&db.conn().expect("read overlay IDs"), 512);
    let _overlay_update_samples = compact_update_samples(&db, &overlay_ids, 1);
    let inspection_connection =
        configured_connection(&path).expect("open independent SQLite oracle connection");

    emit_record(&json!({
        "schema_version": 1,
        "record_type": "compact_name_search_dataset",
        "context": context,
        "entries": entries,
        "native_release": !cfg!(debug_assertions),
        "format_version": 1,
        "index_build_ms": round_ms(index_build_ms),
        "cold_remap_search_ms": round_ms(cold_remap_search_ms),
        "index_sidecar_disk_bytes": index_disk_bytes,
        "process_memory_before_index_bytes": memory_before_index_bytes,
        "process_memory_after_index_bytes": memory_after_index_bytes,
        "process_rss_peak_during_index_bytes": process_rss_peak_during_index_bytes,
        "extra_process_rss_peak_during_index_bytes": memory_before_index_bytes.map(|before| process_rss_peak_during_index_bytes.saturating_sub(before)),
        "synthetic_generation_ms": round_ms(generation_ms),
        "sqlite_population_ms": round_ms(population_ms),
        "sqlite_insert_batch_p95_ms": summarize_samples(&baseline_insert_samples),
        "fixture_profile": "synthetic_global_index_shared_with_historical_sqlite_qualification"
    }));
    let overhead_512_p50_percent =
        compact_relative_overhead_percent(&baseline_512_samples, &candidate_512_samples, 0.50);
    let overhead_512_p95_percent =
        compact_relative_overhead_percent(&baseline_512_samples, &candidate_512_samples, 0.95);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "compact_name_search_write_cost",
        "context": context,
        "batch_rows": 512,
        "without_change_feed": summarize_samples(&baseline_512_samples),
        "with_change_feed": summarize_samples(&candidate_512_samples),
        "p50_overhead_percent": round_ms(overhead_512_p50_percent),
        "p95_overhead_percent": round_ms(overhead_512_p95_percent),
        "sample_count": baseline_512_samples.len()
    }));
    let overhead_5000_p50_percent =
        compact_relative_overhead_percent(&baseline_5000_samples, &candidate_5000_samples, 0.50);
    let overhead_5000_p95_percent =
        compact_relative_overhead_percent(&baseline_5000_samples, &candidate_5000_samples, 0.95);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "compact_name_search_write_cost",
        "context": context,
        "batch_rows": 5000,
        "without_change_feed": summarize_samples(&baseline_5000_samples),
        "with_change_feed": summarize_samples(&candidate_5000_samples),
        "p50_overhead_percent": round_ms(overhead_5000_p50_percent),
        "p95_overhead_percent": round_ms(overhead_5000_p95_percent),
        "sample_count": baseline_5000_samples.len()
    }));

    let mut query_results = Vec::new();
    for (query_class, query) in [
        ("zero_hit", "zzznomatchtoken"),
        ("one_hit", "budget"),
        ("sparse_hit", "report 0000002"),
        ("high_hit", "report"),
    ] {
        let (oracle_window, oracle_match_count) =
            compact_oracle_window(&inspection_connection, query);
        let oracle_source_sha = context["actual_head"].as_str().unwrap_or_default();
        assert!(
            oracle_window.len() <= SEARCH_CANDIDATE_LIMIT,
            "oracle exceeded the complete bounded window"
        );
        for offset in [0u32, 4_016u32] {
            let expected = oracle_window
                .iter()
                .skip(offset as usize)
                .take(SEARCH_RESULT_LIMIT as usize)
                .map(String::as_str)
                .collect::<Vec<_>>();

            for _ in 0..WARMUP_SAMPLES {
                let results = db
                    .search_global_entries(query, SEARCH_RESULT_LIMIT, offset)
                    .expect("warm compact native Search");
                assert_eq!(
                    results.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
                    expected,
                    "compact warm results differ from independent oracle: {query_class} at offset {offset}"
                );
            }
            let mut search_samples = Vec::with_capacity(WARM_SAMPLES);
            for _ in 0..WARM_SAMPLES {
                let started = Instant::now();
                let results = db
                    .search_global_entries(query, SEARCH_RESULT_LIMIT, offset)
                    .expect("run compact native Search");
                search_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
                assert_eq!(
                    results.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
                    expected,
                    "compact Search results differ from independent oracle: {query_class} at offset {offset}"
                );
            }

            let mut legacy_samples = Vec::with_capacity(WARM_SAMPLES);
            let mut legacy_ids = Vec::new();
            for _ in 0..WARMUP_SAMPLES {
                let results = search_global_entries_on_connection(
                    &inspection_connection,
                    query,
                    SEARCH_RESULT_LIMIT,
                    offset,
                )
                .expect("warm legacy SQLite Search");
                legacy_ids = results.into_iter().map(|row| row.id).collect();
            }
            for _ in 0..WARM_SAMPLES {
                let started = Instant::now();
                let results = search_global_entries_on_connection(
                    &inspection_connection,
                    query,
                    SEARCH_RESULT_LIMIT,
                    offset,
                )
                .expect("run legacy SQLite Search");
                legacy_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
                legacy_ids = results.into_iter().map(|row| row.id).collect();
            }
            let legacy_differences = expected
                .iter()
                .zip(legacy_ids.iter())
                .filter(|(expected, actual)| **expected != actual.as_str())
                .count()
                + expected.len().abs_diff(legacy_ids.len());

            for _ in 0..WARMUP_SAMPLES {
                let snapshot = db
                    .search_global_entries_snapshot(query, SEARCH_RESULT_LIMIT, offset)
                    .expect("warm repository Snapshot through compact engine");
                assert_eq!(
                    snapshot
                        .results
                        .iter()
                        .map(|row| row.id.as_str())
                        .collect::<Vec<_>>(),
                    expected,
                    "Snapshot results differ from independent oracle: {query_class} at offset {offset}"
                );
            }
            let mut snapshot_samples = Vec::with_capacity(WARM_SAMPLES);
            for _ in 0..WARM_SAMPLES {
                let started = Instant::now();
                let snapshot = db
                    .search_global_entries_snapshot(query, SEARCH_RESULT_LIMIT, offset)
                    .expect("run repository Snapshot through compact engine");
                snapshot_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
                assert_eq!(
                    snapshot
                        .results
                        .iter()
                        .map(|row| row.id.as_str())
                        .collect::<Vec<_>>(),
                    expected,
                    "Snapshot results differ from independent oracle: {query_class} at offset {offset}"
                );
            }

            let search_summary = summarize_samples(&search_samples);
            let legacy_summary = summarize_samples(&legacy_samples);
            let snapshot_summary = summarize_samples(&snapshot_samples);
            let search_p95 = quantile(
                &{
                    let mut sorted = search_samples.clone();
                    sorted.sort_by(f64::total_cmp);
                    sorted
                },
                0.95,
            );
            let legacy_p95 = quantile(
                &{
                    let mut sorted = legacy_samples.clone();
                    sorted.sort_by(f64::total_cmp);
                    sorted
                },
                0.95,
            );
            let snapshot_p95 = quantile(
                &{
                    let mut sorted = snapshot_samples.clone();
                    sorted.sort_by(f64::total_cmp);
                    sorted
                },
                0.95,
            );
            let query_diagnostics = db.compact_name_search().diagnostics();
            let passed = if query_class == "high_hit" {
                search_p95 <= WARM_QUERY_P95_LIMIT_MS && snapshot_p95 <= WARM_QUERY_P95_LIMIT_MS
            } else {
                search_p95 <= legacy_p95 * 2.0 + 3.0
            };
            emit_record(&json!({
                "schema_version": 1,
                "record_type": "compact_name_search_query_qualification",
                "context": context,
                "source_head_for_oracle": oracle_source_sha,
                "query_class": query_class,
                "query": query,
                "offset": offset,
                "candidate_window": SEARCH_CANDIDATE_LIMIT,
                "oracle_match_count": oracle_match_count,
                "oracle_window_count": oracle_window.len(),
                "expected_page_count": expected.len(),
                "candidate_search": search_summary,
                "legacy_sqlite_search": legacy_summary,
                "repository_snapshot": snapshot_summary,
                "legacy_vs_candidate_id_differences": legacy_differences,
                "name_candidate_count": query_diagnostics.last_name_candidates,
                "path_candidate_count": query_diagnostics.last_path_candidates,
                "overlay_event_count": query_diagnostics.last_overlay_events,
                "candidate_engine_accepted_queries": query_diagnostics.accepted_queries,
                "internal_stage_last_us": {
                    "manifest_and_overlay": query_diagnostics.manifest_overlay_micros,
                    "name_prefilter": query_diagnostics.name_prefilter_micros,
                    "name_scan_and_heap": query_diagnostics.name_scan_heap_micros,
                    "path_scan_and_heap": query_diagnostics.path_scan_heap_micros,
                    "top_k_sort_and_page": query_diagnostics.top_k_sort_page_micros,
                    "page_hydration": query_diagnostics.page_hydration_micros
                },
                "passed": passed
            }));
            assert!(
                passed,
                "native qualification failed for {query_class} at offset {offset}: candidate Search p95={:.3}ms, Snapshot p95={:.3}ms, legacy p95={:.3}ms",
                search_p95,
                snapshot_p95,
                legacy_p95
            );
            query_results.push(json!({
                "query_class": query_class,
                "offset": offset,
                "candidate_search_p95_ms": round_ms(search_p95),
                "legacy_search_p95_ms": round_ms(legacy_p95),
                "snapshot_p95_ms": round_ms(snapshot_p95),
                "passed": passed
            }));
        }
    }

    let diagnostics = db.compact_name_search().diagnostics();
    let idle_cpu_measurement = if entries >= 1_000_000 {
        let measurement = compact_measure_idle_cpu(Duration::from_secs(30 * 60));
        emit_record(&measurement);
        measurement
    } else {
        JsonValue::Null
    };
    let memory_after_queries_bytes = compact_process_memory_bytes();
    let idle_cpu_average = idle_cpu_measurement
        .get("average_process_cpu_percent")
        .and_then(JsonValue::as_f64);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "compact_name_search_qualification_summary",
        "context": context,
        "index_disk_bytes": index_disk_bytes,
        "process_memory_before_index_bytes": memory_before_index_bytes,
        "process_memory_after_index_bytes": memory_after_index_bytes,
        "process_rss_peak_during_index_bytes": process_rss_peak_during_index_bytes,
        "process_memory_after_queries_bytes": memory_after_queries_bytes,
        "extra_process_rss_peak_during_index_bytes": memory_before_index_bytes.map(|before| process_rss_peak_during_index_bytes.saturating_sub(before)),
        "extra_process_rss_after_queries_bytes": memory_before_index_bytes.zip(memory_after_queries_bytes).map(|(before,after)| after.saturating_sub(before)),
        "query_matrix_process_rss_delta_bytes": memory_after_index_bytes.zip(memory_after_queries_bytes).map(|(after_index,after_queries)| after_queries.saturating_sub(after_index)),
        "idle_cpu_measurement": idle_cpu_measurement,
        "change_feed_write_overhead_percent": {
            "512_rows": {
                "p50": round_ms(overhead_512_p50_percent),
                "p95": round_ms(overhead_512_p95_percent)
            },
            "5000_rows": {
                "p50": round_ms(overhead_5000_p50_percent),
                "p95": round_ms(overhead_5000_p95_percent)
            }
        },
        "diagnostics": {
            "queries": diagnostics.queries,
            "accepted_queries": diagnostics.accepted_queries,
            "fallbacks": diagnostics.fallbacks,
            "rebuilds": diagnostics.rebuilds,
            "last_overlay_events": diagnostics.last_overlay_events,
            "last_name_candidates": diagnostics.last_name_candidates,
            "last_path_candidates": diagnostics.last_path_candidates,
            "path_relation_complete": diagnostics.path_relation_complete,
            "last_internal_stage_us": {
                "manifest_and_overlay": diagnostics.manifest_overlay_micros,
                "name_prefilter": diagnostics.name_prefilter_micros,
                "name_scan_and_heap": diagnostics.name_scan_heap_micros,
                "path_scan_and_heap": diagnostics.path_scan_heap_micros,
                "top_k_sort_and_page": diagnostics.top_k_sort_page_micros,
                "page_hydration": diagnostics.page_hydration_micros
            },
            "last_error": diagnostics.last_error
        },
        "query_results": query_results
    }));
    assert!(diagnostics.enabled);
    assert!(diagnostics.accepted_queries > 0);
    assert_eq!(diagnostics.fallbacks, 0);
    assert!(diagnostics.last_error.is_none());
    assert!(
        overhead_512_p95_percent <= 10.0,
        "512-row change-feed write p95 overhead was {overhead_512_p95_percent:.2}% (limit 10%)"
    );
    assert!(
        overhead_5000_p95_percent <= 10.0,
        "5000-row change-feed write p95 overhead was {overhead_5000_p95_percent:.2}% (limit 10%)"
    );
    assert!(
        memory_before_index_bytes
            .zip(memory_after_queries_bytes)
            .is_none_or(|(before, after)| after.saturating_sub(before) <= 128 * 1024 * 1024),
        "query-matrix process RSS growth exceeded 128 MiB"
    );
    assert!(
        idle_cpu_average.is_none_or(|average| average <= 0.5),
        "30-minute idle average process CPU exceeded 0.5%"
    );
    assert!(
        index_disk_bytes <= 128 * 1024 * 1024,
        "compact index disk footprint exceeded 128 MiB: {index_disk_bytes} bytes"
    );
    drop(inspection_connection);
    drop(db);
}
