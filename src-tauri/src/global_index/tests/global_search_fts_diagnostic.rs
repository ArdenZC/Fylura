use super::{
    benchmark_context, configured_connection, emit_record, populate_synthetic_database,
    summarize_samples, BenchmarkCleanup, SEARCH_RESULT_LIMIT,
};
use crate::db::Database;
use crate::global_index::models::GlobalSearchResult;
use crate::global_index::search::{
    diagnostic_search_fts, diagnostic_search_fts_sql, diagnostic_search_tier,
    diagnostic_search_tier_sql, search_global_entries_on_connection,
};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Connection};
use serde_json::{json, Value as JsonValue};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

const DIAGNOSTIC_ENTRIES: u64 = 100_000;
const SAMPLE_COUNT: usize = 10;
const EXPENSIVE_QUERY_CUTOFF_MS: f64 = 1_000.0;
const FTS_QUERIES: [(&str, &str); 2] = [
    ("fts_substring_report", "report"),
    ("fts_substring_invoice", "invoice"),
];

const VARIANT_SAFE_CTE: &str = r#"
    WITH candidates AS MATERIALIZED (
        SELECT ge.rowid AS entry_rowid,
               ge.id AS entry_id,
               ge.modified_at_fs AS modified_at_fs,
               bm25(global_entries_fts, 8.0, 2.0, 1.0) AS rank
        FROM global_entries_fts
        JOIN global_entries ge ON ge.rowid = global_entries_fts.rowid
        JOIN global_volumes gv ON gv.id = ge.volume_id
        WHERE global_entries_fts MATCH ?1
          AND gv.enabled = 1
          AND ge.is_stale = 0
        ORDER BY rank ASC, ge.modified_at_fs DESC, ge.id ASC
        LIMIT ?2
    )
    SELECT ge.id, ge.volume_id, ge.platform_file_id, ge.name, ge.path,
           ge.extension, ge.is_directory, ge.size, ge.created_at_fs,
           ge.modified_at_fs, ge.file_attributes, ge.is_hidden, ge.is_system,
           ge.source_provider,
           EXISTS (
               SELECT 1
               FROM managed_entries me
               JOIN managed_scopes ms ON ms.id = me.managed_scope_id
               WHERE me.global_entry_id = ge.id
                 AND me.enabled = 1
                 AND ms.enabled = 1
           ) AS managed,
           candidates.rank AS rank
    FROM candidates
    JOIN global_entries ge ON ge.rowid = candidates.entry_rowid
    ORDER BY candidates.rank ASC,
             candidates.modified_at_fs DESC,
             candidates.entry_id ASC
"#;

const VARIANT_CROSS_JOIN: &str = r#"
    SELECT ge.id, ge.volume_id, ge.platform_file_id, ge.name, ge.path,
           ge.extension, ge.is_directory, ge.size, ge.created_at_fs,
           ge.modified_at_fs, ge.file_attributes, ge.is_hidden, ge.is_system,
           ge.source_provider,
           EXISTS (
               SELECT 1
               FROM managed_entries me
               JOIN managed_scopes ms ON ms.id = me.managed_scope_id
               WHERE me.global_entry_id = ge.id
                 AND me.enabled = 1
                 AND ms.enabled = 1
           ) AS managed,
           bm25(global_entries_fts, 8.0, 2.0, 1.0) AS rank
    FROM global_entries_fts
    CROSS JOIN global_entries ge
    CROSS JOIN global_volumes gv
    WHERE global_entries_fts MATCH ?1
      AND ge.rowid = global_entries_fts.rowid
      AND gv.id = ge.volume_id
      AND gv.enabled = 1
      AND ge.is_stale = 0
    ORDER BY rank ASC, ge.modified_at_fs DESC, ge.id ASC
    LIMIT ?2
"#;

