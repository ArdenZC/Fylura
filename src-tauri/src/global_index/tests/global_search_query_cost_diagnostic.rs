//! Bounded Global Search query diagnostics, preserving the legacy Name Prefix
//! qualification beside the focused Issue #364 high-hit FTS qualification.
//! Legacy tier SQL remains available only through test helpers for paired comparisons.

use super::{
    actual_match_count, assert_entry_triggers_enabled, assert_search_results, benchmark_context,
    benchmark_escape_glob, candidate_plan_sql, configured_connection, database_metrics,
    emit_record, populate_synthetic_database, round_ms, sidecar_bytes, summarize_samples,
    test_db_path, test_volume, BenchmarkCleanup, QueryCase, INSERT_SYNTHETIC_ENTRY, QUERY_MATRIX,
    SEARCH_CANDIDATE_LIMIT, SEARCH_RESULT_LIMIT,
};
use crate::db::Database;
use crate::global_index::models::GlobalSearchResult;
use crate::global_index::repository::{
    load_global_search_source_health_candidate, DiagnosticSourceHealth, GlobalSearchSnapshot,
    GlobalSearchSourceHealthQueryCandidate as SourceHealthCandidate,
};
use crate::global_index::search::{
    diagnostic_key_only_cte_sql, diagnostic_legacy_fts_sql, diagnostic_legacy_name_prefix_sql,
    diagnostic_search_fts, diagnostic_search_fts_sql, diagnostic_search_tier,
    diagnostic_search_tier_sql, search_global_entries_on_connection,
    with_key_only_cte_search_candidate, with_key_only_fts_search_candidate,
    with_legacy_fts_search_candidate, with_legacy_name_prefix_search_candidate,
};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Connection, StatementStatus};
use serde_json::{json, Value as JsonValue};
use std::collections::HashSet;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(target_os = "windows")]
use std::thread;
use std::thread::JoinHandle;
#[cfg(target_os = "windows")]
use std::time::Duration;
use std::time::{Duration as StdDuration, Instant};

const DIAGNOSTIC_ENTRIES: u64 = 500_000;
const STAGE_WARMUPS: usize = 3;
const STAGE_SAMPLES: usize = 10;
const PAIRED_WARMUPS: usize = 5;
const PAIRED_SAMPLES: usize = 30;
const QUERY_LIMIT: u32 = 80;
const EXACT_SQL_1M_ENTRIES: u64 = 1_000_000;
const EXACT_SQL_1M_SOFT_TIMEOUT_SECONDS: u64 = 30 * 60;
const EXACT_SQL_1M_EVIDENCE_LIMIT_BYTES: u64 = 32 * 1024 * 1024;
const EXACT_SQL_1M_WARMUPS: usize = 5;
const EXACT_SQL_1M_SAMPLES: usize = 30;
const TEST_INDEX_NAME: &str = "idx_issue352_diag_name_mtime";
const SOURCE_HEALTH_TOPOLOGY_NAMES: [&str; 4] = [
    "one_volume_zero_stale",
    "one_volume_ninety_percent_stale",
    "ten_volumes_zero_stale",
    "ten_volumes_ninety_percent_stale",
];
const SOURCE_HEALTH_CANDIDATES: [SourceHealthCandidate; 3] = [
    SourceHealthCandidate::Original,
    SourceHealthCandidate::NarrowAggregate,
    SourceHealthCandidate::GroupByVolumeId,
];

#[derive(Debug, Clone, Copy)]
struct SourceHealthTopology {
    name: &'static str,
    volume_count: usize,
    stale_percent: usize,
}

const SOURCE_HEALTH_TOPOLOGIES: [SourceHealthTopology; 4] = [
    SourceHealthTopology {
        name: SOURCE_HEALTH_TOPOLOGY_NAMES[0],
        volume_count: 1,
        stale_percent: 0,
    },
    SourceHealthTopology {
        name: SOURCE_HEALTH_TOPOLOGY_NAMES[1],
        volume_count: 1,
        stale_percent: 90,
    },
    SourceHealthTopology {
        name: SOURCE_HEALTH_TOPOLOGY_NAMES[2],
        volume_count: 10,
        stale_percent: 0,
    },
    SourceHealthTopology {
        name: SOURCE_HEALTH_TOPOLOGY_NAMES[3],
        volume_count: 10,
        stale_percent: 90,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceHealthSqlRow {
    source_id: String,
    enabled: bool,
    provider: String,
    status: String,
    last_error: Option<String>,
    updated_at: i64,
    active_entry_count: i64,
    max_last_seen_at: Option<i64>,
}

const SAFE_PREFIX_CTE_SQL: &str = r#"
    WITH candidates AS MATERIALIZED (
        SELECT ge.rowid AS entry_rowid,
               ge.id AS entry_id,
               ge.modified_at_fs AS modified_at_fs
        FROM global_entries ge INDEXED BY idx_global_entries_active_name_order
        CROSS JOIN global_volumes gv
        WHERE gv.id = ge.volume_id
          AND gv.enabled = 1
          AND ge.is_stale = 0
          AND ge.name_normalized GLOB ?2
          AND ge.name_normalized <> lower(?1)
        ORDER BY ge.modified_at_fs DESC, ge.id ASC
        LIMIT ?3
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
           0.0 AS rank
    FROM candidates
    JOIN global_entries ge ON ge.rowid = candidates.entry_rowid
    ORDER BY candidates.modified_at_fs DESC, candidates.entry_id ASC
"#;

const TIME_INDEX_PREFIX_SQL: &str = r#"
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
           0.0 AS rank
    FROM global_entries ge INDEXED BY idx_issue352_diag_name_mtime
    CROSS JOIN global_volumes gv
    WHERE gv.id = ge.volume_id
      AND gv.enabled = 1
      AND ge.is_stale = 0
      AND ge.name_normalized GLOB ?2
      AND ge.name_normalized <> lower(?1)
    ORDER BY ge.modified_at_fs DESC, ge.id ASC
    LIMIT ?3
"#;

const SAFE_FTS_CTE_SQL: &str = r#"
    WITH candidates AS MATERIALIZED (
        SELECT ge.rowid AS entry_rowid,
               ge.id AS entry_id,
               ge.modified_at_fs AS modified_at_fs,
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

const FTS_WITHOUT_MANAGED_SQL: &str = r#"
    SELECT ge.rowid, ge.id, ge.volume_id, ge.platform_file_id, ge.name, ge.path,
           ge.extension, ge.is_directory, ge.size, ge.created_at_fs,
           ge.modified_at_fs, ge.file_attributes, ge.is_hidden, ge.is_system,
           ge.source_provider,
           0 AS managed,
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

// Exact read-side SQL shapes used by repository.rs when it builds the
// production search snapshot. Kept here as test-only mirrors so their cost is
// visible separately from the search tier itself.
const SNAPSHOT_SOURCE_HEALTH_SQL: &str = r#"
    SELECT gv.id, gv.enabled, gv.provider, gv.index_status, gv.last_error, gv.updated_at,
           COUNT(ge.id), MAX(ge.last_seen_at)
    FROM global_volumes gv
    LEFT JOIN global_entries ge
      ON ge.volume_id = gv.id AND ge.is_stale = 0
    GROUP BY gv.id, gv.enabled, gv.provider, gv.index_status, gv.last_error, gv.updated_at
    ORDER BY gv.id ASC
"#;

const SNAPSHOT_INDEX_STATUS_SQL: &str = r#"
    SELECT
        (SELECT COUNT(*)
         FROM global_entries entry
         JOIN global_volumes volume ON volume.id = entry.volume_id
         WHERE entry.is_stale = 0 AND volume.enabled = 1),
        (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1),
        (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'ready'),
        (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status IN ('discovered', 'indexing', 'syncing', 'rebuild_required')),
        (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'spotlight_not_indexed'),
        (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'permission_required'),
        (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'spotlight_unavailable'),
        (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'spotlight_external_not_indexed'),
        (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'fsevents_unavailable'),
        (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'unavailable'),
        (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'error'),
        (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'paused'),
        (SELECT MAX(last_incremental_sync_at) FROM global_volumes WHERE enabled = 1),
        (SELECT last_error FROM global_volumes WHERE enabled = 1 AND last_error IS NOT NULL ORDER BY updated_at DESC LIMIT 1)
"#;

#[derive(Clone, Copy)]
struct PrefixQuery {
    class: &'static str,
    query: &'static str,
    extension: bool,
}

const PREFIX_QUERIES: [PrefixQuery; 6] = [
    PrefixQuery {
        class: "name_prefix",
        query: "quarterly",
        extension: false,
    },
    PrefixQuery {
        class: "common_prefix_high_fanout",
        query: "IMG_",
        extension: false,
    },
    PrefixQuery {
        class: "chinese_prefix",
        query: "数据库",
        extension: false,
    },
    PrefixQuery {
        class: "punctuation_prefix",
        query: "final-v2",
        extension: false,
    },
    PrefixQuery {
        class: "unicode_accented_prefix",
        query: "RÉSUMÉ",
        extension: false,
    },
    PrefixQuery {
        class: "extension_prefix",
        query: "jp",
        extension: true,
    },
];

const FTS_QUERIES: [(&str, &str); 4] = [
    ("fts_high_fanout_report", "report"),
    ("fts_high_fanout_invoice", "invoice"),
    ("fts_low_fanout", "0499981"),
    ("fts_no_result", "zzznomatchtoken"),
];

fn fts_phrase(query: &str) -> String {
    format!("\"{}\"", query.replace('"', "\"\""))
}

pub(super) fn explain_details(conn: &Connection, sql: &str, values: &[SqlValue]) -> Vec<String> {
    let mut statement = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("prepare diagnostic EXPLAIN QUERY PLAN");
    statement
        .query_map(params_from_iter(values.iter()), |row| {
            row.get::<_, String>(3)
        })
        .expect("execute diagnostic EXPLAIN QUERY PLAN")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect diagnostic query plan")
}

fn sql_values_json(values: &[SqlValue]) -> Vec<JsonValue> {
    values
        .iter()
        .map(|value| match value {
            SqlValue::Null => JsonValue::Null,
            SqlValue::Integer(value) => json!(value),
            SqlValue::Real(value) => json!(value),
            SqlValue::Text(value) => json!(value),
            SqlValue::Blob(value) => json!({"blob_bytes": value.len()}),
        })
        .collect()
}

fn execute_count_once(
    conn: &Connection,
    sql: &str,
    values: &[SqlValue],
) -> (usize, f64, JsonValue) {
    let mut statement = conn.prepare(sql).expect("prepare staged diagnostic SQL");
    for status in [
        StatementStatus::Sort,
        StatementStatus::FullscanStep,
        StatementStatus::VmStep,
    ] {
        statement.reset_status(status);
    }
    let started = Instant::now();
    let count = statement
        .query_map(params_from_iter(values.iter()), |row| {
            row.get::<_, SqlValue>(0)
        })
        .expect("run staged diagnostic SQL")
        .try_fold(0_usize, |count, _row| Ok::<_, rusqlite::Error>(count + 1))
        .expect("drain staged diagnostic SQL");
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let counters = json!({
        "sort_operations": statement.get_status(StatementStatus::Sort),
        "fullscan_steps": statement.get_status(StatementStatus::FullscanStep),
        "vm_steps": statement.get_status(StatementStatus::VmStep),
        "scope": "last execution; VM steps are work indicators, not CPU time or temp bytes"
    });
    (count, elapsed_ms, counters)
}

fn profile_sql_stage(
    conn: &Connection,
    context: &JsonValue,
    class: &str,
    query: &str,
    stage: &str,
    sql: &str,
    values: Vec<SqlValue>,
) -> usize {
    let mut last_count = 0;
    for _ in 0..STAGE_WARMUPS {
        last_count = execute_count_once(conn, sql, &values).0;
    }
    let mut samples = Vec::with_capacity(STAGE_SAMPLES);
    let mut last_counters = JsonValue::Null;
    for _ in 0..STAGE_SAMPLES {
        let (count, elapsed_ms, counters) = execute_count_once(conn, sql, &values);
        last_count = count;
        samples.push(elapsed_ms);
        last_counters = counters;
    }
    let plan = explain_details(conn, sql, &values);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "query_stage",
        "context": context,
        "query_class": class,
        "query": query,
        "stage": stage,
        "result_count": last_count,
        "latency_ms": summarize_samples(&samples),
        "warmups": STAGE_WARMUPS,
        "sample_policy": "10 consecutive warm-cache samples after three warmups; same SQLite connection and fixture",
        "sql": sql,
        "parameters": sql_values_json(&values),
        "explain_query_plan": plan,
        "uses_temp_btree": plan.iter().any(|line| line.contains("USE TEMP B-TREE")),
        "possible_full_table_scan": plan.iter().any(|line| line.contains("SCAN ge") || line.contains("SCAN global_entries ")),
        "sqlite_statement_counters": last_counters,
        "timing_includes_prepare": false
    }));
    last_count
}

fn prefix_stage_sql(spec: PrefixQuery) -> Vec<(&'static str, String, Vec<SqlValue>)> {
    let (column, index, exact_filter, rank) = if spec.extension {
        (
            "extension",
            "idx_global_entries_active_extension_order",
            "ge.extension <> lower(?1)",
            "1.0",
        )
    } else {
        (
            "name_normalized",
            "idx_global_entries_active_name_order",
            "ge.name_normalized <> lower(?1)",
            "0.0",
        )
    };
    let query = spec.query.to_string();
    let pattern = format!("{}*", benchmark_escape_glob(spec.query));
    let mut stages = Vec::new();
    let index_only = format!(
        "SELECT ge.rowid FROM global_entries ge INDEXED BY {index} WHERE ge.is_stale = 0 AND ge.{column} GLOB ?2 AND {exact_filter}"
    );
    stages.push((
        "A_index_range_without_volume_join",
        index_only,
        vec![
            SqlValue::Text(query.clone()),
            SqlValue::Text(pattern.clone()),
        ],
    ));
    let active = format!(
        "SELECT ge.rowid FROM global_entries ge INDEXED BY {index} JOIN global_volumes gv ON gv.id = ge.volume_id WHERE gv.enabled = 1 AND ge.is_stale = 0 AND ge.{column} GLOB ?2 AND {exact_filter}"
    );
    stages.push((
        "B_active_volume_and_stale_filter",
        active,
        vec![
            SqlValue::Text(query.clone()),
            SqlValue::Text(pattern.clone()),
        ],
    ));
    let order_all = format!(
        "SELECT ge.rowid FROM global_entries ge INDEXED BY {index} JOIN global_volumes gv ON gv.id = ge.volume_id WHERE gv.enabled = 1 AND ge.is_stale = 0 AND ge.{column} GLOB ?2 AND {exact_filter} ORDER BY ge.modified_at_fs DESC, ge.id ASC"
    );
    stages.push((
        "C_global_recency_sort_all_matches",
        order_all,
        vec![
            SqlValue::Text(query.clone()),
            SqlValue::Text(pattern.clone()),
        ],
    ));
    for (stage, limit) in [
        ("D_global_recency_sort_limit_80", SEARCH_RESULT_LIMIT),
        (
            "E_global_recency_sort_limit_4096",
            SEARCH_CANDIDATE_LIMIT as u32,
        ),
    ] {
        let sql = format!(
            "SELECT ge.rowid FROM global_entries ge INDEXED BY {index} JOIN global_volumes gv ON gv.id = ge.volume_id WHERE gv.enabled = 1 AND ge.is_stale = 0 AND ge.{column} GLOB ?2 AND {exact_filter} ORDER BY ge.modified_at_fs DESC, ge.id ASC LIMIT ?3"
        );
        stages.push((
            stage,
            sql,
            vec![
                SqlValue::Text(query.clone()),
                SqlValue::Text(pattern.clone()),
                SqlValue::Integer(limit as i64),
            ],
        ));
    }
    let no_managed = format!(
        r#"
        SELECT ge.rowid, ge.id, ge.volume_id, ge.platform_file_id, ge.name,
               ge.path, ge.extension, ge.is_directory, ge.size,
               ge.created_at_fs, ge.modified_at_fs, ge.file_attributes,
               ge.is_hidden, ge.is_system, ge.source_provider,
               0 AS managed, {rank} AS rank
        FROM global_entries ge INDEXED BY {index}
        JOIN global_volumes gv ON gv.id = ge.volume_id
        WHERE gv.enabled = 1 AND ge.is_stale = 0
          AND ge.{column} GLOB ?2 AND {exact_filter}
        ORDER BY ge.modified_at_fs DESC, ge.id ASC
        LIMIT ?3
        "#
    );
    stages.push((
        "F_full_projection_without_managed_exists_limit_80",
        no_managed,
        vec![
            SqlValue::Text(query.clone()),
            SqlValue::Text(pattern.clone()),
            SqlValue::Integer(QUERY_LIMIT as i64),
        ],
    ));
    let production_mirror = candidate_plan_sql(
        &format!(
            "global_entries ge INDEXED BY {index} JOIN global_volumes gv ON gv.id = ge.volume_id"
        ),
        &format!("gv.enabled = 1 AND ge.is_stale = 0 AND ge.{column} GLOB ?2 AND {exact_filter}"),
        "ge.modified_at_fs DESC, ge.id ASC",
        rank,
        "?3",
    );
    stages.push((
        "G_full_production_tier_projection_and_managed_exists_limit_80",
        production_mirror,
        vec![
            SqlValue::Text(query),
            SqlValue::Text(pattern),
            SqlValue::Integer(QUERY_LIMIT as i64),
        ],
    ));
    stages
}

pub(super) fn map_search_result(row: &rusqlite::Row<'_>) -> rusqlite::Result<GlobalSearchResult> {
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

fn run_result_sql(conn: &Connection, sql: &str, values: &[SqlValue]) -> Vec<GlobalSearchResult> {
    conn.prepare(sql)
        .expect("prepare diagnostic result query")
        .query_map(params_from_iter(values.iter()), map_search_result)
        .expect("execute diagnostic result query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect diagnostic result query")
}

fn prefix_variant_values(query: &str, limit: u32) -> Vec<SqlValue> {
    vec![
        SqlValue::Text(query.to_string()),
        SqlValue::Text(format!("{}*", benchmark_escape_glob(query))),
        SqlValue::Integer(limit as i64),
    ]
}

fn fts_variant_values(query: &str, limit: u32) -> Vec<SqlValue> {
    vec![
        SqlValue::Text(fts_phrase(query)),
        SqlValue::Integer(limit as i64),
    ]
}

struct CandidateVariant<'a> {
    name: &'a str,
    sql: &'a str,
    values: Vec<SqlValue>,
}

fn time_results(
    mut run: impl FnMut() -> Vec<GlobalSearchResult>,
) -> (Vec<GlobalSearchResult>, f64) {
    let started = Instant::now();
    let results = run();
    (results, started.elapsed().as_secs_f64() * 1_000.0)
}

fn profile_paired_variant(
    context: &JsonValue,
    conn: &Connection,
    class: &str,
    query: &str,
    variant: CandidateVariant<'_>,
    production: impl Fn() -> Vec<GlobalSearchResult>,
) {
    let variant_name = variant.name;
    let variant_sql = variant.sql;
    let variant_values = &variant.values;
    let expected = production();
    let candidate = run_result_sql(conn, variant_sql, variant_values);
    assert_eq!(
        candidate, expected,
        "unwarmed {variant_name} mismatch for {query:?}"
    );
    for _ in 0..PAIRED_WARMUPS {
        assert_eq!(production(), expected);
        assert_eq!(run_result_sql(conn, variant_sql, variant_values), expected);
    }
    let mut production_samples = Vec::with_capacity(PAIRED_SAMPLES);
    let mut candidate_samples = Vec::with_capacity(PAIRED_SAMPLES);
    for index in 0..PAIRED_SAMPLES {
        if index % 2 == 0 {
            let (actual, elapsed) = time_results(&production);
            assert_eq!(actual, expected);
            production_samples.push(elapsed);
            let (actual, elapsed) =
                time_results(|| run_result_sql(conn, variant_sql, variant_values));
            assert_eq!(actual, expected);
            candidate_samples.push(elapsed);
        } else {
            let (actual, elapsed) =
                time_results(|| run_result_sql(conn, variant_sql, variant_values));
            assert_eq!(actual, expected);
            candidate_samples.push(elapsed);
            let (actual, elapsed) = time_results(&production);
            assert_eq!(actual, expected);
            production_samples.push(elapsed);
        }
    }
    let production_summary = summarize_samples(&production_samples);
    let candidate_summary = summarize_samples(&candidate_samples);
    let production_median = production_summary["p50"].as_f64().unwrap_or_default();
    let candidate_median = candidate_summary["p50"].as_f64().unwrap_or_default();
    let reduction_percent = if production_median > 0.0 {
        (production_median - candidate_median) / production_median * 100.0
    } else {
        0.0
    };
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "paired_sql_variant",
        "context": context,
        "query_class": class,
        "query": query,
        "variant": variant_name,
        "production_latency_ms": production_summary,
        "candidate_latency_ms": candidate_summary,
        "paired_order": "production-first and candidate-first alternate for each sample on the same Windows job, same database and connection",
        "warmups_per_variant": PAIRED_WARMUPS,
        "samples_per_variant": PAIRED_SAMPLES,
        "semantic_equivalent": true,
        "result_count": expected.len(),
        "result_ids_prefix": expected.iter().take(8).map(|row| row.id.clone()).collect::<Vec<_>>(),
        "candidate_sql": variant_sql,
        "candidate_parameters": sql_values_json(variant_values),
        "candidate_explain_query_plan": explain_details(conn, variant_sql, variant_values),
        "candidate_uses_temp_btree": explain_details(conn, variant_sql, variant_values).iter().any(|line| line.contains("USE TEMP B-TREE")),
        "p50_reduction_percent": reduction_percent,
        "p50_interpretation": "descriptive within-run comparison; not the official baseline gate"
    }));
}

fn record_production_results(
    context: &JsonValue,
    class: &str,
    query: &str,
    stage: &str,
    samples: Vec<f64>,
    results: &[GlobalSearchResult],
    timing_scope: &str,
) {
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "production_query_timing",
        "context": context,
        "query_class": class,
        "query": query,
        "stage": stage,
        "result_count": results.len(),
        "result_ids_prefix": results.iter().take(8).map(|row| row.id.clone()).collect::<Vec<_>>(),
        "latency_ms": summarize_samples(&samples),
        "warmups": PAIRED_WARMUPS,
        "sample_policy": format!("{} warm samples after {} warmups", samples.len(), PAIRED_WARMUPS),
        "timing_scope": timing_scope
    }));
}

fn source_health_candidate_name(candidate: SourceHealthCandidate) -> &'static str {
    match candidate {
        SourceHealthCandidate::Original => "original",
        SourceHealthCandidate::CorrelatedAggregates => "correlated_aggregates",
        SourceHealthCandidate::NarrowAggregate => "narrow_aggregate",
        SourceHealthCandidate::GroupByVolumeId => "candidate_c_group_by_volume_id",
    }
}

fn source_health_topology_candidate_label(candidate: SourceHealthCandidate) -> &'static str {
    match candidate {
        SourceHealthCandidate::Original => "Original",
        SourceHealthCandidate::NarrowAggregate => "Candidate B (current production SQL)",
        SourceHealthCandidate::GroupByVolumeId => "Candidate C (test-only)",
        SourceHealthCandidate::CorrelatedAggregates => unreachable!(
            "the topology diagnostic compares Original, production Candidate B, and Candidate C"
        ),
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct SourceHealthResourceStats {
    calls: u64,
    cpu_time_ms: f64,
    wall_time_ms: f64,
    peak_query_cpu_percent: f64,
    peak_working_set_bytes: u64,
    peak_private_bytes: u64,
    memory_samples: u64,
}

#[derive(Debug, Clone, Copy)]
struct NativeProcessSample {
    cpu_time_100ns: u64,
    working_set_bytes: u64,
    private_bytes: u64,
}

#[cfg(target_os = "windows")]
fn native_process_sample() -> Option<NativeProcessSample> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    let process = unsafe { GetCurrentProcess() };
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let memory_ok = unsafe {
        GetProcessMemoryInfo(
            process,
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let cpu_ok =
        unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) };
    if memory_ok == 0 || cpu_ok == 0 {
        return None;
    }
    let filetime =
        |value: FILETIME| (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime);
    Some(NativeProcessSample {
        cpu_time_100ns: filetime(kernel).saturating_add(filetime(user)),
        working_set_bytes: counters.WorkingSetSize as u64,
        private_bytes: counters.PrivateUsage as u64,
    })
}

#[cfg(not(target_os = "windows"))]
fn native_process_sample() -> Option<NativeProcessSample> {
    None
}

struct SourceHealthResourceSampler {
    active_tag: Arc<AtomicU8>,
    stats: Arc<Mutex<[SourceHealthResourceStats; 7]>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    baseline: Option<NativeProcessSample>,
}

impl SourceHealthResourceSampler {
    fn new() -> Self {
        let active_tag = Arc::new(AtomicU8::new(0));
        let stats = Arc::new(Mutex::new([SourceHealthResourceStats::default(); 7]));
        let stop = Arc::new(AtomicBool::new(false));
        let baseline = native_process_sample();

        #[cfg(target_os = "windows")]
        let worker = {
            let active_tag = Arc::clone(&active_tag);
            let stats = Arc::clone(&stats);
            let stop = Arc::clone(&stop);
            Some(thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    if let (Some(sample), tag) = (
                        native_process_sample(),
                        active_tag.load(Ordering::Relaxed) as usize,
                    ) {
                        if tag > 0 {
                            if let Ok(mut stats) = stats.lock() {
                                if let Some(candidate) = stats.get_mut(tag) {
                                    candidate.peak_working_set_bytes = candidate
                                        .peak_working_set_bytes
                                        .max(sample.working_set_bytes);
                                    candidate.peak_private_bytes =
                                        candidate.peak_private_bytes.max(sample.private_bytes);
                                    candidate.memory_samples += 1;
                                }
                            }
                        }
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            }))
        };

        #[cfg(not(target_os = "windows"))]
        let worker = None;

