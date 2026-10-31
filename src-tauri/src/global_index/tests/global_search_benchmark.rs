use super::{test_db_path, test_entry, test_volume};
use crate::db::Database;
use crate::global_index::search::search_global_entries_on_connection;
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Connection};
use serde_json::{json, Value as JsonValue};
use std::collections::VecDeque;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "global_search_fts_diagnostic.rs"]
mod fts_diagnostic;
#[path = "global_search_index_cost_benchmark.rs"]
mod index_cost_benchmark;
#[path = "global_search_query_cost_diagnostic.rs"]
mod query_cost_diagnostic;

#[path = "issue474_candidates.rs"]
mod issue474_candidates;

#[path = "global_search_covering_indexes.rs"]
mod covering_indexes;

#[path = "schema_upgrade_cost.rs"]
mod schema_upgrade_cost;
#[path = "global_search_source_facts_pair.rs"]
mod source_facts_pair;

const DEFAULT_ENTRIES: u64 = 100_000;
const ALLOWED_ENTRIES: [u64; 5] = [100_000, 500_000, 1_000_000, 2_000_000, 5_000_000];
const INSERT_BATCH_SIZE: u64 = 512;
const SEARCH_RESULT_LIMIT: u32 = 80;
// This is the current search candidate-window contract in search.rs. Keep the
// assertion here so a product change cannot silently invalidate this evidence.
const SEARCH_CANDIDATE_LIMIT: usize = 4_096;
const WARM_QUERY_P95_LIMIT_MS: f64 = 100.0;
const WARMUP_SAMPLES: usize = 5;
// FTS substring queries can be unusually expensive on the synthetic corpus.
// Keep the minimum sample count required for useful p50/p95/p99 evidence so
// the full fixed query matrix completes within the hosted-runner budget.
const WARM_SAMPLES: usize = 30;
const REOPENED_CONNECTION_SAMPLES: usize = 30;

const INSERT_SYNTHETIC_ENTRY: &str = r#"
    INSERT INTO global_entries (
        id, volume_id, platform_file_id, parent_platform_file_id,
        name, name_normalized, path, path_normalized, extension,
        is_directory, size, created_at_fs, modified_at_fs,
        file_attributes, is_hidden, is_system, is_stale,
        source_provider, last_seen_at
    ) VALUES (
        ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
        ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
    )
"#;