const VARIANT_UNSAFE_POSTFILTER: &str = r#"
    WITH candidates AS MATERIALIZED (
        SELECT rowid AS entry_rowid,
               bm25(global_entries_fts, 8.0, 2.0, 1.0) AS rank
        FROM global_entries_fts
        WHERE global_entries_fts MATCH ?1
        ORDER BY rank ASC, rowid ASC
        LIMIT ?2
    )
    SELECT ge.id, ge.volume_id, ge.platform_file_id, ge.name, ge.path,
           ge.extension, ge.is_directory, ge.size, ge.created_at_fs,
           ge.modified_at_fs, ge.file_attributes, ge.is_hidden, ge.is_system,
           ge.source_provider,
           EXISTS (
               SELECT 1
               FROM managed_entries me
               JOIN managed_scopes ms ON ms.id = me.managed_scope_id
               WHERE me.global_entry_id = ge.id
                 AND me.enabled = 1
                 AND ms.enabled = 1
           ) AS managed,
           candidates.rank AS rank
    FROM candidates
    JOIN global_entries ge ON ge.rowid = candidates.entry_rowid
    JOIN global_volumes gv ON gv.id = ge.volume_id
    WHERE gv.enabled = 1 AND ge.is_stale = 0
    ORDER BY candidates.rank ASC, ge.modified_at_fs DESC, ge.id ASC
    LIMIT ?2
"#;

fn fts_phrase(query: &str) -> String {
    format!("\"{}\"", query.replace('"', "\"\""))
}

fn measure<T>(mut run: impl FnMut() -> T) -> (Vec<f64>, T, &'static str) {
    let started = Instant::now();
    let mut last = run();
    let first_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let mut samples = vec![first_ms];
    if first_ms < EXPENSIVE_QUERY_CUTOFF_MS {
        for _ in 1..SAMPLE_COUNT {
            let started = Instant::now();
            last = run();
            samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        }
        (samples, last, "10 samples when first sample < 1000 ms")
    } else {
        (
            samples,
            last,
            "single sample because first sample >= 1000 ms",
        )
    }
}

fn explain_details(conn: &Connection, sql: &str, values: &[SqlValue]) -> Vec<String> {
    let explain = format!("EXPLAIN QUERY PLAN {sql}");
    let mut statement = conn.prepare(&explain).expect("prepare staged query plan");
    statement
        .query_map(params_from_iter(values.iter()), |row| {
            row.get::<_, String>(3)
        })
        .expect("execute staged query plan")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect staged query plan")
}

fn entry_ids_for_rowids(conn: &Connection, rowids: &[i64]) -> Vec<String> {
    rowids
        .iter()
        .take(10)
        .map(|rowid| {
            conn.query_row(
                "SELECT id FROM global_entries WHERE rowid = ?1",
                [rowid],
                |row| row.get(0),
            )
            .expect("resolve staged FTS rowid for evidence")
        })
        .collect()
}

fn map_search_result(row: &rusqlite::Row<'_>) -> rusqlite::Result<GlobalSearchResult> {
    Ok(GlobalSearchResult {
        id: row.get(0)?,
        volume_id: row.get(1)?,
        platform_file_id: row.get(2)?,
        name: row.get(3)?,
        path: row.get(4)?,
        extension: row.get(5)?,
        is_directory: row.get::<_, i64>(6)? != 0,
        size: row.get(7)?,
        created_at_fs: row.get(8)?,
        modified_at_fs: row.get(9)?,
        file_attributes: row.get(10)?,
        is_hidden: row.get::<_, i64>(11)? != 0,
        is_system: row.get::<_, i64>(12)? != 0,
        source_provider: row.get(13)?,
        managed: row.get::<_, i64>(14)? != 0,
        rank: row.get(15)?,
    })
}

fn run_variant(conn: &Connection, sql: &str, query: &str, limit: u32) -> Vec<GlobalSearchResult> {
    let mut statement = conn.prepare(sql).expect("prepare diagnostic SQL variant");
    statement
        .query_map(params![fts_phrase(query), limit], map_search_result)
        .expect("execute diagnostic SQL variant")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect diagnostic SQL variant")
}

