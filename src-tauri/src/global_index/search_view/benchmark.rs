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

struct BenchmarkDatabase {
    path: PathBuf,
    database: Option<Database>,
}

impl BenchmarkDatabase {
    fn create(rows: usize) -> Self {
        let path = env::temp_dir().join(format!(
            "zen-canvas-search-view-bench-{}-{rows}.db",
            std::process::id()
        ));
        remove_files(&path);
        let database = Database::open(&path).expect("open isolated synthetic database");
        let mut connection = database.conn().expect("get synthetic database connection");
        connection
            .execute(
                "INSERT INTO global_volumes (id, platform, stable_volume_id, display_name, mount_path, filesystem_type, drive_kind, enabled, provider, index_status, created_at, updated_at) VALUES ('view-bench-volume', 'windows', 'synthetic-view-bench-volume', 'Synthetic benchmark', 'C:\\Synthetic', 'ntfs', 'fixed', 1, 'windows_mft', 'ready', 1, 1)",
                [],
            )
            .expect("insert isolated synthetic source");

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
        drop(connection);
        Self {
            path,
            database: Some(database),
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
    task: Option<thread::JoinHandle<(u64, f32, f32, usize)>>,
}

impl ResourceSampler {
    fn start() -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let monitor_done = done.clone();
        let task = thread::spawn(move || {
            let Ok(pid) = sysinfo::get_current_pid() else {
                return (0, 0.0, 0.0, 0);
            };
            let mut system = sysinfo::System::new();
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
                    sysinfo::ProcessRefreshKind::new().with_memory(),
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
            (peak_rss, peak_cpu, total_cpu, samples)
        });
        Self {
            done,
            task: Some(task),
        }
    }

    fn finish(mut self) -> (u64, f32, f32, usize) {
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
    let (build_peak_rss, build_peak_cpu, build_average_cpu_sum, build_cpu_samples) =
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
    let (idle_peak_rss, idle_peak_cpu, idle_cpu_sum, idle_samples) = idle_sampler.finish();

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
    let (update_peak_rss, update_peak_cpu, update_average_cpu_sum, update_cpu_samples) =
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