#[derive(Clone, Copy)]
enum MatchRule {
    ExactName(&'static str),
    NamePrefix(&'static str),
    FtsSubstring(&'static str),
    ExactExtension(&'static str),
    ExtensionPrefix(&'static str),
    NoMatch,
}

#[derive(Clone, Copy)]
struct QueryCase {
    class: &'static str,
    query: &'static str,
    match_rule: MatchRule,
}

const QUERY_MATRIX: [QueryCase; 12] = [
    QueryCase {
        class: "exact_basename",
        query: "Annual Budget 2026.xlsx",
        match_rule: MatchRule::ExactName("annual budget 2026.xlsx"),
    },
    QueryCase {
        class: "name_prefix",
        query: "quarterly",
        match_rule: MatchRule::NamePrefix("quarterly"),
    },
    QueryCase {
        class: "common_prefix_high_fanout",
        query: "IMG_",
        match_rule: MatchRule::NamePrefix("img_"),
    },
    QueryCase {
        class: "fts_substring_report",
        query: "report",
        match_rule: MatchRule::FtsSubstring("report"),
    },
    QueryCase {
        class: "fts_substring_invoice",
        query: "invoice",
        match_rule: MatchRule::FtsSubstring("invoice"),
    },
    QueryCase {
        class: "extension_exact",
        query: "pdf",
        match_rule: MatchRule::ExactExtension("pdf"),
    },
    QueryCase {
        class: "extension_prefix",
        query: "jp",
        match_rule: MatchRule::ExtensionPrefix("jp"),
    },
    QueryCase {
        class: "duplicate_basename",
        query: "meeting-notes.md",
        match_rule: MatchRule::ExactName("meeting-notes.md"),
    },
    QueryCase {
        class: "no_result",
        query: "zzznomatchtoken",
        match_rule: MatchRule::NoMatch,
    },
    QueryCase {
        class: "chinese_prefix",
        query: "数据库",
        match_rule: MatchRule::NamePrefix("数据库"),
    },
    QueryCase {
        class: "punctuation_prefix",
        query: "final-v2",
        match_rule: MatchRule::NamePrefix("final-v2"),
    },
    QueryCase {
        class: "unicode_accented_prefix",
        query: "RÉSUMÉ",
        match_rule: MatchRule::NamePrefix("résumé"),
    },
];

#[derive(Debug)]
struct SyntheticEntry {
    index: u64,
    id: String,
    platform_file_id: String,
    parent_platform_file_id: String,
    name: String,
    name_normalized: String,
    path: String,
    path_normalized: String,
    extension: String,
    size: i64,
    created_at_fs: i64,
    modified_at_fs: i64,
    file_attributes: i64,
    is_hidden: bool,
    is_system: bool,
}

#[derive(Clone, Debug)]
struct ExpectedResult {
    id: String,
    name: String,
}

struct QueryExpectation {
    total_matches: u64,
    newest_candidates: VecDeque<ExpectedResult>,
}

impl QueryExpectation {
    fn new() -> Self {
        Self {
            total_matches: 0,
            newest_candidates: VecDeque::with_capacity(SEARCH_CANDIDATE_LIMIT),
        }
    }

    fn observe(&mut self, entry: &SyntheticEntry) {
        self.total_matches += 1;
        self.newest_candidates.push_back(ExpectedResult {
            id: entry.id.clone(),
            name: entry.name.clone(),
        });
        if self.newest_candidates.len() > SEARCH_CANDIDATE_LIMIT {
            self.newest_candidates.pop_front();
        }
    }

    fn ordered_candidates(&self) -> impl Iterator<Item = &ExpectedResult> {
        self.newest_candidates.iter().rev()
    }

    fn expected_page(&self, offset: usize, limit: usize) -> Vec<&str> {
        self.ordered_candidates()
            .skip(offset)
            .take(limit)
            .map(|entry| entry.id.as_str())
            .collect()
    }

    fn expected_first(&self) -> Option<&ExpectedResult> {
        self.ordered_candidates().next()
    }

    fn bounded_match_count(&self) -> usize {
        self.total_matches.min(SEARCH_CANDIDATE_LIMIT as u64) as usize
    }
}

struct BenchmarkCleanup(PathBuf);

impl Drop for BenchmarkCleanup {
    fn drop(&mut self) {
        remove_database_files(&self.0);
    }
}

fn configured_entries() -> u64 {
    let entries = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_ENTRIES);
    assert!(
        ALLOWED_ENTRIES.contains(&entries),
        "ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES must be one of {ALLOWED_ENTRIES:?}, got {entries}"
    );
    entries
}

fn generated_entry(index: u64) -> SyntheticEntry {
    const EXTENSIONS: [&str; 16] = [
        "pdf", "docx", "xlsx", "pptx", "txt", "md", "jpg", "png", "mp4", "zip", "exe", "app", "ts",
        "js", "rs", "json",
    ];

    let (stem, extension) = if index == 0 {
        ("Annual Budget 2026".to_string(), "xlsx")
    } else {
        let extension = EXTENSIONS[((index / 20) % EXTENSIONS.len() as u64) as usize];
        let unique = format!("{index:07}");
        match index % 20 {
            0 => (format!("IMG_20260918_142233_{unique}"), "jpg"),
            1 => (format!("Quarterly Review 2026 {unique}"), "pdf"),
            2 => (format!("Annual Report {unique}"), "pdf"),
            3 => (format!("Outgoing Invoice {unique}"), "xlsx"),
            4 => ("meeting-notes".to_string(), "md"),
            5 => (format!("数据库技术教案 {unique}"), "docx"),
            6 => (format!("Résumé Project {unique}"), "pdf"),
            7 => (format!("final-v2-{unique}"), "md"),
            8 => (format!("Cafe\u{301} notes {unique}"), "txt"),
            9 => (format!("Board Notes_Q3-final.v2 {unique}"), "pptx"),
            10 => (format!("MiXeD Case_2026-Q3.v2 {unique}"), extension),
            11 => (format!("Download Batch {unique}"), extension),
            12 => (format!("Build Release_candidate-42 {unique}"), extension),
            13 => (format!("客户交付-Plan_{unique}"), extension),
            14 => (
                format!(
                    "Long reference manual with supporting materials for regional planning {unique}"
                ),
                extension,
            ),
            15 => (format!("Photo Session 2026_{unique}"), extension),
            16 => (format!("package-lock.release-{unique}"), extension),
            17 => (format!("run_tests.42-build-{unique}"), extension),
            18 => (format!("Canvas Preview Draft {unique}"), extension),
            _ => (format!("Deep Archive Set-{unique}"), extension),
        }
    };

    let name = format!("{stem}.{extension}");
    let directory = synthetic_directory(index);
    let path = format!("{directory}\\{name}");
    let name_normalized = name.to_lowercase();
    let path_normalized = path.to_lowercase();
    SyntheticEntry {
        index,
        id: format!("ge_bench_{index:07}"),
        platform_file_id: format!("synthetic-file-{index:07}"),
        parent_platform_file_id: format!("synthetic-parent-{:07}", index / 20),
        name,
        name_normalized,
        path,
        path_normalized,
        extension: extension.to_string(),
        size: ((index as i64 * 104_729) % 5_000_000_000) + 1,
        created_at_fs: 1_600_000_000 + (index as i64 / 2),
        modified_at_fs: 1_600_000_000 + index as i64,
        file_attributes: (index % 16) as i64,
        is_hidden: index.is_multiple_of(97),
        is_system: index.is_multiple_of(997),
    }
}

fn synthetic_directory(index: u64) -> String {
    match index % 20 {
        // Fixed-width paths keep the FTS document length identical within the
        // report/invoice classes so their BM25 tie order is the durable mtime
        // then id order used by the expected-result model.
        2 => format!(
            r"C:\Users\Owner\Documents\Archive\Year2026\Q3\Team{:07}\Dept{:03}",
            index / 20,
            (index / 20) % 997
        ),
        3 => format!(
            r"C:\Users\Owner\Downloads\Import\Batch{:07}\Group{:03}",
            index / 20,
            (index / 20) % 997
        ),
        4 => format!(r"C:\dev\workspace\project-{:07}\packages\notes", index / 20),
        _ => match index % 6 {
            0 => format!(r"C:\Users\Owner\Desktop\Set-{:04}", index % 1_000),
            1 => format!(r"C:\Users\Owner\Downloads\Incoming\Batch-{:07}", index / 20),
            2 => format!(
                r"C:\Users\Owner\Pictures\Camera Roll\2026\Month-{:02}",
                (index % 12) + 1
            ),
            3 => format!(
                r"C:\Users\Owner\Documents\Projects\Project-{:04}\src\module-{:03}",
                index % 1_000,
                index % 100
            ),
            4 => format!(
                r"C:\dev\workspace\zen-canvas\packages\module-{:04}\src",
                index % 1_000
            ),
            _ => format!(
                r"C:\Users\Owner\Documents\Reference\2026\Q3\Region-{:02}\Team-{:07}\Specs\Archive",
                index % 18,
                index / 20
            ),
        },
    }
}

impl QueryCase {
    fn matches(self, entry: &SyntheticEntry) -> bool {
        match self.match_rule {
            MatchRule::ExactName(name) => entry.name_normalized == name,
            MatchRule::NamePrefix(prefix) => entry.name_normalized.starts_with(prefix),
            MatchRule::FtsSubstring(value) => entry.name_normalized.contains(value),
            MatchRule::ExactExtension(extension) => entry.extension == extension,
            MatchRule::ExtensionPrefix(prefix) => entry.extension.starts_with(prefix),
            MatchRule::NoMatch => false,
        }
    }
}

fn actual_match_count(conn: &Connection, query_case: QueryCase) -> i64 {
    if matches!(query_case.match_rule, MatchRule::NoMatch) {
        return conn
            .query_row("SELECT COUNT(*) FROM global_entries WHERE 0", [], |row| {
                row.get(0)
            })
            .expect("count no-match oracle");
    }

    let sql = match query_case.match_rule {
        MatchRule::ExactName(_) => {
            "SELECT COUNT(*) FROM global_entries ge JOIN global_volumes gv ON gv.id = ge.volume_id WHERE gv.enabled = 1 AND ge.is_stale = 0 AND ge.name_normalized = ?1"
        }
        MatchRule::NamePrefix(_) => {
            "SELECT COUNT(*) FROM global_entries ge JOIN global_volumes gv ON gv.id = ge.volume_id WHERE gv.enabled = 1 AND ge.is_stale = 0 AND ge.name_normalized GLOB ?1"
        }
        MatchRule::FtsSubstring(_) => {
            "SELECT COUNT(*) FROM global_entries_fts WHERE global_entries_fts MATCH ?1"
        }
        MatchRule::ExactExtension(_) => {
            "SELECT COUNT(*) FROM global_entries ge JOIN global_volumes gv ON gv.id = ge.volume_id WHERE gv.enabled = 1 AND ge.is_stale = 0 AND ge.extension = ?1"
        }
        MatchRule::ExtensionPrefix(_) => {
            "SELECT COUNT(*) FROM global_entries ge JOIN global_volumes gv ON gv.id = ge.volume_id WHERE gv.enabled = 1 AND ge.is_stale = 0 AND ge.extension GLOB ?1"
        }
        MatchRule::NoMatch => unreachable!("no-match query handled above"),
    };

    let value = match query_case.match_rule {
        MatchRule::ExactName(name) => SqlValue::Text(name.to_string()),
        MatchRule::NamePrefix(prefix) | MatchRule::ExtensionPrefix(prefix) => {
            SqlValue::Text(format!("{}*", benchmark_escape_glob(prefix)))
        }
        MatchRule::FtsSubstring(value) => {
            SqlValue::Text(format!("\"{}\"", value.replace('"', "\"\"")))
        }
        MatchRule::ExactExtension(extension) => SqlValue::Text(extension.to_string()),
        MatchRule::NoMatch => unreachable!("no-match query handled above"),
    };

    conn.query_row(sql, [value], |row| row.get(0))
        .unwrap_or_else(|error| {
            panic!(
                "count oracle failed for class={}, query={:?}: {error}",
                query_case.class, query_case.query
            )
        })
}

// Keep the count oracle self-contained: the production search helper is
// intentionally private, and the oracle should build its own expected GLOB.
fn benchmark_escape_glob(value: &str) -> String {
    value
        .chars()
        .fold(String::with_capacity(value.len()), |mut result, ch| {
            match ch {
                '*' => result.push_str("[*]"),
                '?' => result.push_str("[?]"),
                '[' => result.push_str("[[]"),
                _ => result.extend(ch.to_lowercase()),
            }
            result
        })
}

fn configured_connection(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "mmap_size", 3_000_000_000_i64)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(conn)
}

fn populate_synthetic_database(
    db: &Database,
    entries: u64,
) -> (Vec<QueryExpectation>, f64, f64, Vec<f64>) {
    let mut expectations = QUERY_MATRIX
        .iter()
        .map(|_| QueryExpectation::new())
        .collect::<Vec<_>>();
    let mut generation_time = Duration::ZERO;
    let mut population_time = Duration::ZERO;
    let mut transaction_samples_ms =
        Vec::with_capacity(entries.div_ceil(INSERT_BATCH_SIZE) as usize);
    let mut conn = db.conn().expect("get synthetic population connection");

    for batch_start in (0..entries).step_by(INSERT_BATCH_SIZE as usize) {
        let batch_end = (batch_start + INSERT_BATCH_SIZE).min(entries);
        let generation_started = Instant::now();
        let batch = (batch_start..batch_end)
            .map(generated_entry)
            .collect::<Vec<_>>();
        for entry in &batch {
            for (query, expected) in QUERY_MATRIX.iter().zip(expectations.iter_mut()) {
                if query.matches(entry) {
                    expected.observe(entry);
                }
            }
        }
        generation_time += generation_started.elapsed();

        let population_started = Instant::now();
        let transaction = conn
            .transaction()
            .expect("begin synthetic population batch transaction");
        {
            let mut statement = transaction
                .prepare(INSERT_SYNTHETIC_ENTRY)
                .expect("prepare synthetic population insert");
            for entry in &batch {
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
                        "synthetic_global_search_benchmark",
                        1_700_000_000_i64 + entry.index as i64,
                    ])
                    .expect("insert synthetic entry with schema triggers enabled");
            }
        }
        transaction
            .commit()
            .expect("commit synthetic population batch");
        let batch_elapsed = population_started.elapsed();
        population_time += batch_elapsed;
        transaction_samples_ms.push(batch_elapsed.as_secs_f64() * 1_000.0);
        // Progress is emitted only for the manual exact-SQL diagnostic.
        // Keep its 30-minute fail-closed budget, but avoid opaque fixture timeouts.
        let progress_mode = env::var("ZC_GLOBAL_SEARCH_EXACT_SQL_DIAGNOSTIC_MODE").ok();
        if progress_mode.as_deref() == Some("exact_sql_1m_diagnostic")
            && (batch_start / 25_000 != batch_end / 25_000 || batch_end == entries)
        {
            eprintln!(
                "exact_sql_1m_fixture_progress committed_rows={batch_end} sqlite_population_ms={:.3}",
                population_time.as_secs_f64() * 1_000.0
            );
        }
    }

    drop(conn);
    (
        expectations,
        generation_time.as_secs_f64() * 1_000.0,
        population_time.as_secs_f64() * 1_000.0,
        transaction_samples_ms,
    )
}

fn assert_entry_triggers_enabled(conn: &Connection) -> Vec<String> {
    let mut statement = conn
        .prepare(
            "SELECT name FROM sqlite_schema WHERE type = 'trigger' AND tbl_name = 'global_entries' ORDER BY name",
        )
        .expect("prepare global entry trigger inspection");
    let triggers = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query global entry triggers")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect global entry triggers");
    for required in [
        "global_entries_ai",
        "global_entries_ad",
        "global_entries_au",
        "global_entries_count_ai",
        "global_entries_count_ad",
        "global_entries_count_au",
    ] {
        assert!(
            triggers.iter().any(|trigger| trigger == required),
            "required production trigger {required} must remain enabled in the synthetic benchmark"
        );
    }
    triggers
}

