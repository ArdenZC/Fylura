use super::*;
use crate::global_index::coverage::Decision;
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

fn claim_transient_test_operation(
    db: &Database,
    operation_id: &str,
) -> crate::global_index::repository::SourceOperationOwner {
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
    .expect("test operation owner is available")
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
    let mut included = fixture_entry(
        "gv_test",
        r"C:\Global\transient-report-budget.txt",
        "transient-report-budget.txt",
        "txt",
        Some(50),
        false,
    );
    // Model a stable native identity so the changed name reuses one catalog slot.
    included.platform_file_id = "mac:stable-transient-identity".into();
    let excluded = fixture_entry(
        "gv_test",
        r"C:\Global\ignored-transient-report.txt",
        "ignored-transient-report.txt",
        "txt",
        Some(40),
        false,
    );
    let mut chinese = fixture_entry(
        "gv_test",
        r"C:\Global\中文目录\预算报告.txt",
        "预算报告.txt",
        "txt",
        Some(41),
        false,
    );
    chinese.platform_file_id = "mac:stable-chinese-transient-identity".into();
    let case_upper = fixture_entry(
        "gv_test",
        r"C:\Global\Case\upper-variant.txt",
        "upper-variant.txt",
        "txt",
        Some(42),
        false,
    );
    let case_lower = fixture_entry(
        "gv_test",
        r"C:\Global\case\lower-variant.txt",
        "lower-variant.txt",
        "txt",
        Some(43),
        false,
    );
    db.filename_search
        .begin_transient_catalog("gv_test", "transient-operation");
    db.filename_search
        .append_transient_catalog(
            "gv_test",
            "transient-operation",
            1,
            &[
                included.clone(),
                excluded.clone(),
                chinese.clone(),
                case_upper.clone(),
                case_lower.clone(),
            ],
            &[
                Decision::Include,
                Decision::Exclude("test_scope"),
                Decision::Include,
                Decision::Include,
                Decision::Include,
            ],
        )
        .expect("append preflighted MFT entries");
    let staged = db
        .filename_search
        .transient_catalog_batch("gv_test", "transient-operation", 0, 5)
        .expect("read compact staged records");
    assert_eq!(staged.entries[0].path, included.path);
    assert_eq!(staged.entries[1].path, excluded.path);
    assert_eq!(staged.entries[2].path, chinese.path);
    assert!(
        db.filename_search
            .transient_catalog_estimated_bytes("gv_test")
            .expect("measure compact catalog")
            < crate::global_index::filename_search::MAX_TRANSIENT_CATALOG_BYTES
    );

    let snapshot = db
        .search_global_entries_snapshot("transient-report", 20, 0)
        .expect("search transient filename catalog");
    assert_eq!(snapshot.results.len(), 1);
    assert_eq!(snapshot.results[0].path, included.path);
    assert_eq!(snapshot.results[0].id, included.entry_id());
    assert!(!snapshot.index_status.collection_complete);
    assert_eq!(snapshot.index_status.processed_entries, 4);
    assert_eq!(snapshot.index_status.total_entries, 4);
    let short_prefix = db
        .search_global_entries_snapshot("tr", 20, 0)
        .expect("two-character filename prefixes use compact postings");
    assert_eq!(short_prefix.results.len(), 1);
    assert_eq!(short_prefix.results[0].path, included.path);
    let exact_name = db
        .search_global_entries_snapshot("transient-report-budget.txt", 20, 0)
        .expect("exact filename query uses compact prefix postings");
    assert_eq!(exact_name.results.len(), 1);
    assert_eq!(exact_name.results[0].path, included.path);
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
    assert_eq!(parent_path_match.results.len(), 4);
    for expected_path in [
        &included.path,
        &chinese.path,
        &case_upper.path,
        &case_lower.path,
    ] {
        assert!(parent_path_match
            .results
            .iter()
            .any(|result| &result.path == expected_path));
    }
    assert!(!parent_path_match
        .results
        .iter()
        .any(|result| result.path == excluded.path));
    let nested_path_match = db
        .search_global_entries_snapshot(r"global\transient-report-budget", 20, 0)
        .expect("query a path fragment spanning a directory and filename");
    assert_eq!(nested_path_match.results.len(), 1);
    assert_eq!(nested_path_match.results[0].path, included.path);
    let chinese_match = db
        .search_global_entries_snapshot("预算报告", 20, 0)
        .expect("short Unicode filename query searches packed names");
    assert_eq!(chinese_match.results.len(), 1);
    assert_eq!(chinese_match.results[0].path, chinese.path);
    let chinese_path_match = db
        .search_global_entries_snapshot("中文目录", 20, 0)
        .expect("Unicode parent path stays searchable");
    assert_eq!(chinese_path_match.results.len(), 1);
    assert_eq!(chinese_path_match.results[0].path, chinese.path);
    let upper_case_result = db
        .search_global_entries_snapshot("upper-variant", 20, 0)
        .expect("case-preserving directory paths remain distinct");
    assert_eq!(upper_case_result.results[0].path, case_upper.path);
    let lower_case_result = db
        .search_global_entries_snapshot("lower-variant", 20, 0)
        .expect("case-sensitive NTFS directory spelling is preserved");
    assert_eq!(lower_case_result.results[0].path, case_lower.path);
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
            &[Decision::Include],
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
    let owner = claim_transient_test_operation(&db, "managed-rebuild");

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
            &[Decision::Include],
        )
        .expect("publish the rebuilt filename row");

    let snapshot = db
        .search_global_entries_snapshot("managed-transient-budget", 10, 0)
        .expect("query the transient rebuild overlay");
    assert_eq!(snapshot.results.len(), 1);
    assert!(snapshot.results[0].managed);
    assert_eq!(snapshot.results[0].id, input.entry_id());
    assert!(!snapshot.index_status.collection_complete);

    let second = fixture_entry(
        "gv_test",
        r"C:\Global\bulk-second-transient.txt",
        "bulk-second-transient.txt",
        "txt",
        Some(49),
        false,
    );
    db.filename_search
        .append_transient_catalog(
            "gv_test",
            "managed-rebuild",
            policy_revision,
            std::slice::from_ref(&second),
            &[Decision::Include],
        )
        .expect("append a second row to exercise separate writer transactions");
    let (catalog_count, catalog_revision) = db
        .filename_search
        .complete_transient_catalog("gv_test", "managed-rebuild")
        .expect("complete compact rebuild catalogue");
    assert_eq!(catalog_count, 2);
    let (persisted, timing) = db
        .upsert_transient_mft_catalog_bulk_owned_timed(
            "gv_test",
            "managed-rebuild",
            catalog_count,
            1,
            catalog_revision.expect("catalog retained its scope revision"),
            &owner,
        )
        .expect("persist in two bounded transactions");
    assert_eq!(persisted, 2);
    assert_eq!(timing.sqlite_transactions, 3);
    let connection = db.conn().expect("open verification connection");
    let active_entries: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM global_entries WHERE volume_id='gv_test' AND is_stale=0",
            [],
            |row| row.get(0),
        )
        .expect("count active durable rows");
    assert_eq!(active_entries, 2);
    let source_entry_count: i64 = connection
        .query_row(
            "SELECT entry_count FROM global_volumes WHERE id='gv_test'",
            [],
            |row| row.get(0),
        )
        .expect("read source statistics trigger result");
    assert_eq!(source_entry_count, 2);
    connection
        .execute(
            "INSERT INTO global_entries_fts(global_entries_fts,rank) VALUES('integrity-check',1)",
            [],
        )
        .expect("FTS remains consistent after bounded writes");
    let triggers: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type='trigger' AND name IN ('global_entries_ai','global_entries_ad','global_entries_au')",
            [],
            |row| row.get(0),
        )
        .expect("read FTS trigger count");
    assert_eq!(triggers, 3);
    let managed_links: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM managed_entries WHERE global_entry_id=?1 AND enabled=1",
            [input.entry_id()],
            |row| row.get(0),
        )
        .expect("read preexisting managed association");
    assert_eq!(managed_links, 1);
    drop(connection);
    let persisted_results = db
        .search_global_entries("managed-transient-budget", 10, 0)
        .expect("search after durable FTS maintenance");
    assert!(persisted_results
        .iter()
        .any(|result| result.id == input.entry_id()));
    let second_results = db
        .search_global_entries("bulk-second-transient", 10, 0)
        .expect("search second separately committed transaction");
    assert!(second_results
        .iter()
        .any(|result| result.id == second.entry_id()));
}

