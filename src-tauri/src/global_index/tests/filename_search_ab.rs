//! #494: test-only A/B file-name relevance experiment.
//!
//! No shipping search path, source authority, schema or FTS ranking is changed.
//! A enumerates FTS5 candidates without BM25 ordering. B builds a disposable,
//! snapshot-scoped, name-only UTF-8/offset view and uses bounded Top-K.
//! B's *full coverage* comparison deliberately runs A as a path/extension
//! fallback: the resulting cost must NOT be reported as a fast B-only search.
//! The ranking classes here are a PROPOSAL, not an approved product contract.

use super::{populate_synthetic_database, test_db_path, test_entry, test_volume};
use crate::db::Database;
use rusqlite::{params, Connection};
use serde_json::json;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::ops::Range;
use std::time::Instant;

const MAX_WINDOW: usize = 4_096;
const WARMUP: usize = 5;
const SAMPLES: usize = 30;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Class {
    NameExact,
    NamePrefix,
    NameSubstring,
    ExtensionExact,
    ExtensionPrefix,
    PathOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Hit {
    id: String,
    class: Class,
    modified: Option<i64>,
}

// Greater = worse so BinaryHeap::peek() is the least desirable retained hit.
impl Ord for Hit {
    fn cmp(&self, other: &Self) -> Ordering {
        self.class
            .cmp(&other.class)
            .then_with(|| other.modified.cmp(&self.modified))
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl PartialOrd for Hit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone)]
struct Row {
    id: String,
    name: String,
    path: String,
    extension: String,
    modified: Option<i64>,
}

fn classify(row: &Row, query: &str) -> Option<Class> {
    let name = row.name.to_lowercase();
    let path = row.path.to_lowercase();
    let extension = row.extension.to_lowercase();
    if name == query {
        Some(Class::NameExact)
    } else if name.starts_with(query) {
        Some(Class::NamePrefix)
    } else if name.contains(query) {
        Some(Class::NameSubstring)
    } else if extension == query {
        Some(Class::ExtensionExact)
    } else if extension.starts_with(query) {
        Some(Class::ExtensionPrefix)
    } else if path.contains(query) {
        Some(Class::PathOnly)
    } else {
        None
    }
}

fn rank_all(rows: impl IntoIterator<Item = Row>, query: &str) -> Vec<Hit> {
    let query = query.to_lowercase();
    let mut unique = HashMap::<String, Hit>::new();
    for row in rows {
        if let Some(class) = classify(&row, &query) {
            let hit = Hit {
                id: row.id,
                class,
                modified: row.modified,
            };
            let previous = unique.entry(hit.id.clone()).or_insert_with(|| hit.clone());
            if hit < *previous {
                *previous = hit;
            }
        }
    }
    let mut ranked = unique.into_values().collect::<Vec<_>>();
    ranked.sort();
    ranked
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: row.get(0)?,
        name: row.get(1)?,
        path: row.get(2)?,
        extension: row.get(3)?,
        modified: row.get(4)?,
    })
}

// Independent product-oracle over the current *authorized, non-stale* SQLite
// facts. It intentionally does not use the FTS or the compact-view code path.
fn oracle(conn: &Connection, query: &str) -> rusqlite::Result<Vec<Hit>> {
    let mut statement = conn.prepare(
        "SELECT ge.id, ge.name, ge.path, ge.extension, ge.modified_at_fs
         FROM global_entries ge
         JOIN global_volumes gv ON gv.id = ge.volume_id
         WHERE gv.enabled = 1 AND ge.is_stale = 0",
    )?;
    let rows = statement
        .query_map([], read_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rank_all(rows, query))
}

// A: existing FTS5 postings give *all* eligible candidates, not BM25 scores
// and not an unsafe LIMIT 80/512. The enumerated candidate count is a cost.
fn candidate_a(conn: &Connection, query: &str) -> rusqlite::Result<Option<Vec<Hit>>> {
    if query.chars().count() < 3 {
        // Trigram MATCH cannot be used to infer that there are no matches.
        return Ok(None);
    }
    let phrase = format!("\"{}\"", query.replace('"', "\"\""));
    let mut statement = conn.prepare(
        "SELECT ge.id, ge.name, ge.path, ge.extension, ge.modified_at_fs
         FROM global_entries_fts
         CROSS JOIN global_entries ge
         CROSS JOIN global_volumes gv
         WHERE global_entries_fts MATCH ?1
           AND ge.rowid = global_entries_fts.rowid
           AND gv.id = ge.volume_id
           AND gv.enabled = 1 AND ge.is_stale = 0",
    )?;
    let rows = statement
        .query_map(params![phrase], read_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(rank_all(rows, query)))
}