fn assert_search_results(
    conn: &Connection,
    query_case: QueryCase,
    expectation: &QueryExpectation,
    limit: u32,
    offset: u32,
    entries: u64,
) -> Vec<crate::global_index::models::GlobalSearchResult> {
    let results = search_global_entries_on_connection(conn, query_case.query, limit, offset)
        .unwrap_or_else(|error| {
            panic!(
                "search failed at entries={entries}, class={}, query={:?}: {error}",
                query_case.class, query_case.query
            )
        });
    let expected = expectation.expected_page(offset as usize, limit as usize);
    let actual = results
        .iter()
        .map(|result| result.id.as_str())
        .collect::<Vec<_>>();
    let unique_count = actual
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>()
        .len();

    assert_eq!(
        results.len(),
        expected.len(),
        "result count mismatch at entries={entries}, class={}, query={:?}",
        query_case.class,
        query_case.query
    );
    assert_eq!(
        actual, expected,
        "result order or IDs mismatch at entries={entries}, class={}, query={:?}",
        query_case.class, query_case.query
    );
    assert_eq!(
        unique_count,
        results.len(),
        "duplicate result IDs at entries={entries}, class={}, query={:?}",
        query_case.class,
        query_case.query
    );
    assert!(
        results.len() <= limit as usize,
        "result limit exceeded at entries={entries}, class={}, query={:?}",
        query_case.class,
        query_case.query
    );

    results
}