        Self {
            active_tag,
            stats,
            stop,
            worker,
            baseline,
        }
    }

    fn tag(candidate: SourceHealthCandidate, snapshot: bool) -> u8 {
        let candidate_offset = match candidate {
            SourceHealthCandidate::Original => 0,
            SourceHealthCandidate::NarrowAggregate => 1,
            SourceHealthCandidate::GroupByVolumeId => 2,
            SourceHealthCandidate::CorrelatedAggregates => {
                unreachable!("the topology diagnostic does not profile candidate A")
            }
        };
        1 + candidate_offset * 2 + u8::from(snapshot)
    }

    fn measure<T>(
        &self,
        candidate: SourceHealthCandidate,
        snapshot: bool,
        operation: impl FnOnce() -> T,
    ) -> (T, f64) {
        let tag = Self::tag(candidate, snapshot) as usize;
        self.active_tag.store(tag as u8, Ordering::Relaxed);
        let before = native_process_sample();
        let started = Instant::now();
        let result = operation();
        let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
        let after = native_process_sample();
        self.active_tag.store(0, Ordering::Relaxed);

        if let Ok(mut stats) = self.stats.lock() {
            if let Some(candidate_stats) = stats.get_mut(tag) {
                candidate_stats.calls += 1;
                candidate_stats.wall_time_ms += elapsed_ms;
                if let (Some(before), Some(after)) = (before, after) {
                    let cpu_time_ms = after.cpu_time_100ns.saturating_sub(before.cpu_time_100ns)
                        as f64
                        / 10_000.0;
                    candidate_stats.cpu_time_ms += cpu_time_ms;
                    if elapsed_ms > 0.0 {
                        candidate_stats.peak_query_cpu_percent = candidate_stats
                            .peak_query_cpu_percent
                            .max(cpu_time_ms / elapsed_ms * 100.0);
                    }
                    candidate_stats.peak_working_set_bytes = candidate_stats
                        .peak_working_set_bytes
                        .max(before.working_set_bytes)
                        .max(after.working_set_bytes);
                    candidate_stats.peak_private_bytes = candidate_stats
                        .peak_private_bytes
                        .max(before.private_bytes)
                        .max(after.private_bytes);
                }
            }
        }
        (result, elapsed_ms)
    }

    fn metrics(&self, candidate: SourceHealthCandidate, snapshot: bool) -> JsonValue {
        let tag = Self::tag(candidate, snapshot) as usize;
        let stats = self
            .stats
            .lock()
            .map(|stats| stats[tag])
            .unwrap_or_default();
        let baseline_working_set = self.baseline.map(|sample| sample.working_set_bytes);
        let baseline_private = self.baseline.map(|sample| sample.private_bytes);
        let supported = self.baseline.is_some();
        json!({
            "status": if supported { "SAMPLED_WINDOWS_PROCESS_METRICS" } else { "NOT_VERIFIED_OUTSIDE_WINDOWS_HOSTED" },
            "measurement_window": "warmups and paired diagnostic calls tagged for this candidate/workload; 50ms background working-set/private-bytes sampling plus process CPU time deltas around each call",
            "calls_including_warmups": stats.calls,
            "process_cpu_time_ms": supported.then_some(round_ms(stats.cpu_time_ms)),
            "average_process_cpu_percent_of_one_core": (supported && stats.wall_time_ms > 0.0).then_some(round_ms(stats.cpu_time_ms / stats.wall_time_ms * 100.0)),
            "peak_observed_query_cpu_percent_of_one_core": supported.then_some(round_ms(stats.peak_query_cpu_percent)),
            "peak_sampled_working_set_bytes": supported.then_some(stats.peak_working_set_bytes),
            "peak_sampled_working_set_delta_from_topology_start_bytes": baseline_working_set.map(|baseline| stats.peak_working_set_bytes.saturating_sub(baseline)),
            "peak_sampled_private_commit_bytes": supported.then_some(stats.peak_private_bytes),
            "peak_sampled_private_commit_delta_from_topology_start_bytes": baseline_private.map(|baseline| stats.peak_private_bytes.saturating_sub(baseline)),
            "background_memory_sample_count": stats.memory_samples,
            "sqlite_temp_store": "MEMORY",
            "sqlite_temp_memory_bytes": "NOT_VERIFIED: temporary SQLite allocations are not isolated from SQLite connection/cache memory by this probe"
        })
    }
}

impl Drop for SourceHealthResourceSampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn measure_source_health_sql_once(
    conn: &Connection,
    candidate: SourceHealthCandidate,
) -> (usize, f64, JsonValue) {
    execute_count_once(conn, candidate.sql(), &[])
}

fn profile_source_health_sql_candidates(context: &JsonValue, conn: &Connection, path: &Path) {
    let original = SourceHealthCandidate::Original;
    for candidate in [
        SourceHealthCandidate::CorrelatedAggregates,
        SourceHealthCandidate::NarrowAggregate,
    ] {
        let mut original_rows = 0;
        let mut candidate_rows = 0;
        for _ in 0..PAIRED_WARMUPS {
            let (base_count, _, _) = measure_source_health_sql_once(conn, original);
            let (candidate_count, _, _) = measure_source_health_sql_once(conn, candidate);
            original_rows = base_count;
            candidate_rows = candidate_count;
            assert_eq!(base_count, candidate_count);
        }

        let mut original_samples = Vec::with_capacity(PAIRED_SAMPLES);
        let mut candidate_samples = Vec::with_capacity(PAIRED_SAMPLES);
        let mut original_counters = JsonValue::Null;
        let mut candidate_counters = JsonValue::Null;
        for sample in 0..PAIRED_SAMPLES {
            if sample % 2 == 0 {
                let (count, elapsed, counters) = measure_source_health_sql_once(conn, original);
                original_rows = count;
                original_samples.push(elapsed);
                original_counters = counters;
                let (count, elapsed, counters) = measure_source_health_sql_once(conn, candidate);
                candidate_rows = count;
                candidate_samples.push(elapsed);
                candidate_counters = counters;
            } else {
                let (count, elapsed, counters) = measure_source_health_sql_once(conn, candidate);
                candidate_rows = count;
                candidate_samples.push(elapsed);
                candidate_counters = counters;
                let (count, elapsed, counters) = measure_source_health_sql_once(conn, original);
                original_rows = count;
                original_samples.push(elapsed);
                original_counters = counters;
            }
            assert_eq!(original_rows, candidate_rows);
        }
        let original_plan = explain_details(conn, original.sql(), &[]);
        let candidate_plan = explain_details(conn, candidate.sql(), &[]);
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "source_health_sql_paired",
            "context": context,
            "candidate": source_health_candidate_name(candidate),
            "pairing": "alternating execution order on same Windows runner, same SQLite connection and same 500k fixture",
            "warmups_per_variant": PAIRED_WARMUPS,
            "samples_per_variant": PAIRED_SAMPLES,
            "original": {
                "sql": original.sql(),
                "parameters": [],
                "row_count": original_rows,
                "latency_ms": summarize_samples(&original_samples),
                "explain_query_plan": original_plan,
                "sqlite_statement_counters": original_counters
            },
            "candidate_result": {
                "sql": candidate.sql(),
                "parameters": [],
                "row_count": candidate_rows,
                "latency_ms": summarize_samples(&candidate_samples),
                "explain_query_plan": candidate_plan,
                "sqlite_statement_counters": candidate_counters
            },
            "database": database_metrics(conn, path),
            "schema_or_index_changes": false
        }));
    }
}

fn measure_source_health_function_once(
    conn: &mut Connection,
    candidate: SourceHealthCandidate,
) -> (DiagnosticSourceHealth, f64) {
    let transaction = conn
        .transaction()
        .expect("begin source-health function sample");
    let started = Instant::now();
    let result = load_global_search_source_health_candidate(&transaction, candidate)
        .expect("run complete source-health function candidate");
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    transaction
        .commit()
        .expect("commit source-health function sample");
    (result, elapsed_ms)
}

fn profile_source_health_functions(db: &Database, path: &Path, context: &JsonValue) {
    let mut conn = db.conn().expect("borrow source-health function connection");
    let original = SourceHealthCandidate::Original;
    for candidate in [
        SourceHealthCandidate::CorrelatedAggregates,
        SourceHealthCandidate::NarrowAggregate,
    ] {
        for _ in 0..PAIRED_WARMUPS {
            let (baseline, _) = measure_source_health_function_once(&mut conn, original);
            let (optimized, _) = measure_source_health_function_once(&mut conn, candidate);
            assert_eq!(baseline, optimized);
        }

        let mut original_samples = Vec::with_capacity(PAIRED_SAMPLES);
        let mut candidate_samples = Vec::with_capacity(PAIRED_SAMPLES);
        let mut facts_match = true;
        for sample in 0..PAIRED_SAMPLES {
            let (baseline, baseline_ms, optimized, candidate_ms) = if sample % 2 == 0 {
                let (baseline, baseline_ms) =
                    measure_source_health_function_once(&mut conn, original);
                let (optimized, candidate_ms) =
                    measure_source_health_function_once(&mut conn, candidate);
                (baseline, baseline_ms, optimized, candidate_ms)
            } else {
                let (optimized, candidate_ms) =
                    measure_source_health_function_once(&mut conn, candidate);
                let (baseline, baseline_ms) =
                    measure_source_health_function_once(&mut conn, original);
                (baseline, baseline_ms, optimized, candidate_ms)
            };
            facts_match &= baseline == optimized;
            assert_eq!(baseline, optimized);
            original_samples.push(baseline_ms);
            candidate_samples.push(candidate_ms);
        }
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "source_health_function_paired",
            "context": context,
            "candidate": source_health_candidate_name(candidate),
            "pairing": "alternating execution order, same pooled SQLite connection, each call in a fresh read transaction",
            "warmups_per_variant": PAIRED_WARMUPS,
            "samples_per_variant": PAIRED_SAMPLES,
            "original": summarize_samples(&original_samples),
            "candidate_result": summarize_samples(&candidate_samples),
            "complete_source_health_equal": facts_match,
            "revision_facts_bytes_and_blake3_equal": facts_match,
            "database": database_metrics(&conn, path),
            "timing_scope": "query execution, row mapping, revision-facts JSON serialization and BLAKE3; excludes pool checkout and transaction setup"
        }));
    }
}

fn run_snapshot_variant(
    db: &Database,
    query: &str,
    candidate: SourceHealthCandidate,
) -> GlobalSearchSnapshot {
    if candidate == SourceHealthCandidate::NarrowAggregate {
        db.search_global_entries_snapshot(query, QUERY_LIMIT, 0)
            .expect("run the unoverridden production repository snapshot")
    } else {
        db.search_global_entries_snapshot_with_source_health_candidate(
            query,
            QUERY_LIMIT,
            0,
            candidate,
        )
        .expect("run test-only source-health SQL repository snapshot variant")
    }
}

fn assert_snapshot_results_equal(
    original: &GlobalSearchSnapshot,
    candidate: &GlobalSearchSnapshot,
) {
    assert_eq!(original.results, candidate.results);
    assert_eq!(original.source_health, candidate.source_health);
    assert_eq!(
        original.source_revision.as_bytes(),
        candidate.source_revision.as_bytes()
    );
    assert_eq!(original.index_status, candidate.index_status);
}

fn topology_candidate_order(round: usize) -> [SourceHealthCandidate; 3] {
    let mut candidates = SOURCE_HEALTH_CANDIDATES;
    let candidate_count = candidates.len();
    candidates.rotate_left(round % candidate_count);
    candidates
}

fn topology_volume_id(volume_number: usize) -> String {
    if volume_number == 1 {
        "gv_test".to_string()
    } else {
        format!("gv_topology_{volume_number:02}")
    }
}

fn insert_topology_volume(transaction: &rusqlite::Transaction<'_>, volume_number: usize) {
    let mut volume = test_volume();
    volume.id = topology_volume_id(volume_number);
    volume.stable_volume_id = format!("issue359-topology-volume-{volume_number:02}");
    volume.display_name = format!("Issue 359 topology volume {volume_number:02}");
    volume.mount_path = format!("D:\\Issue359Topology\\volume-{volume_number:02}\\");
    transaction
        .execute(
            r#"
            INSERT INTO global_volumes (
                id, platform, stable_volume_id, display_name, mount_path,
                filesystem_type, drive_kind, enabled, provider, index_status,
                last_error, journal_id, journal_cursor, last_full_index_at,
                last_incremental_sync_at, entry_count, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
            "#,
            params![
                volume.id,
                volume.platform,
                volume.stable_volume_id,
                volume.display_name,
                volume.mount_path,
                volume.filesystem_type,
                volume.drive_kind,
                i64::from(volume.enabled),
                volume.provider,
                volume.index_status,
                volume.last_error,
                volume.journal_id,
                volume.journal_cursor,
                volume.last_full_index_at,
                volume.last_incremental_sync_at,
                volume.entry_count,
                volume.created_at,
                volume.updated_at,
            ],
        )
        .expect("insert disposable topology volume");
}

fn apply_source_health_topology(conn: &mut Connection, topology: SourceHealthTopology) -> f64 {
    let started = Instant::now();
    let transaction = conn
        .transaction()
        .expect("begin rollbackable source-health topology transition");
    if topology.volume_count == 10 {
        for volume_number in 2..=10 {
            insert_topology_volume(&transaction, volume_number);
        }
        let volume_cases = (0..10)
            .map(|bucket| {
                let volume_number = bucket + 1;
                format!("WHEN {bucket} THEN '{}'", topology_volume_id(volume_number))
            })
            .collect::<Vec<_>>()
            .join(" ");
        let stale_expression = if topology.stale_percent == 90 {
            "CASE WHEN ((rowid - 1) % 100) < 10 THEN 0 ELSE 1 END"
        } else {
            "0"
        };
        let update_sql = format!(
            "UPDATE global_entries SET volume_id = CASE ((rowid - 1) / 50000) {volume_cases} ELSE 'gv_test' END, is_stale = {stale_expression}"
        );
        transaction
            .execute(&update_sql, [])
            .expect("partition the same 500k rows across ten volumes");
    } else if topology.stale_percent == 90 {
        transaction
            .execute(
                "UPDATE global_entries SET is_stale = CASE WHEN ((rowid - 1) % 100) < 10 THEN 0 ELSE 1 END",
                [],
            )
            .expect("mark exactly ninety percent of the one-volume fixture stale");
    }
    transaction
        .commit()
        .expect("commit the disposable topology state for pooled snapshot reads");
    started.elapsed().as_secs_f64() * 1_000.0
}

fn restore_source_health_topology_base(
    conn: &mut Connection,
    base_volume_entry_count: i64,
    base_volume_updated_at: i64,
) -> f64 {
    let started = Instant::now();
    let transaction = conn
        .transaction()
        .expect("begin source-health topology rollback");
    transaction
        .execute(
            "UPDATE global_entries SET volume_id = 'gv_test', is_stale = 0 WHERE volume_id <> 'gv_test' OR is_stale <> 0",
            [],
        )
        .expect("restore all synthetic entries to the single-volume active base");
    transaction
        .execute(
            "UPDATE global_volumes SET entry_count = ?1, updated_at = ?2 WHERE id = 'gv_test'",
            params![base_volume_entry_count, base_volume_updated_at],
        )
        .expect("restore base volume cache and timestamp changed by fixture triggers");
    transaction
        .execute(
            "DELETE FROM global_volumes WHERE substr(id, 1, 12) = 'gv_topology_'",
            [],
        )
        .expect("remove the disposable topology volumes");
    transaction
        .commit()
        .expect("commit topology rollback to the fixture base");
    started.elapsed().as_secs_f64() * 1_000.0
}

fn source_health_topology_facts(conn: &Connection, topology: SourceHealthTopology) -> JsonValue {
    let entry_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM global_entries", [], |row| row.get(0))
        .expect("count the fixed source-health fixture entries");
    let volume_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM global_volumes", [], |row| row.get(0))
        .expect("count source-health fixture volumes");
    let enabled_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM global_volumes WHERE enabled = 1",
            [],
            |row| row.get(0),
        )
        .expect("count enabled source-health volumes");
    let disabled_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM global_volumes WHERE enabled = 0",
            [],
            |row| row.get(0),
        )
        .expect("count disabled source-health volumes");
    assert_eq!(entry_count, DIAGNOSTIC_ENTRIES as i64);
    assert_eq!(volume_count, topology.volume_count as i64);
    assert_eq!(enabled_count, topology.volume_count as i64);
    assert_eq!(disabled_count, 0);

    let mut statement = conn
        .prepare(
            r#"
            SELECT volume.id, volume.enabled, volume.entry_count, volume.updated_at,
                   COUNT(entry.id),
                   COALESCE(SUM(CASE WHEN entry.is_stale = 0 THEN 1 ELSE 0 END), 0),
                   COALESCE(SUM(CASE WHEN entry.is_stale = 1 THEN 1 ELSE 0 END), 0)
            FROM global_volumes volume
            LEFT JOIN global_entries entry ON entry.volume_id = volume.id
            GROUP BY volume.id, volume.enabled
            ORDER BY volume.id ASC
            "#,
        )
        .expect("prepare per-volume stale topology counts");
    let volumes = statement
        .query_map([], |row| {
            Ok(json!({
                "volume_id": row.get::<_, String>(0)?,
                "enabled": row.get::<_, i64>(1)? != 0,
                "global_volume_entry_count": row.get::<_, i64>(2)?,
                "global_volume_updated_at": row.get::<_, i64>(3)?,
                "entry_count": row.get::<_, i64>(4)?,
                "active_entry_count": row.get::<_, i64>(5)?,
                "stale_entry_count": row.get::<_, i64>(6)?
            }))
        })
        .expect("read per-volume stale topology counts")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect per-volume stale topology counts");
    assert_eq!(volumes.len(), topology.volume_count);
    let rows_per_volume = DIAGNOSTIC_ENTRIES as i64 / topology.volume_count as i64;
    let stale_per_volume = rows_per_volume * topology.stale_percent as i64 / 100;
    for volume in &volumes {
        assert_eq!(volume["enabled"], true);
        assert_eq!(volume["entry_count"], rows_per_volume);
        assert_eq!(
            volume["global_volume_entry_count"],
            rows_per_volume - stale_per_volume
        );
        assert_eq!(
            volume["active_entry_count"],
            rows_per_volume - stale_per_volume
        );
        assert_eq!(volume["stale_entry_count"], stale_per_volume);
    }
    let total_stale = stale_per_volume * topology.volume_count as i64;
    let actual_stale_percent = total_stale as f64 / entry_count as f64 * 100.0;
    assert_eq!(actual_stale_percent, topology.stale_percent as f64);

    json!({
        "topology": topology.name,
        "entry_count": entry_count,
        "volume_count": volume_count,
        "enabled_volume_count": enabled_count,
        "disabled_volume_count": disabled_count,
        "target_stale_percent": topology.stale_percent,
        "active_entry_count": entry_count - total_stale,
        "stale_entry_count": total_stale,
        "actual_stale_percent": actual_stale_percent,
        "volume_rows": volumes,
        "all_volume_rows_enabled": true
    })
}

fn schema_index_signature(conn: &Connection) -> String {
    let mut statement = conn
        .prepare(
            "SELECT type, name, COALESCE(sql, '') FROM sqlite_master WHERE type IN ('table', 'index', 'trigger') ORDER BY type, name",
        )
        .expect("prepare disposable fixture schema signature");
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("read disposable fixture schema signature")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect disposable fixture schema signature");
    blake3::hash(&serde_json::to_vec(&rows).expect("serialize fixture schema signature"))
        .to_hex()
        .to_string()
}

fn sqlite_runtime_settings(conn: &Connection) -> JsonValue {
    let sqlite_version: String = conn
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .expect("read SQLite runtime version");
    json!({
        "sqlite_version": sqlite_version,
        "journal_mode": conn.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0)).expect("read journal mode"),
        "synchronous": conn.query_row("PRAGMA synchronous", [], |row| row.get::<_, i64>(0)).expect("read synchronous"),
        "foreign_keys": conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0)).expect("read foreign keys"),
        "temp_store": conn.query_row("PRAGMA temp_store", [], |row| row.get::<_, i64>(0)).expect("read temp store"),
        "mmap_size": conn.query_row("PRAGMA mmap_size", [], |row| row.get::<_, i64>(0)).expect("read mmap size"),
        "page_size": conn.query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0)).expect("read page size")
    })
}

fn measure_source_health_sql_rows(
    conn: &Connection,
    candidate: SourceHealthCandidate,
) -> (Vec<SourceHealthSqlRow>, JsonValue) {
    let mut statement = conn
        .prepare(candidate.sql())
        .expect("prepare topology source-health SQL candidate");
    for status in [
        StatementStatus::Sort,
        StatementStatus::FullscanStep,
        StatementStatus::VmStep,
    ] {
        statement.reset_status(status);
    }
    let rows = statement
        .query_map([], |row| {
            Ok(SourceHealthSqlRow {
                source_id: row.get(0)?,
                enabled: row.get::<_, i64>(1)? != 0,
                provider: row.get(2)?,
                status: row.get(3)?,
                last_error: row.get(4)?,
                updated_at: row.get(5)?,
                active_entry_count: row.get(6)?,
                max_last_seen_at: row.get(7)?,
            })
        })
        .expect("execute topology source-health SQL candidate")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect topology source-health SQL candidate");
    let counters = json!({
        "sort_operations": statement.get_status(StatementStatus::Sort),
        "fullscan_steps": statement.get_status(StatementStatus::FullscanStep),
        "vm_steps": statement.get_status(StatementStatus::VmStep),
        "scope": "last complete SQL execution; VM steps and fullscan counters are work indicators, not CPU time or isolated temporary bytes"
    });
    (rows, counters)
}

fn profile_topology_source_health_sql(
    conn: &Connection,
    topology_facts: &JsonValue,
    context: &JsonValue,
    resources: &SourceHealthResourceSampler,
) {
    let expected = measure_source_health_sql_rows(conn, SourceHealthCandidate::Original).0;
    assert_eq!(
        expected.len(),
        topology_facts["volume_count"].as_u64().unwrap() as usize
    );
    let mut samples = std::array::from_fn::<_, 3, _>(|_| Vec::with_capacity(PAIRED_SAMPLES));
    let mut counters = vec![JsonValue::Null; SOURCE_HEALTH_CANDIDATES.len()];

    for warmup in 0..PAIRED_WARMUPS {
        for candidate in topology_candidate_order(warmup) {
            let (observed, _) = resources.measure(candidate, false, || {
                measure_source_health_sql_rows(conn, candidate)
            });
            assert_eq!(observed.0, expected);
        }
    }
    for sample in 0..PAIRED_SAMPLES {
        for candidate in topology_candidate_order(sample) {
            let candidate_index = SOURCE_HEALTH_CANDIDATES
                .iter()
                .position(|current| *current == candidate)
                .expect("candidate belongs to topology diagnostic");
            let ((observed, statement_counters), elapsed_ms) =
                resources.measure(candidate, false, || {
                    measure_source_health_sql_rows(conn, candidate)
                });
            assert_eq!(observed, expected);
            samples[candidate_index].push(elapsed_ms);
            counters[candidate_index] = statement_counters;
        }
    }

    let original_facts =
        load_global_search_source_health_candidate(conn, SourceHealthCandidate::Original)
            .expect("load ordered Original source facts");
    for (index, candidate) in SOURCE_HEALTH_CANDIDATES.iter().copied().enumerate() {
        let facts = load_global_search_source_health_candidate(conn, candidate)
            .expect("load ordered candidate source facts");
        assert_eq!(facts.source_health, original_facts.source_health);
        assert_eq!(
            facts.revision_facts_json,
            original_facts.revision_facts_json
        );
        assert_eq!(
            facts.source_revision.as_bytes(),
            original_facts.source_revision.as_bytes()
        );
        let explain = explain_details(conn, candidate.sql(), &[]);
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "source_health_topology_sql_paired",
            "context": context,
            "topology": topology_facts,
            "candidate_label": source_health_topology_candidate_label(candidate),
            "candidate": source_health_candidate_name(candidate),
            "paired_group": "Original, production Candidate B, and test-only Candidate C rotate first/second/third across samples on the same runner, database, and connection",
            "warmups_per_candidate": PAIRED_WARMUPS,
            "samples_per_candidate": PAIRED_SAMPLES,
            "latency_ms": summarize_samples(&samples[index]),
            "source_health_row_count": expected.len(),
            "raw_ordered_sql_rows_equal_to_original": true,
            "ordered_source_facts_equal_to_original": true,
            "revision_facts_json_byte_equal_to_original": true,
            "source_revision_blake3_equal_to_original": true,
            "source_revision_blake3": facts.source_revision,
            "revision_facts_json": String::from_utf8(facts.revision_facts_json).expect("revision facts are UTF-8 JSON"),
            "sql": candidate.sql(),
            "explain_query_plan": explain,
            "sqlite_statement_counters": counters[index],
            "resources": resources.metrics(candidate, false),
            "timing_scope": "statement prepare, full ordered row drain and row mapping; excludes EXPLAIN"
        }));
    }
}

fn profile_topology_repository_snapshots(
    db: &Database,
    topology_facts: &JsonValue,
    context: &JsonValue,
    resources: &SourceHealthResourceSampler,
) {
    for (query_class, query) in [
        ("no_result", "zzznomatchtoken"),
        ("high_hit_fts_report", "report"),
    ] {
        let original = run_snapshot_variant(db, query, SourceHealthCandidate::Original);
        let b = run_snapshot_variant(db, query, SourceHealthCandidate::NarrowAggregate);
        let c = run_snapshot_variant(db, query, SourceHealthCandidate::GroupByVolumeId);
        assert_snapshot_results_equal(&original, &b);
        assert_snapshot_results_equal(&original, &c);
        if query_class == "high_hit_fts_report" {
            assert_eq!(original.results.len(), QUERY_LIMIT as usize);
        } else {
            assert!(original.results.is_empty());
        }

        for warmup in 0..PAIRED_WARMUPS {
            for candidate in topology_candidate_order(warmup) {
                let (snapshot, _) = resources.measure(candidate, true, || {
                    run_snapshot_variant(db, query, candidate)
                });
                assert_snapshot_results_equal(&original, &snapshot);
            }
        }
        let mut samples = std::array::from_fn::<_, 3, _>(|_| Vec::with_capacity(PAIRED_SAMPLES));
        for sample in 0..PAIRED_SAMPLES {
            for candidate in topology_candidate_order(sample) {
                let candidate_index = SOURCE_HEALTH_CANDIDATES
                    .iter()
                    .position(|current| *current == candidate)
                    .expect("candidate belongs to topology diagnostic");
                let (snapshot, elapsed_ms) = resources.measure(candidate, true, || {
                    run_snapshot_variant(db, query, candidate)
                });
                assert_snapshot_results_equal(&original, &snapshot);
                samples[candidate_index].push(elapsed_ms);
            }
        }
        for (index, candidate) in SOURCE_HEALTH_CANDIDATES.iter().copied().enumerate() {
            let final_snapshot = run_snapshot_variant(db, query, candidate);
            assert_snapshot_results_equal(&original, &final_snapshot);
            emit_record(&json!({
                "schema_version": 1,
                "record_type": "source_health_topology_repository_snapshot_paired",
                "context": context,
                "topology": topology_facts,
                "query_class": query_class,
                "query": query,
                "candidate_label": source_health_topology_candidate_label(candidate),
                "candidate": source_health_candidate_name(candidate),
                "paired_group": "the exact Database::search_global_entries_snapshot method executes each candidate inside its normal single read transaction; Candidate B is the unoverridden production path, Original and C use the existing test-only SQL override",
                "warmups_per_candidate": PAIRED_WARMUPS,
                "samples_per_candidate": PAIRED_SAMPLES,
                "latency_ms": summarize_samples(&samples[index]),
                "result_count": final_snapshot.results.len(),
                "result_ids_prefix": final_snapshot.results.iter().take(8).map(|row| row.id.clone()).collect::<Vec<_>>(),
                "source_health_equal_to_original": true,
                "source_revision_byte_equal_to_original": true,
                "index_status_equal_to_original": true,
                "search_results_equal_to_original": true,
                "resources": resources.metrics(candidate, true),
                "timing_scope": "pool checkout, transaction setup, exact search tier, source-health row mapping, revision JSON/BLAKE3, index status and transaction commit"
            }));
        }
    }
}

