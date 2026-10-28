//! Correctness evidence for #494.
//!
//! Rules enforced here:
//!
//! * A difference in *ranking* between the proposed order and production BM25 is
//!   product material, not a test failure. Those comparisons are reported as
//!   diffs, never asserted equal.
//! * A *missing* match is always a failure. Every oracle and every candidate
//!   must return exactly the set of authorized, non-stale rows that match the
//!   query, and the tests assert set coverage independently of order.
//! * No assertion is relaxed to make an implementation pass.

use super::candidate_a;
use super::candidate_b::{self, CompactView, Fence};
use super::fixtures;
use super::oracle;
use super::{fold, sqlite_lower, window_of, Class, Hit, MAX_WINDOW, PAGE_LIMIT};
use crate::global_index::models::GlobalSearchResult;
use crate::global_index::search::search_global_entries_on_connection;
use rusqlite::Connection;

const EDGE_QUERIES: [&str; 13] = [
    "report",
    "annual-report.pdf",
    "report.pdf",
    "数据库",
    "数",
    "final-v2",
    "Résumé",
    "RÉSUMÉ",
    "résumé",
    "pdf",
    "quarterly-review",
    "duplicate.txt",
    "zzznomatchtoken",
];

fn production_ids(conn: &Connection, query: &str, limit: u32, offset: u32) -> Vec<String> {
    search_global_entries_on_connection(conn, query, limit, offset)
        .expect("production search")
        .into_iter()
        .map(|row: GlobalSearchResult| row.id)
        .collect()
}

/// The product candidate window of a full ranked oracle list.
fn window(hits: &[Hit]) -> &[Hit] {
    window_of(hits, MAX_WINDOW)
}

fn ids(hits: &[Hit]) -> Vec<String> {
    hits.iter().map(|hit| hit.id.clone()).collect()
}