fn summarize_samples(samples: &[f64]) -> JsonValue {
    assert!(!samples.is_empty(), "latency samples must not be empty");
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    json!({
        "samples": sorted.len(),
        "min": round_ms(sorted[0]),
        "median": round_ms(quantile(&sorted, 0.50)),
        "p50": round_ms(quantile(&sorted, 0.50)),
        "p95": round_ms(quantile(&sorted, 0.95)),
        "p99": round_ms(quantile(&sorted, 0.99)),
        "max": round_ms(*sorted.last().expect("latency sample exists")),
        "quantile_method": "linear_interpolation_over_sorted_samples"
    })
}

fn quantile(sorted: &[f64], percentile: f64) -> f64 {
    let position = percentile * (sorted.len() - 1) as f64;
    let lower_index = position.floor() as usize;
    let upper_index = position.ceil() as usize;
    if lower_index == upper_index {
        sorted[lower_index]
    } else {
        let weight = position - lower_index as f64;
        sorted[lower_index] + (sorted[upper_index] - sorted[lower_index]) * weight
    }
}

fn round_ms(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}

fn benchmark_context(entries: u64) -> JsonValue {
    let command = |program: &str, args: &[&str]| {
        std::process::Command::new(program)
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    json!({
        "dataset_type": "synthetic_sqlite_global_index",
        "entries": entries,
        "source_sha": env::var("ZEN_CANVAS_BENCHMARK_SOURCE_SHA").ok(),
        "actual_head": command("git", &["rev-parse", "HEAD"]),
        "actual_tree": command("git", &["rev-parse", "HEAD^{tree}"]),
        "working_tree": command("git", &["status", "--short"]),
        "rustc": command("rustc", &["-vV"]),
        "debug_assertions": cfg!(debug_assertions),
        "desktop_runtime_feature": cfg!(feature = "desktop-runtime"),
        "rustflags": env::var("RUSTFLAGS").ok(),
        "sqlite_version": rusqlite::version(),
        "runner_os": env::consts::OS,
        "runner_arch": env::consts::ARCH,
        "github_run_id": env::var("GITHUB_RUN_ID").ok(),
        "timestamp_unix_seconds": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_secs(),
    })
}

fn emit_record(record: &JsonValue) {
    let line = serde_json::to_string(record).expect("serialize benchmark JSONL record");
    println!("{line}");
    if let Ok(path) = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT") {
        let output_path = PathBuf::from(path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).expect("create benchmark artifact directory");
        }
        let mut output = OpenOptions::new()
            .create(true)
            .append(true)
            .open(output_path)
            .expect("open benchmark JSONL artifact");
        writeln!(output, "{line}").expect("append benchmark JSONL record");
    }
}

