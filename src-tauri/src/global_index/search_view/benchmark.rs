use super::{memory::MemoryBudget, view::SearchView, worker::supported_candidate_query};
use crate::{
    db::{schema_global_search_changes, Database},
    global_index::search,
};
use rusqlite::{params, Connection};
use serde_json::json;
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const WARMUPS: usize = 5;
const SAMPLES: usize = 30;
const UPDATE_BATCH: usize = 512;
const ALLOWED_ROWS: [usize; 3] = [100_000, 500_000, 1_000_000];

type DiskIoCounters = Option<(u64, u64)>;
type ResourceSample = (u64, f32, f32, usize, DiskIoCounters, DiskIoCounters);
type LatencyPercentiles = (f64, f64, f64);
type WriteBatchMeasurements = (LatencyPercentiles, LatencyPercentiles, bool, bool);

struct BenchmarkDatabase {
    path: PathBuf,
    database: Option<Database>,
    seed_elapsed_ms: f64,
}

impl BenchmarkDatabase {
    fn create(rows: usize) -> Self {
        Self::create_with_settings(rows, true, false, false)
    }

    fn create_with_settings(
        rows: usize,
        feed_enabled: bool,
        worker_enabled: bool,
        disable_wal_autocheckpoint: bool,
    ) -> Self {
        let path = env::temp_dir().join(format!(
            "zen-canvas-search-view-bench-{}-{rows}-{feed_enabled}-{worker_enabled}.db",
            std::process::id()
        ));
        remove_files(&path);
        let database = if disable_wal_autocheckpoint {
            Database::open_with_search_feed_settings_for_write_benchmark(
                &path,
                feed_enabled,
                worker_enabled,
            )
        } else {
            Database::open_with_search_feed_settings_for_test(&path, feed_enabled, worker_enabled)
        }
        .expect("open isolated synthetic database with requested change feed settings");
        let mut connection = database.conn().expect("get synthetic database connection");
        connection
            .execute(
                "INSERT INTO global_volumes (id, platform, stable_volume_id, display_name, mount_path, filesystem_type, drive_kind, enabled, provider, index_status, created_at, updated_at) VALUES ('view-bench-volume', 'windows', 'synthetic-view-bench-volume', 'Synthetic benchmark', 'C:\\Synthetic', 'ntfs', 'fixed', 1, 'windows_mft', 'ready', 1, 1)",
                [],
            )
            .expect("insert isolated synthetic source");

        let seed_started = Instant::now();
        let transaction = connection
            .transaction()
            .expect("begin synthetic entry transaction");
        {
            let mut statement = transaction
                .prepare(
                    "INSERT INTO global_entries (id, volume_id, platform_file_id, parent_platform_file_id, name, name_normalized, path, path_normalized, extension, is_directory, size, created_at_fs, modified_at_fs, file_attributes, is_hidden, is_system, is_stale, source_provider, last_seen_at) VALUES (?1, 'view-bench-volume', ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, 1024, 1700000000, ?9, 0, 0, 0, 0, 'windows_mft', 1)",
                )
                .expect("prepare synthetic entry insert");
            for index in 0..rows {
                let (name, extension) = synthetic_name(index);
                let path = format!(
                    "C:\\Synthetic\\Users\\Owner\\Documents\\Month-03\\Dept{:03}\\{name}",
                    index % 997
                );
                statement
                    .execute(params![
                        format!("view-bench-entry-{index:07}"),
                        format!("synthetic-file-{index:07}"),
                        format!("synthetic-parent-{:07}", index / 20),
                        &name,
                        name.to_ascii_lowercase(),
                        &path,
                        path.to_ascii_lowercase().replace('\\', "/"),
                        extension,
                        index as i64,
                    ])
                    .expect("insert synthetic entry through production schema triggers");
            }
        }
        transaction.commit().expect("commit synthetic entries");
        let seed_elapsed_ms = seed_started.elapsed().as_secs_f64() * 1000.0;
        drop(connection);
        Self {
            path,
            database: Some(database),
            seed_elapsed_ms,
        }
    }

    fn database(&self) -> &Database {
        self.database.as_ref().expect("benchmark database is open")
    }
}

impl Drop for BenchmarkDatabase {
    fn drop(&mut self) {
        self.database.take();
        remove_files(&self.path);
    }
}

fn remove_files(path: &Path) {
    let _ = fs::remove_file(path);
    let mut wal = path.to_path_buf();
    wal.set_extension("db-wal");
    let _ = fs::remove_file(wal);
    let mut shm = path.to_path_buf();
    shm.set_extension("db-shm");
    let _ = fs::remove_file(shm);
}

fn synthetic_name(index: usize) -> (String, &'static str) {
    const EXTENSIONS: [&str; 8] = ["pdf", "docx", "xlsx", "jpg", "jpeg", "txt", "md", "zip"];
    match index % 20 {
        2 => (format!("Annual Report {index:07}.pdf"), "pdf"),
        3 => (format!("Outgoing Invoice {index:07}.xlsx"), "xlsx"),
        4 => (format!("Résumé Archive {index:07}.pdf"), "pdf"),
        _ => {
            let extension = EXTENSIONS[(index / 20) % EXTENSIONS.len()];
            (
                format!("Synthetic Record {index:07}.{extension}"),
                extension,
            )
        }
    }
}

struct ResourceSampler {
    done: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<ResourceSample>>,
}

