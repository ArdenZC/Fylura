//! Independent oracles for the three result channels.
//!
//! The production oracle never calls `search.rs`. It re-derives the five-tier
//! composition from the raw SQLite facts, then the test compares its ordered
//! output with the real production entry point. That catches composition bugs
//! (tier priority, de-duplication, offset placement, window truncation) which
//! is where the interesting defects live.
//!
//! BM25 itself is not re-implemented. Both the oracle and production ask SQLite
//! for `bm25(global_entries_fts, 8.0, 2.0, 1.0)`; the oracle only owns the
//! ordering *around* that score. The report states this limitation explicitly
//! instead of implying a fully independent scorer.

use super::candidate_b::ReadSqlConnection;
use super::{
    extension_class, fold, proposed_class, rank_all, sqlite_lower, Class, Hit, MAX_WINDOW,
};
use rusqlite::params;

/// One authorized, non-stale file fact as the oracle sees it.
#[derive(Clone, Debug)]
pub(super) struct AuthorizedRow {
    pub rowid: i64,
    pub id: String,
    pub name_normalized: String,
    pub path_normalized: String,
    pub extension: String,
    pub modified: Option<i64>,
}

/// Full authorized scan. Used by the oracle only; it is the definition of
/// "ground truth" and its own cost is reported separately so it can never be
/// confused with a candidate architecture's latency.
pub(super) fn authorized_rows<C: ReadSqlConnection + ?Sized>(
    conn: &C,
) -> rusqlite::Result<Vec<AuthorizedRow>> {
    let mut statement = conn.prepare(
        "SELECT ge.rowid, ge.id, ge.name_normalized, ge.path_normalized,
                ge.extension, ge.modified_at_fs
         FROM global_entries ge
         JOIN global_volumes gv ON gv.id = ge.volume_id
         WHERE gv.enabled = 1 AND ge.is_stale = 0",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(AuthorizedRow {
            rowid: row.get(0)?,
            id: row.get(1)?,
            name_normalized: row.get(2)?,
            path_normalized: row.get(3)?,
            extension: row.get(4)?,
            modified: row.get(5)?,
        })
    })?;
    rows.collect()
}

/// The proposed filename/path channel, computed from the full authorized fact
/// set with no candidate window and no FTS involvement.
pub(super) fn proposed_name_path_oracle(rows: &[AuthorizedRow], query: &str) -> Vec<Hit> {
    let query_folded = fold(query.trim());
    if query_folded.is_empty() {
        return Vec::new();
    }
    let query_sql_lower = sqlite_lower(query.trim());
    let hits = rows.iter().filter_map(|row| {
        let class = proposed_class(
            &row.name_normalized,
            &row.path_normalized,
            &row.extension,
            &query_folded,
            &query_sql_lower,
        )?;
        (!class.is_extension()).then_some(Hit {
            id: row.id.clone(),
            class,
            modified: row.modified,
        })
    });
    rank_all(hits)
}

/// The extension channel, independent of any filename/path class.
pub(super) fn extension_oracle(rows: &[AuthorizedRow], query: &str) -> Vec<Hit> {
    let query_sql_lower = sqlite_lower(query.trim());
    let hits = rows.iter().filter_map(|row| {
        let class = extension_class(&row.extension, &query_sql_lower)?;
        Some(Hit {
            id: row.id.clone(),
            class,
            modified: row.modified,
        })
    });
    rank_all(hits)
}

/// The exact ordered id list the production FTS tier yields for a query,
/// including the BM25 tie-break on `(modified_at_fs DESC, id ASC)`.
fn production_fts_tier<C: ReadSqlConnection + ?Sized>(
    conn: &C,
    query: &str,
    limit: u32,
) -> rusqlite::Result<Vec<String>> {
    let phrase = format!("\"{}\"", query.replace('"', "\"\""));
    let mut statement = conn.prepare(
        "SELECT ge.id
         FROM global_entries_fts
         CROSS JOIN global_entries ge
         CROSS JOIN global_volumes gv
         WHERE global_entries_fts MATCH ?1
           AND ge.rowid = global_entries_fts.rowid
           AND gv.id = ge.volume_id
           AND gv.enabled = 1 AND ge.is_stale = 0
         ORDER BY bm25(global_entries_fts, 8.0, 2.0, 1.0) ASC,
                  ge.modified_at_fs DESC, ge.id ASC
         LIMIT ?2",
    )?;
    let rows = statement.query_map(params![phrase, limit], |row| row.get::<_, String>(0))?;
    rows.collect()
}

