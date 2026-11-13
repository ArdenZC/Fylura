use super::*;
use crate::global_index::filename_search::FilenameSearchDiagnostics;
use crate::global_index::models::{GlobalEntryInput, GlobalSearchResult, GlobalVolume};
use rusqlite::Connection;
use std::path::PathBuf;

fn open_candidate_database() -> (Database, PathBuf) {
    let path = test_db_path();
    let db = Database::open(&path).expect("open isolated filename-search database");
    db.upsert_global_volume(&test_volume())
        .expect("insert authorized source");
    db.update_global_volume_state(
        "gv_test",
        INDEX_STATUS_READY,
        None,
        None,
        None,
        Some(10),
        None,
    )
    .expect("mark source ready");
    db.filename_search.enable_for_test();
    (db, path)
}

fn add_volume(db: &Database, id: &str, mount_path: &str) -> GlobalVolume {
    let mut volume = test_volume();
    volume.id = id.into();
    volume.stable_volume_id = format!("stable-{id}");
    volume.mount_path = mount_path.into();
    db.upsert_global_volume(&volume)
        .expect("insert second authorized source");
    db.update_global_volume_state(id, INDEX_STATUS_READY, None, None, None, Some(10), None)
        .expect("mark second source ready");
    volume
}

fn fixture_entry(
    volume_id: &str,
    path: &str,
    name: &str,
    extension: &str,
    modified_at: Option<i64>,
    is_directory: bool,
) -> GlobalEntryInput {
    let mut entry = test_entry(path, name, is_directory);
    entry.volume_id = volume_id.into();
    entry.platform_file_id = format!("native:{path}");
    entry.name = name.into();
    entry.path = path.into();
    entry.extension = extension.into();
    entry.modified_at_fs = modified_at;
    entry
}

fn build_index(db: &Database) -> FilenameSearchDiagnostics {
    db.filename_search
        .rebuild_now_for_test()
        .expect("build compact filename index from SQLite authority");
    let diagnostics = db.filename_search.diagnostics();
    assert!(diagnostics.ready);
    diagnostics
}

fn claim_transient_test_operation(db: &Database, operation_id: &str) {
    let source = db
        .get_global_volume("gv_test")
        .expect("load source to claim test operation")
        .expect("test source exists");
    db.claim_global_index_operation(
        &source,
        operation_id,
        crate::global_index::SourceOperationKind::Initial,
        crate::global_index::SourceOperationRoute::Service,
    )
    .expect("claim test source operation")
    .expect("test operation owner is available");
}

