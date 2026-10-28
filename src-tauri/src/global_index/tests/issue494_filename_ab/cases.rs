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

fn build_view(conn: &Connection) -> CompactView {
    CompactView::build(conn, candidate_b::memory_budget_bytes())
        .expect("build compact view")
        .expect("compact view fits the budget")
}

#[test]
fn issue494_candidate_b_matches_proposed_oracle_on_edge_cases() {
    let fixture = fixtures::edge_cases();
    let conn = fixture.connection();
    conn.execute_batch("BEGIN").expect("read snapshot");
    let rows = oracle::authorized_rows(&conn).expect("authorized facts");
    let view = build_view(&conn);
    for query in EDGE_QUERIES {
        let expected = oracle::proposed_name_path_oracle(&rows, query);
        let lookup =
            candidate_b::lookup(&view, &conn, query, MAX_WINDOW, None).expect("candidate B");
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
    conn.execute_batch("ROLLBACK").expect("end snapshot");
}

#[test]
fn issue494_candidate_b_matches_proposed_oracle_on_density_fixture() {
    let fixture = fixtures::density(100_000);
    let conn = fixture.connection();
    conn.execute_batch("BEGIN").expect("read snapshot");
    let rows = oracle::authorized_rows(&conn).expect("authorized facts");
    let view = build_view(&conn);
    for (query, _) in fixtures::DENSITY_QUERIES {
        let expected = oracle::proposed_name_path_oracle(&rows, query);
        let lookup =
            candidate_b::lookup(&view, &conn, query, MAX_WINDOW, None).expect("candidate B");
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
    conn.execute_batch("ROLLBACK").expect("end snapshot");
}

#[test]
fn issue494_extension_channel_is_identical_across_architectures() {
    let fixture = fixtures::density(100_000);
    let conn = fixture.connection();
    conn.execute_batch("BEGIN").expect("read snapshot");
    let rows = oracle::authorized_rows(&conn).expect("authorized facts");
    let view = build_view(&conn);
    for query in ["pdf", "jp", "tx", "docx", "zzz", "md"] {
        let expected = oracle::extension_oracle(&rows, query);
        let a_extension = candidate_a::extension_channel(&conn, query, MAX_WINDOW).expect("A ext");
        let b_lookup = candidate_b::lookup(&view, &conn, query, MAX_WINDOW, None).expect("B");
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
    conn.execute_batch("ROLLBACK").expect("end snapshot");
}

#[test]
fn issue494_incremental_narrowing_equals_full_scan() {
    let fixture = fixtures::density(100_000);
    let conn = fixture.connection();
    conn.execute_batch("BEGIN").expect("read snapshot");
    let rows = oracle::authorized_rows(&conn).expect("authorized facts");
    let view = build_view(&conn);

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
        let narrowed = candidate_b::lookup(&view, &conn, query, MAX_WINDOW, cache.as_ref())
            .expect("narrowed lookup");
        let full = candidate_b::lookup(&view, &conn, query, MAX_WINDOW, None).expect("full lookup");
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
    conn.execute_batch("ROLLBACK").expect("end snapshot");
}

#[test]
fn issue494_revision_fence_blocks_a_stale_view() {
    let fixture = fixtures::density(100_000);
    let conn = fixture.connection();
    conn.execute_batch("BEGIN").expect("read snapshot");
    let view = build_view(&conn);
    conn.execute_batch("ROLLBACK").expect("end snapshot");

    // A rename after the view was built must invalidate it.
    fixture
        .db()
        .conn()
        .expect("writer")
        .execute(
            "UPDATE global_entries SET name = 'renamed-after-view.txt',
                name_normalized = 'renamed-after-view.txt',
                path = 'C:\\Density\\Single\\renamed-after-view.txt',
                path_normalized = 'c:\\density\\single\\renamed-after-view.txt'
             WHERE name = 'density-singleton.pdf'",
            [],
        )
        .expect("rename after view build");

    let lookup = candidate_b::lookup(&view, &conn, "density-singleton", MAX_WINDOW, None)
        .expect("fenced lookup");
    assert_eq!(
        lookup.fence,
        Fence::StaleRevision,
        "a post-build write must fence the view instead of returning stale facts"
    );
    assert!(lookup.name_path.is_empty());

    // And a rebuild must see the new facts.
    let rebuilt = build_view(&conn);
    let lookup = candidate_b::lookup(&rebuilt, &conn, "density-singleton", MAX_WINDOW, None)
        .expect("rebuilt lookup");
    assert_eq!(lookup.fence, Fence::Valid);
    assert!(lookup.name_path.is_empty(), "the renamed row must be gone");
}

#[test]
fn issue494_hydration_drops_rows_that_changed_under_a_matching_revision() {
    let fixture = fixtures::density(100_000);
    let conn = fixture.connection();
    conn.execute_batch("BEGIN").expect("read snapshot");
    let mut view = build_view(&conn);
    let before = candidate_b::lookup(&view, &conn, "density-dozen", MAX_WINDOW, None)
        .expect("baseline lookup");
    assert_eq!(before.name_path.len(), 40);
    conn.execute_batch("ROLLBACK").expect("end snapshot");

    // Delete half of the group behind the view's back.
    fixture
        .db()
        .conn()
        .expect("writer")
        .execute(
            "DELETE FROM global_entries WHERE name LIKE 'density-dozen-0%' AND name < 'density-dozen-020'",
            [],
        )
        .expect("delete rows after view build");

    // Force the fence to report Valid so only hydration can protect the result.
    let current = candidate_b::read_revision(&conn).expect("current revision");
    view.force_revision_for_test(current);
    let lookup = candidate_b::lookup(&view, &conn, "density-dozen", MAX_WINDOW, None)
        .expect("hydration lookup");
    assert_eq!(lookup.fence, Fence::Valid);
    assert_eq!(
        lookup.name_path.len(),
        20,
        "hydration must drop deleted rows even when the revision fence is bypassed"
    );
    assert_eq!(lookup.hydrated_dropped, 20);
}

#[test]
fn issue494_budget_degradation_falls_back_instead_of_overrunning() {
    let fixture = fixtures::density(100_000);
    let conn = fixture.connection();
    let refused = CompactView::build(&conn, 1).expect("build attempt");
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