#[test]
fn new_source_transient_persistence_builds_fts_and_source_facts_in_batches() {
    let (db, _path) = open_candidate_database();
    add_volume(&db, "gv_new_bulk", r"D:\NewBulk");
    let source = db
        .get_global_volume("gv_new_bulk")
        .expect("load new authorized source")
        .expect("new source exists");
    let owner = db
        .claim_global_index_operation(
            &source,
            "new-source-bulk-operation",
            crate::global_index::SourceOperationKind::Initial,
            crate::global_index::SourceOperationRoute::Service,
        )
        .expect("claim initial operation")
        .expect("new source has an available operation owner");
    let inputs = [
        fixture_entry(
            "gv_new_bulk",
            r"D:\NewBulk\newsourcebulkmarker-one.txt",
            "newsourcebulkmarker-one.txt",
            "txt",
            Some(70),
            false,
        ),
        fixture_entry(
            "gv_new_bulk",
            r"D:\NewBulk\newsourcebulkmarker-two.txt",
            "newsourcebulkmarker-two.txt",
            "txt",
            Some(80),
            false,
        ),
    ];
    let policy_revision = crate::global_index::coverage::load_policy(&db.conn().unwrap())
        .expect("read current Scope revision")
        .revision;
    db.filename_search
        .begin_transient_catalog("gv_new_bulk", &owner.operation_id);
    db.filename_search
        .append_transient_catalog(
            "gv_new_bulk",
            &owner.operation_id,
            policy_revision,
            &inputs,
            &[Decision::Include, Decision::Include],
        )
        .expect("stage the authorized source filenames");
    let (entry_count, retained_revision) = db
        .filename_search
        .complete_transient_catalog("gv_new_bulk", &owner.operation_id)
        .expect("complete transient candidate");
    let (persisted, timing) = db
        .upsert_transient_mft_catalog_bulk_owned_timed(
            "gv_new_bulk",
            &owner.operation_id,
            entry_count,
            1,
            retained_revision.expect("Scope revision remains stable"),
            &owner,
        )
        .expect("persist rows and build their FTS postings");
    assert_eq!(persisted, 2);
    assert!(timing.fts_bulk_build > std::time::Duration::ZERO);
    assert_eq!(timing.sqlite_transactions, 4);

    let connection = db.conn().expect("open durable source verification");
    let stats: (i64, i64, i64, i64) = connection
        .query_row(
            "SELECT gv.entry_count,facts.active_rows,facts.active_ids,gv.fts_bulk_build
             FROM global_volumes gv JOIN global_entry_source_facts facts ON facts.volume_id=gv.id
             WHERE gv.id='gv_new_bulk'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read per-batch source statistics");
    assert_eq!(stats, (2, 2, 2, 0));
    let mut parent_statement = connection
        .prepare(
            "SELECT parent_platform_file_id FROM global_entries
             WHERE volume_id='gv_new_bulk' ORDER BY path",
        )
        .expect("prepare persisted parent identity query");
    let persisted_parent_ids: Vec<String> = parent_statement
        .query_map([], |row| row.get(0))
        .expect("read persisted parent identities")
        .collect::<Result<_, _>>()
        .expect("collect persisted parent identities");
    assert_eq!(
        persisted_parent_ids,
        vec![
            inputs[0].parent_platform_file_id.clone(),
            inputs[1].parent_platform_file_id.clone(),
        ]
    );
    drop(parent_statement);
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM global_entries_fts
                 WHERE global_entries_fts MATCH 'newsourcebulkmarker'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("find the new source's complete FTS rows"),
        2
    );
    connection
        .execute(
            "INSERT INTO global_entries_fts(global_entries_fts,rank) VALUES('integrity-check',1)",
            [],
        )
        .expect("FTS content matches durable entries");
    drop(connection);
    db.finish_global_index_operation(&owner, INDEX_STATUS_READY, None, Some(80), None, true)
        .expect("finish initial operation after durable search is ready");
}