fn sidecar_bytes(path: &Path, suffix: &str) -> u64 {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    fs::metadata(PathBuf::from(name))
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn database_metrics(conn: &Connection, path: &Path) -> JsonValue {
    let page_count: i64 = conn
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .expect("read SQLite page_count");
    let page_size: i64 = conn
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .expect("read SQLite page_size");
    let main_db_bytes = fs::metadata(path)
        .expect("read SQLite main database size")
        .len();
    let wal_bytes = sidecar_bytes(path, "-wal");
    let shm_bytes = sidecar_bytes(path, "-shm");
    json!({
        "main_db_bytes": main_db_bytes,
        "wal_bytes_at_capture": wal_bytes,
        "shm_bytes_at_capture": shm_bytes,
        "main_plus_wal_bytes": main_db_bytes + wal_bytes,
        "page_count": page_count,
        "page_size": page_size,
        "journal_mode": "WAL",
        "temporary_store": "MEMORY",
        "database_size_is_synthetic": true
    })
}

fn plan_record(
    conn: &Connection,
    context: &JsonValue,
    class: &str,
    query: &str,
    sql: String,
    values: Vec<SqlValue>,
) -> JsonValue {
    let explain = format!("EXPLAIN QUERY PLAN {sql}");
    let mut statement = conn
        .prepare(&explain)
        .expect("prepare production-tier query plan");
    let details = statement
        .query_map(params_from_iter(values.iter()), |row| {
            row.get::<_, String>(3)
        })
        .expect("execute production-tier query plan")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect production-tier query plan");
    let possible_full_scan = details
        .iter()
        .any(|detail| detail.contains("SCAN ge") || detail.contains("SCAN global_entries "));
    json!({
        "schema_version": 1,
        "record_type": "query_plan",
        "context": context,
        "query_class": class,
        "query": query,
        "details": details,
        "possible_full_table_scan": possible_full_scan,
        "plan_sql_source": "search.rs::diagnostic_search_tier_sql",
        "actual_sql": sql,
        "sql_blake3": blake3::hash(sql.as_bytes()).to_hex().to_string(),
        "sqlite_version": rusqlite::version(),
        "debug_assertions": cfg!(debug_assertions)
    })
}

fn capture_query_plans(conn: &Connection, context: &JsonValue) -> Vec<JsonValue> {
    use crate::global_index::search::diagnostic_search_tier_sql;
    let exact_sql = diagnostic_search_tier_sql("exact_name");
    let prefix_sql = diagnostic_search_tier_sql("name_prefix");
    let extension_sql = diagnostic_search_tier_sql("exact_extension");
    let extension_prefix_sql = diagnostic_search_tier_sql("extension_prefix");
    let (extension_probe_sql, extension_single_sql) =
        crate::global_index::search::diagnostic_extension_prefix_strategy_sqls();
    let fts_sql = diagnostic_search_tier_sql("fts");
    let punctuation_sql = diagnostic_search_tier_sql("punctuation_prefix");

    vec![
        plan_record(
            conn,
            context,
            "exact_basename",
            "Annual Budget 2026.xlsx",
            exact_sql,
            vec![
                SqlValue::Text("Annual Budget 2026.xlsx".to_string()),
                SqlValue::Integer(SEARCH_RESULT_LIMIT as i64),
            ],
        ),
        plan_record(
            conn,
            context,
            "name_prefix",
            "quarterly",
            prefix_sql,
            vec![
                SqlValue::Text("quarterly".to_string()),
                SqlValue::Text("quarterly*".to_string()),
                SqlValue::Integer(SEARCH_RESULT_LIMIT as i64),
            ],
        ),
        plan_record(
            conn,
            context,
            "extension_exact",
            "pdf",
            extension_sql,
            vec![
                SqlValue::Text("pdf".to_string()),
                SqlValue::Integer(SEARCH_RESULT_LIMIT as i64),
            ],
        ),
        plan_record(
            conn,
            context,
            "extension_prefix",
            "jp",
            extension_prefix_sql,
            vec![
                SqlValue::Text("jp".to_string()),
                SqlValue::Text("jp*".to_string()),
                SqlValue::Integer(SEARCH_RESULT_LIMIT as i64),
            ],
        ),
        // Extension Prefix dispatch can execute discovery, a single-key exact
        // query, or the original fallback. Preserve plans for all three paths.
        plan_record(
            conn,
            context,
            "extension_prefix_discovery",
            "jp",
            extension_probe_sql.to_string(),
            vec![
                SqlValue::Text("jp".to_string()),
                SqlValue::Text("jp*".to_string()),
            ],
        ),
        plan_record(
            conn,
            context,
            "extension_prefix_single_key",
            "jp",
            extension_single_sql,
            vec![
                SqlValue::Text("jp".to_string()),
                SqlValue::Text("jp*".to_string()),
                SqlValue::Integer(SEARCH_RESULT_LIMIT as i64),
                SqlValue::Text("jpg".to_string()),
            ],
        ),
        plan_record(
            conn,
            context,
            "fts_substring_report",
            "report",
            fts_sql,
            vec![
                SqlValue::Text("\"report\"".to_string()),
                SqlValue::Integer(SEARCH_RESULT_LIMIT as i64),
            ],
        ),
        plan_record(
            conn,
            context,
            "punctuation_prefix",
            "final-v2",
            punctuation_sql,
            vec![
                SqlValue::Text("final-v2*".to_string()),
                SqlValue::Integer(SEARCH_RESULT_LIMIT as i64),
            ],
        ),
    ]
}

fn query_record(
    context: &JsonValue,
    query_case: QueryCase,
    expectation: &QueryExpectation,
    warm_samples: &[f64],
    reopened_samples: &[f64],
) -> JsonValue {
    let first = expectation.expected_first();
    json!({
        "schema_version": 1,
        "record_type": "query",
        "context": context,
        "query_class": query_case.class,
        "query": query_case.query,
        "expected_match_count": expectation.total_matches,
        "bounded_candidate_count": expectation.bounded_match_count(),
        "expected_top_result": first.map(|entry| json!({"id": entry.id, "name": entry.name})),
        "result_limit": SEARCH_RESULT_LIMIT,
        "correct": true,
        "warm": {
            "connection_mode": "database_pool_entrypoint_repeated",
            "connection_checkout_included": true,
            "warmup_samples": WARMUP_SAMPLES,
            "latency_ms": summarize_samples(warm_samples)
        },
        "reopened_connection": {
            "connection_mode": "new_sqlite_connection_per_sample",
            "connection_open_included_in_latency": false,
            "samples": reopened_samples.len(),
            "latency_ms": summarize_samples(reopened_samples),
            "os_page_cache_cleared": false
        }
    })
}

fn assert_candidate_and_pagination_contract(
    conn: &Connection,
    context: &JsonValue,
    entries: u64,
    query_case: QueryCase,
    expectation: &QueryExpectation,
) {
    assert!(
        expectation.total_matches > SEARCH_CANDIDATE_LIMIT as u64,
        "fixture must exceed the candidate window for class={}",
        query_case.class
    );
    let first_page = assert_search_results(conn, query_case, expectation, 40, 0, entries);
    let second_page = assert_search_results(conn, query_case, expectation, 40, 40, entries);
    let first_ids = first_page
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<Vec<_>>();
    let second_ids = second_page
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<Vec<_>>();
    assert!(first_ids.iter().all(|id| !second_ids.contains(id)));
    let mut joined_ids = first_ids;
    joined_ids.extend(second_ids);
    assert_eq!(joined_ids, expectation.expected_page(0, 80));

    assert_search_results(
        conn,
        query_case,
        expectation,
        SEARCH_RESULT_LIMIT,
        (SEARCH_CANDIDATE_LIMIT - 6) as u32,
        entries,
    );
    let after_candidate_cap = search_global_entries_on_connection(
        conn,
        query_case.query,
        SEARCH_RESULT_LIMIT,
        SEARCH_CANDIDATE_LIMIT as u32,
    )
    .expect("search after bounded candidate window");
    assert!(
        after_candidate_cap.is_empty(),
        "offset at the candidate cap must be empty at entries={entries}, class={}",
        query_case.class
    );
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "pagination_and_candidate_window",
        "context": context,
        "query_class": query_case.class,
        "query": query_case.query,
        "candidate_limit": SEARCH_CANDIDATE_LIMIT,
        "expected_match_count": expectation.total_matches,
        "page_size": 40,
        "page_boundary_result_count": expectation.expected_page(SEARCH_CANDIDATE_LIMIT - 6, SEARCH_RESULT_LIMIT as usize).len(),
        "offset_at_candidate_cap_empty": true,
        "concatenated_pages_match_first_80": true,
        "duplicate_ids_across_pages": false
    }));
}