impl ResourceSampler {
    fn start() -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let monitor_done = done.clone();
        let task = thread::spawn(move || {
            let Ok(pid) = sysinfo::get_current_pid() else {
                return (0, 0.0, 0.0, 0, None, None);
            };
            let mut system = sysinfo::System::new();
            system.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::Some(&[pid]),
                true,
                sysinfo::ProcessRefreshKind::new()
                    .with_memory()
                    .with_disk_usage(),
            );
            let io_start = system.process(pid).map(|process| {
                let usage = process.disk_usage();
                (usage.total_read_bytes, usage.total_written_bytes)
            });
            let mut peak_rss = 0;
            let mut peak_cpu = 0.0f32;
            let mut total_cpu = 0.0f32;
            let mut samples = 0;
            let mut previous_cpu = process_cpu_seconds();
            let mut previous_sample = Instant::now();
            while !monitor_done.load(Ordering::Acquire) {
                system.refresh_processes_specifics(
                    sysinfo::ProcessesToUpdate::Some(&[pid]),
                    true,
                    sysinfo::ProcessRefreshKind::new()
                        .with_memory()
                        .with_disk_usage(),
                );
                if let Some(process) = system.process(pid) {
                    peak_rss = peak_rss.max(process.memory());
                }
                let now = Instant::now();
                let elapsed = now.duration_since(previous_sample).as_secs_f64();
                let current_cpu = process_cpu_seconds();
                if let (Some(previous), Some(current)) = (previous_cpu, current_cpu) {
                    let cpu_percent = ((current - previous).max(0.0) / elapsed.max(0.001)) * 100.0;
                    let cpu_percent = cpu_percent as f32;
                    peak_cpu = peak_cpu.max(cpu_percent);
                    total_cpu += cpu_percent;
                    samples += 1;
                }
                previous_cpu = current_cpu;
                previous_sample = now;
                thread::sleep(Duration::from_millis(50));
            }
            system.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::Some(&[pid]),
                true,
                sysinfo::ProcessRefreshKind::new()
                    .with_memory()
                    .with_disk_usage(),
            );
            let io_end = system.process(pid).map(|process| {
                let usage = process.disk_usage();
                (usage.total_read_bytes, usage.total_written_bytes)
            });
            (peak_rss, peak_cpu, total_cpu, samples, io_start, io_end)
        });
        Self {
            done,
            task: Some(task),
        }
    }

    fn finish(mut self) -> ResourceSample {
        self.done.store(true, Ordering::Release);
        self.task
            .take()
            .expect("resource sampler task")
            .join()
            .unwrap()
    }
}

#[cfg(unix)]
fn process_cpu_seconds() -> Option<f64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if result != 0 {
        return None;
    }
    let usage = unsafe { usage.assume_init() };
    Some(
        usage.ru_utime.tv_sec as f64
            + usage.ru_utime.tv_usec as f64 / 1_000_000.0
            + usage.ru_stime.tv_sec as f64
            + usage.ru_stime.tv_usec as f64 / 1_000_000.0,
    )
}

#[cfg(target_os = "windows")]
fn process_cpu_seconds() -> Option<f64> {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetProcessTimes},
    };
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let result = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if result == 0 {
        return None;
    }
    let ticks =
        |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    Some((ticks(kernel) + ticks(user)) as f64 / 10_000_000.0)
}

fn current_rss_bytes() -> u64 {
    let Ok(pid) = sysinfo::get_current_pid() else {
        return 0;
    };
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).map_or(0, sysinfo::Process::memory)
}

fn percentiles_ms(samples: &mut [f64]) -> (f64, f64, f64) {
    samples.sort_by(f64::total_cmp);
    let percentile = |fraction: f64| {
        let index = ((samples.len() as f64 * fraction).ceil() as usize)
            .saturating_sub(1)
            .min(samples.len().saturating_sub(1));
        samples.get(index).copied().unwrap_or(0.0)
    };
    (percentile(0.50), percentile(0.95), percentile(0.99))
}

fn measure(mut operation: impl FnMut()) -> (f64, f64, f64) {
    for _ in 0..WARMUPS {
        operation();
    }
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        operation();
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    percentiles_ms(&mut samples)
}

fn update_batch(conn: &mut Connection, rows: usize, batch_size: usize, sequence: usize) -> f64 {
    let started = Instant::now();
    let transaction = conn.transaction().expect("begin paired write transaction");
    {
        let mut update = transaction
            .prepare("UPDATE global_entries SET name=?1, name_normalized=?2, path=?3, path_normalized=?4, modified_at_fs=modified_at_fs+1 WHERE id=?5")
            .expect("prepare paired index update");
        for offset in 0..batch_size {
            let index = offset % rows;
            let id = format!("view-bench-entry-{index:07}");
            let name = format!("Change {sequence:04} Annual Report {index:07}.pdf");
            let path = format!("C:\\Synthetic\\Changes\\{sequence:04}\\{name}");
            update
                .execute(params![
                    name,
                    name.to_ascii_lowercase(),
                    path,
                    path.to_ascii_lowercase().replace('\\', "/"),
                    id,
                ])
                .expect("update paired synthetic entry");
        }
    }
    transaction
        .commit()
        .expect("commit paired write transaction");
    started.elapsed().as_secs_f64() * 1000.0
}