#[test]
fn transient_filename_catalog_searches_before_rowids_and_respects_scope_fence() {
    let (db, _path) = open_candidate_database();
    db.update_global_volume_state(
        "gv_test",
        INDEX_STATUS_INDEXING,
        None,
        None,
        None,
        None,
        None,
    )
    .expect("mark source collection incomplete");
    claim_transient_test_operation(&db, "transient-operation");
    let included = fixture_entry(
        "gv_test",
        r"C:\Global\transient-report-budget.txt",
        "transient-report-budget.txt",
        "txt",
        Some(50),
        false,
    );
    let excluded = fixture_entry(
        "gv_test",
        r"C:\Global\ignored-transient-report.txt",
        "ignored-transient-report.txt",
        "txt",
        Some(40),
        false,
    );
    db.filename_search
        .begin_transient_catalog("gv_test", "transient-operation");
    db.filename_search
        .append_transient_catalog(
            "gv_test",
            "transient-operation",
            1,
            &[included.clone(), excluded],
            &[true, false],
        )
        .expect("append preflighted MFT entries");

    let snapshot = db
        .search_global_entries_snapshot("transient-report", 20, 0)
        .expect("search transient filename catalog");
    assert_eq!(snapshot.results.len(), 1);
    assert_eq!(snapshot.results[0].path, included.path);
    assert_eq!(snapshot.results[0].id, included.entry_id());
    assert!(!snapshot.index_status.collection_complete);
    assert_eq!(snapshot.index_status.processed_entries, 1);
    assert_eq!(snapshot.index_status.total_entries, 1);
    assert!(
        crate::global_index::commands::filter_live_transient_search_results(
            &db,
            snapshot.results.clone(),
        )
        .is_empty(),
        "a transient path deleted after its MFT observation must not reach the UI"
    );
    let interior_match = db
        .search_global_entries_snapshot("report-budget", 20, 0)
        .expect("query filename grams before SQLite rowids exist");
    assert_eq!(interior_match.results.len(), 1);
    assert_eq!(interior_match.results[0].path, included.path);
    let parent_path_match = db
        .search_global_entries_snapshot("global", 20, 0)
        .expect("query a transient parent-path fragment through the on-demand path scan");
    assert_eq!(parent_path_match.results.len(), 1);
    assert_eq!(parent_path_match.results[0].path, included.path);
    let transient_entry = db
        .get_global_entry_or_transient(&included.entry_id())
        .expect("resolve a pre-persistence search selection")
        .expect("authorized transient entry remains openable");
    assert_eq!(transient_entry.path, included.path);
    assert!(!transient_entry.is_stale);

    let mut refreshed = fixture_entry(
        "gv_test",
        r"C:\Global\transient-refreshed-budget.txt",
        "transient-refreshed-budget.txt",
        "txt",
        Some(51),
        false,
    );
    refreshed.platform_file_id = included.platform_file_id.clone();
    db.filename_search
        .append_transient_catalog(
            "gv_test",
            "transient-operation",
            1,
            &[refreshed.clone()],
            &[true],
        )
        .expect("refresh the same native identity");
    let stale_name = db
        .search_global_entries_snapshot("report-budget", 20, 0)
        .expect("stale gram postings must recheck current entry text");
    assert!(stale_name.results.is_empty());
    let refreshed_name = db
        .search_global_entries_snapshot("refreshed-budget", 20, 0)
        .expect("updated transient filename remains searchable");
    assert_eq!(refreshed_name.results.len(), 1);
    assert_eq!(refreshed_name.results[0].path, refreshed.path);

    assert_eq!(
        db.conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM global_entries", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );

    db.save_global_index_coverage(
        1,
        crate::global_index::coverage::CoverageMode::SmartLean,
        vec![],
    )
    .expect("publish a new scope revision");
    assert!(db
        .search_global_entries_snapshot("transient-report", 20, 0)
        .expect("stale catalog must fall back to the fenced database snapshot")
        .results
        .is_empty());
}

#[test]
fn transient_filename_catalog_keeps_existing_managed_link_during_rebuild() {
    let (db, _path) = open_candidate_database();
    let input = fixture_entry(
        "gv_test",
        r"C:\Global\managed-transient-budget.txt",
        "managed-transient-budget.txt",
        "txt",
        Some(50),
        false,
    );
    db.upsert_global_entries_batch(std::slice::from_ref(&input))
        .expect("persist the prior generation");
    db.add_managed_scope(AddManagedScopeRequest {
        path: r"C:\Global".into(),
        global_entry_id: None,
        enabled: true,
        allow_local_ai: true,
        allow_cloud_ai: false,
    })
    .expect("associate the prior generation with its managed scope");
    db.conn()
        .unwrap()
        .execute(
            "UPDATE global_entries SET is_stale=1 WHERE id=?1",
            [input.entry_id()],
        )
        .expect("hide the previous generation while rebuilding");
    db.update_global_volume_state(
        "gv_test",
        INDEX_STATUS_INDEXING,
        None,
        None,
        None,
        None,
        None,
    )
    .expect("mark rebuild incomplete");
    claim_transient_test_operation(&db, "managed-rebuild");

    let policy_revision = crate::global_index::coverage::load_policy(&db.conn().unwrap())
        .expect("read current scope revision")
        .revision;
    db.filename_search
        .begin_transient_catalog("gv_test", "managed-rebuild");
    db.filename_search
        .append_transient_catalog(
            "gv_test",
            "managed-rebuild",
            policy_revision,
            std::slice::from_ref(&input),
            &[true],
        )
        .expect("publish the rebuilt filename row");

    let snapshot = db
        .search_global_entries_snapshot("managed-transient-budget", 10, 0)
        .expect("query the transient rebuild overlay");
    assert_eq!(snapshot.results.len(), 1);
    assert!(snapshot.results[0].managed);
    assert_eq!(snapshot.results[0].id, input.entry_id());
    assert!(!snapshot.index_status.collection_complete);
}

#[derive(Debug)]
struct OracleRow {
    result: GlobalSearchResult,
    name_normalized: String,
    path_normalized: String,
    extension_normalized: String,
}

fn oracle_rows(connection: &Connection) -> Vec<OracleRow> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT ge.id, ge.volume_id, ge.platform_file_id, ge.name, ge.path,
                   ge.extension, ge.is_directory, ge.size, ge.created_at_fs,
                   ge.modified_at_fs, ge.file_attributes, ge.is_hidden, ge.is_system,
                   ge.source_provider,
                   EXISTS (
                       SELECT 1 FROM managed_entries me
                       JOIN managed_scopes ms ON ms.id=me.managed_scope_id
                       WHERE me.global_entry_id=ge.id AND me.enabled=1 AND ms.enabled=1
                   ),
                   ge.name_normalized, ge.path_normalized
            FROM global_entries ge
            JOIN global_volumes gv ON gv.id=ge.volume_id
            WHERE gv.enabled=1 AND gv.coverage_pending=0
              AND (gv.platform != 'windows' OR gv.scope_authorization_state='authorized')
              AND ge.is_stale=0
            "#,
        )
        .expect("prepare independent full-row oracle");
    statement
        .query_map([], |row| {
            let extension: String = row.get(5)?;
            Ok(OracleRow {
                result: GlobalSearchResult {
                    id: row.get(0)?,
                    volume_id: row.get(1)?,
                    platform_file_id: row.get(2)?,
                    name: row.get(3)?,
                    path: row.get(4)?,
                    extension: extension.clone(),
                    is_directory: row.get(6)?,
                    size: row.get(7)?,
                    created_at_fs: row.get(8)?,
                    modified_at_fs: row.get(9)?,
                    file_attributes: row.get(10)?,
                    is_hidden: row.get(11)?,
                    is_system: row.get(12)?,
                    source_provider: row.get(13)?,
                    managed: row.get(14)?,
                    rank: 0.0,
                },
                name_normalized: row.get(15)?,
                path_normalized: row.get(16)?,
                extension_normalized: extension.to_ascii_lowercase(),
            })
        })
        .expect("query complete eligible source set")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect complete eligible source set")
}

