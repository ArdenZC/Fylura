use std::{
    env, fs,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use fylura_tauri::{
    db::Database, FileLibraryFilter, InsertFileRequest, LibraryFilter, LibraryScope,
};
use rusqlite::{params, Connection, ToSql};

const DEFAULT_ROWS: usize = 100_000;
const DEFAULT_P95_MS: f64 = 1_000.0;
const QUERY_RUNS: usize = 1;
const KEYWORD_STRIDE: usize = 1_000;
const SEARCH_LIMIT: i64 = 50;
const OPTIMIZE_PROBE_QUERY: &str = "screenshot";
const BENCH_QUERIES: [&str; 8] = [
    "resume",
    "invoice",
    "screenshot",
    "project",
    "身份证",
    "pdf",
    "report",
    "archive",
];
const COUNT_SQL: &str = r#"
SELECT COUNT(*)
FROM files_fts
JOIN files AS f ON f.rowid = files_fts.rowid
WHERE files_fts MATCH ?1
  AND f.is_stale = 0
"#;
const SEARCH_SQL: &str = r#"
SELECT
    f.id,
    f.path,
    f.name,
    f.extension,
    f.size,
    f.mtime,
    f.is_dir,
    f.state_code,
    bm25(files_fts, 6.0, 1.5) AS rank
FROM files_fts
JOIN files AS f ON f.rowid = files_fts.rowid
WHERE files_fts MATCH ?1
  AND f.is_stale = 0
ORDER BY rank ASC, f.mtime DESC, length(f.path) ASC
LIMIT ?2
"#;

struct QueryDiagnostics {
    label: String,
    query: String,
    match_count: i64,
    count_ms: f64,
    search_ms: f64,
    total_ms: f64,
    rows: usize,
}

struct PagedDiagnostics {
    label: &'static str,
    query: &'static str,
    total: i64,
    elapsed_ms: f64,
    rows: usize,
}

#[test]
fn fts_query_diagnostics_measure_count_and_search() {
    let temp_dir = benchmark_temp_dir();
    fs::create_dir_all(&temp_dir).expect("create benchmark temp dir");
    let db_path = temp_dir.join("zen-canvas-fts-diagnostics.sqlite3");
    let db = Database::open(&db_path).expect("open diagnostics database");
    db.insert_files(&[
        InsertFileRequest {
            id: "/benchmark/resume/resume_alpha.pdf".to_string(),
            path: "/benchmark/resume/resume_alpha.pdf".to_string(),
            name: "resume_alpha.pdf".to_string(),
            extension: "pdf".to_string(),
            size: 2_048,
            mtime: 1_900_000_001,
            ctime: 1_900_000_001,
            is_dir: false,
            state_code: 0,
        },
        InsertFileRequest {
            id: "/benchmark/resume/resume_beta.pdf".to_string(),
            path: "/benchmark/resume/resume_beta.pdf".to_string(),
            name: "resume_beta.pdf".to_string(),
            extension: "pdf".to_string(),
            size: 4_096,
            mtime: 1_900_000_002,
            ctime: 1_900_000_002,
            is_dir: false,
            state_code: 0,
        },
    ])
    .expect("insert diagnostics rows");

    let diagnostics = measure_query(&db, "resume", false).expect("measure query");

    drop(db);
    let _ = fs::remove_dir_all(&temp_dir);

    assert_eq!(diagnostics.label, "english_search");
    assert_eq!(diagnostics.query, "resume");
    assert_eq!(diagnostics.match_count, 2);
    assert_eq!(diagnostics.rows, 2);
    assert!(diagnostics.count_ms >= 0.0);
    assert!(diagnostics.search_ms >= 0.0);
    assert!(diagnostics.total_ms >= diagnostics.search_ms);
}

#[test]
#[ignore]
fn fts_benchmark_100k() {
    let requested_rows = env_usize("ZC_BENCH_ROWS", DEFAULT_ROWS);
    let rows = requested_rows.max(BENCH_QUERIES.len());
    let threshold_ms = env_f64("ZC_BENCH_P95_MS", DEFAULT_P95_MS);
    let explain = env_bool("ZC_BENCH_EXPLAIN");
    let temp_dir = benchmark_temp_dir();
    fs::create_dir_all(&temp_dir).expect("create benchmark temp dir");
    let db_path = temp_dir.join("zen-canvas-fts-benchmark.sqlite3");
    let db = Database::open(&db_path).expect("open benchmark database");

    let insert_start = Instant::now();
    insert_benchmark_rows(&db, rows);
    mark_benchmark_filter_rows(&db);
    let insert_elapsed = insert_start.elapsed();

    println!(
        "SQLite/FTS benchmark seeded rows={rows} insert_ms={:.3}",
        duration_ms(insert_elapsed)
    );

    print_distribution(rows, threshold_ms, explain);
    let full_profile = env::var("ZC_FTS_FULL_PROFILE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(true);
    let pre_optimize = if full_profile {
        let diagnostics = measure_query(&db, OPTIMIZE_PROBE_QUERY, false)
            .expect("measure pre-optimize probe query");
        print_diagnostics_line("pre_optimize_probe", 1, &diagnostics);
        Some(diagnostics)
    } else {
        None
    };
    let optimize_ms = db.optimize_search_index().expect("post-write optimize");
    println!("[fts-bench] post_write_optimize_ms={optimize_ms}");
    if full_profile {
        let post_optimize = measure_query(&db, OPTIMIZE_PROBE_QUERY, false)
            .expect("measure post-optimize probe query");
        print_diagnostics_line("post_optimize_probe", 1, &post_optimize);
        assert!(
            post_optimize.search_ms
                < pre_optimize
                    .as_ref()
                    .expect("full profile pre-optimize probe")
                    .search_ms,
            "post-write optimize should improve {OPTIMIZE_PROBE_QUERY:?} search latency: before {:.3}ms after {:.3}ms",
            pre_optimize
                .as_ref()
                .expect("full profile pre-optimize probe")
                .search_ms,
            post_optimize.search_ms
        );
    }

    let mut search_timings = Vec::with_capacity(BENCH_QUERIES.len() * QUERY_RUNS);
    let mut count_timings = Vec::with_capacity(BENCH_QUERIES.len() * QUERY_RUNS);
    let mut total_timings = Vec::with_capacity(BENCH_QUERIES.len() * QUERY_RUNS);
    for query in BENCH_QUERIES {
        for run in 1..=QUERY_RUNS {
            let diagnostics = measure_query(&db, query, explain)
                .unwrap_or_else(|error| panic!("query {query:?} failed: {error}"));
            assert!(
                diagnostics.rows > 0,
                "benchmark query {query:?} run {run} should return results"
            );

            search_timings.push(diagnostics.search_ms);
            count_timings.push(diagnostics.count_ms);
            total_timings.push(diagnostics.total_ms);
            print_diagnostics_line("query", run, &diagnostics);
        }
    }

    let search_p50 = percentile(&search_timings, 0.50);
    let search_p95 = percentile(&search_timings, 0.95);
    let search_max = search_timings.iter().copied().fold(0.0_f64, f64::max);
    let count_p95 = percentile(&count_timings, 0.95);
    let total_p95 = percentile(&total_timings, 0.95);
    println!(
        "SQLite/FTS benchmark summary rows={rows} samples={} search_p50_ms={search_p50:.3} search_p95_ms={search_p95:.3} search_max_ms={search_max:.3} count_p95_ms={count_p95:.3} total_p95_ms={total_p95:.3} threshold_search_p95_ms={threshold_ms:.3}",
        search_timings.len()
    );

    for diagnostics in measure_required_paged_scenarios(&db) {
        assert!(
            diagnostics.rows > 0,
            "benchmark scenario {} should return rows",
            diagnostics.label
        );
        println!(
            "[fts-bench] label={} query={} total={} elapsed_ms={:.3} rows={}",
            diagnostics.label,
            diagnostics.query,
            diagnostics.total,
            diagnostics.elapsed_ms,
            diagnostics.rows
        );
    }

    drop(db);
    let _ = fs::remove_dir_all(&temp_dir);

    assert!(
        search_p95 <= threshold_ms,
        "SQLite/FTS benchmark search p95 {search_p95:.3}ms exceeded threshold {threshold_ms:.3}ms for {rows} rows"
    );
}

fn print_diagnostics_line(label: &str, run: usize, diagnostics: &QueryDiagnostics) {
    println!(
        "[fts-bench] label={label} scenario={} query={} run={run} matches={} count_ms={:.3} search_ms={:.3} total_ms={:.3} rows={}",
        diagnostics.label,
        diagnostics.query,
        diagnostics.match_count,
        diagnostics.count_ms,
        diagnostics.search_ms,
        diagnostics.total_ms,
        diagnostics.rows
    );
}

fn measure_query(
    db: &Database,
    query: &str,
    explain: bool,
) -> Result<QueryDiagnostics, rusqlite::Error> {
    let fts_query = build_benchmark_fts_query(query).unwrap_or_default();
    let conn = Connection::open(db.path())?;

    if explain {
        let count_params: [&dyn ToSql; 1] = [&fts_query];
        let search_params: [&dyn ToSql; 2] = [&fts_query, &SEARCH_LIMIT];
        print_query_plan(&conn, query, "count", COUNT_SQL, &count_params)?;
        print_query_plan(&conn, query, "search", SEARCH_SQL, &search_params)?;
    }

    let total_start = Instant::now();

    let count_start = Instant::now();
    let match_count = conn.query_row(COUNT_SQL, params![fts_query.as_str()], |row| row.get(0))?;
    let count_ms = duration_ms(count_start.elapsed());

    let search_start = Instant::now();
    let mut stmt = conn.prepare(SEARCH_SQL)?;
    let rows = stmt
        .query_map(params![fts_query.as_str(), SEARCH_LIMIT], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let search_ms = duration_ms(search_start.elapsed());

    Ok(QueryDiagnostics {
        label: search_scenario_label(query).to_string(),
        query: query.to_string(),
        match_count,
        count_ms,
        search_ms,
        total_ms: duration_ms(total_start.elapsed()),
        rows: rows.len(),
    })
}

fn search_scenario_label(query: &str) -> &'static str {
    if query == "身份证" {
        "cjk_search"
    } else if query == "pdf" {
        "extension_search"
    } else {
        "english_search"
    }
}

fn insert_benchmark_rows(db: &Database, rows: usize) {
    let batch = (0..rows).map(benchmark_file).collect::<Vec<_>>();
    db.insert_files(&batch)
        .unwrap_or_else(|error| panic!("insert benchmark rows 0..{rows} failed: {error}"));
}

fn mark_benchmark_filter_rows(db: &Database) {
    let conn = Connection::open(db.path()).expect("open benchmark database for filter setup");
    conn.execute(
        "UPDATE files SET lifecycle = 'Archive', suggested_action = 'Archive' WHERE extension = 'zip'",
        [],
    )
    .expect("mark archive benchmark rows");
    conn.execute(
        "UPDATE files SET suggested_action = 'Review', requires_confirmation = 1 WHERE path LIKE '/benchmark/invoice/%'",
        [],
    )
    .expect("mark review benchmark rows");
    conn.execute(
        "UPDATE files SET lifecycle = 'Active' WHERE path LIKE '/benchmark/project/%'",
        [],
    )
    .expect("mark active benchmark rows");
}

fn measure_required_paged_scenarios(db: &Database) -> Vec<PagedDiagnostics> {
    let scenarios = [
        (
            "scope_query",
            None,
            LibraryScope::Roots {
                roots: vec!["/benchmark/project".to_string()],
            },
            None,
        ),
        (
            "filter_query",
            None,
            LibraryScope::All,
            Some(FileLibraryFilter {
                library_filter: Some(LibraryFilter::Review),
            }),
        ),
        (
            "query_filter_query",
            Some("invoice"),
            LibraryScope::All,
            Some(FileLibraryFilter {
                library_filter: Some(LibraryFilter::Review),
            }),
        ),
    ];

    scenarios
        .into_iter()
        .map(|(label, query, scope, filter)| {
            let started = Instant::now();
            let page = db
                .get_paged_files_in_scope_with_filter(
                    Some(50),
                    Some(0),
                    query,
                    &scope,
                    filter.as_ref(),
                )
                .unwrap_or_else(|error| panic!("paged benchmark scenario {label} failed: {error}"));
            PagedDiagnostics {
                label,
                query: query.unwrap_or("*"),
                total: page.total,
                elapsed_ms: duration_ms(started.elapsed()),
                rows: page.files.len(),
            }
        })
        .collect()
}

fn benchmark_file(index: usize) -> InsertFileRequest {
    let (keyword, extension) = benchmark_keyword_extension(index);
    let name = format!("{keyword}_sample_{index:06}.{extension}");
    let path = format!("/benchmark/{keyword}/{}", name);

    InsertFileRequest {
        id: path.clone(),
        path,
        name,
        extension: extension.to_string(),
        size: 1_024 + i64::try_from(index % 65_536).unwrap_or(0),
        mtime: 1_900_000_000 + i64::try_from(index).unwrap_or(i64::MAX),
        ctime: 1_899_000_000 + i64::try_from(index).unwrap_or(i64::MAX),
        is_dir: false,
        state_code: 0,
    }
}

fn benchmark_keyword_extension(index: usize) -> (&'static str, &'static str) {
    match index % KEYWORD_STRIDE {
        0 => ("resume", "pdf"),
        1 => ("invoice", "pdf"),
        2 => ("screenshot", "png"),
        3 => ("project", "md"),
        4 => ("身份证", "jpg"),
        5 => ("report", "docx"),
        6 => ("archive", "zip"),
        bucket => match bucket % 6 {
            0 => ("document", "txt"),
            1 => ("notes", "md"),
            2 => ("photo", "jpg"),
            3 => ("draft", "docx"),
            4 => ("export", "csv"),
            _ => ("media", "mp4"),
        },
    }
}

fn print_distribution(rows: usize, threshold_ms: f64, explain: bool) {
    println!(
        "[fts-bench] distribution total_rows={rows} threshold_p95_ms={threshold_ms:.3} query_runs={QUERY_RUNS} explain={explain}"
    );
    println!("[fts-bench] queries={}", BENCH_QUERIES.join(","));
    for query in BENCH_QUERIES {
        println!(
            "[fts-bench] distribution query={query} expected_matches={}",
            expected_keyword_matches(rows, query)
        );
    }
}

fn expected_keyword_matches(rows: usize, query: &str) -> usize {
    (0..rows)
        .filter(|index| {
            let (keyword, extension) = benchmark_keyword_extension(*index);
            keyword == query || extension == query
        })
        .count()
}

// Keep this copy aligned with db.rs build_fts_query. The production helper is private; this
// integration benchmark duplicates it so the diagnostic SQL matches Database::search_files.
fn build_benchmark_fts_query(input: &str) -> Option<String> {
    let phrases = input
        .split_whitespace()
        .filter(|phrase| !phrase.is_empty())
        .take(12)
        .map(quote_benchmark_fts_phrase)
        .collect::<Vec<_>>();

    if phrases.is_empty() {
        None
    } else {
        Some(phrases.join(" AND "))
    }
}

fn quote_benchmark_fts_phrase(phrase: &str) -> String {
    format!("\"{}\"", phrase.replace('"', "\"\""))
}

fn print_query_plan(
    conn: &Connection,
    query: &str,
    label: &str,
    sql: &str,
    params: &[&dyn ToSql],
) -> Result<(), rusqlite::Error> {
    let explain_sql = format!("EXPLAIN QUERY PLAN {sql}");
    let mut stmt = conn.prepare(&explain_sql)?;
    let rows = stmt.query_map(params, |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;

    println!("[fts-bench][explain] query={query} {label}_plan:");
    for row in rows {
        let (id, parent, not_used, detail) = row?;
        println!(
            "[fts-bench][explain] query={query} {label} id={id} parent={parent} not_used={not_used} detail={detail}"
        );
    }

    Ok(())
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    assert!(!values.is_empty(), "percentile requires values");
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));
    let index = ((sorted.len() as f64 * percentile).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[index]
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_f64(name: &str, default: f64) -> f64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(default)
}

fn env_bool(name: &str) -> bool {
    env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn benchmark_temp_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    env::temp_dir().join(format!("zen-canvas-fts-benchmark-{nonce}"))
}