fn search_ids(results: &[GlobalSearchResult]) -> Vec<String> {
    results.iter().map(|result| result.id.clone()).collect()
}

// Keep the diagnostic's serialized stage fields explicit at this test-only boundary.
#[allow(clippy::too_many_arguments)]
fn emit_stage_record(
    context: &JsonValue,
    query_class: &str,
    query: &str,
    stage: &str,
    sql: &str,
    values: &[SqlValue],
    samples: &[f64],
    count: usize,
    first_ids: Vec<String>,
    sample_policy: &str,
    result_note: &str,
    conn: &Connection,
) {
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "fts_stage",
        "context": context,
        "query_class": query_class,
        "query": query,
        "stage": stage,
        "result_count": count,
        "first_ids": first_ids,
        "latency_ms": summarize_samples(samples),
        "sample_policy": sample_policy,
        "sql": sql,
        "explain_query_plan": explain_details(conn, sql, values),
        "result_note": result_note
    }));
}

fn add_volume(db: &Database, id: &str, enabled: bool) {
    let mut volume = super::super::test_volume();
    volume.id = id.to_string();
    volume.stable_volume_id = format!("synthetic-{id}");
    volume.display_name = id.to_string();
    volume.mount_path = format!(r"C:\{id}\");
    volume.enabled = enabled;
    db.upsert_global_volume(&volume)
        .expect("add adversarial diagnostic volume");
}

fn insert_entry(
    conn: &Connection,
    id: &str,
    volume_id: &str,
    name: &str,
    path: &str,
    modified_at: i64,
) {
    let normalized_name = name.to_lowercase();
    let normalized_path = path.to_lowercase();
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .unwrap_or("");
    conn.execute(
        r#"
        INSERT INTO global_entries (
            id, volume_id, platform_file_id, parent_platform_file_id,
            name, name_normalized, path, path_normalized, extension,
            is_directory, size, created_at_fs, modified_at_fs,
            file_attributes, is_hidden, is_system, is_stale,
            source_provider, last_seen_at
        ) VALUES (
            ?1, ?2, ?3, 'diagnostic-parent', ?4, ?5, ?6, ?7, ?8,
            0, 1, 1700000000, ?9, 0, 0, 0, 0,
            'issue_346_diagnostic', 1700000000
        )
        "#,
        params![
            id,
            volume_id,
            id,
            name,
            normalized_name,
            path,
            normalized_path,
            extension,
            modified_at,
        ],
    )
    .expect("insert adversarial diagnostic entry with production FTS trigger");
}

fn add_adversarial_rows(db: &Database) {
    add_volume(db, "gv_second_enabled", true);
    add_volume(db, "gv_disabled", false);
    let conn = db.conn().expect("open adversarial row connection");
    let tx = conn
        .unchecked_transaction()
        .expect("start adversarial fixture transaction");

    let invalid_name = format!("ZZZ audit {}.txt", "report ".repeat(16));
    for index in 0..130 {
        insert_entry(
            &tx,
            &format!("ge_disabled_{index:03}"),
            "gv_disabled",
            &invalid_name,
            &format!(r"C:\disabled\report-{index:03}.txt"),
            1_800_000_000 + index,
        );
        insert_entry(
            &tx,
            &format!("ge_stale_{index:03}"),
            "gv_test",
            &invalid_name,
            &format!(r"C:\stale\report-{index:03}.txt"),
            1_800_000_000 + index,
        );
    }
    tx.execute(
        "UPDATE global_entries SET is_stale = 1 WHERE id LIKE 'ge_stale_%'",
        [],
    )
    .expect("mark stale adversarial rows");

    let tied_name = format!("ZZZ audit {}.txt", "report ".repeat(10));
    let tied_path = r"C:diagnosticduplicateZZZ audit report tie.txt";
    insert_entry(
        &tx,
        "ge_tie_a",
        "gv_test",
        &tied_name,
        tied_path,
        1_790_000_000,
    );
    insert_entry(
        &tx,
        "ge_tie_b",
        "gv_second_enabled",
        &tied_name,
        tied_path,
        1_790_000_000,
    );
    tx.execute_batch(
        r#"
        INSERT INTO managed_scopes (
            id, path, global_entry_id, enabled, allow_local_ai, allow_cloud_ai,
            created_at, updated_at
        ) VALUES (
            'scope_issue_346', 'C:\diagnostic\duplicate', 'ge_tie_b',
            1, 1, 0, 1700000000, 1700000000
        );
        INSERT INTO managed_entries (
            id, global_entry_id, managed_scope_id, enabled, created_at, updated_at
        ) VALUES (
            'managed_issue_346', 'ge_tie_b', 'scope_issue_346',
            1, 1700000000, 1700000000
        );
        "#,
    )
    .expect("add managed marker for search projection equivalence");
    tx.commit().expect("commit adversarial diagnostic rows");
}

fn stage_sql(stage: &str) -> (&'static str, bool) {
    match stage {
        "A_fts_match_only" => (
            "SELECT global_entries_fts.rowid FROM global_entries_fts WHERE global_entries_fts MATCH ?1 LIMIT ?2",
            false,
        ),
        "B_match_bm25_order" => (
            "SELECT global_entries_fts.rowid FROM global_entries_fts WHERE global_entries_fts MATCH ?1 ORDER BY bm25(global_entries_fts, 8.0, 2.0, 1.0) ASC LIMIT ?2",
            false,
        ),
        "C_join_global_entries" => (
            "SELECT ge.rowid FROM global_entries_fts JOIN global_entries ge ON ge.rowid = global_entries_fts.rowid WHERE global_entries_fts MATCH ?1 ORDER BY bm25(global_entries_fts, 8.0, 2.0, 1.0) ASC, ge.modified_at_fs DESC, ge.id ASC LIMIT ?2",
            false,
        ),
        "D_enabled_volume_and_not_stale" => (
            "SELECT ge.rowid FROM global_entries_fts JOIN global_entries ge ON ge.rowid = global_entries_fts.rowid JOIN global_volumes gv ON gv.id = ge.volume_id WHERE global_entries_fts MATCH ?1 AND gv.enabled = 1 AND ge.is_stale = 0 ORDER BY bm25(global_entries_fts, 8.0, 2.0, 1.0) ASC, ge.modified_at_fs DESC, ge.id ASC LIMIT ?2",
            false,
        ),
        "E_managed_exists" => (
            "SELECT ge.rowid, EXISTS (SELECT 1 FROM managed_entries me JOIN managed_scopes ms ON ms.id = me.managed_scope_id WHERE me.global_entry_id = ge.id AND me.enabled = 1 AND ms.enabled = 1) AS managed FROM global_entries_fts JOIN global_entries ge ON ge.rowid = global_entries_fts.rowid JOIN global_volumes gv ON gv.id = ge.volume_id WHERE global_entries_fts MATCH ?1 AND gv.enabled = 1 AND ge.is_stale = 0 ORDER BY bm25(global_entries_fts, 8.0, 2.0, 1.0) ASC, ge.modified_at_fs DESC, ge.id ASC LIMIT ?2",
            true,
        ),
        _ => panic!("unknown staged query: {stage}"),
    }
}

fn run_staged_query(
    conn: &Connection,
    sql: &str,
    query: &str,
    limit: u32,
    with_managed: bool,
) -> Vec<(i64, bool)> {
    let mut statement = conn.prepare(sql).expect("prepare staged query");
    statement
        .query_map(params![fts_phrase(query), limit], |row| {
            Ok((
                row.get(0)?,
                if with_managed {
                    row.get::<_, i64>(1)? != 0
                } else {
                    false
                },
            ))
        })
        .expect("execute staged query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect staged query")
}

// Keep the diagnostic's serialized stage fields explicit at this test-only boundary.
#[allow(clippy::too_many_arguments)]
fn stage_result_record(
    context: &JsonValue,
    query_class: &str,
    query: &str,
    stage: &str,
    sql: &str,
    samples: &[f64],
    rowids: &[(i64, bool)],
    sample_policy: &str,
    conn: &Connection,
) {
    let values = vec![
        SqlValue::Text(fts_phrase(query)),
        SqlValue::Integer(SEARCH_RESULT_LIMIT as i64),
    ];
    let first_ids = entry_ids_for_rowids(
        conn,
        &rowids.iter().map(|(rowid, _)| *rowid).collect::<Vec<_>>(),
    );
    let details = explain_details(conn, sql, &values);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "fts_stage",
        "context": context,
        "query_class": query_class,
        "query": query,
        "stage": stage,
        "result_count": rowids.len(),
        "first_ids": first_ids,
        "latency_ms": summarize_samples(samples),
        "sample_policy": sample_policy,
        "sql": sql,
        "explain_query_plan": details,
        "result_note": if stage == "E_managed_exists" {
            format!("managed_true_count={}", rowids.iter().filter(|(_, managed)| *managed).count())
        } else {
            "".to_string()
        }
    }));
}

