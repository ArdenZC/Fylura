//! The #494 scale probe: one fixture, one runner, one build identity.
//!
//! Modes (environment driven, default off):
//!
//! * `ZC_ISSUE494_ENTRIES`   – 100000 | 500000 | 1000000
//! * `ZC_ISSUE494_FIXTURE`   – `official` (default) | `density`
//! * `ZC_ISSUE494_MAX_HITS`  – optional cap; queries whose oracle hit count
//!   exceeds it are measured with a shortened sample budget so a 50k-hit
//!   substring query cannot blow the job timeout.
//!
//! Every row states which call it timed. `b_filename_only` is RAM only and is
//! never presented as complete search latency; `b_full` includes the revision
//! read and SQLite hydration; `production_*` are the shipped path.

use super::bench::{self, ResourceSampler};
use super::candidate_a;
use super::candidate_b::{self, CompactView, NarrowCache};
use super::fixtures::{self, Fixture};
use super::oracle::{self, AuthorizedRow};
use super::{PAGE_LIMIT, SAMPLES, WARMUP};
use crate::db::Database;
use rusqlite::Connection;
use serde_json::json;

const OFFICIAL_QUERIES: [&str; 13] = [
    "zzznomatchtoken",
    "Annual Budget 2026.xlsx",
    "Set-0042",
    "Month-03",
    "report",
    "invoice",
    "IMG_",
    "数据库",
    "final-v2",
    "Résumé",
    "meeting-notes.md",
    "pdf",
    "jp",
];

fn configured_entries() -> u64 {
    let entries = std::env::var("ZC_ISSUE494_ENTRIES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(100_000);
    assert!(
        [100_000, 500_000, 1_000_000].contains(&entries),
        "ZC_ISSUE494_ENTRIES must be 100000, 500000 or 1000000"
    );
    entries
}

fn queries() -> Vec<&'static str> {
    match std::env::var("ZC_ISSUE494_FIXTURE").as_deref() {
        Ok("density") => fixtures::DENSITY_QUERIES
            .iter()
            .map(|(query, _)| *query)
            .collect(),
        _ => OFFICIAL_QUERIES.to_vec(),
    }
}

fn build_fixture(entries: u64) -> Fixture {
    match std::env::var("ZC_ISSUE494_FIXTURE").as_deref() {
        Ok("density") => fixtures::density(entries),
        _ => fixtures::official(entries),
    }
}

fn db_bytes(path: &std::path::Path) -> u64 {
    let main = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    let wal = std::fs::metadata(std::path::PathBuf::from(wal))
        .map(|meta| meta.len())
        .unwrap_or(0);
    main + wal
}

fn class_histogram(hits: &[super::Hit]) -> serde_json::Value {
    let mut counts = std::collections::BTreeMap::<&'static str, u64>::new();
    for hit in hits {
        *counts.entry(hit.class.as_str()).or_insert(0) += 1;
    }
    serde_json::to_value(counts).unwrap_or(serde_json::Value::Null)
}

fn ordered_ids(hits: &[super::Hit]) -> Vec<String> {
    hits.iter().map(|hit| hit.id.clone()).collect()
}

fn assert_equal(actual: &[super::Hit], expected: &[super::Hit], context: &str) {
    assert_eq!(
        ordered_ids(actual),
        ordered_ids(super::window_of(expected, super::MAX_WINDOW)),
        "{context}: candidate and oracle disagree"
    );
}

