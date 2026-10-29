//! One production-shaped fixture, controlled installed-version/size copies. No search
//! microbenchmark rerun: this measures the actual installed-schema open path.
use super::*;
use crate::global_index::stable_entry_id;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

fn historical_indexes(conn: &Connection) {
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS global_entry_source_facts_ai;
        DROP TRIGGER IF EXISTS global_entry_source_facts_ad;
        DROP TRIGGER IF EXISTS global_entry_source_facts_au;
        DROP TRIGGER IF EXISTS global_entry_source_facts_volume_ai;
        DROP TRIGGER IF EXISTS global_entry_source_facts_volume_au;
        DROP TABLE IF EXISTS global_entry_source_facts;",
    )
    .unwrap();
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

thread_local! {
    static BACKFILL_TIMES: std::cell::RefCell<Vec<(&'static str,Instant)>> = const { std::cell::RefCell::new(Vec::new()) };
}
fn measure_backfill(_: &Connection, stage: &str) -> Result<(), crate::db::DbError> {
    BACKFILL_TIMES.with(|v| {
        v.borrow_mut().push((
            if stage == "before" { "before" } else { "after" },
            Instant::now(),
        ))
    });
    Ok(())
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
    assert_eq!(
        entries, 200_000,
        "This integration uses one bounded 200k fixture per platform"
    );
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
    conn.execute_batch("UPDATE global_entries SET id=NULL WHERE rowid=2;")
        .unwrap();
    assert_entry_triggers_enabled(&conn);
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    drop(conn);
    drop(base);
    emit_record(
        &json!({"record_type":"upgrade_fixture","context":benchmark_context(entries),"fixture_generations":1,"rows":entries,"population_ms":started.elapsed().as_secs_f64()*1000.0,"full_schema":true,"id_bytes":67,"volume_id_bytes":67,"stale_every":13,"disabled_volumes":1}),
    );
    let scales = [100_000, 200_000];
    for count in scales {
        for starting_schema in [42, 43] {
            let short = false;
            let path = test_db_path();
            let _cleanup = BenchmarkCleanup(path.clone());
            let preparation_started = Instant::now();
            fs::copy(&base_path, &path).unwrap();
            let conn = configured_connection(&path).unwrap();
            if count < entries {
                conn.execute("DELETE FROM global_entries WHERE rowid>?1", [count as i64])
                    .unwrap();
            }
            // Real43 fixture: rebuild indexes before timing; no facts installed.
            if starting_schema == 43 {
                conn.execute_batch("DROP INDEX idx_global_entries_active_name_order;
                    CREATE INDEX idx_global_entries_active_name_order ON global_entries(name_normalized,modified_at_fs DESC,id ASC,volume_id) WHERE is_stale=0;
                    CREATE INDEX idx_global_entries_active_source_facts ON global_entries(volume_id,last_seen_at,id) WHERE is_stale=0;
                    PRAGMA user_version=43;").unwrap();
            }
            let compacted = true;
            if compacted {
                conn.execute_batch("VACUUM").unwrap();
            }
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .unwrap();
            let preparation_ms = preparation_started.elapsed().as_secs_f64() * 1000.0;
            let resources = crate::db::upgrade::inspect_resources(&conn).unwrap();
            let indexes_before: Vec<(String, i64)> = conn
                .prepare("SELECT name,SUM(pgsize) FROM dbstat GROUP BY name ORDER BY name")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let before_snapshots =
                crate::global_index::repository::with_canonical_source_facts(|| {
                    historical_snapshots(&conn)
                });
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
                    system.refresh_processes_specifics(
                        sysinfo::ProcessesToUpdate::Some(&[pid]),
                        true,
                        sysinfo::ProcessRefreshKind::new().with_memory(),
                    );
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
            BACKFILL_TIMES.with(|v| v.borrow_mut().clear());
            let opened =
                crate::db::schema_global_source_facts::with_backfill_hook(measure_backfill, || {
                    Database::open(&path)
                });
            let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
            done.store(true, Ordering::Release);
            let (peak_rss, peak_files, samples, min_db_free, min_temp_free) =
                monitor.join().unwrap();
            let db = opened.unwrap();
            let backfill_ms = BACKFILL_TIMES.with(|v| {
                let v = v.borrow();
                assert_eq!(v.len(), 2);
                v[1].1.duration_since(v[0].1).as_secs_f64() * 1000.0
            });
            let after_snapshots = snapshots(&db);
            assert_eq!(after_snapshots.len(), 12);
            for (before, after) in before_snapshots.iter().zip(&after_snapshots) {
                assert_eq!(before.results, after.results);
                assert_eq!(before.source_health, after.source_health);
                assert_eq!(before.source_revision, after.source_revision);
                assert_eq!(before.index_status, after.index_status);
            }
            let conn = db.conn().unwrap();
            let canonical: Vec<(String,i64,i64,Option<i64>)> = conn.prepare("SELECT volume_id,COUNT(*),COUNT(id),MAX(last_seen_at) FROM global_entries WHERE is_stale=0 GROUP BY volume_id ORDER BY volume_id").unwrap().query_map([],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().collect::<Result<_,_>>().unwrap();
            let facts: Vec<(String,i64,i64,Option<i64>)> = conn.prepare("SELECT volume_id,active_rows,active_ids,max_last_seen_at FROM global_entry_source_facts WHERE active_rows>0 ORDER BY volume_id").unwrap().query_map([],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().collect::<Result<_,_>>().unwrap();
            assert_eq!(canonical, facts);
            assert_eq!(
                conn.query_row::<i64, _, _>("PRAGMA user_version", [], |r| r.get(0))
                    .unwrap(),
                45
            );
            let indexes_after: Vec<(String, i64)> = conn
                .prepare("SELECT name,SUM(pgsize) FROM dbstat GROUP BY name ORDER BY name")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let backfill_plan:Vec<String>=conn.prepare("EXPLAIN QUERY PLAN SELECT volume_id,COUNT(*),COUNT(id),MAX(last_seen_at) FROM global_entries WHERE is_stale=0 GROUP BY volume_id").unwrap().query_map([],|r|r.get(3)).unwrap().collect::<Result<_,_>>().unwrap();
            let plan:Vec<String>=conn.prepare("EXPLAIN QUERY PLAN SELECT id,volume_id FROM global_entries INDEXED BY idx_global_entries_active_name_order WHERE is_stale=0 AND name_normalized GLOB 'quarterly*' ORDER BY name_normalized,modified_at_fs DESC,id LIMIT 80").unwrap().query_map([],|r|r.get(3)).unwrap().collect::<Result<_,_>>().unwrap();
            emit_record(
                &json!({"record_type":"upgrade_sample","rows":count,"short_ids":short,"starting_schema":starting_schema,"target_schema":45,"open_ms":elapsed_ms,"backfill_sql_ms":backfill_ms,"backfill_plan":backfill_plan,"preflight":{"available_memory_bytes":resources.available_memory_bytes,"key_bytes":resources.key_bytes,"facts_key_bytes":resources.facts_key_bytes,"page_size":resources.page_size,"page_count":resources.page_count,"freelist_pages":resources.freelist_pages,"db_reserved":resources.database_required_bytes,"temp_reserved":resources.temporary_required_bytes,"db_free":resources.database_free_bytes,"temp_free":resources.temporary_free_bytes},"files_before":before,"files_after":sizes(&path),"sampled_peak_files":peak_files,"sampled_process_rss":peak_rss,"samples":samples,"poll_sleep_ms":20,"controlled_copy_compacted":compacted,"copy_reshape_checkpoint_ms":preparation_ms,"minimum_database_free_bytes":min_db_free,"minimum_temp_free_bytes":min_temp_free,"memory_scope":"sampled process-wide current RSS; includes fixture/cache memory; not an OS peak or migration-attributed bound","indexes_before":indexes_before,"indexes_after":indexes_after,"name_plan":plan}),
            );
            drop(conn);
            drop(db);
            let reopened_started = Instant::now();
            let reopened = crate::db::schema_global_source_facts::with_backfill_hook(
                |_, _| panic!("45 reopen repeated backfill"),
                || Database::open(&path).unwrap(),
            );
            let reopened_ms = reopened_started.elapsed().as_secs_f64() * 1000.0;
            let reopened_snapshots = snapshots(&reopened);
            for (before, after) in after_snapshots.iter().zip(&reopened_snapshots) {
                assert_eq!(before.results, after.results);
                assert_eq!(before.source_revision, after.source_revision);
                assert_eq!(before.source_health, after.source_health);
                assert_eq!(before.index_status, after.index_status);
            }
            emit_record(
                &json!({"record_type":"upgrade_reopen","rows":count,"original_schema":starting_schema,"from":45,"to":45,"open_ms":reopened_ms,"repeated_backfill":false,"full_snapshots_equal":true}),
            );
            drop(reopened);
        }
    }
}
