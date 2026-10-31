#[cfg(test)]
#[path = "tests/extension_single_key_tests.rs"]
mod extension_single_key_tests;

#[cfg(test)]
#[path = "tests/extension_recency_probe.rs"]
pub(crate) mod extension_recency_probe;

#[cfg(test)]
#[path = "tests/fts_rank_frontier.rs"]
pub(crate) mod rank_frontier;

use super::filename_search::{
    ChangeSet, FilenameSearchIndex, FilenameTrigramCandidate, RankedFileCandidate,
};
use super::models::GlobalSearchResult;
use crate::db::{Database, DbError};
use rusqlite::{params, Connection, Params};
use std::collections::HashSet;

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
    static LEGACY_FILENAME_JOIN_OVERRIDE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn with_legacy_filename_join_candidates_for_test<T>(operation: impl FnOnce() -> T) -> T {
    struct ResetCandidate(bool);

    impl Drop for ResetCandidate {
        fn drop(&mut self) {
            LEGACY_FILENAME_JOIN_OVERRIDE.with(|override_slot| override_slot.set(self.0));
        }
    }

    LEGACY_FILENAME_JOIN_OVERRIDE.with(|override_slot| {
        let previous = override_slot.replace(true);
        let _reset = ResetCandidate(previous);
        operation()
    })
}