struct NameSlot {
    id: Range<usize>,
    name: Range<usize>,
    modified: Option<i64>,
}

struct CompactFilenameView {
    generation: u64,
    names: Vec<u8>,
    ids: Vec<u8>,
    slots: Vec<NameSlot>,
}

impl CompactFilenameView {
    // The caller must hold a SQLite read transaction for the ENTIRE build,
    // lookup/fallback and result-hydration experiment. A generation integer
    // provided by tests is NOT an implementation of production source fencing.
    fn build(conn: &Connection, generation: u64) -> rusqlite::Result<Self> {
        let mut statement = conn.prepare(
            "SELECT ge.id, ge.name, ge.modified_at_fs
             FROM global_entries ge
             JOIN global_volumes gv ON gv.id = ge.volume_id
             WHERE gv.enabled = 1 AND ge.is_stale = 0",
        )?;
        let mut view = Self {
            generation,
            names: Vec::new(),
            ids: Vec::new(),
            slots: Vec::new(),
        };
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let modified: Option<i64> = row.get(2)?;
            let id_start = view.ids.len();
            view.ids.extend_from_slice(id.as_bytes());
            let name_start = view.names.len();
            view.names.extend_from_slice(name.to_lowercase().as_bytes());
            view.slots.push(NameSlot {
                id: id_start..view.ids.len(),
                name: name_start..view.names.len(),
                modified,
            });
        }
        Ok(view)
    }

    // Conservative heap capacity accounting, NOT RSS or SQLite pager cost.
    fn heap_capacity_bytes(&self) -> usize {
        self.names.capacity()
            + self.ids.capacity()
            + self.slots.capacity() * std::mem::size_of::<NameSlot>()
    }

    fn name_top_k(&self, query: &str, generation: u64, k: usize) -> Option<Vec<Hit>> {
        if generation != self.generation {
            return None;
        }
        let needle = query.to_lowercase();
        if needle.is_empty() {
            return Some(Vec::new());
        }
        let limit = k.min(MAX_WINDOW);
        let mut heap = BinaryHeap::<Hit>::with_capacity(limit);
        for slot in &self.slots {
            let name = std::str::from_utf8(&self.names[slot.name.clone()])
                .expect("UTF-8 source names");
            let class = if name == needle {
                Some(Class::NameExact)
            } else if name.starts_with(&needle) {
                Some(Class::NamePrefix)
            } else if name.contains(&needle) {
                Some(Class::NameSubstring)
            } else {
                None
            };
            let Some(class) = class else { continue };
            if limit == 0 {
                continue;
            }
            let id = std::str::from_utf8(&self.ids[slot.id.clone()])
                .expect("UTF-8 stable IDs")
                .to_string();
            let hit = Hit {
                id,
                class,
                modified: slot.modified,
            };
            if heap.len() < limit {
                heap.push(hit);
            } else if heap.peek().is_some_and(|worst| hit < *worst) {
                heap.pop();
                heap.push(hit);
            }
        }
        Some(heap.into_sorted_vec())
    }
}

fn full_b_with_expensive_fallback(
    conn: &Connection,
    view: &CompactFilenameView,
    generation: u64,
    query: &str,
) -> rusqlite::Result<Option<Vec<Hit>>> {
    let Some(names) = view.name_top_k(query, generation, MAX_WINDOW) else {
        return Ok(None);
    };
    let Some(mut all) = candidate_a(conn, query)? else {
        // A *real* shipping fallback must handle short Unicode/ASCII and
        // punctuation through authorized SQLite source facts. Fail closed here.
        return Ok(None);
    };
    // This deliberately performs A after B for full path/extension coverage:
    // it is a correctness control, not evidence of a faster unified engine.
    all.extend(names);
    let mut unique = HashMap::<String, Hit>::new();
    for hit in all {
        let previous = unique.entry(hit.id.clone()).or_insert_with(|| hit.clone());
        if hit < *previous {
            *previous = hit;
        }
    }
    let mut result = unique.into_values().collect::<Vec<_>>();
    result.sort();
    Ok(Some(result))
}

