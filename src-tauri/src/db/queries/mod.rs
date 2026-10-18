use super::*;
use crate::path_filter::{metadata_is_link_or_reparse, ManagedPathFilter};
use rusqlite::{params, types::Value as SqlValue, Connection, OptionalExtension, Row};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Runtime};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

pub(crate) mod analysis;
pub(crate) mod dedupe;
pub(crate) mod files;
pub(crate) mod library;
pub(crate) mod library_identity;
pub(crate) mod library_shadow;
pub(crate) mod operations;
pub(crate) mod organization;
pub(crate) mod rule_proposals;
pub(crate) mod rules_repo;
pub mod scan;

use rules_repo::current_timestamp_iso;

pub(crate) const WATCHER_DEEP_UPSERT_ENTRY_LIMIT: usize = 5000;
const LEGACY_UNASSIGNED_ARCHIVE_PATH_PREFIX: &str = "zen-canvas://legacy-unassigned/";

fn is_legacy_unassigned_archive_path(path: &str) -> bool {
    path.starts_with(LEGACY_UNASSIGNED_ARCHIVE_PATH_PREFIX)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatcherUpsertWarning {
    pub message: String,
    pub path: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone)]
pub struct WatcherUpsertResult {
    pub upserted: usize,
    pub warnings: Vec<WatcherUpsertWarning>,
}

pub(crate) fn trim_trailing_path_separators(path: &str) -> &str {
    let mut end = path.len();
    while end > 1 {
        let current = &path[..end];
        if !(current.ends_with('\\') || current.ends_with('/')) {
            break;
        }
        if end == 3 && current.as_bytes().get(1) == Some(&b':') {
            break;
        }
        end -= 1;
    }
    &path[..end]
}

fn escape_like_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '~' | '%' | '_') {
            escaped.push('~');
        }
        escaped.push(ch);
    }
    escaped
}

#[derive(Debug, Clone)]
pub(crate) struct ScopedFilesSql {
    pub(crate) cte: String,
    pub(crate) params: Vec<SqlValue>,
}

#[derive(Debug, Clone)]
struct ScopePathSql {
    clause: String,
    params: Vec<SqlValue>,
}

pub(crate) fn scoped_files_sql(scope: Option<&LibraryScope>) -> ScopedFilesSql {
    scoped_files_sql_with_extra_where(scope, None)
}

pub(crate) fn scoped_files_sql_with_extra_where(
    scope: Option<&LibraryScope>,
    extra_where: Option<&str>,
) -> ScopedFilesSql {
    let filter = scope_path_filter(scope, "f");
    let scope_clause = if filter.clause.is_empty() {
        String::new()
    } else {
        format!(" AND ({})", filter.clause)
    };
    let extra_clause = extra_where
        .filter(|clause| !clause.trim().is_empty())
        .map(|clause| format!(" AND ({clause})"))
        .unwrap_or_default();

    ScopedFilesSql {
        cte: format!(
            "scoped_files AS (SELECT f.rowid AS rowid, f.* FROM files AS f WHERE f.is_stale = 0{scope_clause}{extra_clause})"
        ),
        params: filter.params,
    }
}

fn scope_path_filter(scope: Option<&LibraryScope>, alias: &str) -> ScopePathSql {
    let Some(roots) = scope_roots(scope) else {
        return ScopePathSql {
            clause: String::new(),
            params: Vec::new(),
        };
    };

    let root_candidates = normalized_scope_root_candidates(roots);
    if root_candidates.is_empty() {
        return ScopePathSql {
            clause: "1 = 0".to_string(),
            params: Vec::new(),
        };
    }

    let mut clauses = Vec::with_capacity(root_candidates.len());
    let mut params = Vec::with_capacity(root_candidates.len() * 3);
    for candidate in root_candidates {
        let escaped_path = escape_like_pattern(&candidate);
        clauses.push(format!(
            "({alias}.path = ? OR {alias}.path LIKE ? ESCAPE '~' OR {alias}.path LIKE ? ESCAPE '~')"
        ));
        params.push(SqlValue::Text(candidate));
        params.push(SqlValue::Text(descendant_like_pattern(&escaped_path, '/')));
        params.push(SqlValue::Text(descendant_like_pattern(&escaped_path, '\\')));
    }

    ScopePathSql {
        clause: clauses.join(" OR "),
        params,
    }
}

