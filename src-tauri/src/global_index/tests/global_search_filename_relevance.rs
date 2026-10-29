//! Default-off Issue #474 filename relevance candidate and paired 100k/200k/1m evidence.
use crate::db::Database;
use crate::global_index::models::{AddManagedScopeRequest, GlobalEntryInput, GlobalSearchResult};
use crate::global_index::repository::GlobalSearchSnapshot;
use crate::global_index::search::{
    diagnostic_search_filename_relevance, diagnostic_search_filename_relevance_sql,
    diagnostic_search_fts, diagnostic_search_fts_sql, search_global_entries_on_connection,
    with_filename_relevance_candidate,
};
use rusqlite::{ffi, params, Connection, StatementStatus};
use serde_json::{json, Value as JsonValue};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

fn sql_result(row: &rusqlite::Row<'_>) -> rusqlite::Result<GlobalSearchResult> {
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

fn ascii_word(character: Option<char>) -> bool {
    character.is_some_and(|value| value.is_ascii_alphanumeric() || value == '_')
}

fn has_whole_query_boundary(name: &str, query: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let query = query.to_ascii_lowercase();

    // Visit every Unicode scalar boundary in the name. The supported query is
    // ASCII, so an exact match always ends at a valid UTF-8 boundary. This is
    // intentionally independent from SQL's sentinel-and-GLOB predicate.
    name.char_indices().any(|(start, _)| {
        if !name[start..].starts_with(&query) {
            return false;
        }
        let end = start + query.len();
        let before = name[..start].chars().next_back();
        let after = name[end..].chars().next();
        !ascii_word(before) && !ascii_word(after)
    })
}

fn relevance_class(result: &GlobalSearchResult, query: &str) -> u8 {
    let query = query.to_ascii_lowercase();
    let name = result.name.to_ascii_lowercase();
    if has_whole_query_boundary(&name, &query) {
        return 0;
    }
    if name.contains(&query) || result.extension.to_ascii_lowercase().contains(&query) {
        1
    } else {
        2
    }
}

/// Independent Rust oracle over the full eligible FTS result set. It does not
/// call the candidate SQL or use BM25 to derive the proposed ordering.
fn filename_relevance_oracle(conn: &Connection, query: &str) -> Vec<GlobalSearchResult> {
    let sql = r#"
        SELECT ge.id, ge.volume_id, ge.platform_file_id, ge.name, ge.path,
               ge.extension, ge.is_directory, ge.size, ge.created_at_fs,
               ge.modified_at_fs, ge.file_attributes, ge.is_hidden, ge.is_system,
               ge.source_provider,
               EXISTS (
                   SELECT 1 FROM managed_entries me
                   JOIN managed_scopes ms ON ms.id = me.managed_scope_id
                   WHERE me.global_entry_id = ge.id AND me.enabled = 1 AND ms.enabled = 1
               ) AS managed,
               0.0 AS rank
        FROM global_entries_fts
        CROSS JOIN global_entries ge
        CROSS JOIN global_volumes gv
        WHERE global_entries_fts MATCH ?1
          AND ge.rowid = global_entries_fts.rowid
          AND gv.id = ge.volume_id
          AND gv.enabled = 1
          AND gv.coverage_pending = 0
          AND ge.is_stale = 0
    "#;
    let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
    let mut statement = conn.prepare(sql).expect("prepare complete FTS oracle");
    let mut results = statement
        .query_map([fts_query], sql_result)
        .expect("execute complete FTS oracle")
        .collect::<Result<Vec<_>, _>>()
        .expect("read complete FTS oracle");
    for result in &mut results {
        result.rank = f64::from(relevance_class(result, query));
    }
    results.sort_by(|left, right| {
        left.rank
            .total_cmp(&right.rank)
            .then_with(|| right.modified_at_fs.cmp(&left.modified_at_fs))
            .then_with(|| left.id.cmp(&right.id))
    });
    results
}

fn schema_signature(conn: &Connection) -> Vec<(String, String, String)> {
    conn.prepare("SELECT type, name, coalesce(sql, '') FROM main.sqlite_schema ORDER BY type, name")
        .expect("prepare schema signature")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("read schema signature")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect schema signature")
}

fn add_entry(
    entries: &mut Vec<GlobalEntryInput>,
    path: impl Into<String>,
    name: impl Into<String>,
    extension: impl Into<String>,
    is_directory: bool,
    modified_at_fs: Option<i64>,
) {
    let path = path.into();
    let name = name.into();
    let mut entry = super::test_entry(&path, &name, is_directory);
    entry.extension = extension.into();
    entry.modified_at_fs = modified_at_fs;
    entries.push(entry);
}

fn golden_entries() -> Vec<GlobalEntryInput> {
    let mut entries = Vec::new();
    for index in 0..24_i64 {
        let (path, name, directory) = match index {
            0 => (
                r"C:\Global\Documents\Annual\quarterly-report-0".to_string(),
                "Quarterly Report 0.pdf".to_string(),
                false,
            ),
            1 => (
                r"C:\Global\Documents\Annual\quarterly-report-1".to_string(),
                "Quarterly Report 1.pdf".to_string(),
                false,
            ),
            2 => (
                r"C:\Global\Documents\Annual\unicode-report".to_string(),
                "中文report中文.pdf".to_string(),
                false,
            ),
            3 => (
                r"C:\Global\Documents\Annual\report-directory".to_string(),
                "Team Report Archive".to_string(),
                true,
            ),
            4 => (
                r"C:\Global\Documents\Annual\punctuation-report".to_string(),
                "Dossier-report-final.pdf".to_string(),
                false,
            ),
            5 => (
                r"C:\Global\Documents\Annual\underscore-report".to_string(),
                "pre_report_suffix.pdf".to_string(),
                false,
            ),
            _ => (
                format!(r"C:\Global\Documents\Annual\boundary-report-{index}"),
                format!("Quarterly Report {index}.pdf"),
                false,
            ),
        };
        add_entry(
            &mut entries,
            path,
            name,
            if directory { "" } else { "pdf" },
            directory,
            if index == 1 {
                None
            } else {
                Some(100 + index % 4)
            },
        );
    }

    for (path, name, modified_at_fs) in [
        (
            r"C:\Global\Adversarial\prereport-report.pdf",
            "prereport-report.pdf",
            Some(1_001),
        ),
        (
            r"C:\Global\Adversarial\xreporty-report.txt",
            "xreporty-report.txt",
            Some(1_001),
        ),
        (
            r"C:\Global\Adversarial\prereport.txt",
            "prereport.txt",
            Some(1_002),
        ),
        (
            r"/Users/owner/Adversarial/reporting-guide.pdf",
            "reporting-guide.pdf",
            Some(1_003),
        ),
        (
            r"C:\Global\Adversarial\annual-report.pdf",
            "annual-report.pdf",
            Some(1_004),
        ),
        (
            r"C:\Global\Adversarial\annual-report-repeated.pdf",
            "annual reportx annual report.pdf",
            Some(1_005),
        ),
        (
            r"C:\Global\Adversarial\report-archive-repeated.pdf",
            "prereport archivex report archive.pdf",
            Some(1_006),
        ),
        (
            r"C:\Global\Adversarial\unicode-after-report.pdf",
            "reporté notes.pdf",
            Some(1_007),
        ),
        ("", "Report from empty path.pdf", Some(1_008)),
    ] {
        add_entry(&mut entries, path, name, "pdf", false, modified_at_fs);
    }
    let mut hidden = super::test_entry(
        r"C:\Global\Adversarial\hidden-report.pdf",
        "hidden report record.pdf",
        false,
    );
    hidden.is_hidden = true;
    hidden.modified_at_fs = Some(1_009);
    entries.push(hidden);
    let mut system = super::test_entry(
        r"C:\Global\Adversarial\system-report.pdf",
        "system report record.pdf",
        false,
    );
    system.is_system = true;
    system.modified_at_fs = Some(1_010);
    entries.push(system);

    for index in 0..24_i64 {
        let name = match index % 3 {
            0 => format!("Annual reporting guide {index}.pdf"),
            1 => format!("Subreport archive {index}.pdf"),
            _ => format!("Supportreport scan {index}.pdf"),
        };
        add_entry(
            &mut entries,
            format!(r"C:\Global\Library\Files\substring-report-{index}"),
            name,
            "pdf",
            false,
            Some(200 + index),
        );
    }

    for index in 0..2_i64 {
        add_entry(
            &mut entries,
            format!(r"C:\Global\Duplicates\copy-{index}\Quarterly Report duplicate.pdf"),
            "Quarterly Report duplicate.pdf",
            "pdf",
            false,
            Some(500),
        );
    }

    for index in 0..22_i64 {
        let path = if index == 21 {
            format!("/Users/owner/Archive/Reports/path-only-{index}.pdf")
        } else {
            format!(r"C:\Global\Archive\Reports\path-only-{index}.pdf")
        };
        add_entry(
            &mut entries,
            path,
            format!("summary file {index}.pdf"),
            "pdf",
            false,
            Some(10_000 + index),
        );
    }

    for index in 0..22_i64 {
        add_entry(
            &mut entries,
            format!(r"C:\Global\Contracts\Legal\boundary-{index}"),
            format!("Supply contract final {index}.docx"),
            "docx",
            false,
            Some(300 + index % 3),
        );
    }
    for index in 0..22_i64 {
        let name = if index % 2 == 0 {
            format!("Subcontractor invoice {index}.pdf")
        } else {
            format!("Pre-contracting memo {index}.pdf")
        };
        add_entry(
            &mut entries,
            format!(r"C:\Global\Contracts\Inbound\substring-{index}"),
            name,
            "pdf",
            false,
            Some(500 + index),
        );
    }
    for index in 0..22_i64 {
        add_entry(
            &mut entries,
            format!(r"C:\Global\Vendor\Contracts\path-only-{index}.pdf"),
            format!("scan file {index}.pdf"),
            "pdf",
            false,
            Some(20_000 + index),
        );
    }

    for index in 0..64_i64 {
        add_entry(
            &mut entries,
            format!(r"C:\Global\Vendor\Documents\extension-only-{index}.file"),
            format!("manual-file-{index}.file"),
            "document",
            false,
            Some(700 + index),
        );
    }

    add_entry(
        &mut entries,
        r"C:\Global\Archive\Chinese\database-record",
        "档案-数据库-历史.pdf",
        "pdf",
        false,
        Some(900),
    );
    add_entry(
        &mut entries,
        r"C:\Global\Archive\Unicode\resume-record",
        "Café résumé notes.pdf",
        "pdf",
        false,
        Some(901),
    );
    add_entry(
        &mut entries,
        r"C:\Global\Archive\Punctuation\annual-report-file",
        "Quarterly annual.report archive.pdf",
        "pdf",
        false,
        Some(902),
    );
    add_entry(
        &mut entries,
        r"C:\Global\Archive\Sparse\needle-record",
        "Monocle needle notes.pdf",
        "pdf",
        false,
        Some(903),
    );
    add_entry(
        &mut entries,
        r"C:\Global\Archive\Fast\prefix-report",
        "Quarterly Report Prefix 1.pdf",
        "pdf",
        false,
        Some(904),
    );
    add_entry(
        &mut entries,
        r"C:\Global\Archive\Fast\extension-exact",
        "extension exact.pdf",
        "pdf",
        false,
        Some(905),
    );
    add_entry(
        &mut entries,
        r"C:\Global\Archive\Fast\extension-prefix",
        "extension prefix.docx",
        "docx",
        false,
        Some(906),
    );
    entries
}

fn comparison_positions(
    query: &str,
    examples: &[(&str, &str, &str)],
    old: &[GlobalSearchResult],
    proposed: &[GlobalSearchResult],
) -> Vec<JsonValue> {
    examples
        .iter()
        .map(|(name, why_new, limitation)| {
            let old_position = old.iter().position(|result| result.name == *name);
            let proposed_position = proposed.iter().position(|result| result.name == *name);
            json!({
                "query": query,
                "name": name,
                "old_bm25_position_1_based": old_position.map(|position| position + 1),
                "proposed_category": proposed_position.map(|position| proposed[position].rank),
                "proposed_position_1_based": proposed_position.map(|position| position + 1),
                "why_new_order_matches_intent": why_new,
                "limitation": limitation,
            })
        })
        .collect()
}

#[test]
fn filename_relevance_contract_oracle_and_fallbacks() {
    let path = super::test_db_path();
    let _cleanup = super::BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open isolated relevance contract DB");
    db.upsert_global_volume(&super::test_volume())
        .expect("create enabled test source");
    let mut disabled_volume = super::test_volume();
    disabled_volume.id = "gv_disabled".to_string();
    disabled_volume.stable_volume_id = "disabled-volume".to_string();
    disabled_volume.enabled = false;
    db.upsert_global_volume(&disabled_volume)
        .expect("create disabled test source");
    let mut coverage_pending_volume = super::test_volume();
    coverage_pending_volume.id = "gv_coverage_pending".to_string();
    coverage_pending_volume.stable_volume_id = "coverage-pending-volume".to_string();
    db.upsert_global_volume(&coverage_pending_volume)
        .expect("create coverage-pending test source");

    let mut entries = golden_entries();
    let mut disabled = super::test_entry(
        r"C:\Global\Disabled\Report\latest-disabled.pdf",
        "Latest Report Disabled.pdf",
        false,
    );
    disabled.volume_id = disabled_volume.id.clone();
    disabled.modified_at_fs = Some(i64::MAX);
    entries.push(disabled);
    let mut coverage_pending = super::test_entry(
        r"C:\Global\CoveragePending\Report\latest-pending.pdf",
        "Latest Report Coverage Pending.pdf",
        false,
    );
    coverage_pending.volume_id = coverage_pending_volume.id.clone();
    coverage_pending.modified_at_fs = Some(i64::MAX - 1);
    entries.push(coverage_pending);
    let mut stale = super::test_entry(
        r"C:\Global\Stale\Report\latest-stale.pdf",
        "Latest Report Stale.pdf",
        false,
    );
    stale.modified_at_fs = Some(i64::MAX - 1);
    entries.push(stale);
    db.upsert_global_entries_batch(&entries)
        .expect("insert golden relevance fixture through FTS triggers");

    let conn = super::configured_connection(&path).expect("open configured relevance reader");
    conn.execute(
        "UPDATE global_volumes SET coverage_pending = 1 WHERE id = ?1",
        [&coverage_pending_volume.id],
    )
    .expect("mark one enabled source coverage-pending before ranking");
    let stale_id: String = conn
        .query_row(
            "SELECT id FROM global_entries WHERE name = 'Latest Report Stale.pdf'",
            [],
            |row| row.get(0),
        )
        .expect("read stale row id");
    conn.execute(
        "UPDATE global_entries SET is_stale = 1 WHERE id = ?1",
        [&stale_id],
    )
    .expect("mark one row stale before ranking");
    let managed_id: String = conn
        .query_row(
            "SELECT id FROM global_entries WHERE name = 'Quarterly Report 0.pdf'",
            [],
            |row| row.get(0),
        )
        .expect("read managed row id");
    db.add_managed_scope(AddManagedScopeRequest {
        path: r"C:\Global\Documents\Annual\quarterly-report-0".to_string(),
        global_entry_id: Some(managed_id),
        enabled: true,
        allow_local_ai: true,
        allow_cloud_ai: false,
    })
    .expect("make one golden result managed");

    let before_schema = schema_signature(&conn);
    let report_oracle = filename_relevance_oracle(&conn, "report");
    assert_eq!(
        report_oracle.len(),
        85,
        "only eligible report FTS hits count"
    );
    let report_boundary = report_oracle
        .iter()
        .filter(|result| result.rank == 0.0)
        .count();
    let report_substring = report_oracle
        .iter()
        .filter(|result| result.rank == 1.0)
        .count();
    let report_path = report_oracle
        .iter()
        .filter(|result| result.rank == 2.0)
        .count();
    assert!(report_boundary >= 24);
    assert!(report_substring >= 24);
    assert_eq!(report_path, 22);

    let old_report = with_filename_relevance_candidate(false, || {
        search_global_entries_on_connection(&conn, "report", 4_096, 0)
    })
    .expect("old report order");
    let new_report = with_filename_relevance_candidate(true, || {
        search_global_entries_on_connection(&conn, "report", 4_096, 0)
    })
    .expect("new report order");
    assert_eq!(
        new_report, report_oracle,
        "full candidate result window matches Rust oracle"
    );
    assert!(
        old_report.iter().any(|result| result.rank < 0.0),
        "old path retains BM25 scores"
    );
    assert!(new_report
        .iter()
        .all(|result| result.rank >= 0.0 && result.rank <= 2.0));
    assert!(new_report[0].name.contains("Report"));
    assert!(
        new_report[0].modified_at_fs.unwrap() < 10_000,
        "filename relevance outranks newer path-only hits"
    );
    assert!(new_report.iter().any(|result| result.managed));
    assert!(new_report.iter().any(|result| {
        result.name == "Quarterly Report 0.pdf"
            && result.path == r"C:\Global\Documents\Annual\quarterly-report-0"
    }));
    assert!(new_report
        .iter()
        .any(|result| result.name == "hidden report record.pdf" && result.is_hidden));
    assert!(new_report
        .iter()
        .any(|result| result.name == "system report record.pdf" && result.is_system));
    let duplicate_name_rows = new_report
        .iter()
        .filter(|result| result.name == "Quarterly Report duplicate.pdf")
        .collect::<Vec<_>>();
    assert_eq!(
        duplicate_name_rows.len(),
        2,
        "same basename in separate paths remains distinct"
    );
    assert!(
        duplicate_name_rows[0].id < duplicate_name_rows[1].id,
        "equal-time duplicate basenames use stable id order"
    );
    assert!(!new_report
        .iter()
        .any(|result| result.name.contains("Disabled")));
    assert!(!new_report
        .iter()
        .any(|result| result.name.contains("Stale")));
    assert!(!new_report
        .iter()
        .any(|result| result.name.contains("Coverage Pending")));
    let null_time = new_report
        .iter()
        .position(|result| result.name == "Quarterly Report 1.pdf")
        .expect("retain row with NULL modification time");
    assert_eq!(new_report[null_time].modified_at_fs, None);
    assert!(new_report[null_time - 1].modified_at_fs.is_some());
    assert_eq!(new_report[null_time - 1].rank, 0.0);
    assert_eq!(new_report[null_time + 1].rank, 1.0);

    for (name, expected_rank) in [
        ("prereport-report.pdf", 0.0),
        ("xreporty-report.txt", 0.0),
        ("prereport.txt", 1.0),
        ("reporting-guide.pdf", 1.0),
        ("annual-report.pdf", 0.0),
        ("annual reportx annual report.pdf", 0.0),
        ("prereport archivex report archive.pdf", 0.0),
        ("reporté notes.pdf", 0.0),
        ("Report from empty path.pdf", 0.0),
        ("pre_report_suffix.pdf", 1.0),
    ] {
        let row = new_report
            .iter()
            .find(|result| result.name == name)
            .unwrap_or_else(|| panic!("expected adversarial filename {name:?}"));
        assert_eq!(row.rank, expected_rank, "relevance category for {name:?}");
    }
    assert_eq!(
        with_filename_relevance_candidate(true, || {
            search_global_entries_on_connection(&conn, "report", 10, 64)
        })
        .unwrap(),
        report_oracle[64..].to_vec(),
        "page slices the complete new order"
    );
    assert!(with_filename_relevance_candidate(true, || {
        search_global_entries_on_connection(&conn, "report", 80, 4_096)
    })
    .unwrap()
    .is_empty());

    let repeated_phrase_oracle = filename_relevance_oracle(&conn, "report archive");
    assert_eq!(repeated_phrase_oracle.len(), 1);
    assert_eq!(repeated_phrase_oracle[0].rank, 0.0);
    assert_eq!(
        with_filename_relevance_candidate(true, || {
            search_global_entries_on_connection(&conn, "report archive", 80, 0)
        })
        .unwrap(),
        repeated_phrase_oracle,
        "a later whole-phrase occurrence wins when an earlier occurrence is unbounded"
    );
    let repeated_word_phrase_oracle = filename_relevance_oracle(&conn, "annual report");
    let repeated_word_phrase = repeated_word_phrase_oracle
        .iter()
        .find(|result| result.name == "annual reportx annual report.pdf")
        .expect("repeated full phrase remains in FTS candidates");
    assert_eq!(repeated_word_phrase.rank, 0.0);
    assert_eq!(
        with_filename_relevance_candidate(true, || {
            search_global_entries_on_connection(&conn, "annual report", 80, 0)
        })
        .unwrap(),
        repeated_word_phrase_oracle,
        "phrases use the same all-occurrence boundary rule as single words"
    );

    let contract_oracle = filename_relevance_oracle(&conn, "contract");
    assert_eq!(contract_oracle.len(), 66);
    let contract_candidate = with_filename_relevance_candidate(true, || {
        search_global_entries_on_connection(&conn, "contract", 4_096, 0)
    })
    .unwrap();
    assert_eq!(contract_candidate, contract_oracle);

    let extension_oracle = filename_relevance_oracle(&conn, "umen");
    assert_eq!(extension_oracle.len(), 88);
    assert_eq!(
        extension_oracle
            .iter()
            .filter(|result| result.rank == 1.0)
            .count(),
        64,
        "extension-only substring hits use class 1"
    );
    assert_eq!(
        with_filename_relevance_candidate(true, || {
            search_global_entries_on_connection(&conn, "umen", 4_096, 0)
        })
        .unwrap(),
        extension_oracle,
        "extension-only substring uses filename-substring relevance class"
    );

    let doc_prefix_old = with_filename_relevance_candidate(false, || {
        search_global_entries_on_connection(&conn, "doc", 80, 0)
    })
    .unwrap();
    let doc_prefix_new = with_filename_relevance_candidate(true, || {
        search_global_entries_on_connection(&conn, "doc", 80, 0)
    })
    .unwrap();
    assert_eq!(
        doc_prefix_new, doc_prefix_old,
        "extension prefix tier remains first"
    );
    let exact_extension_old = with_filename_relevance_candidate(false, || {
        search_global_entries_on_connection(&conn, "pdf", 80, 0)
    })
    .unwrap();
    let exact_extension_new = with_filename_relevance_candidate(true, || {
        search_global_entries_on_connection(&conn, "pdf", 80, 0)
    })
    .unwrap();
    assert_eq!(
        exact_extension_new, exact_extension_old,
        "exact extension tier remains first"
    );
    let exact_name_old = with_filename_relevance_candidate(false, || {
        search_global_entries_on_connection(&conn, "Quarterly Report 0.pdf", 80, 0)
    })
    .unwrap();
    let exact_name_new = with_filename_relevance_candidate(true, || {
        search_global_entries_on_connection(&conn, "Quarterly Report 0.pdf", 80, 0)
    })
    .unwrap();
    assert_eq!(
        exact_name_new, exact_name_old,
        "exact filename tier remains first"
    );
    let prefix_old = with_filename_relevance_candidate(false, || {
        search_global_entries_on_connection(&conn, "Quarterly", 80, 0)
    })
    .unwrap();
    let prefix_new = with_filename_relevance_candidate(true, || {
        search_global_entries_on_connection(&conn, "Quarterly", 80, 0)
    })
    .unwrap();
    assert_eq!(prefix_new, prefix_old, "filename prefix tier remains first");

    for query in [
        "数据库",
        "café",
        "annual.report",
        "annual  report",
        "re",
        "",
    ] {
        let baseline = with_filename_relevance_candidate(false, || {
            search_global_entries_on_connection(&conn, query, 80, 0)
        })
        .unwrap();
        let candidate = with_filename_relevance_candidate(true, || {
            search_global_entries_on_connection(&conn, query, 80, 0)
        })
        .unwrap();
        assert_eq!(
            candidate, baseline,
            "unsupported query fallback must preserve full old result for {query:?}"
        );
        assert!(diagnostic_search_filename_relevance(&conn, query, 80)
            .unwrap()
            .is_none());
    }

    let sparse_oracle = filename_relevance_oracle(&conn, "needle");
    assert_eq!(sparse_oracle.len(), 1);
    assert_eq!(sparse_oracle[0].rank, 0.0);
    assert_eq!(
        with_filename_relevance_candidate(true, || {
            search_global_entries_on_connection(&conn, "needle", 80, 0)
        })
        .unwrap(),
        sparse_oracle,
        "supported one-hit queries follow the new relevance rule"
    );
    assert_eq!(
        diagnostic_search_filename_relevance(&conn, "needle", 80)
            .unwrap()
            .expect("sparse supported query is not a BM25 fallback"),
        sparse_oracle
    );
    assert!(with_filename_relevance_candidate(true, || {
        search_global_entries_on_connection(&conn, "zzznomatch", 80, 0)
    })
    .unwrap()
    .is_empty());
    assert!(
        diagnostic_search_filename_relevance(&conn, "zzznomatch", 80)
            .unwrap()
            .expect("zero-hit supported query follows relevance route")
            .is_empty()
    );

    // Existing FTS triggers remain the only write path. Prove insert, rename,
    // path move and delete are visible without rebuilding or refreshing a cache.
    let mut incremental = super::test_entry(
        r"C:\Global\Incremental\first.pdf",
        "first document.pdf",
        false,
    );
    incremental.modified_at_fs = Some(30_000);
    db.upsert_global_entries_batch(std::slice::from_ref(&incremental))
        .unwrap();
    let mut renamed = incremental.clone();
    renamed.name = "Newest Report candidate.pdf".to_string();
    renamed.path = r"C:\Global\Moved\Newest Report candidate.pdf".to_string();
    renamed.modified_at_fs = Some(30_001);
    db.upsert_global_entries_batch(std::slice::from_ref(&renamed))
        .unwrap();
    let after_rename = filename_relevance_oracle(&conn, "report");
    let candidate_after_rename = with_filename_relevance_candidate(true, || {
        search_global_entries_on_connection(&conn, "report", 4_096, 0)
    })
    .unwrap();
    assert_eq!(candidate_after_rename, after_rename);
    let renamed_id = candidate_after_rename
        .iter()
        .find(|result| result.name == "Newest Report candidate.pdf")
        .expect("renamed FTS fact is visible immediately")
        .id
        .clone();
    conn.execute("DELETE FROM global_entries WHERE id = ?1", [&renamed_id])
        .expect("delete renamed FTS row using production trigger");
    assert!(!filename_relevance_oracle(&conn, "report")
        .iter()
        .any(|result| result.id == renamed_id));
    assert_eq!(
        schema_signature(&conn),
        before_schema,
        "candidate adds no durable schema objects"
    );

    let old_contract = with_filename_relevance_candidate(false, || {
        search_global_entries_on_connection(&conn, "report", 4_096, 0)
    })
    .unwrap();
    let new_contract = with_filename_relevance_candidate(true, || {
        search_global_entries_on_connection(&conn, "report", 4_096, 0)
    })
    .unwrap();
    let comparisons = comparison_positions(
        "report",
        &[
            (
                "Quarterly Report 0.pdf",
                "The full filename phrase is visible, so it ranks with other whole-word basename matches.",
                "Equal relevance still uses file modification time and stable ID, not query term frequency.",
            ),
            (
                "中文report中文.pdf",
                "CJK neighbors are delimiters under the documented ASCII word-boundary rule.",
                "This deliberately does not apply language-specific segmentation for CJK text.",
            ),
            (
                "Dossier-report-final.pdf",
                "Hyphens separate the searched word from surrounding filename terms.",
                "The rule treats all non-ASCII-word punctuation as a delimiter.",
            ),
            (
                "prereport-report.pdf",
                "A later bounded occurrence outranks the earlier embedded substring.",
                "Only the best relevance class is kept; occurrences are not individually scored.",
            ),
            (
                "xreporty-report.txt",
                "A later complete word match is found even though the first occurrence is embedded.",
                "The Unicode boundary rule is intentionally ASCII-defined for supported queries.",
            ),
            (
                "Annual reporting guide 0.pdf",
                "The query continues into a word, so this remains an ordinary substring.",
                "The system does not use stemming or linguistic word forms.",
            ),
            (
                "Subreport archive 1.pdf",
                "A query embedded at the start of a larger word ranks below a bounded match.",
                "Substring matches remain searchable and are not discarded.",
            ),
            (
                "pre_report_suffix.pdf",
                "Underscore is defined as a word character, so this is not a whole-word match.",
                "Projects that use underscores as separators may prefer another boundary contract.",
            ),
            (
                "reporté notes.pdf",
                "A non-ASCII accented neighbor is a delimiter under the stated ASCII rule.",
                "Accent folding is unsupported and remains on the existing route for accented queries.",
            ),
            (
                "Report from empty path.pdf",
                "Basename relevance works even when the indexed path is empty.",
                "The empty path still limits path navigation and source identity outside ranking.",
            ),
            (
                "summary file 0.pdf",
                "Windows path-only matches remain below any basename or extension match.",
                "Path-only rows still use modification time and stable ID within their class.",
            ),
            (
                "summary file 21.pdf",
                "POSIX path-only matches receive the same class as Windows path-only matches.",
                "Path separator style does not affect ranking, but path normalization remains platform-owned.",
            ),
            (
                "Team Report Archive",
                "Directories use their basename like files and remain eligible results.",
                "Directory/file type is not an additional relevance tie-breaker.",
            ),
        ],
        &old_contract,
        &new_contract,
    );
    let comparison_example_count = comparisons.len();
    let route_comparisons = [
        ("数据库", "档案-数据库-历史.pdf", "SQLite fallback"),
        ("café", "Café résumé notes.pdf", "SQLite fallback"),
        ("pdf", "summary file 0.pdf", "exact extension tier"),
        ("do", "manual-file-0.file", "extension prefix tier"),
        ("umen", "manual-file-0.file", "filename substring tier via extension field"),
        ("needle", "Monocle needle notes.pdf", "supported sparse relevance query"),
        ("annual.report", "Quarterly annual.report archive.pdf", "punctuation fallback"),
        ("zzznomatch", "(no result)", "supported zero-hit relevance query"),
    ]
    .into_iter()
    .map(|(query, name, route)| {
        let old = with_filename_relevance_candidate(false, || {
            search_global_entries_on_connection(&conn, query, 80, 0)
        })
        .unwrap();
        let candidate = with_filename_relevance_candidate(true, || {
            search_global_entries_on_connection(&conn, query, 80, 0)
        })
        .unwrap();
        let old_position = old.iter().position(|result| result.name == name);
        let candidate_position = candidate.iter().position(|result| result.name == name);
        json!({
            "query": query,
            "example": name,
            "route": route,
            "old_position_1_based": old_position.map(|position| position + 1),
            "proposed_position_1_based": candidate_position.map(|position| position + 1),
            "fallback_result_objects_equal": if route.contains("fallback") { json!(old == candidate) } else { JsonValue::Null },
            "proposed_category": candidate_position.map(|position| candidate[position].rank),
        })
    })
    .collect::<Vec<_>>();
    super::emit_record(&json!({
        "schema_version": 1,
        "record_type": "filename_relevance_golden_comparison",
        "contract_query": "report",
        "eligible_match_count": report_oracle.len(),
        "category_counts": {"filename_boundary": report_boundary,"filename_substring_or_extension": report_substring,"path_only": report_path},
        "comparisons": comparisons,
        "route_comparisons": route_comparisons,
        "candidate_matches_independent_rust_oracle": true,
        "fuzzy_candidate_schema_change": false,
        "incremental_insert_update_delete_fts_visibility": true,
        "comparison_example_count": comparison_example_count,
    }));
}

#[test]
fn filename_relevance_63_64_65_hits_keep_one_order_and_live_state() {
    let path = super::test_db_path();
    let _cleanup = super::BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open hit-transition database");
    db.upsert_global_volume(&super::test_volume())
        .expect("create enabled transition source");

    let mut initial_entries = Vec::with_capacity(63);
    for index in 0..63_i64 {
        let (name, entry_path) = match index % 3 {
            0 => (
                format!("Archive Needle case {index:02}.pdf"),
                format!(r"C:\Global\Transition\Boundary\case-{index:02}.pdf"),
            ),
            1 => (
                format!("Archive supersneedle case {index:02}.pdf"),
                format!(r"C:\Global\Transition\Substring\case-{index:02}.pdf"),
            ),
            _ => (
                format!("Archive case {index:02}.pdf"),
                format!(r"C:\Global\Transition\Needle\case-{index:02}.pdf"),
            ),
        };
        let mut entry = super::test_entry(&entry_path, &name, false);
        entry.extension = "pdf".to_string();
        entry.modified_at_fs = if index == 0 { None } else { Some(index % 4) };
        initial_entries.push(entry);
    }
    db.upsert_global_entries_batch(&initial_entries)
        .expect("insert exact 63-hit fixture");
    let mut conn = super::configured_connection(&path).expect("open transition reader");

    let read_candidate = |conn: &Connection| {
        with_filename_relevance_candidate(true, || {
            diagnostic_search_filename_relevance(conn, "needle", 4_096)
        })
        .expect("run supported relevance query")
        .expect("supported query always uses the relevance contract")
    };
    let read_public = |conn: &Connection, offset| {
        with_filename_relevance_candidate(true, || {
            search_global_entries_on_connection(conn, "needle", 20, offset)
        })
        .expect("run paged candidate search")
    };
    let assert_state = |conn: &Connection, expected_count: usize| {
        let oracle = filename_relevance_oracle(conn, "needle");
        let candidate = read_candidate(conn);
        assert_eq!(oracle.len(), expected_count);
        assert_eq!(
            candidate, oracle,
            "complete match order at {expected_count} hits"
        );
        for _ in 0..3 {
            assert_eq!(
                read_candidate(conn),
                oracle,
                "repeat query is deterministic"
            );
        }
        assert_eq!(read_public(conn, 0), page(&oracle, 0, 20));
        assert_eq!(read_public(conn, 20), page(&oracle, 20, 20));
        assert_eq!(read_public(conn, 40), page(&oracle, 40, 20));
        oracle
    };

    let at_63 = assert_state(&conn, 63);
    let initial_ids = at_63
        .iter()
        .map(|result| result.id.clone())
        .collect::<Vec<_>>();

    let sixty_fourth = super::test_entry(
        r"C:\Global\Transition\Boundary\case-63.pdf",
        "Archive Needle case 63.pdf",
        false,
    );
    db.upsert_global_entries_batch(std::slice::from_ref(&sixty_fourth))
        .expect("insert 64th eligible hit through existing FTS triggers");
    let at_64 = assert_state(&conn, 64);
    assert_eq!(
        at_64
            .iter()
            .filter(|result| initial_ids.contains(&result.id))
            .map(|result| result.id.clone())
            .collect::<Vec<_>>(),
        initial_ids,
        "adding hit 64 inserts one row without reordering the existing 63"
    );

    let sixty_fifth = super::test_entry(
        r"C:\Global\Transition\Needle\case-64.pdf",
        "Archive case 64.pdf",
        false,
    );
    db.upsert_global_entries_batch(std::slice::from_ref(&sixty_fifth))
        .expect("insert 65th path-only hit through existing FTS triggers");
    let at_65 = assert_state(&conn, 65);
    let at_64_ids = at_64
        .iter()
        .map(|result| result.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        at_65
            .iter()
            .filter(|result| at_64_ids.contains(&result.id))
            .map(|result| result.id.clone())
            .collect::<Vec<_>>(),
        at_64_ids,
        "adding hit 65 preserves relative order across all existing matches"
    );

    let id_64 = conn
        .query_row(
            "SELECT id FROM global_entries WHERE name = 'Archive Needle case 63.pdf'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("find 64th hit for delete transition");
    conn.execute("DELETE FROM global_entries WHERE id = ?1", [&id_64])
        .expect("delete 64th hit through existing FTS triggers");
    let after_delete_64 = assert_state(&conn, 64);
    assert_eq!(
        after_delete_64,
        at_65
            .iter()
            .filter(|result| result.id != id_64)
            .cloned()
            .collect::<Vec<_>>(),
        "removing hit 64 removes only that row from the stable order"
    );

    let id_65 = conn
        .query_row(
            "SELECT id FROM global_entries WHERE name = 'Archive case 64.pdf'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("find 65th hit for delete transition");
    conn.execute("DELETE FROM global_entries WHERE id = ?1", [&id_65])
        .expect("delete 65th hit through existing FTS triggers");
    assert_eq!(
        assert_state(&conn, 63),
        at_63,
        "return to the original 63-hit order"
    );

    let stale_id = &initial_ids[0];
    conn.execute(
        "UPDATE global_entries SET is_stale = 1 WHERE id = ?1",
        [stale_id],
    )
    .expect("mark one match stale");
    assert_eq!(assert_state(&conn, 62).len(), 62);
    conn.execute(
        "UPDATE global_entries SET is_stale = 0 WHERE id = ?1",
        [stale_id],
    )
    .expect("restore active match");
    assert_eq!(assert_state(&conn, 63), at_63);

    let mut disabled_volume = super::test_volume();
    disabled_volume.id = "gv_disabled_transition".to_string();
    disabled_volume.stable_volume_id = "disabled-transition-volume".to_string();
    disabled_volume.enabled = false;
    db.upsert_global_volume(&disabled_volume)
        .expect("create disabled target for cross-source update");
    conn.execute(
        "UPDATE global_entries SET volume_id = ?1 WHERE id = ?2",
        params![disabled_volume.id, stale_id],
    )
    .expect("move one matching row to another source");
    assert_eq!(assert_state(&conn, 62).len(), 62);
    conn.execute(
        "UPDATE global_entries SET volume_id = 'gv_test' WHERE id = ?1",
        [stale_id],
    )
    .expect("move matching row back to enabled source");
    assert_eq!(assert_state(&conn, 63), at_63);

    let snapshot_transaction = conn.transaction().expect("begin search snapshot");
    let snapshot_before = read_candidate(&snapshot_transaction);
    let writer = super::configured_connection(&path).expect("open concurrent status writer");
    writer
        .execute(
            "UPDATE global_volumes SET enabled = 0 WHERE id = 'gv_test'",
            [],
        )
        .expect("disable search source concurrently");
    assert_eq!(
        read_candidate(&snapshot_transaction),
        snapshot_before,
        "search keeps the same SQLite snapshot while source state changes"
    );
    snapshot_transaction
        .commit()
        .expect("finish consistent read snapshot");
    assert!(
        read_candidate(&conn).is_empty(),
        "new snapshot sees disabled source"
    );
    writer
        .execute(
            "UPDATE global_volumes SET enabled = 1 WHERE id = 'gv_test'",
            [],
        )
        .expect("re-enable search source");
    assert_eq!(
        read_candidate(&conn),
        at_63,
        "re-enabled source restores full order"
    );
}

#[derive(Debug, Clone, Copy)]
struct NativeSample {
    cpu_100ns: u64,
    working_set_bytes: u64,
    private_bytes: Option<u64>,
}

#[cfg(target_os = "windows")]
fn native_sample() -> Option<NativeSample> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    let process = unsafe { GetCurrentProcess() };
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let memory_ok = unsafe {
        GetProcessMemoryInfo(
            process,
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let cpu_ok =
        unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) };
    if memory_ok == 0 || cpu_ok == 0 {
        return None;
    }
    let filetime =
        |value: FILETIME| (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime);
    Some(NativeSample {
        cpu_100ns: filetime(kernel).saturating_add(filetime(user)),
        working_set_bytes: counters.WorkingSetSize as u64,
        private_bytes: Some(counters.PrivateUsage as u64),
    })
}

#[cfg(target_os = "macos")]
fn native_sample() -> Option<NativeSample> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if result != 0 {
        return None;
    }
    let usage = unsafe { usage.assume_init() };
    let timeval_100ns = |value: libc::timeval| {
        (value.tv_sec.max(0) as u64)
            .saturating_mul(10_000_000)
            .saturating_add((value.tv_usec.max(0) as u64).saturating_mul(10))
    };
    Some(NativeSample {
        cpu_100ns: timeval_100ns(usage.ru_utime).saturating_add(timeval_100ns(usage.ru_stime)),
        // macOS reports ru_maxrss in bytes. It is a process high-water mark,
        // not a per-query resident-set sample.
        working_set_bytes: usage.ru_maxrss.max(0) as u64,
        // Standard getrusage does not expose the macOS private footprint.
        private_bytes: None,
    })
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn native_sample() -> Option<NativeSample> {
    None
}

#[derive(Debug, Clone, Copy, Default)]
struct ResourceSample {
    cpu_ms: Option<f64>,
    working_set_bytes: Option<u64>,
    private_bytes: Option<u64>,
    sqlite_highwater_bytes: Option<u64>,
}

fn sqlite_memory_status(reset_highwater: bool) -> Option<(u64, u64)> {
    let mut current = 0_i64;
    let mut highwater = 0_i64;
    let result = unsafe {
        ffi::sqlite3_status64(
            ffi::SQLITE_STATUS_MEMORY_USED,
            &mut current,
            &mut highwater,
            i32::from(reset_highwater),
        )
    };
    (result == ffi::SQLITE_OK).then_some((current.max(0) as u64, highwater.max(0) as u64))
}

fn measure<T>(operation: impl FnOnce() -> T) -> (T, f64, ResourceSample) {
    let before = native_sample();
    let _ = sqlite_memory_status(true);
    let started = Instant::now();
    let value = operation();
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let after = native_sample();
    let process = before.zip(after);
    let resources = ResourceSample {
        cpu_ms: process.map(|(before, after)| {
            after.cpu_100ns.saturating_sub(before.cpu_100ns) as f64 / 10_000.0
        }),
        working_set_bytes: process
            .map(|(before, after)| before.working_set_bytes.max(after.working_set_bytes)),
        private_bytes: process.and_then(|(before, after)| {
            before
                .private_bytes
                .zip(after.private_bytes)
                .map(|(before, after)| before.max(after))
        }),
        sqlite_highwater_bytes: sqlite_memory_status(false).map(|(_, highwater)| highwater),
    };
    (value, elapsed_ms, resources)
}

#[derive(Default)]
struct Samples {
    elapsed_ms: Vec<f64>,
    cpu_ms: f64,
    peak_working_set: u64,
    peak_private_bytes: u64,
    sqlite_highwater_bytes: u64,
    cpu_samples: usize,
    working_set_samples: usize,
    private_samples: usize,
    sqlite_samples: usize,
}

impl Samples {
    fn push(&mut self, elapsed_ms: f64, resource: ResourceSample) {
        self.elapsed_ms.push(elapsed_ms);
        if let Some(cpu_ms) = resource.cpu_ms {
            self.cpu_ms += cpu_ms;
            self.cpu_samples += 1;
        }
        if let Some(working_set) = resource.working_set_bytes {
            self.peak_working_set = self.peak_working_set.max(working_set);
            self.working_set_samples += 1;
        }
        if let Some(private_bytes) = resource.private_bytes {
            self.peak_private_bytes = self.peak_private_bytes.max(private_bytes);
            self.private_samples += 1;
        }
        if let Some(sqlite_highwater_bytes) = resource.sqlite_highwater_bytes {
            self.sqlite_highwater_bytes = self.sqlite_highwater_bytes.max(sqlite_highwater_bytes);
            self.sqlite_samples += 1;
        }
    }

    fn json(&self) -> JsonValue {
        json!({
            "latency_ms": super::summarize_samples(&self.elapsed_ms),
            "process_cpu_ms": if self.cpu_samples == 0 { JsonValue::Null } else { json!(self.cpu_ms) },
            "process_cpu_sample_count": self.cpu_samples,
            "peak_sampled_working_set_bytes": if self.working_set_samples == 0 { JsonValue::Null } else { json!(self.peak_working_set) },
            "working_set_sample_count": self.working_set_samples,
            "peak_sampled_private_bytes": if self.private_samples == 0 { JsonValue::Null } else { json!(self.peak_private_bytes) },
            "private_memory_sample_count": self.private_samples,
            "sqlite_global_highwater_bytes": if self.sqlite_samples == 0 { JsonValue::Null } else { json!(self.sqlite_highwater_bytes) },
            "sqlite_sample_count": self.sqlite_samples,
            "resource_scope": "SQLite global allocator high-water is reset just before and read just after each call; Windows reports process working-set/private bytes and CPU at call boundaries; macOS reports CPU and cumulative process peak RSS through getrusage, but not private footprint or per-query RSS peaks",
        })
    }

    fn p95_ms(&self) -> f64 {
        super::summarize_samples(&self.elapsed_ms)["p95"]
            .as_f64()
            .expect("measured p95 is numeric")
    }
}

fn ids_hash(results: &[GlobalSearchResult]) -> String {
    let ids = results
        .iter()
        .map(|result| result.id.as_str())
        .collect::<Vec<_>>();
    blake3::hash(serde_json::to_string(&ids).unwrap().as_bytes())
        .to_hex()
        .to_string()
}

fn raw_sql_sample(
    conn: &Connection,
    candidate: bool,
    query: &str,
    limit: u32,
) -> (Vec<GlobalSearchResult>, f64, JsonValue, ResourceSample) {
    let sql = if candidate {
        diagnostic_search_filename_relevance_sql()
    } else {
        diagnostic_search_fts_sql()
    };
    let phrase = format!("\"{}\"", query.replace('"', "\"\""));
    let normalized_query = query.to_ascii_lowercase();
    let process_before = native_sample();
    let _ = sqlite_memory_status(true);
    let started = Instant::now();
    let mut statement = conn.prepare(&sql).expect("prepare raw paired SQL");
    for status in [
        StatementStatus::Sort,
        StatementStatus::FullscanStep,
        StatementStatus::VmStep,
    ] {
        statement.reset_status(status);
    }
    let mut rows = if candidate {
        statement.query(params![phrase, normalized_query, limit])
    } else {
        statement.query(params![phrase, limit])
    }
    .expect("execute raw paired SQL");
    let mut results = Vec::new();
    while let Some(row) = rows.next().expect("advance raw paired SQL") {
        results.push(sql_result(row).expect("map raw paired SQL result"));
    }
    drop(rows);
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let process_after = native_sample();
    let sqlite_highwater_bytes = sqlite_memory_status(false).map(|(_, highwater)| highwater);
    let counters = json!({
        "sort_operations": statement.get_status(StatementStatus::Sort),
        "fullscan_steps": statement.get_status(StatementStatus::FullscanStep),
        "vm_steps": statement.get_status(StatementStatus::VmStep),
    });
    let process = process_before.zip(process_after);
    let resources = ResourceSample {
        cpu_ms: process.map(|(before, after)| {
            after.cpu_100ns.saturating_sub(before.cpu_100ns) as f64 / 10_000.0
        }),
        working_set_bytes: process
            .map(|(before, after)| before.working_set_bytes.max(after.working_set_bytes)),
        private_bytes: process.and_then(|(before, after)| {
            before
                .private_bytes
                .zip(after.private_bytes)
                .map(|(before, after)| before.max(after))
        }),
        sqlite_highwater_bytes,
    };
    (results, elapsed_ms, counters, resources)
}

fn diversify_report_matches(conn: &Connection, entries: u64) -> f64 {
    let started = Instant::now();
    let transaction = conn
        .unchecked_transaction()
        .expect("start rank distribution fixture transaction");
    {
        let mut statement = transaction
            .prepare("UPDATE global_entries SET name=?2, name_normalized=?3, path=?4, path_normalized=?5, extension='pdf' WHERE id=?1")
            .expect("prepare rank distribution fixture update");
        for index in (2..entries).step_by(20) {
            let id = format!("ge_bench_{index:07}");
            let (name, path) = match (index / 20) % 3 {
                0 => (
                    format!("Quarterly Report {index:07}.pdf"),
                    format!(
                        r"C:\Users\Owner\Documents\Quarterly\Team\Report-{index:07}\Quarterly Report {index:07}.pdf"
                    ),
                ),
                1 => (
                    format!("Subreport {index:07}.pdf"),
                    format!(r"C:\Users\Owner\Documents\Guides\Subreport {index:07}.pdf"),
                ),
                _ => (
                    format!("Summary file {index:07}.pdf"),
                    format!(
                        r"C:\Users\Owner\Documents\Archive\Reports\Summary file {index:07}.pdf"
                    ),
                ),
            };
            statement
                .execute(params![
                    id,
                    name,
                    name.to_lowercase(),
                    path,
                    path.to_lowercase()
                ])
                .expect("update one synthetic report rank class");
        }
        let mut sparse_count = 0;
        for index in 0..entries {
            if index % 20 != 2 && sparse_count < 63 {
                let name = format!("Archive sparseword note {index:07}.pdf");
                let path = format!(r"C:\Users\Owner\Documents\Sparse\{index:07}\{name}");
                statement
                    .execute(params![
                        format!("ge_bench_{index:07}"),
                        name,
                        format!("archive sparseword note {index:07}.pdf"),
                        path.clone(),
                        path.to_lowercase()
                    ])
                    .expect("update one 63-hit sparse relevance row through existing FTS triggers");
                sparse_count += 1;
            } else if index == entries - 1 {
                let name = format!("Monocle needle note {index:07}.pdf");
                let path = format!(r"C:\Users\Owner\Documents\Sparse\Needle\{name}");
                statement
                    .execute(params![
                        format!("ge_bench_{index:07}"),
                        name,
                        format!("monocle needle note {index:07}.pdf"),
                        path.clone(),
                        path.to_lowercase()
                    ])
                    .expect(
                        "update one single-hit sparse relevance row through existing FTS triggers",
                    );
            }
        }
        assert_eq!(sparse_count, 63);
    }
    transaction
        .commit()
        .expect("commit high-hit and sparse relevance fixture");
    started.elapsed().as_secs_f64() * 1_000.0
}

fn page(results: &[GlobalSearchResult], offset: u32, limit: u32) -> Vec<GlobalSearchResult> {
    results
        .iter()
        .skip(offset as usize)
        .take(limit as usize)
        .cloned()
        .collect()
}

fn eligible_fts_hit_count(conn: &Connection, query: &str) -> i64 {
    let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
    conn.query_row(
        "SELECT COUNT(*) FROM global_entries_fts CROSS JOIN global_entries ge CROSS JOIN global_volumes gv WHERE global_entries_fts MATCH ?1 AND ge.rowid=global_entries_fts.rowid AND gv.id=ge.volume_id AND gv.enabled=1 AND gv.coverage_pending=0 AND ge.is_stale=0",
        [fts_query],
        |row| row.get(0),
    )
    .expect("count enabled non-stale FTS hits")
}

fn explain_fts_plan(conn: &Connection, candidate: bool, query: &str, limit: u32) -> Vec<String> {
    let sql = if candidate {
        diagnostic_search_filename_relevance_sql()
    } else {
        diagnostic_search_fts_sql()
    };
    let explain_sql = format!("EXPLAIN QUERY PLAN {sql}");
    let mut statement = conn.prepare(&explain_sql).expect("prepare FTS query plan");
    let phrase = format!("\"{}\"", query.replace('"', "\"\""));
    let normalized_query = query.to_ascii_lowercase();
    let mut rows = if candidate {
        statement.query(params![phrase, normalized_query, limit])
    } else {
        statement.query(params![phrase, limit])
    }
    .expect("execute FTS query plan");
    let mut plan = Vec::new();
    while let Some(row) = rows.next().expect("advance FTS query plan") {
        let id: i64 = row.get(0).expect("read query-plan step id");
        let parent: i64 = row.get(1).expect("read query-plan parent id");
        let detail: String = row.get(3).expect("read query-plan detail");
        plan.push(format!("{id}:{parent}:{detail}"));
    }
    plan
}

#[cfg(debug_assertions)]
fn require_release_benchmark() {
    panic!("the qualification harness must run from a Release build");
}

#[cfg(not(debug_assertions))]
fn require_release_benchmark() {}

enum MeasuredRoute {
    Search(Vec<GlobalSearchResult>),
    Snapshot(Box<GlobalSearchSnapshot>),
}

fn paired_route_samples(
    db: &Database,
    query: &str,
    limit: u32,
    offset: u32,
    old_expected: &[GlobalSearchResult],
    new_expected: &[GlobalSearchResult],
    expected_snapshot: Option<&GlobalSearchSnapshot>,
) -> [Samples; 2] {
    let mut outputs = [Samples::default(), Samples::default()];
    for iteration in 0..35 {
        for n in 0..2 {
            let candidate = (iteration + n) % 2 == 0;
            let variant = usize::from(candidate);
            let (actual, elapsed, resources) = match expected_snapshot {
                Some(expected_snapshot) => {
                    let (actual, elapsed, resources) = measure(|| {
                        with_filename_relevance_candidate(candidate, || {
                            db.search_global_entries_snapshot(query, limit, offset)
                        })
                    });
                    let actual = actual.expect("measure paired repository snapshot");
                    assert_eq!(actual.source_health, expected_snapshot.source_health);
                    assert_eq!(actual.source_revision, expected_snapshot.source_revision);
                    assert_eq!(actual.index_status, expected_snapshot.index_status);
                    (
                        MeasuredRoute::Snapshot(Box::new(actual)),
                        elapsed,
                        resources,
                    )
                }
                None => {
                    let (actual, elapsed, resources) = measure(|| {
                        with_filename_relevance_candidate(candidate, || {
                            db.search_global_entries(query, limit, offset)
                        })
                    });
                    (
                        MeasuredRoute::Search(actual.expect("measure paired pooled search")),
                        elapsed,
                        resources,
                    )
                }
            };
            let expected = if candidate {
                new_expected
            } else {
                old_expected
            };
            match actual {
                MeasuredRoute::Snapshot(actual) => {
                    assert_eq!(actual.results, expected, "snapshot variant {variant} page");
                }
                MeasuredRoute::Search(actual) => {
                    assert_eq!(actual, expected, "search variant {variant} page");
                }
            }
            if iteration >= 5 {
                outputs[variant].push(elapsed, resources);
            }
        }
    }
    outputs
}

fn paired_raw_sql_samples(
    conn: &Connection,
    query: &str,
    limit: u32,
    old_complete: &[GlobalSearchResult],
    new_complete: &[GlobalSearchResult],
) -> ([Samples; 2], [JsonValue; 2]) {
    let mut outputs = [Samples::default(), Samples::default()];
    let mut counters = [JsonValue::Null, JsonValue::Null];
    let old_expected = old_complete
        .iter()
        .take(limit as usize)
        .cloned()
        .collect::<Vec<_>>();
    let new_expected = new_complete
        .iter()
        .take(limit as usize)
        .cloned()
        .collect::<Vec<_>>();
    for iteration in 0..35 {
        for n in 0..2 {
            let candidate = (iteration + n) % 2 == 0;
            let variant = usize::from(candidate);
            let (actual, elapsed, measured_counters, resources) =
                raw_sql_sample(conn, candidate, query, limit);
            let expected = if candidate {
                &new_expected
            } else {
                &old_expected
            };
            assert_eq!(actual, *expected, "raw SQL variant {variant} result window");
            if iteration >= 5 {
                outputs[variant].push(elapsed, resources);
                counters[variant] = measured_counters;
            }
        }
    }
    (outputs, counters)
}

struct PairInput<'a> {
    context: &'a JsonValue,
    query: &'a str,
    limit: u32,
    offset: u32,
    eligible_hit_count: usize,
    old_complete: &'a [GlobalSearchResult],
    new_complete: &'a [GlobalSearchResult],
}

fn emit_pair(db: &Database, conn: &Connection, input: PairInput<'_>) -> bool {
    let PairInput {
        context,
        query,
        limit,
        offset,
        eligible_hit_count,
        old_complete,
        new_complete,
    } = input;
    let tier_limit = limit.saturating_add(offset).min(4_096);
    let old_expected = page(old_complete, offset, limit);
    let new_expected = page(new_complete, offset, limit);

    let (raw, raw_counters) =
        paired_raw_sql_samples(conn, query, tier_limit, old_complete, new_complete);

    let snapshot_oracle = with_filename_relevance_candidate(false, || {
        db.search_global_entries_snapshot(query, limit, offset)
    })
    .expect("read paired snapshot source facts");
    let snapshot = paired_route_samples(
        db,
        query,
        limit,
        offset,
        &old_expected,
        &new_expected,
        Some(&snapshot_oracle),
    );
    let search = paired_route_samples(db, query, limit, offset, &old_expected, &new_expected, None);
    let search_p95_passed = search[1].p95_ms() <= 100.0;
    let snapshot_p95_passed = snapshot[1].p95_ms() <= 100.0;
    super::emit_record(&json!({
        "schema_version": 1,
        "record_type": "filename_relevance_paired_window",
        "context": context,
        "query": query,
        "hit_count": eligible_hit_count,
        "limit": limit,
        "offset": offset,
        "candidate_window": tier_limit,
        "warmups": 5,
        "samples": 30,
        "route_order": "paired, alternating baseline/candidate first by iteration",
        "candidate_output_window_matches_independent_oracle": true,
        "first_page_ids": {"old_bm25_blake3": ids_hash(&page(old_complete, 0, limit)), "candidate_blake3": ids_hash(&page(new_complete, 0, limit))},
        "this_page_ids": {"old_bm25_blake3": ids_hash(&old_expected), "candidate_blake3": ids_hash(&new_expected)},
        "raw_fts_sql": {"old_bm25": raw[0].json(), "filename_relevance": raw[1].json()},
        "raw_fts_sql_counters": {"old_bm25": raw_counters[0], "filename_relevance": raw_counters[1]},
        "pooled_search": {"old_bm25": search[0].json(), "filename_relevance": search[1].json()},
        "repository_snapshot": {"old_bm25": snapshot[0].json(), "filename_relevance": snapshot[1].json()},
        "candidate_100ms_p95_gate": {
            "pooled_search_passed": search_p95_passed,
            "repository_snapshot_passed": snapshot_p95_passed,
            "passed": search_p95_passed && snapshot_p95_passed
        },
        "snapshot_source_health_revision_status_equal": true,
        "output_fields_checked_against_each_route_oracle": true,
        "resources_note": "Windows process CPU/working-set/private memory is sampled at call boundaries. macOS reports CPU and cumulative peak RSS through getrusage, but not private footprint or per-query RSS peaks. SQLite memory and SQL statement counters are collected independently."
    }));
    search_p95_passed && snapshot_p95_passed
}

fn emit_low_hit_pair(
    db: &Database,
    conn: &Connection,
    context: &JsonValue,
    query: &str,
    offset: u32,
    eligible_hit_count: i64,
) -> bool {
    let limit = 80;
    assert!(
        eligible_hit_count < 80,
        "low-hit pair must fit in one result page"
    );
    let old_complete = with_filename_relevance_candidate(false, || {
        search_global_entries_on_connection(conn, query, 4_096, 0)
    })
    .expect("read low-hit BM25 baseline results");
    let candidate_complete = with_filename_relevance_candidate(true, || {
        search_global_entries_on_connection(conn, query, 4_096, 0)
    })
    .expect("read low-hit candidate results");
    let oracle = filename_relevance_oracle(conn, query);
    assert_eq!(
        candidate_complete, oracle,
        "supported sparse/zero query follows the independent relevance oracle"
    );
    assert_eq!(oracle.len(), eligible_hit_count as usize);
    let old_expected = page(&old_complete, offset, limit);
    let new_expected = page(&oracle, offset, limit);
    let raw = paired_raw_sql_samples(conn, query, 4_096, &old_complete, &oracle);
    let snapshot_oracle = with_filename_relevance_candidate(false, || {
        db.search_global_entries_snapshot(query, limit, offset)
    })
    .expect("read low-hit snapshot source facts");
    let search = paired_route_samples(db, query, limit, offset, &old_expected, &new_expected, None);
    let snapshot = paired_route_samples(
        db,
        query,
        limit,
        offset,
        &old_expected,
        &new_expected,
        Some(&snapshot_oracle),
    );
    let search_p95_passed = search[1].p95_ms() <= 100.0;
    let snapshot_p95_passed = snapshot[1].p95_ms() <= 100.0;
    super::emit_record(&json!({
        "schema_version": 1,
        "record_type": "filename_relevance_low_hit_pair",
        "context": context,
        "query": query,
        "eligible_fts_hits": eligible_hit_count,
        "limit": limit,
        "offset": offset,
        "warmups": 5,
        "samples": 30,
        "fallback_to_current_bm25": false,
        "candidate_complete_order_matches_independent_oracle": true,
        "hit_count_changes_do_not_switch_the_candidate_order": true,
        "old_bm25_and_candidate_full_results_equal": old_complete == oracle,
        "raw_fts_sql": {"old_bm25": raw.0[0].json(), "filename_relevance": raw.0[1].json()},
        "raw_fts_sql_counters": {"old_bm25": raw.1[0], "filename_relevance": raw.1[1]},
        "pooled_search": {"old_bm25": search[0].json(), "candidate_route": search[1].json()},
        "repository_snapshot": {"old_bm25": snapshot[0].json(), "candidate_route": snapshot[1].json()},
        "candidate_100ms_p95_gate": {
            "pooled_search_passed": search_p95_passed,
            "repository_snapshot_passed": snapshot_p95_passed,
            "passed": search_p95_passed && snapshot_p95_passed
        },
        "snapshot_source_health_revision_status_equal": true,
    }));
    search_p95_passed && snapshot_p95_passed
}

#[test]
#[ignore = "Owner #474 bounded mixed filename relevance candidate qualification"]
fn filename_relevance_candidate_paired_100k_200k() {
    require_release_benchmark();
    let entries = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES")
        .expect("set ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES to 100000, 200000, or 1000000")
        .parse::<u64>()
        .expect("parse benchmark entry count");
    assert!(
        matches!(entries, 100_000 | 200_000 | 1_000_000),
        "only the 100k/200k preflight and one authorized 1m qualification run here"
    );
    let output_path = env::var_os("ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT").map(PathBuf::from);
    if let Some(output_path) = &output_path {
        assert!(
            !output_path.exists(),
            "refusing to overwrite existing evidence artifact: {}",
            output_path.display()
        );
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).expect("create evidence parent directory");
        }
    }

    let context = super::benchmark_context(entries);
    let path = super::test_db_path();
    let _cleanup = super::BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open deterministic candidate database");
    db.upsert_global_volume(&super::test_volume())
        .expect("create deterministic candidate source");
    let (_, generation_ms, population_ms, _) = super::populate_synthetic_database(&db, entries);
    let conn = super::configured_connection(&path).expect("open candidate measurement connection");
    let rank_distribution_mutation_ms = diversify_report_matches(&conn, entries);
    let before_schema = schema_signature(&conn);
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM global_entries_fts JOIN global_entries ge ON ge.rowid=global_entries_fts.rowid JOIN global_volumes gv ON gv.id=ge.volume_id WHERE global_entries_fts MATCH ?1 AND gv.enabled=1 AND gv.coverage_pending=0 AND ge.is_stale=0",
            ["\"report\""],
            |row| row.get(0),
        )
        .expect("count mixed relevance matches");
    assert_eq!(
        count as u64,
        entries / 20,
        "fixture preserves 5 percent report hit rate"
    );
    let sparseword_count = eligible_fts_hit_count(&conn, "sparseword");
    let needle_count = eligible_fts_hit_count(&conn, "needle");
    let no_match_count = eligible_fts_hit_count(&conn, "zzznomatchtoken");
    assert_eq!(sparseword_count, 63);
    assert_eq!(needle_count, 1);
    assert_eq!(no_match_count, 0);

    // The public route clamps page size to 200. Isolate each FTS tier here so
    // the comparison covers the full 4096-row candidate window used by the
    // maximum supported offset rather than silently comparing only 200 rows.
    let old_complete =
        with_filename_relevance_candidate(false, || diagnostic_search_fts(&conn, "report", 4_096))
            .expect("read old BM25 FTS window");
    let new_complete = diagnostic_search_filename_relevance(&conn, "report", 4_096)
        .expect("read proposed FTS candidate window")
        .expect("high-hit candidate must qualify");
    let oracle = filename_relevance_oracle(&conn, "report");
    assert_eq!(oracle.len(), count as usize);
    let expected_window = oracle.iter().take(4_096).cloned().collect::<Vec<_>>();
    let first_difference = new_complete
        .iter()
        .zip(&expected_window)
        .position(|(actual, expected)| actual != expected);
    assert!(
        new_complete.len() == expected_window.len() && first_difference.is_none(),
        "candidate/oracle window mismatch: candidate_len={}, oracle_len={}, first_difference={:?}",
        new_complete.len(),
        expected_window.len(),
        first_difference.map(|index| {
            let actual = &new_complete[index];
            let expected = &expected_window[index];
            json!({
                "index":index,
                "candidate":{"id":actual.id,"name":actual.name,"rank":actual.rank,"modified_at_fs":actual.modified_at_fs,"path":actual.path},
                "oracle":{"id":expected.id,"name":expected.name,"rank":expected.rank,"modified_at_fs":expected.modified_at_fs,"path":expected.path},
            })
        })
    );
    let first_order_difference = old_complete
        .iter()
        .zip(&new_complete)
        .position(|(old, candidate)| old.id != candidate.id);
    assert!(
        first_order_difference.is_some(),
        "mixed-category benchmark fixture must demonstrate an actual old/new ID order difference"
    );
    assert!(old_complete.iter().any(|result| result.rank < 0.0));
    let rank_class_counts = json!({
        "filename_boundary": oracle.iter().filter(|row| row.rank == 0.0).count(),
        "filename_substring": oracle.iter().filter(|row| row.rank == 1.0).count(),
        "path_only": oracle.iter().filter(|row| row.rank == 2.0).count()
    });
    let bm25_plan = explain_fts_plan(&conn, false, "report", 4_096);
    let relevance_plan = explain_fts_plan(&conn, true, "report", 4_096);
    assert_eq!(
        with_filename_relevance_candidate(true, || {
            search_global_entries_on_connection(&conn, "report", 80, 4_016)
        })
        .unwrap(),
        page(&expected_window, 4_016, 80),
        "offset 4016 is a slice of the full new-order top-4096 window"
    );
    assert!(with_filename_relevance_candidate(true, || {
        search_global_entries_on_connection(&conn, "report", 80, 4_096)
    })
    .unwrap()
    .is_empty());
    drop(oracle);
    super::emit_record(&json!({
        "schema_version": 1,
        "record_type": "filename_relevance_environment",
        "context": context,
        "entries": entries,
        "report_eligible_hits": count,
        "rank_class_counts": rank_class_counts,
        "fixture_mutation": "reuse existing global_search_benchmark.rs synthetic production-schema fixture; transform its 5 percent report-hit population into equal boundary/substring/path-only classes and prepare 63-hit, 1-hit and 0-hit supported queries; existing FTS update triggers maintain all query terms",
        "fixture_generation_ms": generation_ms,
        "fixture_population_ms": population_ms,
        "relevance_distribution_mutation_through_existing_fts_triggers_ms": rank_distribution_mutation_ms,
        "old_bm25_order_hash": ids_hash(&old_complete),
        "new_filename_relevance_order_hash": ids_hash(&new_complete),
        "old_vs_candidate_order": {
            "top_4096_equal": first_order_difference.is_none(),
            "first_differing_position_1_based": first_order_difference.map(|index| index + 1),
        },
        "top_4096_candidate_window_matches_full_independent_rust_oracle": true,
        "schema_signature_before_benchmark": before_schema,
        "sqlite_version": rusqlite::version(),
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "explain_query_plan": {"old_bm25": bm25_plan, "filename_relevance": relevance_plan},
        "one_million_qualification": entries == 1_000_000
    }));

    let mut failed_p95_gates = Vec::new();
    for (limit, offset) in [(80, 0), (80, 4_016)] {
        if !emit_pair(
            &db,
            &conn,
            PairInput {
                context: &context,
                query: "report",
                limit,
                offset,
                eligible_hit_count: count as usize,
                old_complete: &old_complete,
                new_complete: &expected_window,
            },
        ) {
            failed_p95_gates.push(format!("report 80@{offset}"));
        }
    }
    for (query, offset, hits) in [
        ("sparseword", 0, sparseword_count),
        ("sparseword", 4_016, sparseword_count),
        ("needle", 0, needle_count),
        ("zzznomatchtoken", 0, no_match_count),
    ] {
        if !emit_low_hit_pair(&db, &conn, &context, query, offset, hits) {
            failed_p95_gates.push(format!("{query} 80@{offset}"));
        }
    }
    assert_eq!(
        schema_signature(&conn),
        before_schema,
        "search reads do not alter durable schema"
    );
    super::emit_record(&json!({
        "schema_version": 1,
        "record_type": "filename_relevance_postconditions",
        "context": context,
        "schema_signature_unchanged": true,
        "candidate_persistent_index_count": 0,
        "candidate_trigger_count": 0,
        "database_storage_after_search": super::database_metrics(&conn, &path),
        "candidate_temp_store_effect": "in-memory SQLite sorter; per-request SQLite allocator high-water is measured",
        "additional_candidate_write_amplification": "zero persistent objects or facts; existing FTS5 triggers maintain their current name/path/extension data and the candidate derives rank at read time",
        "fts_trigger_sql_unchanged": true,
        "production_route_default": "BM25 unless the Cargo feature and environment opt-in are both set",
        "candidate_100ms_search_and_snapshot_p95_failures": failed_p95_gates,
        "default_index_authorization_changed": false
    }));
    assert!(
        failed_p95_gates.is_empty(),
        "the unchanged 100ms p95 target failed for candidate queries: {failed_p95_gates:?}"
    );
}