#[cfg(test)]
fn legacy_filename_join_candidates_for_test() -> bool {
    LEGACY_FILENAME_JOIN_OVERRIDE.with(std::cell::Cell::get)
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
    if db.filename_search.is_enabled() && filename_relevance_query_supported(query.trim()) {
        if let Ok(Some(results)) =
            db.filename_search
                .try_with_snapshot(|transaction, index, changes| {
                    search_global_entries_with_filename_index_on_connection(
                        transaction,
                        query,
                        limit,
                        offset,
                        index,
                        changes,
                    )
                })
        {
            return Ok(results);
        }
    }
    let conn = db.conn()?;
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
    #[cfg(test)]
    extension_recency_probe::page(limit, offset);
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let limit = limit.clamp(1, MAX_SEARCH_LIMIT);
    let offset = offset.min(MAX_SEARCH_OFFSET);
    if offset >= MAX_TIER_CANDIDATES {
        return Ok(Vec::new());
    }

    let target = offset.saturating_add(limit).min(MAX_TIER_CANDIDATES);
    let (mut results, mut seen) = search_fast_tiers(conn, query, target)?;

    if results.len() < target as usize && query.chars().count() >= 3 {
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

fn search_fast_tiers(
    conn: &Connection,
    query: &str,
    target: u32,
) -> Result<(Vec<GlobalSearchResult>, HashSet<String>), DbError> {
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
    Ok((results, seen))
}

pub(crate) fn filename_relevance_query_supported(query: &str) -> bool {
    query.len() >= 3
        && !query.starts_with(' ')
        && !query.ends_with(' ')
        && !query.contains("  ")
        && query
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b' ')
}

/// Filename-first route over one pinned SQLite snapshot and its bounded
/// mutation overlay. `None` means this query shape must use SQLite fallback.
pub(crate) fn search_global_entries_with_filename_index_on_connection(
    conn: &Connection,
    query: &str,
    limit: u32,
    offset: u32,
    index: &FilenameSearchIndex,
    changes: &ChangeSet,
) -> Result<Option<Vec<GlobalSearchResult>>, DbError> {
    let query = query.trim();
    if query.is_empty() || !filename_relevance_query_supported(query) {
        return Ok(None);
    }
    let limit = limit.clamp(1, MAX_SEARCH_LIMIT);
    let offset = offset.min(MAX_SEARCH_OFFSET);
    if offset >= MAX_TIER_CANDIDATES {
        return Ok(Some(Vec::new()));
    }
    let target = offset.saturating_add(limit).min(MAX_TIER_CANDIDATES);
    let normalized_query = query.to_ascii_lowercase();
    #[cfg(test)]
    let fast_tiers_started = std::time::Instant::now();
    let (fast_results, seen) = search_fast_tiers(conn, &normalized_query, target)?;
    #[cfg(test)]
    super::filename_search::record_candidate_search_stage("fast_tiers", fast_tiers_started);
    let fast_len = fast_results.len();
    if fast_len >= target as usize {
        return Ok(Some(
            fast_results
                .into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .collect(),
        ));
    }

    #[cfg(test)]
    let overlay_materialization_started = std::time::Instant::now();
    let changed_rowids = changes
        .changed_rowids
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    #[cfg(test)]
    super::filename_search::record_candidate_search_stage(
        "changed_rowid_set",
        overlay_materialization_started,
    );
    #[cfg(test)]
    let overlay_read_started = std::time::Instant::now();
    let overlay_rows = index.overlay_rows(conn, &changes.changed_rowids)?;
    #[cfg(test)]
    super::filename_search::record_candidate_search_stage("overlay_sql", overlay_read_started);
    #[cfg(test)]
    let trigram_collection_started = std::time::Instant::now();
    let trigram_candidates = search_filename_trigram_candidates(conn, &normalized_query)?;
    #[cfg(test)]
    let candidate_stage_name = if legacy_filename_join_candidates_for_test() {
        "fts_join_and_source_filter_enumeration"
    } else {
        "fts_rowid_enumeration"
    };
    #[cfg(test)]
    super::filename_search::record_candidate_search_stage(
        candidate_stage_name,
        trigram_collection_started,
    );
    let remaining = (target as usize).saturating_sub(fast_len);
    #[cfg(test)]
    let rank_started = std::time::Instant::now();
    let mut fuzzy = index.rank_filename_candidates(
        &normalized_query,
        &trigram_candidates,
        &seen,
        &changed_rowids,
        &overlay_rows,
        remaining,
    );
    #[cfg(test)]
    super::filename_search::record_candidate_search_stage(
        "compact_eligibility_match_and_topk",
        rank_started,
    );
    #[cfg(test)]
    let path_only_started = std::time::Instant::now();
    if fuzzy.len() < remaining {
        fuzzy.extend(search_path_only_candidates(
            conn,
            &normalized_query,
            remaining - fuzzy.len(),
        )?);
    }
    #[cfg(test)]
    super::filename_search::record_candidate_search_stage("path_only_query", path_only_started);

    let page_start = offset as usize;
    let page_end = page_start
        .saturating_add(limit as usize)
        .min(target as usize);
    let fast_end = page_end.min(fast_len);
    let mut page = if page_start < fast_end {
        fast_results
            .into_iter()
            .skip(page_start)
            .take(fast_end - page_start)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let fuzzy_start = page_start.saturating_sub(fast_len);
    let fuzzy_end = page_end.saturating_sub(fast_len).min(fuzzy.len());
    #[cfg(test)]
    let hydration_started = std::time::Instant::now();
    if fuzzy_start < fuzzy_end {
        page.extend(hydrate_ranked_filename_page(
            conn,
            &fuzzy[fuzzy_start..fuzzy_end],
        )?);
    }
    #[cfg(test)]
    super::filename_search::record_candidate_search_stage("page_hydration_cte", hydration_started);
    Ok(Some(page))
}

const FILENAME_TRIGRAM_ROWID_SQL: &str = r#"
    SELECT rowid
    FROM global_entries_fts
    WHERE global_entries_fts MATCH ?1
"#;

#[cfg(test)]
const LEGACY_FILENAME_TRIGRAM_JOIN_SQL: &str = r#"
    SELECT ge.rowid, (instr(lower(ge.extension), ?2) > 0)
    FROM global_entries_fts
    CROSS JOIN global_entries ge
    CROSS JOIN global_volumes gv
    WHERE global_entries_fts MATCH ?1
      AND ge.rowid = global_entries_fts.rowid
      AND gv.id = ge.volume_id
      AND gv.enabled = 1 AND gv.coverage_pending = 0 AND ge.is_stale = 0
      AND (instr(ge.name_normalized, ?2) > 0 OR instr(lower(ge.extension), ?2) > 0)
"#;

fn search_filename_trigram_candidates(
    conn: &Connection,
    query: &str,
) -> Result<Vec<FilenameTrigramCandidate>, DbError> {
    let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
    #[cfg(test)]
    if legacy_filename_join_candidates_for_test() {
        let mut statement = conn.prepare(LEGACY_FILENAME_TRIGRAM_JOIN_SQL)?;
        let rows = statement.query_map(params![fts_query, query], |row| {
            Ok(FilenameTrigramCandidate {
                rowid: row.get(0)?,
                extension_match: Some(row.get(1)?),
            })
        })?;
        return rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(DbError::from);
    }

    let mut statement = conn.prepare(FILENAME_TRIGRAM_ROWID_SQL)?;
    let rows = statement.query_map([fts_query], |row| {
        Ok(FilenameTrigramCandidate {
            rowid: row.get(0)?,
            extension_match: None,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(DbError::from)
}

#[cfg(test)]
pub(crate) fn diagnostic_filename_trigram_candidates_sql() -> &'static str {
    FILENAME_TRIGRAM_ROWID_SQL
}

#[cfg(test)]
pub(crate) fn diagnostic_legacy_filename_trigram_join_sql() -> &'static str {
    LEGACY_FILENAME_TRIGRAM_JOIN_SQL
}

#[cfg(test)]
pub(crate) fn diagnostic_filename_rank_stage_profile_on_connection(
    conn: &Connection,
    query: &str,
    limit: u32,
    offset: u32,
    index: &FilenameSearchIndex,
    changes: &ChangeSet,
) -> Result<Option<super::filename_search::FilenameRankStageProfile>, DbError> {
    let query = query.trim();
    if query.is_empty() || !filename_relevance_query_supported(query) {
        return Ok(None);
    }
    let limit = limit.clamp(1, MAX_SEARCH_LIMIT);
    let offset = offset.min(MAX_SEARCH_OFFSET);
    if offset >= MAX_TIER_CANDIDATES {
        return Ok(None);
    }
    let target = offset.saturating_add(limit).min(MAX_TIER_CANDIDATES);
    let normalized_query = query.to_ascii_lowercase();
    let (fast_results, seen) = search_fast_tiers(conn, &normalized_query, target)?;
    let capacity = (target as usize).saturating_sub(fast_results.len());
    let changed_rowids = changes
        .changed_rowids
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    let overlay_rows = index.overlay_rows(conn, &changes.changed_rowids)?;
    let trigram_candidates = search_filename_trigram_candidates(conn, &normalized_query)?;
    Ok(Some(index.diagnostic_rank_stage_profile(
        &normalized_query,
        &trigram_candidates,
        &seen,
        &changed_rowids,
        &overlay_rows,
        capacity,
    )))
}

fn search_path_only_candidates(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> Result<Vec<RankedFileCandidate<'static>>, DbError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let sql = r#"
        SELECT ge.rowid, ge.id, ge.modified_at_fs
        FROM global_entries_fts
        CROSS JOIN global_entries ge
        CROSS JOIN global_volumes gv
        WHERE global_entries_fts MATCH ?1
          AND ge.rowid = global_entries_fts.rowid
          AND gv.id = ge.volume_id
          AND gv.enabled = 1 AND gv.coverage_pending = 0 AND ge.is_stale = 0
          AND instr(ge.name_normalized, ?2) = 0
          AND instr(lower(ge.extension), ?2) = 0
        ORDER BY ge.modified_at_fs DESC, ge.id COLLATE BINARY ASC
        LIMIT ?3
    "#;
    let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
    let mut statement = conn.prepare(sql)?;
    let rows = statement.query_map(
        params![
            fts_query,
            query,
            limit.min(MAX_TIER_CANDIDATES as usize) as u32
        ],
        |row| {
            Ok(RankedFileCandidate {
                rowid: row.get(0)?,
                class: 2,
                modified_at: row.get(2)?,
                stable_id: std::borrow::Cow::Owned(row.get::<_, String>(1)?.into_bytes()),
            })
        },
    )?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(DbError::from)
}

fn hydrate_ranked_filename_page(
    conn: &Connection,
    candidates: &[RankedFileCandidate<'_>],
) -> Result<Vec<GlobalSearchResult>, DbError> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let mut values = Vec::with_capacity(candidates.len() * 3);
    let mut placeholders = Vec::with_capacity(candidates.len());
    for (ordinal, candidate) in candidates.iter().enumerate() {
        let first = ordinal * 3 + 1;
        placeholders.push(format!("(?{first},?{},?{})", first + 1, first + 2));
        values.push(rusqlite::types::Value::Integer(candidate.rowid));
        values.push(rusqlite::types::Value::Integer(ordinal as i64));
        values.push(rusqlite::types::Value::Real(candidate.class as f64 + 2.0));
    }
    let sql = format!(
        r#"
        WITH candidate_keys(entry_rowid, ordinal, result_rank) AS (
            VALUES {}
        )
        SELECT ge.id, ge.volume_id, ge.platform_file_id, ge.name, ge.path,
               ge.extension, ge.is_directory, ge.size, ge.created_at_fs,
               ge.modified_at_fs, ge.file_attributes, ge.is_hidden, ge.is_system,
               ge.source_provider,
               EXISTS (
                   SELECT 1 FROM managed_entries me
                   JOIN managed_scopes ms ON ms.id=me.managed_scope_id
                   WHERE me.global_entry_id=ge.id AND me.enabled=1 AND ms.enabled=1
               ) AS managed,
               candidate_keys.result_rank AS rank,
               candidate_keys.entry_rowid AS candidate_rowid
        FROM candidate_keys
        JOIN global_entries ge ON ge.rowid=candidate_keys.entry_rowid
        JOIN global_volumes gv ON gv.id=ge.volume_id
        WHERE gv.enabled=1 AND gv.coverage_pending=0 AND ge.is_stale=0
        ORDER BY candidate_keys.ordinal ASC
        "#,
        placeholders.join(",")
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(rusqlite::params_from_iter(values.iter()), |row| {
        Ok((map_global_search_result(row)?, row.get::<_, i64>(16)?))
    })?;
    let rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.len() != candidates.len()
        || rows
            .iter()
            .zip(candidates)
            .any(|((result, rowid), candidate)| {
                *rowid != candidate.rowid || result.id.as_bytes() != candidate.stable_id.as_ref()
            })
    {
        return Err(DbError::Validation(
            "filename candidate hydration did not match its SQLite snapshot".into(),
        ));
    }
    Ok(rows.into_iter().map(|(result, _)| result).collect())
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

fn map_global_search_result(row: &rusqlite::Row<'_>) -> rusqlite::Result<GlobalSearchResult> {
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