fn assert_fts_is_only_match_tier(conn: &Connection, query: &str) {
    for tier in [
        "exact_name",
        "name_prefix",
        "exact_extension",
        "extension_prefix",
    ] {
        let results = diagnostic_search_tier(conn, tier, query, SEARCH_RESULT_LIMIT)
            .unwrap_or_else(|error| panic!("run production {tier} tier: {error}"));
        assert!(results.is_empty(), "{tier} unexpectedly matches {query:?}");
    }
}

fn profile_fts_query(
    conn: &Connection,
    context: &JsonValue,
    query_class: &str,
    query: &str,
) -> Vec<GlobalSearchResult> {
    assert_fts_is_only_match_tier(conn, query);
    let fts_value = fts_phrase(query);
    for stage in [
        "A_fts_match_only",
        "B_match_bm25_order",
        "C_join_global_entries",
        "D_enabled_volume_and_not_stale",
        "E_managed_exists",
    ] {
        let (sql, with_managed) = stage_sql(stage);
        let (samples, rowids, policy) =
            measure(|| run_staged_query(conn, sql, query, SEARCH_RESULT_LIMIT, with_managed));
        stage_result_record(
            context,
            query_class,
            query,
            stage,
            sql,
            &samples,
            &rowids,
            policy,
            conn,
        );
    }

    let sql = diagnostic_search_fts_sql();
    let values = vec![
        SqlValue::Text(fts_value),
        SqlValue::Integer(SEARCH_RESULT_LIMIT as i64),
    ];
    let (samples, results, policy) = measure(|| {
        diagnostic_search_fts(conn, query, SEARCH_RESULT_LIMIT)
            .expect("run exact production FTS tier")
    });
    if query == "report" {
        assert_adversarial_results(&results);
    }
    emit_stage_record(
        context,
        query_class,
        query,
        "F_production_full_fts_sql",
        &sql,
        &values,
        &samples,
        results.len(),
        search_ids(&results),
        policy,
        "exact production search_fts function; the four earlier production tiers were independently verified empty",
        conn,
    );
    results
}

