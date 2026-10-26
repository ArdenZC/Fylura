//! #474: one dataset, canonical/prepared facts and an optional exact FTS
//! rank-frontier control. Includes sustained real Repository producer costs.
use super::*;
use crate::global_index::repository::{
    diagnostic_load_production_source_health, with_canonical_source_facts,
};
use crate::global_index::search::{diagnostic_search_tier, rank_frontier};
use crate::global_index::GlobalEntryInput;
fn variant<T>(v: usize, run: impl FnOnce() -> T) -> T {
    match v {
        0 => rank_frontier::with_baseline(|| with_canonical_source_facts(run)),
        1 => rank_frontier::with_baseline(run),
        2 => rank_frontier::with_candidate(run),
        _ => unreachable!(),
    }
}
fn time<T>(run: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let r = run();
    (r, start.elapsed().as_secs_f64() * 1000.)
}
fn same(
    a: &crate::global_index::repository::GlobalSearchSnapshot,
    b: &crate::global_index::repository::GlobalSearchSnapshot,
) {
    assert_eq!(a.results, b.results);
    assert_eq!(a.source_health, b.source_health);
    assert_eq!(a.source_revision.as_bytes(), b.source_revision.as_bytes());
    assert_eq!(a.index_status, b.index_status);
}
fn input(n: u64) -> GlobalEntryInput {
    let e = generated_entry(n);
    GlobalEntryInput {
        volume_id: "writer".into(),
        platform_file_id: e.platform_file_id,
        parent_platform_file_id: e.parent_platform_file_id,
        name: e.name,
        path: e.path,
        extension: e.extension,
        is_directory: false,
        size: e.size,
        created_at_fs: Some(e.created_at_fs),
        modified_at_fs: Some(e.modified_at_fs),
        file_attributes: e.file_attributes,
        is_hidden: e.is_hidden,
        is_system: e.is_system,
        source_provider: "synthetic_global_search_benchmark".into(),
        last_seen_at: 1_700_000_000 + n as i64,
    }
}
fn pool_changes(db: &Database) -> u64 {
    let handles = (0..8).map(|_| db.conn().unwrap()).collect::<Vec<_>>();
    handles.iter().map(|c| c.total_changes()).sum()
}
fn producer_cost(db: &Database, path: &Path, entries: u64, context: &JsonValue, v: usize) {
    let mut volume = test_volume();
    volume.id = "writer".into();
    volume.stable_volume_id = "writer".into();
    db.upsert_global_volume(&volume).unwrap();
    let mut rows = (0..15360)
        .map(|n| input(entries + 10000 + n))
        .collect::<Vec<_>>();
    for stage in ["insert", "metadata_update", "identical_replay", "rename"] {
        if stage == "metadata_update" {
            for e in &mut rows {
                e.last_seen_at += 2_000_000;
            }
        }
        if stage == "rename" {
            for e in &mut rows {
                e.name.push_str("-renamed");
            }
        }
        let conn = configured_connection(path).unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let before = pool_changes(db);
        let mut samples = Vec::new();
        for batch in rows.chunks(512) {
            let (_, ms) = time(|| db.upsert_global_entries_batch(batch).unwrap());
            samples.push(ms);
        }
        let writes = pool_changes(db) - before;
        emit_record(
            &json!({"record_type":"facts_writer","context":context,"variant":v,"stage":stage,
   "rows":rows.len(),"batches":30,"batch_rows":512,"api_ms":summarize_samples(&samples),
   "sqlite_total_changes_including_triggers":writes,"wal_bytes":sidecar_bytes(path,"-wal"),
   "scope":"committed actual Repository upsert batches with original FTS/count triggers, scope/AI checks and pool; wall time includes writer reservation; not pure lock hold"}),
        );
    }
    let mut samples = Vec::new();
    // Worst recency deletions exercise exact max repair; independent commits are
    // the real per-entry provider/Watcher retirement path, not rollback inserts.
    for batch in rows.iter().rev().take(480).collect::<Vec<_>>().chunks(16) {
        let (_, ms) = time(|| {
            for e in batch {
                db.mark_global_entry_stale(&e.entry_id()).unwrap();
            }
        });
        samples.push(ms);
    }
    emit_record(
        &json!({"record_type":"facts_writer","context":context,"variant":v,"stage":"retire_max_descending",
  "rows":480,"batches":30,"batch_rows":16,"api_ms":summarize_samples(&samples)}),
    );
    let (_, bulk_ms) = time(|| db.mark_global_entries_stale_for_volume("writer").unwrap());
    let actual = db
        .search_global_entries_snapshot("zzznomatchtoken", 80, 0)
        .unwrap();
    let canonical = with_canonical_source_facts(|| {
        db.search_global_entries_snapshot("zzznomatchtoken", 80, 0)
            .unwrap()
    });
    same(&actual, &canonical);
    emit_record(
        &json!({"record_type":"facts_writer_bulk","context":context,"variant":v,
  "stage":"retire_remaining_volume","wall_ms":bulk_ms,"source_snapshot_canonical_equal":true}),
    );
}
#[test]
#[ignore = "#474 bounded Source Facts and exact FTS frontier pair; 100k then at most one 1M"]
fn source_facts_snapshot_pair() {
    let entries = configured_entries();
    assert!([100_000, 1_000_000].contains(&entries));
    let mut context = benchmark_context(entries);
    context["source_tree"] = json!(env::var("ZEN_CANVAS_BENCHMARK_SOURCE_TREE").ok());
    let paths = (0..2).map(|_| test_db_path()).collect::<Vec<_>>();
    let _cleanup = paths
        .iter()
        .cloned()
        .map(BenchmarkCleanup)
        .collect::<Vec<_>>();
    let db = Database::open(&paths[0]).unwrap();
    db.upsert_global_volume(&test_volume()).unwrap();
    let (expectations, gen_ms, pop_ms, batches) = populate_synthetic_database(&db, entries);
    let c = configured_connection(&paths[0]).unwrap();
    assert_entry_triggers_enabled(&c);
    c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    drop(c);
    drop(db);
    fs::copy(&paths[0], &paths[1]).unwrap();
    let dbs = paths
        .iter()
        .map(|p| Database::open(p).unwrap())
        .collect::<Vec<_>>();
    let conns = paths
        .iter()
        .map(|p| configured_connection(p).unwrap())
        .collect::<Vec<_>>();
    // Keep schema and complete rows identical; baseline disables only the new
    // write maintenance after opening. Its canonical reads never use stale facts.
    conns[0].execute_batch("DROP TRIGGER global_entry_source_facts_ai;DROP TRIGGER global_entry_source_facts_ad;DROP TRIGGER global_entry_source_facts_au;
 DROP TRIGGER global_entry_source_facts_volume_ai;DROP TRIGGER global_entry_source_facts_volume_au;").unwrap();
    let include_frontier = env::var("ZC_FACTS_FTS_FRONTIER").as_deref() == Ok("1");
    let variants = if include_frontier { 3 } else { 2 };
    emit_record(
        &json!({"record_type":"facts_fixture","context":context,"fixture_generations":1,
  "controlled_copies":1,"generation_ms":gen_ms,"population_ms":pop_ms,"batch_ms":summarize_samples(&batches),
  "variants":variants,"warmups":5,"samples":30,"query_limit":80,"candidate_window":4096,"fts_test_only":include_frontier}),
    );
    for (q, case) in QUERY_MATRIX.iter().copied().enumerate() {
        let oracle = variant(0, || {
            dbs[0]
                .search_global_entries_snapshot(case.query, 80, 0)
                .unwrap()
        });
        assert_eq!(
            oracle.results,
            assert_search_results(&conns[0], case, &expectations[q], 80, 0, entries)
        );
        let mut samples = vec![vec![Vec::<f64>::new(); 5]; variants];
        for round in 0..35 {
            for slot in 0..variants {
                let v = (round + slot) % variants;
                let physical = usize::from(v != 0);
                let db = &dbs[physical];
                let conn = &conns[physical];
                variant(v, || {
                    let (sql, a) = time(|| {
                        search_global_entries_on_connection(conn, case.query, 80, 0).unwrap()
                    });
                    let (pooled, b) = time(|| db.search_global_entries(case.query, 80, 0).unwrap());
                    let (health, c) =
                        time(|| diagnostic_load_production_source_health(conn).unwrap());
                    let (status, d) = time(|| db.global_index_status().unwrap());
                    let (snapshot, e) = time(|| {
                        db.search_global_entries_snapshot(case.query, 80, 0)
                            .unwrap()
                    });
                    assert_eq!(sql, oracle.results);
                    assert_eq!(pooled, oracle.results);
                    assert_eq!(health.0, oracle.source_health);
                    assert_eq!(health.1, oracle.source_revision);
                    assert_eq!(status, oracle.index_status);
                    same(&oracle, &snapshot);
                    if round >= 5 {
                        for (i, ms) in [a, b, c, d, e].into_iter().enumerate() {
                            samples[v][i].push(ms);
                        }
                        emit_record(
                            &json!({"record_type":"facts_sample","context":context,"query_class":case.class,"variant":v,
      "sample":round-5,"sql_route_ms":a,"pooled_ms":b,"source_health_ms":c,"index_status_ms":d,"snapshot_ms":e}),
                        );
                    }
                });
            }
        }
        for v in 0..variants {
            let physical = usize::from(v != 0);
            variant(v, || {
                for (i, name) in [
                    "sql_route",
                    "pooled",
                    "source_health",
                    "index_status",
                    "snapshot",
                ]
                .iter()
                .enumerate()
                {
                    emit_record(
                        &json!({"record_type":"facts_summary","context":context,"query_class":case.class,"variant":v,
      "stage":name,"latency_ms":summarize_samples(&samples[v][i])}),
                    );
                }
                for (limit, offset) in [(1, 0), (80, 40), (200, 4016), (80, 4096)] {
                    same(
                        &variant(0, || {
                            dbs[0]
                                .search_global_entries_snapshot(case.query, limit, offset)
                                .unwrap()
                        }),
                        &dbs[physical]
                            .search_global_entries_snapshot(case.query, limit, offset)
                            .unwrap(),
                    );
                }
                for tier in super::query_cost_diagnostic::exact_sql_1m_tier_order(case.query) {
                    let expected = variant(0, || {
                        diagnostic_search_tier(&conns[0], tier, case.query, 80).unwrap()
                    });
                    let mut timings = Vec::new();
                    for round in 0..35 {
                        let (rows, ms) = time(|| {
                            diagnostic_search_tier(&conns[physical], tier, case.query, 80).unwrap()
                        });
                        assert_eq!(rows, expected);
                        if round >= 5 {
                            timings.push(ms);
                        }
                    }
                    emit_record(
                        &json!({"record_type":"facts_sql_tier","context":context,"query_class":case.class,"variant":v,
      "tier":tier,"latency_ms":summarize_samples(&timings)}),
                    );
                }
                let trace = super::covering_indexes::trace_snapshot(
                    &dbs[physical],
                    &conns[physical],
                    case.query,
                );
                emit_record(
                    &json!({"record_type":"facts_trace","context":context,"query_class":case.class,"variant":v,"statements":trace}),
                );
            });
        }
        eprintln!(
            "facts_pair_progress rows={entries} class={} PASS",
            case.class
        );
    }
    for v in 0..2 {
        variant(v, || {
            producer_cost(&dbs[v], &paths[v], entries, &context, v)
        });
    }
    emit_record(
        &json!({"record_type":"facts_terminal","context":context,"status":"PASS","correctness_classes":12,
  "all_fields_bm25_pages_health_revision_status_equal":true}),
    );
}

#[test]
fn prepared_source_facts_preserve_real_snapshot_contracts() {
    use crate::global_index::repository::diagnostic_snapshot_with_prepared_facts;
    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).unwrap();
    db.upsert_global_volume(&test_volume()).unwrap();
    for id in ["empty", "disabled"] {
        let mut v = test_volume();
        v.id = id.into();
        v.stable_volume_id = id.into();
        v.enabled = id != "disabled";
        db.upsert_global_volume(&v).unwrap();
    }
    let mut rows = (0..128)
        .map(|n| {
            let mut e = input(n);
            e.volume_id = "gv_test".into();
            e
        })
        .collect::<Vec<_>>();
    for e in rows.iter_mut().skip(64) {
        e.volume_id = "disabled".into();
    }
    db.upsert_global_entries_batch(&rows).unwrap();
    let mut conn = configured_connection(&path).unwrap();
    conn.execute_batch(
        "UPDATE global_entries SET id=NULL WHERE rowid=1;
     UPDATE global_volumes SET entry_count=987654;",
    )
    .unwrap();
    let check = || {
        let actual = db
            .search_global_entries_snapshot("zzznomatchtoken", 80, 0)
            .unwrap();
        let canonical = with_canonical_source_facts(|| {
            db.search_global_entries_snapshot("zzznomatchtoken", 80, 0)
                .unwrap()
        });
        same(&actual, &canonical);
    };
    check();
    for sql in [
        "UPDATE global_entries SET last_seen_at=500 WHERE rowid=2",
        "UPDATE global_entries SET last_seen_at=1 WHERE rowid=2",
        "DELETE FROM global_entries WHERE rowid=128",
        "UPDATE global_entries SET volume_id='disabled' WHERE rowid=127",
        "UPDATE global_entries SET is_stale=1 WHERE rowid%3=0",
        "UPDATE global_entries SET is_stale=0 WHERE rowid%3=0",
        "UPDATE global_entries SET id='now-nonnull' WHERE rowid=1",
    ] {
        conn.execute_batch(sql).unwrap();
        check();
    }
    for status in [
        "ready",
        "discovered",
        "indexing",
        "syncing",
        "rebuild_required",
        "paused",
        "error",
        "permission_required",
        "spotlight_not_indexed",
        "spotlight_unavailable",
        "spotlight_external_not_indexed",
        "fsevents_unavailable",
        "unavailable",
    ] {
        for enabled in [0, 1] {
            conn.execute("UPDATE global_volumes SET index_status=?1, enabled=?2,last_error='preserved error'",params![status,enabled]).unwrap();
            check();
        }
    }
    // A committed writer on another connection cannot change an already-pinned
    // search/facts/status read transaction, including NULL/count/MAX transitions.
    let tx = conn.transaction().unwrap();
    let before = diagnostic_snapshot_with_prepared_facts(&tx, "zzznomatchtoken").unwrap();
    db.mark_global_entries_stale_for_volume("gv_test").unwrap();
    same(
        &before,
        &diagnostic_snapshot_with_prepared_facts(&tx, "zzznomatchtoken").unwrap(),
    );
    same(
        &before,
        &with_canonical_source_facts(|| {
            diagnostic_snapshot_with_prepared_facts(&tx, "zzznomatchtoken").unwrap()
        }),
    );
    tx.commit().unwrap();
    check();
    assert_ne!(
        before.source_revision,
        db.search_global_entries_snapshot("zzznomatchtoken", 80, 0)
            .unwrap()
            .source_revision
    );
    conn.execute_batch("DELETE FROM global_entries").unwrap();
    check();
    conn.execute_batch("DELETE FROM global_volumes").unwrap();
    check();
}