#[test]
fn transient_persistence_sqlite_full_restores_connection_and_recovers_on_reopen() {
    const ENTRY_COUNT: usize = 16_384;
    let (db, path) = open_candidate_database();
    let volume = add_volume(&db, "gv_disk_full", r"D:\DiskFull");
    let owner = db
        .claim_global_index_operation(
            &volume,
            "disk-full-initial-operation",
            crate::global_index::SourceOperationKind::Initial,
            crate::global_index::SourceOperationRoute::Service,
        )
        .expect("claim disk-full fixture operation")
        .expect("disk-full fixture source has no active owner");
    let inputs: Vec<_> = (0..ENTRY_COUNT)
        .map(|index| {
            let name = format!("disk-full-recovery-{index:05}.txt");
            let path = format!(r"D:\DiskFull\{name}");
            fixture_entry(
                "gv_disk_full",
                &path,
                &name,
                "txt",
                Some(index as i64),
                false,
            )
        })
        .collect();
    let policy_revision = crate::global_index::coverage::load_policy(&db.conn().unwrap())
        .expect("load current Scope revision")
        .revision;
    db.filename_search
        .begin_transient_catalog("gv_disk_full", &owner.operation_id);
    db.filename_search
        .append_transient_catalog(
            "gv_disk_full",
            &owner.operation_id,
            policy_revision,
            &inputs,
            &vec![Decision::Include; ENTRY_COUNT],
        )
        .expect("stage transient filenames before simulating a full database");
    let (entry_count, retained_revision) = db
        .filename_search
        .complete_transient_catalog("gv_disk_full", &owner.operation_id)
        .expect("complete transient catalog");

    let (page_count, previous_max_page_count, previous_cache_size, previous_journal_size_limit) = {
        let connection = db.conn().expect("read SQLite defaults before failure");
        let page_count = connection
            .query_row("PRAGMA page_count", [], |row| row.get::<_, i64>(0))
            .expect("read current SQLite page count");
        let previous_max_page_count = connection
            .query_row("PRAGMA max_page_count", [], |row| row.get::<_, i64>(0))
            .expect("read current SQLite page limit");
        let previous_cache_size = connection
            .query_row("PRAGMA cache_size", [], |row| row.get::<_, i64>(0))
            .expect("read current SQLite cache size");
        let previous_journal_size_limit = connection
            .query_row("PRAGMA journal_size_limit", [], |row| row.get::<_, i64>(0))
            .expect("read current SQLite journal limit");
        (
            page_count,
            previous_max_page_count,
            previous_cache_size,
            previous_journal_size_limit,
        )
    };
    db.for_each_test_connection(|connection| {
        connection.pragma_update(None, "max_page_count", page_count)?;
        let constrained_max_page_count =
            connection.query_row("PRAGMA max_page_count", [], |row| row.get::<_, i64>(0))?;
        assert_eq!(constrained_max_page_count, page_count);
        Ok(())
    })
    .expect("constrain every pooled SQLite connection to the current database size");

    let error = db
        .upsert_transient_mft_catalog_bulk_owned_timed(
            "gv_disk_full",
            &owner.operation_id,
            entry_count,
            8_192,
            retained_revision.expect("Scope revision remains stable"),
            &owner,
        )
        .expect_err("bounded insert must hit SQLITE_FULL");
    assert!(matches!(
        error,
        crate::db::DbError::Sqlite(rusqlite::Error::SqliteFailure(ref code, _))
            if code.code == rusqlite::ErrorCode::DiskFull
    ));

    db.for_each_test_connection(|connection| {
        let (cache_size, journal_size_limit): (i64, i64) = connection.query_row(
            "SELECT (SELECT cache_size FROM pragma_cache_size),
                    (SELECT journal_size_limit FROM pragma_journal_size_limit)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(cache_size, previous_cache_size);
        assert_eq!(journal_size_limit, previous_journal_size_limit);
        let (active_rows, fts_bulk_build): (i64, bool) = connection
            .query_row(
                "SELECT (SELECT COUNT(*) FROM global_entries WHERE volume_id='gv_disk_full'),
                        fts_bulk_build
                 FROM global_volumes WHERE id='gv_disk_full'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("failed batch left no partial durable rows");
        assert_eq!(active_rows, 0);
        assert!(fts_bulk_build, "pending FTS state is durable for recovery");
        connection.pragma_update(None, "max_page_count", previous_max_page_count)?;
        let restored_page_limit = connection
            .query_row("PRAGMA max_page_count", [], |row| row.get::<_, i64>(0))
            .expect("verify restored test database page limit");
        assert!(restored_page_limit >= page_count);
        Ok(())
    })
    .expect("inspect and restore all pooled connections after failure");

    drop(db);
    let reopened = Database::open(&path).expect("reopen and recover after SQLITE_FULL");
    let (active_rows, fts_bulk_build, fts_bulk_cursor): (i64, bool, i64) = reopened
        .conn()
        .expect("inspect recovered source")
        .query_row(
            "SELECT (SELECT COUNT(*) FROM global_entries WHERE volume_id='gv_disk_full'),
                    fts_bulk_build, fts_bulk_cursor
             FROM global_volumes WHERE id='gv_disk_full'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read recovered FTS state");
    assert_eq!(active_rows, 0);
    assert!(!fts_bulk_build);
    assert_eq!(fts_bulk_cursor, 0);
    let incomplete = reopened
        .search_global_entries_snapshot("disk-full-recovery", 10, 0)
        .expect("search incomplete source after recovery");
    assert!(incomplete.results.is_empty());
    assert!(!incomplete.index_status.collection_complete);
}

#[test]
fn pending_new_source_fts_build_recovers_on_database_reopen() {
    let (db, path) = open_candidate_database();
    let baseline = fixture_entry(
        "gv_test",
        r"C:\Global\baselineftspreservedmarker.txt",
        "baselineftspreservedmarker.txt",
        "txt",
        Some(50),
        false,
    );
    db.upsert_global_entries_batch(std::slice::from_ref(&baseline))
        .expect("persist an already-searchable source");
    add_volume(&db, "gv_bulk_recovery", r"D:\BulkRecovery");

    let pending = fixture_entry(
        "gv_bulk_recovery",
        r"D:\BulkRecovery\pendingftscursormarker.txt",
        "pendingftscursormarker.txt",
        "txt",
        Some(60),
        false,
    );
    let not_yet_indexed = fixture_entry(
        "gv_bulk_recovery",
        r"D:\BulkRecovery\laterftscursormarker.txt",
        "laterftscursormarker.txt",
        "txt",
        Some(80),
        false,
    );
    let inserted_during_build = fixture_entry(
        "gv_bulk_recovery",
        r"D:\BulkRecovery\insertedwhileftsbuildmarker.txt",
        "insertedwhileftsbuildmarker.txt",
        "txt",
        Some(90),
        false,
    );
    {
        let connection = db.conn().expect("open fixture connection");
        connection
            .execute(
                "UPDATE global_volumes SET fts_bulk_build=1, fts_bulk_cursor=0
                 WHERE id='gv_bulk_recovery'",
                [],
            )
            .expect("simulate a durable interrupted FTS build");
    }
    db.upsert_global_entries_batch(&[pending.clone(), not_yet_indexed.clone()])
        .expect("write pending-source rows while their FTS triggers are deferred");
    let mut partial_build_connection = db.conn().expect("open partial FTS writer");
    let partial_build = partial_build_connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .expect("begin one restartable FTS batch");
    let pending_rowid: i64 = partial_build
        .query_row(
            "SELECT rowid FROM global_entries WHERE id=?1",
            [pending.entry_id()],
            |row| row.get(0),
        )
        .expect("read first source rowid");
    let later_rowid: i64 = partial_build
        .query_row(
            "SELECT rowid FROM global_entries WHERE id=?1",
            [not_yet_indexed.entry_id()],
            |row| row.get(0),
        )
        .expect("read next source rowid");
    assert!(pending_rowid < later_rowid);
    partial_build
        .execute(
            "INSERT INTO global_entries_fts(rowid,name,path,extension)
             SELECT rowid,name,path,extension FROM global_entries WHERE rowid=?1",
            [pending_rowid],
        )
        .expect("persist the first bounded FTS batch");
    partial_build
        .execute(
            "UPDATE global_volumes SET fts_bulk_cursor=?1 WHERE id='gv_bulk_recovery'",
            [pending_rowid],
        )
        .expect("persist the cursor atomically with its FTS batch");
    partial_build.commit().expect("commit partial FTS progress");
    partial_build_connection
        .execute(
            "UPDATE global_entries
             SET name='recoveredftscursormarker.txt',
                 name_normalized='recoveredftscursormarker.txt',
                 path='D:\\BulkRecovery\\recoveredftscursormarker.txt',
                 path_normalized='d:/bulkRecovery/recoveredftscursormarker.txt'
             WHERE id=?1",
            [pending.entry_id()],
        )
        .expect("maintain already-indexed rows during a resumed build");
    partial_build_connection
        .execute(
            "DELETE FROM global_entries WHERE id=?1",
            [not_yet_indexed.entry_id()],
        )
        .expect("skip deleting a row not yet covered by the FTS cursor");
    drop(partial_build_connection);
    db.upsert_global_entries_batch(std::slice::from_ref(&inserted_during_build))
        .expect("concurrent incremental insert keeps source facts current");

    let connection = db
        .conn()
        .expect("open pre-recovery verification connection");
    let pending_fts_rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM global_entries_fts
             WHERE global_entries_fts MATCH 'recoveredftscursormarker'",
            [],
            |row| row.get(0),
        )
        .expect("read the not-yet-built FTS source");
    assert_eq!(
        pending_fts_rows, 1,
        "the partial FTS batch exists durably while its source is still hidden"
    );
    drop(connection);
    assert!(db
        .search_global_entries("baselineftspreservedmarker", 10, 0)
        .expect("other indexed source remains searchable")
        .iter()
        .any(|entry| entry.id == baseline.entry_id()));
    assert!(db
        .search_global_entries("recoveredftscursormarker", 10, 0)
        .expect("incomplete source stays hidden until its FTS build completes")
        .is_empty());
    drop(db);

    let reopened = Database::open(&path).expect("reopen and resume the bounded FTS build");
    let connection = reopened.conn().expect("verify recovered source state");
    let (pending_flag, cursor): (i64, i64) = connection
        .query_row(
            "SELECT fts_bulk_build, fts_bulk_cursor FROM global_volumes WHERE id='gv_bulk_recovery'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read recovered source FTS state");
    assert_eq!((pending_flag, cursor), (0, 0));
    let recovered_stats: (i64, i64, i64) = connection
        .query_row(
            "SELECT gv.entry_count, facts.active_rows, facts.active_ids
             FROM global_volumes gv JOIN global_entry_source_facts facts ON facts.volume_id=gv.id
             WHERE gv.id='gv_bulk_recovery'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read recovered source statistics");
    assert_eq!(recovered_stats, (2, 2, 2));
    connection
        .execute(
            "INSERT INTO global_entries_fts(global_entries_fts,rank) VALUES('integrity-check',1)",
            [],
        )
        .expect("recovered FTS matches the external-content table");
    drop(connection);
    assert!(reopened
        .search_global_entries("recoveredftscursormarker", 10, 0)
        .expect("search recovered durable filename")
        .iter()
        .any(|entry| entry.id == pending.entry_id()));
    assert!(reopened
        .search_global_entries("laterftscursormarker", 10, 0)
        .expect("deleted row beyond cursor stays hidden after recovery")
        .is_empty());
    assert!(reopened
        .search_global_entries("insertedwhileftsbuildmarker", 10, 0)
        .expect("incremental insert beyond cursor is included in resumed FTS")
        .iter()
        .any(|entry| entry.id == inserted_during_build.entry_id()));
    assert!(reopened
        .search_global_entries("pendingftscursormarker", 10, 0)
        .expect("renamed row's old FTS terms are removed")
        .is_empty());
    assert!(reopened
        .search_global_entries("baselineftspreservedmarker", 10, 0)
        .expect("previous source remains searchable after recovery")
        .iter()
        .any(|entry| entry.id == baseline.entry_id()));
}

#[test]
fn deleting_source_during_partial_fts_build_preserves_fts_integrity() {
    let (db, _path) = open_candidate_database();
    add_volume(&db, "gv_bulk_delete", r"E:\BulkDelete");
    let connection = db.conn().expect("open source state connection");
    connection
        .execute(
            "UPDATE global_volumes SET fts_bulk_build=1, fts_bulk_cursor=0 WHERE id=?1",
            ["gv_bulk_delete"],
        )
        .expect("simulate deferred initial-source FTS");
    drop(connection);

    let entries = [
        fixture_entry(
            "gv_bulk_delete",
            r"E:\BulkDelete\partialftsindexedmarker.txt",
            "partialftsindexedmarker.txt",
            "txt",
            Some(10),
            false,
        ),
        fixture_entry(
            "gv_bulk_delete",
            r"E:\BulkDelete\partialftsnotindexedmarker.txt",
            "partialftsnotindexedmarker.txt",
            "txt",
            Some(20),
            false,
        ),
    ];
    db.upsert_global_entries_batch(&entries)
        .expect("write rows while their FTS triggers are deferred");

    let mut connection = db.conn().expect("open partial-build connection");
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .expect("begin first FTS chunk");
    let first_rowid: i64 = transaction
        .query_row(
            "SELECT rowid FROM global_entries WHERE id=?1",
            [entries[0].entry_id()],
            |row| row.get(0),
        )
        .expect("read first source rowid");
    transaction
        .execute(
            "INSERT INTO global_entries_fts(rowid,name,path,extension)
             SELECT rowid,name,path,extension FROM global_entries WHERE rowid=?1",
            [first_rowid],
        )
        .expect("persist one FTS chunk");
    transaction
        .execute(
            "UPDATE global_volumes SET fts_bulk_cursor=?2 WHERE id=?1",
            rusqlite::params!["gv_bulk_delete", first_rowid],
        )
        .expect("persist matching cursor");
    transaction.commit().expect("commit the partial FTS state");
    connection
        .execute("DELETE FROM global_volumes WHERE id=?1", ["gv_bulk_delete"])
        .expect("cascade-delete a partially indexed source");
    connection
        .execute(
            "INSERT INTO global_entries_fts(global_entries_fts,rank) VALUES('integrity-check',1)",
            [],
        )
        .expect("cascade cleanup removes indexed rows and skips rows beyond cursor");
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM global_entries_fts
                 WHERE global_entries_fts MATCH 'partialfts'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("no removed-source posting remains"),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM global_fts_bulk_delete_guard",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("delete guard is cleaned up"),
        0
    );
}

#[test]
fn moving_unindexed_row_out_of_bulk_source_keeps_fts_exact() {
    let (db, path) = open_candidate_database();
    add_volume(&db, "gv_move_from_bulk", r"E:\MoveFromBulk");
    add_volume(&db, "gv_move_to_normal", r"F:\MoveToNormal");
    let connection = db.conn().expect("open pending-source connection");
    connection
        .execute(
            "UPDATE global_volumes SET fts_bulk_build=1, fts_bulk_cursor=0 WHERE id=?1",
            ["gv_move_from_bulk"],
        )
        .expect("mark initial source FTS as deferred");
    drop(connection);

    let original = fixture_entry(
        "gv_move_from_bulk",
        r"E:\MoveFromBulk\unindexedbeforemoveftsmarker.txt",
        "unindexedbeforemoveftsmarker.txt",
        "txt",
        Some(30),
        false,
    );
    db.upsert_global_entries_batch(std::slice::from_ref(&original))
        .expect("write row before source FTS catches up");
    let connection = db.conn().expect("move row across source boundary");
    connection
        .execute(
            "UPDATE global_entries SET
                 volume_id='gv_move_to_normal',
                 name='movedoutofbulkftsmarker.txt',
                 name_normalized='movedoutofbulkftsmarker.txt',
                 path='F:\\MoveToNormal\\movedoutofbulkftsmarker.txt',
                 path_normalized='f:/MoveToNormal/movedoutofbulkftsmarker.txt'
             WHERE id=?1",
            [original.entry_id()],
        )
        .expect("transfer an unindexed row into an FTS-ready source");
    assert_eq!(
        connection
            .query_row(
                "SELECT active_rows FROM global_entry_source_facts WHERE volume_id='gv_move_from_bulk'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("source facts decrement for the move"),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT active_rows FROM global_entry_source_facts WHERE volume_id='gv_move_to_normal'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("destination facts increment for the move"),
        1
    );
    connection
        .execute(
            "INSERT INTO global_entries_fts(global_entries_fts,rank) VALUES('integrity-check',1)",
            [],
        )
        .expect("moving a row does not delete a non-existent old posting");
    drop(connection);
    assert!(db
        .search_global_entries("movedoutofbulkftsmarker", 10, 0)
        .expect("destination source uses its current FTS posting")
        .iter()
        .any(|result| result.id == original.entry_id()));
    assert!(db
        .search_global_entries("unindexedbeforemoveftsmarker", 10, 0)
        .expect("old name is no longer indexed")
        .is_empty());
    add_volume(&db, "gv_move_to_bulk", r"G:\MoveToBulk");
    let connection = db.conn().expect("defer destination source FTS");
    connection
        .execute(
            "UPDATE global_volumes SET fts_bulk_build=1, fts_bulk_cursor=0
             WHERE id='gv_move_to_bulk'",
            [],
        )
        .expect("mark the next source as not yet searchable through FTS");
    connection
        .execute(
            "UPDATE global_entries SET
                 volume_id='gv_move_to_bulk',
                 name='movedintobulkftsmarker.txt',
                 name_normalized='movedintobulkftsmarker.txt',
                 path='G:\\MoveToBulk\\movedintobulkftsmarker.txt',
                 path_normalized='g:/MoveToBulk/movedintobulkftsmarker.txt'
             WHERE id=?1",
            [original.entry_id()],
        )
        .expect("remove the old posting without inserting into deferred FTS");
    drop(connection);
    assert!(db
        .search_global_entries("movedoutofbulkftsmarker", 10, 0)
        .expect("moved-out source no longer returns the row")
        .is_empty());
    assert!(db
        .search_global_entries("movedintobulkftsmarker", 10, 0)
        .expect("deferred destination is hidden until its FTS build finishes")
        .is_empty());
    drop(db);

    let reopened = Database::open(&path).expect("recover the source cursors");
    let flags: (i64, i64) = reopened
        .conn()
        .expect("read recovered source flags")
        .query_row(
            "SELECT
                (SELECT fts_bulk_build FROM global_volumes WHERE id='gv_move_from_bulk'),
                (SELECT fts_bulk_build FROM global_volumes WHERE id='gv_move_to_bulk')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("source recovery completed");
    assert_eq!(flags, (0, 0));
    reopened
        .conn()
        .expect("open recovered FTS integrity connection")
        .execute(
            "INSERT INTO global_entries_fts(global_entries_fts,rank) VALUES('integrity-check',1)",
            [],
        )
        .expect("both source moves have consistent FTS state after recovery");
    assert!(reopened
        .search_global_entries("movedintobulkftsmarker", 10, 0)
        .expect("row moved into deferred source is indexed during recovery")
        .iter()
        .any(|result| result.id == original.entry_id()));
    assert!(reopened
        .search_global_entries("movedoutofbulkftsmarker", 10, 0)
        .expect("intermediate source name is removed after a second move")
        .is_empty());
}

#[test]
fn revoked_source_stays_hidden_until_authorized_fts_recovery() {
    let (db, path) = open_candidate_database();
    add_volume(&db, "gv_revoked_bulk", r"G:\RevokedBulk");
    let connection = db.conn().expect("open source state connection");
    connection
        .execute(
            "UPDATE global_volumes SET fts_bulk_build=1, fts_bulk_cursor=0 WHERE id=?1",
            ["gv_revoked_bulk"],
        )
        .expect("start a deferred source FTS build");
    drop(connection);
    let entry = fixture_entry(
        "gv_revoked_bulk",
        r"G:\RevokedBulk\revokedbulksearchmarker.txt",
        "revokedbulksearchmarker.txt",
        "txt",
        Some(50),
        false,
    );
    db.upsert_global_entries_batch(std::slice::from_ref(&entry))
        .expect("persist entry without exposing a partial FTS source");
    let connection = db.conn().expect("revoke source authorization");
    connection
        .execute(
            "UPDATE global_volumes SET scope_authorization_state='not_authorized'
             WHERE id='gv_revoked_bulk'",
            [],
        )
        .expect("simulate user revoking indexing authorization");
    drop(connection);
    drop(db);

    let revoked = Database::open(&path).expect("unavailable source does not block database open");
    let state: (i64, String) = revoked
        .conn()
        .expect("read revoked source state")
        .query_row(
            "SELECT fts_bulk_build, scope_authorization_state
             FROM global_volumes WHERE id='gv_revoked_bulk'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("source remains durably pending while unauthorized");
    assert_eq!(state, (1, "not_authorized".into()));
    assert!(revoked
        .search_global_entries("revokedbulksearchmarker", 10, 0)
        .expect("revoked-source entries remain hidden")
        .is_empty());
    drop(revoked);

    let authorized_connection = rusqlite::Connection::open(&path).unwrap();
    authorized_connection
        .execute(
            "UPDATE global_volumes SET scope_authorization_state='authorized'
             WHERE id='gv_revoked_bulk'",
            [],
        )
        .expect("simulate a later explicit re-authorization");
    drop(authorized_connection);
    let recovered = Database::open(&path).expect("authorized source resumes FTS recovery");
    let state: (i64, i64) = recovered
        .conn()
        .expect("read recovered source state")
        .query_row(
            "SELECT fts_bulk_build, fts_bulk_cursor
             FROM global_volumes WHERE id='gv_revoked_bulk'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("recovery completes after authorization");
    assert_eq!(state, (0, 0));
    assert!(recovered
        .search_global_entries("revokedbulksearchmarker", 10, 0)
        .expect("reauthorized source becomes searchable")
        .iter()
        .any(|result| result.id == entry.entry_id()));
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