fn no_result_tier_sql(tier: &str, query: &str) -> (String, Vec<SqlValue>) {
    let escaped = query
        .to_lowercase()
        .replace('*', "[*]")
        .replace('?', "[?]")
        .replace('[', "[[]");
    match tier {
        "exact_name" => (
            diagnostic_search_tier_sql("exact_name"),
            vec![
                SqlValue::Text(query.to_string()),
                SqlValue::Integer(SEARCH_RESULT_LIMIT as i64),
            ],
        ),
        "name_prefix" => (
            diagnostic_search_tier_sql("name_prefix"),
            vec![
                SqlValue::Text(query.to_string()),
                SqlValue::Text(format!("{escaped}*")),
                SqlValue::Integer(SEARCH_RESULT_LIMIT as i64),
            ],
        ),
        "exact_extension" => (
            diagnostic_search_tier_sql("exact_extension"),
            vec![
                SqlValue::Text(query.to_string()),
                SqlValue::Integer(SEARCH_RESULT_LIMIT as i64),
            ],
        ),
        "extension_prefix" => (
            diagnostic_search_tier_sql("extension_prefix"),
            vec![
                SqlValue::Text(query.to_string()),
                SqlValue::Text(format!("{escaped}*")),
                SqlValue::Integer(SEARCH_RESULT_LIMIT as i64),
            ],
        ),
        "fts" => (
            diagnostic_search_fts_sql(),
            vec![
                SqlValue::Text(fts_phrase(query)),
                SqlValue::Integer(SEARCH_RESULT_LIMIT as i64),
            ],
        ),
        _ => panic!("unknown no-result tier: {tier}"),
    }
}

