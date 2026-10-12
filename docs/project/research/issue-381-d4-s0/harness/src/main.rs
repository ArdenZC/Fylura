#![recursion_limit = "256"]

use rusqlite::{params, Connection};
use std::{
    collections::BTreeSet,
    env, fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

#[derive(Debug, Error)]
enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("validation: {0}")]
    Validation(String),
}

fn current_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_secs() as i64
}

fn trim_trailing_path_separators(path: &str) -> String {
    path.trim_end_matches(['/', '\\']).to_string()
}

fn normalize_path_text(path: &str) -> String {
    path.replace('\\', "/")
}

mod settings {
    use super::DbError;
    pub const APP_SETTINGS_KEY: &str = "app_settings_v1";
    pub fn default_settings_json() -> Result<String, DbError> {
        Ok(r#"{"defaultScanFolders":[]}"#.to_string())
    }
}

mod db {
    use super::{
        current_unix_seconds, normalize_path_text, trim_trailing_path_separators, DbError,
    };
    use rusqlite::Connection;
    use serde_json;

    #[path = "production_schema/schema.rs"]
    mod schema;
    #[path = "production_schema/schema_automation.rs"]
    mod schema_automation;
    #[path = "production_schema/schema_automation_triggers.rs"]
    mod schema_automation_triggers;

    pub fn initialize(path: &str) -> Result<(), DbError> {
        let conn = Connection::open(path)?;
        schema::migrate(&conn)
    }
}

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

const ROOT_ID: &str = "d2-root";
const SESSION_ID: &str = "d2-session";
const RUN_ID: &str = "d2-run";
const TAG_ID: &str = "d2-tag";
const GROUP_ID: &str = "d2-duplicate-group";
const DEDUPE_RUN_ID: &str = "d2-dedupe-run";
const ANALYSIS_RUN_ID: &str = "d2-analysis-run";
const STARTED_AT: i64 = 100;
const BUSY_TIMEOUT_MS: u64 = 5_000;
const BRANCH_COUNT: usize = 128;

#[derive(Clone, Debug)]
struct Candidate {
    id: String,
    path: String,
}

#[derive(Clone, Copy, Debug)]
enum Variant {
    A,
    B,
    C400,
    Cbulk,
    CbulkFull,
}

impl Variant {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "A" => Some(Self::A),
            "B" => Some(Self::B),
            "C400" => Some(Self::C400),
            "C" => Some(Self::Cbulk),
            "Cfull" => Some(Self::CbulkFull),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::A => "A_full_fts_400_ids",
            Self::B => "B_scoped_fts_400_ids",
            Self::C400 => "C_fenced_preflight_400_ids",
            Self::Cbulk => "C_fenced_preflight_set_update",
            Self::CbulkFull => "C_fenced_preflight_set_update_full_fts",
        }
    }

    fn conditional_fts(self) -> bool {
        !matches!(self, Self::A | Self::CbulkFull)
    }

    fn preflight(self) -> bool {
        matches!(self, Self::C400 | Self::Cbulk | Self::CbulkFull)
    }

    fn set_update(self) -> bool {
        matches!(self, Self::Cbulk | Self::CbulkFull)
    }
}

#[derive(Clone, Copy, Debug)]
struct RootIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

#[derive(Default)]
struct FsMetrics {
    leaf_checks: usize,
    distinct_directories: usize,
    elapsed_ms: f64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.get(1).map(String::as_str) == Some("--crash-d4-pointer") {
        return crash_d4_pointer(&args[2], &args[3]);
    }
    if args.get(1).map(String::as_str) == Some("--d4-query-diagnostic") {
        return d4_query_diagnostic(&args[2], &args[3]);
    }
    if args.get(1).map(String::as_str) == Some("--d4-s0") {
        let out_dir = env::var_os("ZC_ISSUE381_D4_OUTPUT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target/issue-381-d4-s0-output"));
        return run_d4_s0(&out_dir, &[10_000, 100_000], true);
    }
    if args.get(1).map(String::as_str) == Some("--d4-s0-10k") {
        let out_dir = env::var_os("ZC_ISSUE381_D4_OUTPUT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target/issue-381-d4-s0-output"));
        return run_d4_s0(&out_dir, &[10_000], false);
    }
    if args.get(1).map(String::as_str) == Some("crash-after-update") {
        return crash_after_update(&args[2]);
    }
    let out_dir = env::var_os("ZC_ISSUE381_D2_OUTPUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/issue-381-d2-output"));
    fs::create_dir_all(out_dir.join("data"))?;
    let results_path = out_dir.join("results.jsonl");

    if args.get(1).map(String::as_str) == Some("--smoke") {
        run_experiments(&out_dir, &results_path, &[1_000], 1)?;
        println!("smoke run complete: {}", results_path.display());
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("--contracts") {
        run_failure_contracts(&out_dir)?;
        println!("failure contracts complete");
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("--perf") {
        run_experiments(&out_dir, &results_path, &[10_000, 100_000], 2)?;
        println!("performance suite complete: {}", results_path.display());
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("--fts") {
        run_fts_contract(&out_dir)?;
        println!("FTS insert/delete/rename contract complete");
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("--single") {
        let rows: usize = args.get(2).ok_or("--single needs row count")?.parse()?;
        let variant = Variant::parse(
            args.get(3)
                .ok_or("--single needs A, B, C400, C, or Cfull")?,
        )
        .ok_or("unknown --single variant")?;
        run_single_case(&out_dir, rows, variant)?;
        return Ok(());
    }

    run_experiments(&out_dir, &results_path, &[10_000, 100_000], 2)?;
    run_failure_contracts(&out_dir)?;
    println!("experiments complete: {}", results_path.display());
    Ok(())
}

fn run_experiments(
    out_dir: &Path,
    results_path: &Path,
    scales: &[usize],
    repetitions: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    if results_path.exists() {
        fs::remove_file(results_path)?;
    }
    for &rows in scales {
        let root = out_dir.join("data").join(format!("root-{rows}"));
        prepare_root(&root)?;
        let base = out_dir
            .join("data")
            .join(format!("schema37-base-{rows}.sqlite"));
        prepare_fixture(&base, &root, rows)?;
        if rows == scales[0] {
            write_schema_dump(&base, &out_dir.join("schema37-sqlite-schema.sql"))?;
        }
        for replicate in 1..=repetitions {
            for variant in [
                Variant::A,
                Variant::B,
                Variant::C400,
                Variant::Cbulk,
                Variant::CbulkFull,
            ] {
                let db_path = out_dir.join("data").join(format!(
                    "case-{rows}-r{replicate}-{}.sqlite",
                    variant.label()
                ));
                fs::copy(&base, &db_path)?;
                let record = run_case(&db_path, &root, rows, replicate, variant)?;
                let mut file = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(results_path)?;
                writeln!(file, "{}", serde_json::to_string(&record)?)?;
                file.flush()?;
                println!("{}", serde_json::to_string(&record)?);
                fs::remove_file(&db_path)?;
                remove_sidecars(&db_path)?;
                restore_control_file(&root)?;
            }
        }
        fs::remove_file(&base)?;
        remove_sidecars(&base)?;
    }
    Ok(())
}

fn run_single_case(
    out_dir: &Path,
    rows: usize,
    variant: Variant,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = out_dir
        .join("data")
        .join(format!("root-single-{rows}-{}", variant.label()));
    prepare_root(&root)?;
    let base = out_dir
        .join("data")
        .join(format!("single-base-{rows}-{}.sqlite", variant.label()));
    prepare_fixture(&base, &root, rows)?;
    let db_path = out_dir
        .join("data")
        .join(format!("single-case-{rows}-{}.sqlite", variant.label()));
    fs::copy(&base, &db_path)?;
    let record = run_case(&db_path, &root, rows, 1, variant)?;
    let result_path = out_dir.join("single-case.jsonl");
    if result_path.exists() {
        fs::remove_file(&result_path)?;
    }
    append_json_line(&result_path, &record)?;
    println!("{}", record);
    drop(record);
    fs::remove_file(&db_path)?;
    remove_sidecars(&db_path)?;
    fs::remove_file(&base)?;
    remove_sidecars(&base)?;
    Ok(())
}

fn prepare_root(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if root.exists() {
        fs::remove_dir_all(root)?;
    }
    for index in 0..BRANCH_COUNT {
        fs::create_dir_all(root.join(format!("branch-{index:03}")))?;
    }
    fs::write(
        root.join("branch-000/renametestbefore.txt"),
        b"fts rename probe",
    )?;
    Ok(())
}

fn prepare_fixture(
    path: &Path,
    root: &Path,
    rows: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    db::initialize(path.to_str().expect("utf-8 fixture path"))?;
    let conn = open_product_style(path)?;
    let root_text = root.to_string_lossy().into_owned();
    let now = current_unix_seconds();
    conn.execute(
        "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, current_generation, active_run_id, active_generation, revision, needs_reconciliation, created_at, updated_at) VALUES (?1, ?2, 'D2 managed root', 'file_library', 1, 'scanning', 1, ?3, 1, 1, 0, ?4, ?4)",
        params![ROOT_ID, root_text, RUN_ID, now],
    )?;
    conn.execute(
        "INSERT INTO scan_sessions (id, status, phase, revision, started_at, created_at, updated_at) VALUES (?1, 'running', 'running', 1, ?2, ?3, ?3)",
        params![SESSION_ID, STARTED_AT, now],
    )?;
    conn.execute(
        "INSERT INTO scan_runs (id, scan_root_id, generation, parent_session_id, lease_token, status, phase, coverage_complete, stale_reconciliation_allowed, revision, started_at, watcher_revision_at_start, created_at, updated_at) VALUES (?1, ?2, 1, ?3, 'd2-lease', 'running', 'reconciling_missing', 1, 1, 1, ?4, 0, ?5, ?5)",
        params![RUN_ID, ROOT_ID, SESSION_ID, STARTED_AT, now],
    )?;
    conn.execute(
        "INSERT INTO scan_session_roots (session_id, requested_index, requested_path, normalized_requested_path, resolution, effective_root_id, effective_path, effective_index, run_id, status, created_at, updated_at) VALUES (?1, 0, ?2, ?2, 'effective', ?3, ?2, 0, ?4, 'running', ?5, ?5)",
        params![SESSION_ID, root_text, ROOT_ID, RUN_ID, now],
    )?;
    conn.execute(
        "INSERT INTO user_tags (id, display_name, normalized_name, color_token, created_at, updated_at) VALUES (?1, 'D2 tag', 'd2 tag', 'blue', ?2, ?2)",
        params![TAG_ID, now],
    )?;
    conn.execute(
        "INSERT INTO user_tags (id, display_name, normalized_name, color_token, created_at, updated_at) VALUES ('d4-concurrent-tag', 'D4 concurrent tag', 'd4 concurrent tag', 'green', ?1, ?1)",
        [now],
    )?;
    conn.execute(
        "INSERT INTO dedupe_runs (id, request_key, scope_json, scope_hash, status, phase, created_at, updated_at) VALUES (?1, 'd2-dedupe', '{}', 'd2', 'completed', 'completed', ?2, ?2)",
        params![DEDUPE_RUN_ID, now],
    )?;
    conn.execute(
        "INSERT INTO analysis_runs (id, request_key, scope_json, scope_hash, source_snapshot_json, source_snapshot_hash, detector_set_json, detector_set_hash, status, phase, created_at, updated_at) VALUES (?1, 'd2-analysis', '{}', 'd2-scope', '{}', 'd2-snapshot', '[]', 'd2-detectors', 'completed', 'completed', ?2, ?2)",
        params![ANALYSIS_RUN_ID, now],
    )?;

    let tx = conn.unchecked_transaction()?;
    for index in 0..rows {
        let branch = index % BRANCH_COUNT;
        let file_id = format!("missing-{index:07}");
        let name = format!("needle-stale-{index:07}.txt");
        let file_path = root
            .join(format!("branch-{branch:03}"))
            .join(&name)
            .to_string_lossy()
            .into_owned();
        tx.execute(
            "INSERT INTO files (id, path, name, extension, size, mtime, ctime, is_dir, state_code, file_type, purpose, lifecycle, context, risk_level, classification_reason, matched_rules, confidence, suggested_name, classification_status, is_stale, last_seen_at, content_hash) VALUES (?1, ?2, ?3, 'txt', 120, 10, 5, 0, 0, 'Document', 'Work', 'Active', ?4, 'Normal', 'user classification reason', '[\"rule:d2\"]', 0.91, 'user suggested name', 'classified', 0, 1, ?5)",
            params![file_id, file_path, name, format!("user-context-{index}"), format!("sha256-d2-{index:07}")],
        )?;
        tx.execute(
            "INSERT INTO file_user_tags (file_id, tag_id, created_at) VALUES (?1, ?2, ?3)",
            params![file_id, TAG_ID, now],
        )?;
        if index % 10 == 0 || index < 2 {
            tx.execute(
                "INSERT INTO file_fingerprints (file_id, path_snapshot, identity_status, size, fingerprint_status, captured_at, last_verified_at, prehash, full_hash) VALUES (?1, ?2, 'verified', 120, 'complete', ?3, ?3, 'd2-prehash', 'd2-fullhash')",
                params![file_id, file_path, now],
            )?;
        }
        if index == 0 {
            tx.execute(
                "INSERT INTO content_artifacts (id, file_id, scan_root_id, source_size, source_mtime, source_is_dir, extractor_family, extractor_version, policy_revision, content_fingerprint, status, summary, created_at, updated_at) VALUES ('d2-artifact', ?1, ?2, 120, 10, 0, 'txt', 'd2', 1, 'fingerprint-d2', 'current', 'preserved content record', ?3, ?3)",
                params![file_id, ROOT_ID, now],
            )?;
        }
    }
    for index in 0..rows {
        let branch = index % BRANCH_COUNT;
        let file_id = format!("live-{index:07}");
        let name = format!("needle-live-{index:07}.txt");
        let file_path = root
            .join(format!("branch-{branch:03}"))
            .join(&name)
            .to_string_lossy()
            .into_owned();
        tx.execute(
            "INSERT INTO files (id, path, name, extension, size, mtime, ctime, is_dir, state_code, file_type, purpose, lifecycle, context, risk_level, classification_reason, matched_rules, confidence, suggested_name, classification_status, is_stale, last_seen_at) VALUES (?1, ?2, ?3, 'txt', 96, 20, 10, 0, 0, 'Document', 'Work', 'Active', ?4, 'Normal', 'live classification reason', '[\\\"rule:d4\\\"]', 0.87, 'live suggested name', 'classified', 0, 200)",
            params![file_id, file_path, name, format!("live-context-{index}")],
        )?;
        if index == 0 {
            tx.execute(
                "INSERT INTO scan_seen (run_id, file_id, observed_path, observed_at) VALUES (?1, ?2, ?3, ?4)",
                params![RUN_ID, file_id, file_path, now],
            )?;
        }
    }
    let before = root.join("branch-000/renametestbefore.txt");
    tx.execute(
        "INSERT INTO files (id, path, name, extension, size, mtime, ctime, is_dir, state_code, file_type, purpose, lifecycle, context, risk_level, classification_reason, matched_rules, confidence, suggested_name, classification_status, is_stale, last_seen_at) VALUES ('live-control', ?1, 'renametestbefore.txt', 'txt', 16, 1, 1, 0, 0, 'Document', 'Work', 'Active', 'live user metadata', 'Normal', 'control history', '[]', 0.75, 'control suggestion', 'classified', 0, 1000)",
        params![before.to_string_lossy()],
    )?;
    tx.execute(
        "INSERT INTO classification_history (id, file_id, file_name, file_path, extension, source, file_type, purpose, lifecycle, context, risk_level, suggested_action, suggested_target_path, suggested_name, confidence, reason, keywords_json, user_confirmed, created_at) VALUES ('d2-history', 'missing-0000000', 'needle-stale-0000000.txt', ?1, 'txt', 'user', 'Document', 'Work', 'Active', 'history context', 'Normal', 'Keep', '', 'historic suggestion', 0.9, 'historic reason', '[]', 1, ?2)",
        params![root.join("branch-000/needle-stale-0000000.txt").to_string_lossy(), now],
    )?;
    tx.execute(
        "INSERT INTO duplicate_groups (id, size_each, full_hash, member_count, physical_copy_count, potential_reclaimable_bytes, reclaimable_confidence, status, last_built_run_id, created_at, updated_at, last_verified_at) VALUES (?1, 120, 'd2-duplicate-hash', 2, 2, 120, 'estimated', 'active', ?2, ?3, ?3, ?3)",
        params![GROUP_ID, DEDUPE_RUN_ID, now],
    )?;
    for index in 0..2 {
        let file_id = format!("missing-{index:07}");
        let file_path = root
            .join(format!("branch-{:03}", index % BRANCH_COUNT))
            .join(format!("needle-stale-{index:07}.txt"));
        tx.execute(
            "INSERT INTO duplicate_group_members (group_id, file_id, path_snapshot, identity_status, size, verified_at) VALUES (?1, ?2, ?3, 'verified', 120, ?4)",
            params![GROUP_ID, file_id, file_path.to_string_lossy(), now],
        )?;
    }
    tx.execute(
        "INSERT INTO analysis_findings (id, finding_key, run_id, detector_id, detector_version, scope_hash, status, tier, category, action_kind, title, reason, confidence, primary_subject_kind, primary_subject_id, path_snapshot, created_at, updated_at, published_at) VALUES ('d2-finding', 'd2-finding-key', ?1, 'duplicate_detector', 1, 'd2-scope', 'active', 'review', 'duplicate', 'review_duplicate_group', 'Duplicate group', 'existing evidence', 'estimated', 'duplicate_group', ?2, ?3, ?4, ?4, ?4)",
        params![ANALYSIS_RUN_ID, GROUP_ID, root.to_string_lossy(), now],
    )?;
    tx.commit()?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")?;
    Ok(())
}

fn open_product_style(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "mmap_size", 3_000_000_000_i64)?;
    conn.busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))?;
    Ok(conn)
}