fn mixed_change_stress(
    conn: &mut Connection,
    worker: Option<&crate::global_index::search_view::SearchViewManager>,
) -> (f64, (f64, f64, f64)) {
    const TRANSACTIONS: usize = 100;
    const ROWS_PER_TRANSACTION: usize = 50;
    let started = Instant::now();
    let mut samples = Vec::with_capacity(TRANSACTIONS);
    for transaction_index in 0..TRANSACTIONS {
        let transaction_started = Instant::now();
        let transaction = conn.transaction().expect("begin mixed write stress batch");
        {
            let mut update = transaction
                .prepare("UPDATE global_entries SET name=?1, name_normalized=?2, path=?3, path_normalized=?4, modified_at_fs=modified_at_fs+1 WHERE id=?5")
                .expect("prepare mixed rename and move");
            let mut delete = transaction
                .prepare("DELETE FROM global_entries WHERE id=?1")
                .expect("prepare mixed delete");
            let mut insert = transaction
                .prepare("INSERT INTO global_entries (id, volume_id, platform_file_id, parent_platform_file_id, name, name_normalized, path, path_normalized, extension, is_directory, size, created_at_fs, modified_at_fs, file_attributes, is_hidden, is_system, is_stale, source_provider, last_seen_at) VALUES (?1, 'view-bench-volume', ?2, ?3, ?4, ?5, ?6, ?7, 'pdf', 0, 1024, 1700000000, ?8, 0, 0, 0, 0, 'windows_mft', 1) ON CONFLICT(id) DO UPDATE SET name=excluded.name, name_normalized=excluded.name_normalized, path=excluded.path, path_normalized=excluded.path_normalized, modified_at_fs=excluded.modified_at_fs")
                .expect("prepare mixed insert and restore");
            for offset in 0..ROWS_PER_TRANSACTION {
                let index = offset;
                let id = format!("view-bench-entry-{index:07}");
                let stamp = transaction_index * ROWS_PER_TRANSACTION + offset;
                let name = format!("Mixed {transaction_index:03} {index:07}.pdf");
                let path = format!("C:\\Synthetic\\Moved\\{transaction_index:03}\\{name}");
                match transaction_index % 5 {
                    0 | 4 => {
                        update
                            .execute(params![
                                name,
                                name.to_ascii_lowercase(),
                                path,
                                path.to_ascii_lowercase().replace('\\', "/"),
                                id,
                            ])
                            .expect("rename mixed synthetic entry");
                    }
                    1 => {
                        let moved =
                            format!("C:\\Synthetic\\Moved\\Batch-{transaction_index:03}\\{name}");
                        update
                            .execute(params![
                                name,
                                name.to_ascii_lowercase(),
                                moved,
                                moved.to_ascii_lowercase().replace('\\', "/"),
                                id,
                            ])
                            .expect("move mixed synthetic entry");
                    }
                    2 => {
                        delete.execute([id]).expect("delete mixed synthetic entry");
                    }
                    _ => {
                        let platform_file_id = format!("synthetic-file-{index:07}");
                        let parent_id = format!("synthetic-parent-{:07}", index / 20);
                        insert
                            .execute(params![
                                id,
                                platform_file_id,
                                parent_id,
                                name,
                                name.to_ascii_lowercase(),
                                path,
                                path.to_ascii_lowercase().replace('\\', "/"),
                                stamp as i64,
                            ])
                            .expect("restore mixed synthetic entry");
                    }
                }
            }
        }
        transaction
            .commit()
            .expect("commit mixed write stress batch");
        samples.push(transaction_started.elapsed().as_secs_f64() * 1000.0);
        if let Some(worker) = worker {
            worker.request_refresh_for_test();
        }
    }
    (
        started.elapsed().as_secs_f64() * 1000.0,
        percentiles_ms(&mut samples),
    )
}