fn profile_no_result(conn: &Connection, context: &JsonValue) {
    let query = "zzznomatchtoken";
    let tiers = [
        "exact_name",
        "name_prefix",
        "exact_extension",
        "extension_prefix",
        "fts",
    ];
    for tier in tiers {
        let (sql, values) = no_result_tier_sql(tier, query);
        let (samples, results, policy) = measure(|| {
            diagnostic_search_tier(conn, tier, query, SEARCH_RESULT_LIMIT)
                .unwrap_or_else(|error| panic!("run production {tier} tier: {error}"))
        });
        assert!(results.is_empty(), "no-result fixture matched {tier}");
        emit_stage_record(
            context,
            "no_result",
            query,
            tier,
            &sql,
            &values,
            &samples,
            0,
            Vec::new(),
            policy,
            "production tier function; SQL and EXPLAIN shown as an exact query-shape mirror",
            conn,
        );
    }
    let (samples, results, policy) = measure(|| {
        search_global_entries_on_connection(conn, query, SEARCH_RESULT_LIMIT, 0)
            .expect("run full production no-result search")
    });
    assert!(results.is_empty());
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "no_result_total",
        "context": context,
        "query_class": "no_result",
        "query": query,
        "result_count": results.len(),
        "first_ids": [],
        "latency_ms": summarize_samples(&samples),
        "sample_policy": policy,
        "component_order": tiers,
        "timing_scope": "production search_global_entries_on_connection; connection checkout excluded"
    }));
}

fn assert_adversarial_fixture(conn: &Connection, base_report_matches: u64) {
    assert!(
        base_report_matches > 4_096,
        "the established generator must exceed the production candidate cap"
    );
    let invalid_rows: i64 = conn
        .query_row(
            r#"
            SELECT COUNT(*)
            FROM global_entries ge
            JOIN global_volumes gv ON gv.id = ge.volume_id
            WHERE (gv.enabled = 0 OR ge.is_stale = 1)
              AND (ge.id LIKE 'ge_disabled_%' OR ge.id LIKE 'ge_stale_%')
            "#,
            [],
            |row| row.get(0),
        )
        .expect("count disabled and stale adversarial rows");
    assert_eq!(invalid_rows, 260);
}

fn assert_adversarial_results(results: &[GlobalSearchResult]) {
    assert_eq!(results.len(), 80);
    assert_eq!(results[0].id, "ge_tie_a");
    assert_eq!(results[1].id, "ge_tie_b");
    assert_eq!(
        results[0].rank, results[1].rank,
        "duplicate FTS documents should tie on BM25"
    );
    assert_eq!(results[0].modified_at_fs, results[1].modified_at_fs);
    assert!(!results[0].managed && results[1].managed);
    assert!(results.iter().all(|entry| {
        entry.volume_id != "gv_disabled"
            && !entry.id.starts_with("ge_disabled_")
            && !entry.id.starts_with("ge_stale_")
    }));
    let unique_ids = results
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(unique_ids.len(), results.len());
}