#[test]
#[ignore = "manual #494 A/B scale experiment; gated by ZC_ISSUE494_ENTRIES"]
fn issue494_filename_ab_scale_probe() {
    let entries = configured_entries();
    let fixture_kind = std::env::var("ZC_ISSUE494_FIXTURE").unwrap_or_else(|_| "official".into());
    let started = std::time::Instant::now();
    let fixture = build_fixture(entries);
    let fixture_ms = started.elapsed().as_secs_f64() * 1_000.0;
    bench::emit(bench::record(
        "fixture",
        json!({
            "fixture": fixture_kind,
            "entries": entries,
            "fixture_build_ms": fixture_ms,
            "db_bytes": db_bytes(&fixture.path),
            "note": "fixture_build_ms includes official generator, SQLite insert and FTS triggers",
        }),
    ));

    let conn: Connection = fixture.connection();

    // Phase 1: cold build of B with resource sampling, before any oracle fact
    // set is materialised, so the RSS delta is attributable to B.
    let sampler = ResourceSampler::start();
    let build_started = std::time::Instant::now();
    let view =
        CompactView::build(&conn, candidate_b::memory_budget_bytes()).expect("build compact view");
    let cold_build_ms = build_started.elapsed().as_secs_f64() * 1_000.0;
    let resources = sampler.finish();
    let view = view.expect("compact view must fit the configured budget");
    bench::emit(bench::record(
        "compact_view_build",
        json!({
            "fixture": fixture_kind,
            "entries": entries,
            "rows": view.rows(),
            "cold_build_ms": cold_build_ms,
            "stats": view.stats(),
            "revision": {
                "entry_count": view.revision().entry_count,
                "enabled_volumes": view.revision().enabled_volumes,
                "stale_entries": view.revision().stale_entries,
            },
            "resources": resources,
            "note": "estimated_total_bytes is capacity accounting; resources are sampled process RSS/CPU/IO around the build only",
        }),
    ));

    // Phase 2: correctness. The oracle fact set is materialised here and kept
    // for the whole run; it is deliberately built after the RSS sample.
    conn.execute_batch("BEGIN").expect("coherent read snapshot");
    let rows: Vec<AuthorizedRow> = oracle::authorized_rows(&conn).expect("authorized facts");
    let mut measured = 0usize;
    for query in queries() {
        let expected = oracle::proposed_name_path_oracle(&rows, query);
        let expected_extension = oracle::extension_oracle(&rows, query);
        let a = candidate_a::search(&conn, query, super::MAX_WINDOW).expect("candidate A");
        assert_equal(&a.name_path, &expected, &format!("A {query}"));
        let b =
            candidate_b::lookup(&view, &conn, query, super::MAX_WINDOW, None).expect("candidate B");
        assert_eq!(b.fence, candidate_b::Fence::Valid);
        assert_equal(&b.name_path, &expected, &format!("B {query}"));
        assert_equal(&b.extension, &expected_extension, &format!("B ext {query}"));
        assert_eq!(b.hydrated_dropped, 0);
        measured += 1;
    }
    assert_eq!(measured, queries().len(), "every query must be verified");

    // Phase 3: timing.
    for query in queries() {
        let expected = oracle::proposed_name_path_oracle(&rows, query);
        let hits = expected.len();
        let sample_budget = if hits > 20_000 { 5 } else { SAMPLES };

        let production_sql = bench::timed(
            || {
                let _ = crate::global_index::search::search_global_entries_on_connection(
                    &conn, query, PAGE_LIMIT, 0,
                )
                .expect("production sql search");
            },
            WARMUP,
            sample_budget,
        );
        let production_pooled = bench::timed(
            || {
                let _ = fixture
                    .db()
                    .search_global_entries(query, PAGE_LIMIT, 0)
                    .expect("pooled production search");
            },
            WARMUP,
            sample_budget,
        );
        let production_snapshot = bench::timed(
            || {
                let _ = fixture
                    .db()
                    .search_global_entries_snapshot(query, PAGE_LIMIT, 0)
                    .expect("production snapshot search");
            },
            WARMUP,
            sample_budget,
        );
        let a_full = bench::timed(
            || {
                let _ = candidate_a::search(&conn, query, super::MAX_WINDOW).expect("A");
            },
            WARMUP,
            sample_budget,
        );
        let a_enumeration = bench::timed(
            || {
                let _ = candidate_a::enumeration_only(&conn, query).expect("A enum");
            },
            WARMUP,
            sample_budget,
        );
        // A first page and the whole candidate window are different product
        // questions. Measuring only the 4096 window would compare B's
        // 4096-row hydration against production's 80-row hydration.
        let a_page = bench::timed(
            || {
                let _ = candidate_a::search(&conn, query, PAGE_LIMIT as usize).expect("A page");
            },
            WARMUP,
            sample_budget,
        );
        let b_filename_only = bench::timed(
            || {
                let _ = candidate_b::filename_only(&view, query, super::MAX_WINDOW, None);
            },
            WARMUP,
            sample_budget,
        );
        let b_filename_only_page = bench::timed(
            || {
                let _ = candidate_b::filename_only(&view, query, PAGE_LIMIT as usize, None);
            },
            WARMUP,
            sample_budget,
        );
        let b_full_page = bench::timed(
            || {
                let _ = candidate_b::lookup(&view, &conn, query, PAGE_LIMIT as usize, None)
                    .expect("B page");
            },
            WARMUP,
            sample_budget,
        );
        let b_full = bench::timed(
            || {
                let _ =
                    candidate_b::lookup(&view, &conn, query, super::MAX_WINDOW, None).expect("B");
            },
            WARMUP,
            sample_budget,
        );
        let b_extension = bench::timed(
            || {
                let _ = candidate_a::extension_channel(&conn, query, super::MAX_WINDOW)
                    .expect("extension channel");
            },
            WARMUP,
            sample_budget,
        );
        let oracle_scan = bench::once(|| {
            let _ = oracle::proposed_name_path_oracle(&rows, query);
        });
        let fts_matches = oracle::production_fts_match_count(&conn, query).expect("fts count");

        bench::emit(bench::record(
            "query",
            json!({
                "fixture": fixture_kind,
                "entries": entries,
                "query": query,
                "oracle_hits": hits,
                "oracle_classes": class_histogram(&expected),
                "production_fts_matches": fts_matches,
                "samples": sample_budget,
                "percentile_samples": production_sql.samples,
                "production_sql": production_sql.triple(),
                "production_pooled": production_pooled.triple(),
                "production_snapshot": production_snapshot.triple(),
                "production_sql_max_ms": production_sql.max,
                "production_sql_min_ms": production_sql.min,
                "a_full_max_ms": a_full.max,
                "b_full_min_ms": b_full.min,
                "a_full": a_full.triple(),
                "a_page": a_page.triple(),
                "a_enumeration_only": a_enumeration.triple(),
                "b_filename_only": b_filename_only.triple(),
                "b_filename_only_page": b_filename_only_page.triple(),
                "b_full": b_full.triple(),
                "b_full_page": b_full_page.triple(),
                "extension_channel_sql": b_extension.triple(),
                "oracle_full_scan_once_ms": oracle_scan,
                "note": "b_filename_only is RAM only; b_full adds revision read + SQLite hydration; percentiles are per-call, never summed",
            }),
        ));
    }

    // Phase 4: incremental write and rebuild cost.
    let incremental = incremental_cost(&fixture, &conn, entries);
    bench::emit(bench::record(
        "incremental",
        json!({
            "fixture": fixture_kind,
            "entries": entries,
            "incremental": incremental,
        }),
    ));

    conn.execute_batch("ROLLBACK").expect("end snapshot");
    drop(conn);
    drop(rows);
}