fn scope_roots(scope: Option<&LibraryScope>) -> Option<&[String]> {
    match scope {
        None | Some(LibraryScope::All) => None,
        Some(LibraryScope::CurrentScan { roots, .. }) | Some(LibraryScope::Roots { roots }) => {
            Some(roots.as_slice())
        }
    }
}

fn normalized_scope_root_candidates(roots: &[String]) -> Vec<String> {
    let mut candidates = Vec::new();
    for root in roots
        .iter()
        .map(|root| root.trim())
        .filter(|root| !root.is_empty())
    {
        let root = trim_trailing_path_separators(root);
        if root.is_empty() {
            continue;
        }
        let normalized_root = normalize_path_text(root);
        for candidate in path_lookup_candidates(root, &normalized_root) {
            push_unique(&mut candidates, candidate);
        }
    }
    candidates
}

fn descendant_like_pattern(escaped_path: &str, separator: char) -> String {
    if escaped_path.ends_with('/') || escaped_path.ends_with('\\') {
        format!("{escaped_path}%")
    } else {
        format!("{escaped_path}{separator}%")
    }
}

pub(crate) fn build_fts_query(input: &str) -> Option<String> {
    let phrases = input
        .split_whitespace()
        .filter(|phrase| !phrase.is_empty())
        .take(12)
        .map(quote_fts_phrase)
        .collect::<Vec<_>>();

    if phrases.is_empty() {
        None
    } else {
        Some(phrases.join(" AND "))
    }
}

fn quote_fts_phrase(phrase: &str) -> String {
    format!("\"{}\"", phrase.replace('"', "\"\""))
}

pub(crate) fn bool_to_i64(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

pub(crate) fn path_lookup_candidates(first: &str, second: &str) -> Vec<String> {
    path_lookup_candidates_for_values(&[first, second])
}

fn path_lookup_candidates_for_values(values: &[&str]) -> Vec<String> {
    let mut candidates = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        push_unique(&mut candidates, trimmed.to_string());
        push_unique(&mut candidates, normalize_path_text(trimmed));
    }
    candidates
}

pub(crate) fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|item| item == &value) {
        values.push(value);
    }
}

fn normalize_path_for_db(path: &Path) -> String {
    normalize_path_text(&path.to_string_lossy())
}

pub(crate) fn normalize_path_text(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    if let Some(stripped) = normalized.strip_prefix("//?/UNC/") {
        return format!("//{stripped}");
    }
    if let Some(stripped) = normalized.strip_prefix("//?/") {
        return stripped.to_string();
    }
    normalized
}

fn resolved_file_name(target_path: &str, new_name: &str) -> String {
    let trimmed = new_name.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }

    target_path
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(target_path)
        .to_string()
}

fn extension_from_file_name(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_string()
}