fn assert_same_order(actual: &[Hit], expected: &[Hit], context: &str) {
    let actual_ids = ids(actual);
    let expected_ids = ids(expected);
    assert_eq!(
        actual_ids,
        expected_ids,
        "{context}: ordered ids differ\nactual classes: {:?}\nexpected classes: {:?}",
        actual.iter().map(|hit| hit.class).collect::<Vec<_>>(),
        expected.iter().map(|hit| hit.class).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Oracle validity
// ---------------------------------------------------------------------------

#[test]
fn issue494_production_oracle_reproduces_production_search() {
    let fixture = fixtures::edge_cases();
    let conn = fixture.connection();
    for query in EDGE_QUERIES {
        for (limit, offset) in [(PAGE_LIMIT, 0), (10, 3), (PAGE_LIMIT, 1_000)] {
            let expected = production_ids(&conn, query, limit, offset);
            let rows = oracle::authorized_rows(&conn).expect("authorized facts");
            let actual =
                oracle::production_oracle(&conn, &rows, query, limit, offset).expect("oracle");
            assert_eq!(
                actual, expected,
                "production oracle diverged for {query} limit {limit} offset {offset}"
            );
        }
    }
}

#[test]
fn issue494_production_oracle_holds_on_synthetic_scale_fixture() {
    // 100k is the smallest official scale and the pre-flight gate for 500k/1M.
    let fixture = fixtures::official(100_000);
    let conn = fixture.connection();
    let rows = oracle::authorized_rows(&conn).expect("authorized facts");
    for query in [
        "report",
        "invoice",
        "IMG_",
        "pdf",
        "jp",
        "数据库",
        "final-v2",
        "zzznomatchtoken",
        "Annual Budget 2026.xlsx",
    ] {
        for (limit, offset) in [(PAGE_LIMIT, 0), (PAGE_LIMIT, 80)] {
            let expected = production_ids(&conn, query, limit, offset);
            let actual =
                oracle::production_oracle(&conn, &rows, query, limit, offset).expect("oracle");
            assert_eq!(
                actual, expected,
                "production oracle diverged for {query} limit {limit} offset {offset}"
            );
        }
    }
}

#[test]
fn issue494_oracles_exclude_stale_and_deauthorized_rows() {
    let fixture = fixtures::edge_cases();
    let conn = fixture.connection();
    let rows = oracle::authorized_rows(&conn).expect("authorized facts");
    assert!(
        rows.iter()
            .all(|row| !row.path_normalized.contains("revoked")),
        "disabled volume rows must never reach the oracle fact set"
    );
    assert!(
        !rows
            .iter()
            .any(|row| row.name_normalized == "pre-report-notes.pdf"),
        "stale rows must never reach the oracle fact set"
    );
    for query in EDGE_QUERIES {
        let proposed = oracle::proposed_name_path_oracle(&rows, query);
        assert!(proposed
            .iter()
            .all(|hit| !hit.id.is_empty() && !hit.class.is_extension()));
    }
}

// ---------------------------------------------------------------------------
// Candidate A
// ---------------------------------------------------------------------------

#[test]
fn issue494_candidate_a_matches_proposed_oracle_on_edge_cases() {
    let fixture = fixtures::edge_cases();
    let conn = fixture.connection();
    let rows = oracle::authorized_rows(&conn).expect("authorized facts");
    for query in EDGE_QUERIES {
        let expected = oracle::proposed_name_path_oracle(&rows, query);
        let actual = candidate_a::search(&conn, query, MAX_WINDOW).expect("candidate A");
        assert_same_order(
            &actual.name_path,
            window(&expected),
            &format!("A name/path {query}"),
        );
    }
}

#[test]
fn issue494_candidate_a_matches_proposed_oracle_on_density_fixture() {
    let fixture = fixtures::density(100_000);
    let conn = fixture.connection();
    let rows = oracle::authorized_rows(&conn).expect("authorized facts");
    for (query, expected_hits) in fixtures::DENSITY_QUERIES {
        let expected = oracle::proposed_name_path_oracle(&rows, query);
        let actual = candidate_a::search(&conn, query, MAX_WINDOW).expect("candidate A");
        assert_same_order(
            &actual.name_path,
            window(&expected),
            &format!("A name/path {query}"),
        );
        if !query.contains("path-only") && query != "pdf" {
            assert_eq!(
                expected.len() as u64,
                expected_hits,
                "density fixture must produce the documented hit count for {query}"
            );
        }
    }
}

#[test]
fn issue494_candidate_a_paginates_without_gaps_or_repeats() {
    let fixture = fixtures::density(100_000);
    let conn = fixture.connection();
    let rows = oracle::authorized_rows(&conn).expect("authorized facts");
    let query = "density-five-thousand";
    let expected = oracle::proposed_name_path_oracle(&rows, query);
    assert_eq!(expected.len(), 5_000);
    let actual = candidate_a::search(&conn, query, MAX_WINDOW).expect("candidate A");
    assert_eq!(actual.name_path.len(), MAX_WINDOW);
    // 64 divides the 4096 candidate window exactly, so the whole window is
    // reachable page by page with no short final page.
    const PAGE: usize = 64;
    let mut paged = Vec::new();
    for page in 0..(actual.name_path.len() / PAGE) {
        let start = page * PAGE;
        paged.extend_from_slice(&actual.name_path[start..start + PAGE]);
    }
    assert_eq!(ids(&paged), ids(&expected[..paged.len()]));
    let unique: std::collections::HashSet<&str> = paged.iter().map(|hit| hit.id.as_str()).collect();
    assert_eq!(unique.len(), paged.len(), "pagination must not repeat ids");
    assert_eq!(
        paged.len(),
        MAX_WINDOW,
        "the whole candidate window must be reachable by paging"
    );
}

// ---------------------------------------------------------------------------
// Candidate B
// ---------------------------------------------------------------------------

fn build_view(snapshot: &candidate_b::ReadSnapshot<'_>) -> CompactView {
    CompactView::build(snapshot, candidate_b::memory_budget_bytes())
        .expect("build compact view")
        .expect("compact view fits the budget")
}

#[test]
fn issue494_candidate_b_matches_proposed_oracle_on_edge_cases() {
    let fixture = fixtures::edge_cases();
    let conn = fixture.connection();
    let snapshot = candidate_b::ReadSnapshot::begin(&conn).expect("read snapshot");
    let rows = oracle::authorized_rows(snapshot.connection()).expect("authorized facts");
    let view = build_view(&snapshot);
    for query in EDGE_QUERIES {
        let expected = oracle::proposed_name_path_oracle(&rows, query);
        let lookup =
            candidate_b::lookup(&view, &snapshot, query, MAX_WINDOW, None).expect("candidate B");
        assert_eq!(lookup.fence, Fence::Valid);
        assert_eq!(lookup.hydrated_dropped, 0, "{query} dropped rows");
        assert_same_order(
            &lookup.name_path,
            window(&expected),
            &format!("B name/path {query}"),
        );

        let expected_extension = oracle::extension_oracle(&rows, query);
        assert_same_order(
            &lookup.extension,
            window(&expected_extension),
            &format!("B extension {query}"),
        );
    }
}

#[test]
fn issue494_candidate_b_matches_proposed_oracle_on_density_fixture() {
    let fixture = fixtures::density(100_000);
    let conn = fixture.connection();
    let snapshot = candidate_b::ReadSnapshot::begin(&conn).expect("read snapshot");
    let rows = oracle::authorized_rows(snapshot.connection()).expect("authorized facts");
    let view = build_view(&snapshot);
    for (query, _) in fixtures::DENSITY_QUERIES {
        let expected = oracle::proposed_name_path_oracle(&rows, query);
        let lookup =
            candidate_b::lookup(&view, &snapshot, query, MAX_WINDOW, None).expect("candidate B");
        assert_eq!(lookup.fence, Fence::Valid);
        assert_same_order(
            &lookup.name_path,
            window(&expected),
            &format!("B name/path {query}"),
        );
        let expected_extension = oracle::extension_oracle(&rows, query);
        assert_same_order(
            &lookup.extension,
            window(&expected_extension),
            &format!("B extension {query}"),
        );
    }
}

#[test]
fn issue494_extension_channel_is_identical_across_architectures() {
    let fixture = fixtures::density(100_000);
    let conn = fixture.connection();
    let snapshot = candidate_b::ReadSnapshot::begin(&conn).expect("read snapshot");
    let rows = oracle::authorized_rows(snapshot.connection()).expect("authorized facts");
    let view = build_view(&snapshot);
    for query in ["pdf", "jp", "tx", "docx", "zzz", "md"] {
        let expected = oracle::extension_oracle(&rows, query);
        let a_extension = candidate_a::extension_channel(&conn, query, MAX_WINDOW).expect("A ext");
        let b_lookup = candidate_b::lookup(&view, &snapshot, query, MAX_WINDOW, None).expect("B");
        assert_same_order(
            &a_extension,
            window(&expected),
            &format!("A extension {query}"),
        );
        assert_same_order(
            &b_lookup.extension,
            window(&expected),
            &format!("B extension {query}"),
        );
    }
}

#[test]
fn issue494_incremental_narrowing_equals_full_scan() {
    let fixture = fixtures::density(100_000);
    let conn = fixture.connection();
    let snapshot = candidate_b::ReadSnapshot::begin(&conn).expect("read snapshot");
    let rows = oracle::authorized_rows(snapshot.connection()).expect("authorized facts");
    let view = build_view(&snapshot);

    let typed = [
        "den",
        "dens",
        "densi",
        "densit",
        "density",
        "density-",
        "density-f",
        "density-fi",
        "density-fiv",
        "density-five",
    ];
    let mut cache: Option<candidate_b::NarrowCache> = None;
    for query in typed {
        let expected = oracle::proposed_name_path_oracle(&rows, query);
        let narrowed = candidate_b::lookup(&view, &snapshot, query, MAX_WINDOW, cache.as_ref())
            .expect("narrowed lookup");
        let full =
            candidate_b::lookup(&view, &snapshot, query, MAX_WINDOW, None).expect("full lookup");
        assert_same_order(
            &narrowed.name_path,
            window(&expected),
            &format!("narrowed name/path {query}"),
        );
        assert_same_order(
            &narrowed.name_path,
            &full.name_path,
            &format!("narrow vs full {query}"),
        );
        if query != "den" {
            assert!(
                narrowed.narrowed,
                "{query} should have used the narrowing cache"
            );
        }
        cache = candidate_b::NarrowCache::build(&view, query);
    }
}

#[test]
fn issue494_cross_connection_data_version_is_not_a_shared_generation() {
    let fixture = fixtures::edge_cases();
    let reader = fixture.connection();
    let writer = fixture.connection();
    let reader_data_version: i64 = reader
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .expect("reader data_version");
    let writer_data_version_before: i64 = writer
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .expect("writer data_version before write");
    let reader_max_rowid: i64 = reader
        .query_row(
            "SELECT COALESCE(MAX(rowid), 0) FROM global_entries",
            [],
            |row| row.get(0),
        )
        .expect("reader baseline max rowid");
    let writer_max_rowid_before: i64 = writer
        .query_row(
            "SELECT COALESCE(MAX(rowid), 0) FROM global_entries",
            [],
            |row| row.get(0),
        )
        .expect("writer baseline max rowid");

    let snapshot = candidate_b::ReadSnapshot::begin(&reader).expect("reader snapshot");
    let view = build_view(&snapshot);
    let target_id: String = writer
        .query_row(
            "SELECT ge.id FROM global_entries ge
             JOIN global_volumes gv ON gv.id = ge.volume_id
             WHERE ge.is_stale = 0 AND gv.enabled = 1
             ORDER BY ge.rowid LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("choose an existing fact");
    writer
        .execute(
            "UPDATE global_entries SET modified_at_fs = COALESCE(modified_at_fs, 0) + 1
             WHERE id = ?1",
            [&target_id],
        )
        .expect("commit an update through the other connection");

    // data_version is only comparable on the same connection. The writer's
    // own commit leaves its counter unchanged. The reader's active transaction
    // continues to report its snapshot value and advances only after the read
    // transaction ends. MAX(rowid) is unchanged because the writer updates an
    // existing row.
    let writer_data_version_after: i64 = writer
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .expect("writer data_version after own commit");
    let writer_max_rowid_after: i64 = writer
        .query_row(
            "SELECT COALESCE(MAX(rowid), 0) FROM global_entries",
            [],
            |row| row.get(0),
        )
        .expect("writer max rowid after update");
    let reader_data_version_in_snapshot: i64 = reader
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .expect("reader data_version while snapshot is active");
    let reader_max_rowid_after: i64 = snapshot
        .connection()
        .query_row(
            "SELECT COALESCE(MAX(rowid), 0) FROM global_entries",
            [],
            |row| row.get(0),
        )
        .expect("reader snapshot max rowid after external commit");
    assert_eq!(writer_data_version_before, writer_data_version_after);
    assert_eq!(writer_max_rowid_before, writer_max_rowid_after);
    assert_eq!(reader_data_version, reader_data_version_in_snapshot);
    assert_eq!(reader_max_rowid_after, reader_max_rowid);
    assert_ne!(
        writer
            .query_row(
                "SELECT modified_at_fs FROM global_entries WHERE id = ?1",
                [&target_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .expect("updated fact"),
        snapshot
            .connection()
            .query_row(
                "SELECT modified_at_fs FROM global_entries WHERE id = ?1",
                [&target_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .expect("snapshot fact"),
        "the current writer and the reader's stable snapshot expose different facts"
    );

    let other_snapshot = candidate_b::ReadSnapshot::begin(&writer).expect("writer snapshot");
    let lookup = candidate_b::lookup(&view, &other_snapshot, "report", MAX_WINDOW, None)
        .expect("cross-connection old view lookup");
    assert_eq!(lookup.fence, Fence::StaleSnapshot);
    assert!(lookup.name_path.is_empty());
    drop(other_snapshot);
    drop(snapshot);
    let reader_data_version_after_snapshot: i64 = reader
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .expect("reader data_version after snapshot ends");
    assert_ne!(reader_data_version, reader_data_version_after_snapshot);
}

#[test]
fn issue494_same_connection_write_invalidates_a_snapshot_view() {
    let fixture = fixtures::density(100_000);
    let conn = fixture.connection();
    let snapshot = candidate_b::ReadSnapshot::begin(&conn).expect("read snapshot");
    let view = build_view(&snapshot);
    let before_data_version: i64 = snapshot
        .connection()
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .expect("data_version before same-connection write");
    let before_max_rowid: i64 = snapshot
        .connection()
        .query_row(
            "SELECT COALESCE(MAX(rowid), 0) FROM global_entries",
            [],
            |row| row.get(0),
        )
        .expect("max rowid before same-connection write");
    assert!(
        snapshot
            .connection()
            .execute(
                "UPDATE global_entries SET last_seen_at = last_seen_at + 1
                 WHERE name = 'density-singleton.pdf'",
                [],
            )
            .is_err(),
        "the snapshot wrapper must reject writes through its reader connection"
    );
    drop(snapshot);

    // SQLite does not advance data_version for commits made by the connection
    // doing the comparison, and an UPDATE preserves MAX(rowid). The old tuple
    // therefore remains unchanged even though visible facts have changed.
    conn.execute(
        "UPDATE global_entries SET name = 'renamed-after-view.txt',
            name_normalized = 'renamed-after-view.txt',
            path = 'C:\\Density\\Single\\renamed-after-view.txt',
            path_normalized = 'c:\\density\\single\\renamed-after-view.txt',
            modified_at_fs = COALESCE(modified_at_fs, 0) + 1
         WHERE name = 'density-singleton.pdf'",
        [],
    )
    .expect("same-connection update after view build");
    let after_data_version: i64 = conn
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .expect("data_version after same-connection write");
    let after_max_rowid: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(rowid), 0) FROM global_entries",
            [],
            |row| row.get(0),
        )
        .expect("max rowid after same-connection write");
    assert_eq!(before_data_version, after_data_version);
    assert_eq!(before_max_rowid, after_max_rowid);

    let next_snapshot = candidate_b::ReadSnapshot::begin(&conn).expect("new read snapshot");
    let lookup = candidate_b::lookup(&view, &next_snapshot, "density-singleton", MAX_WINDOW, None)
        .expect("snapshot-fenced lookup");
    assert_eq!(
        lookup.fence,
        Fence::StaleSnapshot,
        "the old view must be rejected even when data_version and MAX(rowid) match"
    );
    assert!(lookup.name_path.is_empty());

    let rebuilt = build_view(&next_snapshot);
    let lookup = candidate_b::lookup(
        &rebuilt,
        &next_snapshot,
        "density-singleton",
        MAX_WINDOW,
        None,
    )
    .expect("rebuilt lookup");
    assert_eq!(lookup.fence, Fence::Valid);
    assert!(lookup.name_path.is_empty(), "the renamed row must be gone");
}

#[test]
fn issue494_cross_connection_mutations_are_snapshot_consistent_and_rebuild_completely() {
    let fixture = fixtures::density(100_000);
    let reader = fixture.connection();
    let snapshot = candidate_b::ReadSnapshot::begin(&reader).expect("reader snapshot");
    let old_rows = oracle::authorized_rows(snapshot.connection()).expect("old snapshot facts");
    let old_view = build_view(&snapshot);
    let query = "density-five-thousand";
    let old_expected = oracle::proposed_name_path_oracle(&old_rows, query);
    assert_eq!(old_expected.len(), 5_000);
    let old_top =
        candidate_b::lookup(&old_view, &snapshot, query, 80, None).expect("baseline top-k lookup");
    assert_eq!(old_top.fence, Fence::Valid);
    assert_same_order(
        &old_top.name_path,
        &old_expected[..80],
        "old snapshot baseline",
    );

    // A second connection commits deletes, a rename, an mtime reorder and new
    // best-ranked rows while the reader retains its original SQLite snapshot.
    let writer = fixture.db().conn().expect("second connection writer");
    writer
        .execute(
            "DELETE FROM global_entries
             WHERE name LIKE 'density-five-thousand-0%'
               AND name < 'density-five-thousand-01000'",
            [],
        )
        .expect("delete old top candidates");
    writer
        .execute(
            "UPDATE global_entries SET name = 'renamed-out-of-query.txt',
                name_normalized = 'renamed-out-of-query.txt',
                path = 'C:\\Density\\Five\\renamed-out-of-query.txt',
                path_normalized = 'c:\\density\\five\\renamed-out-of-query.txt'
             WHERE name = 'density-five-thousand-01001.md'",
            [],
        )
        .expect("rename a selected candidate out of the query");
    writer
        .execute(
            "UPDATE global_entries SET modified_at_fs = 2100000000
             WHERE name = 'density-five-thousand-01000.md'",
            [],
        )
        .expect("change an existing candidate's rank");
    {
        let mut insert = writer
            .prepare(
                "INSERT INTO global_entries(
                    id, volume_id, platform_file_id, parent_platform_file_id,
                    name, name_normalized, path, path_normalized, extension,
                    is_directory, size, created_at_fs, modified_at_fs,
                    file_attributes, is_hidden, is_system, source_provider,
                    last_seen_at, is_stale
                 ) VALUES (?1, 'gv_test', ?1, 'frn:parent', ?2, ?2, ?3, ?4, 'md',
                           0, 1024, 1600000000, ?5, 0, 0, 0,
                           'synthetic_issue494_density', 1800000000, 0)",
            )
            .expect("prepare external inserts");
        for index in 0..80 {
            let name = format!("density-five-thousand-{index:05}.md");
            let path = format!(r"C:\Density\Five\{name}");
            insert
                .execute(rusqlite::params![
                    format!("external-new-{index}"),
                    name,
                    path.clone(),
                    path.to_lowercase(),
                    2_000_000_000 + index
                ])
                .expect("insert new best-ranked hit");
        }
    }

    let still_old = candidate_b::lookup(&old_view, &snapshot, query, 80, None)
        .expect("lookup remains inside original snapshot");
    assert_eq!(still_old.fence, Fence::Valid);
    assert_eq!(still_old.hydrated_dropped, 0);
    assert_same_order(
        &still_old.name_path,
        &old_expected[..80],
        "old snapshot remains complete after external commit",
    );

    let other_reader = fixture.connection();
    let other_snapshot =
        candidate_b::ReadSnapshot::begin(&other_reader).expect("different-connection snapshot");
    let cross_connection = candidate_b::lookup(&old_view, &other_snapshot, query, 80, None)
        .expect("a view cannot cross connections");
    assert_eq!(cross_connection.fence, Fence::StaleSnapshot);
    assert!(cross_connection.name_path.is_empty());
    drop(other_snapshot);
    drop(snapshot);

    let next_snapshot = candidate_b::ReadSnapshot::begin(&reader).expect("latest reader snapshot");
    let stale = candidate_b::lookup(&old_view, &next_snapshot, query, 80, None)
        .expect("stale view is safely rejected");
    assert_eq!(stale.fence, Fence::StaleSnapshot);
    assert!(stale.name_path.is_empty());

    let new_rows = oracle::authorized_rows(next_snapshot.connection()).expect("latest facts");
    let new_expected = oracle::proposed_name_path_oracle(&new_rows, query);
    let new_view = build_view(&next_snapshot);
    let rebuilt = candidate_b::lookup(&new_view, &next_snapshot, query, 80, None)
        .expect("rebuilt top-k lookup");
    assert_eq!(rebuilt.fence, Fence::Valid);
    assert_eq!(rebuilt.hydrated_dropped, 0);
    assert_eq!(new_expected.len(), 4_079);
    assert_same_order(
        &rebuilt.name_path,
        &new_expected[..80],
        "rebuilt snapshot fills and orders the top-k after delete, rename, mtime update and insert",
    );
    assert!(
        rebuilt
            .name_path
            .iter()
            .any(|hit| hit.id == "external-new-79"),
        "the highest-ordered inserted result must not disappear after hydration"
    );
    assert_eq!(rebuilt.name_path[0].modified, Some(2_100_000_000));
    assert_eq!(rebuilt.name_path[1].id, "external-new-79");
}

#[test]
fn issue494_hydration_mismatch_falls_back_instead_of_returning_partial_or_misordered_top_k() {
    let fixture = fixtures::density(100_000);
    let conn = fixture.connection();
    let snapshot = candidate_b::ReadSnapshot::begin(&conn).expect("read snapshot");
    let initial_rows = oracle::authorized_rows(snapshot.connection()).expect("initial facts");
    let view = build_view(&snapshot);
    let query = "density-dozen";
    let initial = oracle::proposed_name_path_oracle(&initial_rows, query);
    assert_eq!(initial.len(), 40);
    let preselected = &initial[..2];

    // Deliberately bypass query_only to model a broken caller that mutates the
    // connection after building its view. Delete one selected row and reorder
    // the other: returning the surviving hydrated subset would have a hole and
    // could not prove which third row belongs in the page.
    snapshot
        .connection()
        .execute_batch("PRAGMA query_only = OFF")
        .expect("disable read-only guard for adversarial test");
    snapshot
        .connection()
        .execute(
            "DELETE FROM global_entries WHERE id = ?1",
            [&preselected[0].id],
        )
        .expect("delete a selected top-k row on the same connection");
    snapshot
        .connection()
        .execute(
            "UPDATE global_entries SET modified_at_fs = 2100000000 WHERE id = ?1",
            [&preselected[1].id],
        )
        .expect("reorder the other selected top-k row");

    let current_rows = oracle::authorized_rows(snapshot.connection()).expect("current facts");
    let authoritative = oracle::proposed_name_path_oracle(&current_rows, query);
    assert_eq!(authoritative.len(), 39);
    let lookup = candidate_b::lookup(&view, &snapshot, query, 2, None)
        .expect("mismatched hydration returns fallback");
    assert_eq!(lookup.fence, Fence::HydrationMismatch);
    assert!(lookup.name_path.is_empty());
    assert!(lookup.extension.is_empty());
    assert!(lookup.hydrated_dropped > 0);
    assert_ne!(ids(&authoritative[..2]), ids(preselected));
}

#[test]
fn issue494_narrow_cache_is_bound_to_view_identity_across_shrink_and_growth() {
    let fixture = fixtures::density(100_000);
    let conn = fixture.connection();

    let first_snapshot = candidate_b::ReadSnapshot::begin(&conn).expect("initial snapshot");
    let first_view = build_view(&first_snapshot);
    let broad_query = "density-five-thousand";
    let old_cache =
        candidate_b::NarrowCache::build(&first_view, broad_query).expect("cache original 5k view");
    assert_eq!(old_cache.len(), 5_000);
    drop(first_snapshot);

    // Shrink the active view to one row. The old cached slot indices include
    // values beyond the new view's name_span; identity mismatch must bypass
    // before indexing.
    conn.execute(
        "DELETE FROM global_entries WHERE name <> 'density-five-thousand-04999.md'",
        [],
    )
    .expect("shrink view source");
    let shrink_snapshot = candidate_b::ReadSnapshot::begin(&conn).expect("shrunken snapshot");
    let shrink_rows = oracle::authorized_rows(shrink_snapshot.connection()).expect("shrink facts");
    let shrink_view = build_view(&shrink_snapshot);
    let narrow_query = "density-five-thousand-0499";
    let shrink_expected = oracle::proposed_name_path_oracle(&shrink_rows, narrow_query);
    let after_shrink = candidate_b::lookup(
        &shrink_view,
        &shrink_snapshot,
        narrow_query,
        MAX_WINDOW,
        Some(&old_cache),
    )
    .expect("old cache safely bypassed after shrink");
    assert_eq!(after_shrink.fence, Fence::Valid);
    assert!(!after_shrink.narrowed);
    assert_same_order(&after_shrink.name_path, &shrink_expected, "shrunken view");
    let small_cache =
        candidate_b::NarrowCache::build(&shrink_view, broad_query).expect("cache small view");
    assert_eq!(small_cache.len(), 1);
    drop(shrink_snapshot);

    // Grow the next view. Reusing the one-slot cache would omit new matches;
    // the new view identity must force a complete scan.
    {
        let mut insert = conn
            .prepare(
                "INSERT INTO global_entries(
                    id, volume_id, platform_file_id, parent_platform_file_id,
                    name, name_normalized, path, path_normalized, extension,
                    is_directory, size, created_at_fs, modified_at_fs,
                    file_attributes, is_hidden, is_system, source_provider,
                    last_seen_at, is_stale
                 ) VALUES (?1, 'gv_test', ?1, 'frn:parent', ?2, ?2, ?3, ?4, 'md',
                           0, 1024, 1600000000, ?5, 0, 0, 0,
                           'synthetic_issue494_density', 1800000000, 0)",
            )
            .expect("prepare growth inserts");
        for index in 0..9 {
            let name = format!("density-five-thousand-0499{index}.md");
            let path = format!(r"C:\Density\Five\{name}");
            insert
                .execute(rusqlite::params![
                    format!("growth-{index}"),
                    name,
                    path.clone(),
                    path.to_lowercase(),
                    1_800_000_000 + index
                ])
                .expect("insert growth row");
        }
    }

    let growth_snapshot = candidate_b::ReadSnapshot::begin(&conn).expect("grown snapshot");
    let growth_rows = oracle::authorized_rows(growth_snapshot.connection()).expect("growth facts");
    let growth_view = build_view(&growth_snapshot);
    let growth_expected = oracle::proposed_name_path_oracle(&growth_rows, narrow_query);
    let after_growth = candidate_b::lookup(
        &growth_view,
        &growth_snapshot,
        narrow_query,
        MAX_WINDOW,
        Some(&small_cache),
    )
    .expect("old cache safely bypassed after growth");
    assert_eq!(after_growth.fence, Fence::Valid);
    assert!(!after_growth.narrowed);
    assert_eq!(growth_expected.len(), 10);
    assert_same_order(&after_growth.name_path, &growth_expected, "grown view");
}

#[test]
fn issue494_budget_degradation_falls_back_instead_of_overrunning() {
    let fixture = fixtures::density(100_000);
    let conn = fixture.connection();
    let snapshot = candidate_b::ReadSnapshot::begin(&conn).expect("read snapshot");
    let refused = CompactView::build(&snapshot, 1).expect("build attempt");
    assert!(
        refused.is_none(),
        "an impossible RAM budget must degrade instead of allocating"
    );
    // The caller contract: no view means the request falls back to SQLite and
    // reports the budget fence rather than a fast, incomplete answer.
    let fence = if refused.is_none() {
        Fence::BudgetExceeded
    } else {
        Fence::Valid
    };
    assert_eq!(fence, Fence::BudgetExceeded);
}

// ---------------------------------------------------------------------------
// FTS vs plain substring
// ---------------------------------------------------------------------------

#[test]
fn issue494_fts_match_set_matches_plain_substring_scan() {
    let fixture = fixtures::official(100_000);
    let conn = fixture.connection();
    let rows = oracle::authorized_rows(&conn).expect("authorized facts");
    let mut total_fts_only = 0u64;
    let mut total_rust_only = 0u64;
    for query in [
        "report",
        "invoice",
        "IMG_",
        "数据库",
        "final-v2",
        "Résumé",
        "résumé",
        "cafe\u{301}",
        "meeting-notes",
    ] {
        let divergence = oracle::match_set_divergence(&conn, &rows, query).expect("divergence");
        total_fts_only += divergence.fts_only;
        total_rust_only += divergence.rust_only;
        eprintln!(
            "{}",
            serde_json::json!({
                "experiment": "issue494_filename_ab",
                "record": "fts_vs_rust_divergence",
                "query": query,
                "fts_only": divergence.fts_only,
                "rust_only": divergence.rust_only,
                "both": divergence.both,
            })
        );
    }
    assert_eq!(
        total_rust_only, 0,
        "a plain substring scan must never find a match the production FTS tier misses"
    );
    assert_eq!(
        total_fts_only, 0,
        "the production FTS tier must not match rows a plain substring scan would miss"
    );
}

// ---------------------------------------------------------------------------
// Ranking differences are reported, never asserted equal
// ---------------------------------------------------------------------------

#[test]
fn issue494_ranking_diff_report_is_produced_not_asserted() {
    let fixture = fixtures::official(100_000);
    let conn = fixture.connection();
    let rows = oracle::authorized_rows(&conn).expect("authorized facts");
    for query in ["report", "invoice", "pdf", "数据库"] {
        let production = production_ids(&conn, query, PAGE_LIMIT, 0);
        let proposed: Vec<String> = ids(&oracle::proposed_name_path_oracle(&rows, query))
            .into_iter()
            .take(PAGE_LIMIT as usize)
            .collect();
        let production_set: std::collections::HashSet<&str> =
            production.iter().map(String::as_str).collect();
        let proposed_set: std::collections::HashSet<&str> =
            proposed.iter().map(String::as_str).collect();
        let overlap = production_set.intersection(&proposed_set).count();
        eprintln!(
            "{}",
            serde_json::json!({
                "experiment": "issue494_filename_ab",
                "record": "ranking_diff",
                "query": query,
                "production_page": production.len(),
                "proposed_page": proposed.len(),
                "overlap": overlap,
                "production_only": production.len() - overlap,
                "proposed_only": proposed.len() - overlap,
                "first_id_equal": production.first() == proposed.first(),
            })
        );
        assert!(
            overlap > 0 || production.is_empty() || proposed.is_empty(),
            "the two rankings must at least agree on some rows for {query}"
        );
    }
}

// ---------------------------------------------------------------------------
// Structural guarantees
// ---------------------------------------------------------------------------

#[test]
fn issue494_window_and_limit_contract_is_unchanged() {
    let fixture = fixtures::density(100_000);
    let conn = fixture.connection();
    let rows = oracle::authorized_rows(&conn).expect("authorized facts");
    // The oracle must return nothing beyond the production window.
    let proposed = oracle::proposed_name_path_oracle(&rows, "density-fifty-thousand");
    assert_eq!(proposed.len(), 50_000, "the oracle must not truncate");
    assert_eq!(window(&proposed).len(), MAX_WINDOW);
    // And production must return an empty page beyond it.
    assert!(production_ids(
        &conn,
        "density-fifty-thousand",
        PAGE_LIMIT,
        MAX_WINDOW as u32
    )
    .is_empty());
}

#[test]
fn issue494_duplicate_basenames_and_cross_tier_overlap_are_deduplicated() {
    let fixture = fixtures::edge_cases();
    let conn = fixture.connection();
    let rows = oracle::authorized_rows(&conn).expect("authorized facts");
    let hits = oracle::proposed_name_path_oracle(&rows, "duplicate.txt");
    assert_eq!(hits.len(), 2, "both duplicate basenames must be returned");
    assert_eq!(hits[0].class, Class::NameExact);
    assert_eq!(hits[1].class, Class::NameExact);
    assert_ne!(hits[0].id, hits[1].id);

    // "report" hits the name classes and the path of nothing else; the
    // production tiers overlap for "report.pdf" (exact name + FTS) and must
    // still return each row once.
    let production = production_ids(&conn, "report.pdf", PAGE_LIMIT, 0);
    let unique: std::collections::HashSet<&str> = production.iter().map(String::as_str).collect();
    assert_eq!(unique.len(), production.len());
}

#[test]
fn issue494_case_and_unicode_semantics_match_the_stored_facts() {
    let fixture = fixtures::edge_cases();
    let conn = fixture.connection();
    let rows = oracle::authorized_rows(&conn).expect("authorized facts");
    // Unicode case folding reaches the proposed classes.
    let folded = oracle::proposed_name_path_oracle(&rows, "RÉSUMÉ");
    assert_eq!(
        folded.len(),
        1,
        "unicode uppercase must fold for the proposal"
    );
    assert_eq!(folded[0].class, Class::NamePrefixOrWordBoundary);
    // Production reaches the same row only through its FTS tier, because its
    // exact/prefix tiers compare against SQLite's ASCII-only lower().
    let production = production_ids(&conn, "RÉSUMÉ", PAGE_LIMIT, 0);
    assert_eq!(production, ids(&folded));
    assert!(sqlite_lower("RÉSUMÉ") != fold("RÉSUMÉ"));
}

#[test]
fn issue494_short_cjk_query_is_not_silently_empty() {
    let fixture = fixtures::edge_cases();
    let conn = fixture.connection();
    let rows = oracle::authorized_rows(&conn).expect("authorized facts");
    let proposed = oracle::proposed_name_path_oracle(&rows, "数");
    assert_eq!(
        proposed.len(),
        1,
        "a 1-codepoint CJK query must still match"
    );
    let a = candidate_a::search(&conn, "数", MAX_WINDOW).expect("candidate A");
    assert_eq!(
        a.enumeration, "scan",
        "sub-3-codepoint queries cannot use trigram MATCH"
    );
    assert_same_order(&a.name_path, window(&proposed), "A short CJK");
}