fn write_schema_dump(db_path: &Path, dump_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::open(db_path)?;
    let mut statement = conn.prepare(
        "SELECT type, name, sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY type, name",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut text = String::new();
    for row in rows {
        let (kind, name, sql) = row?;
        text.push_str(&format!("-- {kind}: {name}\n{sql};\n\n"));
    }
    fs::write(dump_path, text)?;
    Ok(())
}

fn install_scoped_fts_trigger(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        DROP TRIGGER IF EXISTS files_au;
        CREATE TRIGGER files_au AFTER UPDATE ON files
        WHEN old.name IS NOT new.name OR old.path IS NOT new.path
        BEGIN
            INSERT INTO files_fts(files_fts, rowid, name, path)
            VALUES('delete', old.rowid, old.name, old.path);
            INSERT INTO files_fts(rowid, name, path) VALUES(new.rowid, new.name, new.path);
        END;
        "#,
    )
}

fn collect_candidates(conn: &Connection, root: &str) -> rusqlite::Result<Vec<Candidate>> {
    let slash_desc = format!("{}/%", root.trim_end_matches('/'));
    let backslash_desc = format!("{}\\%", root.trim_end_matches(['/', '\\']));
    let mut statement = conn.prepare(
        r#"
        SELECT f.id, f.path
        FROM files AS f
        WHERE f.is_stale = 0 AND f.last_seen_at < ?1
          AND (f.path = ?2 OR f.path LIKE ?3 ESCAPE '~' OR f.path LIKE ?4 ESCAPE '~')
          AND NOT EXISTS (
              SELECT 1 FROM scan_seen AS seen
              WHERE seen.run_id = ?5 AND (seen.file_id = f.id OR seen.observed_path = f.path)
          )
        "#,
    )?;
    let rows = statement.query_map(
        params![STARTED_AT, root, slash_desc, backslash_desc, RUN_ID],
        |row| {
            Ok(Candidate {
                id: row.get(0)?,
                path: row.get(1)?,
            })
        },
    )?;
    rows.collect()
}

fn root_identity(root: &Path) -> Result<RootIdentity, String> {
    let metadata = fs::symlink_metadata(root).map_err(|e| format!("root metadata: {e}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("root is not a real directory".to_string());
    }
    #[cfg(unix)]
    {
        Ok(RootIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        Ok(RootIdentity {})
    }
}

fn verify_filesystem_boundary(
    root: &Path,
    expected: RootIdentity,
    candidates: &[Candidate],
) -> Result<FsMetrics, String> {
    let started = Instant::now();
    let actual = root_identity(root)?;
    #[cfg(unix)]
    if actual.device != expected.device || actual.inode != expected.inode {
        return Err("root source identity changed".to_string());
    }
    let mut parents = BTreeSet::new();
    for candidate in candidates {
        let path = Path::new(&candidate.path);
        let parent = path
            .parent()
            .ok_or_else(|| "candidate has no parent".to_string())?;
        if !parent.starts_with(root) {
            return Err(format!("candidate escaped root: {}", candidate.id));
        }
        parents.insert(parent.to_path_buf());
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("candidate leaf cannot be observed: {error}")),
            Ok(_) => return Err(format!("candidate leaf reappeared: {}", candidate.id)),
        }
    }
    for parent in &parents {
        let relative = parent
            .strip_prefix(root)
            .map_err(|_| "parent escaped root".to_string())?;
        let mut cursor = root.to_path_buf();
        for component in relative.components() {
            cursor.push(component.as_os_str());
            let metadata = fs::symlink_metadata(&cursor)
                .map_err(|e| format!("directory chain cannot be observed: {e}"))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!(
                    "directory chain contains link or non-directory: {}",
                    cursor.display()
                ));
            }
        }
    }
    Ok(FsMetrics {
        leaf_checks: candidates.len(),
        distinct_directories: parents.len(),
        elapsed_ms: ms(started.elapsed()),
    })
}

fn create_temp_snapshot(conn: &Connection, candidates: &[Candidate]) -> rusqlite::Result<f64> {
    let started = Instant::now();
    conn.execute_batch(
        "CREATE TEMP TABLE stale_candidate_snapshot(id TEXT PRIMARY KEY, path TEXT NOT NULL);",
    )?;
    conn.execute_batch("BEGIN;")?;
    {
        let mut statement =
            conn.prepare("INSERT INTO stale_candidate_snapshot(id, path) VALUES (?1, ?2)")?;
        for candidate in candidates {
            statement.execute(params![candidate.id, candidate.path])?;
        }
    }
    conn.execute_batch("COMMIT;")?;
    Ok(ms(started.elapsed()))
}

fn verify_snapshot_matches_current_candidates(
    conn: &Connection,
    root: &str,
) -> rusqlite::Result<bool> {
    let slash_desc = format!("{}/%", root.trim_end_matches('/'));
    let backslash_desc = format!("{}\\%", root.trim_end_matches(['/', '\\']));
    let unseen: i64 = conn.query_row(
        r#"
        SELECT count(*) FROM files AS f
        WHERE f.is_stale=0 AND f.last_seen_at < ?1
          AND (f.path = ?2 OR f.path LIKE ?3 ESCAPE '~' OR f.path LIKE ?4 ESCAPE '~')
          AND NOT EXISTS (SELECT 1 FROM scan_seen s WHERE s.run_id=?5 AND (s.file_id=f.id OR s.observed_path=f.path))
          AND NOT EXISTS (SELECT 1 FROM stale_candidate_snapshot c WHERE c.id=f.id AND c.path=f.path)
        "#,
        params![STARTED_AT, root, slash_desc, backslash_desc, RUN_ID],
        |row| row.get(0),
    )?;
    let changed: i64 = conn.query_row(
        r#"
        SELECT count(*) FROM stale_candidate_snapshot c
        LEFT JOIN files f ON f.id=c.id
        WHERE f.id IS NULL OR f.path <> c.path OR f.is_stale <> 0 OR f.last_seen_at >= ?1
        "#,
        params![STARTED_AT],
        |row| row.get(0),
    )?;
    Ok(unseen == 0 && changed == 0)
}

fn verify_durable_fence(conn: &Connection) -> rusqlite::Result<bool> {
    let root: (i64, i64, Option<String>, Option<i64>, i64, i64) = conn.query_row(
        "SELECT revision, watcher_revision, active_run_id, active_generation, current_generation, enabled FROM scan_roots WHERE id=?1",
        [ROOT_ID],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    )?;
    let run: (i64, String, i64, i64, i64, Option<String>, String, i64, i64, String) = conn.query_row(
        "SELECT revision, status, cancel_requested, generation, watcher_revision_at_start, parent_session_id, lease_token, coverage_error_count, started_at, phase FROM scan_runs WHERE id=?1",
        [RUN_ID],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?)),
    )?;
    let session: (i64, String, i64) = conn.query_row(
        "SELECT revision, status, cancel_requested FROM scan_sessions WHERE id=?1",
        [SESSION_ID],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    Ok(root == (1, 0, Some(RUN_ID.to_string()), Some(1), 1, 1)
        && run
            == (
                1,
                "running".to_string(),
                0,
                1,
                0,
                Some(SESSION_ID.to_string()),
                "d2-lease".to_string(),
                0,
                STARTED_AT,
                "reconciling_missing".to_string(),
            )
        && session == (1, "running".to_string(), 0))
}