fn incremental_cost(fixture: &Fixture, conn: &Connection, entries: u64) -> serde_json::Value {
    const BATCH: u64 = 512;
    let db: &Database = fixture.db();
    let before_bytes = db_bytes(&fixture.path);

    let mut updates = Vec::new();
    for index in 0..BATCH {
        updates.push(crate::global_index::models::GlobalEntryInput {
            volume_id: "gv_test".to_string(),
            platform_file_id: format!("frn:C:\\Density\\Single\\incremental-{index:05}"),
            parent_platform_file_id: "frn:parent".to_string(),
            name: format!("incremental-{index:05}.txt"),
            path: format!(r"C:\Density\Single\incremental-{index:05}.txt"),
            extension: "txt".to_string(),
            is_directory: false,
            size: 2048,
            created_at_fs: Some(1_600_000_000),
            modified_at_fs: Some(1_800_000_000 + index as i64),
            file_attributes: 0,
            is_hidden: false,
            is_system: false,
            source_provider: "synthetic_issue494_density".to_string(),
            last_seen_at: 1_800_000_000,
        });
    }
    let insert_ms = bench::once(|| {
        db.upsert_global_entries_batch(&updates)
            .expect("incremental insert batch");
    });
    let after_bytes = db_bytes(&fixture.path);

    let rebuild_ms = bench::once(|| {
        let rebuilt =
            CompactView::build(conn, candidate_b::memory_budget_bytes()).expect("rebuild attempt");
        assert!(rebuilt.is_some(), "rebuild must fit the budget");
    });

    let delete_ms = bench::once(|| {
        db.conn()
            .expect("writer")
            .execute(
                "DELETE FROM global_entries WHERE name LIKE 'incremental-%'",
                [],
            )
            .expect("incremental delete batch");
    });
    let after_delete_bytes = db_bytes(&fixture.path);
    let rebuild_after_delete_ms = bench::once(|| {
        let rebuilt =
            CompactView::build(conn, candidate_b::memory_budget_bytes()).expect("rebuild attempt");
        assert!(rebuilt.is_some(), "rebuild must fit the budget");
    });

    // Narrowing cache: how much does one keystroke cost once a prefix is warm?
    let view = CompactView::build(conn, candidate_b::memory_budget_bytes())
        .expect("view")
        .expect("view fits");
    let cache = NarrowCache::build(&view, "density-five");
    let cache_len = cache.as_ref().map_or(0, NarrowCache::len);
    let narrowed_ms = bench::timed(
        || {
            let _ = candidate_b::filename_only(
                &view,
                "density-five-thousand",
                super::MAX_WINDOW,
                cache.as_ref(),
            );
        },
        WARMUP,
        SAMPLES,
    );
    let full_ms = bench::timed(
        || {
            let _ =
                candidate_b::filename_only(&view, "density-five-thousand", super::MAX_WINDOW, None);
        },
        WARMUP,
        SAMPLES,
    );

    json!({
        "entries": entries,
        "batch_rows": BATCH,
        "insert_512_ms": insert_ms,
        "delete_512_ms": delete_ms,
        "db_bytes_before": before_bytes,
        "db_bytes_after_insert": after_bytes,
        "db_bytes_after_delete": after_delete_bytes,
        "write_amplification_bytes_per_row": (after_bytes.saturating_sub(before_bytes)) as f64 / BATCH as f64,
        "full_rebuild_after_insert_ms": rebuild_ms,
        "full_rebuild_after_delete_ms": rebuild_after_delete_ms,
        "narrow_cache_slots": cache_len,
        "narrowed_query": narrowed_ms.triple(),
        "full_scan_query": full_ms.triple(),
        "note": "B has no incremental patch path: any revision change forces a full rebuild, which is what these numbers price",
    })
}