fn remove_database_files(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(sidecar_path(path, "-wal"));
    let _ = fs::remove_file(sidecar_path(path, "-shm"));
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

#[test]
#[ignore = "synthetic Global Search baseline; set ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES to a supported scale"]
fn global_search_synthetic_benchmark_baseline() {
    let entries = configured_entries();
    let context = benchmark_context(entries);
    let output_path = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT").ok();
    if let Some(path) = &output_path {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create benchmark artifact directory");
        }
        fs::write(path, "").expect("truncate benchmark JSONL artifact");
    }

    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open synthetic Global Index database");
    db.upsert_global_volume(&test_volume())
        .expect("insert synthetic benchmark volume");

    let (expectations, generation_ms, population_ms, transaction_samples_ms) =
        populate_synthetic_database(&db, entries);
    assert_eq!(expectations.len(), QUERY_MATRIX.len());

    let population_conn = db.conn().expect("inspect synthetic population result");
    let trigger_names = assert_entry_triggers_enabled(&population_conn);
    let total_entries: i64 = population_conn
        .query_row(
            "SELECT COUNT(*) FROM global_entries WHERE volume_id = 'gv_test'",
            [],
            |row| row.get(0),
        )
        .expect("count synthetic Global Index entries");
    let volume_entry_count: i64 = population_conn
        .query_row(
            "SELECT entry_count FROM global_volumes WHERE id = 'gv_test'",
            [],
            |row| row.get(0),
        )
        .expect("read count-trigger-maintained volume entry_count");
    assert_eq!(total_entries, entries as i64);
    assert_eq!(volume_entry_count, entries as i64);

    let fts_report_count: i64 = population_conn
        .query_row(
            "SELECT COUNT(*) FROM global_entries_fts WHERE global_entries_fts MATCH '\"report\"'",
            [],
            |row| row.get(0),
        )
        .expect("verify FTS insert triggers populated the trigram index");
    assert_eq!(
        fts_report_count as u64, expectations[3].total_matches,
        "FTS trigger index should contain every synthetic report row"
    );
    let db_metrics = database_metrics(&population_conn, &path);
    drop(population_conn);

    emit_record(&json!({
        "schema_version": 1,
        "record_type": "dataset",
        "context": context,
        "fixture": {
            "entry_count": entries,
            "path_data_is_synthetic_only": true,
            "name_and_path_profiles": ["english", "mixed_case", "numeric", "spaces", "underscore", "hyphen", "dot", "chinese", "unicode_nfc", "unicode_nfd", "duplicate_basename", "long_basename", "high_fanout_prefix"],
            "path_profiles": ["shallow_desktop", "downloads", "pictures", "documents", "deep_reference", "many_sibling_directories", "developer_project"],
            "extensions": ["pdf", "docx", "xlsx", "pptx", "txt", "md", "jpg", "png", "mp4", "zip", "exe", "app", "ts", "js", "rs", "json"],
            "modified_time": "one-second monotonic distribution from a fixed epoch",
            "file_size": "deterministic modulo distribution from 1 byte through 5,000,000,000 bytes"
        },
        "dataset_generation_ms": round_ms(generation_ms),
        "sqlite_population_ms": round_ms(population_ms),
        "population_transaction_batch_size": INSERT_BATCH_SIZE,
        "population_batch_latency_ms": summarize_samples(&transaction_samples_ms),
        "production_entry_triggers_preserved": trigger_names,
        "fts_report_rows_verified": fts_report_count,
        "database": db_metrics,
        "timing_scope": "synthetic SQLite population only; excludes filesystem scan and platform indexing"
    }));

    let inspection_connection = configured_connection(&path).expect("open query inspection reader");
    for (query_case, expectation) in QUERY_MATRIX.iter().copied().zip(expectations.iter()) {
        let actual_count = actual_match_count(&inspection_connection, query_case);
        assert_eq!(
            actual_count as u64, expectation.total_matches,
            "independent count oracle mismatch at entries={entries}, class={}, query={:?}",
            query_case.class, query_case.query
        );
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "count_oracle",
            "context": context,
            "query_class": query_case.class,
            "query": query_case.query,
            "expected_match_count": expectation.total_matches,
            "sqlite_match_count": actual_count,
            "correct": true
        }));
    }

    let plan_records = capture_query_plans(&inspection_connection, &context);
    for record in plan_records {
        emit_record(&record);
    }

    let common_prefix_case = QUERY_MATRIX
        .iter()
        .position(|query| query.class == "common_prefix_high_fanout")
        .expect("common-prefix query case");
    assert_candidate_and_pagination_contract(
        &inspection_connection,
        &context,
        entries,
        QUERY_MATRIX[common_prefix_case],
        &expectations[common_prefix_case],
    );

    let mut performance_regressions = Vec::new();
    for (query_case, expectation) in QUERY_MATRIX.iter().copied().zip(expectations.iter()) {
        let expected_first_page = expectation.expected_page(0, SEARCH_RESULT_LIMIT as usize);
        drop(assert_search_results(
            &inspection_connection,
            query_case,
            expectation,
            SEARCH_RESULT_LIMIT,
            0,
            entries,
        ));
        for _ in 0..WARMUP_SAMPLES {
            let results = db
                .search_global_entries(query_case.query, SEARCH_RESULT_LIMIT, 0)
                .expect("warm synthetic Global Search through database entrypoint");
            assert_eq!(
                results
                    .iter()
                    .map(|result| result.id.as_str())
                    .collect::<Vec<_>>(),
                expected_first_page,
                "warmup results differ from expected IDs for class={}",
                query_case.class
            );
        }

        let mut warm_samples = Vec::with_capacity(WARM_SAMPLES);
        for _ in 0..WARM_SAMPLES {
            let started = Instant::now();
            let results = db
                .search_global_entries(query_case.query, SEARCH_RESULT_LIMIT, 0)
                .expect("run warm synthetic Global Search through database entrypoint");
            warm_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
            let actual_ids = results
                .iter()
                .map(|result| result.id.as_str())
                .collect::<Vec<_>>();
            assert_eq!(actual_ids, expected_first_page);
            assert_eq!(
                actual_ids
                    .iter()
                    .copied()
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
                results.len()
            );
            assert!(results.len() <= SEARCH_RESULT_LIMIT as usize);
        }

        let mut reopened_samples = Vec::with_capacity(REOPENED_CONNECTION_SAMPLES);
        for _ in 0..REOPENED_CONNECTION_SAMPLES {
            let reopened = configured_connection(&path)
                .expect("reopen SQLite connection for reopened-connection sample");
            let started = Instant::now();
            let results = search_global_entries_on_connection(
                &reopened,
                query_case.query,
                SEARCH_RESULT_LIMIT,
                0,
            )
            .expect("run reopened-connection synthetic Global Search query");
            reopened_samples.push(started.elapsed().as_secs_f64() * 1_000.0);
            let actual_ids = results
                .iter()
                .map(|result| result.id.as_str())
                .collect::<Vec<_>>();
            assert_eq!(actual_ids, expected_first_page);
            assert_eq!(
                actual_ids
                    .iter()
                    .copied()
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
                results.len()
            );
            assert!(results.len() <= SEARCH_RESULT_LIMIT as usize);
            drop(reopened);
        }

        emit_record(&query_record(
            &context,
            query_case,
            expectation,
            &warm_samples,
            &reopened_samples,
        ));

        if entries <= 1_000_000 {
            let mut sorted_warm_samples = warm_samples.clone();
            sorted_warm_samples.sort_by(f64::total_cmp);
            let warm_p95_ms = quantile(&sorted_warm_samples, 0.95);
            let passed = warm_p95_ms <= WARM_QUERY_P95_LIMIT_MS;
            emit_record(&json!({
                "schema_version": 1,
                "record_type": "performance_gate",
                "context": context,
                "query_class": query_case.class,
                "entries": entries,
                "warm_p95_ms": round_ms(warm_p95_ms),
                "limit_ms": WARM_QUERY_P95_LIMIT_MS,
                "passed": passed
            }));
            if !passed {
                performance_regressions.push(format!(
                    "{}: {:.3}ms > {:.3}ms",
                    query_case.class, warm_p95_ms, WARM_QUERY_P95_LIMIT_MS
                ));
            }
        }
    }

    drop(inspection_connection);
    drop(db);
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "performance_gate_summary",
        "context": context,
        "passed": performance_regressions.is_empty(),
        "regressions": &performance_regressions
    }));
    assert!(
        performance_regressions.is_empty(),
        "warm Global Search p95 exceeded the historical {WARM_QUERY_P95_LIMIT_MS:.3}ms budget at entries={entries}: {}",
        performance_regressions.join("; ")
    );
}