fn profile_repository_snapshot_candidates(db: &Database, path: &Path, context: &JsonValue) {
    for (class, query) in [
        ("no_result", "zzznomatchtoken"),
        ("name_prefix", "quarterly"),
        ("fts_high_fanout_report", "report"),
    ] {
        let original_kind = SourceHealthCandidate::Original;
        for candidate in [
            SourceHealthCandidate::CorrelatedAggregates,
            SourceHealthCandidate::NarrowAggregate,
        ] {
            for _ in 0..PAIRED_WARMUPS {
                let original = run_snapshot_variant(db, query, original_kind);
                let optimized = run_snapshot_variant(db, query, candidate);
                assert_snapshot_results_equal(&original, &optimized);
            }
            let mut original_samples = Vec::with_capacity(PAIRED_SAMPLES);
            let mut candidate_samples = Vec::with_capacity(PAIRED_SAMPLES);
            for sample in 0..PAIRED_SAMPLES {
                let (original, original_ms, optimized, candidate_ms) = if sample % 2 == 0 {
                    let started = Instant::now();
                    let original = run_snapshot_variant(db, query, original_kind);
                    let original_ms = started.elapsed().as_secs_f64() * 1_000.0;
                    let started = Instant::now();
                    let optimized = run_snapshot_variant(db, query, candidate);
                    let candidate_ms = started.elapsed().as_secs_f64() * 1_000.0;
                    (original, original_ms, optimized, candidate_ms)
                } else {
                    let started = Instant::now();
                    let optimized = run_snapshot_variant(db, query, candidate);
                    let candidate_ms = started.elapsed().as_secs_f64() * 1_000.0;
                    let started = Instant::now();
                    let original = run_snapshot_variant(db, query, original_kind);
                    let original_ms = started.elapsed().as_secs_f64() * 1_000.0;
                    (original, original_ms, optimized, candidate_ms)
                };
                assert_snapshot_results_equal(&original, &optimized);
                original_samples.push(original_ms);
                candidate_samples.push(candidate_ms);
            }
            let final_snapshot = run_snapshot_variant(db, query, candidate);
            emit_record(&json!({
                "schema_version": 1,
                "record_type": "repository_snapshot_paired",
                "context": context,
                "query_class": class,
                "query": query,
                "candidate": source_health_candidate_name(candidate),
                "pairing": "alternating complete snapshot calls, same 500k database and connection pool; each call performs search, source-health/revision and index status in one read transaction",
                "warmups_per_variant": PAIRED_WARMUPS,
                "samples_per_variant": PAIRED_SAMPLES,
                "original": summarize_samples(&original_samples),
                "candidate_result": summarize_samples(&candidate_samples),
                "source_health_equal": true,
                "source_revision_byte_equal": true,
                "index_status_equal": true,
                "search_results_equal": true,
                "result_count": final_snapshot.results.len(),
                "database": database_metrics(
                    &db.conn().expect("borrow database metrics connection"),
                    path,
                )
            }));
        }

        // The candidate selected for production is also exercised through the
        // public repository entry point on this same large fixture.
        let candidate = SourceHealthCandidate::NarrowAggregate;
        for _ in 0..PAIRED_WARMUPS {
            let actual = db
                .search_global_entries_snapshot(query, QUERY_LIMIT, 0)
                .expect("warm production repository snapshot");
            let candidate_snapshot = run_snapshot_variant(db, query, candidate);
            assert_snapshot_results_equal(&actual, &candidate_snapshot);
        }
        let mut production_samples = Vec::with_capacity(PAIRED_SAMPLES);
        let mut candidate_samples = Vec::with_capacity(PAIRED_SAMPLES);
        for sample in 0..PAIRED_SAMPLES {
            let (production, production_ms, candidate_snapshot, candidate_ms) = if sample % 2 == 0 {
                let started = Instant::now();
                let production = db
                    .search_global_entries_snapshot(query, QUERY_LIMIT, 0)
                    .expect("measure actual production repository snapshot");
                let production_ms = started.elapsed().as_secs_f64() * 1_000.0;
                let started = Instant::now();
                let candidate_snapshot = run_snapshot_variant(db, query, candidate);
                let candidate_ms = started.elapsed().as_secs_f64() * 1_000.0;
                (production, production_ms, candidate_snapshot, candidate_ms)
            } else {
                let started = Instant::now();
                let candidate_snapshot = run_snapshot_variant(db, query, candidate);
                let candidate_ms = started.elapsed().as_secs_f64() * 1_000.0;
                let started = Instant::now();
                let production = db
                    .search_global_entries_snapshot(query, QUERY_LIMIT, 0)
                    .expect("measure actual production repository snapshot");
                let production_ms = started.elapsed().as_secs_f64() * 1_000.0;
                (production, production_ms, candidate_snapshot, candidate_ms)
            };
            assert_snapshot_results_equal(&production, &candidate_snapshot);
            production_samples.push(production_ms);
            candidate_samples.push(candidate_ms);
        }
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "production_repository_snapshot_validation",
            "context": context,
            "query_class": class,
            "query": query,
            "candidate": source_health_candidate_name(candidate),
            "pairing": "actual Database::search_global_entries_snapshot and test-only candidate snapshot alternate on the same 500k fixture; both use a read transaction for search, source facts and index status",
            "warmups_per_variant": PAIRED_WARMUPS,
            "samples_per_variant": PAIRED_SAMPLES,
            "production_entrypoint": summarize_samples(&production_samples),
            "candidate_snapshot_helper": summarize_samples(&candidate_samples),
            "source_health_equal": true,
            "source_revision_byte_equal": true,
            "index_status_equal": true,
            "search_results_equal": true
        }));
    }
}

fn measure_production_tier(
    context: &JsonValue,
    conn: &Connection,
    class: &str,
    query: &str,
    tier: &str,
    limit: u32,
    sample_count: usize,
) -> Vec<GlobalSearchResult> {
    let run = || {
        diagnostic_search_tier(conn, tier, query, limit)
            .unwrap_or_else(|error| panic!("run production {tier} tier: {error}"))
    };
    let expected = run();
    for _ in 0..PAIRED_WARMUPS {
        assert_eq!(run(), expected);
    }
    let mut samples = Vec::with_capacity(sample_count);
    for _ in 0..sample_count {
        let (results, elapsed) = time_results(run);
        assert_eq!(results, expected);
        samples.push(elapsed);
    }
    record_production_results(
        context,
        class,
        query,
        "production_tier_exact_helper",
        samples,
        &expected,
        "search.rs test-only wrapper around the actual production tier; connection reuse; includes statement preparation and result mapping",
    );
    expected
}

fn profile_full_search_entrypoint(
    context: &JsonValue,
    db: &Database,
    conn: &Connection,
    path: &Path,
    class: &str,
    query: &str,
) -> Vec<GlobalSearchResult> {
    let expected = db
        .search_global_entries(query, QUERY_LIMIT, 0)
        .expect("warm Global Search through pooled Database entrypoint");
    for _ in 0..PAIRED_WARMUPS {
        assert_eq!(
            db.search_global_entries(query, QUERY_LIMIT, 0)
                .expect("warm Global Search through pooled Database entrypoint"),
            expected
        );
    }
    let mut pooled_samples = Vec::with_capacity(PAIRED_SAMPLES);
    for _ in 0..PAIRED_SAMPLES {
        let started = Instant::now();
        let results = db
            .search_global_entries(query, QUERY_LIMIT, 0)
            .expect("search through pooled Database entrypoint");
        pooled_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        assert_eq!(results, expected);
    }
    record_production_results(
        context,
        class,
        query,
        "full_search_database_pool_entrypoint",
        pooled_samples,
        &expected,
        "Database::search_global_entries, includes pool checkout and all production tiers",
    );

    let snapshot_expected = db
        .search_global_entries_snapshot(query, QUERY_LIMIT, 0)
        .expect("warm repository search snapshot")
        .results;
    assert_eq!(snapshot_expected, expected);
    for _ in 0..PAIRED_WARMUPS {
        assert_eq!(
            db.search_global_entries_snapshot(query, QUERY_LIMIT, 0)
                .expect("warm repository search snapshot")
                .results,
            expected
        );
    }
    let mut snapshot_samples = Vec::with_capacity(PAIRED_SAMPLES);
    for _ in 0..PAIRED_SAMPLES {
        let started = Instant::now();
        let snapshot = db
            .search_global_entries_snapshot(query, QUERY_LIMIT, 0)
            .expect("search with repository snapshot facts");
        snapshot_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        assert_eq!(snapshot.results, expected);
    }
    record_production_results(
        context,
        class,
        query,
        "full_search_repository_snapshot",
        snapshot_samples,
        &expected,
        "GlobalIndexRepository snapshot read transaction plus source health, revision and index status; excludes Tauri command/coordinator serialization",
    );

    let expected_connection = search_global_entries_on_connection(conn, query, QUERY_LIMIT, 0)
        .expect("search on persistent diagnostic connection");
    assert_eq!(expected_connection, expected);
    let mut connection_samples = Vec::with_capacity(PAIRED_SAMPLES);
    for _ in 0..PAIRED_SAMPLES {
        let started = Instant::now();
        let results = search_global_entries_on_connection(conn, query, QUERY_LIMIT, 0)
            .expect("search on persistent diagnostic connection");
        connection_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        assert_eq!(results, expected);
    }
    record_production_results(
        context,
        class,
        query,
        "full_search_persistent_connection",
        connection_samples,
        &expected,
        "search_global_entries_on_connection; excludes pool checkout and repository source/status snapshot",
    );

    if matches!(
        class,
        "name_prefix"
            | "common_prefix_high_fanout"
            | "fts_high_fanout_report"
            | "fts_high_fanout_invoice"
    ) {
        let mut reopened_samples = Vec::with_capacity(PAIRED_SAMPLES);
        for _ in 0..PAIRED_WARMUPS {
            let reopened =
                configured_connection(path).expect("open warmup reopened SQLite connection");
            assert_eq!(
                search_global_entries_on_connection(&reopened, query, QUERY_LIMIT, 0)
                    .expect("warm search on reopened SQLite connection"),
                expected
            );
        }
        for _ in 0..PAIRED_SAMPLES {
            let reopened = configured_connection(path).expect("open reopened SQLite connection");
            let started = Instant::now();
            let results = search_global_entries_on_connection(&reopened, query, QUERY_LIMIT, 0)
                .expect("search on reopened SQLite connection");
            reopened_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
            assert_eq!(results, expected);
        }
        record_production_results(
            context,
            class,
            query,
            "full_search_reopened_connection",
            reopened_samples,
            &expected,
            "each sample opens a fresh configured SQLite connection outside the timed interval; the operating-system file cache is not cleared",
        );
    }
    expected
}

fn add_volume(db: &Database, id: &str, enabled: bool) {
    let mut volume = super::super::test_volume();
    volume.id = id.to_string();
    volume.stable_volume_id = format!("issue352-{id}");
    volume.display_name = id.to_string();
    volume.mount_path = format!(r"C:\{id}\");
    volume.enabled = enabled;
    db.upsert_global_volume(&volume)
        .expect("add adversarial test volume");
}

fn rollback_insert_samples(conn: &Connection, samples: usize, rows_per_sample: u64) -> Vec<f64> {
    let rows = (0..rows_per_sample)
        .map(|offset| super::generated_entry(DIAGNOSTIC_ENTRIES + 10_000 + offset))
        .collect::<Vec<_>>();
    let mut timings = Vec::with_capacity(samples);
    for _ in 0..samples {
        let transaction = conn
            .unchecked_transaction()
            .expect("begin rollback-only index maintenance sample");
        let mut statement = transaction
            .prepare(INSERT_SYNTHETIC_ENTRY)
            .expect("prepare rollback-only index maintenance insert");
        let started = Instant::now();
        for entry in &rows {
            statement
                .execute(params![
                    entry.id,
                    "gv_test",
                    entry.platform_file_id,
                    entry.parent_platform_file_id,
                    entry.name,
                    entry.name_normalized,
                    entry.path,
                    entry.path_normalized,
                    entry.extension,
                    0_i64,
                    entry.size,
                    entry.created_at_fs,
                    entry.modified_at_fs,
                    entry.file_attributes,
                    i64::from(entry.is_hidden),
                    i64::from(entry.is_system),
                    0_i64,
                    "issue352_index_write_cost",
                    1_700_600_000_i64 + entry.index as i64,
                ])
                .expect("insert rollback-only index maintenance row");
        }
        timings.push(started.elapsed().as_secs_f64() * 1_000.0);
        drop(statement);
        transaction
            .rollback()
            .expect("rollback index maintenance sample rows");
    }
    timings
}

fn insert_diagnostic_entry(
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
        .map(|(_, suffix)| suffix.to_lowercase())
        .unwrap_or_default();
    conn.execute(
        INSERT_SYNTHETIC_ENTRY,
        params![
            id,
            volume_id,
            id,
            "issue352-parent",
            name,
            normalized_name,
            path,
            normalized_path,
            extension,
            0_i64,
            1_i64,
            1_700_000_000_i64,
            modified_at,
            0_i64,
            0_i64,
            0_i64,
            0_i64,
            "issue352_query_cost_diagnostic",
            1_700_000_000_i64,
        ],
    )
    .unwrap_or_else(|error| panic!("insert adversarial entry {id}: {error}"));
}

fn populate_scale_overlay(conn: &Connection) -> (f64, Vec<(String, usize)>) {
    let started = Instant::now();
    let transaction = conn
        .unchecked_transaction()
        .expect("begin match-scale overlay transaction");
    let mut statement = transaction
        .prepare(INSERT_SYNTHETIC_ENTRY)
        .expect("prepare scale overlay insert");
    for (label, count) in [
        ("scale5", 5_usize),
        ("scale100", 100),
        ("scale1000", 1_000),
        ("scale4096", 4_096),
        ("scale10000", 10_000),
        ("scale25000", 25_000),
    ] {
        for index in 0..count {
            let id = format!("issue352-{label}-{index:05}");
            let name = format!("{label} candidate {index:05}.txt");
            let normalized_name = name.to_lowercase();
            let path = format!(r"C:\Issue352Scale\{label}\{name}");
            let normalized_path = path.to_lowercase();
            statement
                .execute(params![
                    id,
                    "gv_test",
                    id,
                    "issue352-scale-parent",
                    name,
                    normalized_name,
                    path,
                    normalized_path,
                    "txt",
                    0_i64,
                    1_i64,
                    1_700_500_000_i64,
                    1_700_500_000_i64 + index as i64,
                    0_i64,
                    0_i64,
                    0_i64,
                    0_i64,
                    "issue352_query_cost_scale",
                    1_700_500_000_i64,
                ])
                .unwrap_or_else(|error| panic!("insert match-scale overlay row: {error}"));
        }
    }
    drop(statement);

    let mut actual = Vec::new();
    for (label, expected) in [
        ("zzznomatchtoken", 0_usize),
        ("scale5", 5),
        ("scale100", 100),
        ("scale1000", 1_000),
        ("scale4096", 4_096),
        ("scale10000", 10_000),
        ("scale25000", 25_000),
    ] {
        let query = if expected == 0 {
            label.to_string()
        } else {
            format!("{label} ")
        };
        let glob = format!("{}*", benchmark_escape_glob(&query));
        let actual_count: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM global_entries ge JOIN global_volumes gv ON gv.id = ge.volume_id WHERE gv.enabled = 1 AND ge.is_stale = 0 AND ge.name_normalized GLOB ?1",
                [glob],
                |row| row.get(0),
            )
            .expect("count scale overlay matches");
        assert_eq!(actual_count as usize, expected, "scale count for {label}");
        actual.push((label.to_string(), expected));
    }
    let overlay_entries: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM global_entries WHERE id LIKE 'issue352-scale%'",
            [],
            |row| row.get(0),
        )
        .expect("count temporary scale rows");
    assert_eq!(
        overlay_entries, 40_201,
        "count temporary scale overlay rows"
    );

    for (label, expected) in &actual {
        let query = if label.starts_with("zzz") {
            label.clone()
        } else {
            format!("{label} ")
        };
        let count_sql = "SELECT ge.rowid FROM global_entries ge INDEXED BY idx_global_entries_active_name_order JOIN global_volumes gv ON gv.id = ge.volume_id WHERE gv.enabled = 1 AND ge.is_stale = 0 AND ge.name_normalized GLOB ?1 ORDER BY ge.modified_at_fs DESC, ge.id ASC LIMIT ?2";
        let values = vec![
            SqlValue::Text(format!("{}*", benchmark_escape_glob(&query))),
            SqlValue::Integer(QUERY_LIMIT as i64),
        ];
        let returned = profile_sql_stage(
            &transaction,
            &json!({"entries_before_overlay": DIAGNOSTIC_ENTRIES, "overlay_rows": overlay_entries}),
            "match_count_scaling",
            &query,
            "production_name_prefix_order_limit_80",
            count_sql,
            values,
        );
        assert_eq!(returned, (*expected).min(QUERY_LIMIT as usize));
    }
    transaction
        .rollback()
        .expect("rollback temporary match-scale overlay");
    (started.elapsed().as_secs_f64() * 1_000.0, actual)
}

fn add_adversarial_rows(db: &Database, conn: &mut Connection, context: &JsonValue) {
    add_volume(db, "gv_issue352_second_enabled", true);
    add_volume(db, "gv_issue352_disabled", false);
    let transaction = conn
        .unchecked_transaction()
        .expect("begin adversarial semantic transaction");

    for index in 0..100 {
        insert_diagnostic_entry(
            &transaction,
            &format!("issue352-disabled-quarterly-{index:03}"),
            "gv_issue352_disabled",
            &format!("Quarterly Review 2026 disabled-{index:03}.pdf"),
            &format!(r"C:\Disabled\Quarterly\{index:03}.pdf"),
            1_900_000_000 + index,
        );
        insert_diagnostic_entry(
            &transaction,
            &format!("issue352-stale-quarterly-{index:03}"),
            "gv_test",
            &format!("Quarterly Review 2026 stale-{index:03}.pdf"),
            &format!(r"C:\Stale\Quarterly\{index:03}.pdf"),
            1_950_000_000 + index,
        );
        insert_diagnostic_entry(
            &transaction,
            &format!("issue352-disabled-report-{index:03}"),
            "gv_issue352_disabled",
            "report.txt",
            &format!(r"C:\Disabled\r{index:03}.txt"),
            1_900_000_000 + index,
        );
        insert_diagnostic_entry(
            &transaction,
            &format!("issue352-stale-report-{index:03}"),
            "gv_test",
            "report.txt",
            &format!(r"C:\Stale\r{index:03}.txt"),
            1_950_000_000 + index,
        );
    }
    transaction
        .execute(
            "UPDATE global_entries SET is_stale = 1 WHERE id LIKE 'issue352-stale-%'",
            [],
        )
        .expect("mark adversarial stale rows");

    insert_diagnostic_entry(
        &transaction,
        "issue352-tie-a",
        "gv_test",
        "Quarterly Review 2026 duplicate.pdf",
        r"C:\Shared\Quarterly\duplicate.pdf",
        1_800_000_000,
    );
    insert_diagnostic_entry(
        &transaction,
        "issue352-tie-b",
        "gv_issue352_second_enabled",
        "Quarterly Review 2026 duplicate.pdf",
        r"C:\Shared\Quarterly\duplicate.pdf",
        1_800_000_000,
    );
    insert_diagnostic_entry(
        &transaction,
        "issue352-report-tie-a",
        "gv_test",
        "Report-Alpha.txt",
        r"C:\Shared\Report\tie.txt",
        1_800_000_000,
    );
    insert_diagnostic_entry(
        &transaction,
        "issue352-report-tie-b",
        "gv_issue352_second_enabled",
        "Report-Bravo.txt",
        r"C:\Shared\Report\tie.txt",
        1_800_000_000,
    );
    insert_diagnostic_entry(
        &transaction,
        "issue352-managed",
        "gv_test",
        "Quarterly Review 2026 managed.pdf",
        r"C:\Managed\Quarterly\managed.pdf",
        1_790_000_000,
    );
    insert_diagnostic_entry(
        &transaction,
        "issue352-unmanaged",
        "gv_test",
        "Quarterly Review 2026 unmanaged.pdf",
        r"C:\Unmanaged\Quarterly\unmanaged.pdf",
        1_780_000_000,
    );
    insert_diagnostic_entry(
        &transaction,
        "issue352-cross-tier-jp",
        "gv_test",
        "jp-crossover.jpg",
        r"C:\Shared\jp-crossover.jpg",
        1_770_000_000,
    );
    transaction
        .execute_batch(
            r#"
            INSERT INTO managed_scopes (
                id, path, global_entry_id, enabled, allow_local_ai, allow_cloud_ai,
                created_at, updated_at
            ) VALUES (
                'issue352-scope', 'C:\Managed\Quarterly', 'issue352-managed',
                1, 1, 0, 1700000000, 1700000000
            );
            INSERT INTO managed_entries (
                id, global_entry_id, managed_scope_id, enabled, created_at, updated_at
            ) VALUES (
                'issue352-managed-row', 'issue352-managed', 'issue352-scope',
                1, 1700000000, 1700000000
            );
            "#,
        )
        .expect("add managed marker to the enabled test scope");

    let production = with_legacy_name_prefix_search_candidate(|| {
        diagnostic_search_tier(&transaction, "name_prefix", "quarterly", 80)
    })
    .expect("run legacy prefix tier on adversarial rows");
    let values = prefix_variant_values("quarterly", 80);
    let safe = run_result_sql(&transaction, SAFE_PREFIX_CTE_SQL, &values);
    assert_eq!(
        safe, production,
        "safe prefix CTE must preserve every result field"
    );
    let cte_helper = with_key_only_cte_search_candidate(|| {
        diagnostic_search_tier(&transaction, "name_prefix", "quarterly", 80)
    })
    .expect("run key-only CTE through the production prefix helper");
    assert_eq!(
        cte_helper, production,
        "key-only CTE helper must preserve every production prefix field"
    );
    assert!(production.iter().any(|row| row.id == "issue352-tie-a"));
    let tie_a = production
        .iter()
        .position(|row| row.id == "issue352-tie-a")
        .expect("first tied entry");
    let tie_b = production
        .iter()
        .position(|row| row.id == "issue352-tie-b")
        .expect("second tied entry");
    assert!(tie_a < tie_b, "ID tie-break must remain ascending");
    assert!(production
        .iter()
        .any(|row| row.id == "issue352-managed" && row.managed));
    assert!(production
        .iter()
        .any(|row| row.id == "issue352-unmanaged" && !row.managed));
    assert!(!production.iter().any(|row| row.id.contains("disabled")));
    assert!(!production.iter().any(|row| row.id.contains("stale")));

    let production_4096 = with_legacy_name_prefix_search_candidate(|| {
        diagnostic_search_tier(
            &transaction,
            "name_prefix",
            "quarterly",
            SEARCH_CANDIDATE_LIMIT as u32,
        )
    })
    .expect("collect legacy prefix candidate window");
    let safe_4096 = run_result_sql(
        &transaction,
        SAFE_PREFIX_CTE_SQL,
        &prefix_variant_values("quarterly", SEARCH_CANDIDATE_LIMIT as u32),
    );
    assert_eq!(
        safe_4096, production_4096,
        "4096-candidate prefix CTE equivalence"
    );
    let production_page = with_legacy_name_prefix_search_candidate(|| {
        search_global_entries_on_connection(&transaction, "quarterly", 80, 4_016)
    })
    .expect("legacy search at candidate-window boundary");
    assert_eq!(production_page, production_4096[4_016..4_096]);
    let cte_page = with_key_only_cte_search_candidate(|| {
        search_global_entries_on_connection(&transaction, "quarterly", 80, 4_016)
    })
    .expect("key-only CTE full search at candidate-window boundary");
    assert_eq!(cte_page, production_page);
    assert!(with_legacy_name_prefix_search_candidate(|| {
        search_global_entries_on_connection(&transaction, "quarterly", 80, 4_096)
    })
    .expect("legacy search after candidate cap")
    .is_empty());
    assert_eq!(
        with_legacy_name_prefix_search_candidate(|| {
            search_global_entries_on_connection(&transaction, "quarterly", 500, 0)
        })
        .expect("legacy search limit clamp")
        .len(),
        200
    );
    let clamped_production = with_legacy_name_prefix_search_candidate(|| {
        search_global_entries_on_connection(&transaction, "quarterly", 500, 0)
    })
    .expect("legacy search result at clamped limit");
    let clamped_candidate = run_result_sql(
        &transaction,
        SAFE_PREFIX_CTE_SQL,
        &prefix_variant_values("quarterly", 200),
    );
    assert_eq!(clamped_candidate, clamped_production);
    assert_eq!(
        with_key_only_cte_search_candidate(|| {
            search_global_entries_on_connection(&transaction, "quarterly", 500, 0)
        })
        .expect("key-only CTE full search with the production limit clamp"),
        clamped_production
    );

    let duplicate_tier_results = with_legacy_name_prefix_search_candidate(|| {
        search_global_entries_on_connection(
            &transaction,
            "Quarterly Review 2026 duplicate.pdf",
            80,
            0,
        )
    })
    .expect("search duplicate basename across two enabled volumes");
    assert_eq!(
        duplicate_tier_results
            .iter()
            .map(|row| row.id.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        duplicate_tier_results.len(),
        "tier overlap must be deduplicated by stable entry ID"
    );
    assert_eq!(duplicate_tier_results.len(), 2);
    assert!(duplicate_tier_results
        .iter()
        .any(|row| row.id == "issue352-tie-a"));
    assert!(duplicate_tier_results
        .iter()
        .any(|row| row.id == "issue352-tie-b"));

    let cross_tier_results = with_legacy_name_prefix_search_candidate(|| {
        search_global_entries_on_connection(&transaction, "jp", 80, 0)
    })
    .expect("search for an entry matching both a basename and extension prefix");
    assert_eq!(
        cross_tier_results
            .iter()
            .filter(|row| row.id == "issue352-cross-tier-jp")
            .count(),
        1,
        "one row matching two tiers must be de-duplicated by stable entry ID"
    );
    assert_eq!(
        with_key_only_cte_search_candidate(|| {
            search_global_entries_on_connection(&transaction, "jp", 80, 0)
        })
        .expect("key-only CTE full search for the cross-tier duplicate"),
        cross_tier_results
    );

    let unsafe_prefix = r#"
        WITH candidates AS MATERIALIZED (
            SELECT ge.rowid AS entry_rowid, ge.modified_at_fs, ge.id
            FROM global_entries ge INDEXED BY idx_global_entries_active_name_order
            WHERE ge.is_stale = 0 AND ge.name_normalized GLOB ?1
            ORDER BY ge.modified_at_fs DESC, ge.id ASC
            LIMIT ?2
        )
        SELECT ge.rowid
        FROM candidates
        JOIN global_entries ge ON ge.rowid = candidates.entry_rowid
        JOIN global_volumes gv ON gv.id = ge.volume_id
        WHERE gv.enabled = 1 AND ge.is_stale = 0
        ORDER BY candidates.modified_at_fs DESC, candidates.id ASC
    "#;
    let unsafe_count = run_count_sql(
        &transaction,
        unsafe_prefix,
        &[
            SqlValue::Text("quarterly*".to_string()),
            SqlValue::Integer(80),
        ],
    );
    assert!(
        unsafe_count < 80,
        "limit-before-enabled-filter should underfill"
    );
    assert_eq!(
        production.len(),
        80,
        "filtered-first production query must fill page"
    );

    let fts_production =
        with_legacy_fts_search_candidate(|| diagnostic_search_fts(&transaction, "report", 80))
            .expect("run legacy FTS tier on adversarial rows");
    let fts_safe = run_result_sql(
        &transaction,
        SAFE_FTS_CTE_SQL,
        &fts_variant_values("report", 80),
    );
    assert_eq!(
        fts_safe, fts_production,
        "safe FTS CTE must preserve every result field"
    );
    let fts_cte_helper = with_key_only_fts_search_candidate(|| {
        diagnostic_search_tier(&transaction, "fts", "report", 80)
    })
    .expect("run key-only CTE through the production FTS helper");
    assert_eq!(
        fts_cte_helper, fts_production,
        "key-only CTE helper must preserve every production FTS field"
    );
    assert!(!fts_production.iter().any(|row| row.id.contains("disabled")));
    assert!(!fts_production.iter().any(|row| row.id.contains("stale")));
    let fts_tie_a = fts_production
        .iter()
        .position(|row| row.id == "issue352-report-tie-a")
        .expect("first equal-rank FTS tie row");
    let fts_tie_b = fts_production
        .iter()
        .position(|row| row.id == "issue352-report-tie-b")
        .expect("second equal-rank FTS tie row");
    assert!(
        fts_tie_a < fts_tie_b,
        "FTS tie-break must remain ID ascending"
    );

    let unsafe_fts = r#"
        WITH candidates AS MATERIALIZED (
            SELECT ge.rowid AS entry_rowid,
                   ge.modified_at_fs,
                   ge.id,
                   bm25(global_entries_fts, 8.0, 2.0, 1.0) AS rank
            FROM global_entries_fts
            CROSS JOIN global_entries ge
            WHERE global_entries_fts MATCH ?1
              AND ge.rowid = global_entries_fts.rowid
            ORDER BY rank ASC, ge.modified_at_fs DESC, ge.id ASC
            LIMIT ?2
        )
        SELECT ge.rowid
        FROM candidates
        CROSS JOIN global_entries ge
        CROSS JOIN global_volumes gv
        WHERE ge.rowid = candidates.entry_rowid
          AND gv.id = ge.volume_id
          AND gv.enabled = 1
          AND ge.is_stale = 0
        ORDER BY candidates.rank ASC, candidates.modified_at_fs DESC, candidates.id ASC
    "#;
    let unsafe_fts_count = run_count_sql(
        &transaction,
        unsafe_fts,
        &[SqlValue::Text(fts_phrase("report")), SqlValue::Integer(80)],
    );
    assert!(
        unsafe_fts_count < 80,
        "limit-before-FTS-eligibility filter should underfill"
    );

    let fts_plan = explain_details(
        &transaction,
        &diagnostic_search_fts_sql(),
        &fts_variant_values("report", 80),
    );
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "semantic_equivalence_gate",
        "context": context,
        "production_vs_safe_prefix_cte_fields_equal": true,
        "production_vs_safe_fts_cte_fields_equal": true,
        "safe_prefix_cte_equal_through_4096": true,
        "limit_80_offset_4016_equal": true,
        "offset_4096_returns_empty": true,
        "limit_clamps_to_200": true,
        "tier_overlap_deduplicates_by_stable_entry_id": true,
        "enabled_volumes": 2,
        "disabled_volume_rows_excluded": true,
        "stale_rows_excluded": true,
        "managed_and_unmanaged_projection_equal": true,
        "duplicate_basename_and_cross_volume_same_path_preserved": true,
        "equal_mtime_id_tie_order_equal": true,
        "unsafe_prefix_filter_after_limit_returned": unsafe_count,
        "unsafe_fts_filter_after_limit_returned": unsafe_fts_count,
        "unsafe_variants_underfilled_80_row_page": true,
        "production_fts_plan": fts_plan,
        "fixture_is_transactional_and_rolled_back": true
    }));

    transaction
        .rollback()
        .expect("rollback adversarial semantic rows");
}

