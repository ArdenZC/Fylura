//! Candidate A: keep SQLite + FTS5, drop BM25 from the hot path, rank in Rust.
//!
//! A never asks SQLite to sort. It enumerates the complete FTS posting set for
//! the query (no `ORDER BY`, no `LIMIT 80/512`), then classifies and keeps a
//! bounded Top-K in Rust. The full enumeration cost — including the 50k-hit
//! case — is part of A's measured latency, not hidden behind a prefix window.
//!
//! Short queries (< 3 codepoints) cannot use a trigram MATCH at all. A must
//! then scan the authorized fact set; that path is measured separately and is
//! reported as A's short-query tail instead of being silently declared
//! unsupported.

use super::candidate_b::ReadSqlConnection;
use super::{fold, proposed_class, sqlite_lower, Class, Hit, MAX_WINDOW};
use crate::db::DbError;
use rusqlite::params;
use std::collections::BinaryHeap;

/// Bounded Top-K collector. `Hit::cmp` is "better first", so the heap's peek is
/// the worst retained hit and a candidate replaces it only when strictly better.
pub(super) struct TopK {
    limit: usize,
    heap: BinaryHeap<Hit>,
}

impl TopK {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            limit,
            heap: BinaryHeap::with_capacity(limit.min(1024)),
        }
    }

    pub(super) fn offer(&mut self, hit: Hit) {
        if self.limit == 0 {
            return;
        }
        if self.heap.len() < self.limit {
            self.heap.push(hit);
        } else if let Some(worst) = self.heap.peek() {
            if hit < *worst {
                self.heap.pop();
                self.heap.push(hit);
            }
        }
    }

    pub(super) fn finish(self) -> Vec<Hit> {
        self.heap.into_sorted_vec()
    }
}

/// The two candidate channels A produces.
#[derive(Clone, Debug, Default)]
pub(super) struct Channels {
    pub name_path: Vec<Hit>,
    pub extension: Vec<Hit>,
    /// Rows the candidate source produced before ranking. Reported so a fast
    /// number cannot hide a truncated candidate set.
    pub candidates_enumerated: u64,
    /// `fts` when the trigram postings covered the query, `scan` otherwise.
    pub enumeration: &'static str,
}