fn assert_variant_page_equivalence(
    conn: &Connection,
    query: &str,
    limit: u32,
    offset: u32,
    production: &[GlobalSearchResult],
) -> Vec<GlobalSearchResult> {
    let target = offset.saturating_add(limit).min(4_096);
    let safe_rows = run_variant(conn, VARIANT_SAFE_CTE, query, target);
    let cross_rows = run_variant(conn, VARIANT_CROSS_JOIN, query, target);
    let safe_page = safe_rows
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect::<Vec<_>>();
    let cross_page = cross_rows
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect::<Vec<_>>();
    assert_eq!(
        safe_page, production,
        "safe materialized CTE mismatch for {query:?}, limit={limit}, offset={offset}"
    );
    assert_eq!(
        cross_page, production,
        "CROSS JOIN variant mismatch for {query:?}, limit={limit}, offset={offset}"
    );
    production.to_vec()
}

fn profile_variants(
    conn: &Connection,
    context: &JsonValue,
    production_results: &[Vec<GlobalSearchResult>],
) {
    let mut measured_variants = Vec::new();
    for ((query_class, query), production) in FTS_QUERIES.iter().copied().zip(production_results) {
        let production_ids = search_ids(production);
        let mut per_query = Vec::new();
        for (variant, sql) in [
            ("materialized_safe_bounded_candidates", VARIANT_SAFE_CTE),
            ("forced_fts_first_cross_join", VARIANT_CROSS_JOIN),
        ] {
            let (samples, results, policy) =
                measure(|| run_variant(conn, sql, query, SEARCH_RESULT_LIMIT));
            assert_eq!(
                results.as_slice(),
                production.as_slice(),
                "{variant} must preserve every result field for {query}"
            );
            let values = vec![
                SqlValue::Text(fts_phrase(query)),
                SqlValue::Integer(SEARCH_RESULT_LIMIT as i64),
            ];
            emit_record(&json!({
                "schema_version": 1,
                "record_type": "fts_variant",
                "context": context,
                "query_class": query_class,
                "query": query,
                "variant": variant,
                "sql": sql,
                "explain_query_plan": explain_details(conn, sql, &values),
                "result_count": results.len(),
                "first_ids": search_ids(&results).into_iter().take(10).collect::<Vec<_>>(),
                "production_first_ids": production_ids.iter().take(10).collect::<Vec<_>>(),
                "all_fields_and_order_equal": true,
                "latency_ms": summarize_samples(&samples),
                "sample_policy": policy
            }));
            per_query.push(results);
        }
        measured_variants.push(per_query);
    }

    for (query, limit, offset) in [
        ("report", 40, 40),
        ("report", 80, 0),
        ("report", 80, 4_016),
        ("invoice", 80, 0),
    ] {
        let query_index = usize::from(query == "invoice");
        let expected = if offset == 0 {
            production_results[query_index].clone()
        } else {
            search_global_entries_on_connection(conn, query, limit, offset)
                .expect("run production pagination page for variant equality")
        };
        let results = if offset == 0 {
            for variant in &measured_variants[query_index] {
                assert_eq!(
                    variant, &expected,
                    "first page variant changed all fields for {query}"
                );
            }
            expected.clone()
        } else {
            assert_variant_page_equivalence(conn, query, limit, offset, &expected)
        };
        if query == "report" && offset == 40 {
            let first_page_ids = production_results[0]
                .iter()
                .take(40)
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>();
            let second_page_ids = expected
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>();
            assert!(first_page_ids
                .iter()
                .all(|id| !second_page_ids.contains(id)));
            let mut concatenated_pages = first_page_ids;
            concatenated_pages.extend(second_page_ids);
            assert_eq!(
                concatenated_pages,
                production_results[0]
                    .iter()
                    .map(|entry| entry.id.as_str())
                    .collect::<Vec<_>>()
            );
        }
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "variant_semantic_gate",
            "context": context,
            "query": query,
            "limit": limit,
            "offset": offset,
            "result_count": results.len(),
            "all_fields_and_order_equal": true,
            "pagination_and_candidate_cap_equal": true
        }));
    }
    let after_cap = search_global_entries_on_connection(conn, "report", 80, 4_096)
        .expect("verify production search after candidate cap");
    assert!(after_cap.is_empty());
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "candidate_window_contract",
        "context": context,
        "query": "report",
        "candidate_limit": 4096,
        "concatenated_first_80_match": true,
        "duplicate_ids_across_pages": false,
        "offset_at_candidate_cap_empty": true
    }));

    let production = &production_results[0];
    let unsafe_rows = run_variant(conn, VARIANT_UNSAFE_POSTFILTER, "report", 80);
    assert_eq!(production.len(), 80);
    assert!(
        unsafe_rows.len() < production.len(),
        "pre-filter LIMIT should underfill after disabled/stale rows are removed"
    );
    let unsafe_values = vec![SqlValue::Text(fts_phrase("report")), SqlValue::Integer(80)];
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "unsafe_candidate_window_counterexample",
        "context": context,
        "query": "report",
        "candidate_limit": 80,
        "production_active_result_count": production.len(),
        "postfilter_result_count": unsafe_rows.len(),
        "explain_query_plan": explain_details(conn, VARIANT_UNSAFE_POSTFILTER, &unsafe_values),
        "sql": VARIANT_UNSAFE_POSTFILTER,
        "disabled_and_stale_rows_precede_active_rows": true,
        "semantics_equal": false,
        "reason": "candidate LIMIT is applied before enabled-volume and stale filters, so valid active results are omitted"
    }));
}