fn oracle_tier(row: &OracleRow, query: &str) -> Option<u8> {
    if row.name_normalized == query {
        Some(0)
    } else if row.name_normalized.starts_with(query) {
        Some(1)
    } else if row.extension_normalized == query {
        Some(2)
    } else if row.extension_normalized.starts_with(query) {
        Some(3)
    } else if row.name_normalized.contains(query)
        && has_ascii_word_boundary(&row.name_normalized, query)
    {
        Some(4)
    } else if row.name_normalized.contains(query) || row.extension_normalized.contains(query) {
        Some(5)
    } else if row.path_normalized.to_ascii_lowercase().contains(query) {
        Some(6)
    } else {
        None
    }
}

fn has_ascii_word_boundary(name: &str, query: &str) -> bool {
    name.match_indices(query).any(|(start, matched)| {
        let before = name[..start].chars().next_back();
        let after = name[start + matched.len()..].chars().next();
        !before.is_some_and(is_ascii_word_character) && !after.is_some_and(is_ascii_word_character)
    })
}

fn is_ascii_word_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

fn oracle_search(
    connection: &Connection,
    query: &str,
    limit: u32,
    offset: u32,
) -> Vec<GlobalSearchResult> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() || offset >= 4096 {
        return Vec::new();
    }
    let mut ranked = oracle_rows(connection)
        .into_iter()
        .filter_map(|mut row| {
            let tier = oracle_tier(&row, &query)?;
            row.result.rank = match tier {
                0..=2 => 0.0,
                3 => 1.0,
                4 => 2.0,
                5 => 3.0,
                _ => 4.0,
            };
            Some((tier, row.result))
        })
        .collect::<Vec<_>>();
    ranked.sort_unstable_by(|(left_tier, left), (right_tier, right)| {
        left_tier
            .cmp(right_tier)
            .then_with(|| right.modified_at_fs.cmp(&left.modified_at_fs))
            .then_with(|| left.id.as_bytes().cmp(right.id.as_bytes()))
    });
    ranked
        .into_iter()
        .take(4096)
        .skip(offset as usize)
        .take(limit.clamp(1, 200) as usize)
        .map(|(_, result)| result)
        .collect()
}