fn find_active_file_row_id_by_path(
    conn: &Connection,
    path_candidates: &[String],
) -> Result<Option<String>, DbError> {
    for candidate in path_candidates {
        let found = conn
            .query_row(
                "SELECT id FROM files WHERE path = ?1 AND is_stale = 0 ORDER BY id LIMIT 1",
                params![candidate],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}

fn find_active_file_row_id_by_identity(
    conn: &Connection,
    identity_candidates: &[String],
) -> Result<Option<String>, DbError> {
    for candidate in identity_candidates {
        let found = conn
            .query_row(
                "SELECT id FROM files WHERE id = ?1 AND is_stale = 0 \
                 AND path NOT LIKE 'zen-canvas://legacy-unassigned/%'",
                params![candidate],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}

fn find_active_file_row_id(
    conn: &Connection,
    path_candidates: &[String],
    identity_candidates: &[String],
) -> Result<Option<String>, DbError> {
    if let Some(file_id) = find_active_file_row_id_by_path(conn, path_candidates)? {
        return Ok(Some(file_id));
    }
    find_active_file_row_id_by_identity(conn, identity_candidates)
}

#[cfg(test)]
mod active_file_resolution_tests {
    use super::*;

    #[test]
    fn active_path_resolution_wins_over_archived_primary_key_match() {
        let conn = Connection::open_in_memory().expect("open lookup fixture");
        conn.execute_batch(
            "CREATE TABLE files (id TEXT PRIMARY KEY, path TEXT UNIQUE, is_stale INTEGER);
             INSERT INTO files (id, path, is_stale) VALUES
               ('/managed/a.txt', 'zen-canvas://legacy-unassigned/old', 1),
               ('source-current', '/managed/a.txt', 0);",
        )
        .expect("seed archived and active rows");

        assert_eq!(
            find_active_file_row_id(
                &conn,
                &["/managed/a.txt".to_string()],
                &["/managed/a.txt".to_string()],
            )
            .expect("resolve active path"),
            Some("source-current".to_string())
        );

        conn.execute(
            "UPDATE files SET is_stale = 0 WHERE id = '/managed/a.txt'",
            [],
        )
        .expect("simulate an incorrectly un-staled archive row");
        assert_eq!(
            find_active_file_row_id(&conn, &[], &["/managed/a.txt".to_string()])
                .expect("resolve explicit active identity"),
            None
        );
    }
}

fn system_time_to_unix_seconds(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
}

pub(crate) fn current_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

pub fn upsert_files_by_paths_for_db(db: &Database, paths: &[String]) -> Result<usize, DbError> {
    Ok(upsert_files_by_paths_for_db_with_warnings(db, paths)?.upserted)
}

pub fn upsert_files_by_paths_for_db_with_warnings(
    db: &Database,
    paths: &[String],
) -> Result<WatcherUpsertResult, DbError> {
    let mut files = Vec::new();
    let mut seen = Vec::new();
    let mut warnings = Vec::new();
    let roots = db.list_watcher_root_configs()?;

    for raw_path in paths
        .iter()
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
    {
        let path = trim_trailing_path_separators(raw_path);
        if path.is_empty() {
            continue;
        }

        let matching_roots = roots
            .iter()
            .filter(|root| scan::watcher_path_within_root(&root.path, path))
            .collect::<Vec<_>>();
        let selected_root = match matching_roots.as_slice() {
            [root] => Some(Path::new(&root.path)),
            [] => None,
            _ => return Err(DbError::Validation(
                "Bulk upsert path maps to overlapping managed roots; reconciliation is required."
                    .into(),
            )),
        };
        collect_upsert_requests_for_path(
            PathBuf::from(path),
            selected_root,
            &mut files,
            &mut seen,
            &mut warnings,
            WATCHER_DEEP_UPSERT_ENTRY_LIMIT,
        )?;
    }

    let upserted = files.len();
    if upserted > 0 {
        db.insert_files(&files)?;
    }
    Ok(WatcherUpsertResult { upserted, warnings })
}

fn collect_upsert_requests_for_path(
    path: PathBuf,
    selected_root: Option<&Path>,
    files: &mut Vec<InsertFileRequest>,
    seen: &mut Vec<String>,
    warnings: &mut Vec<WatcherUpsertWarning>,
    limit: usize,
) -> Result<(), DbError> {
    let path_filter = match selected_root {
        Some(root) => ManagedPathFilter::new(root),
        None if path.is_dir() => ManagedPathFilter::new(&path),
        None => ManagedPathFilter::unselected(&path),
    };
    // Keep verified selected-root authority for both lexical and resolved input.
    // Resolving the input also retains protected/dependency checks when `..` or
    // an alias leads outside the lexical root; the ancestor check rejects links.
    if path_filter.excludes(&path, path.is_dir())
        || path
            .canonicalize()
            .ok()
            .is_some_and(|resolved| path_filter.excludes(&resolved, path.is_dir()))
    {
        return Ok(());
    }
    if !path_filter.direct_path_has_no_links(&path)? {
        return Err(DbError::Validation(
            "Bulk upsert cannot follow a linked ancestor inside the selected root.".into(),
        ));
    }
    let mut stack = vec![path];

    while let Some(path_buf) = stack.pop() {
        if files.len() >= limit {
            let path = normalize_path_for_db(&path_buf);
            let message = format!(
                "This directory has too many entries; only part of the directory was indexed. Run a full scan manually. Limit: {limit}."
            );
            eprintln!("{message}");
            warnings.push(WatcherUpsertWarning {
                message,
                path: Some(path),
                limit,
            });
            break;
        }

        let metadata = match fs::symlink_metadata(&path_buf) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(DbError::Io(error)),
        };
        if path_filter.excludes(&path_buf, metadata.is_dir())
            || metadata_is_link_or_reparse(&metadata)
        {
            continue;
        }

        let normalized_path = normalize_path_for_db(&path_buf);
        if seen.iter().any(|value| value == &normalized_path) {
            continue;
        }
        push_unique(seen, normalized_path.clone());

        files.push(insert_request_from_metadata(
            normalized_path,
            &path_buf,
            &metadata,
        ));

        if !metadata.is_dir() {
            continue;
        }

        let entries = match fs::read_dir(&path_buf) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(DbError::Io(error)),
        };

        for entry in entries {
            let entry = entry.map_err(DbError::Io)?;
            let file_type = entry.file_type().map_err(DbError::Io)?;
            if file_type.is_symlink() {
                continue;
            }
            if path_filter.excludes(&entry.path(), file_type.is_dir()) {
                continue;
            }
            stack.push(entry.path());
        }
    }

    Ok(())
}

pub(crate) fn upsert_files_by_paths_with_optional_optimize(
    db: &Database,
    paths: &[String],
) -> Result<usize, DbError> {
    let upserted = upsert_files_by_paths_for_db(db, paths)?;
    let _ = optimize_search_index_after_bulk_upsert(db, upserted);
    Ok(upserted)
}

pub(crate) fn optimize_search_index_after_bulk_upsert(
    db: &Database,
    upserted: usize,
) -> Option<SearchIndexOptimizeReport> {
    if upserted >= OPTIMIZE_AFTER_UPSERT_THRESHOLD {
        Some(run_search_index_optimize("watcher_bulk_upsert", db))
    } else {
        None
    }
}

pub fn run_search_index_optimize(trigger: &str, db: &Database) -> SearchIndexOptimizeReport {
    let started = Instant::now();
    match db.optimize_search_index() {
        Ok(duration_ms) => SearchIndexOptimizeReport {
            trigger: trigger.to_string(),
            duration_ms,
            success: true,
            error: None,
        },
        Err(error) => {
            #[cfg(all(test, feature = "performance-test-tauri"))]
            if let DbError::Sqlite(rusqlite::Error::SqliteFailure(code, _)) = &error {
                eprintln!(
                    "[issue345-sqlite] operation=optimize_search_index trigger={} sqlite_primary={} sqlite_extended={}",
                    trigger,
                    code.extended_code & 0xff,
                    code.extended_code
                );
            }
            let message = error.to_string();
            eprintln!("SQLite/FTS optimize failed for {trigger}: {message}");
            SearchIndexOptimizeReport {
                trigger: trigger.to_string(),
                duration_ms: started.elapsed().as_millis(),
                success: false,
                error: Some(message),
            }
        }
    }
}

pub fn emit_search_index_optimized<R: Runtime>(
    app: &AppHandle<R>,
    report: &SearchIndexOptimizeReport,
) {
    if let Err(error) = app.emit(SEARCH_INDEX_OPTIMIZED_EVENT, report) {
        eprintln!(
            "Failed to emit {SEARCH_INDEX_OPTIMIZED_EVENT} event for {}: {error}",
            report.trigger
        );
    }
}

fn insert_request_from_metadata(
    normalized_path: String,
    path: &Path,
    metadata: &fs::Metadata,
) -> InsertFileRequest {
    let is_dir = metadata.is_dir();
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| resolved_file_name(&normalized_path, ""));
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let size = if metadata.is_file() {
        i64::try_from(metadata.len()).unwrap_or(i64::MAX)
    } else {
        0
    };
    let mtime = metadata
        .modified()
        .ok()
        .and_then(system_time_to_unix_seconds)
        .unwrap_or_else(current_unix_seconds);
    let ctime = metadata
        .created()
        .ok()
        .and_then(system_time_to_unix_seconds)
        .unwrap_or(mtime);

    InsertFileRequest {
        id: normalized_path.clone(),
        path: normalized_path,
        name,
        extension,
        size,
        mtime,
        ctime,
        is_dir,
        state_code: 0,
    }
}

pub(crate) fn indexed_file_from_row(row: &Row<'_>) -> rusqlite::Result<IndexedFileRow> {
    Ok(IndexedFileRow {
        id: row.get(0)?,
        path: row.get(1)?,
        name: row.get(2)?,
        extension: row.get(3)?,
        size: row.get(4)?,
        mtime: row.get(5)?,
        ctime: row.get(6)?,
        is_dir: row.get::<_, i64>(7)? != 0,
        state_code: row.get(8)?,
        file_type: row.get(9)?,
        purpose: row.get(10)?,
        lifecycle: row.get(11)?,
        context: row.get(12)?,
        risk_level: row.get(13)?,
        suggested_action: row.get(14)?,
        suggested_target_path: row.get(15)?,
        suggested_name: row.get(16)?,
        confidence: row.get(17)?,
        classification_reason: row.get(18)?,
        classification_status: row.get(19)?,
        matched_rules: row.get(20)?,
        requires_confirmation: row.get::<_, i64>(21)? != 0,
        content_hash: row.get(22)?,
        is_duplicate: row.get::<_, i64>(23)? != 0,
        is_stale: row.get::<_, i64>(24)? != 0,
        last_seen_at: row.get(25)?,
        last_classified_at: row.get(26)?,
        classified_rule_version: row.get(27)?,
        last_classified_mtime: row.get(28)?,
        last_classified_size: row.get(29)?,
    })
}

fn file_record_from_indexed(row: IndexedFileRow, now: &str) -> FileRecordDto {
    let created_at = unix_seconds_to_iso(if row.ctime == 0 { row.mtime } else { row.ctime });
    let modified_at = unix_seconds_to_iso(row.mtime);
    let last_seen_at = unix_seconds_to_iso(if row.last_seen_at == 0 {
        row.mtime
    } else {
        row.last_seen_at
    });
    let file_type = normalized_file_type(&row);
    let matched_rules = serde_json::from_str::<Vec<String>>(&row.matched_rules).unwrap_or_default();

    FileRecordDto {
        id: row.id,
        name: row.name.clone(),
        path: row.path.clone(),
        directory: parent_directory(&row.path),
        extension: row.extension,
        size: row.size,
        file_type,
        purpose: row.purpose,
        lifecycle: row.lifecycle,
        context: row.context,
        risk_level: row.risk_level,
        hash: (!row.content_hash.is_empty()).then_some(row.content_hash.clone()),
        created_at,
        modified_at,
        scanned_at: now.to_string(),
        last_seen_at,
        is_hidden: row.name.starts_with('.'),
        is_deleted: false,
        is_duplicate: row.is_duplicate,
        suggested_action: row.suggested_action,
        suggested_target_path: row.suggested_target_path,
        suggested_name: if row.suggested_name.is_empty() {
            row.name
        } else {
            row.suggested_name
        },
        confidence: row.confidence,
        classification_reason: row.classification_reason,
        classification_status: row.classification_status,
        matched_rules,
        requires_confirmation: row.requires_confirmation,
        last_opened_at: None,
        open_count: 0,
        indexed_at: now.to_string(),
        source_id: None,
        is_stale: row.is_stale,
        state_code: row.state_code,
    }
}

pub(crate) fn parent_directory(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    normalized
        .rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default()
}

pub(crate) fn infer_file_type(extension: &str, is_dir: bool) -> &'static str {
    if is_dir {
        return "Other";
    }

    match extension.to_ascii_lowercase().as_str() {
        "pdf" | "doc" | "docx" | "txt" | "md" | "rtf" => "Document",
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "bmp" | "tiff" | "svg" => "Image",
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" => "Video",
        "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" => "Audio",
        "zip" | "rar" | "7z" | "tar" | "gz" => "ArchivePackage",
        "exe" | "msi" | "dmg" | "pkg" | "appimage" => "Installer",
        "xls" | "xlsx" | "csv" | "numbers" => "Spreadsheet",
        "ppt" | "pptx" | "key" => "Presentation",
        "js" | "jsx" | "ts" | "tsx" | "rs" | "go" | "py" | "java" | "kt" | "swift" | "c"
        | "cpp" | "h" | "hpp" | "cs" | "php" | "rb" | "html" | "css" | "scss" | "json" | "yaml"
        | "yml" | "toml" => "Code",
        _ => "Other",
    }
}