fn update_stale_rows(
    conn: &Connection,
    candidates: &[Candidate],
    variant: Variant,
) -> rusqlite::Result<usize> {
    if variant.set_update() {
        return conn.execute(
            "UPDATE files AS f SET is_stale=1 FROM stale_candidate_snapshot AS c WHERE f.id=c.id AND f.is_stale=0",
            [],
        );
    }
    let mut marked = 0;
    for chunk in candidates.chunks(400) {
        let placeholders = (1..=chunk.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let values = chunk
            .iter()
            .map(|item| rusqlite::types::Value::Text(item.id.clone()))
            .collect::<Vec<_>>();
        marked += conn.execute(
            &format!("UPDATE files SET is_stale=1 WHERE is_stale=0 AND id IN ({placeholders})"),
            rusqlite::params_from_iter(values),
        )?;
    }
    Ok(marked)
}

fn invalidate_related_state(conn: &Connection) -> rusqlite::Result<f64> {
    let started = Instant::now();
    let group_ids = {
        let mut statement = conn.prepare(
            "SELECT DISTINCT group_id FROM duplicate_group_members WHERE file_id IN (SELECT id FROM files WHERE is_stale=1)",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    conn.execute(
        "UPDATE files SET content_hash='' WHERE is_stale=1 AND content_hash<>''",
        [],
    )?;
    conn.execute(
        "UPDATE file_fingerprints SET fingerprint_status='missing', identity_status='missing', revision=revision+1, error_code='missing' WHERE file_id IN (SELECT id FROM files WHERE is_stale=1) AND (fingerprint_status<>'missing' OR identity_status<>'missing')",
        [],
    )?;
    conn.execute(
        "UPDATE duplicate_groups SET status='stale', revision=revision+1, updated_at=?1 WHERE status='active' AND id IN (SELECT group_id FROM duplicate_group_members WHERE file_id IN (SELECT id FROM files WHERE is_stale=1))",
        [current_unix_seconds()],
    )?;
    for group_id in group_ids {
        conn.execute(
            "UPDATE analysis_findings SET status='stale', stale_at=?1, revision=revision+1, updated_at=?1 WHERE status='active' AND primary_subject_kind='duplicate_group' AND primary_subject_id=?2",
            params![current_unix_seconds(), group_id],
        )?;
    }
    conn.execute(
        "UPDATE dedupe_authority_state SET revision=revision+1, status='rebuild_required', updated_at=?1 WHERE id=1",
        [current_unix_seconds()],
    )?;
    Ok(ms(started.elapsed()))
}

fn publish_revisions(conn: &Connection) -> rusqlite::Result<usize> {
    let root_changed = conn.execute(
        "UPDATE scan_roots SET library_change_revision=library_change_revision+1 WHERE id=?1 AND revision=1 AND watcher_revision=0 AND active_run_id=?2 AND active_generation=1 AND current_generation=1 AND enabled=1",
        params![ROOT_ID, RUN_ID],
    )?;
    let library_changed = conn.execute(
        "UPDATE library_query_state SET revision=revision+1, updated_at=?1 WHERE singleton_id=1",
        [current_unix_seconds()],
    )?;
    let run_changed = conn.execute(
        "UPDATE scan_runs SET coverage_complete=1, stale_reconciliation_allowed=1, phase='reconciling_missing', last_checkpoint_at=?1, revision=revision+1, updated_at=?1 WHERE id=?2 AND status='running' AND cancel_requested=0 AND coverage_error_count=0 AND revision=1 AND lease_token='d2-lease' AND generation=1 AND watcher_revision_at_start=0",
        params![current_unix_seconds(), RUN_ID],
    )?;
    Ok(root_changed + library_changed + run_changed)
}

fn run_case(
    db_path: &Path,
    root: &Path,
    rows: usize,
    replicate: usize,
    variant: Variant,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let conn = open_product_style(db_path)?;
    if variant.conditional_fts() {
        install_scoped_fts_trigger(&conn)?;
    }
    let expected_root = root_identity(root).map_err(|e| format!("capture root identity: {e}"))?;
    let root_text = root.to_string_lossy().into_owned();

    let preflight_start = Instant::now();
    let (preflight_candidates, preflight_candidate_sql_ms, preflight_fs, snapshot_prepare_ms) =
        if variant.preflight() {
            let candidate_start = Instant::now();
            let candidates = collect_candidates(&conn, &root_text)?;
            let candidate_ms = ms(candidate_start.elapsed());
            let fs_metrics = verify_filesystem_boundary(root, expected_root, &candidates)
                .map_err(|e| format!("preflight boundary rejected: {e}"))?;
            let temp_ms = create_temp_snapshot(&conn, &candidates)?;
            (Some(candidates), candidate_ms, fs_metrics, temp_ms)
        } else {
            (None, 0.0, FsMetrics::default(), 0.0)
        };
    let preflight_total_ms = if variant.preflight() {
        ms(preflight_start.elapsed())
    } else {
        0.0
    };
    let preflight_candidate_count = preflight_candidates.as_ref().map_or(0, Vec::len);

    let begin_call = Instant::now();
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let lock_acquired = Instant::now();
    let candidates_start = Instant::now();
    let candidates = match preflight_candidates {
        Some(candidates) => candidates,
        None => collect_candidates(&conn, &root_text)?,
    };
    let candidate_sql_ms = if variant.preflight() {
        0.0
    } else {
        ms(candidates_start.elapsed())
    };
    if candidates.len() != rows || (variant.preflight() && preflight_candidate_count != rows) {
        conn.execute_batch("ROLLBACK")?;
        return Err(format!(
            "candidate count mismatch: got {}, expected {rows}",
            candidates.len()
        )
        .into());
    }
    let initial_fs = if variant.preflight() {
        FsMetrics::default()
    } else {
        verify_filesystem_boundary(root, expected_root, &candidates)
            .map_err(|e| format!("initial in-lock boundary check failed: {e}"))?
    };

    let (reader_stop, reader_samples, reader_handle) = start_reader(db_path, rows)?;
    let (writer_start_rx, writer_result_rx, writer_handles) = start_competing_writers(db_path)?;
    writer_start_rx.recv_timeout(Duration::from_secs(2))?;
    writer_start_rx.recv_timeout(Duration::from_secs(2))?;

    let fence_started = Instant::now();
    let db_fence_ok = verify_durable_fence(&conn)?;
    let candidate_set_ok = if variant.preflight() {
        verify_snapshot_matches_current_candidates(&conn, &root_text)?
    } else {
        true
    };
    let final_fs = verify_filesystem_boundary(root, expected_root, &candidates)
        .map_err(|e| format!("final in-lock boundary check failed: {e}"))?;
    if !db_fence_ok || !candidate_set_ok {
        conn.execute_batch("ROLLBACK")?;
        reader_stop.store(true, Ordering::Release);
        let _ = reader_handle.join();
        for handle in writer_handles {
            let _ = handle.join();
        }
        return Err(format!(
            "fence rejected publish: durable={db_fence_ok} candidates={candidate_set_ok}"
        )
        .into());
    }
    let fence_and_fs_ms = ms(fence_started.elapsed());

    let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    let wal_before = file_size(&wal_path);
    let update_started = Instant::now();
    let marked = update_stale_rows(&conn, &candidates, variant)?;
    let update_sql_ms = ms(update_started.elapsed());
    let invalidation_ms = invalidate_related_state(&conn)?;
    let wal_before_commit = file_size(&wal_path);
    let revision_started = Instant::now();
    let revision_rows = publish_revisions(&conn)?;
    let revision_sql_ms = ms(revision_started.elapsed());
    if marked != rows || revision_rows != 3 {
        conn.execute_batch("ROLLBACK")?;
        return Err(format!(
            "publish row counts invalid: marked={marked}, revision_rows={revision_rows}"
        )
        .into());
    }
    let commit_started = Instant::now();
    conn.execute_batch("COMMIT")?;
    let commit_ms = ms(commit_started.elapsed());
    let lock_residency_ms = ms(lock_acquired.elapsed());
    let begin_to_commit_ms = ms(begin_call.elapsed());
    let wal_after_commit = file_size(&wal_path);

    reader_stop.store(true, Ordering::Release);
    let _ = reader_handle.join();
    let mut writer_results = Vec::new();
    for _ in 0..2 {
        writer_results.push(writer_result_rx.recv_timeout(Duration::from_secs(7))?);
    }
    for handle in writer_handles {
        let _ = handle.join();
    }

    let checks = verify_correctness(&conn, root, rows)?;
    let states: BTreeSet<i64> = reader_samples
        .lock()
        .expect("reader samples")
        .iter()
        .copied()
        .collect();
    let partial_visible = states
        .iter()
        .any(|count| *count != 0 && *count != rows as i64);
    let readers_only_complete_states = !partial_visible;
    let writers_all_succeeded = writer_results.iter().all(|item| item.1.is_ok());
    let writers_max_wait_ms = writer_results
        .iter()
        .map(|item| item.2)
        .fold(0.0_f64, f64::max);
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");

    Ok(serde_json::json!({
        "record_type": "stale_reconcile_case",
        "variant": variant.label(),
        "rows": rows,
        "replicate": replicate,
        "schema_version": 37,
        "sqlite_version": conn.query_row("SELECT sqlite_version()", [], |r| r.get::<_, String>(0))?,
        "host_os": env::consts::OS,
        "host_arch": env::consts::ARCH,
        "busy_timeout_ms": BUSY_TIMEOUT_MS,
        "journal_mode": "WAL",
        "synchronous": "NORMAL",
        "preflight_total_ms": preflight_total_ms,
        "preflight_candidate_sql_ms": preflight_candidate_sql_ms,
        "preflight_filesystem_ms": preflight_fs.elapsed_ms,
        "preflight_leaf_checks": preflight_fs.leaf_checks,
        "preflight_distinct_parent_dirs": preflight_fs.distinct_directories,
        "temporary_candidate_snapshot_ms": snapshot_prepare_ms,
        "candidate_sql_ms_inside_tx": candidate_sql_ms,
        "initial_fs_validation_ms_inside_tx": initial_fs.elapsed_ms,
        "initial_fs_leaf_checks_inside_tx": initial_fs.leaf_checks,
        "final_fence_and_fs_validation_ms_inside_tx": fence_and_fs_ms,
        "final_fs_validation_ms_inside_tx": final_fs.elapsed_ms,
        "final_fs_leaf_checks_inside_tx": final_fs.leaf_checks,
        "final_fs_distinct_parent_dirs": final_fs.distinct_directories,
        "stale_update_sql_and_trigger_ms": update_sql_ms,
        "related_invalidation_sql_ms": invalidation_ms,
        "final_revision_cas_sql_ms": revision_sql_ms,
        "commit_ms": commit_ms,
        "begin_wait_ms": ms(lock_acquired.duration_since(begin_call)),
        "begin_immediate_to_commit_ms": begin_to_commit_ms,
        "lock_residency_after_begin_ms": lock_residency_ms,
        "wal_bytes_before": wal_before,
        "wal_bytes_before_commit": wal_before_commit,
        "wal_bytes_after_commit": wal_after_commit,
        "marked_stale": marked,
        "reader_stale_count_samples": reader_samples.lock().expect("reader samples").len(),
        "reader_visible_stale_counts": states,
        "reader_observed_partial_state": partial_visible,
        "only_old_or_complete_visibility_observed": readers_only_complete_states,
        "concurrent_writers": writer_results.iter().map(|item| serde_json::json!({"kind": item.0, "outcome": if item.1.is_ok() {"committed"} else {"busy_timeout"}, "wait_ms": item.2, "error": item.1.as_ref().err()})).collect::<Vec<_>>(),
        "all_concurrent_writers_succeeded_within_5000ms": writers_all_succeeded && writers_max_wait_ms < BUSY_TIMEOUT_MS as f64,
        "max_concurrent_writer_wait_ms": writers_max_wait_ms,
        "correctness": checks,
        "passed": checks["all"] == true && readers_only_complete_states,
    }))
}

type WriterResult = (String, Result<(), String>, f64);

fn start_competing_writers(
    db_path: &Path,
) -> Result<
    (
        mpsc::Receiver<()>,
        mpsc::Receiver<WriterResult>,
        Vec<thread::JoinHandle<()>>,
    ),
    Box<dyn std::error::Error>,
> {
    let (start_tx, start_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let mut handles = Vec::new();
    for kind in ["ordinary_file_metadata", "user_tag_metadata"] {
        let db = db_path.to_path_buf();
        let started = start_tx.clone();
        let results = result_tx.clone();
        let label = kind.to_string();
        handles.push(thread::spawn(move || {
            let conn = Connection::open(db).expect("writer connection");
            conn.busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))
                .expect("writer timeout");
            let started_at = Instant::now();
            started.send(()).expect("announce competing writer");
            let result = if label == "ordinary_file_metadata" {
                conn.execute(
                    "UPDATE files SET context='concurrent-writer-context' WHERE id='live-control'",
                    [],
                )
            } else {
                conn.execute(
                    "UPDATE user_tags SET updated_at=updated_at+1 WHERE id=?1",
                    [TAG_ID],
                )
            };
            let elapsed = ms(started_at.elapsed());
            let mapped = result.map(|_| ()).map_err(|error| error.to_string());
            let _ = results.send((label, mapped, elapsed));
        }));
    }
    drop(start_tx);
    drop(result_tx);
    Ok((start_rx, result_rx, handles))
}

fn start_reader(
    db_path: &Path,
    expected_rows: usize,
) -> Result<
    (
        Arc<AtomicBool>,
        Arc<Mutex<Vec<i64>>>,
        thread::JoinHandle<()>,
    ),
    Box<dyn std::error::Error>,
> {
    let stop = Arc::new(AtomicBool::new(false));
    let samples = Arc::new(Mutex::new(Vec::new()));
    let stop_thread = Arc::clone(&stop);
    let samples_thread = Arc::clone(&samples);
    let db = db_path.to_path_buf();
    let handle = thread::spawn(move || {
        let conn = Connection::open(db).expect("visibility reader connection");
        conn.busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))
            .expect("reader timeout");
        while !stop_thread.load(Ordering::Acquire) {
            let value: i64 = conn
                .query_row("SELECT count(*) FROM files WHERE is_stale=1", [], |row| {
                    row.get(0)
                })
                .expect("read published stale count");
            samples_thread.lock().expect("reader samples").push(value);
            thread::sleep(Duration::from_millis(10));
        }
        let final_value: i64 = conn
            .query_row("SELECT count(*) FROM files WHERE is_stale=1", [], |row| {
                row.get(0)
            })
            .expect("final read");
        samples_thread
            .lock()
            .expect("reader samples")
            .push(final_value);
    });
    // This metadata value is only used as an explicit sanity bound in result decoding.
    let _ = expected_rows;
    Ok((stop, samples, handle))
}

fn verify_correctness(
    conn: &Connection,
    root: &Path,
    rows: usize,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let stale: i64 = conn.query_row("SELECT count(*) FROM files WHERE is_stale=1", [], |row| {
        row.get(0)
    })?;
    let active_needle: i64 = conn.query_row(
        "SELECT count(*) FROM files_fts JOIN files f ON f.rowid=files_fts.rowid WHERE files_fts MATCH 'needle' AND f.is_stale=0",
        [],
        |row| row.get(0),
    )?;
    let raw_needle: i64 = conn.query_row(
        "SELECT count(*) FROM files_fts WHERE files_fts MATCH 'needle'",
        [],
        |row| row.get(0),
    )?;
    let tag_count: i64 = conn.query_row(
        "SELECT count(*) FROM file_user_tags WHERE file_id LIKE 'missing-%'",
        [],
        |row| row.get(0),
    )?;
    let history_count: i64 = conn.query_row("SELECT count(*) FROM classification_history WHERE id='d2-history' AND reason='historic reason'", [], |row| row.get(0))?;
    let user_metadata: (String, String, String) = conn.query_row(
        "SELECT context, suggested_name, classification_reason FROM files WHERE id='missing-0000000'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let artifact_status: String = conn.query_row(
        "SELECT status FROM content_artifacts WHERE id='d2-artifact'",
        [],
        |row| row.get(0),
    )?;
    let group_status: String = conn.query_row(
        "SELECT status FROM duplicate_groups WHERE id=?1",
        [GROUP_ID],
        |row| row.get(0),
    )?;
    let finding_status: String = conn.query_row(
        "SELECT status FROM analysis_findings WHERE id='d2-finding'",
        [],
        |row| row.get(0),
    )?;
    let fingerprints_missing: i64 = conn.query_row("SELECT count(*) FROM file_fingerprints WHERE fingerprint_status='missing' AND file_id LIKE 'missing-%'", [], |row| row.get(0))?;

    let old = root.join("branch-000/renametestbefore.txt");
    let new = root.join("branch-000/renametestafter.txt");
    fs::rename(&old, &new)?;
    conn.execute(
        "UPDATE files SET path=?1, name='renametestafter.txt' WHERE id='live-control'",
        [new.to_string_lossy().as_ref()],
    )?;
    let old_fts: i64 = conn.query_row(
        "SELECT count(*) FROM files_fts WHERE files_fts MATCH 'renametestbefore'",
        [],
        |row| row.get(0),
    )?;
    let new_fts: i64 = conn.query_row(
        "SELECT count(*) FROM files_fts WHERE files_fts MATCH 'renametestafter'",
        [],
        |row| row.get(0),
    )?;
    fs::rename(&new, &old)?;

    let checks = serde_json::json!({
        "stale_count_exact": stale == rows as i64,
        "file_library_fts_excludes_stale": active_needle == 0,
        "stale_rows_remain_in_fts_source_index": raw_needle == rows as i64,
        "file_tags_preserved": tag_count == rows as i64,
        "classification_history_preserved": history_count == 1,
        "user_metadata_preserved": user_metadata == ("user-context-0".to_string(), "user suggested name".to_string(), "user classification reason".to_string()),
        "content_artifact_invalidated_by_existing_trigger": artifact_status == "stale",
        "duplicate_group_stale": group_status == "stale",
        "analysis_finding_stale": finding_status == "stale",
        "fingerprints_invalidated": fingerprints_missing > 0,
        "normal_rename_removes_old_fts": old_fts == 0,
        "normal_rename_adds_new_fts": new_fts == 1,
        "stale_rows": stale,
        "raw_fts_needle_rows": raw_needle,
        "library_fts_needle_rows": active_needle,
        "file_tag_rows": tag_count,
        "fingerprints_invalidated_count": fingerprints_missing,
        "rename_old_match_count": old_fts,
        "rename_new_match_count": new_fts,
    });
    let all = checks.as_object().unwrap().iter().all(|(key, value)| {
        key.ends_with("_preserved") || key == "all" || value == true || value.is_number()
    });
    let all = all
        && stale == rows as i64
        && active_needle == 0
        && raw_needle == rows as i64
        && tag_count == rows as i64
        && history_count == 1
        && user_metadata
            == (
                "user-context-0".to_string(),
                "user suggested name".to_string(),
                "user classification reason".to_string(),
            )
        && artifact_status == "stale"
        && group_status == "stale"
        && finding_status == "stale"
        && fingerprints_missing > 0
        && old_fts == 0
        && new_fts == 1;
    let mut checks = checks;
    checks["all"] = serde_json::Value::Bool(all);
    Ok(checks)
}