#[test]
fn global_search_unicode_case_and_canonical_form_evidence() {
    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open Unicode Global Search test database");
    db.upsert_global_volume(&test_volume())
        .expect("insert Unicode test volume");

    let nfc_name = "Café Résumé 2026.pdf";
    let nfd_name = "Cafe\u{301} Re\u{301}sume\u{301} 2026.pdf";
    let mut nfc = test_entry(&format!(r"C:\Global\Unicode\{nfc_name}"), nfc_name, false);
    nfc.platform_file_id = "unicode-nfc".to_string();
    nfc.extension = "pdf".to_string();
    let nfc_id = nfc.entry_id();

    let mut nfd = test_entry(&format!(r"C:\Global\Unicode\{nfd_name}"), nfd_name, false);
    nfd.platform_file_id = "unicode-nfd".to_string();
    nfd.extension = "pdf".to_string();
    let nfd_id = nfd.entry_id();
    db.upsert_global_entries_batch(&[nfc, nfd])
        .expect("insert NFC and NFD Unicode entries");

    let mixed_case_query = "CAFÉ RÉSUMÉ 2026.PDF";
    let expected_rust_lowercase = mixed_case_query.to_lowercase();
    let conn = db.conn().expect("inspect SQLite Unicode lower behavior");
    let sqlite_lowercase: String = conn
        .query_row("SELECT lower(?1)", [mixed_case_query], |row| row.get(0))
        .expect("lower mixed-case Unicode query with SQLite");
    assert_ne!(
        sqlite_lowercase, expected_rust_lowercase,
        "this fixture must exercise the current SQLite/Rust Unicode lowercase difference"
    );
    drop(conn);

    let mixed_case_results = db
        .search_global_entries(mixed_case_query, 20, 0)
        .expect("search with mixed-case accented query");
    assert_eq!(mixed_case_results.len(), 1);
    assert_eq!(mixed_case_results[0].id, nfc_id);

    let nfc_results = db
        .search_global_entries(nfc_name, 20, 0)
        .expect("search NFC filename form");
    assert_eq!(nfc_results.len(), 1);
    assert_eq!(nfc_results[0].id, nfc_id);

    let nfd_results = db
        .search_global_entries(nfd_name, 20, 0)
        .expect("search NFD filename form");
    assert_eq!(nfd_results.len(), 1);
    assert_eq!(nfd_results[0].id, nfd_id);

    assert!(!nfc_results.iter().any(|result| result.id == nfd_id));
    assert!(!nfd_results.iter().any(|result| result.id == nfc_id));
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "unicode_search_semantics",
        "sqlite_lowercase": sqlite_lowercase,
        "rust_lowercase": expected_rust_lowercase,
        "mixed_case_query_result_ids": mixed_case_results.iter().map(|result| &result.id).collect::<Vec<_>>(),
        "nfc_query_result_ids": nfc_results.iter().map(|result| &result.id).collect::<Vec<_>>(),
        "nfd_query_result_ids": nfd_results.iter().map(|result| &result.id).collect::<Vec<_>>(),
        "unicode_canonical_forms_cross_match": false,
        "correct": true
    }));
    drop(db);
}
#[path = "filename_first_candidate_benchmark.rs"]
mod filename_first_candidate_benchmark;