fn run_count_sql(conn: &Connection, sql: &str, values: &[SqlValue]) -> usize {
    conn.prepare(sql)
        .expect("prepare count diagnostic SQL")
        .query_map(params_from_iter(values.iter()), |row| row.get::<_, i64>(0))
        .expect("execute count diagnostic SQL")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect count diagnostic SQL")
        .len()
}

#[test]
#[ignore = "Owner-qualified Issue #359 topology diagnostic; one fixed 500k fixture, four rollbackable states"]
fn global_search_source_health_topology_diagnostic() {
    assert_eq!(
        env::var("ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES")
            .ok()
            .as_deref(),
        Some("500000"),
        "Issue #359 topology validation is fixed at 500,000 rows and must not become a 1m run"
    );
    assert_eq!(DIAGNOSTIC_ENTRIES, 500_000);
    let output_path = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT").ok();
    if let Some(path) = &output_path {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create topology diagnostic evidence directory");
        }
        fs::write(path, "").expect("truncate topology diagnostic JSONL evidence");
    }

    let context = benchmark_context(DIAGNOSTIC_ENTRIES);
    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open the single 500k topology diagnostic fixture");
    db.upsert_global_volume(&test_volume())
        .expect("insert the one enabled base synthetic volume");

    let fixture_started = Instant::now();
    let (_expectations, generation_ms, population_ms, population_transactions) =
        populate_synthetic_database(&db, DIAGNOSTIC_ENTRIES);
    let fixture_build_wall_ms = fixture_started.elapsed().as_secs_f64() * 1_000.0;
    let mut conn = configured_connection(&path).expect("open configured topology SQL connection");
    let base_topology = SOURCE_HEALTH_TOPOLOGIES[0];
    let base_facts = source_health_topology_facts(&conn, base_topology);
    assert_eq!(base_facts["entry_count"], DIAGNOSTIC_ENTRIES);
    let base_volume_entry_count = base_facts["volume_rows"][0]["global_volume_entry_count"]
        .as_i64()
        .expect("read original base volume's cached active count");
    let base_volume_updated_at = base_facts["volume_rows"][0]["global_volume_updated_at"]
        .as_i64()
        .expect("read original base volume update timestamp");
    let schema_signature_before = schema_index_signature(&conn);
    let benchmark_connection_settings = sqlite_runtime_settings(&conn);
    let pooled_snapshot_connection_settings = {
        let pooled = db
            .conn()
            .expect("borrow repository pool connection for PRAGMAs");
        sqlite_runtime_settings(&pooled)
    };
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "source_health_topology_environment",
        "context": context,
        "runner": {
            "os": env::consts::OS,
            "arch": env::consts::ARCH,
            "image_os": env::var("ImageOS").ok(),
            "image_version": env::var("ImageVersion").ok(),
            "github_job": env::var("GITHUB_JOB").ok(),
            "github_run_attempt": env::var("GITHUB_RUN_ATTEMPT").ok()
        },
        "fixture_type": "one deterministic production-schema synthetic SQLite database, populated once, then topology rows updated transactionally and reverted on the same disposable file",
        "fixture_builds": 1,
        "entries_after_build": DIAGNOSTIC_ENTRIES,
        "base_topology": base_facts,
        "fixture_generation_ms": generation_ms,
        "fixture_population_ms": population_ms,
        "fixture_build_wall_ms": fixture_build_wall_ms,
        "fixture_population_transaction_ms": summarize_samples(&population_transactions),
        "benchmark_connection_sqlite_settings": benchmark_connection_settings,
        "repository_pool_sqlite_settings": pooled_snapshot_connection_settings,
        "database": database_metrics(&conn, &path),
        "schema_index_signature_before": schema_signature_before,
        "filesystem_file_discovery_measured": false,
        "full_500k_search_matrix_repeated": false,
        "one_million_row_benchmark_run": false
    }));

    let mut topologies_completed = 0;
    let mut topology_transition_ms = Vec::with_capacity(SOURCE_HEALTH_TOPOLOGIES.len());
    let mut topology_rollback_ms = Vec::with_capacity(SOURCE_HEALTH_TOPOLOGIES.len());
    for topology in SOURCE_HEALTH_TOPOLOGIES {
        let transition_ms = apply_source_health_topology(&mut conn, topology);
        topology_transition_ms.push(transition_ms);
        let topology_facts = source_health_topology_facts(&conn, topology);
        let resources = SourceHealthResourceSampler::new();
        let topology_pool_settings = {
            let pooled = db
                .conn()
                .expect("borrow repository pool connection for topology PRAGMAs");
            sqlite_runtime_settings(&pooled)
        };
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "source_health_topology_state",
            "context": context,
            "topology": topology_facts,
            "state_transition_ms": round_ms(transition_ms),
            "state_transition_committed_only_inside_disposable_fixture": true,
            "state_is_reverted_to_single_volume_active_base_before_next_variant": true,
            "single_fixture_path": path.file_name().and_then(|name| name.to_str()),
            "benchmark_connection_sqlite_settings": sqlite_runtime_settings(&conn),
            "repository_pool_sqlite_settings": topology_pool_settings,
            "database": database_metrics(&conn, &path)
        }));

        let mut fact_conn = db
            .conn()
            .expect("borrow source-health semantic comparison connection");
        let transaction = fact_conn
            .transaction()
            .expect("begin one-read-transaction source-facts equality check");
        let original_facts = load_global_search_source_health_candidate(
            &transaction,
            SourceHealthCandidate::Original,
        )
        .expect("load Original ordered source facts");
        for candidate in [
            SourceHealthCandidate::NarrowAggregate,
            SourceHealthCandidate::GroupByVolumeId,
        ] {
            let candidate_facts =
                load_global_search_source_health_candidate(&transaction, candidate)
                    .expect("load B/C ordered source facts in the same read transaction");
            assert_eq!(candidate_facts.source_health, original_facts.source_health);
            assert_eq!(
                candidate_facts.revision_facts_json,
                original_facts.revision_facts_json
            );
            assert_eq!(
                candidate_facts.source_revision.as_bytes(),
                original_facts.source_revision.as_bytes()
            );
        }
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "source_health_topology_revision_fact_equality",
            "context": context,
            "topology": topology_facts,
            "single_read_transaction": true,
            "ordered_source_facts": original_facts.source_health.iter().map(|fact| json!({
                "source_id": fact.source_id,
                "enabled": fact.enabled,
                "provider": fact.provider,
                "status": fact.status,
                "last_error": fact.last_error,
                "updated_at": fact.updated_at
            })).collect::<Vec<_>>(),
            "revision_facts_json": String::from_utf8(original_facts.revision_facts_json.clone()).expect("revision facts are UTF-8 JSON"),
            "source_revision_blake3": original_facts.source_revision,
            "Original_B_C_ordered_source_facts_equal": true,
            "Original_B_C_revision_facts_json_byte_equal": true,
            "Original_B_C_source_revision_blake3_byte_equal": true
        }));
        transaction
            .commit()
            .expect("commit read-only source-facts comparison snapshot");
        drop(fact_conn);

        profile_topology_source_health_sql(&conn, &topology_facts, &context, &resources);
        profile_topology_repository_snapshots(&db, &topology_facts, &context, &resources);
        drop(resources);

        let rollback_ms = restore_source_health_topology_base(
            &mut conn,
            base_volume_entry_count,
            base_volume_updated_at,
        );
        topology_rollback_ms.push(rollback_ms);
        let restored_facts = source_health_topology_facts(&conn, base_topology);
        assert_eq!(restored_facts, base_facts);
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "source_health_topology_rollback",
            "context": context,
            "topology_reverted": topology.name,
            "rollback_ms": round_ms(rollback_ms),
            "restored_base_state": restored_facts,
            "entry_count_restored": true,
            "volume_count_restored": true,
            "active_stale_distribution_restored": true,
            "rollback_verified_before_next_topology": true
        }));
        topologies_completed += 1;
    }

    assert_eq!(topologies_completed, SOURCE_HEALTH_TOPOLOGIES.len());
    assert_eq!(
        source_health_topology_facts(&conn, base_topology),
        base_facts
    );
    let schema_signature_after = schema_index_signature(&conn);
    assert_eq!(schema_signature_before, schema_signature_after);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "source_health_topology_diagnostic_complete",
        "context": context,
        "entries_after_all_variants": conn.query_row("SELECT COUNT(*) FROM global_entries", [], |row| row.get::<_, i64>(0)).expect("count restored entries"),
        "topologies_completed": topologies_completed,
        "topology_state_transition_ms": summarize_samples(&topology_transition_ms),
        "topology_rollback_ms": summarize_samples(&topology_rollback_ms),
        "one_fixture_build": true,
        "all_variants_transactionally_restored_before_next": true,
        "schema_and_index_signature_unchanged": true,
        "schema_index_signature_before": schema_signature_before,
        "schema_index_signature_after": schema_signature_after,
        "production_sql_changed": false,
        "persistent_schema_or_index_changed": false,
        "full_500k_search_matrix_repeated": false,
        "one_million_row_benchmark_run": false,
        "performance_gate_changed": false
    }));
}

#[test]
#[ignore = "single 500k-row Issue #352 query-stage diagnostic; one reusable Windows Hosted fixture"]
fn global_search_query_cost_diagnostic() {
    assert_eq!(
        env::var("ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES")
            .ok()
            .as_deref(),
        Some("500000"),
        "Issue #352 diagnostic is fixed at 500,000 rows; it must not become a 1m run"
    );
    let output_path = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT").ok();
    if let Some(path) = &output_path {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create query-cost evidence directory");
        }
        fs::write(path, "").expect("truncate query-cost JSONL evidence");
    }

    let context = benchmark_context(DIAGNOSTIC_ENTRIES);
    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open single 500k diagnostic SQLite fixture");
    db.upsert_global_volume(&super::super::test_volume())
        .expect("insert the base enabled synthetic volume");
    let population_started = Instant::now();
    let (expectations, generation_ms, population_ms, transaction_samples) =
        populate_synthetic_database(&db, DIAGNOSTIC_ENTRIES);
    let fixture_elapsed_ms = population_started.elapsed().as_secs_f64() * 1_000.0;
    let conn = configured_connection(&path).expect("open configured query-cost reader");

    let sqlite_version: String = conn
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .expect("read SQLite version");
    let pragmas = json!({
        "journal_mode": conn.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0)).expect("journal mode"),
        "synchronous": conn.query_row("PRAGMA synchronous", [], |row| row.get::<_, i64>(0)).expect("synchronous"),
        "foreign_keys": conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0)).expect("foreign keys"),
        "temp_store": conn.query_row("PRAGMA temp_store", [], |row| row.get::<_, i64>(0)).expect("temp store"),
        "mmap_size": conn.query_row("PRAGMA mmap_size", [], |row| row.get::<_, i64>(0)).expect("mmap size"),
        "page_size": conn.query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0)).expect("page size")
    });
    let actual_total: i64 = conn
        .query_row("SELECT COUNT(*) FROM global_entries", [], |row| row.get(0))
        .expect("count exact base fixture entries");
    assert_eq!(actual_total, DIAGNOSTIC_ENTRIES as i64);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "diagnostic_environment_and_dataset",
        "context": context,
        "runner": {
            "os": env::consts::OS,
            "arch": env::consts::ARCH,
            "image_os": env::var("ImageOS").ok(),
            "image_version": env::var("ImageVersion").ok(),
            "github_job": env::var("GITHUB_JOB").ok(),
            "github_run_attempt": env::var("GITHUB_RUN_ATTEMPT").ok()
        },
        "entries": actual_total,
        "fixture_type": "the existing global_search_benchmark.rs deterministic synthetic fixture with production schema, indices, FTS triggers and 512-row insert transactions",
        "sqlite_version": sqlite_version,
        "sqlite_pragmas": pragmas,
        "generation_ms": generation_ms,
        "population_ms": population_ms,
        "total_fixture_build_wall_ms": fixture_elapsed_ms,
        "population_transaction_ms": summarize_samples(&transaction_samples),
        "database": database_metrics(&conn, &path),
        "filesystem_or_OS_file_discovery_measured": false,
        "process_cpu_and_working_set_bytes": "NOT CAPTURED; VM steps and SQLite sort counters are reported instead"
    }));

    // The real command calls search_global_entries_snapshot(), which runs
    // these two repository reads after search. Profile their exact SQL shapes
    // separately so its extra latency is not attributed to candidate search.
    for (stage, sql) in [
        (
            "repository_snapshot_source_health_count_max",
            SNAPSHOT_SOURCE_HEALTH_SQL,
        ),
        (
            "repository_snapshot_index_status_aggregate",
            SNAPSHOT_INDEX_STATUS_SQL,
        ),
    ] {
        profile_sql_stage(
            &conn,
            &context,
            "repository_snapshot_facts",
            "search_global_files",
            stage,
            sql,
            Vec::new(),
        );
    }

    // Reuse this exact fixture for Issue #359: compare both exact source-health
    // candidates, their facts/hash work, and complete repository snapshots.
    profile_source_health_sql_candidates(&context, &conn, &path);
    profile_source_health_functions(&db, &path, &context);
    profile_repository_snapshot_candidates(&db, &path, &context);

    let mut correctness_classes = 0;
    for (query_case, expectation) in QUERY_MATRIX.iter().copied().zip(expectations.iter()) {
        let actual_count = actual_match_count(&conn, query_case);
        assert_eq!(
            actual_count as u64, expectation.total_matches,
            "500k count oracle for {}",
            query_case.class
        );
        assert_search_results(
            &conn,
            query_case,
            expectation,
            SEARCH_RESULT_LIMIT,
            0,
            DIAGNOSTIC_ENTRIES,
        );
        correctness_classes += 1;
    }
    assert_eq!(correctness_classes, QUERY_MATRIX.len());
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "source_health_diagnostic_500k_correctness",
        "context": context,
        "official_query_classes_checked": correctness_classes,
        "official_query_classes_passed": correctness_classes,
        "query_correctness": true,
        "performance_matrix_repeated": false,
        "performance_gate_changed": false
    }));

    for spec in PREFIX_QUERIES.iter().copied() {
        let expectation_index = QUERY_MATRIX
            .iter()
            .position(|item| item.class == spec.class)
            .expect("corresponding fixed baseline query class");
        let expected_count = expectations[expectation_index].total_matches as usize;
        let actual_count = actual_match_count(&conn, QUERY_MATRIX[expectation_index]);
        assert_eq!(
            actual_count as usize, expected_count,
            "fixture count for {}",
            spec.class
        );
        let tier = if spec.extension {
            "extension_prefix"
        } else {
            "name_prefix"
        };
        let actual_tier = diagnostic_search_tier(&conn, tier, spec.query, QUERY_LIMIT)
            .expect("probe actual routed production tier");
        assert_eq!(actual_tier.len(), expected_count.min(QUERY_LIMIT as usize));
        if !spec.extension {
            assert_eq!(
                run_result_sql(
                    &conn,
                    SAFE_PREFIX_CTE_SQL,
                    &prefix_variant_values(spec.query, QUERY_LIMIT),
                ),
                actual_tier,
                "safe prefix CTE equality for Unicode and punctuation behavior"
            );
        }

        let stage_records = prefix_stage_sql(spec);
        for (stage, sql, values) in &stage_records {
            let stage_count = profile_sql_stage(
                &conn,
                &context,
                spec.class,
                spec.query,
                stage,
                sql,
                values.clone(),
            );
            match *stage {
                "A_index_range_without_volume_join"
                | "B_active_volume_and_stale_filter"
                | "C_global_recency_sort_all_matches" => {
                    assert_eq!(stage_count, expected_count);
                }
                "D_global_recency_sort_limit_80"
                | "F_full_projection_without_managed_exists_limit_80"
                | "G_full_production_tier_projection_and_managed_exists_limit_80" => {
                    assert_eq!(stage_count, expected_count.min(QUERY_LIMIT as usize));
                }
                "E_global_recency_sort_limit_4096" => {
                    assert_eq!(stage_count, expected_count.min(SEARCH_CANDIDATE_LIMIT));
                }
                _ => unreachable!("unknown prefix stage: {stage}"),
            }
        }
        let (column, index, predicate, rank, glob_value) = if spec.extension {
            (
                "extension",
                "idx_global_entries_active_extension_order",
                "ge.extension <> lower(?1)",
                "1.0",
                format!("{}*", benchmark_escape_glob(spec.query)),
            )
        } else {
            (
                "name_normalized",
                "idx_global_entries_active_name_order",
                "ge.name_normalized <> lower(?1)",
                "0.0",
                format!("{}*", benchmark_escape_glob(spec.query)),
            )
        };
        let production_plan_sql = candidate_plan_sql(
            &format!("global_entries ge INDEXED BY {index} JOIN global_volumes gv ON gv.id = ge.volume_id"),
            &format!("gv.enabled = 1 AND ge.is_stale = 0 AND ge.{column} GLOB ?2 AND {predicate}"),
            "ge.modified_at_fs DESC, ge.id ASC",
            rank,
            "?3",
        );
        let production_plan_values = vec![
            SqlValue::Text(spec.query.to_string()),
            SqlValue::Text(glob_value),
            SqlValue::Integer(QUERY_LIMIT as i64),
        ];
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "production_tier_plan",
            "context": context,
            "query_class": spec.class,
            "query": spec.query,
            "production_tier": tier,
            "total_eligible_matches": expected_count,
            "plan_sql_mirrors_actual_search_rs_candidate_sql": production_plan_sql,
            "parameters": sql_values_json(&production_plan_values),
            "explain_query_plan": explain_details(&conn, &production_plan_sql, &production_plan_values),
            "actual_runtime_tier_helper": true
        }));

        let tier_results = measure_production_tier(
            &context,
            &conn,
            spec.class,
            spec.query,
            tier,
            QUERY_LIMIT,
            PAIRED_SAMPLES,
        );
        let aggregate =
            profile_full_search_entrypoint(&context, &db, &conn, &path, spec.class, spec.query);
        assert_eq!(
            aggregate, tier_results,
            "the expected tier should fill aggregate page"
        );
    }

    for (class, query) in FTS_QUERIES {
        for tier in [
            "exact_name",
            "name_prefix",
            "exact_extension",
            "extension_prefix",
        ] {
            assert!(diagnostic_search_tier(&conn, tier, query, QUERY_LIMIT)
                .expect("prove FTS query reaches only after the earlier tiers")
                .is_empty());
        }
        let fts_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM global_entries_fts WHERE global_entries_fts MATCH ?1",
                [fts_phrase(query)],
                |row| row.get(0),
            )
            .expect("count exact FTS match set");
        let values = fts_variant_values(query, QUERY_LIMIT);
        let limit4096 = fts_variant_values(query, SEARCH_CANDIDATE_LIMIT as u32);
        let stages: Vec<(&str, String, Vec<SqlValue>)> = vec![
            (
                "A_fts_match_rowids_only",
                "SELECT global_entries_fts.rowid FROM global_entries_fts WHERE global_entries_fts MATCH ?1".to_string(),
                vec![SqlValue::Text(fts_phrase(query))],
            ),
            (
                "B_fts_match_plus_bm25_no_order",
                "SELECT global_entries_fts.rowid, bm25(global_entries_fts, 8.0, 2.0, 1.0) FROM global_entries_fts WHERE global_entries_fts MATCH ?1".to_string(),
                vec![SqlValue::Text(fts_phrase(query))],
            ),
            (
                "C_fts_bm25_rank_sort_all_hits",
                "SELECT global_entries_fts.rowid FROM global_entries_fts WHERE global_entries_fts MATCH ?1 ORDER BY bm25(global_entries_fts, 8.0, 2.0, 1.0) ASC".to_string(),
                vec![SqlValue::Text(fts_phrase(query))],
            ),
            (
                "D_fts_rowid_join_global_entries",
                "SELECT ge.rowid FROM global_entries_fts CROSS JOIN global_entries ge WHERE global_entries_fts MATCH ?1 AND ge.rowid = global_entries_fts.rowid".to_string(),
                vec![SqlValue::Text(fts_phrase(query))],
            ),
            (
                "E_fts_enabled_volume_and_stale_filter",
                "SELECT ge.rowid FROM global_entries_fts CROSS JOIN global_entries ge CROSS JOIN global_volumes gv WHERE global_entries_fts MATCH ?1 AND ge.rowid = global_entries_fts.rowid AND gv.id = ge.volume_id AND gv.enabled = 1 AND ge.is_stale = 0".to_string(),
                vec![SqlValue::Text(fts_phrase(query))],
            ),
            (
                "F_fts_bm25_limit_80",
                "SELECT ge.rowid FROM global_entries_fts CROSS JOIN global_entries ge CROSS JOIN global_volumes gv WHERE global_entries_fts MATCH ?1 AND ge.rowid = global_entries_fts.rowid AND gv.id = ge.volume_id AND gv.enabled = 1 AND ge.is_stale = 0 ORDER BY bm25(global_entries_fts, 8.0, 2.0, 1.0) ASC, ge.modified_at_fs DESC, ge.id ASC LIMIT ?2".to_string(),
                values.clone(),
            ),
            (
                "G_fts_bm25_limit_4096",
                "SELECT ge.rowid FROM global_entries_fts CROSS JOIN global_entries ge CROSS JOIN global_volumes gv WHERE global_entries_fts MATCH ?1 AND ge.rowid = global_entries_fts.rowid AND gv.id = ge.volume_id AND gv.enabled = 1 AND ge.is_stale = 0 ORDER BY bm25(global_entries_fts, 8.0, 2.0, 1.0) ASC, ge.modified_at_fs DESC, ge.id ASC LIMIT ?2".to_string(),
                limit4096,
            ),
            (
                "H_fts_full_projection_without_managed_limit_80",
                FTS_WITHOUT_MANAGED_SQL.to_string(),
                values.clone(),
            ),
            (
                "I_fts_exact_production_projection_managed_rank_limit_80",
                diagnostic_search_fts_sql().replacen("SELECT ge.id,", "SELECT ge.rowid, ge.id,", 1),
                values,
            ),
        ];
        for (stage, sql, values) in stages {
            let returned = profile_sql_stage(&conn, &context, class, query, stage, &sql, values);
            match stage {
                "A_fts_match_rowids_only"
                | "B_fts_match_plus_bm25_no_order"
                | "C_fts_bm25_rank_sort_all_hits"
                | "D_fts_rowid_join_global_entries"
                | "E_fts_enabled_volume_and_stale_filter" => {
                    assert_eq!(
                        returned as i64, fts_count,
                        "unbounded FTS stage count for {query}"
                    );
                }
                "F_fts_bm25_limit_80"
                | "H_fts_full_projection_without_managed_limit_80"
                | "I_fts_exact_production_projection_managed_rank_limit_80" => {
                    assert_eq!(returned, (fts_count as usize).min(QUERY_LIMIT as usize));
                }
                "G_fts_bm25_limit_4096" => {
                    assert_eq!(returned, (fts_count as usize).min(SEARCH_CANDIDATE_LIMIT));
                }
                _ => unreachable!(),
            }
        }
        let fts_results = measure_production_tier(
            &context,
            &conn,
            class,
            query,
            "fts",
            QUERY_LIMIT,
            PAIRED_SAMPLES,
        );
        let full = profile_full_search_entrypoint(&context, &db, &conn, &path, class, query);
        assert_eq!(fts_results, full, "FTS tier should be the only match tier");
        assert_eq!(
            run_result_sql(
                &conn,
                SAFE_FTS_CTE_SQL,
                &fts_variant_values(query, QUERY_LIMIT),
            ),
            fts_results,
            "safe FTS CTE must equal the production tier for every measured hit count"
        );
        if fts_count as usize > SEARCH_CANDIDATE_LIMIT {
            let production_window =
                diagnostic_search_fts(&conn, query, SEARCH_CANDIDATE_LIMIT as u32)
                    .expect("collect production FTS candidate cap");
            let safe_window = run_result_sql(
                &conn,
                SAFE_FTS_CTE_SQL,
                &fts_variant_values(query, SEARCH_CANDIDATE_LIMIT as u32),
            );
            assert_eq!(
                safe_window, production_window,
                "FTS candidate-window equivalence"
            );
            let boundary_page = search_global_entries_on_connection(&conn, query, 80, 4_016)
                .expect("full search at FTS candidate-window boundary");
            assert_eq!(boundary_page, production_window[4_016..4_096]);
            assert!(search_global_entries_on_connection(&conn, query, 80, 4_096)
                .expect("full search after FTS candidate cap")
                .is_empty());
        }
        if query == "report" || query == "invoice" {
            let values = fts_variant_values(query, QUERY_LIMIT);
            profile_paired_variant(
                &context,
                &conn,
                class,
                query,
                CandidateVariant {
                    name: "key-only-materialized-fts-cte",
                    sql: SAFE_FTS_CTE_SQL,
                    values,
                },
                || {
                    diagnostic_search_fts(&conn, query, QUERY_LIMIT)
                        .expect("paired production FTS tier")
                },
            );
        }
    }

    // Measure the disposable time-first index only after all production-plan
    // baselines have been captured. It is never part of Schema migrations.
    let pages_before: i64 = conn
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .expect("read page count before test-only index");
    let page_size: i64 = conn
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .expect("read SQLite page size for test-only index");
    let main_bytes_before = fs::metadata(&path)
        .expect("read database size before test-only index")
        .len();
    let wal_bytes_before = sidecar_bytes(&path, "-wal");
    let fts_values = fts_variant_values("report", QUERY_LIMIT);
    let fts_plan_before = explain_details(&conn, &diagnostic_search_fts_sql(), &fts_values);
    let index_started = Instant::now();
    conn.execute_batch(&format!(
        "CREATE INDEX {TEST_INDEX_NAME} ON global_entries (modified_at_fs DESC, id ASC, name_normalized, volume_id) WHERE is_stale = 0"
    ))
    .expect("create disposable time-first name index");
    let index_build_ms = index_started.elapsed().as_secs_f64() * 1_000.0;
    let pages_after: i64 = conn
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .expect("read page count after test-only index");
    let main_bytes_after = fs::metadata(&path)
        .expect("read database size after test-only index")
        .len();
    let wal_bytes_after = sidecar_bytes(&path, "-wal");
    let time_values = prefix_variant_values("quarterly", QUERY_LIMIT);
    let production_prefix = || {
        diagnostic_search_tier(&conn, "name_prefix", "quarterly", QUERY_LIMIT)
            .expect("paired production prefix query")
    };
    profile_paired_variant(
        &context,
        &conn,
        "name_prefix",
        "quarterly",
        CandidateVariant {
            name: "key-only-materialized-prefix-cte",
            sql: SAFE_PREFIX_CTE_SQL,
            values: time_values.clone(),
        },
        production_prefix,
    );
    profile_paired_variant(
        &context,
        &conn,
        "name_prefix",
        "quarterly",
        CandidateVariant {
            name: "disposable-time-first-index",
            sql: TIME_INDEX_PREFIX_SQL,
            values: time_values.clone(),
        },
        production_prefix,
    );
    let index_plan = explain_details(&conn, TIME_INDEX_PREFIX_SQL, &time_values);
    let base_plan = explain_details(
        &conn,
        &candidate_plan_sql(
            "global_entries ge INDEXED BY idx_global_entries_active_name_order JOIN global_volumes gv ON gv.id = ge.volume_id",
            "gv.enabled = 1 AND ge.is_stale = 0 AND ge.name_normalized GLOB ?2 AND ge.name_normalized <> lower(?1)",
            "ge.modified_at_fs DESC, ge.id ASC",
            "0.0",
            "?3",
        ),
        &time_values,
    );
    let fts_plan_after = explain_details(&conn, &diagnostic_search_fts_sql(), &fts_values);
    let write_with_index = rollback_insert_samples(&conn, 5, 256);
    conn.execute_batch(&format!("DROP INDEX {TEST_INDEX_NAME}"))
        .expect("drop disposable time-first index before baseline write samples");
    let write_without_index = rollback_insert_samples(&conn, 5, 256);
    let with_index_p50 = summarize_samples(&write_with_index)["p50"]
        .as_f64()
        .unwrap_or_default();
    let without_index_p50 = summarize_samples(&write_without_index)["p50"]
        .as_f64()
        .unwrap_or_default();
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "ephemeral_index_cost",
        "context": context,
        "index_name": TEST_INDEX_NAME,
        "index_sql": format!("CREATE INDEX {TEST_INDEX_NAME} ON global_entries (modified_at_fs DESC, id ASC, name_normalized, volume_id) WHERE is_stale = 0"),
        "build_ms": index_build_ms,
        "page_count_before": pages_before,
        "page_count_after": pages_after,
        "page_count_delta": pages_after - pages_before,
        "estimated_index_bytes_from_page_delta": (pages_after - pages_before) * page_size,
        "database_main_bytes_before": main_bytes_before,
        "database_main_bytes_after": main_bytes_after,
        "database_wal_bytes_before": wal_bytes_before,
        "database_wal_bytes_after": wal_bytes_after,
        "database_main_plus_wal_bytes_delta": (main_bytes_after + wal_bytes_after).saturating_sub(main_bytes_before + wal_bytes_before),
        "index_plan": index_plan,
        "index_uses_temp_btree": index_plan.iter().any(|line| line.contains("USE TEMP B-TREE")),
        "production_prefix_plan_unchanged_by_explicit_index_hint": base_plan,
        "fts_plan_unchanged": fts_plan_before == fts_plan_after,
        "fts_plan_before": fts_plan_before,
        "fts_plan_after": fts_plan_after,
        "rollback_insert_write_sample_rows": 256,
        "rollback_insert_write_samples": 5,
        "write_with_index_latency_ms": summarize_samples(&write_with_index),
        "write_without_index_latency_ms": summarize_samples(&write_without_index),
        "write_p50_change_percent": if without_index_p50 > 0.0 { (with_index_p50 - without_index_p50) / without_index_p50 * 100.0 } else { 0.0 },
        "write_cost_scope": "small transaction-only diagnostic with production entry and FTS triggers; not a bulk population or full writer benchmark",
        "index_dropped_before_remaining_tests": true
    }));

    let (scale_overlay_ms, scale_results) = populate_scale_overlay(&conn);
    let count_before_rollback: i64 = conn
        .query_row("SELECT COUNT(*) FROM global_entries", [], |row| row.get(0))
        .expect("count rows before adversarial rollback");
    assert_eq!(count_before_rollback, DIAGNOSTIC_ENTRIES as i64);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "match_count_scale_summary",
        "context": context,
        "database_rows_during_queries": DIAGNOSTIC_ENTRIES,
        "synthetic_overlay_rows": 40_201,
        "overlay_insert_and_query_wall_ms": scale_overlay_ms,
        "overlay_transaction_rolled_back": true,
        "eligible_match_counts": scale_results,
        "result_page_limit": QUERY_LIMIT,
        "candidate_window": SEARCH_CANDIDATE_LIMIT
    }));

    let mut conn = conn;
    add_adversarial_rows(&db, &mut conn, &context);
    let final_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM global_entries", [], |row| row.get(0))
        .expect("verify only base entries remain after test overlays");
    assert_eq!(final_rows, DIAGNOSTIC_ENTRIES as i64);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "diagnostic_complete",
        "context": context,
        "base_rows_after_rollback": final_rows,
        "100ms_historical_gate_changed": false,
        "source_health_candidate_exercised": true,
        "search_tier_sql_changed": false,
        "schema_or_index_changed": false,
        "source_health_candidates_and_full_snapshots_measured": true,
        "official_500k_query_correctness_classes_checked": QUERY_MATRIX.len(),
        "official_500k_query_correctness_passed": true,
        "one_fixture_build": true,
        "one_million_row_benchmark_run": false,
        "official_full_benchmark_matrix_repeated": false,
        "normal_production_search_semantics_modified": false
    }));
}