#[test]
fn issue494_bounded_top_k_is_equal_to_unbounded_name_oracle() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open migrated fixture");
    db.upsert_global_volume(&test_volume()).expect("authorize fixture source");
    let mut rows = Vec::new();
    for index in 0..180 {
        let name = format!("annual-report-{index:03}.txt");
        let mut item = test_entry(&format!(r"C:\Global\{name}"), &name, false);
        item.modified_at_fs = Some((index % 7) as i64);
        rows.push(item);
    }
    db.upsert_global_entries_batch(&rows).expect("seed real FTS triggers");
    let conn = Connection::open(&path).expect("read fixture");
    conn.execute_batch("BEGIN").expect("stable read snapshot");
    let view = CompactFilenameView::build(&conn, 1).expect("build name view");
    let expected = oracle(&conn, "report").expect("independent matching oracle");
    let actual = view.name_top_k("report", 1, 80).expect("generation matches");
    assert_eq!(actual, expected[..80]);
    assert!(view.name_top_k("report", 2, 80).is_none());
    conn.execute_batch("ROLLBACK").expect("end read snapshot");
    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn issue494_a_b_coverage_fencing_and_revision_boundaries() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open migrated fixture");
    db.upsert_global_volume(&test_volume()).expect("seed enabled volume");
    let mut disabled = test_volume();
    disabled.id = "gv_disabled_494".to_string();
    disabled.stable_volume_id = "disabled-volume-494".to_string();
    disabled.enabled = false;
    db.upsert_global_volume(&disabled).expect("seed disabled volume");
    let mut ignored = test_entry(r"C:\Global\report\disabled-report.txt", "disabled-report.txt", false);
    ignored.volume_id = disabled.id.clone();
    let inputs = [
        test_entry(r"C:\Global\report\notes.txt", "notes.txt", false),
        test_entry(r"C:\Global\annual-report.txt", "annual-report.txt", false),
        test_entry(r"C:\Global\report-summary.txt", "report-summary.txt", false),
        test_entry(r"C:\Global\invoice-2026.txt", "invoice-2026.txt", false),
        test_entry(r"C:\Global\数据库示例.txt", "数据库示例.txt", false),
        test_entry(r"C:\Global\report-stale.txt", "report-stale.txt", false),
        ignored,
    ];
    db.upsert_global_entries_batch(&inputs).expect("seed real indexed fixture");
    db.conn().expect("writer").execute(
        "UPDATE global_entries SET is_stale = 1 WHERE name = 'report-stale.txt'",
        [],
    ).expect("mark stale fixture");

    let conn = Connection::open(&path).expect("read candidate fixture");
    conn.execute_batch("BEGIN").expect("same-source transaction");
    let view = CompactFilenameView::build(&conn, 41).expect("read authorized names");
    for query in ["report", "invoice", "数据库", "zzznomatchtoken"] {
        let expected = oracle(&conn, query).expect("full oracle");
        let a = candidate_a(&conn, query).expect("A candidate").expect("FTS length >=3");
        let b = full_b_with_expensive_fallback(&conn, &view, 41, query)
            .expect("B with explicit A fallback").expect("fenced source");
        assert_eq!(a, expected, "A differs from oracle for {query}");
        assert_eq!(b, expected, "B/fallback differs from oracle for {query}");
        for page in 0..=expected.len() {
            let end = (page + 2).min(expected.len());
            assert_eq!(&b[page..end], &expected[page..end]);
        }
    }
    assert!(candidate_a(&conn, "数").expect("short FTS").is_none());
    assert!(full_b_with_expensive_fallback(&conn, &view, 42, "report")
        .expect("revision check").is_none());
    conn.execute_batch("ROLLBACK").expect("release reader");

    db.conn().expect("writer").execute(
        "UPDATE global_entries SET name = 'renamed.txt',
            name_normalized = 'renamed.txt',
            path = 'C:\\Global\\renamed.txt',
            path_normalized = 'c:\\global\\renamed.txt'
         WHERE name = 'annual-report.txt'",
        [],
    ).expect("rename real indexed row");
    let new_reader = Connection::open(&path).expect("reopen");
    new_reader.execute_batch("BEGIN").expect("new generation");
    let new_view = CompactFilenameView::build(&new_reader, 42).expect("rebuild view");
    assert!(view.name_top_k("report", 42, 80).is_none());
    assert_eq!(
        new_view.name_top_k("report", 42, 80).expect("valid generation"),
        oracle(&new_reader, "report").expect("post-rename oracle")
            .into_iter().filter(|hit| hit.class <= Class::NameSubstring)
            .collect::<Vec<_>>()
    );
    new_reader.execute_batch("ROLLBACK").expect("end snapshot");
    drop(new_reader);
    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(path);
}

