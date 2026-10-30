#[cfg(test)]
#[path = "tests/extension_single_key_tests.rs"]
mod extension_single_key_tests;

#[cfg(test)]
#[path = "tests/extension_recency_probe.rs"]
pub(crate) mod extension_recency_probe;

#[cfg(test)]
#[path = "tests/fts_rank_frontier.rs"]
pub(crate) mod rank_frontier;

use super::models::GlobalSearchResult;
use crate::db::{Database, DbError};
use rusqlite::{params, Connection, Params};
use std::collections::HashSet;
#[cfg(feature = "compact-name-search-candidate")]
use std::sync::Arc;

const MAX_SEARCH_LIMIT: u32 = 200;
const MAX_SEARCH_OFFSET: u32 = 1_000_000;
// The final page is taken from this bounded, de-duplicated candidate window.
// A layer may fetch up to target + already-seen ids to compensate for overlap,
// but never turns a keystroke into an unbounded result materialization.
const MAX_TIER_CANDIDATES: u32 = 4_096;

#[cfg(test)]
std::thread_local! {
    static KEY_ONLY_CTE_SEARCH_OVERRIDE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static KEY_ONLY_FTS_SEARCH_OVERRIDE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static LEGACY_NAME_PREFIX_SEARCH_OVERRIDE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static LEGACY_FTS_SEARCH_OVERRIDE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Enables the remaining test-only key projection candidates while exercising
/// the normal pooled and repository entry points. The name-prefix production
/// tier is always key-only; this override only selects alternatives for tiers
/// that remain on their current production query paths.
#[cfg(test)]
pub(crate) fn with_key_only_cte_search_candidate<T>(operation: impl FnOnce() -> T) -> T {
    struct ResetCandidate(bool);

    impl Drop for ResetCandidate {
        fn drop(&mut self) {
            KEY_ONLY_CTE_SEARCH_OVERRIDE.with(|override_slot| override_slot.set(self.0));
        }
    }

    KEY_ONLY_CTE_SEARCH_OVERRIDE.with(|override_slot| {
        let previous = override_slot.replace(true);
        let _reset = ResetCandidate(previous);
        operation()
    })
}

#[cfg(test)]
pub(crate) fn with_key_only_fts_search_candidate<T>(operation: impl FnOnce() -> T) -> T {
    struct ResetCandidate(bool);

    impl Drop for ResetCandidate {
        fn drop(&mut self) {
            KEY_ONLY_FTS_SEARCH_OVERRIDE.with(|override_slot| override_slot.set(self.0));
        }
    }

    KEY_ONLY_FTS_SEARCH_OVERRIDE.with(|override_slot| {
        let previous = override_slot.replace(true);
        let _reset = ResetCandidate(previous);
        operation()
    })
}

/// Forces the pre-CTE name-prefix SQL while exercising the normal pooled and
/// repository entry points. This is only for paired regression/performance
/// comparisons; release builds always use the key-only production query.
#[cfg(test)]
pub(crate) fn with_legacy_name_prefix_search_candidate<T>(operation: impl FnOnce() -> T) -> T {
    struct ResetCandidate(bool);

    impl Drop for ResetCandidate {
        fn drop(&mut self) {
            LEGACY_NAME_PREFIX_SEARCH_OVERRIDE.with(|override_slot| override_slot.set(self.0));
        }
    }

    LEGACY_NAME_PREFIX_SEARCH_OVERRIDE.with(|override_slot| {
        let previous = override_slot.replace(true);
        let _reset = ResetCandidate(previous);
        operation()
    })
}

#[cfg(test)]
pub(crate) fn with_legacy_fts_search_candidate<T>(operation: impl FnOnce() -> T) -> T {
    struct ResetCandidate(bool);

    impl Drop for ResetCandidate {
        fn drop(&mut self) {
            LEGACY_FTS_SEARCH_OVERRIDE.with(|override_slot| override_slot.set(self.0));
        }
    }

    LEGACY_FTS_SEARCH_OVERRIDE.with(|override_slot| {
        let previous = override_slot.replace(true);
        let _reset = ResetCandidate(previous);
        operation()
    })
}

#[cfg(test)]
fn key_only_cte_search_candidate_enabled() -> bool {
    KEY_ONLY_CTE_SEARCH_OVERRIDE.with(std::cell::Cell::get)
}

#[cfg(test)]
fn key_only_fts_search_candidate_enabled() -> bool {
    KEY_ONLY_FTS_SEARCH_OVERRIDE.with(std::cell::Cell::get)
}

#[cfg(test)]
fn legacy_name_prefix_search_candidate_enabled() -> bool {
    LEGACY_NAME_PREFIX_SEARCH_OVERRIDE.with(std::cell::Cell::get)
}

#[cfg(test)]
fn legacy_fts_search_candidate_enabled() -> bool {
    LEGACY_FTS_SEARCH_OVERRIDE.with(std::cell::Cell::get)
}

/// Global search is intentionally a separate entry point from the library
/// query. It never accepts `LibraryScope` and never joins the AI `files`
/// table, so a Spotlight query cannot accidentally widen an AI operation.
///
/// The stream is filled in order by exact name, name prefix, extension exact,
/// extension prefix, and finally indexed FTS/punctuation-prefix candidates.
/// Each layer is queried only while the previous layers have not filled the
/// requested bounded window. Results are de-duplicated by stable entry id.
/// `offset` is applied after that deterministic de-duplication, and `cursor`
/// in the public command is the same numeric offset. The candidate window is
/// capped at `MAX_TIER_CANDIDATES`; an offset beyond it returns an empty page.
pub fn search_global_entries(
    db: &Database,
    query: &str,
    limit: u32,
    offset: u32,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    #[allow(unused_mut)]
    let mut conn = db.conn()?;
    #[cfg(feature = "compact-name-search-candidate")]
    if db.compact_name_search().is_enabled() {
        let transaction = conn.transaction()?;
        let results = search_global_entries_on_connection_with_compact(
            &transaction,
            query,
            limit,
            offset,
            db.compact_name_search(),
        )?;
        transaction.commit()?;
        return Ok(results);
    }
    search_global_entries_on_connection(&conn, query, limit, offset)
}

/// Runs the same search against an existing SQLite connection. The repository
/// snapshot uses this entry point while holding one read transaction so the
/// result rows and source/index facts cannot describe different database
/// states.
pub(crate) fn search_global_entries_on_connection(
    conn: &Connection,
    query: &str,
    limit: u32,
    offset: u32,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    search_global_entries_impl(conn, query, limit, offset, None)
}

#[cfg(feature = "compact-name-search-candidate")]
pub(crate) fn search_global_entries_on_connection_with_compact(
    conn: &Connection,
    query: &str,
    limit: u32,
    offset: u32,
    compact: &Arc<super::compact_name_search::CompactNameSearchManager>,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    search_global_entries_impl(conn, query, limit, offset, Some(compact))
}

trait CompactSearchCandidate {
    fn search_page(
        &self,
        conn: &Connection,
        query: &str,
        target: usize,
        page_skip: usize,
        page_len: usize,
        already_seen: &HashSet<String>,
    ) -> Result<Option<Vec<GlobalSearchResult>>, DbError>;
}

#[cfg(feature = "compact-name-search-candidate")]
impl CompactSearchCandidate for Arc<super::compact_name_search::CompactNameSearchManager> {
    fn search_page(
        &self,
        conn: &Connection,
        query: &str,
        target: usize,
        page_skip: usize,
        page_len: usize,
        already_seen: &HashSet<String>,
    ) -> Result<Option<Vec<GlobalSearchResult>>, DbError> {
        super::compact_name_search::CompactNameSearchManager::search_page(
            self,
            conn,
            query,
            target,
            page_skip,
            page_len,
            already_seen,
        )
    }
}

fn search_global_entries_impl(
    conn: &Connection,
    query: &str,
    limit: u32,
    offset: u32,
    compact: Option<&dyn CompactSearchCandidate>,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    #[cfg(test)]
    extension_recency_probe::page(limit, offset);
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let normalized_query = query.to_lowercase();
    let query = normalized_query.as_str();

    let limit = limit.clamp(1, MAX_SEARCH_LIMIT);
    let offset = offset.min(MAX_SEARCH_OFFSET);
    if offset >= MAX_TIER_CANDIDATES {
        return Ok(Vec::new());
    }

    let target = offset.saturating_add(limit).min(MAX_TIER_CANDIDATES);
    let mut results = Vec::with_capacity(target as usize);
    let mut seen = HashSet::new();

    let tier_limit = layer_limit(target, seen.len());
    append_unique(
        &mut results,
        &mut seen,
        search_exact_name(conn, query, tier_limit)?,
        target,
    );
    if results.len() < target as usize {
        let tier_limit = layer_limit(target, seen.len());
        append_unique(
            &mut results,
            &mut seen,
            search_name_prefix(conn, query, tier_limit)?,
            target,
        );
    }
    if results.len() < target as usize {
        let tier_limit = layer_limit(target, seen.len());
        append_unique(
            &mut results,
            &mut seen,
            search_exact_extension(conn, query, tier_limit)?,
            target,
        );
    }
    if results.len() < target as usize {
        let tier_limit = layer_limit(target, seen.len());
        append_unique(
            &mut results,
            &mut seen,
            search_extension_prefix(conn, query, tier_limit)?,
            target,
        );
    }

    if results.len() < target as usize {
        if let Some(compact) = compact {
            let leading = results.len() as u32;
            let fuzzy_target = target.saturating_sub(leading) as usize;
            let fuzzy_skip = offset.saturating_sub(leading) as usize;
            let fuzzy_len = target.saturating_sub(offset.max(leading)) as usize;
            if let Some(fuzzy_page) =
                compact.search_page(conn, query, fuzzy_target, fuzzy_skip, fuzzy_len, &seen)?
            {
                let mut page = results
                    .iter()
                    .skip(offset as usize)
                    .take(limit as usize)
                    .cloned()
                    .collect::<Vec<_>>();
                page.extend(fuzzy_page);
                return Ok(page);
            }
        }
        if query
            .chars()
            .all(|character| character.is_alphanumeric() || character.is_whitespace())
        {
            let tier_limit = layer_limit(target, seen.len());
            append_unique(
                &mut results,
                &mut seen,
                search_fts(conn, query, tier_limit)?,
                target,
            );
        } else if !query.is_empty() {
            let tier_limit = layer_limit(target, seen.len());
            append_unique(
                &mut results,
                &mut seen,
                search_punctuation_prefix(conn, query, tier_limit)?,
                target,
            );
        }
    }

    Ok(results
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect())
}

fn layer_limit(target: u32, seen: usize) -> u32 {
    target
        .saturating_add(seen.min(MAX_TIER_CANDIDATES as usize) as u32)
        .clamp(1, MAX_TIER_CANDIDATES)
}

fn append_unique(
    results: &mut Vec<GlobalSearchResult>,
    seen: &mut HashSet<String>,
    candidates: Vec<GlobalSearchResult>,
    target: u32,
) {
    for candidate in candidates {
        if seen.insert(candidate.id.clone()) {
            results.push(candidate);
            if results.len() >= target as usize {
                break;
            }
        }
    }
}

fn candidate_sql(from: &str, predicate: &str, order: &str, rank: &str, limit_slot: &str) -> String {
    format!(
        r#"
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
               {rank} AS rank
        FROM {from}
        WHERE {predicate}
        ORDER BY {order}
        LIMIT {limit_slot}
        "#
    )
}

fn key_only_candidate_sql(
    from: &str,
    predicate: &str,
    order: &str,
    rank: &str,
    outer_order: &str,
    limit_slot: &str,
) -> String {
    format!(
        r#"
        WITH candidates AS MATERIALIZED (
            SELECT ge.rowid AS entry_rowid,
                   ge.id AS entry_id,
                   ge.modified_at_fs AS modified_at_fs,
                   {rank} AS candidate_rank
            FROM {from}
            WHERE {predicate}
            ORDER BY {order}
            LIMIT {limit_slot}
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
               candidates.candidate_rank AS rank
        FROM candidates
        JOIN global_entries ge ON ge.rowid = candidates.entry_rowid
        ORDER BY {outer_order}
        "#
    )
}

fn key_only_name_prefix_sql() -> String {
    key_only_candidate_sql(
        "global_entries ge INDEXED BY idx_global_entries_active_name_order CROSS JOIN global_volumes gv",
        "gv.id = ge.volume_id AND gv.enabled = 1 AND gv.coverage_pending = 0 AND ge.is_stale = 0 AND ge.name_normalized GLOB ?2 AND ge.name_normalized <> lower(?1)",
        "ge.modified_at_fs DESC, ge.id ASC",
        "0.0",
        "candidates.modified_at_fs DESC, candidates.entry_id ASC",
        "?3",
    )
}

#[cfg(test)]
fn key_only_punctuation_prefix_sql() -> String {
    key_only_candidate_sql(
        "global_entries ge INDEXED BY idx_global_entries_active_name_order CROSS JOIN global_volumes gv",
        "gv.id = ge.volume_id AND gv.enabled = 1 AND gv.coverage_pending = 0 AND ge.is_stale = 0 AND ge.name_normalized GLOB ?1",
        "ge.modified_at_fs DESC, ge.id ASC",
        "0.0",
        "candidates.modified_at_fs DESC, candidates.entry_id ASC",
        "?2",
    )
}

#[cfg(test)]
fn key_only_extension_prefix_sql() -> String {
    key_only_candidate_sql(
        "global_entries ge INDEXED BY idx_global_entries_active_extension_order CROSS JOIN global_volumes gv",
        "gv.id = ge.volume_id AND gv.enabled = 1 AND gv.coverage_pending = 0 AND ge.is_stale = 0 AND ge.extension GLOB ?2 AND ge.extension <> lower(?1)",
        "ge.modified_at_fs DESC, ge.id ASC",
        "1.0",
        "candidates.modified_at_fs DESC, candidates.entry_id ASC",
        "?3",
    )
}

fn key_only_fts_sql() -> String {
    key_only_candidate_sql(
        "global_entries_fts CROSS JOIN global_entries ge CROSS JOIN global_volumes gv",
        "global_entries_fts MATCH ?1 AND ge.rowid = global_entries_fts.rowid AND gv.id = ge.volume_id AND gv.enabled = 1 AND gv.coverage_pending = 0 AND ge.is_stale = 0",
        "candidate_rank ASC, ge.modified_at_fs DESC, ge.id ASC",
        "bm25(global_entries_fts, 8.0, 2.0, 1.0)",
        "candidates.candidate_rank ASC, candidates.modified_at_fs DESC, candidates.entry_id ASC",
        "?2",
    )
}

#[cfg(test)]
pub(crate) fn diagnostic_key_only_cte_sql(tier: &str) -> String {
    match tier {
        "name_prefix" => key_only_name_prefix_sql(),
        "punctuation_prefix" => key_only_punctuation_prefix_sql(),
        "extension_prefix" => key_only_extension_prefix_sql(),
        "fts" => key_only_fts_sql(),
        _ => panic!("unknown key-only CTE search tier: {tier}"),
    }
}

#[cfg(test)]
pub(crate) fn diagnostic_search_tier_sql(tier: &str) -> String {
    match tier {
        "exact_name" => candidate_sql(
            "global_entries ge INDEXED BY idx_global_entries_active_name_order JOIN global_volumes gv ON gv.id = ge.volume_id",
            "gv.enabled = 1 AND gv.coverage_pending = 0 AND ge.is_stale = 0 AND ge.name_normalized = lower(?1)",
            "ge.modified_at_fs DESC, ge.id ASC",
            "0.0",
            "?2",
        ),
        "name_prefix" => key_only_name_prefix_sql(),
        "exact_extension" => candidate_sql(
            "global_entries ge INDEXED BY idx_global_entries_active_extension_order JOIN global_volumes gv ON gv.id = ge.volume_id",
            "gv.enabled = 1 AND gv.coverage_pending = 0 AND ge.is_stale = 0 AND ge.extension = lower(?1)",
            "ge.modified_at_fs DESC, ge.id ASC",
            "0.0",
            "?2",
        ),
        // This is the exact production fallback SQL. A single-key prefix
        // instead uses the discovery + exact-key SQL reported separately.
        "extension_prefix" => extension_prefix_legacy_sql(),
        "fts" => diagnostic_search_fts_sql(),
        "punctuation_prefix" => candidate_sql(
            "global_entries ge INDEXED BY idx_global_entries_active_name_order JOIN global_volumes gv ON gv.id = ge.volume_id",
            "gv.enabled = 1 AND gv.coverage_pending = 0 AND ge.is_stale = 0 AND ge.name_normalized GLOB ?1",
            "ge.modified_at_fs DESC, ge.id ASC",
            "0.0",
            "?2",
        ),
        _ => panic!("unknown diagnostic production search tier: {tier}"),
    }
}

#[cfg(test)]
pub(crate) fn diagnostic_legacy_name_prefix_sql() -> String {
    candidate_sql(
        "global_entries ge INDEXED BY idx_global_entries_active_name_order JOIN global_volumes gv ON gv.id = ge.volume_id",
        "gv.enabled = 1 AND gv.coverage_pending = 0 AND ge.is_stale = 0 AND ge.name_normalized GLOB ?2 AND ge.name_normalized <> lower(?1)",
        "ge.modified_at_fs DESC, ge.id ASC",
        "0.0",
        "?3",
    )
}

fn collect_candidates<P: Params>(
    statement: &mut rusqlite::Statement<'_>,
    parameters: P,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    let rows = statement.query_map(parameters, map_global_search_result)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
}

fn search_exact_name(
    conn: &Connection,
    query: &str,
    limit: u32,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    let sql = candidate_sql(
        "global_entries ge INDEXED BY idx_global_entries_active_name_order JOIN global_volumes gv ON gv.id = ge.volume_id",
        "gv.enabled = 1 AND gv.coverage_pending = 0 AND ge.is_stale = 0 AND ge.name_normalized = lower(?1)",
        "ge.modified_at_fs DESC, ge.id ASC",
        "0.0",
        "?2",
    );
    let mut statement = conn.prepare(&sql)?;
    collect_candidates(&mut statement, params![query, limit])
}

fn search_name_prefix(
    conn: &Connection,
    query: &str,
    limit: u32,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    #[cfg(test)]
    if legacy_name_prefix_search_candidate_enabled() {
        let sql = diagnostic_legacy_name_prefix_sql();
        let mut statement = conn.prepare(&sql)?;
        return collect_candidates(
            &mut statement,
            params![query, format!("{}*", escape_glob(query)), limit],
        );
    }

    let sql = key_only_name_prefix_sql();
    let mut statement = conn.prepare(&sql)?;
    collect_candidates(
        &mut statement,
        params![query, format!("{}*", escape_glob(query)), limit],
    )
}

fn search_exact_extension(
    conn: &Connection,
    query: &str,
    limit: u32,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    let sql = candidate_sql(
        "global_entries ge INDEXED BY idx_global_entries_active_extension_order JOIN global_volumes gv ON gv.id = ge.volume_id",
        "gv.enabled = 1 AND gv.coverage_pending = 0 AND ge.is_stale = 0 AND ge.extension = lower(?1)",
        "ge.modified_at_fs DESC, ge.id ASC",
        "0.0",
        "?2",
    );
    let mut statement = conn.prepare(&sql)?;
    collect_candidates(&mut statement, params![query, limit])
}

// Use the existing active extension covering index to discover whether this
// prefix maps to zero, exactly one, or multiple distinct extension values.
// Two values are enough to establish that the legacy range-and-sort path is
// necessary; no extension-name cache or new schema is introduced.
const EXTENSION_PREFIX_SINGLE_KEY_PROBE_SQL: &str = "
    SELECT DISTINCT ge.extension
    FROM global_entries ge INDEXED BY idx_global_entries_active_extension_order
    WHERE ge.is_stale = 0
      AND ge.extension GLOB ?2
      AND ge.extension <> lower(?1)
    LIMIT 2
";

fn extension_prefix_legacy_sql() -> String {
    candidate_sql(
        "global_entries ge INDEXED BY idx_global_entries_active_extension_order JOIN global_volumes gv ON gv.id = ge.volume_id",
        "gv.enabled = 1 AND gv.coverage_pending = 0 AND ge.is_stale = 0 AND ge.extension GLOB ?2 AND ge.extension <> lower(?1)",
        "ge.modified_at_fs DESC, ge.id ASC",
        "1.0",
        "?3",
    )
}

fn extension_prefix_single_key_sql() -> String {
    candidate_sql(
        "global_entries ge INDEXED BY idx_global_entries_active_extension_order CROSS JOIN global_volumes gv",
        "gv.id = ge.volume_id AND gv.enabled = 1 AND gv.coverage_pending = 0 AND ge.is_stale = 0 AND ge.extension = ?4 COLLATE BINARY AND ge.extension GLOB ?2 AND ge.extension <> lower(?1)",
        "ge.modified_at_fs DESC, ge.id ASC",
        "1.0",
        "?3",
    )
}

#[cfg(test)]
pub(crate) fn diagnostic_extension_prefix_strategy_sqls() -> (&'static str, String) {
    (
        EXTENSION_PREFIX_SINGLE_KEY_PROBE_SQL,
        extension_prefix_single_key_sql(),
    )
}

fn search_extension_prefix_legacy(
    conn: &Connection,
    query: &str,
    glob: &str,
    limit: u32,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    let mut statement = conn.prepare(&extension_prefix_legacy_sql())?;
    collect_candidates(&mut statement, params![query, glob, limit])
}

fn search_extension_prefix(
    conn: &Connection,
    query: &str,
    limit: u32,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    #[cfg(test)]
    if extension_recency_probe::mode() != 0 {
        return extension_recency_probe::dispatch(conn, query, limit);
    }
    #[cfg(test)]
    if key_only_cte_search_candidate_enabled() {
        let sql = key_only_extension_prefix_sql();
        let mut statement = conn.prepare(&sql)?;
        return collect_candidates(
            &mut statement,
            params![query, format!("{}*", escape_glob(query)), limit],
        );
    }

    // The discovery and accelerated query must observe the SAME read snapshot.
    // The Repository Snapshot entrypoint already owns its transaction; pooled
    // standalone Search takes a read transaction just for this tier.
    let transaction = if conn.is_autocommit() {
        Some(conn.unchecked_transaction()?)
    } else {
        None
    };
    let read = transaction.as_deref().unwrap_or(conn);
    let glob = format!("{}*", escape_glob(query));
    let extensions = {
        let mut statement = read.prepare(EXTENSION_PREFIX_SINGLE_KEY_PROBE_SQL)?;
        let rows = statement.query_map(params![query, &glob], |row| {
            row.get::<_, rusqlite::types::Value>(0)
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    };

    match extensions {
        Ok(values) if values.is_empty() => Ok(Vec::new()),
        Ok(values) if values.len() == 1 => {
            if let rusqlite::types::Value::Text(extension) = &values[0] {
                // Exactly one extension means the composite index's
                // (modified_at_fs DESC, id ASC) suffix is already the final
                // tier order. Keep every original predicate before LIMIT.
                let mut statement = read.prepare(&extension_prefix_single_key_sql())?;
                return collect_candidates(&mut statement, params![query, &glob, limit, extension]);
            }
            search_extension_prefix_legacy(read, query, &glob, limit)
        }
        // Multiple extension keys need a global time sort. Malformed SQLite
        // extension values also retain the original production query/error.
        _ => search_extension_prefix_legacy(read, query, &glob, limit),
    }
}

fn search_fts(
    conn: &Connection,
    query: &str,
    limit: u32,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    #[cfg(test)]
    if rank_frontier::enabled() {
        return rank_frontier::run(conn, query, limit);
    }
    #[cfg(test)]
    if legacy_fts_search_candidate_enabled() {
        let sql = diagnostic_legacy_fts_sql();
        let mut statement = conn.prepare(&sql)?;
        let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
        return collect_candidates(&mut statement, params![fts_query, limit]);
    }

    #[cfg(test)]
    if key_only_cte_search_candidate_enabled() || key_only_fts_search_candidate_enabled() {
        let sql = key_only_fts_sql();
        let mut statement = conn.prepare(&sql)?;
        let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
        return collect_candidates(&mut statement, params![fts_query, limit]);
    }

    let sql = key_only_fts_sql();
    let mut statement = conn.prepare(&sql)?;
    let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
    collect_candidates(&mut statement, params![fts_query, limit])
}

/// Test-only access to the exact production FTS query for the staged latency
/// diagnostic. This wrapper does not change the runtime search path.
#[cfg(test)]
pub(crate) fn diagnostic_search_fts(
    conn: &Connection,
    query: &str,
    limit: u32,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    search_fts(conn, query, limit)
}

#[cfg(test)]
pub(crate) fn diagnostic_search_fts_sql() -> String {
    key_only_fts_sql()
}

#[cfg(test)]
pub(crate) fn diagnostic_legacy_fts_sql() -> String {
    candidate_sql(
        "global_entries_fts CROSS JOIN global_entries ge CROSS JOIN global_volumes gv",
        "global_entries_fts MATCH ?1 AND ge.rowid = global_entries_fts.rowid AND gv.id = ge.volume_id AND gv.enabled = 1 AND gv.coverage_pending = 0 AND ge.is_stale = 0",
        "rank ASC, ge.modified_at_fs DESC, ge.id ASC",
        "bm25(global_entries_fts, 8.0, 2.0, 1.0)",
        "?2",
    )
}

/// Run one of the real production search tiers in isolation so no-result
/// latency can be attributed to the tier that consumes it.
#[cfg(test)]
pub(crate) fn diagnostic_search_tier(
    conn: &Connection,
    tier: &str,
    query: &str,
    limit: u32,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    match tier {
        "exact_name" => search_exact_name(conn, query, limit),
        "name_prefix" => search_name_prefix(conn, query, limit),
        "exact_extension" => search_exact_extension(conn, query, limit),
        "extension_prefix" => search_extension_prefix(conn, query, limit),
        "fts" => search_fts(conn, query, limit),
        "punctuation_prefix" => search_punctuation_prefix(conn, query, limit),
        _ => panic!("unknown diagnostic search tier: {tier}"),
    }
}

fn search_punctuation_prefix(
    conn: &Connection,
    prefix: &str,
    limit: u32,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    #[cfg(test)]
    if key_only_cte_search_candidate_enabled() {
        let sql = key_only_punctuation_prefix_sql();
        let mut statement = conn.prepare(&sql)?;
        return collect_candidates(
            &mut statement,
            params![format!("{}*", escape_glob(prefix)), limit],
        );
    }

    let sql = candidate_sql(
        "global_entries ge INDEXED BY idx_global_entries_active_name_order JOIN global_volumes gv ON gv.id = ge.volume_id",
        "gv.enabled = 1 AND gv.coverage_pending = 0 AND ge.is_stale = 0 AND ge.name_normalized GLOB ?1",
        "ge.modified_at_fs DESC, ge.id ASC",
        "0.0",
        "?2",
    );
    let mut statement = conn.prepare(&sql)?;
    collect_candidates(
        &mut statement,
        params![format!("{}*", escape_glob(prefix)), limit],
    )
}

fn escape_glob(value: &str) -> String {
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

pub(super) fn map_global_search_result(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<GlobalSearchResult> {
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