#[derive(Default)]
struct ExactSql1mEvidenceState {
    bytes_written: u64,
    phase: String,
    terminal_status: Option<String>,
    peak_sampled_working_set_bytes: u64,
}

struct ExactSql1mEvidence {
    path: PathBuf,
    started: Instant,
    max_bytes: u64,
    state: Mutex<ExactSql1mEvidenceState>,
}

impl ExactSql1mEvidence {
    fn create(path: PathBuf) -> Self {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create exact SQL 1M evidence directory");
        }
        fs::write(&path, "").expect("initialize exact SQL 1M JSONL evidence");
        Self {
            path,
            started: Instant::now(),
            // Reserve 2 MiB of the combined 32 MiB budget for the existing
            // semantic helper, terminal marker and workflow hash metadata.
            max_bytes: EXACT_SQL_1M_EVIDENCE_LIMIT_BYTES - 2 * 1024 * 1024,
            state: Mutex::new(ExactSql1mEvidenceState::default()),
        }
    }

    fn set_phase(&self, phase: impl Into<String>) {
        self.lock_state().phase = phase.into();
    }

    fn record_working_set(&self, bytes: u64) {
        let mut state = self.lock_state();
        state.peak_sampled_working_set_bytes = state.peak_sampled_working_set_bytes.max(bytes);
    }

    fn peak_sampled_working_set_bytes(&self) -> u64 {
        self.lock_state().peak_sampled_working_set_bytes
    }

    fn bytes_written(&self) -> u64 {
        self.lock_state().bytes_written
    }

    fn emit(&self, record: &JsonValue) {
        let mut state = self.lock_state();
        assert!(
            state.terminal_status.is_none(),
            "cannot append evidence after terminal record"
        );
        self.append_locked(&mut state, record, false)
            .unwrap_or_else(|error| panic!("write bounded exact SQL 1M JSONL record: {error}"));
    }

    fn finish(&self, status: &str, reason: &str) -> Result<(), String> {
        let mut state = self.lock_state();
        if state.terminal_status.is_some() {
            return Ok(());
        }
        let terminal = json!({
            "schema_version": 1,
            "record_type": "diagnostic_terminal",
            "status": status,
            "reason": reason,
            "phase_at_termination": state.phase.clone(),
            "elapsed_ms": round_ms(self.started.elapsed().as_secs_f64() * 1_000.0),
            "evidence_bytes_before_terminal": state.bytes_written,
            "evidence_limit_bytes": EXACT_SQL_1M_EVIDENCE_LIMIT_BYTES,
            "peak_sampled_working_set_bytes": state.peak_sampled_working_set_bytes
        });
        self.append_locked(&mut state, &terminal, true)?;
        state.terminal_status = Some(status.to_string());
        Ok(())
    }

    fn is_terminal(&self) -> bool {
        self.lock_state().terminal_status.is_some()
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, ExactSql1mEvidenceState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn append_locked(
        &self,
        state: &mut ExactSql1mEvidenceState,
        record: &JsonValue,
        terminal: bool,
    ) -> Result<(), String> {
        let mut line = serde_json::to_vec(record).map_err(|error| error.to_string())?;
        line.push(b'\n');
        let next_size = state.bytes_written.saturating_add(line.len() as u64);
        let allowed = if terminal {
            EXACT_SQL_1M_EVIDENCE_LIMIT_BYTES
        } else {
            self.max_bytes
        };
        if next_size > allowed {
            return Err(format!(
                "JSONL evidence would exceed its {allowed}-byte write limit (current {}, next line {})",
                state.bytes_written,
                line.len()
            ));
        }

        let mut output = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| error.to_string())?;
        output
            .write_all(&line)
            .and_then(|()| output.flush())
            .map_err(|error| error.to_string())?;
        if terminal {
            output.sync_data().map_err(|error| error.to_string())?;
        }
        state.bytes_written = next_size;
        let line = String::from_utf8(line).map_err(|error| error.to_string())?;
        print!("{line}");
        Ok(())
    }
}

struct ExactSql1mCompletionGuard(Arc<ExactSql1mEvidence>);

impl ExactSql1mCompletionGuard {
    fn complete(&self) {
        self.0
            .finish(
                "PASS",
                "all 12 query, SQL, pooled, snapshot, pagination and resource checks completed",
            )
            .expect("append durable PASS terminal record");
    }
}

impl Drop for ExactSql1mCompletionGuard {
    fn drop(&mut self) {
        if !self.0.is_terminal() {
            let reason = if std::thread::panicking() {
                "test_panicked_or_assertion_failed"
            } else {
                "test_exited_before_all_checks_completed"
            };
            let _ = self.0.finish("INCOMPLETE", reason);
        }
    }
}