/// Rows for one non-FTS production tier, ordered by `(modified DESC, id ASC)`.
fn tier_rows(rows: &[AuthorizedRow], query: &str, tier: ProductionTier) -> Vec<Hit> {
    let query_sql_lower = sqlite_lower(query);
    let mut hits = rows
        .iter()
        .filter(|row| match tier {
            ProductionTier::ExactName => row.name_normalized == query_sql_lower,
            ProductionTier::NamePrefix => {
                row.name_normalized.starts_with(&query_sql_lower)
                    && row.name_normalized != query_sql_lower
            }
            ProductionTier::ExactExtension => row.extension == query_sql_lower,
            ProductionTier::ExtensionPrefix => {
                row.extension.starts_with(&query_sql_lower) && row.extension != query_sql_lower
            }
            ProductionTier::PunctuationPrefix => row.name_normalized.starts_with(&query_sql_lower),
        })
        .map(|row| Hit {
            id: row.id.clone(),
            class: Class::NameExact,
            modified: row.modified,
        })
        .collect::<Vec<_>>();
    hits.sort();
    hits
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProductionTier {
    ExactName,
    NamePrefix,
    ExactExtension,
    ExtensionPrefix,
    PunctuationPrefix,
}

/// Mirrors `layer_limit` in `search.rs`.
fn layer_limit(target: u32, seen: usize) -> u32 {
    target
        .saturating_add(seen.min(MAX_WINDOW) as u32)
        .clamp(1, MAX_WINDOW as u32)
}

fn append_unique(
    results: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
    ids: Vec<String>,
    target: u32,
) {
    for id in ids {
        if seen.insert(id.clone()) {
            results.push(id);
            if results.len() >= target as usize {
                break;
            }
        }
    }
}

/// Full ordered id list for the current production search, rebuilt from the
/// documented tier contract rather than from `search.rs`.
pub(super) fn production_oracle<C: ReadSqlConnection + ?Sized>(
    conn: &C,
    rows: &[AuthorizedRow],
    query: &str,
    limit: u32,
    offset: u32,
) -> rusqlite::Result<Vec<String>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    if offset as usize >= MAX_WINDOW {
        return Ok(Vec::new());
    }
    let target = (offset + limit).min(MAX_WINDOW as u32);
    let mut results = Vec::with_capacity(target as usize);
    let mut seen = std::collections::HashSet::new();

    let tiers = [
        ProductionTier::ExactName,
        ProductionTier::NamePrefix,
        ProductionTier::ExactExtension,
        ProductionTier::ExtensionPrefix,
    ];
    for tier in tiers {
        if results.len() >= target as usize {
            break;
        }
        let tier_limit = layer_limit(target, seen.len()) as usize;
        let ids = tier_rows(rows, query, tier)
            .into_iter()
            .take(tier_limit)
            .map(|hit| hit.id)
            .collect::<Vec<_>>();
        append_unique(&mut results, &mut seen, ids, target);
    }

    if results.len() < target as usize && query.chars().count() >= 3 {
        let tier_limit = layer_limit(target, seen.len());
        let all_alphanumeric = query
            .chars()
            .all(|character| character.is_alphanumeric() || character.is_whitespace());
        let ids = if all_alphanumeric {
            production_fts_tier(conn, query, tier_limit)?
        } else {
            tier_rows(rows, query, ProductionTier::PunctuationPrefix)
                .into_iter()
                .take(tier_limit as usize)
                .map(|hit| hit.id)
                .collect()
        };
        append_unique(&mut results, &mut seen, ids, target);
    }

    Ok(results
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect())
}

/// How many authorized rows the production FTS tier would match, before the
/// candidate window truncates it. Reported so a "fast" number cannot hide a
/// truncated result set.
pub(super) fn production_fts_match_count<C: ReadSqlConnection + ?Sized>(
    conn: &C,
    query: &str,
) -> rusqlite::Result<u64> {
    if query.chars().count() < 3 {
        return Ok(0);
    }
    let phrase = format!("\"{}\"", query.replace('"', "\"\""));
    conn.query_row(
        "SELECT COUNT(*)
         FROM global_entries_fts
         CROSS JOIN global_entries ge
         CROSS JOIN global_volumes gv
         WHERE global_entries_fts MATCH ?1
           AND ge.rowid = global_entries_fts.rowid
           AND gv.id = ge.volume_id
           AND gv.enabled = 1 AND ge.is_stale = 0",
        params![phrase],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count as u64)
}

/// Divergence between the trigram MATCH set and a plain Rust substring scan.
/// #494 explicitly forbids assuming these are equivalent, so the experiment
/// measures the difference instead of asserting it away.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub(super) struct MatchSetDivergence {
    pub fts_only: u64,
    pub rust_only: u64,
    pub both: u64,
}

pub(super) fn match_set_divergence<C: ReadSqlConnection + ?Sized>(
    conn: &C,
    rows: &[AuthorizedRow],
    query: &str,
) -> rusqlite::Result<MatchSetDivergence> {
    if query.chars().count() < 3 {
        return Ok(MatchSetDivergence::default());
    }
    let phrase = format!("\"{}\"", query.replace('"', "\"\""));
    let mut statement = conn.prepare(
        "SELECT ge.rowid
         FROM global_entries_fts
         CROSS JOIN global_entries ge
         CROSS JOIN global_volumes gv
         WHERE global_entries_fts MATCH ?1
           AND ge.rowid = global_entries_fts.rowid
           AND gv.id = ge.volume_id
           AND gv.enabled = 1 AND ge.is_stale = 0",
    )?;
    let fts = statement
        .query_map(params![phrase], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<std::collections::HashSet<i64>>>()?;

    let folded = fold(query);
    let mut divergence = MatchSetDivergence::default();
    let mut rust = std::collections::HashSet::new();
    for row in rows {
        if row.name_normalized.contains(&folded)
            || row.path_normalized.contains(&folded)
            || row.extension.contains(&folded)
        {
            rust.insert(row.rowid);
        }
    }
    for rowid in &fts {
        if rust.contains(rowid) {
            divergence.both += 1;
        } else {
            divergence.fts_only += 1;
        }
    }
    divergence.rust_only = rust.iter().filter(|rowid| !fts.contains(rowid)).count() as u64;
    Ok(divergence)
}