fn run_failure_contracts(out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let rows = 1_000;
    let failures = out_dir.join("failure-contracts.jsonl");
    let _ = fs::remove_file(&failures);
    let cases = [
        "rollback",
        "watcher_change",
        "cancel",
        "old_generation",
        "source_replacement",
        "symlink_replacement",
        "leaf_reappears",
    ];
    for case in cases {
        let root = out_dir.join("data").join(format!("fault-root-{case}"));
        prepare_root(&root)?;
        let db_path = out_dir.join("data").join(format!("fault-{case}.sqlite"));
        prepare_fixture(&db_path, &root, rows)?;
        let conn = open_product_style(&db_path)?;
        install_scoped_fts_trigger(&conn)?;
        let identity = root_identity(&root).map_err(|e| format!("capture root: {e}"))?;
        let candidates = collect_candidates(&conn, &root.to_string_lossy())?;
        create_temp_snapshot(&conn, &candidates)?;

        if case == "rollback" {
            conn.execute_batch("BEGIN IMMEDIATE")?;
            update_stale_rows(&conn, &candidates, Variant::Cbulk)?;
            invalidate_related_state(&conn)?;
            publish_revisions(&conn)?;
            conn.execute_batch("ROLLBACK")?;
        } else {
            match case {
                "watcher_change" => {
                    conn.execute(
                        "UPDATE scan_roots SET watcher_revision=1 WHERE id=?1",
                        [ROOT_ID],
                    )?;
                }
                "cancel" => {
                    conn.execute(
                        "UPDATE scan_runs SET cancel_requested=1 WHERE id=?1",
                        [RUN_ID],
                    )?;
                }
                "old_generation" => {
                    conn.execute("UPDATE scan_roots SET current_generation=2, active_generation=2 WHERE id=?1", [ROOT_ID])?;
                }
                "source_replacement" => {
                    let moved = root.with_extension("old");
                    if moved.exists() {
                        fs::remove_dir_all(&moved)?;
                    }
                    fs::rename(&root, &moved)?;
                    prepare_root(&root)?;
                }
                "symlink_replacement" => {
                    let target = out_dir.join("data").join(format!("outside-{case}"));
                    fs::create_dir_all(&target)?;
                    let branch = root.join("branch-000");
                    let saved = root.join("branch-000-saved");
                    fs::rename(&branch, &saved)?;
                    std::os::unix::fs::symlink(&target, &branch)?;
                }
                "leaf_reappears" => {
                    let leaf = Path::new(&candidates[0].path);
                    fs::write(leaf, b"file came back after preflight")?;
                }
                _ => unreachable!(),
            }
            conn.execute_batch("BEGIN IMMEDIATE")?;
            let durable = verify_durable_fence(&conn)?;
            let candidates_match =
                verify_snapshot_matches_current_candidates(&conn, &root.to_string_lossy())?;
            let fs_result = verify_filesystem_boundary(&root, identity, &candidates);
            let allowed = durable && candidates_match && fs_result.is_ok();
            if allowed {
                let _ = update_stale_rows(&conn, &candidates, Variant::Cbulk)?;
                let _ = invalidate_related_state(&conn)?;
                let _ = publish_revisions(&conn)?;
                conn.execute_batch("COMMIT")?;
            } else {
                conn.execute_batch("ROLLBACK")?;
            }
            let record = serde_json::json!({
                "record_type": "fence_failure_contract",
                "case": case,
                "durable_fence": durable,
                "candidate_snapshot_matches": candidates_match,
                "filesystem_boundary": fs_result.as_ref().map(|_| "verified").unwrap_or("rejected"),
                "rejection_reason": fs_result.err(),
                "published": allowed,
                "stale_count_after": conn.query_row("SELECT count(*) FROM files WHERE is_stale=1", [], |row| row.get::<_, i64>(0))?,
                "passed": !allowed && conn.query_row("SELECT count(*) FROM files WHERE is_stale=1", [], |row| row.get::<_, i64>(0))? == 0,
            });
            append_json_line(&failures, &record)?;
            println!("{}", record);
        }
        if case == "rollback" {
            let stale: i64 =
                conn.query_row("SELECT count(*) FROM files WHERE is_stale=1", [], |row| {
                    row.get(0)
                })?;
            let artifact: String = conn.query_row(
                "SELECT status FROM content_artifacts WHERE id='d2-artifact'",
                [],
                |row| row.get(0),
            )?;
            let run_revision: i64 = conn.query_row(
                "SELECT revision FROM scan_runs WHERE id=?1",
                [RUN_ID],
                |row| row.get(0),
            )?;
            let record = serde_json::json!({"record_type":"fence_failure_contract","case":"rollback","stale_count_after":stale,"artifact_status_after":artifact,"run_revision_after":run_revision,"passed":stale==0 && artifact=="current" && run_revision==1});
            append_json_line(&failures, &record)?;
            println!("{}", record);
        }
        drop(conn);
        let _ = fs::remove_file(&db_path);
        remove_sidecars(&db_path)?;
    }

    let root = out_dir.join("data/root-crash");
    prepare_root(&root)?;
    let base = out_dir.join("data/crash-base.sqlite");
    prepare_fixture(&base, &root, rows)?;
    let crash_db = out_dir.join("data/crash-after-update.sqlite");
    fs::copy(&base, &crash_db)?;
    let mut child = Command::new(env::current_exe()?)
        .arg("crash-after-update")
        .arg(&crash_db)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("crash child stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim() != "AFTER_UNCOMMITTED_STALE_UPDATE" {
        let _ = child.kill();
        return Err(format!("crash child did not reach failpoint: {line}").into());
    }
    child.kill()?;
    let _ = child.wait()?;
    let after_crash = open_product_style(&crash_db)?;
    let stale: i64 =
        after_crash.query_row("SELECT count(*) FROM files WHERE is_stale=1", [], |row| {
            row.get(0)
        })?;
    let integrity: String =
        after_crash.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    let record = serde_json::json!({"record_type":"fence_failure_contract","case":"process_kill_after_update_before_commit","stale_count_after":stale,"integrity_check":integrity,"passed":stale==0 && integrity=="ok"});
    append_json_line(&failures, &record)?;
    println!("{}", record);
    drop(after_crash);
    for path in [&base, &crash_db] {
        let _ = fs::remove_file(path);
        remove_sidecars(path)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum D4Model {
    TombstoneOverlay,
    VersionedProjection,
}

impl D4Model {
    fn label(self) -> &'static str {
        match self {
            Self::TombstoneOverlay => "tombstone_overlay",
            Self::VersionedProjection => "versioned_file_projection",
        }
    }
}

#[derive(Clone, Debug)]
struct D4Fence {
    generation: i64,
    root_revision: i64,
    run_revision: i64,
    session_revision: i64,
    watcher_revision: i64,
    library_revision: i64,
}

#[derive(Default)]
struct D4StageMetrics {
    elapsed_ms: f64,
    max_chunk_tx_ms: f64,
    chunk_count: usize,
    projection_fts_ms: f64,
}

#[derive(Default, Debug)]
struct D4WriterResults {
    metadata_wait_ms: f64,
    metadata_ok: bool,
    tag_wait_ms: f64,
    tag_ok: bool,
}

fn run_d4_s0(
    out_dir: &Path,
    scales: &[usize],
    run_failure_contracts: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(out_dir.join("data"))?;
    let results_path = out_dir.join("results.jsonl");
    if results_path.exists() {
        fs::remove_file(&results_path)?;
    }
    for &rows in scales {
        let root = out_dir.join("data").join(format!("root-{rows}"));
        prepare_root(&root)?;
        let base = out_dir
            .join("data")
            .join(format!("schema37-base-{rows}.sqlite"));
        prepare_fixture(&base, &root, rows)?;
        if rows == 10_000 {
            write_schema_dump(&base, &out_dir.join("schema37-sqlite-schema.sql"))?;
        }
        for replicate in 1..=3 {
            for model in [D4Model::TombstoneOverlay, D4Model::VersionedProjection] {
                let db_path = out_dir
                    .join("data")
                    .join(format!("case-{rows}-r{replicate}-{}.sqlite", model.label()));
                let _ = fs::remove_file(&db_path);
                remove_sidecars(&db_path)?;
                fs::copy(&base, &db_path)?;
                let record = run_d4_case(&db_path, &root, rows, replicate, model)?;
                append_json_line(&results_path, &record)?;
                println!("{}", record);
                drop(record);
                let _ = fs::remove_file(&db_path);
                remove_sidecars(&db_path)?;
            }
        }
        let _ = fs::remove_file(&base);
        remove_sidecars(&base)?;
        fs::remove_dir_all(&root)?;
    }
    if run_failure_contracts {
        run_d4_failure_contracts(out_dir)?;
    }
    println!("D4-S0 harness complete: {}", results_path.display());
    Ok(())
}

fn initialize_d4_tables(
    conn: &mut Connection,
    model: D4Model,
    root_path: &str,
) -> Result<f64, Box<dyn std::error::Error>> {
    conn.execute_batch(
        r#"
        CREATE TABLE d4_s0_manifests (
            manifest_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE d4_s0_manifest_roots (
            manifest_id TEXT NOT NULL,
            root_id TEXT NOT NULL,
            operation_id TEXT,
            generation INTEGER NOT NULL,
            PRIMARY KEY(manifest_id, root_id)
        );
        CREATE TABLE d4_s0_current_manifest (
            singleton_id INTEGER PRIMARY KEY CHECK(singleton_id = 1),
            manifest_id TEXT NOT NULL
        );
        CREATE TABLE d4_s0_operations (
            operation_id TEXT PRIMARY KEY,
            manifest_id TEXT NOT NULL,
            model TEXT NOT NULL,
            session_id TEXT NOT NULL,
            run_id TEXT NOT NULL,
            root_id TEXT NOT NULL,
            candidate_count INTEGER NOT NULL,
            expected_generation INTEGER NOT NULL,
            expected_root_revision INTEGER NOT NULL,
            expected_run_revision INTEGER NOT NULL,
            expected_session_revision INTEGER NOT NULL,
            expected_watcher_revision INTEGER NOT NULL,
            expected_library_revision INTEGER NOT NULL,
            state TEXT NOT NULL
        );
        CREATE TABLE d4_s0_staged_tombstones (
            operation_id TEXT NOT NULL,
            file_id TEXT NOT NULL,
            path_snapshot TEXT NOT NULL,
            PRIMARY KEY(operation_id, file_id)
        );
        CREATE INDEX d4_s0_staged_tombstones_file
            ON d4_s0_staged_tombstones(file_id, operation_id);
        CREATE TABLE d4_s0_file_projection AS
            SELECT CAST('' AS TEXT) AS manifest_id,
                   CAST('' AS TEXT) AS root_id,
                   f.rowid AS source_rowid,
                   f.*
            FROM files AS f WHERE 0;
        CREATE UNIQUE INDEX d4_s0_file_projection_key
            ON d4_s0_file_projection(manifest_id, root_id, id);
        CREATE INDEX d4_s0_file_projection_rowid
            ON d4_s0_file_projection(manifest_id, root_id, source_rowid);
        CREATE VIRTUAL TABLE d4_s0_projection_fts USING fts5(
            manifest_id UNINDEXED,
            root_id UNINDEXED,
            file_id UNINDEXED,
            name,
            path,
            tokenize='trigram'
        );
        "#,
    )?;
    let now = current_unix_seconds();
    conn.execute(
        "INSERT INTO d4_s0_manifests(manifest_id, session_id, created_at) VALUES ('manifest-0', ?1, ?2)",
        params![SESSION_ID, now],
    )?;
    conn.execute(
        "INSERT INTO d4_s0_manifest_roots(manifest_id, root_id, operation_id, generation) VALUES ('manifest-0', ?1, NULL, 0)",
        [ROOT_ID],
    )?;
    conn.execute(
        "INSERT INTO d4_s0_current_manifest(singleton_id, manifest_id) VALUES (1, 'manifest-0')",
        [],
    )?;
    if model == D4Model::VersionedProjection {
        let started = Instant::now();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let root_desc = format!("{}/%", root_path.trim_end_matches('/'));
        tx.execute(
            "INSERT INTO d4_s0_file_projection SELECT 'manifest-0', ?1, f.rowid, f.* FROM files AS f WHERE f.path = ?2 OR f.path LIKE ?3 ESCAPE '~'",
            params![ROOT_ID, root_path, root_desc],
        )?;
        tx.execute(
            "INSERT INTO d4_s0_projection_fts(manifest_id, root_id, file_id, name, path) SELECT manifest_id, root_id, id, name, path FROM d4_s0_file_projection WHERE manifest_id='manifest-0' AND root_id=?1",
            [ROOT_ID],
        )?;
        tx.commit()?;
        Ok(ms(started.elapsed()))
    } else {
        Ok(0.0)
    }
}

fn capture_d4_fence(conn: &Connection) -> rusqlite::Result<D4Fence> {
    let (generation, root_revision, watcher_revision): (i64, i64, i64) = conn.query_row(
        "SELECT current_generation, revision, watcher_revision FROM scan_roots WHERE id=?1",
        [ROOT_ID],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let run_revision: i64 = conn.query_row(
        "SELECT revision FROM scan_runs WHERE id=?1",
        [RUN_ID],
        |row| row.get(0),
    )?;
    let session_revision: i64 = conn.query_row(
        "SELECT revision FROM scan_sessions WHERE id=?1",
        [SESSION_ID],
        |row| row.get(0),
    )?;
    let library_revision: i64 = conn.query_row(
        "SELECT revision FROM library_query_state WHERE singleton_id=1",
        [],
        |row| row.get(0),
    )?;
    Ok(D4Fence {
        generation,
        root_revision,
        run_revision,
        session_revision,
        watcher_revision,
        library_revision,
    })
}

fn create_d4_operation(
    conn: &mut Connection,
    model: D4Model,
    operation_id: &str,
    manifest_id: &str,
    candidate_count: usize,
) -> rusqlite::Result<D4Fence> {
    let fence = capture_d4_fence(conn)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT INTO d4_s0_manifests(manifest_id, session_id, created_at) VALUES (?1, ?2, ?3)",
        params![manifest_id, SESSION_ID, current_unix_seconds()],
    )?;
    tx.execute(
        "INSERT INTO d4_s0_manifest_roots(manifest_id, root_id, operation_id, generation) VALUES (?1, ?2, ?3, ?4)",
        params![manifest_id, ROOT_ID, operation_id, fence.generation],
    )?;
    tx.execute(
        "INSERT INTO d4_s0_operations(operation_id, manifest_id, model, session_id, run_id, root_id, candidate_count, expected_generation, expected_root_revision, expected_run_revision, expected_session_revision, expected_watcher_revision, expected_library_revision, state) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 'staging')",
        params![operation_id, manifest_id, model.label(), SESSION_ID, RUN_ID, ROOT_ID, candidate_count as i64, fence.generation, fence.root_revision, fence.run_revision, fence.session_revision, fence.watcher_revision, fence.library_revision],
    )?;
    tx.commit()?;
    Ok(fence)
}

fn start_d4_writers(
    db_path: &Path,
) -> (Vec<mpsc::Sender<()>>, Vec<thread::JoinHandle<(f64, bool)>>) {
    let mut senders = Vec::new();
    let mut handles = Vec::new();
    for kind in ["metadata", "tag"] {
        let (tx_start, rx_start) = mpsc::channel::<()>();
        senders.push(tx_start);
        let path = db_path.to_path_buf();
        let kind = kind.to_string();
        handles.push(thread::spawn(move || {
            if rx_start.recv().is_err() {
                return (0.0, false);
            }
            let started = Instant::now();
            let result = (|| -> rusqlite::Result<()> {
                let mut conn = open_product_style(&path)?;
                let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                if kind == "metadata" {
                    tx.execute(
                        "UPDATE files SET context='concurrent metadata writer value' WHERE id='live-0000000'",
                        [],
                    )?;
                } else {
                    tx.execute(
                        "INSERT OR IGNORE INTO file_user_tags(file_id, tag_id, created_at) VALUES ('live-0000000', 'd4-concurrent-tag', ?1)",
                        [current_unix_seconds()],
                    )?;
                }
                tx.execute(
                    "UPDATE library_query_state SET revision=revision+1, updated_at=?1 WHERE singleton_id=1",
                    [current_unix_seconds()],
                )?;
                tx.commit()?;
                Ok(())
            })();
            (ms(started.elapsed()), result.is_ok())
        }));
    }
    (senders, handles)
}

fn stage_d4_tombstones(
    conn: &mut Connection,
    operation_id: &str,
    candidates: &[Candidate],
    writers: &[mpsc::Sender<()>],
) -> rusqlite::Result<D4StageMetrics> {
    let started = Instant::now();
    let mut metrics = D4StageMetrics::default();
    let mut writers_started = false;
    for chunk in candidates.chunks(1_000) {
        let chunk_started = Instant::now();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        {
            let mut statement = tx.prepare(
                "INSERT INTO d4_s0_staged_tombstones(operation_id, file_id, path_snapshot) VALUES (?1, ?2, ?3)",
            )?;
            for candidate in chunk {
                statement.execute(params![operation_id, candidate.id, candidate.path])?;
            }
        }
        tx.commit()?;
        metrics.max_chunk_tx_ms = metrics.max_chunk_tx_ms.max(ms(chunk_started.elapsed()));
        metrics.chunk_count += 1;
        if !writers_started {
            for signal in writers {
                let _ = signal.send(());
            }
            writers_started = true;
        }
        thread::yield_now();
    }
    metrics.elapsed_ms = ms(started.elapsed());
    Ok(metrics)
}

fn stage_d4_projection(
    conn: &mut Connection,
    root_path: &str,
    operation_id: &str,
    manifest_id: &str,
    candidates: &[Candidate],
    writers: &[mpsc::Sender<()>],
) -> rusqlite::Result<D4StageMetrics> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS temp.d4_s0_candidate_snapshot; CREATE TEMP TABLE d4_s0_candidate_snapshot(file_id TEXT PRIMARY KEY, path_snapshot TEXT NOT NULL);",
    )?;
    {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        {
            let mut statement = tx.prepare(
                "INSERT INTO temp.d4_s0_candidate_snapshot(file_id, path_snapshot) VALUES (?1, ?2)",
            )?;
            for candidate in candidates {
                statement.execute(params![candidate.id, candidate.path])?;
            }
        }
        tx.commit()?;
    }
    let mut metrics = D4StageMetrics::default();
    let started = Instant::now();
    let mut last_source_rowid = 0_i64;
    let root_desc = format!("{}/%", root_path.trim_end_matches('/'));
    let mut writers_started = false;
    loop {
        let chunk_started = Instant::now();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let inserted = tx.execute(
            "INSERT INTO d4_s0_file_projection SELECT ?1, ?2, f.rowid, f.* FROM files AS f WHERE f.rowid>?3 AND (f.path=?4 OR f.path LIKE ?5 ESCAPE '~') AND NOT EXISTS (SELECT 1 FROM temp.d4_s0_candidate_snapshot AS c WHERE c.file_id=f.id) ORDER BY f.rowid LIMIT 1000",
            params![manifest_id, ROOT_ID, last_source_rowid, root_path, root_desc],
        )?;
        if inserted == 0 {
            tx.rollback()?;
            break;
        }
        let max_source_rowid: i64 = tx.query_row(
            "SELECT MAX(source_rowid) FROM d4_s0_file_projection WHERE manifest_id=?1 AND root_id=?2",
            params![manifest_id, ROOT_ID],
            |row| row.get(0),
        )?;
        let fts_started = Instant::now();
        tx.execute(
            "INSERT INTO d4_s0_projection_fts(manifest_id, root_id, file_id, name, path) SELECT manifest_id, root_id, id, name, path FROM d4_s0_file_projection WHERE manifest_id=?1 AND root_id=?2 AND source_rowid>?3 AND source_rowid<=?4",
            params![manifest_id, ROOT_ID, last_source_rowid, max_source_rowid],
        )?;
        metrics.projection_fts_ms += ms(fts_started.elapsed());
        tx.commit()?;
        last_source_rowid = max_source_rowid;
        metrics.max_chunk_tx_ms = metrics.max_chunk_tx_ms.max(ms(chunk_started.elapsed()));
        metrics.chunk_count += 1;
        if !writers_started {
            for signal in writers {
                let _ = signal.send(());
            }
            writers_started = true;
        }
        thread::yield_now();
    }
    if !writers_started {
        for signal in writers {
            let _ = signal.send(());
        }
    }
    let _ = operation_id;
    metrics.elapsed_ms = ms(started.elapsed());
    Ok(metrics)
}