struct ExactSql1mWatchdog {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

fn checked_optional_file_bytes(path: &Path) -> Result<u64, String> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn checked_database_bytes(path: &Path) -> Result<u64, String> {
    let main_bytes = fs::metadata(path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .len();
    let mut wal_path = path.as_os_str().to_os_string();
    wal_path.push("-wal");
    let mut shm_path = path.as_os_str().to_os_string();
    shm_path.push("-shm");
    Ok(main_bytes
        .saturating_add(checked_optional_file_bytes(&PathBuf::from(wal_path))?)
        .saturating_add(checked_optional_file_bytes(&PathBuf::from(shm_path))?))
}

impl ExactSql1mWatchdog {
    fn start(
        evidence: Arc<ExactSql1mEvidence>,
        database_path: PathBuf,
        semantic_evidence_path: PathBuf,
        soft_timeout: StdDuration,
        max_database_bytes: u64,
        max_working_set_bytes: u64,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let worker = std::thread::Builder::new()
            .name("global-search-exact-sql-1m-budget".to_string())
            .spawn(move || {
                let started = Instant::now();
                loop {
                    if thread_stop.load(Ordering::Relaxed) || evidence.is_terminal() {
                        break;
                    }
                    if started.elapsed() >= soft_timeout {
                        let _ = evidence.finish(
                            "SOFT_TIMEOUT",
                            "the test-owned 30-minute elapsed budget expired; partial JSONL is retained",
                        );
                        // `process::exit` makes the job fail even if the test
                        // is inside a blocking SQLite operation. The terminal
                        // marker and each preceding line were flushed first.
                        std::process::exit(124);
                    }

                    match native_process_sample() {
                        Some(sample) => {
                            evidence.record_working_set(sample.working_set_bytes);
                            if sample.working_set_bytes > max_working_set_bytes {
                                let _ = evidence.finish(
                                    "RESOURCE_LIMIT",
                                    "sampled diagnostic-process working set exceeded the measured runner budget",
                                );
                                std::process::exit(125);
                            }
                        }
                        None => {
                            let _ = evidence.finish(
                                "RESOURCE_LIMIT",
                                "Windows process memory counters became unavailable during the run",
                            );
                            std::process::exit(126);
                        }
                    }

                    let database_bytes = match checked_database_bytes(&database_path) {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            let _ = evidence.finish(
                                "RESOURCE_LIMIT",
                                &format!("could not measure SQLite main/WAL/SHM bytes: {error}"),
                            );
                            std::process::exit(129);
                        }
                    };
                    if database_bytes > max_database_bytes {
                        let _ = evidence.finish(
                            "RESOURCE_LIMIT",
                            "SQLite main database and sidecars exceeded the measured disk budget",
                        );
                        std::process::exit(127);
                    }
                    let semantic_bytes = match fs::metadata(&semantic_evidence_path) {
                        Ok(metadata) => metadata.len(),
                        Err(error) => {
                            let _ = evidence.finish(
                                "RESOURCE_LIMIT",
                                &format!("could not measure semantic JSONL bytes: {error}"),
                            );
                            std::process::exit(130);
                        }
                    };
                    if semantic_bytes > 1024 * 1024 {
                        let _ = evidence.finish(
                            "RESOURCE_LIMIT",
                            "existing semantic helper exceeded its reserved 1 MiB JSONL budget",
                        );
                        std::process::exit(128);
                    }

                    std::thread::sleep(StdDuration::from_millis(250));
                }
            })
            .expect("start exact SQL 1M budget watchdog");
        Self {
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for ExactSql1mWatchdog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[test]
#[ignore = "Owner-approved test-only Issue #364 exact SQL 1M diagnostic; dedicated manual Hosted run requires a separate Owner GO"]
fn global_search_exact_sql_1m_diagnostic() {
    let output_path = PathBuf::from(
        env::var("ZC_GLOBAL_SEARCH_EXACT_SQL_DIAGNOSTIC_OUTPUT")
            .expect("dedicated exact SQL 1M workflow must set the JSONL path"),
    );
    let evidence = Arc::new(ExactSql1mEvidence::create(output_path.clone()));
    let completion = ExactSql1mCompletionGuard(Arc::clone(&evidence));

    assert_eq!(
        env::var("ZC_GLOBAL_SEARCH_EXACT_SQL_DIAGNOSTIC_ENTRIES")
            .ok()
            .as_deref(),
        Some("1000000"),
        "the independent diagnostic is fixed to its own one-million-row fixture"
    );
    assert_eq!(
        env::var("GITHUB_EVENT_NAME").ok().as_deref(),
        Some("workflow_dispatch"),
        "the exact SQL 1M diagnostic is manual-only"
    );
    assert_eq!(
        env::var("GITHUB_JOB").ok().as_deref(),
        Some("synthetic-global-search-exact-sql-diagnostic"),
        "the exact SQL 1M test must run only in its dedicated job"
    );
    assert_eq!(
        env::var("GITHUB_RUN_ATTEMPT").ok().as_deref(),
        Some("1"),
        "the hosted diagnostic is one attempt with no retry"
    );
    assert_eq!(
        env::var("ZC_GLOBAL_SEARCH_EXACT_SQL_DIAGNOSTIC_MODE")
            .ok()
            .as_deref(),
        Some("exact_sql_1m_diagnostic"),
        "the independent 1M job must select only its explicit manual mode"
    );
    let source_sha = required_exact_sql_1m_env("ZEN_CANVAS_BENCHMARK_SOURCE_SHA");
    let source_tree = required_exact_sql_1m_env("ZEN_CANVAS_BENCHMARK_SOURCE_TREE");
    assert!(
        is_git_object_id(&source_sha),
        "source SHA must be a full Git object ID"
    );
    assert!(
        is_git_object_id(&source_tree),
        "source Tree must be a full Git object ID"
    );

    let limits = exact_sql_1m_resource_limits();
    let semantic_output_path = PathBuf::from(required_exact_sql_1m_env(
        "ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT",
    ));
    if let Some(parent) = semantic_output_path.parent() {
        fs::create_dir_all(parent).expect("create semantic-helper evidence directory");
    }
    fs::write(&semantic_output_path, "").expect("initialize semantic-helper JSONL");
    let temp_root = PathBuf::from(required_exact_sql_1m_env(
        "ZC_GLOBAL_SEARCH_EXACT_SQL_DIAGNOSTIC_TEMP_ROOT",
    ));
    // Windows may spell one directory with its 8.3 alias (runner~1) or long name
    // (runneradmin). Compare resolved directories, not their textual spelling;
    // the workflow still measures and bounds the disk containing this directory.
    let rust_temp_root =
        fs::canonicalize(env::temp_dir()).expect("canonicalize Rust's actual temporary directory");
    let measured_temp_root =
        fs::canonicalize(&temp_root).expect("canonicalize the measured temporary directory");
    assert_eq!(
        rust_temp_root.to_string_lossy().to_ascii_lowercase(),
        measured_temp_root.to_string_lossy().to_ascii_lowercase(),
        "the measured disk budget must describe Rust's actual temporary directory"
    );

    let mut context = benchmark_context(EXACT_SQL_1M_ENTRIES);
    context["source_sha"] = json!(source_sha);
    context["source_tree"] = json!(source_tree);
    context["workflow_mode"] = json!("exact_sql_1m_diagnostic");
    context["workflow_job"] = json!(env::var("GITHUB_JOB").ok());
    context["run_id"] = json!(env::var("GITHUB_RUN_ID").ok());
    context["run_attempt"] = json!(env::var("GITHUB_RUN_ATTEMPT").ok());

    evidence.set_phase("preflight");
    evidence.emit(&json!({
        "schema_version": 1,
        "record_type": "exact_sql_1m_preflight",
        "context": context,
        "fixture_rows": EXACT_SQL_1M_ENTRIES,
        "soft_timeout_seconds": EXACT_SQL_1M_SOFT_TIMEOUT_SECONDS,
        "evidence_limit_bytes_combined": EXACT_SQL_1M_EVIDENCE_LIMIT_BYTES,
        "measured_limits": limits,
        "sql_provenance_values": ["CURRENT_PRODUCTION", "LEGACY", "TEST_CANDIDATE"],
        "legacy_or_test_candidate_sql_in_current_tier_measurements": false,
        "performance_gate_changed": false
    }));

    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open the sole exact SQL 1M fixture");
    db.upsert_global_volume(&test_volume())
        .expect("insert the one enabled synthetic fixture volume");
    let watchdog = ExactSql1mWatchdog::start(
        Arc::clone(&evidence),
        path.clone(),
        semantic_output_path.clone(),
        StdDuration::from_secs(EXACT_SQL_1M_SOFT_TIMEOUT_SECONDS),
        limits.max_database_bytes,
        limits.max_working_set_bytes,
    );

    evidence.set_phase("populate_one_million_rows");
    let fixture_started = Instant::now();
    let (expectations, generation_ms, population_ms, transaction_samples) =
        populate_synthetic_database(&db, EXACT_SQL_1M_ENTRIES);
    let fixture_wall_ms = fixture_started.elapsed().as_secs_f64() * 1_000.0;
    let mut conn =
        configured_connection(&path).expect("open configured exact SQL diagnostic reader");
    let actual_total: i64 = conn
        .query_row("SELECT COUNT(*) FROM global_entries", [], |row| row.get(0))
        .expect("count exact synthetic fixture rows");
    assert_eq!(actual_total, EXACT_SQL_1M_ENTRIES as i64);
    assert_eq!(expectations.len(), QUERY_MATRIX.len());
    let triggers = assert_entry_triggers_enabled(&conn);
    let schema_before = schema_index_signature(&conn);
    let sqlite_version: String = conn
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .expect("read bundled SQLite runtime version");
    let sqlite_source_id: String = conn
        .query_row("SELECT sqlite_source_id()", [], |row| row.get(0))
        .expect("read SQLite source identity");
    let compile_options = conn
        .prepare("PRAGMA compile_options")
        .expect("prepare SQLite compile option query")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("read SQLite compile options")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect SQLite compile options");
    evidence.emit(&json!({
        "schema_version": 1,
        "record_type": "exact_sql_1m_environment_and_fixture",
        "context": context,
        "fixture_rows": actual_total,
        "fixture_build_count": 1,
        "fixture_source": "existing global_search_benchmark.rs deterministic generator and QueryExpectation oracle, using production schema, indexes and FTS triggers",
        "fixture_generation_ms": round_ms(generation_ms),
        "fixture_population_ms": round_ms(population_ms),
        "fixture_wall_ms": round_ms(fixture_wall_ms),
        "population_transaction_latency_ms": summarize_samples(&transaction_samples),
        "runner": {
            "os": env::consts::OS,
            "arch": env::consts::ARCH,
            "image_os": env::var("ImageOS").ok(),
            "image_version": env::var("ImageVersion").ok(),
            "runner_os_description": env::var("RUNNER_OS").ok(),
            "github_job": env::var("GITHUB_JOB").ok(),
            "github_run_attempt": env::var("GITHUB_RUN_ATTEMPT").ok(),
            "rustc_version": env::var("ZEN_CANVAS_RUSTC_VERSION").ok(),
            "cargo_version": env::var("ZEN_CANVAS_CARGO_VERSION").ok(),
            "total_physical_memory_bytes_at_preflight": limits.total_physical_memory_bytes,
            "free_physical_memory_bytes_at_preflight": limits.free_physical_memory_bytes,
            "free_database_temp_disk_bytes_at_preflight": limits.free_disk_bytes,
            "free_evidence_temp_disk_bytes_at_preflight": limits.free_evidence_disk_bytes
        },
        "measured_resource_limits": limits,
        "process_memory_counters_available": native_process_sample().is_some(),
        "peak_sampled_working_set_bytes_so_far": evidence.peak_sampled_working_set_bytes(),
        "sqlite_version": sqlite_version,
        "sqlite_source_id": sqlite_source_id,
        "sqlite_compile_options": compile_options,
        "sqlite_pragmas": sqlite_runtime_settings(&conn),
        "database": database_metrics(&conn, &path),
        "database_file_budget_bytes": limits.max_database_bytes,
        "schema_index_trigger_signature_before": schema_before,
        "required_production_triggers": triggers,
        "existing_100k_500k_scale_guards_modified": false
    }));

    let source_health_oracle =
        load_global_search_source_health_candidate(&conn, SourceHealthCandidate::Original).expect(
            "read source-health and revision facts with independent original aggregate SQL",
        );
    let index_status_oracle = db
        .global_index_status()
        .expect("read index status oracle from repository");
    let mut query_correctness = 0;
    for (query_case, expectation) in QUERY_MATRIX.iter().copied().zip(expectations.iter()) {
        evidence.set_phase(format!("correctness:{}", query_case.class));
        let actual_count = actual_match_count(&conn, query_case);
        assert_eq!(
            actual_count as u64, expectation.total_matches,
            "1M independent result-count oracle for {}",
            query_case.class
        );
        assert_search_results(
            &conn,
            query_case,
            expectation,
            SEARCH_RESULT_LIMIT,
            0,
            EXACT_SQL_1M_ENTRIES,
        );

        let routed = trace_current_production_route(
            &evidence,
            &context,
            &conn,
            query_case,
            SEARCH_RESULT_LIMIT,
            0,
        );
        let direct =
            search_global_entries_on_connection(&conn, query_case.query, SEARCH_RESULT_LIMIT, 0)
                .expect("run the normal production search route on the same connection");
        assert_eq!(routed, direct, "tier replay must equal production route");
        assert_eq!(
            direct,
            assert_search_results(
                &conn,
                query_case,
                expectation,
                SEARCH_RESULT_LIMIT,
                0,
                EXACT_SQL_1M_ENTRIES,
            ),
            "full production result objects must preserve the fixture oracle"
        );

        let candidate_window = collect_current_production_candidate_window(
            &evidence,
            &context,
            &conn,
            query_case.class,
            query_case.query,
            SEARCH_CANDIDATE_LIMIT as u32,
        );
        let expected_window = expectation.expected_page(0, SEARCH_CANDIDATE_LIMIT);
        let candidate_ids = candidate_window
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(candidate_ids, expected_window, "full 4096 window oracle");
        assert_eq!(
            candidate_window.len(),
            expectation.bounded_match_count(),
            "candidate window must contain every eligible result up to 4096"
        );
        assert_eq!(
            candidate_ids.iter().copied().collect::<HashSet<_>>().len(),
            candidate_ids.len(),
            "candidate window de-duplicates stable entry IDs"
        );
        let boundary_page = search_global_entries_on_connection(
            &conn,
            query_case.query,
            SEARCH_RESULT_LIMIT,
            (SEARCH_CANDIDATE_LIMIT - SEARCH_RESULT_LIMIT as usize) as u32,
        )
        .expect("search immediately before 4096 candidate cap");
        let expected_boundary = expectation.expected_page(
            SEARCH_CANDIDATE_LIMIT - SEARCH_RESULT_LIMIT as usize,
            SEARCH_RESULT_LIMIT as usize,
        );
        assert_eq!(
            boundary_page
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            expected_boundary,
            "pagination immediately before 4096 cap"
        );
        assert!(
            search_global_entries_on_connection(
                &conn,
                query_case.query,
                SEARCH_RESULT_LIMIT,
                SEARCH_CANDIDATE_LIMIT as u32,
            )
            .expect("search at 4096 candidate cap")
            .is_empty(),
            "offset 4096 must return no results"
        );
        for (page_name, offset) in [("overlap_page", 40_u32), ("candidate_cap_page", 4_016_u32)] {
            let page = assert_search_results(
                &conn,
                query_case,
                expectation,
                SEARCH_RESULT_LIMIT,
                offset,
                EXACT_SQL_1M_ENTRIES,
            );
            assert_eq!(
                page.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
                expectation.expected_page(offset as usize, SEARCH_RESULT_LIMIT as usize),
                "page ID oracle for {page_name}"
            );
            evidence.emit(&json!({
                "schema_version": 1,
                "record_type": "exact_sql_1m_pagination_correctness",
                "context": context,
                "query_class": query_case.class,
                "query": query_case.query,
                "page": page_name,
                "limit": SEARCH_RESULT_LIMIT,
                "offset": offset,
                "result_count": page.len(),
                "oracle_order_dedup_filter_and_candidate_cap_equal": true
            }));
        }
        evidence.emit(&json!({
            "schema_version": 1,
            "record_type": "exact_sql_1m_candidate_window_correctness",
            "context": context,
            "query_class": query_case.class,
            "query": query_case.query,
            "eligible_matches": expectation.total_matches,
            "candidate_window_limit": SEARCH_CANDIDATE_LIMIT,
            "candidate_window_result_count": candidate_window.len(),
            "page_before_cap_count": boundary_page.len(),
            "offset_at_cap_returns_empty": true,
            "unique_ids": true,
            "fixture_oracle_order_equal": true
        }));

        profile_pooled_and_snapshot_1m(
            &evidence,
            &context,
            &db,
            &query_case,
            &direct,
            ExpectedSnapshotFacts1m {
                source_health: &source_health_oracle.source_health,
                source_revision: &source_health_oracle.source_revision,
                index_status: &index_status_oracle,
            },
        );
        query_correctness += 1;
    }
    assert_eq!(query_correctness, QUERY_MATRIX.len());

    evidence.set_phase("legacy_candidate_semantic_regression_checks");
    add_adversarial_rows(&db, &mut conn, &context);
    let semantic_lines = fs::read_to_string(&semantic_output_path)
        .expect("read existing bounded adversarial semantic helper output");
    assert!(!semantic_lines.trim().is_empty());
    assert_eq!(semantic_lines.lines().count(), 1);
    for line in semantic_lines.lines() {
        let mut record: JsonValue =
            serde_json::from_str(line).expect("parse existing semantic helper JSONL");
        relabel_semantic_helper_provenance(&mut record);
        evidence.emit(&record);
    }

    let final_total: i64 = conn
        .query_row("SELECT COUNT(*) FROM global_entries", [], |row| row.get(0))
        .expect("verify base entry count after rolled-back semantic overlay");
    assert_eq!(final_total, EXACT_SQL_1M_ENTRIES as i64);
    let schema_after = schema_index_signature(&conn);
    assert_eq!(
        schema_after, schema_before,
        "no schema/index/trigger changes"
    );
    let database_bytes = fs::metadata(&path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
        .saturating_add(sidecar_bytes(&path, "-wal"))
        .saturating_add(sidecar_bytes(&path, "-shm"));
    assert!(
        database_bytes <= limits.max_database_bytes,
        "SQLite database plus WAL/SHM exceeded the measured file budget"
    );
    let evidence_bytes = evidence.bytes_written();
    let semantic_bytes = fs::metadata(&semantic_output_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    assert!(
        semantic_bytes <= 1024 * 1024,
        "semantic helper evidence exceeded its reserved 1 MiB budget"
    );
    assert!(
        evidence_bytes.saturating_add(semantic_bytes) <= EXACT_SQL_1M_EVIDENCE_LIMIT_BYTES,
        "combined exact SQL evidence must remain at or below 32 MiB"
    );
    evidence.emit(&json!({
        "schema_version": 1,
        "record_type": "exact_sql_1m_correctness_summary",
        "context": context,
        "query_classes_checked": query_correctness,
        "query_classes_total": QUERY_MATRIX.len(),
        "query_oracles_all_passed": true,
        "full_pooled_results_equal_raw_route": true,
        "snapshot_full_results_source_health_revision_index_status_equal_oracles": true,
        "pagination_dedup_sort_filter_and_4096_window_checked": true,
        "schema_index_trigger_signature_unchanged": schema_after == schema_before,
        "base_rows_after_semantic_transaction_rollback": final_total,
        "database_main_wal_shm_bytes": database_bytes,
        "database_budget_bytes": limits.max_database_bytes,
        "evidence_bytes_before_summary": evidence_bytes,
        "semantic_helper_bytes": semantic_bytes,
        "evidence_budget_bytes_combined": EXACT_SQL_1M_EVIDENCE_LIMIT_BYTES,
        "peak_sampled_working_set_bytes": evidence.peak_sampled_working_set_bytes(),
        "working_set_budget_bytes": limits.max_working_set_bytes,
        "performance_threshold_changed": false,
        "production_sql_modified": false
    }));
    evidence.set_phase("complete");
    completion.complete();
    drop(watchdog);
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
struct ExactSql1mResourceLimits {
    total_physical_memory_bytes: u64,
    free_physical_memory_bytes: u64,
    free_disk_bytes: u64,
    free_evidence_disk_bytes: u64,
    max_working_set_bytes: u64,
    max_database_bytes: u64,
}

fn required_exact_sql_1m_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("dedicated exact SQL 1M workflow must set {name}"))
}

fn required_exact_sql_1m_env_u64(name: &str) -> u64 {
    required_exact_sql_1m_env(name)
        .parse::<u64>()
        .unwrap_or_else(|error| panic!("{name} must be a positive integer: {error}"))
}

fn is_git_object_id(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn exact_sql_1m_resource_limits() -> ExactSql1mResourceLimits {
    let limits = ExactSql1mResourceLimits {
        total_physical_memory_bytes: required_exact_sql_1m_env_u64(
            "ZC_GLOBAL_SEARCH_EXACT_SQL_DIAGNOSTIC_TOTAL_MEMORY_BYTES",
        ),
        free_physical_memory_bytes: required_exact_sql_1m_env_u64(
            "ZC_GLOBAL_SEARCH_EXACT_SQL_DIAGNOSTIC_FREE_MEMORY_BYTES",
        ),
        free_disk_bytes: required_exact_sql_1m_env_u64(
            "ZC_GLOBAL_SEARCH_EXACT_SQL_DIAGNOSTIC_FREE_DISK_BYTES",
        ),
        free_evidence_disk_bytes: required_exact_sql_1m_env_u64(
            "ZC_GLOBAL_SEARCH_EXACT_SQL_DIAGNOSTIC_FREE_EVIDENCE_DISK_BYTES",
        ),
        max_working_set_bytes: required_exact_sql_1m_env_u64(
            "ZC_GLOBAL_SEARCH_EXACT_SQL_DIAGNOSTIC_MAX_WORKING_SET_BYTES",
        ),
        max_database_bytes: required_exact_sql_1m_env_u64(
            "ZC_GLOBAL_SEARCH_EXACT_SQL_DIAGNOSTIC_MAX_DATABASE_BYTES",
        ),
    };
    assert!(limits.total_physical_memory_bytes > 0);
    assert!(limits.free_physical_memory_bytes > 0);
    assert!(limits.free_disk_bytes > 0);
    assert!(limits.free_evidence_disk_bytes > 0);
    assert!(limits.max_working_set_bytes > 0);
    assert!(limits.max_database_bytes > 0);
    assert!(
        limits.max_working_set_bytes <= limits.total_physical_memory_bytes,
        "measured working-set limit cannot exceed physical memory"
    );
    assert!(
        limits.max_database_bytes < limits.free_disk_bytes,
        "measured database budget must preserve free disk reserve"
    );
    assert!(
        native_process_sample().is_some(),
        "Windows process memory counters are required"
    );
    limits
}

fn exact_sql_1m_tier_order(query: &str) -> Vec<&'static str> {
    let mut tiers = vec![
        "exact_name",
        "name_prefix",
        "exact_extension",
        "extension_prefix",
    ];
    if query.chars().count() >= 3 {
        if query
            .chars()
            .all(|character| character.is_alphanumeric() || character.is_whitespace())
        {
            tiers.push("fts");
        } else if !query.is_empty() {
            tiers.push("punctuation_prefix");
        }
    }
    tiers
}

pub(super) fn exact_sql_1m_tier_parameters(tier: &str, query: &str, limit: u32) -> Vec<SqlValue> {
    match tier {
        "exact_name" | "exact_extension" => vec![
            SqlValue::Text(query.to_string()),
            SqlValue::Integer(limit as i64),
        ],
        "name_prefix" | "extension_prefix" => vec![
            SqlValue::Text(query.to_string()),
            SqlValue::Text(format!("{}*", benchmark_escape_glob(query))),
            SqlValue::Integer(limit as i64),
        ],
        "fts" => vec![
            SqlValue::Text(fts_phrase(query)),
            SqlValue::Integer(limit as i64),
        ],
        "punctuation_prefix" => vec![
            SqlValue::Text(format!("{}*", benchmark_escape_glob(query))),
            SqlValue::Integer(limit as i64),
        ],
        _ => panic!("unknown current search tier {tier}"),
    }
}

fn exact_sql_1m_layer_limit(target: u32, seen: usize) -> u32 {
    target
        .saturating_add(seen.min(SEARCH_CANDIDATE_LIMIT) as u32)
        .clamp(1, SEARCH_CANDIDATE_LIMIT as u32)
}

fn collect_current_production_candidate_window(
    evidence: &ExactSql1mEvidence,
    context: &JsonValue,
    conn: &Connection,
    query_class: &str,
    query: &str,
    target: u32,
) -> Vec<GlobalSearchResult> {
    let mut results = Vec::with_capacity(target as usize);
    let mut seen = HashSet::new();
    for (tier_index, tier) in exact_sql_1m_tier_order(query).iter().copied().enumerate() {
        let seen_before = seen.len();
        if results.len() >= target as usize {
            evidence.emit(&json!({
                "schema_version": 1,
                "record_type": "exact_sql_1m_candidate_window_layer",
                "context": context,
                "query_class": query_class,
                "query": query,
                "tier": tier,
                "sql_provenance": "CURRENT_PRODUCTION",
                "route_status": "SKIPPED_TARGET_FILLED",
                "target": target,
                "effective_target": target,
                "seen_before": seen_before,
                "layer_limit": null,
                "limit": null,
                "sql_limit": null,
                "sql": null,
                "bound_parameters": null
            }));
            continue;
        }
        let layer_limit = exact_sql_1m_layer_limit(target, seen_before);
        let sql = diagnostic_search_tier_sql(tier);
        let parameters = exact_sql_1m_tier_parameters(tier, query, layer_limit);
        let explain_query_plan = explain_details(conn, &sql, &parameters);
        let (rows, elapsed_ms, counters, _) =
            execute_search_sql_with_counters(conn, &sql, &parameters);
        let runtime_tier = diagnostic_search_tier(conn, tier, query, layer_limit)
            .unwrap_or_else(|error| panic!("run 4096-window {tier} production tier: {error}"));
        assert_eq!(rows, runtime_tier);
        let appended_unique =
            append_exact_sql_1m_unique(&mut results, &mut seen, rows.clone(), target);
        let route_status = if results.len() >= target as usize {
            "EXECUTED_TARGET_FILLED"
        } else if rows.is_empty() {
            "EXECUTED_NO_MATCH"
        } else {
            "EXECUTED_MORE_LAYERS_REQUIRED"
        };
        let uses_temp_btree = explain_query_plan
            .iter()
            .any(|line| line.contains("USE TEMP B-TREE"));
        evidence.emit(&json!({
            "schema_version": 1,
            "record_type": "exact_sql_1m_candidate_window_layer",
            "context": context,
            "query_class": query_class,
            "query": query,
            "tier": tier,
            "tier_order_index": tier_index,
            "sql_provenance": "CURRENT_PRODUCTION",
            "route_status": route_status,
            "target": target,
            "effective_target": target,
            "seen_before": seen_before,
            "seen_after": seen.len(),
            "layer_limit": layer_limit,
            "limit": layer_limit,
            "sql_limit": layer_limit,
            "sql": sql,
            "bound_parameters": sql_values_json(&parameters),
            "route_probe_elapsed_ms": round_ms(elapsed_ms),
            "sqlite_statement_counters_last_execution": counters,
            "explain_query_plan": explain_query_plan,
            "uses_temp_btree": uses_temp_btree,
            "sql_rows": rows.len(),
            "appended_unique_rows": appended_unique,
            "full_result_objects_equal_runtime_tier_helper": true,
            "sql_provenance_is_current_production": true
        }));
    }
    evidence.emit(&json!({
        "schema_version": 1,
        "record_type": "exact_sql_1m_candidate_window_route",
        "context": context,
        "query_class": query_class,
        "query": query,
        "sql_provenance": "CURRENT_PRODUCTION",
        "candidate_window_limit": target,
        "candidate_window_result_count": results.len(),
        "tiers_in_production_order": exact_sql_1m_tier_order(query),
        "unique_ids": seen.len() == results.len(),
        "full_result_objects_collected_in_production_order": true
    }));
    results
}

fn append_exact_sql_1m_unique(
    results: &mut Vec<GlobalSearchResult>,
    seen: &mut HashSet<String>,
    candidates: Vec<GlobalSearchResult>,
    target: u32,
) -> usize {
    let before = results.len();
    for candidate in candidates {
        if seen.insert(candidate.id.clone()) {
            results.push(candidate);
            if results.len() >= target as usize {
                break;
            }
        }
    }
    results.len() - before
}

fn trace_current_production_route(
    evidence: &ExactSql1mEvidence,
    context: &JsonValue,
    conn: &Connection,
    query_case: QueryCase,
    limit: u32,
    offset: u32,
) -> Vec<GlobalSearchResult> {
    let query = query_case.query.trim();
    let effective_limit = limit.clamp(1, 200);
    let effective_offset = offset.min(1_000_000);
    if query.is_empty() || effective_offset >= SEARCH_CANDIDATE_LIMIT as u32 {
        return Vec::new();
    }
    let target = effective_offset
        .saturating_add(effective_limit)
        .min(SEARCH_CANDIDATE_LIMIT as u32);
    let tiers = exact_sql_1m_tier_order(query);
    let mut results = Vec::with_capacity(target as usize);
    let mut seen = HashSet::new();
    let mut executed = 0_usize;

    for (tier_index, tier) in tiers.iter().copied().enumerate() {
        let seen_before = seen.len();
        if results.len() >= target as usize {
            evidence.emit(&json!({
                "schema_version": 1,
                "record_type": "exact_sql_1m_route_layer",
                "context": context,
                "query_class": query_case.class,
                "query": query_case.query,
                "tier": tier,
                "sql_provenance": "CURRENT_PRODUCTION",
                "route_status": "SKIPPED_TARGET_FILLED",
                "target": target,
                "effective_target": target,
                "seen_before": seen_before,
                "layer_limit": null,
                "limit": null,
                "sql_limit": null,
                "sql": null,
                "bound_parameters": null,
                "plan_and_counters": "NOT_EXECUTED: production route stops after target is filled"
            }));
            continue;
        }

        let layer_limit = exact_sql_1m_layer_limit(target, seen_before);
        let sql = diagnostic_search_tier_sql(tier);
        if tier == "fts" {
            assert_eq!(sql, diagnostic_search_fts_sql());
        }
        let parameters = exact_sql_1m_tier_parameters(tier, query, layer_limit);
        let explain_query_plan = explain_details(conn, &sql, &parameters);
        let (rows, route_probe_ms, route_probe_counters, _) =
            execute_search_sql_with_counters(conn, &sql, &parameters);
        let runtime_tier = diagnostic_search_tier(conn, tier, query, layer_limit)
            .unwrap_or_else(|error| panic!("run actual {tier} production tier: {error}"));
        assert_eq!(
            rows, runtime_tier,
            "test-fetched current SQL must equal the production tier helper for {tier}"
        );

        for _ in 0..EXACT_SQL_1M_WARMUPS {
            let (warm, _, _, _) = execute_search_sql_with_counters(conn, &sql, &parameters);
            assert_eq!(warm, runtime_tier, "current {tier} SQL warmup equality");
        }
        let mut samples = Vec::with_capacity(EXACT_SQL_1M_SAMPLES);
        let mut counters_last_sample = JsonValue::Null;
        let mut resources = ProcessResourceAccumulator::default();
        for _ in 0..EXACT_SQL_1M_SAMPLES {
            let (sample_rows, elapsed_ms, counters, observation) =
                execute_search_sql_with_counters(conn, &sql, &parameters);
            assert_eq!(
                sample_rows, runtime_tier,
                "current {tier} SQL full result object equality"
            );
            samples.push(elapsed_ms);
            counters_last_sample = counters;
            resources.observe(observation);
        }

        let appended_unique =
            append_exact_sql_1m_unique(&mut results, &mut seen, rows.clone(), target);
        let uses_temp_btree = explain_query_plan
            .iter()
            .any(|line| line.contains("USE TEMP B-TREE"));
        let possible_full_table_scan = explain_query_plan
            .iter()
            .any(|line| line.contains("SCAN ge") || line.contains("SCAN global_entries "));
        let route_status = if rows.is_empty() {
            "EXECUTED_NO_MATCH"
        } else if results.len() >= target as usize {
            "EXECUTED_TARGET_FILLED"
        } else {
            "EXECUTED_MORE_LAYERS_REQUIRED"
        };
        evidence.emit(&json!({
            "schema_version": 1,
            "record_type": "exact_sql_1m_route_layer",
            "context": context,
            "query_class": query_case.class,
            "query": query_case.query,
            "tier": tier,
            "tier_order_index": tier_index,
            "sql_provenance": "CURRENT_PRODUCTION",
            "route_status": route_status,
            "target": target,
            "effective_target": target,
            "seen_before": seen_before,
            "seen_after": seen.len(),
            "layer_limit": layer_limit,
            "limit": layer_limit,
            "sql_limit": layer_limit,
            "bound_parameters": sql_values_json(&parameters),
            "sql": sql,
            "route_probe_elapsed_ms": round_ms(route_probe_ms),
            "route_probe_sqlite_counters": route_probe_counters,
            "latency_ms": summarize_samples(&samples),
            "warmups": EXACT_SQL_1M_WARMUPS,
            "samples": EXACT_SQL_1M_SAMPLES,
            "sqlite_statement_counters_last_sample": counters_last_sample,
            "process_resource_samples": resources.json(samples.iter().sum()),
            "explain_query_plan": explain_query_plan,
            "uses_temp_btree": uses_temp_btree,
            "possible_full_table_scan": possible_full_table_scan,
            "raw_sql_timing_scope": "current tier statement prepare, full ordered result drain and GlobalSearchResult mapping; no pool checkout, remaining tiers or snapshot facts",
            "sql_accessor_matches_runtime_production_tier": true,
            "legacy_or_test_candidate_sql_used_for_this_record": false,
            "sql_rows": rows.len(),
            "appended_unique_rows": appended_unique,
            "seen_ids_are_deduplicated": true
        }));
        executed += 1;
    }

    evidence.emit(&json!({
        "schema_version": 1,
        "record_type": "exact_sql_1m_route_summary",
        "context": context,
        "query_class": query_case.class,
        "query": query_case.query,
        "sql_provenance": "CURRENT_PRODUCTION",
        "target": target,
        "executed_tier_count": executed,
        "tiers_in_production_order": tiers,
        "routed_result_count": results.len(),
        "result_id_prefix": results.iter().take(8).map(|row| row.id.clone()).collect::<Vec<_>>(),
        "full_result_objects_replayed_in_production_order": true,
        "legacy_or_test_candidate_sql_in_current_route": false
    }));
    results
        .into_iter()
        .skip(effective_offset as usize)
        .take(effective_limit as usize)
        .collect()
}

struct ExpectedSnapshotFacts1m<'a> {
    source_health: &'a [crate::global_index::models::GlobalSearchSourceHealth],
    source_revision: &'a str,
    index_status: &'a crate::global_index::models::GlobalIndexStatus,
}

fn profile_pooled_and_snapshot_1m(
    evidence: &ExactSql1mEvidence,
    context: &JsonValue,
    db: &Database,
    query_case: &QueryCase,
    expected_results: &[GlobalSearchResult],
    expected_snapshot: ExpectedSnapshotFacts1m<'_>,
) {
    let ExpectedSnapshotFacts1m {
        source_health: expected_source_health,
        source_revision: expected_source_revision,
        index_status: expected_index_status,
    } = expected_snapshot;
    for _ in 0..EXACT_SQL_1M_WARMUPS {
        let results = db
            .search_global_entries(query_case.query, SEARCH_RESULT_LIMIT, 0)
            .expect("warm actual pooled production search");
        assert_eq!(results.as_slice(), expected_results);
    }
    let mut pooled_samples = Vec::with_capacity(EXACT_SQL_1M_SAMPLES);
    let mut pooled_resources = ProcessResourceAccumulator::default();
    for _ in 0..EXACT_SQL_1M_SAMPLES {
        let (results, elapsed_ms, resources) = measure_with_process_counters(|| {
            db.search_global_entries(query_case.query, SEARCH_RESULT_LIMIT, 0)
        });
        let results = results.expect("measure actual pooled production search");
        assert_eq!(
            results.as_slice(),
            expected_results,
            "pooled full result objects/oracle"
        );
        pooled_samples.push(elapsed_ms);
        pooled_resources.observe(resources);
    }
    evidence.emit(&json!({
        "schema_version": 1,
        "record_type": "exact_sql_1m_pooled_search",
        "context": context,
        "query_class": query_case.class,
        "query": query_case.query,
        "sql_provenance": "CURRENT_PRODUCTION",
        "limit": SEARCH_RESULT_LIMIT,
        "offset": 0,
        "result_count": expected_results.len(),
        "latency_ms": summarize_samples(&pooled_samples),
        "warmups": EXACT_SQL_1M_WARMUPS,
        "samples": EXACT_SQL_1M_SAMPLES,
        "process_resource_samples": pooled_resources.json(pooled_samples.iter().sum()),
        "entire_ordered_result_objects_equal_route_and_oracle": true,
        "timing_scope": "actual Database::search_global_entries pool checkout, production tier route, de-duplication and page extraction; excludes repository snapshot facts"
    }));

    let snapshot_oracle = db
        .search_global_entries_snapshot(query_case.query, SEARCH_RESULT_LIMIT, 0)
        .expect("build actual full repository snapshot oracle");
    assert_eq!(snapshot_oracle.results.as_slice(), expected_results);
    assert_eq!(
        snapshot_oracle.source_health.as_slice(),
        expected_source_health
    );
    assert_eq!(
        snapshot_oracle.source_revision.as_str(),
        expected_source_revision
    );
    assert_eq!(&snapshot_oracle.index_status, expected_index_status);
    for _ in 0..EXACT_SQL_1M_WARMUPS {
        let snapshot = db
            .search_global_entries_snapshot(query_case.query, SEARCH_RESULT_LIMIT, 0)
            .expect("warm actual Repository Snapshot");
        assert_snapshot_results_equal(&snapshot_oracle, &snapshot);
    }
    let mut snapshot_samples = Vec::with_capacity(EXACT_SQL_1M_SAMPLES);
    let mut snapshot_resources = ProcessResourceAccumulator::default();
    for _ in 0..EXACT_SQL_1M_SAMPLES {
        let (snapshot, elapsed_ms, resources) = measure_with_process_counters(|| {
            db.search_global_entries_snapshot(query_case.query, SEARCH_RESULT_LIMIT, 0)
        });
        let snapshot = snapshot.expect("measure actual Repository Snapshot");
        assert_snapshot_results_equal(&snapshot_oracle, &snapshot);
        assert_eq!(snapshot.results.as_slice(), expected_results);
        assert_eq!(snapshot.source_health.as_slice(), expected_source_health);
        assert_eq!(snapshot.source_revision.as_str(), expected_source_revision);
        assert_eq!(&snapshot.index_status, expected_index_status);
        snapshot_samples.push(elapsed_ms);
        snapshot_resources.observe(resources);
    }
    evidence.emit(&json!({
        "schema_version": 1,
        "record_type": "exact_sql_1m_repository_snapshot",
        "context": context,
        "query_class": query_case.class,
        "query": query_case.query,
        "sql_provenance": "CURRENT_PRODUCTION",
        "limit": SEARCH_RESULT_LIMIT,
        "offset": 0,
        "result_count": snapshot_oracle.results.len(),
        "latency_ms": summarize_samples(&snapshot_samples),
        "warmups": EXACT_SQL_1M_WARMUPS,
        "samples": EXACT_SQL_1M_SAMPLES,
        "source_health": snapshot_oracle.source_health,
        "source_revision": snapshot_oracle.source_revision,
        "index_status": snapshot_oracle.index_status,
        "source_health_equals_independent_original_aggregate": true,
        "source_revision_equals_independent_original_aggregate_hash": true,
        "index_status_equals_repository_index_status_oracle": true,
        "all_result_objects_equal_route_and_oracle": true,
        "process_resource_samples": snapshot_resources.json(snapshot_samples.iter().sum()),
        "sqlite_statement_counters": "not exposed by the Repository Snapshot API; current exact tier counters are emitted in route-layer records",
        "timing_scope": "actual Repository Snapshot pool checkout and same read transaction for production search, Source Health, Source Revision and Index Status"
    }));
}

fn relabel_semantic_helper_provenance(record: &mut JsonValue) {
    let object = record
        .as_object_mut()
        .expect("existing semantic helper output is a JSON object");
    object.insert(
        "record_type".to_string(),
        json!("exact_sql_1m_adversarial_semantic_equivalence"),
    );
    object.insert(
        "sql_provenance".to_string(),
        json!({
            "legacy_baseline": "LEGACY",
            "safe_cte_sql_mirror": "TEST_CANDIDATE",
            "runtime_override": "CURRENT_PRODUCTION",
            "note": "the helper forces Legacy SQL for its baseline, compares a test-only SQL mirror, then checks the runtime override against the current production tier; only the runtime override is CURRENT_PRODUCTION"
        }),
    );
    object.insert(
        "legacy_baseline_forced_by_test_override".to_string(),
        json!(true),
    );
    object.insert("test_candidate_sql_executed".to_string(), json!(true));
    object.insert(
        "current_runtime_override_sql_executed".to_string(),
        json!(true),
    );
    for (old, new) in [
        (
            "production_vs_safe_prefix_cte_fields_equal",
            "legacy_vs_test_candidate_name_prefix_cte_full_result_objects_equal",
        ),
        (
            "production_vs_safe_fts_cte_fields_equal",
            "legacy_vs_test_candidate_fts_cte_full_result_objects_equal",
        ),
        ("production_fts_plan", "current_production_fts_plan"),
    ] {
        if let Some(value) = object.remove(old) {
            object.insert(new.to_string(), value);
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ProcessObservation {
    cpu_time_ms: f64,
    working_set_bytes: u64,
    private_bytes: u64,
}

#[derive(Debug, Default)]
struct ProcessResourceAccumulator {
    cpu_time_ms: f64,
    peak_sampled_working_set_bytes: u64,
    peak_sampled_private_bytes: u64,
    observed_calls: usize,
}

impl ProcessResourceAccumulator {
    fn observe(&mut self, observation: Option<ProcessObservation>) {
        if let Some(observation) = observation {
            self.cpu_time_ms += observation.cpu_time_ms;
            self.peak_sampled_working_set_bytes = self
                .peak_sampled_working_set_bytes
                .max(observation.working_set_bytes);
            self.peak_sampled_private_bytes = self
                .peak_sampled_private_bytes
                .max(observation.private_bytes);
            self.observed_calls += 1;
        }
    }

    fn json(&self, wall_time_ms: f64) -> JsonValue {
        if self.observed_calls == 0 {
            return json!({
                "status": "NOT_VERIFIED_ON_THIS_RUNNER",
                "cpu_time_ms": null,
                "cpu_percent_of_one_logical_core": null,
                "peak_sampled_working_set_bytes": null,
                "peak_sampled_private_bytes": null,
                "sampled_calls": 0
            });
        }
        json!({
            "status": "WINDOWS_PROCESS_COUNTERS_MEASURED",
            "process_cpu_time_ms": round_ms(self.cpu_time_ms),
            "process_wall_time_ms": round_ms(wall_time_ms),
            "cpu_percent_of_one_logical_core": if wall_time_ms > 0.0 { round_ms(self.cpu_time_ms / wall_time_ms * 100.0) } else { 0.0 },
            "peak_sampled_working_set_bytes": self.peak_sampled_working_set_bytes,
            "peak_sampled_private_bytes": self.peak_sampled_private_bytes,
            "sampled_calls": self.observed_calls,
            "sampling_scope": "Windows process counters read immediately before and after each measured request; these are process-wide boundary samples, not an OS peak or SQL-attributed memory measurement"
        })
    }
}

fn process_observation(
    before: Option<NativeProcessSample>,
    after: Option<NativeProcessSample>,
) -> Option<ProcessObservation> {
    let (before, after) = (before?, after?);
    Some(ProcessObservation {
        cpu_time_ms: after.cpu_time_100ns.saturating_sub(before.cpu_time_100ns) as f64 / 10_000.0,
        working_set_bytes: before.working_set_bytes.max(after.working_set_bytes),
        private_bytes: before.private_bytes.max(after.private_bytes),
    })
}

fn measure_with_process_counters<T>(
    operation: impl FnOnce() -> T,
) -> (T, f64, Option<ProcessObservation>) {
    let before = native_process_sample();
    let started = Instant::now();
    let value = operation();
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let after = native_process_sample();
    (value, elapsed_ms, process_observation(before, after))
}

fn key_only_values(tier: &str, query: &str, limit: u32) -> Vec<SqlValue> {
    match tier {
        "fts" => fts_variant_values(query, limit),
        "punctuation_prefix" => vec![
            SqlValue::Text(format!("{}*", benchmark_escape_glob(query))),
            SqlValue::Integer(limit as i64),
        ],
        "name_prefix" | "extension_prefix" => prefix_variant_values(query, limit),
        _ => panic!("unsupported key-only CTE tier: {tier}"),
    }
}

fn execute_search_sql_with_counters(
    conn: &Connection,
    sql: &str,
    values: &[SqlValue],
) -> (
    Vec<GlobalSearchResult>,
    f64,
    JsonValue,
    Option<ProcessObservation>,
) {
    let before = native_process_sample();
    let started = Instant::now();
    let mut statement = conn.prepare(sql).expect("prepare measured search SQL");
    for status in [
        StatementStatus::Sort,
        StatementStatus::FullscanStep,
        StatementStatus::VmStep,
    ] {
        statement.reset_status(status);
    }
    let results = statement
        .query_map(params_from_iter(values.iter()), map_search_result)
        .expect("execute measured search SQL")
        .collect::<Result<Vec<_>, _>>()
        .expect("drain measured search SQL");
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let counters = json!({
        "sort_operations": statement.get_status(StatementStatus::Sort),
        "fullscan_steps": statement.get_status(StatementStatus::FullscanStep),
        "vm_steps": statement.get_status(StatementStatus::VmStep),
        "scope": "last execution; VM steps and sort operations are work indicators, not CPU time or temporary bytes"
    });
    let after = native_process_sample();
    (
        results,
        elapsed_ms,
        counters,
        process_observation(before, after),
    )
}

fn with_legacy_search_tier<T>(tier: &str, operation: impl FnOnce() -> T) -> T {
    match tier {
        "name_prefix" => with_legacy_name_prefix_search_candidate(operation),
        "fts" => with_legacy_fts_search_candidate(operation),
        _ => operation(),
    }
}

fn with_key_only_search_tier<T>(tier: &str, operation: impl FnOnce() -> T) -> T {
    match tier {
        "fts" => with_key_only_fts_search_candidate(operation),
        _ => with_key_only_cte_search_candidate(operation),
    }
}

struct KeyOnlySqlPair<'a> {
    query_class: &'a str,
    tier: &'a str,
    query: &'a str,
    hit_count: usize,
    limit: u32,
    samples: usize,
}

fn profile_key_only_sql_pair(
    context: &JsonValue,
    conn: &Connection,
    case: KeyOnlySqlPair<'_>,
) -> JsonValue {
    let production_sql = match case.tier {
        "name_prefix" => diagnostic_legacy_name_prefix_sql(),
        "fts" => diagnostic_legacy_fts_sql(),
        _ => diagnostic_search_tier_sql(case.tier),
    };
    let candidate_sql = diagnostic_key_only_cte_sql(case.tier);
    let values = key_only_values(case.tier, case.query, case.limit);
    let expected = with_legacy_search_tier(case.tier, || {
        diagnostic_search_tier(conn, case.tier, case.query, case.limit)
    })
    .unwrap_or_else(|error| {
        panic!(
            "production {} query failed for {:?}: {error}",
            case.tier, case.query
        )
    });
    let candidate = run_result_sql(conn, &candidate_sql, &values);
    assert_eq!(
        candidate, expected,
        "key-only CTE must match the entire production result object for {}",
        case.query_class
    );
    let helper_candidate = with_key_only_search_tier(case.tier, || {
        diagnostic_search_tier(conn, case.tier, case.query, case.limit)
    })
    .unwrap_or_else(|error| {
        panic!(
            "key-only production helper failed for {:?}: {error}",
            case.query
        )
    });
    assert_eq!(helper_candidate, candidate);

    for _ in 0..PAIRED_WARMUPS {
        let (original, _, _, _) = execute_search_sql_with_counters(conn, &production_sql, &values);
        let (candidate, _, _, _) = execute_search_sql_with_counters(conn, &candidate_sql, &values);
        assert_eq!(original, expected);
        assert_eq!(candidate, expected);
    }

    let mut production_samples = Vec::with_capacity(case.samples);
    let mut candidate_samples = Vec::with_capacity(case.samples);
    let mut production_resources = ProcessResourceAccumulator::default();
    let mut candidate_resources = ProcessResourceAccumulator::default();
    let mut production_counters = JsonValue::Null;
    let mut candidate_counters = JsonValue::Null;
    for sample in 0..case.samples {
        let mut record_original = || {
            let (results, elapsed, counters, resources) =
                execute_search_sql_with_counters(conn, &production_sql, &values);
            assert_eq!(results, expected);
            production_samples.push(elapsed);
            production_resources.observe(resources);
            production_counters = counters;
        };
        let mut record_candidate = || {
            let (results, elapsed, counters, resources) =
                execute_search_sql_with_counters(conn, &candidate_sql, &values);
            assert_eq!(results, expected);
            candidate_samples.push(elapsed);
            candidate_resources.observe(resources);
            candidate_counters = counters;
        };
        if sample % 2 == 0 {
            record_original();
            record_candidate();
        } else {
            record_candidate();
            record_original();
        }
    }

    let production_summary = summarize_samples(&production_samples);
    let candidate_summary = summarize_samples(&candidate_samples);
    let production_p95 = production_summary["p95"].as_f64().unwrap_or_default();
    let candidate_p95 = candidate_summary["p95"].as_f64().unwrap_or_default();
    let candidate_plan = explain_details(conn, &candidate_sql, &values);
    let production_plan = explain_details(conn, &production_sql, &values);
    if case.tier == "fts" {
        assert!(candidate_sql.contains(
            "global_entries_fts CROSS JOIN global_entries ge CROSS JOIN global_volumes gv"
        ));
        assert!(candidate_sql.contains("bm25(global_entries_fts, 8.0, 2.0, 1.0)"));
        assert!(candidate_plan.iter().any(|line| {
            line.contains("global_entries_fts") && line.contains("VIRTUAL TABLE INDEX")
        }));
    }
    let record = json!({
        "schema_version": 1,
        "record_type": "key_only_cte_sql_paired",
        "context": context,
        "query_class": case.query_class,
        "tier": case.tier,
        "query": case.query,
        "eligible_hit_count": case.hit_count,
        "limit": case.limit,
        "result_count": expected.len(),
        "samples_per_variant": case.samples,
        "warmups_per_variant": PAIRED_WARMUPS,
        "paired_order": "same runner, SQLite fixture, connection and query values; Original-first and Candidate-first alternate by sample",
        "original_latency_ms": production_summary,
        "key_only_cte_latency_ms": candidate_summary,
        "p95_improvement_percent": if production_p95 > 0.0 { round_ms((production_p95 - candidate_p95) / production_p95 * 100.0) } else { 0.0 },
        "original_sql": production_sql,
        "candidate_sql": candidate_sql,
        "parameters": sql_values_json(&values),
        "original_explain_query_plan": production_plan,
        "candidate_explain_query_plan": candidate_plan,
        "original_uses_temp_btree": production_plan.iter().any(|line| line.contains("USE TEMP B-TREE")),
        "candidate_uses_temp_btree": candidate_plan.iter().any(|line| line.contains("USE TEMP B-TREE")),
        "candidate_materializes_key_columns_before_wide_projection": true,
        "entire_global_search_result_equal_including_rank": true,
        "original_sqlite_statement_counters": production_counters,
        "candidate_sqlite_statement_counters": candidate_counters,
        "original_process_resources": production_resources.json(production_samples.iter().sum()),
        "candidate_process_resources": candidate_resources.json(candidate_samples.iter().sum()),
        "temporary_memory_bytes": "NOT VERIFIED: SQLite exposes temp_store and plans here, but not per-statement temp B-tree bytes",
        "timing_scope": "statement prepare, full ordered result drain and GlobalSearchResult mapping; excludes pool checkout, remaining tiers and Repository Snapshot"
    });
    emit_record(&record);
    record
}

fn pooled_search_sample(
    db: &Database,
    query: &str,
    limit: u32,
    offset: u32,
    tier: &str,
    key_only_cte: bool,
) -> (Vec<GlobalSearchResult>, f64, Option<ProcessObservation>) {
    let (result, elapsed_ms, resources) = if key_only_cte {
        with_key_only_search_tier(tier, || {
            measure_with_process_counters(|| db.search_global_entries(query, limit, offset))
        })
    } else {
        with_legacy_search_tier(tier, || {
            measure_with_process_counters(|| db.search_global_entries(query, limit, offset))
        })
    };
    (
        result.unwrap_or_else(|error| panic!("pooled search failed for {query:?}: {error}")),
        elapsed_ms,
        resources,
    )
}

fn snapshot_sample(
    db: &Database,
    query: &str,
    limit: u32,
    offset: u32,
    tier: &str,
    key_only_cte: bool,
) -> (GlobalSearchSnapshot, f64, Option<ProcessObservation>) {
    let (result, elapsed_ms, resources) = if key_only_cte {
        with_key_only_search_tier(tier, || {
            measure_with_process_counters(|| {
                db.search_global_entries_snapshot(query, limit, offset)
            })
        })
    } else {
        with_legacy_search_tier(tier, || {
            measure_with_process_counters(|| {
                db.search_global_entries_snapshot(query, limit, offset)
            })
        })
    };
    (
        result.unwrap_or_else(|error| panic!("repository snapshot failed for {query:?}: {error}")),
        elapsed_ms,
        resources,
    )
}

fn profile_pooled_search_pair(
    context: &JsonValue,
    db: &Database,
    tier: &str,
    query_case: QueryCase,
    hit_count: usize,
) -> JsonValue {
    let expected = with_legacy_search_tier(tier, || {
        db.search_global_entries(query_case.query, QUERY_LIMIT, 0)
    })
    .unwrap_or_else(|error| panic!("warm legacy pooled search failed: {error}"));
    let cte_expected = with_key_only_search_tier(tier, || {
        db.search_global_entries(query_case.query, QUERY_LIMIT, 0)
    })
    .unwrap_or_else(|error| panic!("warm key-only pooled search failed: {error}"));
    assert_eq!(cte_expected, expected);

    for _ in 0..PAIRED_WARMUPS {
        assert_eq!(
            with_legacy_search_tier(tier, || {
                db.search_global_entries(query_case.query, QUERY_LIMIT, 0)
            })
            .expect("warm legacy pooled search"),
            expected
        );
        assert_eq!(
            with_key_only_search_tier(tier, || {
                db.search_global_entries(query_case.query, QUERY_LIMIT, 0)
            })
            .expect("warm key-only pooled search"),
            expected
        );
    }

    let mut original_samples = Vec::with_capacity(PAIRED_SAMPLES);
    let mut cte_samples = Vec::with_capacity(PAIRED_SAMPLES);
    let mut original_resources = ProcessResourceAccumulator::default();
    let mut cte_resources = ProcessResourceAccumulator::default();
    for sample in 0..PAIRED_SAMPLES {
        let mut original = || {
            let (results, elapsed, resources) =
                pooled_search_sample(db, query_case.query, QUERY_LIMIT, 0, tier, false);
            assert_eq!(results, expected);
            original_samples.push(elapsed);
            original_resources.observe(resources);
        };
        let mut candidate = || {
            let (results, elapsed, resources) =
                pooled_search_sample(db, query_case.query, QUERY_LIMIT, 0, tier, true);
            assert_eq!(results, expected);
            cte_samples.push(elapsed);
            cte_resources.observe(resources);
        };
        if sample % 2 == 0 {
            original();
            candidate();
        } else {
            candidate();
            original();
        }
    }
    let original_summary = summarize_samples(&original_samples);
    let cte_summary = summarize_samples(&cte_samples);
    let original_p95 = original_summary["p95"].as_f64().unwrap_or_default();
    let cte_p95 = cte_summary["p95"].as_f64().unwrap_or_default();
    let record = json!({
        "schema_version": 1,
        "record_type": "key_only_cte_pooled_search_paired",
        "context": context,
        "query_class": query_case.class,
        "query": query_case.query,
        "eligible_match_count": hit_count,
        "limit": QUERY_LIMIT,
        "offset": 0,
        "result_count": expected.len(),
        "original_latency_ms": original_summary,
        "key_only_cte_latency_ms": cte_summary,
        "p95_improvement_percent": if original_p95 > 0.0 { round_ms((original_p95 - cte_p95) / original_p95 * 100.0) } else { 0.0 },
        "original_passes_historical_100ms_gate": original_p95 <= 100.0,
        "key_only_cte_passes_historical_100ms_gate": cte_p95 <= 100.0,
        "historical_100ms_gate_changed": false,
        "entire_ordered_results_equal": true,
        "original_result_ids_prefix": expected.iter().take(8).map(|row| row.id.clone()).collect::<Vec<_>>(),
        "samples_per_variant": PAIRED_SAMPLES,
        "warmups_per_variant": PAIRED_WARMUPS,
        "paired_order": "actual Database::search_global_entries pool entry point; same 100k or 500k database and connection pool; first position alternates each paired sample",
        "original_process_resources": original_resources.json(original_samples.iter().sum()),
        "candidate_process_resources": cte_resources.json(cte_samples.iter().sum()),
        "timing_scope": "pool checkout plus all production search tiers, result de-duplication and page extraction; excludes Snapshot facts"
    });
    emit_record(&record);
    record
}

fn profile_repository_snapshot_pair(
    context: &JsonValue,
    db: &Database,
    tier: &str,
    query_case: QueryCase,
    hit_count: usize,
) -> JsonValue {
    let expected = with_legacy_search_tier(tier, || {
        db.search_global_entries_snapshot(query_case.query, QUERY_LIMIT, 0)
    })
    .unwrap_or_else(|error| panic!("warm legacy repository snapshot failed: {error}"));
    let cte_expected = with_key_only_search_tier(tier, || {
        db.search_global_entries_snapshot(query_case.query, QUERY_LIMIT, 0)
    })
    .unwrap_or_else(|error| panic!("warm key-only repository snapshot failed: {error}"));
    assert_snapshot_results_equal(&expected, &cte_expected);

    for _ in 0..PAIRED_WARMUPS {
        let original = with_legacy_search_tier(tier, || {
            db.search_global_entries_snapshot(query_case.query, QUERY_LIMIT, 0)
        })
        .expect("warm legacy repository snapshot");
        let candidate = with_key_only_search_tier(tier, || {
            db.search_global_entries_snapshot(query_case.query, QUERY_LIMIT, 0)
        })
        .expect("warm key-only repository snapshot");
        assert_snapshot_results_equal(&original, &candidate);
        assert_snapshot_results_equal(&expected, &original);
    }

    let mut original_samples = Vec::with_capacity(PAIRED_SAMPLES);
    let mut cte_samples = Vec::with_capacity(PAIRED_SAMPLES);
    let mut original_resources = ProcessResourceAccumulator::default();
    let mut cte_resources = ProcessResourceAccumulator::default();
    for sample in 0..PAIRED_SAMPLES {
        let mut original = || {
            let (snapshot, elapsed, resources) =
                snapshot_sample(db, query_case.query, QUERY_LIMIT, 0, tier, false);
            assert_snapshot_results_equal(&expected, &snapshot);
            original_samples.push(elapsed);
            original_resources.observe(resources);
        };
        let mut candidate = || {
            let (snapshot, elapsed, resources) =
                snapshot_sample(db, query_case.query, QUERY_LIMIT, 0, tier, true);
            assert_snapshot_results_equal(&expected, &snapshot);
            cte_samples.push(elapsed);
            cte_resources.observe(resources);
        };
        if sample % 2 == 0 {
            original();
            candidate();
        } else {
            candidate();
            original();
        }
    }
    let original_summary = summarize_samples(&original_samples);
    let cte_summary = summarize_samples(&cte_samples);
    let original_p95 = original_summary["p95"].as_f64().unwrap_or_default();
    let cte_p95 = cte_summary["p95"].as_f64().unwrap_or_default();
    let record = json!({
        "schema_version": 1,
        "record_type": "key_only_cte_repository_snapshot_paired",
        "context": context,
        "query_class": query_case.class,
        "query": query_case.query,
        "eligible_match_count": hit_count,
        "limit": QUERY_LIMIT,
        "offset": 0,
        "result_count": expected.results.len(),
        "original_latency_ms": original_summary,
        "key_only_cte_latency_ms": cte_summary,
        "p95_improvement_percent": if original_p95 > 0.0 { round_ms((original_p95 - cte_p95) / original_p95 * 100.0) } else { 0.0 },
        "original_result_ids_prefix": expected.results.iter().take(8).map(|row| row.id.clone()).collect::<Vec<_>>(),
        "search_results_equal": true,
        "source_health_equal": true,
        "source_revision_equal": true,
        "source_revision_blake3": expected.source_revision,
        "index_status_equal": true,
        "same_read_transaction": true,
        "source_health_candidate": "#359 Candidate B, current production implementation, both variants",
        "no_source_health_Candidate_C_comparison": true,
        "samples_per_variant": PAIRED_SAMPLES,
        "warmups_per_variant": PAIRED_WARMUPS,
        "paired_order": "actual Database::search_global_entries_snapshot; each call performs the exact production search, #359 source-health, revision hash and index-status reads in one SQLite read transaction; order alternates",
        "original_process_resources": original_resources.json(original_samples.iter().sum()),
        "candidate_process_resources": cte_resources.json(cte_samples.iter().sum()),
        "sqlite_statement_counters": "NOT AVAILABLE from the repository entry point; per-tier SQLite statement counters are reported in key_only_cte_sql_paired records",
        "temporary_memory_bytes": "NOT VERIFIED: SQLite exposes temp_store and plans here, but not per-statement temp B-tree bytes",
        "timing_scope": "pool checkout, read transaction setup, full search, source-health rows, source_revision facts/hash, index status and transaction commit"
    });
    emit_record(&record);
    record
}

fn profile_key_only_pagination(context: &JsonValue, db: &Database, conn: &Connection) {
    let page_cases = [
        ("first_page_80", 80_u32, 0_u32),
        ("limit_200", 200, 0),
        ("limit_clamp_500_to_200", 500, 0),
        ("last_80_before_candidate_cap", 80, 4_016),
        ("last_200_before_candidate_cap", 200, 3_896),
        ("candidate_cap_offset_4096", 80, 4_096),
    ];
    for (class, query, tier) in [("name_prefix", "quarterly", "name_prefix")] {
        let production_window = with_legacy_name_prefix_search_candidate(|| {
            diagnostic_search_tier(conn, tier, query, 4_096)
        })
        .unwrap_or_else(|error| panic!("collect legacy 4096 window for {query}: {error}"));
        let candidate_sql = diagnostic_key_only_cte_sql(tier);
        let values = key_only_values(tier, query, 4_096);
        let cte_window = run_result_sql(conn, &candidate_sql, &values);
        assert_eq!(cte_window, production_window);
        assert_eq!(
            with_key_only_cte_search_candidate(|| {
                diagnostic_search_tier(conn, tier, query, 4_096)
            })
            .expect("collect key-only CTE helper 4096 window"),
            production_window
        );

        for (page_class, limit, offset) in page_cases {
            let original = with_legacy_name_prefix_search_candidate(|| {
                db.search_global_entries(query, limit, offset)
            })
            .unwrap_or_else(|error| panic!("legacy page {page_class}: {error}"));
            let candidate = with_key_only_cte_search_candidate(|| {
                db.search_global_entries(query, limit, offset)
            })
            .unwrap_or_else(|error| panic!("key-only CTE page {page_class}: {error}"));
            assert_eq!(candidate, original);
            assert!(original.len() <= limit.min(200) as usize);
            if offset >= SEARCH_CANDIDATE_LIMIT as u32 {
                assert!(original.is_empty());
            }
            emit_record(&json!({
                "schema_version": 1,
                "record_type": "key_only_cte_pagination_equivalence",
                "context": context,
                "query_class": class,
                "tier": tier,
                "query": query,
                "page_class": page_class,
                "requested_limit": limit,
                "effective_limit": limit.clamp(1, 200),
                "offset": offset,
                "candidate_window": SEARCH_CANDIDATE_LIMIT,
                "result_count": original.len(),
                "entire_ordered_result_objects_equal": true,
                "rank_and_tie_break_equality": true,
                "offset_at_or_beyond_candidate_cap_is_empty": offset >= SEARCH_CANDIDATE_LIMIT as u32 && original.is_empty()
            }));
        }
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "key_only_cte_candidate_window_equivalence",
            "context": context,
            "query_class": class,
            "tier": tier,
            "query": query,
            "candidate_window": SEARCH_CANDIDATE_LIMIT,
            "result_count": production_window.len(),
            "all_result_fields_equal_through_4096": true,
            "fts_rank_and_stable_order_equal": tier != "fts" || production_window == cte_window
        }));
    }
}

fn profile_key_only_fts_pagination(context: &JsonValue, db: &Database, conn: &Connection) {
    let page_cases = [
        ("first_page_80", 80_u32, 0_u32),
        ("limit_200", 200, 0),
        ("limit_clamp_500_to_200", 500, 0),
        ("last_80_before_candidate_cap", 80, 4_016),
        ("last_200_before_candidate_cap", 200, 3_896),
        ("candidate_cap_offset_4096", 80, 4_096),
    ];
    for (class, query, tier) in [
        ("fts_substring_report", "report", "fts"),
        ("fts_substring_invoice", "invoice", "fts"),
    ] {
        let legacy_window = with_legacy_fts_search_candidate(|| {
            diagnostic_search_tier(conn, tier, query, SEARCH_CANDIDATE_LIMIT as u32)
        })
        .unwrap_or_else(|error| panic!("collect legacy 4096 FTS window for {query}: {error}"));
        let candidate_window = with_key_only_fts_search_candidate(|| {
            diagnostic_search_tier(conn, tier, query, SEARCH_CANDIDATE_LIMIT as u32)
        })
        .unwrap_or_else(|error| panic!("collect key-only 4096 FTS window for {query}: {error}"));
        assert_eq!(legacy_window.len(), SEARCH_CANDIDATE_LIMIT);
        assert_eq!(candidate_window, legacy_window);

        for (page_class, limit, offset) in page_cases {
            let original =
                with_legacy_fts_search_candidate(|| db.search_global_entries(query, limit, offset))
                    .unwrap_or_else(|error| panic!("legacy FTS page {page_class}: {error}"));
            let candidate = with_key_only_fts_search_candidate(|| {
                db.search_global_entries(query, limit, offset)
            })
            .unwrap_or_else(|error| panic!("key-only FTS page {page_class}: {error}"));
            assert_eq!(candidate, original);
            assert!(original.len() <= limit.min(200) as usize);
            if offset >= SEARCH_CANDIDATE_LIMIT as u32 {
                assert!(original.is_empty());
            }
            emit_record(&json!({
                "schema_version": 1,
                "record_type": "key_only_fts_pagination_equivalence",
                "context": context,
                "query_class": class,
                "tier": tier,
                "query": query,
                "page_class": page_class,
                "requested_limit": limit,
                "effective_limit": limit.clamp(1, 200),
                "offset": offset,
                "candidate_window": SEARCH_CANDIDATE_LIMIT,
                "result_count": original.len(),
                "entire_ordered_result_objects_equal": true,
                "bm25_rank_and_tie_break_equality": true,
                "offset_at_or_beyond_candidate_cap_is_empty": offset >= SEARCH_CANDIDATE_LIMIT as u32 && original.is_empty()
            }));
        }
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "key_only_fts_candidate_window_equivalence",
            "context": context,
            "query_class": class,
            "tier": tier,
            "query": query,
            "candidate_window": SEARCH_CANDIDATE_LIMIT,
            "result_count": legacy_window.len(),
            "all_result_fields_equal_through_4096": true,
            "fts_rank_and_stable_order_equal": candidate_window == legacy_window
        }));
    }
}