fn wait_for_worker_generation(
    database: &Database,
    target: i64,
    timeout: Duration,
) -> Result<f64, &'static str> {
    database.global_search_view.request_refresh_for_test();
    let started = Instant::now();
    let deadline = started + timeout;
    loop {
        if database
            .global_search_view
            .current_generation_for_test()
            .is_some_and(|generation| generation >= target)
        {
            return Ok(started.elapsed().as_secs_f64() * 1000.0);
        }
        if !database.global_search_view.enabled_for_test() {
            return Err("worker_disabled");
        }
        if Instant::now() >= deadline {
            return Err("worker_timeout");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn checkpoint_without_worker(conn: &mut Connection) -> f64 {
    let started = Instant::now();
    let generation = schema_global_search_changes::read_state(conn)
        .expect("read write-cost feed before checkpoint")
        .generation;
    assert!(
        schema_global_search_changes::reset_for_controlled_benchmark(conn, generation)
            .expect("reset controlled benchmark feed")
    );
    started.elapsed().as_secs_f64() * 1000.0
}

fn checkpoint_with_worker(database: &Database, conn: &mut Connection) -> Result<f64, &'static str> {
    let target = schema_global_search_changes::read_state(conn)
        .expect("read worker checkpoint generation")
        .generation;
    let catchup_ms = wait_for_worker_generation(database, target, Duration::from_secs(15))?;
    assert!(
        schema_global_search_changes::reset_after_full_snapshot(conn, target)
            .expect("reset worker checkpoint feed")
    );
    Ok(catchup_ms)
}

fn measure_write_batches(
    database: &Database,
    conn: &mut Connection,
    rows: usize,
    batch_size: usize,
    feed_enabled: bool,
    worker_enabled: bool,
) -> WriteBatchMeasurements {
    let mut write_samples = Vec::with_capacity(SAMPLES);
    let mut worker_samples = Vec::with_capacity(SAMPLES + WARMUPS);
    let mut worker_failed = false;
    let mut feed_saturated = false;
    let total_samples = WARMUPS + SAMPLES;
    for sequence in 0..total_samples {
        let elapsed = update_batch(conn, rows, batch_size, sequence);
        if feed_enabled {
            feed_saturated |= schema_global_search_changes::read_state(conn)
                .expect("read write-cost feed state")
                .feed_saturated;
        }
        if sequence >= WARMUPS {
            write_samples.push(elapsed);
        }
        if worker_enabled && !worker_failed {
            match checkpoint_with_worker(database, conn) {
                Ok(elapsed) if sequence >= WARMUPS => worker_samples.push(elapsed),
                Ok(_) => {}
                Err(_) => worker_failed = true,
            }
        } else if feed_enabled && !worker_enabled && sequence == WARMUPS - 1 {
            checkpoint_without_worker(conn);
        }
    }
    if feed_enabled && !worker_enabled {
        checkpoint_without_worker(conn);
    }
    let (write_p50, write_p95, write_p99) = percentiles_ms(&mut write_samples);
    let worker_percentiles = if worker_samples.is_empty() {
        (0.0, 0.0, 0.0)
    } else {
        percentiles_ms(&mut worker_samples)
    };
    (
        (write_p50, write_p95, write_p99),
        worker_percentiles,
        feed_saturated,
        worker_failed,
    )
}

fn independent_oracle(conn: &Connection, query: &str) -> (usize, Vec<String>) {
    let count = conn
        .query_row(
            "SELECT COUNT(*) FROM global_entries entry JOIN global_volumes volume ON volume.id=entry.volume_id WHERE volume.enabled=1 AND entry.is_stale=0 AND (instr(entry.name_normalized, ?1)>0 OR instr(entry.path_normalized, ?1)>0 OR instr(entry.extension, ?1)>0)",
            [query],
            |row| row.get::<_, i64>(0),
        )
        .expect("count independent SQL oracle") as usize;
    let mut statement = conn
        .prepare(
            "SELECT entry.id FROM global_entries entry JOIN global_volumes volume ON volume.id=entry.volume_id WHERE volume.enabled=1 AND entry.is_stale=0 AND (instr(entry.name_normalized, ?1)>0 OR instr(entry.path_normalized, ?1)>0 OR instr(entry.extension, ?1)>0) ORDER BY entry.modified_at_fs DESC, entry.id ASC LIMIT 4096",
        )
        .expect("prepare independent SQL oracle");
    let ids = statement
        .query_map([query], |row| row.get(0))
        .expect("query independent SQL oracle")
        .collect::<Result<Vec<String>, _>>()
        .expect("collect independent SQL oracle");
    (count, ids)
}

#[test]
#[ignore = "manual synthetic Global Search view scale and resource qualification"]
fn global_search_view_scale_benchmark() {
    let rows = env::var("ZEN_CANVAS_SEARCH_VIEW_BENCHMARK_ENTRIES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100_000);
    assert!(
        ALLOWED_ROWS.contains(&rows),
        "ZEN_CANVAS_SEARCH_VIEW_BENCHMARK_ENTRIES must be one of {ALLOWED_ROWS:?}"
    );
    let rss_before_database_open = current_rss_bytes();
    let fixture = BenchmarkDatabase::create(rows);
    let rss_after_database_seed = current_rss_bytes();
    let db = fixture.database();
    let mut conn = db.conn().expect("open benchmark connection");
    let seed_state =
        schema_global_search_changes::read_state(&conn).expect("read initial benchmark generation");
    let generation = seed_state.generation;
    let budget = MemoryBudget::new(256 * 1024 * 1024);

    let build_before_rss = current_rss_bytes();
    let build_sampler = ResourceSampler::start();
    let build_started = Instant::now();
    let view = SearchView::build(&conn, generation, budget.clone())
        .expect("build bounded compact search view");
    let build_elapsed_ms = build_started.elapsed().as_secs_f64() * 1000.0;
    let (build_peak_rss, build_peak_cpu, build_average_cpu_sum, build_cpu_samples, _, _) =
        build_sampler.finish();
    let build_after_rss = current_rss_bytes();
    assert!(
        view.active_bytes() <= super::memory::MAX_ACTIVE_VIEW_BYTES,
        "active view exceeded the 160 MiB target"
    );
    assert!(budget.used() <= 256 * 1024 * 1024);

    let snapshot_reset =
        schema_global_search_changes::reset_after_full_snapshot(&mut conn, generation)
            .expect("reset feed after captured full snapshot");
    assert!(
        snapshot_reset,
        "benchmark fixture changed during full snapshot"
    );

    let idle_sampler = ResourceSampler::start();
    thread::sleep(Duration::from_secs(30));
    let (idle_peak_rss, idle_peak_cpu, idle_cpu_sum, idle_samples, _, _) = idle_sampler.finish();

    eprintln!(
        "search_view_resources={}",
        json!({
            "rows": rows,
            "database_open_and_seed_rss_delta_bytes": rss_after_database_seed.saturating_sub(rss_before_database_open),
            "build_elapsed_ms": build_elapsed_ms,
            "build_rss_before_bytes": build_before_rss,
            "build_rss_after_bytes": build_after_rss,
            "build_rss_delta_bytes": build_after_rss.saturating_sub(build_before_rss),
            "build_peak_rss_bytes": build_peak_rss,
            "build_peak_rss_delta_bytes": build_peak_rss.saturating_sub(build_before_rss),
            "build_peak_process_cpu_percent": build_peak_cpu,
            "build_average_sampled_process_cpu_percent": if build_cpu_samples == 0 { 0.0 } else { build_average_cpu_sum / build_cpu_samples as f32 },
            "build_cpu_samples": build_cpu_samples,
            "idle_peak_process_cpu_percent": idle_peak_cpu,
            "idle_average_sampled_process_cpu_percent": if idle_samples == 0 { 0.0 } else { idle_cpu_sum / idle_samples as f32 },
            "idle_cpu_samples": idle_samples,
            "idle_peak_rss_bytes": idle_peak_rss,
            "accounted_active_view_bytes": view.active_bytes(),
            "accounted_total_peak_bytes": budget.peak(),
            "accounted_total_current_bytes": budget.used(),
            "active_view_limit_bytes": super::memory::MAX_ACTIVE_VIEW_BYTES,
            "total_budget_bytes": 256 * 1024 * 1024,
            "feed_saturated_during_seed": seed_state.feed_saturated,
        })
    );

    for query in [
        "report",
        "invoice",
        "pdf",
        "jp",
        "month-03",
        "zzznomatchtoken",
        "数据库",
        "résumé",
    ] {
        let is_shadow_supported = supported_candidate_query(query).is_some();
        let (expected_count, expected_window) = if is_shadow_supported {
            let (count, ids) = independent_oracle(&conn, query);
            (Some(count), ids)
        } else {
            (None, Vec::new())
        };
        let (candidate_ids, candidate_stats, fts_stats, fts_membership_equal, fts_ordered_equal) =
            if let Some(expected_count) = expected_count {
                let candidate = view
                    .candidate_window(query, budget.clone())
                    .expect("candidate oracle query");
                let candidate_ids = candidate
                    .ids
                    .iter()
                    .map(|id| (*id).to_string())
                    .collect::<Vec<_>>();
                assert_eq!(
                    candidate.matching_count, expected_count,
                    "match count for {query}"
                );
                assert_eq!(
                    candidate_ids, expected_window,
                    "ordered Top-K oracle for {query}"
                );
                drop(candidate);

                let candidate_stats = measure(|| {
                    let result = view
                        .candidate_window(query, budget.clone())
                        .expect("candidate query sample");
                    std::hint::black_box(result.ids.len());
                });
                let fts_stats = measure(|| {
                    let result = search::search_global_fts_shadow_reference_ids(&conn, query, 4096)
                        .expect("FTS reference query sample");
                    std::hint::black_box(result.len());
                });
                let mut candidate_membership = candidate_ids.clone();
                candidate_membership.sort_unstable();
                let mut fts_ids =
                    search::search_global_fts_shadow_reference_ids(&conn, query, 4096)
                        .expect("FTS membership comparison");
                let fts_ordered_equal = candidate_ids == fts_ids;
                fts_ids.sort_unstable();
                (
                    Some(candidate_ids),
                    Some(candidate_stats),
                    Some(fts_stats),
                    Some(candidate_membership == fts_ids),
                    Some(fts_ordered_equal),
                )
            } else {
                (None, None, None, None, None)
            };

        let production_page = search::search_global_entries_on_connection(&conn, query, 80, 0)
            .expect("read production page for candidate comparison")
            .into_iter()
            .map(|result| result.id)
            .collect::<Vec<_>>();
        let candidate_first_page_equal = candidate_ids.as_ref().map(|candidate_ids| {
            candidate_ids
                .iter()
                .take(80)
                .map(String::as_str)
                .eq(production_page.iter().map(String::as_str))
        });
        let full_search_stats = measure(|| {
            let result = search::search_global_entries_on_connection(&conn, query, 80, 0)
                .expect("full production search sample");
            std::hint::black_box(result.len());
        });
        let snapshot_endpoint_stats = measure(|| {
            let result = db
                .search_global_entries_snapshot(query, 80, 0)
                .expect("full production snapshot endpoint sample");
            std::hint::black_box(result.results.len());
        });
        eprintln!(
            "search_view_latency={}",
            json!({
                "rows": rows,
                "query": query,
                "shadow_candidate_supported": is_shadow_supported,
                "match_count": expected_count,
                "warmups": WARMUPS,
                "samples": SAMPLES,
                "candidate_view_only_p50_ms": candidate_stats.map(|stats| stats.0),
                "candidate_view_only_p95_ms": candidate_stats.map(|stats| stats.1),
                "candidate_view_only_p99_ms": candidate_stats.map(|stats| stats.2),
                "sqlite_fts_only_p50_ms": fts_stats.map(|stats| stats.0),
                "sqlite_fts_only_p95_ms": fts_stats.map(|stats| stats.1),
                "sqlite_fts_only_p99_ms": fts_stats.map(|stats| stats.2),
                "fts_window_membership_equal": fts_membership_equal,
                "fts_window_order_equal": fts_ordered_equal,
                "candidate_first_page_matches_production": candidate_first_page_equal,
                "full_search_p50_ms": full_search_stats.0,
                "full_search_p95_ms": full_search_stats.1,
                "full_search_p99_ms": full_search_stats.2,
                "snapshot_endpoint_p50_ms": snapshot_endpoint_stats.0,
                "snapshot_endpoint_p95_ms": snapshot_endpoint_stats.1,
                "snapshot_endpoint_p99_ms": snapshot_endpoint_stats.2,
                "historical_search_limit_ms": 100.0,
            })
        );
    }

    let old_view = Arc::new(view);
    let before_update_bytes = budget.used();
    {
        let transaction = conn.transaction().expect("begin batch update");
        {
            let mut update = transaction
                .prepare("UPDATE global_entries SET name=?1, name_normalized=?2, path=?3, path_normalized=?4, modified_at_fs=modified_at_fs+10 WHERE id=?5")
                .expect("prepare synthetic batch update");
            for index in 0..UPDATE_BATCH {
                let id = format!("view-bench-entry-{index:07}");
                let name = format!("Changed Report {index:07}.pdf");
                let path = format!("C:\\Synthetic\\Users\\Owner\\Archive\\{name}");
                update
                    .execute(params![
                        name,
                        name.to_ascii_lowercase(),
                        path,
                        path.to_ascii_lowercase().replace('\\', "/"),
                        id
                    ])
                    .expect("update synthetic row through production triggers");
            }
        }
        transaction.commit().expect("commit synthetic batch update");
    }
    let events =
        schema_global_search_changes::changes_since(&conn, old_view.generation, UPDATE_BATCH + 1)
            .expect("read synthetic update feed");
    let update_started = Instant::now();
    let update_sampler = ResourceSampler::start();
    let update = old_view.apply_changes(&conn, &events, budget.clone(), update_started);
    let update_elapsed_ms = update_started.elapsed().as_secs_f64() * 1000.0;
    let (update_peak_rss, update_peak_cpu, update_average_cpu_sum, update_cpu_samples, _, _) =
        update_sampler.finish();
    let mut published_view = None;
    let update_status = match update {
        Ok(next_view) => {
            let next_view = Arc::new(next_view);
            let ids = next_view
                .candidate_window("archive", budget.clone())
                .expect("query updated view")
                .ids;
            assert!(ids.iter().any(|id| id.starts_with("view-bench-entry-")));
            published_view = Some(next_view);
            "applied"
        }
        Err(error) => {
            // An update budget refusal is an expected safe fallback signal; it
            // is reported as evidence and does not publish a partial view.
            eprintln!("search_view_update_fallback={error:?}");
            "fallback"
        }
    };
    eprintln!(
        "search_view_update={}",
        json!({
            "rows": rows,
            "batch_events": events.len(),
            "elapsed_ms": update_elapsed_ms,
            "status": update_status,
            "memory_before_bytes": before_update_bytes,
            "memory_after_bytes": budget.used(),
            "memory_peak_bytes": budget.peak(),
            "peak_rss_bytes": update_peak_rss,
            "peak_process_cpu_percent": (update_cpu_samples >= 5).then_some(update_peak_cpu),
            "average_sampled_process_cpu_percent": (update_cpu_samples >= 5).then_some(update_average_cpu_sum / update_cpu_samples as f32),
            "cpu_samples": update_cpu_samples,
            "old_generation": old_view.generation,
            "captured_generation": schema_global_search_changes::read_state(&conn).unwrap().generation,
        })
    );
    let rebuild_generation = schema_global_search_changes::read_state(&conn)
        .expect("read rebuild generation")
        .generation;
    let before_full_rebuild_bytes = budget.used();
    let replacement_view = SearchView::build(&conn, rebuild_generation, budget.clone())
        .expect("build replacement while readers retain the previous view");
    let after_full_rebuild_bytes = budget.used();
    drop(published_view);
    let after_incremental_arc_drop_bytes = budget.used();
    drop(old_view);
    eprintln!(
        "search_view_old_view_release={}",
        json!({
            "memory_before_rebuild_bytes": before_full_rebuild_bytes,
            "memory_after_full_rebuild_bytes": after_full_rebuild_bytes,
            "memory_after_incremental_arc_drop_bytes": after_incremental_arc_drop_bytes,
            "memory_after_old_base_drop_bytes": budget.used(),
        })
    );
    drop(replacement_view);
    eprintln!(
        "search_view_final_memory={}",
        json!({ "memory_after_all_views_drop_bytes": budget.used() })
    );
}

#[test]
#[ignore = "manual 30-minute opt-in worker idle stability qualification"]
fn global_search_view_worker_idle_stability_benchmark() {
    let rows = env::var("ZEN_CANVAS_SEARCH_VIEW_IDLE_ENTRIES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100_000);
    assert!(
        ALLOWED_ROWS.contains(&rows),
        "ZEN_CANVAS_SEARCH_VIEW_IDLE_ENTRIES must be one of {ALLOWED_ROWS:?}"
    );
    let idle_seconds = env::var("ZEN_CANVAS_SEARCH_VIEW_IDLE_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1_800);
    assert!(idle_seconds > 0, "idle duration must be positive");

    let fixture = BenchmarkDatabase::create_with_settings(rows, true, true, false);
    let database = fixture.database();
    let conn = database.conn().expect("get idle qualification connection");
    let target_generation = schema_global_search_changes::read_state(&conn)
        .expect("read seeded idle qualification generation")
        .generation;
    let startup_catchup_ms =
        wait_for_worker_generation(database, target_generation, Duration::from_secs(120))
            .expect("opt-in worker catches up before idle interval");
    let before = database
        .global_search_view
        .diagnostics(&conn)
        .expect("read worker diagnostics before idle")
        .expect("change feed is installed");
    assert_eq!(before.generation_lag, 0);
    assert!(before.worker_enabled);

    let rss_before_bytes = current_rss_bytes();
    let sampler = ResourceSampler::start();
    thread::sleep(Duration::from_secs(idle_seconds));
    let (
        peak_rss_bytes,
        peak_cpu,
        average_cpu_sum,
        cpu_samples,
        process_io_before,
        process_io_after,
    ) = sampler.finish();
    let rss_after_bytes = current_rss_bytes();
    let after = database
        .global_search_view
        .diagnostics(&conn)
        .expect("read worker diagnostics after idle")
        .expect("change feed remains installed");
    assert_eq!(after.feed_generation, before.feed_generation);
    assert_eq!(after.view_generation, before.view_generation);
    assert_eq!(after.generation_lag, 0);
    assert_eq!(after.full_builds, before.full_builds);
    assert_eq!(after.incremental_batches, before.incremental_batches);
    assert_eq!(
        after.accounted_memory_used_bytes,
        before.accounted_memory_used_bytes
    );
    assert_eq!(after.retries, before.retries);
    assert_eq!(after.retry_exhausted, before.retry_exhausted);

    eprintln!(
        "search_view_idle_stability={}",
        json!({
            "rows": rows,
            "idle_seconds": idle_seconds,
            "startup_catchup_ms": startup_catchup_ms,
            "worker_enabled_before_and_after": before.worker_enabled && after.worker_enabled,
            "feed_generation_before_and_after": [before.feed_generation, after.feed_generation],
            "view_generation_before_and_after": [before.view_generation, after.view_generation],
            "generation_lag_before_and_after": [before.generation_lag, after.generation_lag],
            "full_builds_before_and_after": [before.full_builds, after.full_builds],
            "incremental_batches_before_and_after": [before.incremental_batches, after.incremental_batches],
            "accounted_memory_before_and_after_bytes": [before.accounted_memory_used_bytes, after.accounted_memory_used_bytes],
            "accounted_peak_memory_after_bytes": after.accounted_memory_peak_bytes,
            "retry_count_before_and_after": [before.retries, after.retries],
            "retry_exhausted_before_and_after": [before.retry_exhausted, after.retry_exhausted],
            "rss_before_bytes": rss_before_bytes,
            "rss_after_bytes": rss_after_bytes,
            "rss_delta_bytes": rss_after_bytes as i64 - rss_before_bytes as i64,
            "sampled_peak_rss_bytes": peak_rss_bytes,
            "peak_process_cpu_percent": peak_cpu,
            "average_sampled_process_cpu_percent": if cpu_samples == 0 { 0.0 } else { average_cpu_sum / cpu_samples as f32 },
            "cpu_samples": cpu_samples,
            "process_io_counters_reported_nonzero": process_io_before.zip(process_io_after).is_some_and(|(start, end)| end.0 > start.0 || end.1 > start.1),
        })
    );
    drop(conn);
    drop(fixture);
}

#[test]
#[ignore = "manual paired no-feed/feed write amplification qualification"]
fn global_search_view_change_feed_write_cost_benchmark() {
    let rows = env::var("ZEN_CANVAS_SEARCH_VIEW_WRITE_COST_ENTRIES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100_000);
    assert!(
        ALLOWED_ROWS.contains(&rows),
        "ZEN_CANVAS_SEARCH_VIEW_WRITE_COST_ENTRIES must be one of {ALLOWED_ROWS:?}"
    );

    for (scenario, feed_enabled, worker_enabled) in [
        ("A_no_feed", false, false),
        ("B_feed_worker_off", true, false),
        ("C_feed_worker_on", true, true),
    ] {
        let rss_before = current_rss_bytes();
        let sampler = ResourceSampler::start();
        let fixture =
            BenchmarkDatabase::create_with_settings(rows, feed_enabled, worker_enabled, true);
        let database = fixture.database();
        let mut conn = database.conn().expect("get write-cost fixture connection");
        let trigger_count = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type='trigger' AND name LIKE 'global_search_changes_%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("count installed change-feed triggers");
        assert_eq!(trigger_count == 0, !feed_enabled);
        let sidecar_installed = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='global_search_change_state')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .expect("check change-feed sidecar installation");
        assert_eq!(sidecar_installed, feed_enabled);

        let mut initial_feed_reset_ms = 0.0;
        let initial_worker_catchup_ms = if worker_enabled {
            let generation = schema_global_search_changes::read_state(&conn)
                .expect("read seeded worker generation")
                .generation;
            wait_for_worker_generation(database, generation, Duration::from_secs(60))
                .expect("worker builds initial view from identical seeded rows")
        } else if feed_enabled {
            initial_feed_reset_ms = checkpoint_without_worker(&mut conn);
            0.0
        } else {
            0.0
        };

        let automatic_checkpoint_pages: i64 = conn
            .query_row("PRAGMA wal_autocheckpoint", [], |row| row.get(0))
            .expect("read paired writer WAL checkpoint setting");
        assert_eq!(automatic_checkpoint_pages, 0);
        let pre_workload_wal_checkpoint = conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .expect("checkpoint seeded database before measuring writes");
        assert_eq!(
            pre_workload_wal_checkpoint.0, 0,
            "seed checkpoint was not busy"
        );
        assert_eq!(
            pre_workload_wal_checkpoint.1, pre_workload_wal_checkpoint.2,
            "seed checkpoint applied every pending WAL frame"
        );

        let mut workload_wal_path = fixture.path.clone();
        workload_wal_path.set_extension("db-wal");
        let mut workload_results = Vec::new();
        for batch_size in [1, 512, 5_000] {
            let wal_before_bytes =
                fs::metadata(&workload_wal_path).map_or(0, |metadata| metadata.len());
            let (write_ms, worker_ms, feed_saturated, worker_failed) = measure_write_batches(
                database,
                &mut conn,
                rows,
                batch_size,
                feed_enabled,
                worker_enabled,
            );
            let wal_after_bytes =
                fs::metadata(&workload_wal_path).map_or(0, |metadata| metadata.len());
            workload_results.push(json!({
                "batch_size": batch_size,
                "warmups": WARMUPS,
                "samples": SAMPLES,
                "write_p50_p95_p99_ms": write_ms,
                "write_per_entry_p50_p95_p99_us": [
                    write_ms.0 * 1000.0 / batch_size as f64,
                    write_ms.1 * 1000.0 / batch_size as f64,
                    write_ms.2 * 1000.0 / batch_size as f64,
                ],
                "worker_catchup_p50_p95_p99_ms": worker_ms,
                "feed_saturated_during_samples": feed_saturated,
                "worker_failed_or_disabled": worker_failed,
                "wal_file_size_bytes_before_workload": wal_before_bytes,
                "wal_file_size_bytes_after_workload": wal_after_bytes,
                "wal_file_growth_bytes_during_workload": wal_after_bytes.saturating_sub(wal_before_bytes),
            }));
        }

        let mixed_worker = worker_enabled
            .then_some(&database.global_search_view)
            .filter(|worker| worker.enabled_for_test());
        let mixed_wal_before_bytes =
            fs::metadata(&workload_wal_path).map_or(0, |metadata| metadata.len());
        let (mixed_wall_ms, mixed_transaction_ms) = mixed_change_stress(&mut conn, mixed_worker);
        let mixed_worker_catchup_ms = if worker_enabled {
            if database.global_search_view.enabled_for_test() {
                checkpoint_with_worker(database, &mut conn).ok()
            } else {
                None
            }
        } else {
            None
        };

        let (peak_rss, peak_cpu, total_cpu, cpu_samples, process_io_before, process_io_after) =
            sampler.finish();
        let rss_after = current_rss_bytes();
        let mixed_wal_after_bytes =
            fs::metadata(&workload_wal_path).map_or(0, |metadata| metadata.len());
        let diagnostics_snapshot = database
            .global_search_view
            .diagnostics(&conn)
            .expect("read worker diagnostics after paired writes");
        if worker_enabled {
            let diagnostics = diagnostics_snapshot
                .as_ref()
                .expect("feed-enabled worker has diagnostics");
            assert!(
                diagnostics.worker_enabled,
                "worker disabled during paired writes"
            );
            assert!(
                !diagnostics.feed_saturated,
                "worker left the change feed saturated"
            );
            assert_eq!(diagnostics.generation_lag, 0, "worker did not catch up");
            assert!(diagnostics.fallback_reason.is_none());
            assert!(
                diagnostics.full_builds <= 2,
                "worker repeatedly rebuilt the full view"
            );
            for result in &workload_results {
                assert_eq!(result["worker_failed_or_disabled"], false);
                assert_eq!(result["feed_saturated_during_samples"], false);
            }
        }
        let worker_diagnostics = diagnostics_snapshot.map(|diagnostics| {
            json!({
                "worker_enabled": diagnostics.worker_enabled,
                "feed_generation": diagnostics.feed_generation,
                "view_generation": diagnostics.view_generation,
                "generation_lag": diagnostics.generation_lag,
                "feed_saturated": diagnostics.feed_saturated,
                "fallback_reason": diagnostics.fallback_reason,
                "accounted_memory_used_bytes": diagnostics.accounted_memory_used_bytes,
                "accounted_memory_peak_bytes": diagnostics.accounted_memory_peak_bytes,
                "refresh_requests": diagnostics.refresh_requests,
                "full_builds": diagnostics.full_builds,
                "incremental_batches": diagnostics.incremental_batches,
                "comparisons": diagnostics.comparisons,
                "comparable_membership_checks": diagnostics.comparable_membership_checks,
                "full_membership_equal": diagnostics.full_membership_equal,
                "window_membership_equal": diagnostics.window_membership_equal,
                "ordered_equal": diagnostics.ordered_equal,
                "page_equal": diagnostics.page_equal,
                "production_page_equal": diagnostics.production_page_equal,
                "sqlite_tier_skips": diagnostics.sqlite_tier_skips,
                "empty_or_sparse_skips": diagnostics.empty_or_sparse_skips,
                "large_corpus_skips": diagnostics.large_corpus_skips,
                "unsupported_queries": diagnostics.unsupported_queries,
                "candidate_probes": diagnostics.candidate_probes,
                "missing_or_stale_views": diagnostics.missing_or_stale_views,
                "feed_saturation_observations": diagnostics.feed_saturation_observations,
                "feed_gap_or_source_changes": diagnostics.feed_gap_or_source_changes,
                "update_budget_failures": diagnostics.update_budget_failures,
                "retries": diagnostics.retries,
                "retry_exhausted": diagnostics.retry_exhausted,
                "memory_refusals": diagnostics.memory_refusals,
                "database_errors": diagnostics.database_errors,
            })
        });
        let process_io_read_delta_bytes = process_io_before
            .zip(process_io_after)
            .map(|(before, after)| after.0.saturating_sub(before.0));
        let process_io_written_delta_bytes = process_io_before
            .zip(process_io_after)
            .map(|(before, after)| after.1.saturating_sub(before.1));
        eprintln!(
            "search_view_write_cost={}",
            json!({
                "scenario": scenario,
                "rows": rows,
                "seed_write_ms": fixture.seed_elapsed_ms,
                "feed_installed": feed_enabled,
                "worker_enabled": worker_enabled,
                "change_trigger_count": trigger_count,
                "sidecar_installed": sidecar_installed,
                "initial_worker_catchup_ms": initial_worker_catchup_ms,
                "initial_feed_reset_ms": initial_feed_reset_ms,
                "pre_workload_wal_checkpoint_busy_log_checkpointed_frames": pre_workload_wal_checkpoint,
                "single_512_5000_updates": workload_results,
                "worker_diagnostics": worker_diagnostics,
                "mixed_rename_move_delete_5000": {
                    "transactions": 100,
                    "updates_per_transaction": 50,
                    "wall_ms": mixed_wall_ms,
                    "transaction_p50_p95_p99_ms": mixed_transaction_ms,
                    "worker_catchup_ms": mixed_worker_catchup_ms,
                    "wal_file_size_bytes_before_workload": mixed_wal_before_bytes,
                    "wal_file_size_bytes_after_workload": mixed_wal_after_bytes,
                    "wal_file_growth_bytes_during_workload": mixed_wal_after_bytes.saturating_sub(mixed_wal_before_bytes),
                },
                "process_io_read_bytes_before": process_io_before.map(|io| io.0),
                "process_io_read_bytes_after": process_io_after.map(|io| io.0),
                "process_io_read_bytes_delta": process_io_read_delta_bytes,
                "process_io_written_bytes_before": process_io_before.map(|io| io.1),
                "process_io_written_bytes_after": process_io_after.map(|io| io.1),
                "process_io_written_bytes_delta": process_io_written_delta_bytes,
                "process_io_counters_reported_nonzero": process_io_read_delta_bytes.unwrap_or(0) > 0 || process_io_written_delta_bytes.unwrap_or(0) > 0,
                "wal_autocheckpoint_disabled_during_workloads": true,
                "wal_growth_is_append_only_database_log_bytes": true,
                "rss_before_bytes": rss_before,
                "rss_after_bytes": rss_after,
                "peak_rss_bytes": peak_rss,
                "peak_process_cpu_percent": peak_cpu,
                "average_process_cpu_percent": if cpu_samples == 0 { 0.0 } else { total_cpu / cpu_samples as f32 },
                "cpu_samples": cpu_samples,
            })
        );
        drop(conn);
        drop(fixture);
    }
}
