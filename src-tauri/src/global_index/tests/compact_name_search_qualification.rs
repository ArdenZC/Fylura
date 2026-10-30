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
fn compact_raw_update_samples(db: &Database, ids: &[String], repetitions: usize) -> Vec<f64> {
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
fn compact_writer_inputs(count: usize) -> Vec<crate::global_index::models::GlobalEntryInput> {
    (0..count)
        .map(|index| {
            let name = format!("feed-cost-{index:05}.bin");
            crate::global_index::models::GlobalEntryInput {
                volume_id: "gv_compact_feed_write".into(),
                platform_file_id: format!("compact-feed-id-{index:05}"),
                parent_platform_file_id: "compact-feed-root".into(),
                path: format!(r"F:\feed-cost\{name}"),
                name,
                extension: "bin".into(),
                is_directory: false,
                size: 1_024 + index as i64,
                created_at_fs: Some(1_600_000_000 + index as i64),
                modified_at_fs: Some(1_700_000_000 + index as i64),
                file_attributes: 0,
                is_hidden: false,
                is_system: false,
                source_provider: "synthetic_feed_writer_qualification".into(),
                last_seen_at: 1_700_000_000,
            }
        })
        .collect()
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_clone_database(source: &Path, destination: &Path) {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).expect("create paired writer database directory");
    }
    let connection = Connection::open(source).expect("open paired writer clone source");
    connection
        .execute("VACUUM INTO ?1", [destination.to_string_lossy().as_ref()])
        .expect("create transactionally consistent paired writer database");
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_prepare_writer_fixture(
    db: &Database,
    inputs: &[crate::global_index::models::GlobalEntryInput],
) {
    let mut volume = test_volume();
    volume.id = "gv_compact_feed_write".into();
    volume.stable_volume_id = "stable-compact-feed-write".into();
    volume.mount_path = r"F:\feed-cost".into();
    volume.display_name = "Feed write qualification".into();
    volume.enabled = false;
    db.upsert_global_volume(&volume)
        .expect("create disabled source for writer qualification");
    assert_eq!(
        db.upsert_global_entries_batch(inputs)
            .expect("seed source-faithful production writer fixtures"),
        inputs.len()
    );
}

#[cfg(feature = "compact-name-search-candidate")]
type CompactWriterSamples = (Vec<f64>, Vec<f64>, Vec<f64>, Vec<u64>, Vec<u64>);

#[cfg(feature = "compact-name-search-candidate")]
fn compact_paired_writer_samples(
    baseline: &Database,
    candidate: &Database,
    entries: &mut [crate::global_index::models::GlobalEntryInput],
    warmup_pairs: usize,
    measured_pairs: usize,
) -> CompactWriterSamples {
    let mut baseline_samples = Vec::with_capacity(measured_pairs);
    let mut candidate_samples = Vec::with_capacity(measured_pairs);
    let mut paired_percent_deltas = Vec::with_capacity(measured_pairs);
    let mut baseline_vm_steps = Vec::with_capacity(measured_pairs);
    let mut candidate_vm_steps = Vec::with_capacity(measured_pairs);
    for pair in 0..warmup_pairs + measured_pairs {
        let pair_base = 1_710_000_000_i64 + pair as i64 * (entries.len() as i64 + 1);
        for (index, entry) in entries.iter_mut().enumerate() {
            entry.modified_at_fs = Some(pair_base + index as i64);
        }
        let (baseline_elapsed, candidate_elapsed) = if pair.is_multiple_of(2) {
            let baseline_sample = compact_measure_production_batch(baseline, entries);
            let candidate_sample = compact_measure_production_batch(candidate, entries);
            (baseline_sample, candidate_sample)
        } else {
            let candidate_sample = compact_measure_production_batch(candidate, entries);
            let baseline_sample = compact_measure_production_batch(baseline, entries);
            (baseline_sample, candidate_sample)
        };
        if pair >= warmup_pairs {
            baseline_samples.push(baseline_elapsed.0);
            candidate_samples.push(candidate_elapsed.0);
            paired_percent_deltas.push(
                ((candidate_elapsed.0 - baseline_elapsed.0) / baseline_elapsed.0.max(0.001))
                    * 100.0,
            );
            baseline_vm_steps.push(baseline_elapsed.1);
            candidate_vm_steps.push(candidate_elapsed.1);
        }
    }
    (
        baseline_samples,
        candidate_samples,
        paired_percent_deltas,
        baseline_vm_steps,
        candidate_vm_steps,
    )
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_measure_production_batch(
    db: &Database,
    entries: &[crate::global_index::models::GlobalEntryInput],
) -> (f64, u64) {
    let started = Instant::now();
    let changed = db
        .upsert_global_entries_batch(entries)
        .expect("run real transactional Global Index batch writer");
    let elapsed = started.elapsed().as_secs_f64() * 1_000.0;
    assert_eq!(changed, entries.len());
    (
        elapsed,
        crate::global_index::repository::last_global_entry_batch_vm_steps(),
    )
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_wal_file_bytes(database_path: &Path) -> u64 {
    fs::metadata(PathBuf::from(format!("{}-wal", database_path.display())))
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_wal_frame_snapshot(db: &Database) -> JsonValue {
    let Ok(conn) = db.conn() else {
        return JsonValue::Null;
    };
    let wal_frames = conn.query_row("PRAGMA wal_checkpoint(NOOP)", [], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    });
    let page_size: i64 = conn
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .unwrap_or_default();
    wal_frames
        .map(|(busy, log_frames, checkpointed_frames)| {
            json!({
                "busy": busy,
                "wal_log_frames": log_frames,
                "checkpointed_frames": checkpointed_frames,
                "page_size_bytes": page_size,
                "wal_log_bytes_estimate": log_frames.max(0) as u64 * page_size.max(0) as u64
            })
        })
        .unwrap_or(JsonValue::Null)
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_feed_ledger(db: &Database) -> JsonValue {
    let Ok(conn) = db.conn() else {
        return JsonValue::Null;
    };
    conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(event_count),0),
                COALESCE((SELECT change_floor FROM compact_name_search_state WHERE singleton=1),0),
                COALESCE((SELECT base_generation FROM compact_name_search_state WHERE singleton=1),-1)
         FROM compact_name_search_changes",
        [],
        |row| {
            Ok(json!({
                "outbox_rows": row.get::<_, i64>(0)?,
                "represented_entry_changes": row.get::<_, i64>(1)?,
                "change_floor": row.get::<_, i64>(2)?,
                "base_generation": row.get::<_, i64>(3)?
            }))
        },
    )
    .unwrap_or(JsonValue::Null)
}

#[cfg(feature = "compact-name-search-candidate")]
#[derive(Default)]
struct CompactWriteHookCounters {
    global_entry_inserts: AtomicU64,
    global_entry_updates: AtomicU64,
    global_entry_deletes: AtomicU64,
    feed_inserts: AtomicU64,
    feed_deletes: AtomicU64,
    feed_state_updates: AtomicU64,
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_install_write_hook(db: &Database) -> Arc<CompactWriteHookCounters> {
    let counters = Arc::new(CompactWriteHookCounters::default());
    let callback_counters = Arc::clone(&counters);
    db.install_test_update_hook(Arc::new(move |action, database, table, _rowid| {
        if database != "main" {
            return;
        }
        match (table, action) {
            ("global_entries", rusqlite::hooks::Action::SQLITE_INSERT) => {
                callback_counters
                    .global_entry_inserts
                    .fetch_add(1, AtomicOrdering::Relaxed);
            }
            ("global_entries", rusqlite::hooks::Action::SQLITE_UPDATE) => {
                callback_counters
                    .global_entry_updates
                    .fetch_add(1, AtomicOrdering::Relaxed);
            }
            ("global_entries", rusqlite::hooks::Action::SQLITE_DELETE) => {
                callback_counters
                    .global_entry_deletes
                    .fetch_add(1, AtomicOrdering::Relaxed);
            }
            ("compact_name_search_changes", rusqlite::hooks::Action::SQLITE_INSERT) => {
                callback_counters
                    .feed_inserts
                    .fetch_add(1, AtomicOrdering::Relaxed);
            }
            ("compact_name_search_changes", rusqlite::hooks::Action::SQLITE_DELETE) => {
                callback_counters
                    .feed_deletes
                    .fetch_add(1, AtomicOrdering::Relaxed);
            }
            ("compact_name_search_state", rusqlite::hooks::Action::SQLITE_UPDATE) => {
                callback_counters
                    .feed_state_updates
                    .fetch_add(1, AtomicOrdering::Relaxed);
            }
            _ => {}
        }
    }))
    .expect("instrument all writer pool connections with SQLite update hooks");
    counters
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_reset_write_hook(counters: &CompactWriteHookCounters) {
    for counter in [
        &counters.global_entry_inserts,
        &counters.global_entry_updates,
        &counters.global_entry_deletes,
        &counters.feed_inserts,
        &counters.feed_deletes,
        &counters.feed_state_updates,
    ] {
        counter.store(0, AtomicOrdering::Relaxed);
    }
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_write_hook_summary(counters: &CompactWriteHookCounters) -> JsonValue {
    json!({
        "global_entries_insert": counters.global_entry_inserts.load(AtomicOrdering::Relaxed),
        "global_entries_update": counters.global_entry_updates.load(AtomicOrdering::Relaxed),
        "global_entries_delete": counters.global_entry_deletes.load(AtomicOrdering::Relaxed),
        "change_feed_insert": counters.feed_inserts.load(AtomicOrdering::Relaxed),
        "change_feed_delete": counters.feed_deletes.load(AtomicOrdering::Relaxed),
        "change_feed_state_update": counters.feed_state_updates.load(AtomicOrdering::Relaxed)
    })
}

#[cfg(feature = "compact-name-search-candidate")]
fn compact_vm_step_summary(samples: &[u64]) -> JsonValue {
    let mut sorted = samples
        .iter()
        .map(|value| *value as f64)
        .collect::<Vec<_>>();
    sorted.sort_by(f64::total_cmp);
    json!({
        "sample_count": sorted.len(),
        "p50": round_ms(quantile(&sorted, 0.50)),
        "p95": round_ms(quantile(&sorted, 0.95)),
        "p99": round_ms(quantile(&sorted, 0.99)),
        "max": sorted.last().copied().unwrap_or_default()
    })
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
    drop(population_conn);

    let baseline_write_path = PathBuf::from(format!("{}.write-baseline", path.display()));
    let candidate_write_path = PathBuf::from(format!("{}.write-candidate", path.display()));
    let _baseline_write_cleanup = BenchmarkCleanup(baseline_write_path.clone());
    let _candidate_write_cleanup = BenchmarkCleanup(candidate_write_path.clone());
    compact_clone_database(&path, &baseline_write_path);
    compact_clone_database(&path, &candidate_write_path);
    let baseline_write_db =
        Database::open(&baseline_write_path).expect("open matched no-feed writer database clone");
    let candidate_write_db =
        Database::open(&candidate_write_path).expect("open matched feed writer database clone");
    let mut write_inputs = compact_writer_inputs(5_000);
    compact_prepare_writer_fixture(&baseline_write_db, &write_inputs);
    compact_prepare_writer_fixture(&candidate_write_db, &write_inputs);
    candidate_write_db
        .enable_compact_name_search_for_test()
        .expect("acquire owner lease and install batched change feed");

    let baseline_wal_512_before = compact_wal_file_bytes(&baseline_write_path);
    let candidate_wal_512_before = compact_wal_file_bytes(&candidate_write_path);
    let (
        baseline_512_samples,
        candidate_512_samples,
        paired_512_percent_deltas,
        baseline_512_vm_steps,
        candidate_512_vm_steps,
    ) = compact_paired_writer_samples(
        &baseline_write_db,
        &candidate_write_db,
        &mut write_inputs[..512],
        WARMUP_SAMPLES,
        WARM_SAMPLES,
    );
    let baseline_wal_512_after = compact_wal_file_bytes(&baseline_write_path);
    let candidate_wal_512_after = compact_wal_file_bytes(&candidate_write_path);
    let candidate_512_ledger = compact_feed_ledger(&candidate_write_db);
    let baseline_512_wal_frames = compact_wal_frame_snapshot(&baseline_write_db);
    let candidate_512_wal_frames = compact_wal_frame_snapshot(&candidate_write_db);

    let baseline_wal_5000_before = compact_wal_file_bytes(&baseline_write_path);
    let candidate_wal_5000_before = compact_wal_file_bytes(&candidate_write_path);
    let (
        baseline_5000_samples,
        candidate_5000_samples,
        paired_5000_percent_deltas,
        baseline_5000_vm_steps,
        candidate_5000_vm_steps,
    ) = compact_paired_writer_samples(
        &baseline_write_db,
        &candidate_write_db,
        &mut write_inputs,
        WARMUP_SAMPLES,
        WARM_SAMPLES,
    );
    let baseline_wal_5000_after = compact_wal_file_bytes(&baseline_write_path);
    let candidate_wal_5000_after = compact_wal_file_bytes(&candidate_write_path);
    let candidate_5000_ledger = compact_feed_ledger(&candidate_write_db);
    let baseline_5000_wal_frames = compact_wal_frame_snapshot(&baseline_write_db);
    let candidate_5000_wal_frames = compact_wal_frame_snapshot(&candidate_write_db);

    // Hooks are enabled only for separate diagnostic transactions so their
    // callback cost cannot contaminate the paired latency samples.
    let baseline_hook_counters = compact_install_write_hook(&baseline_write_db);
    let candidate_hook_counters = compact_install_write_hook(&candidate_write_db);
    let mut hook_probe_512 = write_inputs[..512].to_vec();
    for (index, entry) in hook_probe_512.iter_mut().enumerate() {
        entry.modified_at_fs = Some(1_720_000_000 + index as i64);
    }
    compact_reset_write_hook(&baseline_hook_counters);
    compact_reset_write_hook(&candidate_hook_counters);
    let _ = compact_measure_production_batch(&baseline_write_db, &hook_probe_512);
    let _ = compact_measure_production_batch(&candidate_write_db, &hook_probe_512);
    let baseline_512_hook_counts = compact_write_hook_summary(&baseline_hook_counters);
    let candidate_512_hook_counts = compact_write_hook_summary(&candidate_hook_counters);

    let mut hook_probe_5000 = write_inputs.clone();
    for (index, entry) in hook_probe_5000.iter_mut().enumerate() {
        entry.modified_at_fs = Some(1_730_000_000 + index as i64);
    }
    compact_reset_write_hook(&baseline_hook_counters);
    compact_reset_write_hook(&candidate_hook_counters);
    let _ = compact_measure_production_batch(&baseline_write_db, &hook_probe_5000);
    let _ = compact_measure_production_batch(&candidate_write_db, &hook_probe_5000);
    let baseline_5000_hook_counts = compact_write_hook_summary(&baseline_hook_counters);
    let candidate_5000_hook_counts = compact_write_hook_summary(&candidate_hook_counters);
    drop((baseline_write_db, candidate_write_db));

    db.enable_compact_name_search_for_test()
        .expect("acquire the production candidate owner lease for Search qualification");
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
    let _overlay_update_samples = compact_raw_update_samples(&db, &overlay_ids, 1);
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
    let overhead_512_p50_percent = quantile(
        &{
            let mut values = paired_512_percent_deltas.clone();
            values.sort_by(f64::total_cmp);
            values
        },
        0.50,
    );
    let overhead_512_p95_percent = quantile(
        &{
            let mut values = paired_512_percent_deltas.clone();
            values.sort_by(f64::total_cmp);
            values
        },
        0.95,
    );
    let overhead_512_p99_percent = quantile(
        &{
            let mut values = paired_512_percent_deltas.clone();
            values.sort_by(f64::total_cmp);
            values
        },
        0.99,
    );
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "compact_name_search_write_cost",
        "context": context,
        "batch_rows": 512,
        "without_change_feed": summarize_samples(&baseline_512_samples),
        "with_change_feed": summarize_samples(&candidate_512_samples),
        "writer_vdbe_vm_steps": {
            "no_feed": compact_vm_step_summary(&baseline_512_vm_steps),
            "feed": compact_vm_step_summary(&candidate_512_vm_steps)
        },
        "sqlite_update_hook_diagnostic_transaction": {
            "no_feed": baseline_512_hook_counts,
            "feed": candidate_512_hook_counts
        },
        "paired_order": "alternating_no_feed_feed_and_feed_no_feed",
        "warmup_pairs": WARMUP_SAMPLES,
        "measured_pairs": WARM_SAMPLES,
        "p50_overhead_percent": round_ms(overhead_512_p50_percent),
        "p95_overhead_percent": round_ms(overhead_512_p95_percent),
        "p99_overhead_percent": round_ms(overhead_512_p99_percent),
        "wal_file_bytes_before_after": {
            "no_feed": [baseline_wal_512_before, baseline_wal_512_after],
            "feed": [candidate_wal_512_before, candidate_wal_512_after]
        },
        "wal_frames_after_measured_runs": {
            "no_feed": baseline_512_wal_frames,
            "feed": candidate_512_wal_frames
        },
        "feed_ledger": candidate_512_ledger,
        "sample_count": baseline_512_samples.len()
    }));
    let overhead_5000_p50_percent = quantile(
        &{
            let mut values = paired_5000_percent_deltas.clone();
            values.sort_by(f64::total_cmp);
            values
        },
        0.50,
    );
    let overhead_5000_p95_percent = quantile(
        &{
            let mut values = paired_5000_percent_deltas.clone();
            values.sort_by(f64::total_cmp);
            values
        },
        0.95,
    );
    let overhead_5000_p99_percent = quantile(
        &{
            let mut values = paired_5000_percent_deltas.clone();
            values.sort_by(f64::total_cmp);
            values
        },
        0.99,
    );
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "compact_name_search_write_cost",
        "context": context,
        "batch_rows": 5000,
        "without_change_feed": summarize_samples(&baseline_5000_samples),
        "with_change_feed": summarize_samples(&candidate_5000_samples),
        "writer_vdbe_vm_steps": {
            "no_feed": compact_vm_step_summary(&baseline_5000_vm_steps),
            "feed": compact_vm_step_summary(&candidate_5000_vm_steps)
        },
        "sqlite_update_hook_diagnostic_transaction": {
            "no_feed": baseline_5000_hook_counts,
            "feed": candidate_5000_hook_counts
        },
        "paired_order": "alternating_no_feed_feed_and_feed_no_feed",
        "warmup_pairs": WARMUP_SAMPLES,
        "measured_pairs": WARM_SAMPLES,
        "p50_overhead_percent": round_ms(overhead_5000_p50_percent),
        "p95_overhead_percent": round_ms(overhead_5000_p95_percent),
        "p99_overhead_percent": round_ms(overhead_5000_p99_percent),
        "wal_file_bytes_before_after": {
            "no_feed": [baseline_wal_5000_before, baseline_wal_5000_after],
            "feed": [candidate_wal_5000_before, candidate_wal_5000_after]
        },
        "wal_frames_after_measured_runs": {
            "no_feed": baseline_5000_wal_frames,
            "feed": candidate_5000_wal_frames
        },
        "feed_ledger": candidate_5000_ledger,
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

            let mut search_samples = Vec::with_capacity(WARM_SAMPLES);
            let mut legacy_samples = Vec::with_capacity(WARM_SAMPLES);
            let mut legacy_ids = Vec::new();
            let run_candidate_search = || {
                let started = Instant::now();
                let results = db
                    .search_global_entries(query, SEARCH_RESULT_LIMIT, offset)
                    .expect("run compact native Search");
                let elapsed = started.elapsed().as_secs_f64() * 1_000.0;
                assert_eq!(
                    results.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
                    expected,
                    "compact Search results differ from independent oracle: {query_class} at offset {offset}"
                );
                elapsed
            };
            let run_legacy_search = || {
                let results = search_global_entries_on_connection(
                    &inspection_connection,
                    query,
                    SEARCH_RESULT_LIMIT,
                    offset,
                )
                .expect("run legacy SQLite Search");
                results.into_iter().map(|row| row.id).collect::<Vec<_>>()
            };
            for pair in 0..WARMUP_SAMPLES {
                if pair.is_multiple_of(2) {
                    let _ = run_candidate_search();
                    legacy_ids = run_legacy_search();
                } else {
                    legacy_ids = run_legacy_search();
                    let _ = run_candidate_search();
                }
            }
            let mut paired_search_percent_deltas = Vec::with_capacity(WARM_SAMPLES);
            for pair in 0..WARM_SAMPLES {
                let (candidate_elapsed, legacy_elapsed) = if pair.is_multiple_of(2) {
                    let candidate_elapsed = run_candidate_search();
                    let started = Instant::now();
                    legacy_ids = run_legacy_search();
                    let legacy_elapsed = started.elapsed().as_secs_f64() * 1_000.0;
                    (candidate_elapsed, legacy_elapsed)
                } else {
                    let started = Instant::now();
                    legacy_ids = run_legacy_search();
                    let legacy_elapsed = started.elapsed().as_secs_f64() * 1_000.0;
                    let candidate_elapsed = run_candidate_search();
                    (candidate_elapsed, legacy_elapsed)
                };
                search_samples.push(candidate_elapsed);
                legacy_samples.push(legacy_elapsed);
                paired_search_percent_deltas.push(
                    ((candidate_elapsed - legacy_elapsed) / legacy_elapsed.max(0.001)) * 100.0,
                );
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
            let paired_candidate_legacy_p50_delta_percent = quantile(
                &{
                    let mut sorted = paired_search_percent_deltas.clone();
                    sorted.sort_by(f64::total_cmp);
                    sorted
                },
                0.50,
            );
            let paired_candidate_legacy_p95_delta_percent = quantile(
                &{
                    let mut sorted = paired_search_percent_deltas.clone();
                    sorted.sort_by(f64::total_cmp);
                    sorted
                },
                0.95,
            );
            let query_diagnostics = db.compact_name_search().diagnostics();
            let passed = if query_class == "high_hit" {
                search_p95 <= WARM_QUERY_P95_LIMIT_MS
                    && snapshot_p95 <= WARM_QUERY_P95_LIMIT_MS
                    && paired_candidate_legacy_p50_delta_percent <= -10.0
                    && paired_candidate_legacy_p95_delta_percent <= -10.0
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
                "paired_candidate_vs_legacy_delta_percent": {
                    "p50": round_ms(paired_candidate_legacy_p50_delta_percent),
                    "p95": round_ms(paired_candidate_legacy_p95_delta_percent)
                },
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
                "high_hit_minimum_paired_speedup_percent": 10.0,
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
                "p95": round_ms(overhead_512_p95_percent),
                "p99": round_ms(overhead_512_p99_percent)
            },
            "5000_rows": {
                "p50": round_ms(overhead_5000_p50_percent),
                "p95": round_ms(overhead_5000_p95_percent),
                "p99": round_ms(overhead_5000_p99_percent)
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
        overhead_512_p50_percent <= 10.0,
        "512-row change-feed write paired p50 overhead was {overhead_512_p50_percent:.2}% (limit 10%)"
    );
    assert!(
        overhead_512_p95_percent <= 10.0,
        "512-row change-feed write paired p95 overhead was {overhead_512_p95_percent:.2}% (limit 10%)"
    );
    assert!(
        overhead_5000_p50_percent <= 10.0,
        "5000-row change-feed write paired p50 overhead was {overhead_5000_p50_percent:.2}% (limit 10%)"
    );
    assert!(
        overhead_5000_p95_percent <= 10.0,
        "5000-row change-feed write paired p95 overhead was {overhead_5000_p95_percent:.2}% (limit 10%)"
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