#[allow(dead_code)] // Retained for the earlier multi-hit research; not run in the Issue #364 product qualification.
fn profile_key_only_hit_count_scale(context: &JsonValue, conn: &Connection, base_entries: u64) {
    let transaction = conn
        .unchecked_transaction()
        .expect("begin rollback-only key-only hit-count overlay");
    let mut insert = transaction
        .prepare(INSERT_SYNTHETIC_ENTRY)
        .expect("prepare key-only hit-count overlay insert");
    let scales = [
        ("scale1", 1_usize),
        ("scale5", 5),
        ("scale100", 100),
        ("scale1000", 1_000),
        ("scale4096", 4_096),
        ("scale10000", 10_000),
        ("scale25000", 25_000),
    ];
    for (label, count) in scales {
        for index in 0..count {
            let id = format!("issue360-{label}-{index:05}");
            let name = format!("{label} candidate {index:05}.txt");
            let path = format!(r"C:\Issue360Scale\{label}\{name}");
            insert
                .execute(params![
                    id,
                    "gv_test",
                    id,
                    "issue360-scale-parent",
                    name,
                    name.to_lowercase(),
                    path,
                    path.to_lowercase(),
                    "txt",
                    0_i64,
                    1_i64,
                    1_700_700_000_i64,
                    1_900_000_000_i64 + index as i64,
                    0_i64,
                    0_i64,
                    0_i64,
                    0_i64,
                    "issue360_key_only_scale",
                    1_700_700_000_i64 + index as i64,
                ])
                .unwrap_or_else(|error| panic!("insert key-only hit-count row: {error}"));
        }
    }
    drop(insert);

    let mut scale_records = Vec::new();
    let no_hit_query = "issue360-no-hit";
    let no_hit_count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM global_entries ge JOIN global_volumes gv ON gv.id = ge.volume_id WHERE gv.enabled = 1 AND ge.is_stale = 0 AND ge.name_normalized GLOB ?1",
            [format!("{}*", benchmark_escape_glob(no_hit_query))],
            |row| row.get(0),
        )
        .expect("count zero-hit key-only prefix");
    assert_eq!(no_hit_count, 0);
    let no_hit = profile_key_only_sql_pair(
        context,
        &transaction,
        KeyOnlySqlPair {
            query_class: "hit_scale_0",
            tier: "name_prefix",
            query: no_hit_query,
            hit_count: 0,
            limit: QUERY_LIMIT,
            samples: PAIRED_SAMPLES,
        },
    );
    scale_records.push(json!({"query": no_hit_query, "hit_count": 0, "record": no_hit}));

    for (label, expected_count) in scales {
        let query = format!("{label} ");
        let expected_pattern = format!("{}*", benchmark_escape_glob(&query));
        let actual_count: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM global_entries ge JOIN global_volumes gv ON gv.id = ge.volume_id WHERE gv.enabled = 1 AND ge.is_stale = 0 AND ge.name_normalized GLOB ?1",
                [expected_pattern],
                |row| row.get(0),
            )
            .expect("count key-only prefix scale matches");
        assert_eq!(actual_count as usize, expected_count);
        let record = profile_key_only_sql_pair(
            context,
            &transaction,
            KeyOnlySqlPair {
                query_class: &format!("hit_scale_{expected_count}"),
                tier: "name_prefix",
                query: &query,
                hit_count: expected_count,
                limit: QUERY_LIMIT,
                samples: PAIRED_SAMPLES,
            },
        );
        scale_records.push(json!({
            "query": query,
            "hit_count": expected_count,
            "returned_page_count": expected_count.min(QUERY_LIMIT as usize),
            "record": record
        }));

        if expected_count >= SEARCH_CANDIDATE_LIMIT {
            let values = prefix_variant_values(&query, SEARCH_CANDIDATE_LIMIT as u32);
            let production_window =
                run_result_sql(&transaction, &diagnostic_legacy_name_prefix_sql(), &values);
            let cte_window = run_result_sql(
                &transaction,
                &diagnostic_key_only_cte_sql("name_prefix"),
                &values,
            );
            assert_eq!(cte_window, production_window);
            emit_record(&json!({
                "schema_version": 1,
                "record_type": "key_only_cte_scale_window_equivalence",
                "context": context,
                "query": query,
                "eligible_hit_count": expected_count,
                "candidate_window": SEARCH_CANDIDATE_LIMIT,
                "result_count": cte_window.len(),
                "all_fields_equal": true
            }));
        }
    }

    let overlay_count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM global_entries WHERE id LIKE 'issue360-scale%'",
            [],
            |row| row.get(0),
        )
        .expect("count temporary key-only scale overlay");
    assert_eq!(overlay_count, 40_202);
    transaction
        .rollback()
        .expect("rollback key-only hit-count overlay");
    let remaining_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM global_entries", [], |row| row.get(0))
        .expect("count base fixture rows after hit-count rollback");
    assert_eq!(remaining_rows, base_entries as i64);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "key_only_cte_hit_count_scale_summary",
        "context": context,
        "base_fixture_rows": base_entries,
        "overlay_rows": overlay_count,
        "eligible_match_counts": scale_records.iter().map(|record| json!({"query":record["query"],"hit_count":record["hit_count"]})).collect::<Vec<_>>(),
        "per_count_paired_sql_records": scale_records,
        "candidate_window": SEARCH_CANDIDATE_LIMIT,
        "overlay_rolled_back_before_next_test": true,
        "persistent_schema_or_index_changed": false
    }));
}