#[test]
#[ignore = "focused 100k-row Global Search FTS staging and no-result diagnostic"]
fn global_search_fts_latency_diagnostic() {
    assert_eq!(
        env::var("ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DIAGNOSTIC_ENTRIES),
        DIAGNOSTIC_ENTRIES,
        "the issue #346 focused diagnostic is fixed to the established 100k fixture"
    );
    if let Ok(output) = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT") {
        let output = PathBuf::from(output);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).expect("create diagnostic artifact directory");
        }
        fs::write(output, "").expect("truncate focused diagnostic artifact");
    }

    let context = benchmark_context(DIAGNOSTIC_ENTRIES);
    let path = super::super::test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open focused diagnostic database");
    db.upsert_global_volume(&super::super::test_volume())
        .expect("create established synthetic benchmark volume");
    let (expectations, generation_ms, population_ms, _) =
        populate_synthetic_database(&db, DIAGNOSTIC_ENTRIES);
    let base_report_matches = expectations[3].total_matches;
    assert!(base_report_matches > 4_096);
    add_adversarial_rows(&db);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "diagnostic_dataset",
        "context": context,
        "base_fixture": "reused global_search_benchmark::populate_synthetic_database; same 100k generated rows, schema and production triggers as #342",
        "base_rows": DIAGNOSTIC_ENTRIES,
        "base_report_match_oracle_count": base_report_matches,
        "adversarial_rows": 262,
        "generation_ms": generation_ms,
        "population_ms": population_ms,
        "added_semantics": ["second enabled volume", "disabled volume", "stale rows", "managed and unmanaged rows", "duplicate name and path across volumes", "equal rank and mtime tie-break", "more than 4096 FTS matches"],
        "production_triggers_preserved": true
    }));

    let conn = configured_connection(&path).expect("open staged diagnostic reader");
    assert_adversarial_fixture(&conn, base_report_matches);
    let mut production_fts_results = Vec::new();
    for (query_class, query) in FTS_QUERIES {
        production_fts_results.push(profile_fts_query(&conn, &context, query_class, query));
    }
    profile_no_result(&conn, &context);
    profile_variants(&conn, &context, &production_fts_results);
    drop(conn);
    drop(db);
}