pub(crate) fn unix_seconds_to_iso(seconds: i64) -> String {
    OffsetDateTime::from_unix_timestamp(seconds)
        .ok()
        .and_then(|time| time.format(&Rfc3339).ok())
        .unwrap_or_else(current_timestamp_iso)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn collect_upsert_requests_reports_warning_when_deep_directory_limit_is_reached() {
        let root = test_dir("watcher-large-upsert");
        fs::write(root.join("one.txt"), "1").expect("write one");
        fs::write(root.join("two.txt"), "2").expect("write two");
        fs::write(root.join("three.txt"), "3").expect("write three");

        let mut files = Vec::new();
        let mut seen = Vec::new();
        let mut warnings = Vec::new();

        collect_upsert_requests_for_path(
            root.clone(),
            None,
            &mut files,
            &mut seen,
            &mut warnings,
            2,
        )
        .expect("collect upsert requests");

        assert_eq!(files.len(), 2);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].limit, 2);
        assert!(warnings[0]
            .message
            .contains("only part of the directory was indexed"));
        assert!(warnings[0]
            .path
            .as_deref()
            .is_some_and(|path| !path.is_empty()));
    }

    #[test]
    fn issue375_path_upsert_covers_user_build_and_selected_root() {
        let fixture = test_dir("issue375-upsert");
        let project_root = fixture.join("Project");
        let project_build_file = project_root.join("build/logo.ai");
        let project_source_file = project_root.join("src/main.rs");
        let selected_build_root = fixture.join("chosen/build");
        let selected_build_file = selected_build_root.join("README.md");
        fs::create_dir_all(
            project_build_file
                .parent()
                .expect("project build directory"),
        )
        .expect("create project build directory");
        fs::create_dir_all(
            project_source_file
                .parent()
                .expect("project source directory"),
        )
        .expect("create project source directory");
        fs::create_dir_all(&selected_build_root).expect("create selected build root");
        fs::write(&project_build_file, b"project asset").expect("write project build asset");
        fs::write(&project_source_file, b"project source").expect("write project source");
        fs::write(&selected_build_file, b"selected build root").expect("write selected-root file");

        let db = Database::open(fixture.join("watcher-upsert.sqlite3"))
            .expect("open isolated watcher-upsert database");
        let project_root_text = project_root.to_string_lossy().into_owned();
        let project_count = upsert_files_by_paths_for_db(&db, &[project_root_text])
            .expect("bulk-upsert ordinary project path");
        assert_eq!(
            project_count, 5,
            "root, user build and source directory contents are indexed"
        );

        fs::write(project_root.join("package.json"), b"{}").unwrap();
        let generated_file = project_root.join("build/generated.js");
        fs::write(&generated_file, b"generated output").unwrap();
        assert_eq!(
            upsert_files_by_paths_for_db(&db, &[generated_file.to_string_lossy().into_owned()])
                .unwrap(),
            0,
            "a direct file upsert cannot bypass recognized build exclusion"
        );
        fs::write(
            selected_build_root.parent().unwrap().join("package.json"),
            b"{}",
        )
        .unwrap();
        let selected_root_text = selected_build_root.to_string_lossy().into_owned();
        let roots = vec![crate::settings::ScanRootSetting {
            id: "issue375-explicit-build".into(),
            path: selected_root_text,
            label: "Explicit build".into(),
            enabled: true,
            created_at: "2026-10-18T00:00:00.000Z".into(),
        }];
        let mut settings = crate::settings::get_app_settings(&db).unwrap();
        settings.default_scan_folders = roots.clone();
        crate::settings::save_app_settings(&db, &settings).unwrap();
        db.sync_file_library_watcher_roots(&roots).unwrap();
        assert_eq!(db.list_watcher_root_configs().unwrap().len(), 1);

        let selected_file_text = selected_build_file.to_string_lossy().into_owned();
        assert_eq!(
            db.conn()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM files WHERE path = ?1",
                    [normalize_path_for_db(&selected_build_file)],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0,
            "no directory upsert has preinserted the selected file"
        );
        let selected_count = upsert_files_by_paths_for_db(&db, &[selected_file_text])
            .expect("directly upsert a fresh file under the registered selected build root");
        assert_eq!(
            selected_count, 1,
            "the fresh selected-root file is inserted"
        );

        let mut excluded_paths = vec![generated_file];
        for root in [&project_root, &selected_build_root] {
            for relative in ["node_modules/pkg/index.js", ".venv/pyvenv.cfg"] {
                let dependency = root.join(relative);
                fs::create_dir_all(dependency.parent().unwrap()).unwrap();
                fs::write(&dependency, b"dependency").unwrap();
                assert_eq!(
                    upsert_files_by_paths_for_db(&db, &[dependency.to_string_lossy().into_owned()])
                        .unwrap(),
                    0,
                    "direct dependency paths remain excluded: {}",
                    dependency.display()
                );
                excluded_paths.push(dependency);
            }
        }
        for protected_root in crate::path_filter::managed_protected_roots() {
            let protected_file = protected_root.join("zen-canvas-upsert-policy-probe.txt");
            assert_eq!(
                upsert_files_by_paths_for_db(&db, &[protected_file.to_string_lossy().into_owned()])
                    .unwrap(),
                0,
                "direct protected paths remain excluded"
            );
            excluded_paths.push(protected_file);
        }

        let outside = fixture.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("escape.txt"), b"outside selected root").unwrap();
        let linked = selected_build_root.join("linked");
        #[cfg(windows)]
        let (outside, linked) = {
            let spaced_outside = outside.with_file_name("outside with spaces");
            fs::rename(&outside, &spaced_outside).unwrap();
            (spaced_outside, linked.with_file_name("linked with spaces"))
        };
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &linked).unwrap();
        #[cfg(windows)]
        {
            use std::os::windows::{fs::MetadataExt, process::CommandExt};

            assert!(linked.is_absolute() && outside.is_absolute());
            let linked_text = linked.to_string_lossy().replace('/', "\\");
            let outside_text = outside.to_string_lossy().replace('/', "\\");
            assert!(linked_text.contains(' ') && outside_text.contains(' '));
            // cmd's built-in consumes one command line, not separately escaped
            // program arguments. Keep Windows separators and quote both paths.
            let command = format!(r#"mklink /J "{linked_text}" "{outside_text}""#);
            let output = std::process::Command::new("cmd.exe")
                .args(["/D", "/C"])
                .raw_arg(command)
                .output()
                .expect("create junction under selected root");
            assert!(output.status.success(), "junction creation: {output:?}");
            let metadata = fs::symlink_metadata(&linked).expect("read junction metadata");
            assert!(metadata_is_link_or_reparse(&metadata));
            assert_ne!(metadata.file_attributes() & 0x400, 0);
            eprintln!(
                "issue375 Windows junction created: absolute paths with spaces; reparse attributes={:#x}",
                metadata.file_attributes()
            );
        }
        let escaped_file = linked.join("escape.txt");
        assert!(matches!(
            upsert_files_by_paths_for_db(&db, &[escaped_file.to_string_lossy().into_owned()]),
            Err(DbError::Validation(_))
        ));
        excluded_paths.push(escaped_file);
        #[cfg(unix)]
        fs::remove_file(&linked).unwrap();
        #[cfg(windows)]
        {
            fs::remove_dir(&linked).unwrap();
            assert_eq!(
                fs::read(outside.join("escape.txt")).unwrap(),
                b"outside selected root"
            );
        }

        let conn = db.conn().expect("open watcher-upsert query connection");
        {
            let is_indexed = |path: &Path| {
                let normalized = normalize_path_for_db(path);
                conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM files WHERE id = ?1 AND is_stale = 0)",
                    [&normalized],
                    |row| row.get::<_, bool>(0),
                )
                .expect("query watcher-upsert result")
            };
            assert!(is_indexed(&project_build_file));
            assert!(is_indexed(&project_source_file));
            assert!(is_indexed(&selected_build_file));
            for path in &excluded_paths {
                let count: i64 = conn
                    .query_row(
                        "SELECT count(*) FROM files WHERE path = ?1",
                        [normalize_path_for_db(path)],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(
                    count,
                    0,
                    "excluded file was not inserted: {}",
                    path.display()
                );
            }
            #[cfg(windows)]
            {
                let external_rows: i64 = conn
                    .query_row(
                        "SELECT count(*) FROM files WHERE path = ?1",
                        [normalize_path_for_db(&outside.join("escape.txt"))],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(external_rows, 0, "junction target was not indexed");
                eprintln!(
                    "issue375 Windows junction escape denied: DbError::Validation; alias rows=0; external rows=0; target preserved"
                );
            }
        }
        drop(conn);
        drop(db);
        fs::remove_dir_all(fixture).expect("remove isolated watcher-upsert fixture");
    }

    fn test_dir(prefix: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = crate::path_filter::managed_test_temp_dir()
            .join(format!("zen-canvas-{prefix}-{nonce}"));
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }
}