#[test]
#[ignore = "controlled 100k/500k Issue #360 key-only CTE qualification on one synthetic fixture"]
fn global_search_key_only_cte_qualification() {
    let entries = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .expect("qualification fixture size must be set");
    assert!(
        matches!(entries, 100_000 | 500_000),
        "qualification is bounded to the paired 100k and 500k fixtures; never run 1m"
    );
    let output_path = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT").ok();
    if let Some(path) = &output_path {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create key-only evidence directory");
        }
        fs::write(path, "").expect("truncate key-only JSONL evidence");
    }

    let context = benchmark_context(entries);
    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open key-only qualification database");
    db.upsert_global_volume(&super::super::test_volume())
        .expect("insert key-only base enabled volume");
    let population_started = Instant::now();
    let (expectations, generation_ms, population_ms, transaction_samples) =
        populate_synthetic_database(&db, entries);
    let fixture_elapsed_ms = population_started.elapsed().as_secs_f64() * 1_000.0;
    let conn = configured_connection(&path).expect("open key-only qualification reader");
    let schema_signature_before = schema_index_signature(&conn);
    let sqlite_version: String = conn
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .expect("read qualification SQLite version");
    let actual_total: i64 = conn
        .query_row("SELECT COUNT(*) FROM global_entries", [], |row| row.get(0))
        .expect("count qualification base rows");
    assert_eq!(actual_total, entries as i64);
    let pragmas = json!({
        "journal_mode": conn.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0)).expect("journal mode"),
        "synchronous": conn.query_row("PRAGMA synchronous", [], |row| row.get::<_, i64>(0)).expect("synchronous"),
        "foreign_keys": conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0)).expect("foreign keys"),
        "temp_store": conn.query_row("PRAGMA temp_store", [], |row| row.get::<_, i64>(0)).expect("temp store"),
        "mmap_size": conn.query_row("PRAGMA mmap_size", [], |row| row.get::<_, i64>(0)).expect("mmap size"),
        "page_size": conn.query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0)).expect("page size")
    });
    let trigger_names = assert_entry_triggers_enabled(&conn);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "key_only_cte_environment_and_dataset",
        "context": context,
        "runner": {
            "os": env::consts::OS,
            "arch": env::consts::ARCH,
            "image_os": env::var("ImageOS").ok(),
            "image_version": env::var("ImageVersion").ok(),
            "github_job": env::var("GITHUB_JOB").ok(),
            "github_run_id": env::var("GITHUB_RUN_ID").ok(),
            "github_run_attempt": env::var("GITHUB_RUN_ATTEMPT").ok()
        },
        "base_entries": actual_total,
        "fixture_type": "existing deterministic synthetic Global Search fixture with production schema, indexes, FTS triggers and 512-row insert transactions; no filesystem scan",
        "sqlite_version": sqlite_version,
        "sqlite_pragmas": pragmas,
        "generation_ms": generation_ms,
        "population_ms": population_ms,
        "total_fixture_build_wall_ms": fixture_elapsed_ms,
        "population_transaction_ms": summarize_samples(&transaction_samples),
        "database": database_metrics(&conn, &path),
        "production_trigger_names": trigger_names,
        "process_cpu_rss": "sampled around measured SQL and request calls on Windows; Linux returns NOT_VERIFIED_ON_THIS_RUNNER",
        "filesystem_or_OS_file_discovery_measured": false,
        "source_health_variant": "#359 Candidate B current production SQL, left unchanged"
    }));

    // Count-oracle and full result-object equality cover the complete historical
    // 12-query matrix before any performance assertions are emitted.
    let mut correctness_records = Vec::with_capacity(QUERY_MATRIX.len());
    for (query_case, expectation) in QUERY_MATRIX.iter().copied().zip(expectations.iter()) {
        let actual_count = actual_match_count(&conn, query_case);
        assert_eq!(
            actual_count as u64, expectation.total_matches,
            "qualification count oracle for {}",
            query_case.class
        );
        let expected = assert_search_results(
            &conn,
            query_case,
            expectation,
            SEARCH_RESULT_LIMIT,
            0,
            entries,
        );
        let original = with_legacy_name_prefix_search_candidate(|| {
            db.search_global_entries(query_case.query, SEARCH_RESULT_LIMIT, 0)
        })
        .unwrap_or_else(|error| panic!("legacy full search for {}: {error}", query_case.class));
        assert_eq!(
            original, expected,
            "legacy result object for {}",
            query_case.class
        );
        let key_only = with_key_only_cte_search_candidate(|| {
            db.search_global_entries(query_case.query, SEARCH_RESULT_LIMIT, 0)
        })
        .unwrap_or_else(|error| {
            panic!(
                "key-only full-search candidate failed for {}: {error}",
                query_case.class
            )
        });
        assert_eq!(
            key_only, original,
            "full result object for {}",
            query_case.class
        );
        let original_snapshot = with_legacy_name_prefix_search_candidate(|| {
            db.search_global_entries_snapshot(query_case.query, SEARCH_RESULT_LIMIT, 0)
        })
        .unwrap_or_else(|error| panic!("legacy snapshot for {}: {error}", query_case.class));
        let key_only_snapshot = with_key_only_cte_search_candidate(|| {
            db.search_global_entries_snapshot(query_case.query, SEARCH_RESULT_LIMIT, 0)
        })
        .unwrap_or_else(|error| panic!("key-only snapshot for {}: {error}", query_case.class));
        assert_snapshot_results_equal(&original_snapshot, &key_only_snapshot);
        correctness_records.push(json!({
            "query_class": query_case.class,
            "query": query_case.query,
            "eligible_match_count": actual_count,
            "result_count": original.len(),
            "full_search_entire_result_objects_equal": true,
            "repository_snapshot_search_source_health_revision_and_index_status_equal": true
        }));
    }
    assert_eq!(correctness_records.len(), 12);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "key_only_cte_12_query_correctness",
        "context": context,
        "query_classes_checked": correctness_records.len(),
        "query_classes_passed": correctness_records.len(),
        "count_oracle_correct": true,
        "all_result_fields_including_bm25_rank_equal": true,
        "snapshot_search_source_health_revision_index_status_equal": true,
        "query_results": correctness_records
    }));

    // The one performance pair is the established high-fanout English
    // name-prefix case. The complete 12-query correctness matrix above still
    // covers zero/low/high hits, Chinese, punctuation and accented prefixes.
    let (query_case, expectation) = QUERY_MATRIX
        .iter()
        .copied()
        .zip(expectations.iter())
        .find(|(query, _)| query.class == "name_prefix")
        .expect("high-fanout name-prefix benchmark case");
    profile_key_only_sql_pair(
        &context,
        &conn,
        KeyOnlySqlPair {
            query_class: query_case.class,
            tier: "name_prefix",
            query: query_case.query,
            hit_count: expectation.total_matches as usize,
            limit: QUERY_LIMIT,
            samples: PAIRED_SAMPLES,
        },
    );
    let pooled_record = profile_pooled_search_pair(
        &context,
        &db,
        "name_prefix",
        query_case,
        expectation.total_matches as usize,
    );
    let snapshot_record = profile_repository_snapshot_pair(
        &context,
        &db,
        "name_prefix",
        query_case,
        expectation.total_matches as usize,
    );
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "issue364_name_prefix_full_path_summary",
        "context": context,
        "query_class": query_case.class,
        "query": query_case.query,
        "eligible_match_count": expectation.total_matches,
        "pooled_search_record": pooled_record,
        "repository_snapshot_record": snapshot_record,
        "historical_100ms_gate": 100.0,
        "only_one_performance_query_paired": true,
        "source_health_candidate_comparison_mixed_in": false,
        "other_tiers_promoted": false
    }));

    profile_key_only_pagination(&context, &db, &conn);

    // Reuse the same fixture for disabled-volume, stale-row, cross-volume tie,
    // managed/unmanaged, exact field, cross-tier duplicate and filter-before-
    // limit adversarial assertions. The intentionally unsafe variants must
    // underfill, while production and CTE candidates stay equivalent.
    let mut conn = conn;
    add_adversarial_rows(&db, &mut conn, &context);
    let schema_signature_after = schema_index_signature(&conn);
    assert_eq!(schema_signature_before, schema_signature_after);
    let remaining_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM global_entries", [], |row| row.get(0))
        .expect("count remaining qualification base rows");
    assert_eq!(remaining_rows, entries as i64);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "key_only_cte_qualification_complete",
        "context": context,
        "base_rows_after_rollback": remaining_rows,
        "schema_index_signature_unchanged": true,
        "source_health_production_sql_modified": false,
        "production_search_sql_modified": true,
        "production_tier_changed": "name_prefix only",
        "schema_or_persistent_index_changed": false,
        "100ms_gate_changed": false,
        "official_full_12_class_correctness": "PASS",
        "one_fixture_per_size": true,
        "one_million_row_benchmark_run": false,
        "macos_native_measurement": "NOT VERIFIED: qualification executes on Windows Hosted; macOS quality remains a separate CI lane",
        "temporary_btree_byte_measurement": "NOT VERIFIED"
    }));
}

#[test]
#[ignore = "one controlled 100k/500k Issue #364 high-hit FTS key-only CTE qualification"]
fn global_search_fts_key_only_cte_qualification() {
    let entries = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .expect("qualification fixture size must be set");
    assert!(
        matches!(entries, 100_000 | 500_000),
        "qualification is bounded to one 100k and one 500k fixture; never run 1m"
    );
    let output_path = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT").ok();
    if let Some(path) = &output_path {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create high-hit FTS evidence directory");
        }
        fs::write(path, "").expect("truncate high-hit FTS JSONL evidence");
    }

    let context = benchmark_context(entries);
    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open high-hit FTS qualification database");
    db.upsert_global_volume(&super::super::test_volume())
        .expect("insert high-hit FTS base enabled volume");
    let population_started = Instant::now();
    let (expectations, generation_ms, population_ms, transaction_samples) =
        populate_synthetic_database(&db, entries);
    let fixture_elapsed_ms = population_started.elapsed().as_secs_f64() * 1_000.0;
    let conn = configured_connection(&path).expect("open high-hit FTS qualification reader");
    let schema_signature_before = schema_index_signature(&conn);
    let sqlite_version: String = conn
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .expect("read qualification SQLite version");
    let actual_total: i64 = conn
        .query_row("SELECT COUNT(*) FROM global_entries", [], |row| row.get(0))
        .expect("count qualification base rows");
    assert_eq!(actual_total, entries as i64);
    let pragmas = json!({
        "journal_mode": conn.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0)).expect("journal mode"),
        "synchronous": conn.query_row("PRAGMA synchronous", [], |row| row.get::<_, i64>(0)).expect("synchronous"),
        "foreign_keys": conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0)).expect("foreign keys"),
        "temp_store": conn.query_row("PRAGMA temp_store", [], |row| row.get::<_, i64>(0)).expect("temp store"),
        "mmap_size": conn.query_row("PRAGMA mmap_size", [], |row| row.get::<_, i64>(0)).expect("mmap size"),
        "page_size": conn.query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0)).expect("page size")
    });
    let trigger_names = assert_entry_triggers_enabled(&conn);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "key_only_cte_environment_and_dataset",
        "context": context,
        "runner": {
            "os": env::consts::OS,
            "arch": env::consts::ARCH,
            "image_os": env::var("ImageOS").ok(),
            "image_version": env::var("ImageVersion").ok(),
            "github_job": env::var("GITHUB_JOB").ok(),
            "github_run_id": env::var("GITHUB_RUN_ID").ok(),
            "github_run_attempt": env::var("GITHUB_RUN_ATTEMPT").ok()
        },
        "base_entries": actual_total,
        "fixture_type": "existing deterministic synthetic Global Search fixture with production schema, indexes, FTS triggers and 512-row insert transactions; no filesystem scan",
        "sqlite_version": sqlite_version,
        "sqlite_pragmas": pragmas,
        "generation_ms": generation_ms,
        "population_ms": population_ms,
        "total_fixture_build_wall_ms": fixture_elapsed_ms,
        "population_transaction_ms": summarize_samples(&transaction_samples),
        "database": database_metrics(&conn, &path),
        "production_trigger_names": trigger_names,
        "process_cpu_rss": "sampled around measured SQL and request calls on Windows; Linux returns NOT_VERIFIED_ON_THIS_RUNNER",
        "filesystem_or_OS_file_discovery_measured": false,
        "source_health_variant": "#359 Candidate B current production SQL, left unchanged"
    }));

    // Count-oracle and full result-object equality cover the complete historical
    // 12-query matrix before any performance assertions are emitted.
    let mut correctness_records = Vec::with_capacity(QUERY_MATRIX.len());
    for (query_case, expectation) in QUERY_MATRIX.iter().copied().zip(expectations.iter()) {
        let actual_count = actual_match_count(&conn, query_case);
        assert_eq!(
            actual_count as u64, expectation.total_matches,
            "qualification count oracle for {}",
            query_case.class
        );
        let expected = assert_search_results(
            &conn,
            query_case,
            expectation,
            SEARCH_RESULT_LIMIT,
            0,
            entries,
        );
        let original = with_legacy_fts_search_candidate(|| {
            db.search_global_entries(query_case.query, SEARCH_RESULT_LIMIT, 0)
        })
        .unwrap_or_else(|error| panic!("legacy full search for {}: {error}", query_case.class));
        assert_eq!(
            original, expected,
            "legacy result object for {}",
            query_case.class
        );
        let key_only = with_key_only_fts_search_candidate(|| {
            db.search_global_entries(query_case.query, SEARCH_RESULT_LIMIT, 0)
        })
        .unwrap_or_else(|error| {
            panic!(
                "key-only full-search candidate failed for {}: {error}",
                query_case.class
            )
        });
        assert_eq!(
            key_only, original,
            "full result object for {}",
            query_case.class
        );
        let original_snapshot = with_legacy_fts_search_candidate(|| {
            db.search_global_entries_snapshot(query_case.query, SEARCH_RESULT_LIMIT, 0)
        })
        .unwrap_or_else(|error| panic!("legacy snapshot for {}: {error}", query_case.class));
        let key_only_snapshot = with_key_only_fts_search_candidate(|| {
            db.search_global_entries_snapshot(query_case.query, SEARCH_RESULT_LIMIT, 0)
        })
        .unwrap_or_else(|error| panic!("key-only snapshot for {}: {error}", query_case.class));
        assert_snapshot_results_equal(&original_snapshot, &key_only_snapshot);
        correctness_records.push(json!({
            "query_class": query_case.class,
            "query": query_case.query,
            "eligible_match_count": actual_count,
            "result_count": original.len(),
            "full_search_entire_result_objects_equal": true,
            "repository_snapshot_search_source_health_revision_and_index_status_equal": true
        }));
    }
    assert_eq!(correctness_records.len(), 12);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "key_only_cte_12_query_correctness",
        "context": context,
        "query_classes_checked": correctness_records.len(),
        "query_classes_passed": correctness_records.len(),
        "count_oracle_correct": true,
        "all_result_fields_including_bm25_rank_equal": true,
        "snapshot_search_source_health_revision_index_status_equal": true,
        "query_results": correctness_records
    }));

    // Keep Name Prefix (#423) on the same production path for both variants.
    // Only report and invoice high-hit FTS query profiles are paired.
    let mut paired_fts_records = Vec::with_capacity(2);
    for query_class in ["fts_substring_report", "fts_substring_invoice"] {
        let (query_case, expectation) = QUERY_MATRIX
            .iter()
            .copied()
            .zip(expectations.iter())
            .find(|(query, _)| query.class == query_class)
            .unwrap_or_else(|| panic!("missing high-hit FTS benchmark case {query_class}"));
        let raw_sql = profile_key_only_sql_pair(
            &context,
            &conn,
            KeyOnlySqlPair {
                query_class: query_case.class,
                tier: "fts",
                query: query_case.query,
                hit_count: expectation.total_matches as usize,
                limit: QUERY_LIMIT,
                samples: PAIRED_SAMPLES,
            },
        );
        let pooled_search = profile_pooled_search_pair(
            &context,
            &db,
            "fts",
            query_case,
            expectation.total_matches as usize,
        );
        let repository_snapshot = profile_repository_snapshot_pair(
            &context,
            &db,
            "fts",
            query_case,
            expectation.total_matches as usize,
        );
        paired_fts_records.push(json!({
            "query_class": query_case.class,
            "query": query_case.query,
            "eligible_match_count": expectation.total_matches,
            "raw_sql_record": raw_sql,
            "pooled_search_record": pooled_search,
            "repository_snapshot_record": repository_snapshot
        }));
    }
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "issue364_high_hit_fts_full_path_summary",
        "context": context,
        "paired_query_classes": ["fts_substring_report", "fts_substring_invoice"],
        "paired_fts_records": paired_fts_records,
        "production_name_prefix_candidate_changed_or_compared": false,
        "historical_100ms_gate": 100.0,
        "each_high_hit_fts_query_paired_once_per_fixture": true,
        "source_health_candidate_comparison_mixed_in": false,
        "other_tiers_promoted": false
    }));

    profile_key_only_fts_pagination(&context, &db, &conn);

    // Reuse the same fixture for disabled-volume, stale-row, cross-volume tie,
    // managed/unmanaged, exact field, cross-tier duplicate and filter-before-
    // limit adversarial assertions. The intentionally unsafe variants must
    // underfill, while production and CTE candidates stay equivalent.
    let mut conn = conn;
    add_adversarial_rows(&db, &mut conn, &context);
    let schema_signature_after = schema_index_signature(&conn);
    assert_eq!(schema_signature_before, schema_signature_after);
    let remaining_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM global_entries", [], |row| row.get(0))
        .expect("count remaining qualification base rows");
    assert_eq!(remaining_rows, entries as i64);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "key_only_cte_qualification_complete",
        "context": context,
        "base_rows_after_rollback": remaining_rows,
        "schema_index_signature_unchanged": true,
        "source_health_production_sql_modified": false,
        "production_search_sql_modified_by_qualification": false,
        "production_tier_under_review": "high-hit FTS report and invoice only",
        "name_prefix_423_tier_modified_or_benchmarked": false,
        "schema_or_persistent_index_changed": false,
        "100ms_gate_changed": false,
        "official_full_12_class_correctness": "PASS",
        "one_fixture_per_size": true,
        "one_million_row_benchmark_run": false,
        "macos_native_measurement": "NOT VERIFIED: qualification executes on Windows Hosted; macOS quality remains a separate CI lane",
        "temporary_btree_byte_measurement": "NOT VERIFIED"
    }));
}