fn timed_samples(mut run: impl FnMut(), warmup: usize, samples: usize) -> (f64, f64, f64) {
    for _ in 0..warmup {
        run();
    }
    let mut elapsed = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        run();
        elapsed.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    elapsed.sort_by(f64::total_cmp);
    let percentile = |q: f64| -> f64 {
        elapsed[((elapsed.len() - 1) as f64 * q).ceil() as usize]
    };
    (percentile(0.50), percentile(0.95), percentile(0.99))
}

// Manual only. Uses the existing official schema, 12-class fixture generator,
// FTS triggers and source guards. A/B below are NOT the existing pooled Search
// nor a complete app Snapshot. No experimental metric replaces its 100ms gate.
#[test]
#[ignore = "run only as an explicitly bounded #494 synthetic experiment"]
fn issue494_filename_ab_synthetic_release_probe() {
    let entries: u64 = std::env::var("ZC_FILENAME_AB_ENTRIES")
        .unwrap_or_else(|_| "100000".to_string())
        .parse().expect("parse benchmark entry count");
    assert!([100_000, 500_000, 1_000_000].contains(&entries));
    let path = test_db_path();
    let db = Database::open(&path).expect("official synthetic schema");
    db.upsert_global_volume(&test_volume()).expect("seed fixture volume");
    let (_, generation_ms, insertion_ms, _) = populate_synthetic_database(&db, entries);
    let conn = Connection::open(&path).expect("open read-only experiment");
    conn.execute_batch("BEGIN").expect("coherent read snapshot");
    let started = Instant::now();
    let view = CompactFilenameView::build(&conn, 1).expect("build compact view");
    let build_ms = started.elapsed().as_secs_f64() * 1000.0;
    for query in ["report", "invoice", "zzznomatchtoken"] {
        let oracle_ids = oracle(&conn, query).expect("independent oracle");
        let a = candidate_a(&conn, query).expect("A").expect("valid query");
        let b_full = full_b_with_expensive_fallback(&conn, &view, 1, query)
            .expect("B+fallback").expect("valid generation");
        assert_eq!(a, oracle_ids, "A full result oracle {query}");
        assert_eq!(b_full, oracle_ids, "B/full fallback oracle {query}");
        let a_ms = timed_samples(|| {
            let _ = candidate_a(&conn, query).expect("A").expect("valid query");
        }, WARMUP, SAMPLES);
        let b_name_ms = timed_samples(|| {
            let _ = view.name_top_k(query, 1, 80).expect("fenced view");
        }, WARMUP, SAMPLES);
        let b_full_ms = timed_samples(|| {
            let _ = full_b_with_expensive_fallback(&conn, &view, 1, query)
                .expect("full B").expect("valid generation");
        }, WARMUP, SAMPLES);
        eprintln!("{}", json!({
            "experiment": "issue494_filename_ab",
            "entries": entries,
            "query": query,
            "oracle_hits": oracle_ids.len(),
            "build_ms": build_ms,
            "fixture_generation_ms": generation_ms,
            "fixture_insertion_ms": insertion_ms,
            "compact_heap_capacity_bytes_only": view.heap_capacity_bytes(),
            "a_full_enumeration_p50_p95_p99_ms": [a_ms.0, a_ms.1, a_ms.2],
            "b_filename_only_p50_p95_p99_ms": [b_name_ms.0, b_name_ms.1, b_name_ms.2],
            "b_full_with_expensive_fts_fallback_p50_p95_p99_ms": [b_full_ms.0, b_full_ms.1, b_full_ms.2],
            "caveat": "SQL-only test, no RSS/CPU/writer/pooled/source-facts/Snapshot measurement"
        }));
    }
    conn.execute_batch("ROLLBACK").expect("end benchmark snapshot");
    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(path);
}