pub(super) fn escape_glob(value: &str) -> String {
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

/// Extension channel. Served by the existing covering index in every
/// architecture (production, A and B), so it is deliberately not a differentiator
/// and is measured as its own cost line.
pub(super) fn extension_channel<C: ReadSqlConnection + ?Sized>(
    conn: &C,
    query: &str,
    window: usize,
) -> Result<Vec<Hit>, DbError> {
    let query_sql_lower = sqlite_lower(query.trim());
    if query_sql_lower.is_empty() {
        return Ok(Vec::new());
    }
    let mut hits = Vec::new();
    let limit = window.min(MAX_WINDOW);

    let mut exact = conn.prepare(
        "SELECT ge.id, ge.extension, ge.modified_at_fs
         FROM global_entries ge INDEXED BY idx_global_entries_active_extension_order
         JOIN global_volumes gv ON gv.id = ge.volume_id
         WHERE gv.enabled = 1 AND ge.is_stale = 0 AND ge.extension = ?1
         ORDER BY ge.modified_at_fs DESC, ge.id ASC
         LIMIT ?2",
    )?;
    let rows = exact.query_map(params![query_sql_lower, limit as u32], |row| {
        Ok(Hit {
            id: row.get(0)?,
            class: Class::ExtensionExact,
            modified: row.get(2)?,
        })
    })?;
    for row in rows {
        hits.push(row?);
    }

    let glob = format!("{}*", escape_glob(query.trim()));
    let mut prefix = conn.prepare(
        "SELECT ge.id, ge.extension, ge.modified_at_fs
         FROM global_entries ge INDEXED BY idx_global_entries_active_extension_order
         JOIN global_volumes gv ON gv.id = ge.volume_id
         WHERE gv.enabled = 1 AND ge.is_stale = 0
           AND ge.extension GLOB ?2 AND ge.extension <> ?1
         ORDER BY ge.modified_at_fs DESC, ge.id ASC
         LIMIT ?3",
    )?;
    let rows = prefix.query_map(params![query_sql_lower, glob, limit as u32], |row| {
        Ok(Hit {
            id: row.get(0)?,
            class: Class::ExtensionPrefix,
            modified: row.get(2)?,
        })
    })?;
    for row in rows {
        hits.push(row?);
    }

    let mut ranked = super::rank_all(hits);
    ranked.truncate(limit);
    Ok(ranked)
}

/// Enumerate the complete trigram posting set for `query`. No ordering, no
/// early LIMIT: the caller must see every match so the Top-K is a true global
/// Top-K.
fn enumerate_fts<C: ReadSqlConnection + ?Sized>(
    conn: &C,
    query: &str,
    collector: &mut impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<()>,
) -> rusqlite::Result<u64> {
    let phrase = format!("\"{}\"", query.replace('"', "\"\""));
    let mut statement = conn.prepare_cached(
        "SELECT ge.name_normalized, ge.path_normalized, ge.extension,
                ge.modified_at_fs, ge.id
         FROM global_entries_fts
         CROSS JOIN global_entries ge
         CROSS JOIN global_volumes gv
         WHERE global_entries_fts MATCH ?1
           AND ge.rowid = global_entries_fts.rowid
           AND gv.id = ge.volume_id
           AND gv.enabled = 1 AND ge.is_stale = 0",
    )?;
    let mut rows = statement.query(params![phrase])?;
    let mut count = 0u64;
    while let Some(row) = rows.next()? {
        collector(row)?;
        count += 1;
    }
    Ok(count)
}

/// Full authorized scan, used only when the query is too short for a trigram
/// MATCH. This is A's honest short-query tail.
fn enumerate_scan<C: ReadSqlConnection + ?Sized>(
    conn: &C,
    collector: &mut impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<()>,
) -> rusqlite::Result<u64> {
    let mut statement = conn.prepare_cached(
        "SELECT ge.name_normalized, ge.path_normalized, ge.extension,
                ge.modified_at_fs, ge.id
         FROM global_entries ge
         JOIN global_volumes gv ON gv.id = ge.volume_id
         WHERE gv.enabled = 1 AND ge.is_stale = 0",
    )?;
    let mut rows = statement.query([])?;
    let mut count = 0u64;
    while let Some(row) = rows.next()? {
        collector(row)?;
        count += 1;
    }
    Ok(count)
}

/// Candidate A search. `window` is the product candidate window (4096).
pub(super) fn search<C: ReadSqlConnection + ?Sized>(
    conn: &C,
    query: &str,
    window: usize,
) -> Result<Channels, DbError> {
    let trimmed = query.trim();
    let mut channels = Channels::default();
    if trimmed.is_empty() {
        return Ok(channels);
    }
    let window = window.min(MAX_WINDOW);
    let query_folded = fold(trimmed);
    let query_sql_lower = sqlite_lower(trimmed);
    let use_fts = trimmed.chars().count() >= 3;
    let mut name_path = TopK::new(window);
    let enumerated = {
        let mut collect = |row: &rusqlite::Row<'_>| -> rusqlite::Result<()> {
            let name: String = row.get(0)?;
            let path: String = row.get(1)?;
            let extension: String = row.get(2)?;
            let modified: Option<i64> = row.get(3)?;
            let id: String = row.get(4)?;
            if let Some(class) =
                proposed_class(&name, &path, &extension, &query_folded, &query_sql_lower)
            {
                if !class.is_extension() {
                    name_path.offer(Hit {
                        id,
                        class,
                        modified,
                    });
                }
            }
            Ok(())
        };
        if use_fts {
            enumerate_fts(conn, trimmed, &mut collect)?
        } else {
            enumerate_scan(conn, &mut collect)?
        }
    };

    channels.enumeration = if use_fts { "fts" } else { "scan" };
    channels.candidates_enumerated = enumerated;
    channels.name_path = name_path.finish();
    channels.extension = extension_channel(conn, trimmed, window)?;
    Ok(channels)
}

/// Raw enumeration count without ranking, for the cost-attribution report.
pub(super) fn enumeration_only<C: ReadSqlConnection + ?Sized>(
    conn: &C,
    query: &str,
) -> rusqlite::Result<u64> {
    let trimmed = query.trim();
    if trimmed.chars().count() >= 3 {
        enumerate_fts(conn, trimmed, &mut |_| Ok(()))
    } else {
        enumerate_scan(conn, &mut |_| Ok(()))
    }
}