fn join_d4_writers(handles: Vec<thread::JoinHandle<(f64, bool)>>) -> D4WriterResults {
    let mut output = D4WriterResults::default();
    for handle in handles {
        let (wait_ms, ok) = handle.join().unwrap_or((0.0, false));
        if output.metadata_wait_ms == 0.0 {
            output.metadata_wait_ms = wait_ms;
            output.metadata_ok = ok;
        } else {
            output.tag_wait_ms = wait_ms;
            output.tag_ok = ok;
        }
    }
    output
}

struct D4PublishMetrics {
    published: bool,
    reason: Option<String>,
    begin_wait_ms: f64,
    final_fence_ms: f64,
    filesystem_recheck_ms: f64,
    pointer_update_ms: f64,
    commit_ms: f64,
    lock_ms: f64,
}

fn publish_d4_manifest(
    conn: &mut Connection,
    model: D4Model,
    operation_id: &str,
    manifest_id: &str,
    root: &Path,
    root_identity_before: RootIdentity,
    candidates: &[Candidate],
    check_library_revision: bool,
) -> Result<D4PublishMetrics, Box<dyn std::error::Error>> {
    let begin_call = Instant::now();
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let acquired = Instant::now();
    let begin_wait_ms = ms(acquired.duration_since(begin_call));
    let fence_start = Instant::now();
    let expected: D4Fence = tx.query_row(
        "SELECT expected_generation, expected_root_revision, expected_run_revision, expected_session_revision, expected_watcher_revision, expected_library_revision FROM d4_s0_operations WHERE operation_id=?1",
        [operation_id],
        |row| {
            Ok(D4Fence {
                generation: row.get(0)?,
                root_revision: row.get(1)?,
                run_revision: row.get(2)?,
                session_revision: row.get(3)?,
                watcher_revision: row.get(4)?,
                library_revision: row.get(5)?,
            })
        },
    )?;
    let actual = capture_d4_fence(&tx)?;
    let root_state: (Option<String>, Option<i64>, i64) = tx.query_row(
        "SELECT active_run_id, active_generation, enabled FROM scan_roots WHERE id=?1",
        [ROOT_ID],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let run_state: (String, i64, i64, i64, String) = tx.query_row(
        "SELECT status, cancel_requested, coverage_error_count, coverage_complete, lease_token FROM scan_runs WHERE id=?1",
        [RUN_ID],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;
    let session_state: (String, i64) = tx.query_row(
        "SELECT status, cancel_requested FROM scan_sessions WHERE id=?1",
        [SESSION_ID],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let fence_matches = actual.generation == expected.generation
        && actual.root_revision == expected.root_revision
        && actual.run_revision == expected.run_revision
        && actual.session_revision == expected.session_revision
        && actual.watcher_revision == expected.watcher_revision
        && (!check_library_revision || actual.library_revision == expected.library_revision)
        && root_state.0.as_deref() == Some(RUN_ID)
        && root_state.1 == Some(expected.generation)
        && root_state.2 == 1
        && run_state.0 == "running"
        && run_state.1 == 0
        && run_state.2 == 0
        && run_state.3 == 1
        && run_state.4 == "d2-lease"
        && session_state.0 == "running"
        && session_state.1 == 0;
    let snapshot_matches = match model {
        D4Model::TombstoneOverlay => {
            verify_snapshot_matches_current_candidates(&tx, &root.to_string_lossy())?
        }
        D4Model::VersionedProjection => {
            let projected: i64 = tx.query_row(
                "SELECT COUNT(*) FROM d4_s0_file_projection WHERE manifest_id=?1 AND root_id=?2",
                params![manifest_id, ROOT_ID],
                |row| row.get(0),
            )?;
            let base: i64 = tx.query_row(
                "SELECT COUNT(*) FROM files WHERE id LIKE 'missing-%' OR id LIKE 'live-%' OR id='live-control'",
                [],
                |row| row.get(0),
            )?;
            projected == base - candidates.len() as i64
        }
    };
    let final_fence_ms = ms(fence_start.elapsed());
    if !fence_matches || !snapshot_matches {
        tx.rollback()?;
        conn.execute(
            "UPDATE d4_s0_operations SET state='rejected' WHERE operation_id=?1",
            [operation_id],
        )?;
        return Ok(D4PublishMetrics {
            published: false,
            reason: Some(
                if !fence_matches {
                    "revision_or_lifecycle_fence_changed"
                } else {
                    "candidate_snapshot_changed"
                }
                .to_string(),
            ),
            begin_wait_ms,
            final_fence_ms,
            filesystem_recheck_ms: 0.0,
            pointer_update_ms: 0.0,
            commit_ms: 0.0,
            lock_ms: ms(acquired.elapsed()),
        });
    }
    let fs_metrics = verify_filesystem_boundary(root, root_identity_before, candidates)
        .map_err(|error| format!("final filesystem fence rejected publication: {error}"))?;
    let filesystem_recheck_ms = fs_metrics.elapsed_ms;
    let pointer_started = Instant::now();
    tx.execute(
        "UPDATE d4_s0_current_manifest SET manifest_id=?1 WHERE singleton_id=1",
        [manifest_id],
    )?;
    tx.execute(
        "UPDATE d4_s0_operations SET state='published' WHERE operation_id=?1 AND state='staging'",
        [operation_id],
    )?;
    tx.execute(
        "UPDATE library_query_state SET revision=revision+1, updated_at=?1 WHERE singleton_id=1",
        [current_unix_seconds()],
    )?;
    let pointer_update_ms = ms(pointer_started.elapsed());
    let commit_started = Instant::now();
    tx.commit()?;
    let commit_ms = ms(commit_started.elapsed());
    Ok(D4PublishMetrics {
        published: true,
        reason: None,
        begin_wait_ms,
        final_fence_ms,
        filesystem_recheck_ms,
        pointer_update_ms,
        commit_ms,
        lock_ms: ms(acquired.elapsed()),
    })
}

fn crash_d4_pointer(path: &str, manifest_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = open_product_style(Path::new(path))?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute(
        "UPDATE d4_s0_current_manifest SET manifest_id=?1 WHERE singleton_id=1",
        [manifest_id],
    )?;
    println!("D4_POINTER_UPDATED_UNCOMMITTED");
    std::io::stdout().flush()?;
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

fn d4_visible_file_count(
    conn: &Connection,
    model: D4Model,
    root_path: &str,
) -> rusqlite::Result<i64> {
    match model {
        D4Model::TombstoneOverlay => {
            let root_desc = format!("{}/%", root_path.trim_end_matches('/'));
            conn.query_row(
                "SELECT COUNT(*) FROM files AS f WHERE f.is_stale=0 AND (f.path=?1 OR f.path LIKE ?2 ESCAPE '~') AND NOT EXISTS (SELECT 1 FROM d4_s0_current_manifest AS current JOIN d4_s0_manifest_roots AS root ON root.manifest_id=current.manifest_id AND root.root_id=?3 JOIN d4_s0_staged_tombstones AS tombstone ON tombstone.operation_id=root.operation_id AND tombstone.file_id=f.id WHERE current.singleton_id=1)",
                params![root_path, root_desc, ROOT_ID],
                |row| row.get(0),
            )
        }
        D4Model::VersionedProjection => conn.query_row(
            "SELECT COUNT(*) FROM d4_s0_file_projection AS projection JOIN d4_s0_current_manifest AS current ON current.manifest_id=projection.manifest_id WHERE current.singleton_id=1 AND projection.root_id=?1",
            [ROOT_ID],
            |row| row.get(0),
        ),
    }
}

fn d4_search_count(
    conn: &Connection,
    model: D4Model,
    root_path: &str,
    term: &str,
) -> rusqlite::Result<i64> {
    match model {
        D4Model::TombstoneOverlay => {
            let root_desc = format!("{}/%", root_path.trim_end_matches('/'));
            conn.query_row(
                "SELECT COUNT(*) FROM files_fts JOIN files AS f ON f.rowid=files_fts.rowid WHERE files_fts MATCH ?1 AND (f.path=?2 OR f.path LIKE ?3 ESCAPE '~') AND f.is_stale=0 AND NOT EXISTS (SELECT 1 FROM d4_s0_current_manifest AS current JOIN d4_s0_manifest_roots AS root ON root.manifest_id=current.manifest_id AND root.root_id=?4 JOIN d4_s0_staged_tombstones AS tombstone ON tombstone.operation_id=root.operation_id AND tombstone.file_id=f.id WHERE current.singleton_id=1)",
                params![term, root_path, root_desc, ROOT_ID],
                |row| row.get(0),
            )
        }
        D4Model::VersionedProjection => conn.query_row(
            "SELECT COUNT(*) FROM d4_s0_projection_fts AS search JOIN d4_s0_current_manifest AS current ON current.manifest_id=search.manifest_id JOIN d4_s0_file_projection AS projection ON projection.manifest_id=search.manifest_id AND projection.root_id=search.root_id AND projection.id=search.file_id WHERE current.singleton_id=1 AND search.root_id=?1 AND d4_s0_projection_fts MATCH ?2",
            params![ROOT_ID, term],
            |row| row.get(0),
        ),
    }
}

fn d4_search_page_size(
    conn: &Connection,
    model: D4Model,
    root_path: &str,
) -> rusqlite::Result<i64> {
    match model {
        D4Model::TombstoneOverlay => {
            let root_desc = format!("{}/%", root_path.trim_end_matches('/'));
            let mut stmt = conn.prepare(
                "WITH fts_matches AS MATERIALIZED (SELECT files_fts.rowid, bm25(files_fts, 6.0, 1.5) AS rank FROM files_fts WHERE files_fts MATCH ?1) SELECT f.id FROM files AS f JOIN fts_matches AS fm ON fm.rowid=f.rowid WHERE (f.path=?2 OR f.path LIKE ?3 ESCAPE '~') AND f.is_stale=0 AND NOT EXISTS (SELECT 1 FROM d4_s0_current_manifest AS current JOIN d4_s0_manifest_roots AS root ON root.manifest_id=current.manifest_id AND root.root_id=?4 JOIN d4_s0_staged_tombstones AS tombstone ON tombstone.operation_id=root.operation_id AND tombstone.file_id=f.id WHERE current.singleton_id=1) ORDER BY fm.rank, f.mtime DESC, f.name COLLATE NOCASE ASC, f.id ASC LIMIT 25",
            )?;
            let rows = stmt.query_map(
                params!["\"needle-live-0000000\"", root_path, root_desc, ROOT_ID],
                |_| Ok(()),
            )?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?.len() as i64)
        }
        D4Model::VersionedProjection => {
            let mut stmt = conn.prepare(
                "WITH fts_matches AS MATERIALIZED (SELECT d4_s0_projection_fts.rowid, d4_s0_projection_fts.manifest_id, d4_s0_projection_fts.root_id, d4_s0_projection_fts.file_id, bm25(d4_s0_projection_fts, 6.0, 1.5, 0.0, 0.0, 0.0) AS rank FROM d4_s0_projection_fts JOIN d4_s0_current_manifest AS current ON current.manifest_id=d4_s0_projection_fts.manifest_id WHERE current.singleton_id=1 AND d4_s0_projection_fts.root_id=?1 AND d4_s0_projection_fts MATCH ?2) SELECT projection.id FROM d4_s0_file_projection AS projection JOIN fts_matches AS fm ON fm.manifest_id=projection.manifest_id AND fm.root_id=projection.root_id AND fm.file_id=projection.id WHERE projection.root_id=?1 ORDER BY fm.rank, projection.mtime DESC, projection.name COLLATE NOCASE ASC, projection.id ASC LIMIT 25",
            )?;
            let rows = stmt.query_map(params![ROOT_ID, "\"needle-live-0000000\""], |_| Ok(()))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?.len() as i64)
        }
    }
}

fn d4_query_diagnostic(db_path: &str, root_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let conn = open_product_style(Path::new(db_path))?;
    let version: String = conn.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
    println!("bundled_sqlite_version={version}");
    let root_desc = format!("{}/%", root_path.trim_end_matches('/'));
    let sqls = [
        ("fts_count", "SELECT COUNT(*) FROM files_fts WHERE files_fts MATCH 'needle'"),
        ("joined_count", "SELECT COUNT(*) FROM files_fts JOIN files AS f ON f.rowid=files_fts.rowid WHERE files_fts MATCH 'needle' AND (f.path=?1 OR f.path LIKE ?2 ESCAPE '~') AND f.is_stale=0"),
        ("overlay_count", "SELECT COUNT(*) FROM files_fts JOIN files AS f ON f.rowid=files_fts.rowid WHERE files_fts MATCH 'needle' AND (f.path=?1 OR f.path LIKE ?2 ESCAPE '~') AND f.is_stale=0 AND NOT EXISTS (SELECT 1 FROM d4_s0_current_manifest AS current JOIN d4_s0_manifest_roots AS root ON root.manifest_id=current.manifest_id AND root.root_id=?3 JOIN d4_s0_staged_tombstones AS tombstone ON tombstone.operation_id=root.operation_id AND tombstone.file_id=f.id WHERE current.singleton_id=1)"),
        ("page_name_no_overlay", "SELECT f.id FROM files_fts JOIN files AS f ON f.rowid=files_fts.rowid WHERE files_fts MATCH 'needle' AND (f.path=?1 OR f.path LIKE ?2 ESCAPE '~') AND f.is_stale=0 ORDER BY f.name LIMIT 25"),
        ("page_rank_no_overlay", "SELECT f.id FROM files_fts JOIN files AS f ON f.rowid=files_fts.rowid WHERE files_fts MATCH 'needle' AND (f.path=?1 OR f.path LIKE ?2 ESCAPE '~') AND f.is_stale=0 ORDER BY bm25(files_fts), f.name LIMIT 25"),
        ("page_rank_overlay", "SELECT f.id FROM files_fts JOIN files AS f ON f.rowid=files_fts.rowid WHERE files_fts MATCH 'needle' AND (f.path=?1 OR f.path LIKE ?2 ESCAPE '~') AND f.is_stale=0 AND NOT EXISTS (SELECT 1 FROM d4_s0_current_manifest AS current JOIN d4_s0_manifest_roots AS root ON root.manifest_id=current.manifest_id AND root.root_id=?3 JOIN d4_s0_staged_tombstones AS tombstone ON tombstone.operation_id=root.operation_id AND tombstone.file_id=f.id WHERE current.singleton_id=1) ORDER BY bm25(files_fts), f.name LIMIT 25"),
    ];
    for (label, sql) in sqls {
        let started = Instant::now();
        if label.ends_with("count") {
            let count: i64 = if label == "fts_count" {
                conn.query_row(sql, [], |row| row.get(0))?
            } else if label == "joined_count" {
                conn.query_row(sql, params![root_path, root_desc], |row| row.get(0))?
            } else {
                conn.query_row(sql, params![root_path, root_desc, ROOT_ID], |row| {
                    row.get(0)
                })?
            };
            println!(
                "{label}: result={count} elapsed_ms={:.3}",
                ms(started.elapsed())
            );
        } else {
            let mut statement = conn.prepare(sql)?;
            let mut rows = if label.ends_with("no_overlay") {
                statement.query(params![root_path, root_desc])?
            } else {
                statement.query(params![root_path, root_desc, ROOT_ID])?
            };
            let mut count = 0;
            while rows.next()?.is_some() {
                count += 1;
            }
            println!(
                "{label}: result={count} elapsed_ms={:.3}",
                ms(started.elapsed())
            );
        }
    }
    let mut plan = conn.prepare("EXPLAIN QUERY PLAN SELECT f.id FROM files_fts JOIN files AS f ON f.rowid=files_fts.rowid WHERE files_fts MATCH 'needle' AND (f.path=?1 OR f.path LIKE ?2 ESCAPE '~') AND f.is_stale=0 AND NOT EXISTS (SELECT 1 FROM d4_s0_current_manifest AS current JOIN d4_s0_manifest_roots AS root ON root.manifest_id=current.manifest_id AND root.root_id=?3 JOIN d4_s0_staged_tombstones AS tombstone ON tombstone.operation_id=root.operation_id AND tombstone.file_id=f.id WHERE current.singleton_id=1) ORDER BY bm25(files_fts), f.name LIMIT 25")?;
    let plan_rows = plan
        .query_map(params![root_path, root_desc, ROOT_ID], |row| {
            row.get::<_, String>(3)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    println!("overlay_page_plan={plan_rows:?}");
    Ok(())
}

fn d4_query_plan(
    conn: &Connection,
    model: D4Model,
    root_path: &str,
) -> rusqlite::Result<Vec<String>> {
    let sql = match model {
        D4Model::TombstoneOverlay => {
            "EXPLAIN QUERY PLAN SELECT COUNT(*) FROM files AS f WHERE f.is_stale=0 AND NOT EXISTS (SELECT 1 FROM d4_s0_current_manifest AS current JOIN d4_s0_manifest_roots AS root ON root.manifest_id=current.manifest_id AND root.root_id=?2 JOIN d4_s0_staged_tombstones AS tombstone ON tombstone.operation_id=root.operation_id AND tombstone.file_id=f.id WHERE current.singleton_id=1) AND f.path=?1"
        }
        D4Model::VersionedProjection => {
            "EXPLAIN QUERY PLAN SELECT COUNT(*) FROM d4_s0_file_projection AS projection JOIN d4_s0_current_manifest AS current ON current.manifest_id=projection.manifest_id WHERE current.singleton_id=1 AND projection.root_id=?2"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params![root_path, ROOT_ID], |row| row.get::<_, String>(3))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn d4_search_query_plan(
    conn: &Connection,
    model: D4Model,
    root_path: &str,
) -> rusqlite::Result<Vec<String>> {
    let (sql, params): (&str, Vec<rusqlite::types::Value>) = match model {
        D4Model::TombstoneOverlay => (
            "EXPLAIN QUERY PLAN WITH fts_matches AS MATERIALIZED (SELECT files_fts.rowid, bm25(files_fts, 6.0, 1.5) AS rank FROM files_fts WHERE files_fts MATCH ?1) SELECT f.id FROM files AS f JOIN fts_matches AS fm ON fm.rowid=f.rowid WHERE (f.path=?2 OR f.path LIKE ?3 ESCAPE '~') AND f.is_stale=0 AND NOT EXISTS (SELECT 1 FROM d4_s0_current_manifest AS current JOIN d4_s0_manifest_roots AS root ON root.manifest_id=current.manifest_id AND root.root_id=?4 JOIN d4_s0_staged_tombstones AS tombstone ON tombstone.operation_id=root.operation_id AND tombstone.file_id=f.id WHERE current.singleton_id=1) ORDER BY fm.rank, f.mtime DESC, f.name COLLATE NOCASE ASC, f.id ASC LIMIT 25",
            vec!["\"needle-live-0000000\"".to_string().into(), root_path.to_string().into(), format!("{}/%", root_path.trim_end_matches('/')).into(), ROOT_ID.to_string().into()],
        ),
        D4Model::VersionedProjection => (
            "EXPLAIN QUERY PLAN WITH fts_matches AS MATERIALIZED (SELECT d4_s0_projection_fts.rowid, d4_s0_projection_fts.manifest_id, d4_s0_projection_fts.root_id, d4_s0_projection_fts.file_id, bm25(d4_s0_projection_fts, 6.0, 1.5, 0.0, 0.0, 0.0) AS rank FROM d4_s0_projection_fts JOIN d4_s0_current_manifest AS current ON current.manifest_id=d4_s0_projection_fts.manifest_id WHERE current.singleton_id=1 AND d4_s0_projection_fts.root_id=?1 AND d4_s0_projection_fts MATCH ?2) SELECT projection.id FROM d4_s0_file_projection AS projection JOIN fts_matches AS fm ON fm.manifest_id=projection.manifest_id AND fm.root_id=projection.root_id AND fm.file_id=projection.id WHERE projection.root_id=?1 ORDER BY fm.rank, projection.mtime DESC, projection.name COLLATE NOCASE ASC, projection.id ASC LIMIT 25",
            vec![ROOT_ID.to_string().into(), "\"needle-live-0000000\"".to_string().into()],
        ),
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            row.get::<_, String>(3)
        })?
        .collect::<Result<Vec<_>, _>>();
    rows
}

fn d4_median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(0.0)
}

fn d4_query_timings(
    conn: &Connection,
    model: D4Model,
    root_path: &str,
) -> rusqlite::Result<serde_json::Value> {
    let mut library_times = Vec::new();
    let mut stale_fts_times = Vec::new();
    let mut search_page_times = Vec::new();
    for sample in 0..5 {
        eprintln!(
            "D4 query sample={sample} model={} library start",
            model.label()
        );
        let started = Instant::now();
        let _ = d4_visible_file_count(conn, model, root_path)?;
        library_times.push(ms(started.elapsed()));
        eprintln!(
            "D4 query sample={sample} model={} stale start",
            model.label()
        );
        let started = Instant::now();
        let _ = d4_search_count(conn, model, root_path, "stale")?;
        stale_fts_times.push(ms(started.elapsed()));
        eprintln!(
            "D4 query sample={sample} model={} page start",
            model.label()
        );
        let started = Instant::now();
        let _ = d4_search_page_size(conn, model, root_path)?;
        search_page_times.push(ms(started.elapsed()));
        eprintln!(
            "D4 query sample={sample} model={} sample complete",
            model.label()
        );
    }
    let library_count = d4_visible_file_count(conn, model, root_path)?;
    let visible_stale_hits = d4_search_count(conn, model, root_path, "stale")?;
    let visible_needle_hits = d4_search_count(conn, model, root_path, "needle")?;
    let page_size = d4_search_page_size(conn, model, root_path)?;
    eprintln!("D4 query model={} raw FTS start", model.label());
    let raw_stale_fts_hits: i64 = conn.query_row(
        "SELECT COUNT(*) FROM files_fts WHERE files_fts MATCH 'stale'",
        [],
        |row| row.get(0),
    )?;
    eprintln!("D4 query model={} plan start", model.label());
    let plan = d4_query_plan(conn, model, root_path)?;
    let search_plan = d4_search_query_plan(conn, model, root_path)?;
    eprintln!("D4 query model={} plan complete", model.label());
    Ok(serde_json::json!({
        "library_count": library_count,
        "visible_stale_search_hits": visible_stale_hits,
        "visible_needle_search_hits": visible_needle_hits,
        "search_page_size": page_size,
        "raw_schema37_fts_stale_hits": raw_stale_fts_hits,
        "library_count_ms_median_5": d4_median(&mut library_times),
        "stale_search_ms_median_5": d4_median(&mut stale_fts_times),
        "search_page_ms_median_5": d4_median(&mut search_page_times),
        "query_plan": plan,
        "search_page_query_plan": search_plan,
    }))
}

fn d4_preservation_snapshot(conn: &Connection) -> rusqlite::Result<serde_json::Value> {
    let files: i64 = conn.query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?;
    let tags: i64 = conn.query_row(
        "SELECT COUNT(*) FROM file_user_tags WHERE file_id LIKE 'missing-%'",
        [],
        |row| row.get(0),
    )?;
    let history: i64 = conn.query_row(
        "SELECT COUNT(*) FROM classification_history WHERE id='d2-history'",
        [],
        |row| row.get(0),
    )?;
    let fingerprints: i64 = conn.query_row(
        "SELECT COUNT(*) FROM file_fingerprints WHERE file_id LIKE 'missing-%'",
        [],
        |row| row.get(0),
    )?;
    let group_members: i64 = conn.query_row(
        "SELECT COUNT(*) FROM duplicate_group_members WHERE group_id=?1",
        [GROUP_ID],
        |row| row.get(0),
    )?;
    let group_status: String = conn.query_row(
        "SELECT status FROM duplicate_groups WHERE id=?1",
        [GROUP_ID],
        |row| row.get(0),
    )?;
    let artifact_status: String = conn.query_row(
        "SELECT status FROM content_artifacts WHERE id='d2-artifact'",
        [],
        |row| row.get(0),
    )?;
    let analysis_status: String = conn.query_row(
        "SELECT status FROM analysis_findings WHERE id='d2-finding'",
        [],
        |row| row.get(0),
    )?;
    let stale_flagged_rows: i64 =
        conn.query_row("SELECT COUNT(*) FROM files WHERE is_stale=1", [], |row| {
            row.get(0)
        })?;
    let context: String = conn.query_row(
        "SELECT context FROM files WHERE id='missing-0000001'",
        [],
        |row| row.get(0),
    )?;
    Ok(serde_json::json!({
        "files": files,
        "candidate_tag_links": tags,
        "classification_history_rows": history,
        "fingerprint_rows": fingerprints,
        "duplicate_group_members": group_members,
        "duplicate_group_status": group_status,
        "content_artifact_status": artifact_status,
        "analysis_finding_status": analysis_status,
        "base_files_is_stale_rows": stale_flagged_rows,
        "untouched_candidate_context": context,
    }))
}

fn run_d4_case(
    db_path: &Path,
    root: &Path,
    rows: usize,
    replicate: usize,
    model: D4Model,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    eprintln!(
        "D4 case start rows={rows} rep={replicate} model={}",
        model.label()
    );
    let root_text = root.to_string_lossy().into_owned();
    let root_identity_before =
        root_identity(root).map_err(|error| format!("capture root identity: {error}"))?;
    let mut conn = open_product_style(db_path)?;
    let baseline_projection_ms = initialize_d4_tables(&mut conn, model, &root_text)?;
    eprintln!("D4 case initialized rows={rows} model={}", model.label());
    let candidates = collect_candidates(&conn, &root_text)?;
    if candidates.len() != rows {
        return Err(format!(
            "D4 candidate mismatch: got {}, expected {rows}",
            candidates.len()
        )
        .into());
    }
    let fs_preflight = verify_filesystem_boundary(root, root_identity_before, &candidates)
        .map_err(|error| format!("D4 preflight rejected: {error}"))?;
    conn.execute_batch("DROP TABLE IF EXISTS temp.stale_candidate_snapshot;")?;
    let candidate_snapshot_prepare_ms = create_temp_snapshot(&conn, &candidates)?;
    let operation_id = format!("d4-op-{rows}-{replicate}-{}", model.label());
    let mut manifest_id = format!("manifest-{rows}-{replicate}-{}", model.label());
    create_d4_operation(&mut conn, model, &operation_id, &manifest_id, rows)?;
    let (writer_signals, writer_handles) = start_d4_writers(db_path);
    let stage = match model {
        D4Model::TombstoneOverlay => {
            stage_d4_tombstones(&mut conn, &operation_id, &candidates, &writer_signals)?
        }
        D4Model::VersionedProjection => stage_d4_projection(
            &mut conn,
            &root_text,
            &operation_id,
            &manifest_id,
            &candidates,
            &writer_signals,
        )?,
    };
    let writers = join_d4_writers(writer_handles);
    eprintln!(
        "D4 case staged rows={rows} model={} writers={writers:?}",
        model.label()
    );
    if !writers.metadata_ok
        || !writers.tag_ok
        || writers.metadata_wait_ms >= BUSY_TIMEOUT_MS as f64
        || writers.tag_wait_ms >= BUSY_TIMEOUT_MS as f64
    {
        return Err(format!("D4 ordinary writer did not finish under 5000 ms: {writers:?}").into());
    }
    let projected_metadata_mismatch = if model == D4Model::VersionedProjection {
        let projected: String = conn.query_row(
            "SELECT context FROM d4_s0_file_projection JOIN d4_s0_current_manifest USING(manifest_id) WHERE singleton_id=1 AND id='live-0000000'",
            [],
            |row| row.get(0),
        )?;
        let current: String = conn.query_row(
            "SELECT context FROM files WHERE id='live-0000000'",
            [],
            |row| row.get(0),
        )?;
        projected != current
    } else {
        false
    };
    let mut read_conn = open_product_style(db_path)?;
    let old_tx = read_conn.transaction()?;
    let old_view_before = d4_visible_file_count(&old_tx, model, &root_text)?;
    let old_stale_search_before = d4_search_count(&old_tx, model, &root_text, "stale")?;
    let first_publish = publish_d4_manifest(
        &mut conn,
        model,
        &operation_id,
        &manifest_id,
        root,
        root_identity_before,
        &candidates,
        model == D4Model::VersionedProjection,
    )?;
    let mut publish = first_publish.clone_for_json();
    let mut restage = D4StageMetrics::default();
    let mut retry_publish = None;
    if model == D4Model::VersionedProjection && !publish.published {
        let retry_operation = format!("{operation_id}-retry");
        manifest_id = format!("{manifest_id}-retry");
        conn.execute_batch("DROP TABLE IF EXISTS temp.stale_candidate_snapshot;")?;
        let _ = create_temp_snapshot(&conn, &candidates)?;
        create_d4_operation(&mut conn, model, &retry_operation, &manifest_id, rows)?;
        restage = stage_d4_projection(
            &mut conn,
            &root_text,
            &retry_operation,
            &manifest_id,
            &candidates,
            &[],
        )?;
        retry_publish = Some(publish_d4_manifest(
            &mut conn,
            model,
            &retry_operation,
            &manifest_id,
            root,
            root_identity_before,
            &candidates,
            true,
        )?);
        publish = retry_publish
            .as_ref()
            .expect("retry publish result")
            .clone_for_json();
    }
    if !publish.published {
        return Err(format!("D4 valid staged publication rejected: {:?}", publish.reason).into());
    }
    eprintln!("D4 case published rows={rows} model={}", model.label());
    let active_manifest: String = conn.query_row(
        "SELECT manifest_id FROM d4_s0_current_manifest WHERE singleton_id=1",
        [],
        |row| row.get(0),
    )?;
    let visible_after = d4_visible_file_count(&conn, model, &root_text)?;
    let old_view_after = d4_visible_file_count(&old_tx, model, &root_text)?;
    let old_stale_search_after = d4_search_count(&old_tx, model, &root_text, "stale")?;
    old_tx.commit()?;
    let query_metrics = d4_query_timings(&conn, model, &root_text)?;
    eprintln!("D4 case queried rows={rows} model={}", model.label());
    let preservation = d4_preservation_snapshot(&conn)?;
    eprintln!("D4 case preserved rows={rows} model={}", model.label());
    let pointer_revision: i64 = conn.query_row(
        "SELECT revision FROM library_query_state WHERE singleton_id=1",
        [],
        |row| row.get(0),
    )?;
    let staged_tombstones: i64 = conn.query_row(
        "SELECT COUNT(*) FROM d4_s0_staged_tombstones WHERE operation_id=?1",
        [&operation_id],
        |row| row.get(0),
    )?;
    let projection_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM d4_s0_file_projection WHERE manifest_id=?1 AND root_id=?2",
        params![manifest_id, ROOT_ID],
        |row| row.get(0),
    )?;
    let raw_tag_usage: i64 = conn.query_row(
        "SELECT COUNT(*) FROM file_user_tags WHERE tag_id=?1",
        [TAG_ID],
        |row| row.get(0),
    )?;
    let original_hashes: i64 = conn.query_row(
        "SELECT COUNT(*) FROM files WHERE id LIKE 'missing-%' AND content_hash<>''",
        [],
        |row| row.get(0),
    )?;
    let wal_bytes = file_size(&PathBuf::from(format!("{}-wal", db_path.display())));
    let expected_visible = rows as i64 + 1;
    if old_view_before != (rows as i64 * 2) + 1
        || old_view_after != old_view_before
        || old_stale_search_before != rows as i64
        || old_stale_search_after != old_stale_search_before
        || visible_after != expected_visible
        || query_metrics["visible_stale_search_hits"] != 0
        || query_metrics["visible_needle_search_hits"] != rows as i64
        || query_metrics["search_page_size"] != 1
        || query_metrics["raw_schema37_fts_stale_hits"] != rows as i64
        || preservation["candidate_tag_links"] != rows as i64
        || preservation["classification_history_rows"] != 1
        || preservation["duplicate_group_members"] != 2
        || preservation["base_files_is_stale_rows"] != 0
        || original_hashes != rows as i64
    {
        return Err("D4-S0 model contract assertion failed".into());
    }
    Ok(serde_json::json!({
        "record_type": "d4_s0_staged_publication_case",
        "source_master_head": "3275ac7c72df8ae4139a30f16c7f649971324e97",
        "schema_version": 37,
        "model": model.label(),
        "scale_candidate_stale_rows": rows,
        "total_base_files": rows * 2 + 1,
        "replicate": replicate,
        "sqlite_version": conn.query_row("SELECT sqlite_version()", [], |row| row.get::<_, String>(0))?,
        "candidate_fs_preflight": {"leaf_checks": fs_preflight.leaf_checks, "directories": fs_preflight.distinct_directories, "elapsed_ms": fs_preflight.elapsed_ms},
        "candidate_snapshot_prepare_ms": candidate_snapshot_prepare_ms,
        "baseline_projection_build_ms": baseline_projection_ms,
        "stage": {"elapsed_ms": stage.elapsed_ms, "max_chunk_tx_ms": stage.max_chunk_tx_ms, "chunk_count": stage.chunk_count, "projection_fts_ms": stage.projection_fts_ms},
        "concurrent_writers": {"metadata_wait_ms": writers.metadata_wait_ms, "metadata_committed": writers.metadata_ok, "tag_wait_ms": writers.tag_wait_ms, "tag_committed": writers.tag_ok, "busy_timeout_ms": BUSY_TIMEOUT_MS},
        "projection_metadata_snapshot_mismatch_after_writer": projected_metadata_mismatch,
        "first_publish": {"published": first_publish.published, "reason": first_publish.reason.as_deref().unwrap_or("none"), "begin_wait_ms": first_publish.begin_wait_ms, "final_fence_ms": first_publish.final_fence_ms, "filesystem_recheck_ms": first_publish.filesystem_recheck_ms, "pointer_update_ms": first_publish.pointer_update_ms, "commit_ms": first_publish.commit_ms, "lock_ms": first_publish.lock_ms},
        "projection_restage": {"elapsed_ms": restage.elapsed_ms, "max_chunk_tx_ms": restage.max_chunk_tx_ms, "chunk_count": restage.chunk_count, "projection_fts_ms": restage.projection_fts_ms, "publish": retry_publish.as_ref().map(|p| serde_json::json!({"published": p.published, "begin_wait_ms": p.begin_wait_ms, "final_fence_ms": p.final_fence_ms, "filesystem_recheck_ms": p.filesystem_recheck_ms, "pointer_update_ms": p.pointer_update_ms, "commit_ms": p.commit_ms, "lock_ms": p.lock_ms}))},
        "published_manifest": active_manifest,
        "published_visible_files": visible_after,
        "prepublish_reader_visible_files_before_and_after_commit": [old_view_before, old_view_after],
        "prepublish_reader_stale_search_before_and_after_commit": [old_stale_search_before, old_stale_search_after],
        "expected_stale_count": rows,
        "pointer_library_revision": pointer_revision,
        "staged_tombstone_rows": staged_tombstones,
        "published_projection_rows": projection_rows,
        "raw_tag_usage_count_includes_effectively_stale": raw_tag_usage,
        "candidate_hashes_preserved": original_hashes,
        "preserved_associations_and_invalidation_gap": preservation,
        "query": query_metrics,
        "wal_bytes": wal_bytes,
        "old_and_new_visibility_atomic": old_view_before == old_view_after && old_stale_search_before == old_stale_search_after && visible_after == expected_visible,
        "source_filesystem_sqlite_commit_is_atomic": false
    }))
}

impl D4PublishMetrics {
    fn clone_for_json(&self) -> Self {
        Self {
            published: self.published,
            reason: self.reason.clone(),
            begin_wait_ms: self.begin_wait_ms,
            final_fence_ms: self.final_fence_ms,
            filesystem_recheck_ms: self.filesystem_recheck_ms,
            pointer_update_ms: self.pointer_update_ms,
            commit_ms: self.commit_ms,
            lock_ms: self.lock_ms,
        }
    }
}

fn run_d4_failure_contracts(out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let root = out_dir.join("data/failure-root");
    prepare_root(&root)?;
    let db_path = out_dir.join("data/failure-base.sqlite");
    let _ = fs::remove_file(&db_path);
    remove_sidecars(&db_path)?;
    prepare_fixture(&db_path, &root, 10_000)?;
    let root_text = root.to_string_lossy().into_owned();
    let root_id = root_identity(&root).map_err(|error| format!("capture failure root: {error}"))?;
    let mut conn = open_product_style(&db_path)?;
    initialize_d4_tables(&mut conn, D4Model::TombstoneOverlay, &root_text)?;
    let candidates = collect_candidates(&conn, &root_text)?;
    conn.execute_batch("DROP TABLE IF EXISTS temp.stale_candidate_snapshot;")?;
    create_temp_snapshot(&conn, &candidates)?;
    let operation_id = "d4-failure-op";
    let manifest_id = "d4-failure-manifest";
    create_d4_operation(
        &mut conn,
        D4Model::TombstoneOverlay,
        operation_id,
        manifest_id,
        candidates.len(),
    )?;
    stage_d4_tombstones(&mut conn, operation_id, &candidates, &[])?;
    drop(conn);
    let mut conn = open_product_style(&db_path)?;
    conn.execute_batch("DROP TABLE IF EXISTS temp.stale_candidate_snapshot;")?;
    create_temp_snapshot(&conn, &candidates)?;
    let persisted_stage_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM d4_s0_staged_tombstones WHERE operation_id=?1",
        [operation_id],
        |row| row.get(0),
    )?;
    let pre_crash_pointer: String = conn.query_row(
        "SELECT manifest_id FROM d4_s0_current_manifest WHERE singleton_id=1",
        [],
        |row| row.get(0),
    )?;
    let interrupted_stage_visible: i64 =
        d4_visible_file_count(&conn, D4Model::TombstoneOverlay, &root_text)?;

    let mut rejected = Vec::new();
    conn.execute("UPDATE scan_roots SET watcher_revision=watcher_revision+1, revision=revision+1 WHERE id=?1", [ROOT_ID])?;
    let watcher = publish_d4_manifest(
        &mut conn,
        D4Model::TombstoneOverlay,
        operation_id,
        manifest_id,
        &root,
        root_id,
        &candidates,
        false,
    )?;
    rejected.push(!watcher.published);
    conn.execute(
        "UPDATE scan_roots SET watcher_revision=0, revision=1 WHERE id=?1",
        [ROOT_ID],
    )?;

    conn.execute(
        "UPDATE scan_runs SET cancel_requested=1 WHERE id=?1",
        [RUN_ID],
    )?;
    let run_cancel = publish_d4_manifest(
        &mut conn,
        D4Model::TombstoneOverlay,
        operation_id,
        manifest_id,
        &root,
        root_id,
        &candidates,
        false,
    )?;
    rejected.push(!run_cancel.published);
    conn.execute(
        "UPDATE scan_runs SET cancel_requested=0 WHERE id=?1",
        [RUN_ID],
    )?;

    conn.execute(
        "UPDATE scan_sessions SET cancel_requested=1 WHERE id=?1",
        [SESSION_ID],
    )?;
    let session_cancel = publish_d4_manifest(
        &mut conn,
        D4Model::TombstoneOverlay,
        operation_id,
        manifest_id,
        &root,
        root_id,
        &candidates,
        false,
    )?;
    rejected.push(!session_cancel.published);
    conn.execute(
        "UPDATE scan_sessions SET cancel_requested=0 WHERE id=?1",
        [SESSION_ID],
    )?;

    conn.execute(
        "UPDATE scan_roots SET current_generation=2, revision=2 WHERE id=?1",
        [ROOT_ID],
    )?;
    let generation = publish_d4_manifest(
        &mut conn,
        D4Model::TombstoneOverlay,
        operation_id,
        manifest_id,
        &root,
        root_id,
        &candidates,
        false,
    )?;
    rejected.push(!generation.published);
    conn.execute(
        "UPDATE scan_roots SET current_generation=1, revision=1 WHERE id=?1",
        [ROOT_ID],
    )?;

    conn.execute(
        "UPDATE scan_runs SET revision=revision+1 WHERE id=?1",
        [RUN_ID],
    )?;
    let run_revision = publish_d4_manifest(
        &mut conn,
        D4Model::TombstoneOverlay,
        operation_id,
        manifest_id,
        &root,
        root_id,
        &candidates,
        false,
    )?;
    rejected.push(!run_revision.published);
    conn.execute("UPDATE scan_runs SET revision=1 WHERE id=?1", [RUN_ID])?;

    conn.execute(
        "UPDATE scan_sessions SET revision=revision+1 WHERE id=?1",
        [SESSION_ID],
    )?;
    let session_revision = publish_d4_manifest(
        &mut conn,
        D4Model::TombstoneOverlay,
        operation_id,
        manifest_id,
        &root,
        root_id,
        &candidates,
        false,
    )?;
    rejected.push(!session_revision.published);
    conn.execute(
        "UPDATE scan_sessions SET revision=1 WHERE id=?1",
        [SESSION_ID],
    )?;

    let first_candidate = PathBuf::from(&candidates[0].path);
    fs::write(&first_candidate, b"reappeared during staged scan")?;
    let reappeared_rejected = publish_d4_manifest(
        &mut conn,
        D4Model::TombstoneOverlay,
        operation_id,
        manifest_id,
        &root,
        root_id,
        &candidates,
        false,
    )
    .is_err();
    fs::remove_file(&first_candidate)?;

    let original_dir = root.join("branch-000");
    let saved_dir = root.join("branch-000.saved");
    fs::rename(&original_dir, &saved_dir)?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("branch-001"), &original_dir)?;
    #[cfg(not(unix))]
    fs::rename(&saved_dir, &original_dir)?;
    #[cfg(unix)]
    let symlink_parent_rejected = verify_filesystem_boundary(&root, root_id, &candidates).is_err();
    #[cfg(not(unix))]
    let symlink_parent_rejected = false;
    #[cfg(unix)]
    {
        fs::remove_file(&original_dir)?;
        fs::rename(&saved_dir, &original_dir)?;
    }

    let pointer_after_fences: String = conn.query_row(
        "SELECT manifest_id FROM d4_s0_current_manifest WHERE singleton_id=1",
        [],
        |row| row.get(0),
    )?;
    let rollback_tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    rollback_tx.execute("UPDATE d4_s0_current_manifest SET manifest_id='rollback-must-not-publish' WHERE singleton_id=1", [])?;
    let constraint_failed = rollback_tx
        .execute(
            "INSERT INTO d4_s0_current_manifest(singleton_id,manifest_id) VALUES (2,'invalid')",
            [],
        )
        .is_err();
    rollback_tx.rollback()?;
    let pointer_after_rollback: String = conn.query_row(
        "SELECT manifest_id FROM d4_s0_current_manifest WHERE singleton_id=1",
        [],
        |row| row.get(0),
    )?;

    drop(conn);
    let mut child = Command::new(env::current_exe()?)
        .arg("--crash-d4-pointer")
        .arg(&db_path)
        .arg("crash-uncommitted-manifest")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().ok_or("crash child has no stdout")?;
    let mut reader = BufReader::new(stdout);
    let mut ready = String::new();
    reader.read_line(&mut ready)?;
    let crash_ready = ready.trim() == "D4_POINTER_UPDATED_UNCOMMITTED";
    child.kill()?;
    let _ = child.wait()?;
    let conn = open_product_style(&db_path)?;
    let pointer_after_crash: String = conn.query_row(
        "SELECT manifest_id FROM d4_s0_current_manifest WHERE singleton_id=1",
        [],
        |row| row.get(0),
    )?;
    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    let old_view_after_crash = d4_visible_file_count(&conn, D4Model::TombstoneOverlay, &root_text)?;

    let old_root_a = Some("old-a".to_string());
    let old_root_b = Some("old-b".to_string());
    conn.execute("INSERT INTO d4_s0_manifest_roots(manifest_id,root_id,operation_id,generation) VALUES ('manifest-0','root-a','old-a',4)", [])?;
    conn.execute("INSERT INTO d4_s0_manifest_roots(manifest_id,root_id,operation_id,generation) VALUES ('manifest-0','root-b','old-b',9)", [])?;
    conn.execute("INSERT INTO d4_s0_manifests(manifest_id,session_id,created_at) VALUES ('multi-root-new',?1,?2)", params![SESSION_ID,current_unix_seconds()])?;
    conn.execute("INSERT INTO d4_s0_manifest_roots(manifest_id,root_id,operation_id,generation) VALUES ('multi-root-new','root-a','new-a',5)", [])?;
    conn.execute("INSERT INTO d4_s0_manifest_roots(manifest_id,root_id,operation_id,generation) VALUES ('multi-root-new','root-b','new-b',10)", [])?;
    let mut multi_read_conn = open_product_style(&db_path)?;
    let multi_reader = multi_read_conn.transaction()?;
    let before_pair: (Option<String>, Option<String>) = multi_reader.query_row(
        "SELECT (SELECT operation_id FROM d4_s0_manifest_roots WHERE manifest_id=current.manifest_id AND root_id='root-a'), (SELECT operation_id FROM d4_s0_manifest_roots WHERE manifest_id=current.manifest_id AND root_id='root-b') FROM d4_s0_current_manifest AS current WHERE singleton_id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    conn.execute(
        "UPDATE d4_s0_current_manifest SET manifest_id='multi-root-new' WHERE singleton_id=1",
        [],
    )?;
    let after_old_reader: (Option<String>, Option<String>) = multi_reader.query_row(
        "SELECT (SELECT operation_id FROM d4_s0_manifest_roots WHERE manifest_id=current.manifest_id AND root_id='root-a'), (SELECT operation_id FROM d4_s0_manifest_roots WHERE manifest_id=current.manifest_id AND root_id='root-b') FROM d4_s0_current_manifest AS current WHERE singleton_id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    multi_reader.commit()?;
    let after_pair: (Option<String>, Option<String>) = conn.query_row(
        "SELECT (SELECT operation_id FROM d4_s0_manifest_roots WHERE manifest_id=current.manifest_id AND root_id='root-a'), (SELECT operation_id FROM d4_s0_manifest_roots WHERE manifest_id=current.manifest_id AND root_id='root-b') FROM d4_s0_current_manifest AS current WHERE singleton_id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let multi_root_atomic = before_pair == (old_root_a, old_root_b)
        && after_old_reader == before_pair
        && after_pair == (Some("new-a".to_string()), Some("new-b".to_string()));
    let all_fences_rejected = rejected.iter().all(|value| *value);
    let passed = all_fences_rejected
        && reappeared_rejected
        && symlink_parent_rejected
        && pointer_after_fences == "manifest-0"
        && constraint_failed
        && pointer_after_rollback == "manifest-0"
        && crash_ready
        && pointer_after_crash == "manifest-0"
        && integrity == "ok"
        && old_view_after_crash == 20_001
        && persisted_stage_rows == 10_000
        && pre_crash_pointer == "manifest-0"
        && interrupted_stage_visible == 20_001
        && multi_root_atomic;
    let record = serde_json::json!({
        "record_type": "d4_s0_failure_contracts",
        "schema_version": 37,
        "stage_chunks_survive_restart_before_publish": persisted_stage_rows == 10_000 && interrupted_stage_visible == 20_001,
        "fence_rejections": {"watcher_revision": rejected[0], "run_cancel": rejected[1], "session_cancel": rejected[2], "root_generation": rejected[3], "run_revision": rejected[4], "session_revision": rejected[5]},
        "filesystem_reappearance_rejected": reappeared_rejected,
        "symlink_parent_chain_rejected_linux": symlink_parent_rejected,
        "pointer_unchanged_after_fences": pointer_after_fences,
        "constraint_failure_rolled_back_pointer": constraint_failed && pointer_after_rollback == "manifest-0",
        "process_killed_after_uncommitted_pointer_update": crash_ready,
        "pointer_after_crash": pointer_after_crash,
        "integrity_check_after_crash": integrity,
        "visible_count_after_crash": old_view_after_crash,
        "multi_root_manifest_mapping": {"old_reader_before": before_pair, "old_reader_after": after_old_reader, "new_reader_after": after_pair, "atomic": multi_root_atomic},
        "passed": passed,
        "filesystem_to_sqlite_toctou_closed": false
    });
    append_json_line(&out_dir.join("failure-contracts.json"), &record)?;
    println!("{}", record);
    drop(conn);
    drop(multi_read_conn);
    let _ = fs::remove_file(&db_path);
    remove_sidecars(&db_path)?;
    fs::remove_dir_all(&root)?;
    if !passed {
        return Err("D4-S0 failure contract failed".into());
    }
    Ok(())
}

fn run_fts_contract(out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let path = out_dir.join("data/fts-trigger-contract.sqlite");
    let _ = fs::remove_file(&path);
    db::initialize(path.to_str().expect("utf-8 path"))?;
    let conn = open_product_style(&path)?;
    install_scoped_fts_trigger(&conn)?;
    let file_path = out_dir.join("data/fts-insert-before.txt");
    let renamed_path = out_dir.join("data/fts-insert-after.txt");
    conn.execute(
        "INSERT INTO files (id, path, name) VALUES ('fts-contract', ?1, 'insertneedlebefore.txt')",
        [file_path.to_string_lossy().as_ref()],
    )?;
    let insert_match: i64 = conn.query_row(
        "SELECT count(*) FROM files_fts WHERE files_fts MATCH 'insertneedlebefore'",
        [],
        |row| row.get(0),
    )?;
    conn.execute(
        "UPDATE files SET path=?1, name='insertneedleafter.txt' WHERE id='fts-contract'",
        [renamed_path.to_string_lossy().as_ref()],
    )?;
    let old_rename_match: i64 = conn.query_row(
        "SELECT count(*) FROM files_fts WHERE files_fts MATCH 'insertneedlebefore'",
        [],
        |row| row.get(0),
    )?;
    let new_rename_match: i64 = conn.query_row(
        "SELECT count(*) FROM files_fts WHERE files_fts MATCH 'insertneedleafter'",
        [],
        |row| row.get(0),
    )?;
    conn.execute("DELETE FROM files WHERE id='fts-contract'", [])?;
    let delete_match: i64 = conn.query_row(
        "SELECT count(*) FROM files_fts WHERE files_fts MATCH 'insertneedleafter'",
        [],
        |row| row.get(0),
    )?;
    let record = serde_json::json!({
        "record_type": "conditional_fts_contract",
        "schema_version": 37,
        "sqlite_version": conn.query_row("SELECT sqlite_version()", [], |row| row.get::<_, String>(0))?,
        "insert_indexes_search_terms": insert_match == 1,
        "rename_removes_old_terms": old_rename_match == 0,
        "rename_indexes_new_terms": new_rename_match == 1,
        "delete_removes_terms": delete_match == 0,
        "raw_counts": {"insert": insert_match, "old_after_rename": old_rename_match, "new_after_rename": new_rename_match, "after_delete": delete_match},
        "passed": insert_match == 1 && old_rename_match == 0 && new_rename_match == 1 && delete_match == 0,
    });
    append_json_line(&out_dir.join("fts-contract.json"), &record)?;
    println!("{}", record);
    drop(conn);
    let _ = fs::remove_file(&path);
    remove_sidecars(&path)?;
    Ok(())
}

fn crash_after_update(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let conn = open_product_style(Path::new(path))?;
    conn.execute_batch("BEGIN IMMEDIATE")?;
    conn.execute("UPDATE files SET is_stale=1 WHERE id LIKE 'missing-%'", [])?;
    conn.execute(
        "UPDATE library_query_state SET revision=revision+1 WHERE singleton_id=1",
        [],
    )?;
    println!("AFTER_UNCOMMITTED_STALE_UPDATE");
    std::io::stdout().flush()?;
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

fn append_json_line(
    path: &Path,
    value: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", serde_json::to_string(value)?)?;
    file.flush()?;
    Ok(())
}

fn restore_control_file(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let before = root.join("branch-000/renametestbefore.txt");
    let after = root.join("branch-000/renametestafter.txt");
    if after.exists() && !before.exists() {
        fs::rename(after, before)?;
    }
    Ok(())
}

fn remove_sidecars(db_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for suffix in ["-wal", "-shm"] {
        let path = PathBuf::from(format!("{}{}", db_path.display(), suffix));
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}