fn assert_page_matches_oracle(db: &Database, query: &str, limit: u32, offset: u32) {
    let expected = {
        let connection = db.conn().expect("read oracle snapshot");
        oracle_search(&connection, query, limit, offset)
    };
    let direct = db
        .search_global_entries(query, limit, offset)
        .expect("run public Global Search through enabled candidate");
    assert_eq!(
        direct, expected,
        "public search query={query:?}, offset={offset}"
    );
    let actual = db
        .search_global_entries_snapshot(query, limit, offset)
        .expect("run production repository snapshot through enabled candidate");
    assert_eq!(actual.results, expected, "query={query:?}, offset={offset}");
}

fn assert_page_matches_legacy_join_oracle(db: &Database, query: &str, limit: u32, offset: u32) {
    super::super::search::with_legacy_filename_join_candidates_for_test(|| {
        assert_page_matches_oracle(db, query, limit, offset);
    });
}

#[test]
fn filename_candidate_matches_full_independent_oracle_across_tiers_and_unicode_neighbors() {
    let (db, path) = open_candidate_database();
    let second = add_volume(&db, "gv_second", r"D:\Global\");
    let entries = vec![
        fixture_entry("gv_test", r"C:\Global\foo", "foo", "", Some(40), true),
        fixture_entry(
            "gv_test",
            r"C:\Global\foobar.txt",
            "foobar.txt",
            "txt",
            Some(50),
            false,
        ),
        fixture_entry(
            "gv_test",
            r"C:\Global\x.foo",
            "x.foo",
            "foo",
            Some(80),
            false,
        ),
        fixture_entry(
            "gv_test",
            r"C:\Global\extension.foo-plus",
            "extension.foo-plus",
            "foo-plus",
            Some(70),
            false,
        ),
        fixture_entry(
            "gv_test",
            r"C:\Global\x-foo.part",
            "x-foo.part",
            "part",
            Some(60),
            false,
        ),
        fixture_entry(
            "gv_test",
            r"C:\Global\myfoobar.part",
            "myfoobar.part",
            "part",
            Some(100),
            false,
        ),
        fixture_entry(
            "gv_test",
            r"C:\Global\xfooable-foo.part",
            "xfooable-foo.part",
            "part",
            Some(110),
            false,
        ),
        fixture_entry(
            "gv_test",
            r"C:\Global\报告reporté.part",
            "报告reporté.part",
            "part",
            Some(120),
            false,
        ),
        fixture_entry(
            "gv_test",
            r"C:\Global\report_report.part",
            "report_report.part",
            "part",
            Some(130),
            false,
        ),
        fixture_entry(
            "gv_test",
            r"C:\Global\prefix-foo bar.part",
            "prefix-foo bar.part",
            "part",
            Some(135),
            false,
        ),
        fixture_entry(
            "gv_test",
            r"C:\Global\Vault\needle.txt",
            "needle.txt",
            "txt",
            Some(140),
            false,
        ),
        fixture_entry(&second.id, r"D:\Global\foo", "foo", "", Some(40), true),
        fixture_entry(
            &second.id,
            r"D:\Global\myfoobar.part",
            "myfoobar.part",
            "part",
            Some(100),
            false,
        ),
        fixture_entry(
            "gv_test",
            r"C:\Global\empty.txt",
            "empty.txt",
            "txt",
            None,
            false,
        ),
    ];
    db.upsert_global_entries_batch(&entries)
        .expect("insert filename contract fixtures");
    build_index(&db);

    assert_page_matches_oracle(&db, "foo", 80, 0);
    assert_page_matches_oracle(&db, "FOO", 80, 0);
    assert_page_matches_oracle(&db, "foo", 4, 0);
    assert_page_matches_oracle(&db, "foo", 4, 4);
    assert_page_matches_oracle(&db, "report", 80, 0);
    assert_page_matches_oracle(&db, "report", 80, 5);
    assert_page_matches_oracle(&db, "foo bar", 80, 0);
    assert_page_matches_oracle(&db, "needle", 80, 0);
    assert_page_matches_oracle(&db, "vault", 80, 0);
    assert_page_matches_oracle(&db, "absent", 80, 0);
    assert_page_matches_oracle(&db, "foo", 80, 4016);
    for (query, limit, offset) in [
        ("foo", 80, 0),
        ("FOO", 80, 0),
        ("foo", 4, 0),
        ("foo", 4, 4),
        ("report", 80, 0),
        ("report", 80, 5),
        ("foo bar", 80, 0),
        ("needle", 80, 0),
        ("vault", 80, 0),
        ("absent", 80, 0),
        ("foo", 80, 4016),
    ] {
        assert_page_matches_legacy_join_oracle(&db, query, limit, offset);
    }
    assert!(!super::super::search::filename_relevance_query_supported(
        "报告"
    ));
    assert!(!super::super::search::filename_relevance_query_supported(
        "foo-bar"
    ));

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn filename_candidate_filters_unqualified_fts_rowids_through_snapshot_membership() {
    let (db, path) = open_candidate_database();
    let mut unauthorized_volume = test_volume();
    unauthorized_volume.id = "gv_unauthorized".into();
    unauthorized_volume.stable_volume_id = "stable-unauthorized".into();
    unauthorized_volume.mount_path = r"D:\Global\".into();
    unauthorized_volume.scope_authorization_state = "not_authorized".into();
    db.upsert_global_volume(&unauthorized_volume)
        .expect("insert an enabled but unauthorized source");
    db.update_global_volume_state(
        &unauthorized_volume.id,
        INDEX_STATUS_READY,
        None,
        None,
        None,
        Some(10),
        None,
    )
    .expect("mark unauthorized source ready");

    let authorized = fixture_entry(
        "gv_test",
        r"C:\Global\Annual Report authorized.txt",
        "Annual Report authorized.txt",
        "txt",
        Some(30),
        false,
    );
    let unauthorized = fixture_entry(
        &unauthorized_volume.id,
        r"D:\Global\Annual Report private.txt",
        "Annual Report private.txt",
        "txt",
        Some(20),
        false,
    );
    let stale = fixture_entry(
        "gv_test",
        r"C:\Global\Annual Report stale.txt",
        "Annual Report stale.txt",
        "txt",
        Some(10),
        false,
    );
    db.upsert_global_entries_batch(&[authorized.clone(), unauthorized, stale.clone()])
        .expect("insert eligible, unauthorized, and stale FTS rows");
    db.conn()
        .expect("obtain hooked SQLite connection")
        .execute(
            "UPDATE global_entries SET is_stale=1 WHERE id=?1",
            [stale.entry_id()],
        )
        .expect("mark one FTS row stale before building the view");
    let diagnostics = build_index(&db);
    assert_eq!(diagnostics.index_entries, 1);

    let raw_connection = db.conn().expect("inspect raw FTS candidates");
    let mut raw_statement = raw_connection
        .prepare(super::super::search::diagnostic_filename_trigram_candidates_sql())
        .expect("prepare raw Filename Candidate FTS query");
    let raw_candidates: Vec<i64> = raw_statement
        .query_map(["{name extension} : \"report\""], |row| row.get(0))
        .expect("enumerate raw FTS rowids")
        .collect::<rusqlite::Result<_>>()
        .expect("collect raw FTS rowids");
    assert_eq!(raw_candidates.len(), 3);
    drop(raw_statement);
    drop(raw_connection);

    let before = db.filename_search.diagnostics();
    let snapshot = db
        .search_global_entries_snapshot("report", 80, 0)
        .expect("search the real product Snapshot path");
    let after = db.filename_search.diagnostics();
    assert_eq!(after.accepted_queries - before.accepted_queries, 1);
    assert_eq!(after.fallback_not_ready - before.fallback_not_ready, 0);
    assert_eq!(after.fallback_busy - before.fallback_busy, 0);
    assert_eq!(
        after.fallback_untracked_change - before.fallback_untracked_change,
        0
    );
    assert_eq!(
        after.fallback_source_change - before.fallback_source_change,
        0
    );
    assert_eq!(
        after.fallback_overlay_limit - before.fallback_overlay_limit,
        0
    );
    assert_eq!(snapshot.results.len(), 1);
    assert_eq!(snapshot.results[0].id, authorized.entry_id());

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn filename_candidate_preserves_complete_4096_candidate_window_and_deep_page() {
    let (db, path) = open_candidate_database();
    let mut entries = Vec::with_capacity(5_000);
    for index in 0..5_000_i64 {
        let modified = if index % 11 == 0 {
            None
        } else {
            Some(index % 37)
        };
        entries.push(fixture_entry(
            "gv_test",
            &format!(r"C:\Global\item-{index:05}-needle.part"),
            &format!("item-{index:05}-needle.part"),
            "part",
            modified,
            false,
        ));
    }
    db.upsert_global_entries_batch(&entries)
        .expect("insert 5,000 full-window fixtures");
    build_index(&db);

    assert_page_matches_oracle(&db, "needle", 80, 0);
    assert_page_matches_oracle(&db, "needle", 80, 4016);

    let mut changed = entries.clone();
    for (index, entry) in changed.iter_mut().enumerate() {
        entry.modified_at_fs = Some(100_000 - index as i64);
    }
    db.upsert_global_entries_batch(&changed)
        .expect("apply a single 5,000-row producer batch to the overlay");
    assert_page_matches_oracle(&db, "needle", 80, 0);
    assert_page_matches_oracle(&db, "needle", 80, 4016);

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn filename_overlay_tracks_batch_update_delete_and_rollback_without_stale_results() {
    let (db, path) = open_candidate_database();
    let original = fixture_entry(
        "gv_test",
        r"C:\Global\oldtoken.txt",
        "oldtoken.txt",
        "txt",
        Some(10),
        false,
    );
    let mut original = original;
    original.platform_file_id = "mac:dev:1:ino:22".into();
    original.parent_platform_file_id = "mac:dev:1:ino:1".into();
    let companion = fixture_entry(
        "gv_test",
        r"C:\Global\keeptoken.txt",
        "keeptoken.txt",
        "txt",
        Some(20),
        false,
    );
    db.upsert_global_entries_batch(&[original.clone(), companion.clone()])
        .expect("insert initial overlay fixtures");
    build_index(&db);

    let added = fixture_entry(
        "gv_test",
        r"C:\Global\tokenadded.txt",
        "tokenadded.txt",
        "txt",
        Some(15),
        false,
    );
    let source_fallbacks_before = db.filename_search.diagnostics().fallback_source_change;
    db.upsert_global_entries_batch(std::slice::from_ref(&added))
        .expect("insert new file after compact base build");
    assert_page_matches_oracle(&db, "added", 80, 0);
    assert_eq!(
        db.filename_search.diagnostics().fallback_source_change,
        source_fallbacks_before,
        "entry-count trigger writes must not invalidate an unchanged active-source set"
    );

    let mut renamed = original.clone();
    renamed.name = "fresh tokennew copy.txt".into();
    renamed.path = r"C:\Global\fresh tokennew copy.txt".into();
    renamed.modified_at_fs = Some(30);
    assert_eq!(renamed.entry_id(), original.entry_id());
    db.upsert_global_entries_batch(&[renamed.clone()])
        .expect("rename and update time in a single committed producer batch");
    assert_page_matches_oracle(&db, "oldtoken", 80, 0);
    assert_page_matches_oracle(&db, "tokennew", 80, 0);

    let mut connection = db.conn().expect("obtain hooked writer connection");
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .expect("begin delete rollback transaction");
    transaction
        .execute(
            "DELETE FROM global_entries WHERE id=?1",
            [companion.entry_id()],
        )
        .expect("delete inside transaction");
    transaction.rollback().expect("roll back delete");
    drop(connection);
    assert_page_matches_oracle(&db, "keeptoken", 80, 0);

    db.conn()
        .expect("obtain hooked writer connection")
        .execute(
            "DELETE FROM global_entries WHERE id=?1",
            [renamed.entry_id()],
        )
        .expect("commit deletion");
    assert_page_matches_oracle(&db, "tokennew", 80, 0);

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn separate_database_writer_and_source_revocation_fall_back_to_authoritative_sqlite() {
    let (reader, path) = open_candidate_database();
    let writer = Database::open(&path).expect("open independent connection manager");
    let first = fixture_entry(
        "gv_test",
        r"C:\Global\stable-token.txt",
        "stable-token.txt",
        "txt",
        Some(10),
        false,
    );
    reader
        .upsert_global_entries_batch(std::slice::from_ref(&first))
        .expect("insert initial base row");
    build_index(&reader);

    let external = fixture_entry(
        "gv_test",
        r"C:\Global\externaltoken.txt",
        "externaltoken.txt",
        "txt",
        Some(20),
        false,
    );
    writer
        .upsert_global_entries_batch(std::slice::from_ref(&external))
        .expect("commit cross-manager write without shared feed");
    assert_page_matches_oracle(&reader, "externaltoken", 80, 0);
    assert!(
        reader
            .filename_search
            .diagnostics()
            .fallback_untracked_change
            > 0,
        "a cross-manager commit must invalidate the compact view and use authoritative SQLite"
    );

    reader
        .set_global_volume_enabled("gv_test", false)
        .expect("revoke the source in SQLite authority");
    let snapshot = reader
        .search_global_entries_snapshot("externaltoken", 80, 0)
        .expect("snapshot falls back after source state changes");
    assert!(snapshot.results.is_empty());
    assert!(!snapshot.source_health[0].enabled);

    drop(writer);
    drop(reader);
    let _ = std::fs::remove_file(path);
}

#[test]
fn concurrent_external_rename_stays_bound_to_the_pinned_read_snapshot() {
    let (reader, path) = open_candidate_database();
    let writer = Database::open(&path).expect("open independent mutator");
    let original = fixture_entry(
        "gv_test",
        r"C:\Global\stabletoken.txt",
        "stabletoken.txt",
        "txt",
        Some(10),
        false,
    );
    let mut original = original;
    original.platform_file_id = "mac:dev:1:ino:42".into();
    original.parent_platform_file_id = "mac:dev:1:ino:1".into();
    reader
        .upsert_global_entries_batch(std::slice::from_ref(&original))
        .expect("insert snapshot fixture");
    build_index(&reader);

    let mut renamed = original.clone();
    renamed.name = "newtoken.txt".into();
    renamed.path = r"C:\Global\newtoken.txt".into();
    renamed.modified_at_fs = Some(20);
    assert_eq!(renamed.entry_id(), original.entry_id());

    let pinned_results = reader
        .filename_search
        .try_with_snapshot(|transaction, index, changes| {
            writer
                .upsert_global_entries_batch(std::slice::from_ref(&renamed))
                .expect("commit rename after the read snapshot is pinned");
            super::super::search::search_global_entries_with_filename_index_on_connection(
                transaction,
                "stabletoken",
                80,
                0,
                index,
                changes,
            )
        })
        .expect("candidate transaction itself should complete")
        .expect("candidate view remains valid for the already-pinned snapshot");
    assert_eq!(pinned_results.len(), 1);
    assert_eq!(pinned_results[0].id, original.entry_id());
    assert_eq!(pinned_results[0].name, "stabletoken.txt");

    let after_commit = reader
        .search_global_entries("stabletoken", 80, 0)
        .expect("next query detects the foreign commit and uses SQLite");
    assert!(after_commit.is_empty());
    let renamed_result = reader
        .search_global_entries("newtoken", 80, 0)
        .expect("renamed row is visible in the new authoritative snapshot");
    assert_eq!(renamed_result.len(), 1);
    assert_eq!(renamed_result[0].id, original.entry_id());
    assert_eq!(renamed_result[0].name, "newtoken.txt");

    drop(writer);
    drop(reader);
    let _ = std::fs::remove_file(path);
}

#[test]
fn filename_candidate_unready_and_unsupported_shapes_keep_the_sqlite_fallback() {
    let (db, path) = open_candidate_database();
    let entry = fixture_entry(
        "gv_test",
        r"C:\Global\Unicode-报告.txt",
        "Unicode-报告.txt",
        "txt",
        Some(10),
        false,
    );
    db.upsert_global_entries_batch(std::slice::from_ref(&entry))
        .expect("insert fallback fixture");

    assert!(crate::global_index::search::filename_relevance_query_supported("txt"));
    let before_ready_diagnostics = db.filename_search.diagnostics();
    let before_ready = db
        .search_global_entries_snapshot("txt", 80, 0)
        .expect("unready candidate uses SQLite for an exact-extension search");
    assert_eq!(before_ready.results.len(), 1);
    assert_eq!(before_ready.results[0].id, entry.entry_id());
    let after_unready_fallback = db.filename_search.diagnostics();
    assert_eq!(
        after_unready_fallback.accepted_queries, before_ready_diagnostics.accepted_queries,
        "an unready Filename Candidate must not claim the SQLite result"
    );
    assert!(
        after_unready_fallback.fallback_not_ready > before_ready_diagnostics.fallback_not_ready,
        "a supported extension query must record the not-ready SQLite fallback"
    );
    let unsupported = db
        .search_global_entries_snapshot("Unicode-报告", 80, 0)
        .expect("punctuation and Unicode query remains on existing SQLite behavior");
    assert_eq!(unsupported.results.len(), 1);
    assert_eq!(unsupported.results[0].id, entry.entry_id());

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn mutation_hooks_ignore_temp_writes_and_discard_rolled_back_rowids() {
    let (db, path) = open_candidate_database();
    let entry = fixture_entry(
        "gv_test",
        r"C:\Global\atomic-token.txt",
        "atomic-token.txt",
        "txt",
        Some(10),
        false,
    );
    db.upsert_global_entries_batch(std::slice::from_ref(&entry))
        .expect("insert tracker fixture");
    build_index(&db);
    let baseline = db.filename_search.diagnostics();

    db.conn()
        .expect("obtain hooked connection")
        .execute_batch("CREATE TEMP TABLE filename_hook_temp(value INTEGER); INSERT INTO filename_hook_temp VALUES(1);")
        .expect("commit temp-only writes without advancing main database generation");
    assert_page_matches_oracle(&db, "atomic-token", 80, 0);

    let mut connection = db.conn().expect("obtain hooked connection");
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .expect("begin rollback transaction");
    transaction
        .execute(
            "UPDATE global_entries SET name='rolled-back.txt' WHERE id=?1",
            [entry.entry_id()],
        )
        .expect("update tracked row in rollback transaction");
    transaction
        .rollback()
        .expect("roll back tracked row update");
    drop(connection);
    assert_page_matches_oracle(&db, "atomic-token", 80, 0);
    assert_page_matches_oracle(&db, "rolled-back", 80, 0);

    let after = db.filename_search.diagnostics();
    assert_eq!(after.index_entries, baseline.index_entries);
    assert_eq!(
        after.fallback_untracked_change,
        baseline.fallback_untracked_change
    );

    drop(db);
    let _ = std::fs::remove_file(path);
}
