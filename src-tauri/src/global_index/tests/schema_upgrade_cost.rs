//! One production-shaped fixture, controlled short-ID/size copies. No search
//! microbenchmark rerun: this measures the actual installed-schema open path.
use super::*;
use crate::global_index::stable_entry_id;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

fn historical_indexes(conn: &Connection) {
    conn.execute_batch("DROP INDEX idx_global_entries_active_name_order;
        DROP INDEX idx_global_entries_active_source_facts;
        CREATE INDEX idx_global_entries_active_name_order ON global_entries(name_normalized,modified_at_fs DESC,id ASC) WHERE is_stale=0;
        PRAGMA user_version=42;").unwrap();
}
fn snapshots(db: &Database) -> Vec<crate::global_index::repository::GlobalSearchSnapshot> {
    QUERY_MATRIX
        .iter()
        .map(|q| db.search_global_entries_snapshot(q.query, 80, 0).unwrap())
        .collect()
}
fn historical_snapshots(
    conn: &Connection,
) -> Vec<crate::global_index::repository::GlobalSearchSnapshot> {
    use crate::global_index::repository::{
        diagnostic_compare_snapshot_queries_on_connection, GlobalSearchSourceHealthQueryCandidate,
    };
    let tx = conn.unchecked_transaction().unwrap();
    let snapshots = QUERY_MATRIX
        .iter()
        .map(|q| {
            diagnostic_compare_snapshot_queries_on_connection(
                &tx,
                q.query,
                80,
                0,
                GlobalSearchSourceHealthQueryCandidate::Original,
            )
            .unwrap()
            .0
        })
        .collect();
    tx.commit().unwrap();
    snapshots
}

fn sizes(path: &Path) -> [u64; 3] {
    ["", "-wal", "-shm"]
        .map(|suffix| fs::metadata(format!("{}{suffix}", path.display())).map_or(0, |m| m.len()))
}

#[test]
#[ignore = "bounded real schema-upgrade measurement; one generated fixture"]
fn schema_upgrade_production_shape_cost() {
    let entries: u64 = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES")
        .unwrap_or("200000".into())
        .parse()
        .unwrap();
    assert!([200_000, 1_000_000].contains(&entries));
    let base_path = test_db_path();
    let _base_cleanup = BenchmarkCleanup(base_path.clone());
    let started = Instant::now();
    let base = Database::open(&base_path).unwrap();
    let volumes = (0..4)
        .map(|i| {
            let mut v = test_volume();
            v.stable_volume_id =
                format!("\\\\?\\Volume{{A0C934B1-073A-4A24-B72C-99AAB23D877{i}}}\\");
            v.id = format!(
                "gv_{}",
                blake3::hash(v.stable_volume_id.as_bytes()).to_hex()
            );
            v.enabled = i != 3;
            v.index_status = crate::global_index::INDEX_STATUS_READY.into();
            base.upsert_global_volume(&v).unwrap();
            v
        })
        .collect::<Vec<_>>();
    // Full product indexes/FTS/triggers/constraints remain installed throughout
    // population; construct historical 42 once, before inserting user data.
    historical_indexes(&base.conn().unwrap());
    let mut conn = base.conn().unwrap();
    for first in (0..entries).step_by(INSERT_BATCH_SIZE as usize) {
        let tx = conn.transaction().unwrap();
        {
            let mut statement = tx.prepare(INSERT_SYNTHETIC_ENTRY).unwrap();
            for i in first..(first + INSERT_BATCH_SIZE).min(entries) {
                let e = generated_entry(i);
                let volume = &volumes[(i % 4) as usize];
                let id = stable_entry_id(
                    &volume.id,
                    &e.platform_file_id,
                    &e.parent_platform_file_id,
                    &e.name,
                    &e.path,
                );
                statement
                    .execute(params![
                        id,
                        volume.id,
                        e.platform_file_id,
                        e.parent_platform_file_id,
                        e.name,
                        e.name_normalized,
                        e.path,
                        e.path_normalized,
                        e.extension,
                        0_i64,
                        e.size,
                        e.created_at_fs,
                        e.modified_at_fs,
                        e.file_attributes,
                        i64::from(e.is_hidden),
                        i64::from(e.is_system),
                        i64::from(i % 13 == 0),
                        "synthetic_global_search_benchmark",
                        1_700_000_000_i64 + i as i64
                    ])
                    .unwrap();
            }
        }
        tx.commit().unwrap();
    }
    assert_entry_triggers_enabled(&conn);
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    drop(conn);
    drop(base);
    emit_record(
        &json!({"record_type":"upgrade_fixture","context":benchmark_context(entries),"fixture_generations":1,"rows":entries,"population_ms":started.elapsed().as_secs_f64()*1000.0,"full_schema":true,"id_bytes":67,"volume_id_bytes":67,"stale_every":13,"disabled_volumes":1}),
    );
    let scales = if entries == 200_000 {
        vec![100_000, 200_000]
    } else {
        vec![entries]
    };
    for count in scales {
        for short in [false, true] {
            let path = test_db_path();
            let _cleanup = BenchmarkCleanup(path.clone());
            fs::copy(&base_path, &path).unwrap();
            let conn = configured_connection(&path).unwrap();
            if count < entries {
                conn.execute("DELETE FROM global_entries WHERE rowid>?1", [count as i64])
                    .unwrap();
            }
            if short {
                conn.execute(
                    "UPDATE global_entries SET id='ge_bench_' || printf('%07d',rowid)",
                    [],
                )
                .unwrap();
                // Historical short model had one short gv_test source. Keep
                // all four enabled/stale groups while changing only key width.
                conn.execute_batch("BEGIN; PRAGMA defer_foreign_keys=ON;")
                    .unwrap();
                for (i, v) in volumes.iter().enumerate() {
                    let id = format!("gv_short_{i}");
                    conn.execute(
                        "UPDATE global_entries SET volume_id=?1 WHERE volume_id=?2",
                        params![id, v.id],
                    )
                    .unwrap();
                    conn.execute(
                        "UPDATE global_volumes SET id=?1 WHERE id=?2",
                        params![id, v.id],
                    )
                    .unwrap();
                }
                conn.execute_batch("COMMIT").unwrap();
            }
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .unwrap();
            let resources = crate::db::upgrade::inspect_resources(&conn).unwrap();
            let indexes_before: Vec<(String, i64)> = conn
                .prepare("SELECT name,SUM(pgsize) FROM dbstat GROUP BY name ORDER BY name")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let before_snapshots = historical_snapshots(&conn);
            drop(conn);
            let before = sizes(&path);
            let done = Arc::new(AtomicBool::new(false));
            let monitor_done = done.clone();
            let monitor_path = path.clone();
            let temp_directory = resources.temporary_directory.clone();
            let monitor = std::thread::spawn(move || {
                let pid = sysinfo::get_current_pid().unwrap();
                let mut system = sysinfo::System::new();
                let mut peak_rss = 0;
                let mut peak_files = [0; 3];
                let mut samples = 0;
                let mut min_db_free = u64::MAX;
                let mut min_temp_free = u64::MAX;
                while !monitor_done.load(Ordering::Acquire) {
                    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
                    peak_rss = peak_rss.max(system.process(pid).map_or(0, |p| p.memory()));
                    for (peak, size) in peak_files.iter_mut().zip(sizes(&monitor_path)) {
                        *peak = (*peak).max(size);
                    }
                    min_db_free = min_db_free.min(
                        crate::db::upgrade::storage_free_bytes(monitor_path.parent().unwrap())
                            .unwrap(),
                    );
                    min_temp_free = min_temp_free
                        .min(crate::db::upgrade::storage_free_bytes(&temp_directory).unwrap());
                    samples += 1;
                    std::thread::sleep(Duration::from_millis(20));
                }
                (peak_rss, peak_files, samples, min_db_free, min_temp_free)
            });
            let started = Instant::now();
            let opened = Database::open(&path);
            let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
            done.store(true, Ordering::Release);
            let (peak_rss, peak_files, samples, min_db_free, min_temp_free) =
                monitor.join().unwrap();
            let db = opened.unwrap();
            let after_snapshots = snapshots(&db);
            assert_eq!(after_snapshots.len(), 12);
            for (before, after) in before_snapshots.iter().zip(&after_snapshots) {
                assert_eq!(before.results, after.results);
                assert_eq!(before.source_health, after.source_health);
                assert_eq!(before.source_revision, after.source_revision);
                assert_eq!(before.index_status, after.index_status);
            }
            let conn = db.conn().unwrap();
            assert_eq!(
                conn.query_row::<i64, _, _>("PRAGMA user_version", [], |r| r.get(0))
                    .unwrap(),
                43
            );
            let indexes_after: Vec<(String, i64)> = conn
                .prepare("SELECT name,SUM(pgsize) FROM dbstat GROUP BY name ORDER BY name")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let plan:Vec<String>=conn.prepare("EXPLAIN QUERY PLAN SELECT id,volume_id FROM global_entries INDEXED BY idx_global_entries_active_name_order WHERE is_stale=0 AND name_normalized GLOB 'quarterly*' ORDER BY name_normalized,modified_at_fs DESC,id LIMIT 80").unwrap().query_map([],|r|r.get(3)).unwrap().collect::<Result<_,_>>().unwrap();
            emit_record(
                &json!({"record_type":"upgrade_sample","rows":count,"short_ids":short,"open_ms":elapsed_ms,"preflight":{"key_bytes":resources.key_bytes,"page_size":resources.page_size,"page_count":resources.page_count,"freelist_pages":resources.freelist_pages,"db_reserved":resources.database_required_bytes,"temp_reserved":resources.temporary_required_bytes,"db_free":resources.database_free_bytes,"temp_free":resources.temporary_free_bytes},"files_before":before,"files_after":sizes(&path),"sampled_peak_files":peak_files,"sampled_process_rss":peak_rss,"samples_20ms":samples,"minimum_database_free_bytes":min_db_free,"minimum_temp_free_bytes":min_temp_free,"memory_scope":"sampled process-wide current RSS; includes fixture/cache memory; not an OS peak or migration-attributed bound","indexes_before":indexes_before,"indexes_after":indexes_after,"name_plan":plan}),
            );
            drop(conn);
            drop(db);
        }
    }
}
