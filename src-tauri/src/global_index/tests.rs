use super::*;
use crate::db::Database;
use crate::global_index::repository::{SourceOperationKind, SourceOperationRoute};
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn test_db_path() -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "zen-canvas-global-index-{}-{id}.db",
        std::process::id()
    ))
}

pub(super) fn test_volume() -> GlobalVolume {
    GlobalVolume {
        id: "gv_test".to_string(),
        platform: "windows".to_string(),
        stable_volume_id: "test-volume".to_string(),
        display_name: "Test volume".to_string(),
        mount_path: r"C:\Global\".to_string(),
        filesystem_type: "ntfs".to_string(),
        drive_kind: "fixed".to_string(),
        enabled: true,
        provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
        index_status: INDEX_STATUS_DISCOVERED.to_string(),
        last_error: None,
        journal_id: None,
        journal_cursor: None,
        last_full_index_at: None,
        last_incremental_sync_at: None,
        entry_count: 0,
        created_at: 1,
        updated_at: 1,
    }
}

fn test_entry(path: &str, name: &str, is_directory: bool) -> GlobalEntryInput {
    GlobalEntryInput {
        volume_id: "gv_test".to_string(),
        platform_file_id: format!("frn:{path}"),
        parent_platform_file_id: "frn:parent".to_string(),
        name: name.to_string(),
        path: path.to_string(),
        extension: if is_directory {
            String::new()
        } else {
            "txt".to_string()
        },
        is_directory,
        size: if is_directory { 0 } else { 1024 },
        created_at_fs: Some(10),
        modified_at_fs: Some(20),
        file_attributes: 0,
        is_hidden: false,
        is_system: false,
        source_provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
        last_seen_at: 30,
    }
}

fn seed_running_direct_operation_owner(
    db: &Database,
    volume: &GlobalVolume,
    operation_id: &str,
    owner_pid: i64,
    owner_started_at: i64,
) {
    db.conn()
        .unwrap()
        .execute(
            r#"
            UPDATE global_volumes
            SET operation_revision = 1,
                operation_owner_id = ?2,
                operation_owner_pid = ?3,
                operation_owner_started_at = ?4,
                operation_owner_route = 'direct',
                operation_kind = 'incremental',
                operation_state = 'running'
            WHERE id = ?1
            "#,
            rusqlite::params![volume.id, operation_id, owner_pid, owner_started_at],
        )
        .expect("persist running Direct operation owner");
}

#[test]
fn stable_entry_id_uses_native_identity_without_path_coupling() {
    let original = stable_entry_id(
        "gv_test",
        "mac:dev:1:ino:2",
        "mac:dev:1:ino:1",
        "note.txt",
        "/Users/test/note.txt",
    );
    let moved = stable_entry_id(
        "gv_test",
        "mac:dev:1:ino:2",
        "mac:dev:1:ino:1",
        "note.txt",
        "/Users/test/archive/note.txt",
    );
    assert_eq!(original, moved);

    let path_based = stable_entry_id(
        "gv_test",
        "path:/Users/test/note.txt",
        "path:/Users/test",
        "note.txt",
        "/Users/test/note.txt",
    );
    let path_moved = stable_entry_id(
        "gv_test",
        "path:/Users/test/note.txt",
        "path:/Users/test",
        "note.txt",
        "/Users/test/archive/note.txt",
    );
    assert_ne!(path_based, path_moved);
}

#[test]
fn mac_native_identity_survives_parent_and_name_changes() {
    let before = stable_entry_id(
        "gv_test",
        "mac:dev:1:ino:2",
        "mac:dev:1:ino:1",
        "note.txt",
        "/Users/test/note.txt",
    );
    let after = stable_entry_id(
        "gv_test",
        "mac:dev:1:ino:2",
        "mac:dev:1:ino:9",
        "renamed.txt",
        "/Users/test/archive/renamed.txt",
    );
    assert_eq!(before, after);
}

#[test]
fn migration_creates_global_and_managed_domains_separately() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open test database");
    let conn = Connection::open(&path).expect("open migrated database");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name IN ('global_volumes', 'global_entries', 'global_entries_fts', 'managed_scopes', 'managed_entries', 'ai_analysis_state', 'ai_jobs', 'ai_job_items')",
            [],
            |row| row.get(0),
        )
        .expect("global schema tables");
    assert_eq!(count, 8);
    let version: i32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("schema version");
    assert_eq!(version, 42);
    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn fresh_native_discovery_normalizes_legacy_recursive_fallback_without_erasing_error() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open test database");
    let mut legacy = test_volume();
    legacy.provider = PROVIDER_WINDOWS_RECURSIVE_FALLBACK.to_string();
    legacy.index_status = INDEX_STATUS_PERMISSION_REQUIRED.to_string();
    legacy.last_error = Some("native access denied".to_string());
    db.upsert_global_volume(&legacy)
        .expect("insert legacy provider snapshot");

    let mut discovered = legacy;
    discovered.provider = PROVIDER_WINDOWS_MFT_USN.to_string();
    db.upsert_global_volume(&discovered)
        .expect("apply fresh native discovery");

    let persisted = db
        .get_global_volume("gv_test")
        .expect("load volume")
        .expect("volume remains present");
    assert_eq!(persisted.provider, PROVIDER_WINDOWS_MFT_USN);
    assert_eq!(persisted.index_status, INDEX_STATUS_PERMISSION_REQUIRED);
    assert_eq!(
        persisted.last_error.as_deref(),
        Some("native access denied")
    );

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn global_search_is_independent_from_legacy_files_and_ai_is_scope_gated() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open test database");
    db.upsert_global_volume(&test_volume())
        .expect("insert global volume");
    let directory = test_entry(r"C:\Global\Reports", "Reports", true);
    let document = test_entry(r"C:\Global\Reports\报告.txt", "报告.txt", false);
    db.upsert_global_entries_batch(&[directory, document.clone()])
        .expect("insert global entries");

    let collecting_status = db.global_index_status().expect("collecting status");
    assert_eq!(collecting_status.processed_entries, 2);
    assert!(!collecting_status.collection_complete);

    let results = db
        .search_global_entries("报告", 20, 0)
        .expect("global search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].path, document.path);
    assert!(!results[0].managed);

    let conn = Connection::open(&path).expect("inspect isolated tables");
    let legacy_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
        .expect("legacy files count");
    assert_eq!(legacy_count, 0);
    drop(conn);

    let scope = db
        .add_managed_scope(AddManagedScopeRequest {
            path: r"C:\Global\Reports".to_string(),
            global_entry_id: None,
            enabled: true,
            allow_local_ai: true,
            allow_cloud_ai: false,
        })
        .expect("add managed scope");
    assert!(scope.enabled);
    let managed_results = db
        .search_global_entries("报告", 20, 0)
        .expect("managed global search");
    assert!(managed_results[0].managed);

    let conn = Connection::open(&path).expect("inspect managed tables");
    let managed_entries: i64 = conn
        .query_row("SELECT COUNT(*) FROM managed_entries", [], |row| row.get(0))
        .expect("managed entry count");
    let ai_jobs: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_jobs", [], |row| row.get(0))
        .expect("AI job count");
    let ai_states: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_analysis_state", [], |row| {
            row.get(0)
        })
        .expect("AI analysis state count");
    assert_eq!(managed_entries, 2);
    assert_eq!(ai_jobs, 1);
    assert_eq!(ai_states, 1);

    db.update_global_volume_state(
        "gv_test",
        INDEX_STATUS_READY,
        None,
        None,
        None,
        Some(40),
        Some(40),
    )
    .expect("mark volume ready");
    let completed_status = db.global_index_status().expect("completed status");
    assert_eq!(completed_status.status, INDEX_STATUS_READY);
    assert!(completed_status.collection_complete);
    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn global_search_ranking_is_exact_then_prefix_then_extension_with_stable_id_ties() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open test database");
    db.upsert_global_volume(&test_volume())
        .expect("insert global volume");

    let mut exact = test_entry(r"C:\Global\r", "r", false);
    exact.platform_file_id = "identity-exact".to_string();
    exact.modified_at_fs = Some(20);
    let mut prefix_b = test_entry(r"C:\Global\report-b.txt", "report-b.txt", false);
    prefix_b.platform_file_id = "identity-prefix-b".to_string();
    prefix_b.modified_at_fs = Some(10);
    let mut prefix_a = test_entry(r"C:\Global\report-a.txt", "report-a.txt", false);
    prefix_a.platform_file_id = "identity-prefix-a".to_string();
    prefix_a.modified_at_fs = Some(10);
    let prefix_a_id = prefix_a.entry_id();
    let prefix_b_id = prefix_b.entry_id();
    let mut extension = test_entry(r"C:\Global\archive.r", "archive.r", false);
    extension.platform_file_id = "identity-extension".to_string();
    extension.extension = "r".to_string();
    extension.modified_at_fs = Some(9);
    let mut extension_prefix = test_entry(r"C:\Global\archive.rust", "archive.rust", false);
    extension_prefix.platform_file_id = "identity-extension-prefix".to_string();
    extension_prefix.extension = "rust".to_string();
    extension_prefix.modified_at_fs = Some(8);

    db.upsert_global_entries_batch(&[prefix_b, extension, exact, prefix_a, extension_prefix])
        .expect("insert ranked entries");
    let results = db.search_global_entries("r", 20, 0).expect("ranked search");
    assert_eq!(results.len(), 5);
    assert_eq!(results[0].name, "r");
    let mut expected_prefix_ids = vec![prefix_a_id, prefix_b_id];
    expected_prefix_ids.sort();
    assert_eq!(
        results[1..3]
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>(),
        expected_prefix_ids
    );
    assert_eq!(results[3].name, "archive.r");
    assert_eq!(results[4].name, "archive.rust");
    let result_ids = results
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        result_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        result_ids.len(),
        "layer union must not duplicate an entry"
    );
    let limited = db
        .search_global_entries("r", 3, 0)
        .expect("bounded ranked search");
    assert_eq!(
        limited
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["r", results[1].name.as_str(), results[2].name.as_str()]
    );
    let page_after_prefix = db
        .search_global_entries("r", 20, 2)
        .expect("offset in deduplicated tier stream");
    assert_eq!(
        page_after_prefix
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        results[2..]
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>()
    );
    let prefix_results = db
        .search_global_entries("rep", 20, 0)
        .expect("prefix search");
    assert_eq!(prefix_results.len(), 2);
    assert!(prefix_results
        .iter()
        .all(|entry| entry.name.starts_with("report-")));
    let extension_results = db
        .search_global_entries("r", 20, 0)
        .expect("extension search");
    assert_eq!(extension_results[3].name, "archive.r");
    assert_eq!(extension_results[4].name, "archive.rust");
    let repeated_ids = db
        .search_global_entries("rep", 20, 0)
        .expect("repeat ranked search")
        .into_iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    assert_eq!(
        prefix_results
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        repeated_ids
    );

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn extension_tiers_order_by_durable_id_across_reinsertion_and_pages() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open test database");
    db.upsert_global_volume(&test_volume())
        .expect("insert global volume");

    let mut exact_low = test_entry(r"C:\Global\exact-low.bin", "exact-low.bin", false);
    exact_low.platform_file_id = "extension-exact-low".to_string();
    exact_low.extension = "same".to_string();
    exact_low.modified_at_fs = Some(10);
    let mut exact_high = test_entry(r"C:\Global\exact-high.bin", "exact-high.bin", false);
    exact_high.platform_file_id = "extension-exact-high".to_string();
    exact_high.extension = "same".to_string();
    exact_high.modified_at_fs = Some(10);
    let mut prefix_low = test_entry(
        r"C:\Global\prefix-low.same-long",
        "prefix-low.same-long",
        false,
    );
    prefix_low.platform_file_id = "extension-prefix-low".to_string();
    prefix_low.extension = "same-long".to_string();
    prefix_low.modified_at_fs = Some(10);
    let mut prefix_high = test_entry(
        r"C:\Global\prefix-high.same-wide",
        "prefix-high.same-wide",
        false,
    );
    prefix_high.platform_file_id = "extension-prefix-high".to_string();
    prefix_high.extension = "same-wide".to_string();
    prefix_high.modified_at_fs = Some(10);

    let mut expected_exact = vec![exact_low.entry_id(), exact_high.entry_id()];
    expected_exact.sort();
    let mut expected_prefix = vec![prefix_low.entry_id(), prefix_high.entry_id()];
    expected_prefix.sort();
    db.upsert_global_entries_batch(&[
        prefix_high.clone(),
        exact_high.clone(),
        prefix_low.clone(),
        exact_low.clone(),
    ])
    .expect("insert reverse-order extension entries");

    let full = db
        .search_global_entries("same", 20, 0)
        .expect("search exact and prefix extension tiers");
    let ids = full
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(&ids[..2], expected_exact.as_slice());
    assert_eq!(&ids[2..], expected_prefix.as_slice());
    assert_eq!(
        db.search_global_entries("same", 20, 0)
            .expect("repeat extension search")
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        ids
    );

    let paged = db
        .search_global_entries("same", 1, 1)
        .expect("page extension search");
    assert_eq!(paged[0].id, ids[1]);

    {
        let conn = db.conn().expect("open direct test connection");
        conn.execute(
            "DELETE FROM global_entries WHERE id = ?1",
            rusqlite::params![ids[0]],
        )
        .expect("delete durable entry");
    }
    let removed = if ids[0] == exact_low.entry_id() {
        exact_low.clone()
    } else {
        exact_high.clone()
    };
    db.upsert_global_entries_batch(std::slice::from_ref(&removed))
        .expect("reinsert durable entry");
    let after_reinsert = db
        .search_global_entries("same", 20, 0)
        .expect("search after reinsert");
    assert_eq!(
        after_reinsert
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>(),
        ids,
        "reinserted rows retain durable-ID ordering"
    );

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn punctuation_search_preserves_literal_prefix_semantics_and_glob_escaping() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open test database");
    db.upsert_global_volume(&test_volume())
        .expect("insert global volume");
    let names = [
        ".gitignore",
        ".gitignore.local",
        "gitignore",
        "C++",
        "C++-guide",
        "C",
        "report!",
        "report-final",
        "[name]",
        "[name]-backup",
        "file*",
        "file-star",
        "what?",
        "what-question",
        "ordinary-report",
    ];
    let entries = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let mut entry = test_entry(&format!(r"C:\Global\{name}"), name, false);
            entry.platform_file_id = format!("punctuation-{index}");
            entry.modified_at_fs = Some(10);
            entry
        })
        .collect::<Vec<_>>();
    db.upsert_global_entries_batch(&entries)
        .expect("insert punctuation entries");

    for (query, expected) in [
        (".gitignore", [".gitignore", ".gitignore.local"].as_slice()),
        ("C++", ["C++", "C++-guide"].as_slice()),
        ("report!", ["report!"].as_slice()),
        ("[name]", ["[name]", "[name]-backup"].as_slice()),
        ("file*", ["file*"].as_slice()),
        ("what?", ["what?"].as_slice()),
    ] {
        let results = db
            .search_global_entries(query, 20, 0)
            .expect("literal punctuation search");
        let names = results
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, expected, "query {query:?} must remain literal");
        assert_eq!(
            db.search_global_entries(query, 20, 0)
                .expect("repeat literal punctuation search")
                .into_iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            results
                .into_iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>()
        );
    }
    assert!(
        db.search_global_entries("!!!", 20, 0)
            .expect("punctuation-only search")
            .is_empty(),
        "punctuation-only queries must not widen to the full index"
    );

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn global_search_fts_layer_fills_remaining_page_and_offset_is_stable() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open test database");
    db.upsert_global_volume(&test_volume())
        .expect("insert global volume");

    let mut name_match = test_entry(r"C:\Global\archive-needle.txt", "archive-needle.txt", false);
    name_match.platform_file_id = "identity-fts-name".to_string();
    name_match.modified_at_fs = Some(20);
    let mut path_match = test_entry(r"C:\Global\needle\deep.log", "deep.log", false);
    path_match.platform_file_id = "identity-fts-path".to_string();
    path_match.extension = "log".to_string();
    path_match.modified_at_fs = Some(10);
    db.upsert_global_entries_batch(&[name_match, path_match])
        .expect("insert FTS entries");

    let full = db
        .search_global_entries("needle", 20, 0)
        .expect("FTS search");
    assert_eq!(full.len(), 2);
    let full_ids = full
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<Vec<_>>();

    let first_page = db
        .search_global_entries("needle", 1, 0)
        .expect("first FTS page");
    let second_page = db
        .search_global_entries("needle", 1, 1)
        .expect("second FTS page");
    let paged_ids = first_page
        .iter()
        .chain(second_page.iter())
        .map(|entry| entry.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(paged_ids, full_ids);
    assert_ne!(first_page[0].id, second_page[0].id);

    let repeated_ids = db
        .search_global_entries("needle", 20, 0)
        .expect("repeat FTS search")
        .into_iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    assert_eq!(repeated_ids, full_ids);

    assert!(db
        .search_global_entries("zzznomatchtoken", 20, 0)
        .expect("zero-match FTS search")
        .is_empty());

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn global_search_high_hit_fts_candidate_preserves_filters_ties_and_snapshot() {
    use crate::global_index::search::{
        diagnostic_search_fts, with_key_only_fts_search_candidate, with_legacy_fts_search_candidate,
    };

    let path = test_db_path();
    let db = Database::open(&path).expect("open test database");
    db.upsert_global_volume(&test_volume())
        .expect("insert primary enabled volume");

    for (id, stable_id, enabled) in [
        ("gv_fts_second", "fts-second-volume", true),
        ("gv_fts_stale", "fts-stale-volume", true),
        ("gv_fts_disabled", "fts-disabled-volume", false),
    ] {
        let mut volume = test_volume();
        volume.id = id.to_string();
        volume.stable_volume_id = stable_id.to_string();
        volume.enabled = enabled;
        db.upsert_global_volume(&volume)
            .expect("insert FTS comparison volume");
    }

    let mut active_a = test_entry(r"C:\Global\Report\alpha.txt", "alpha.txt", false);
    active_a.platform_file_id = "fts-active-a".to_string();
    active_a.modified_at_fs = Some(10);
    let mut active_b = active_a.clone();
    active_b.volume_id = "gv_fts_second".to_string();
    active_b.platform_file_id = "fts-active-b".to_string();
    let mut active_c = test_entry(r"C:\Global\Report\beta.txt", "beta.txt", false);
    active_c.platform_file_id = "fts-active-c".to_string();
    active_c.modified_at_fs = Some(9);
    let mut stale = active_a.clone();
    stale.volume_id = "gv_fts_stale".to_string();
    stale.platform_file_id = "fts-stale".to_string();
    stale.modified_at_fs = Some(30);
    let mut disabled = active_a.clone();
    disabled.volume_id = "gv_fts_disabled".to_string();
    disabled.platform_file_id = "fts-disabled".to_string();
    disabled.modified_at_fs = Some(40);

    db.upsert_global_entries_batch(&[active_a, active_b, active_c, stale, disabled])
        .expect("insert same-rank FTS comparison rows");
    db.conn()
        .expect("get connection to mark stale FTS row")
        .execute(
            "UPDATE global_entries SET is_stale = 1 WHERE volume_id = ?1 AND platform_file_id = ?2",
            rusqlite::params!["gv_fts_stale", "fts-stale"],
        )
        .expect("mark stale FTS row");

    let conn = db.conn().expect("get direct FTS test connection");
    let legacy_tier =
        with_legacy_fts_search_candidate(|| diagnostic_search_fts(&conn, "report", 2))
            .expect("run legacy FTS tier");
    let candidate_tier =
        with_key_only_fts_search_candidate(|| diagnostic_search_fts(&conn, "report", 2))
            .expect("run key-only FTS tier");
    assert_eq!(candidate_tier, legacy_tier);
    assert_eq!(legacy_tier.len(), 2, "filters must precede the limit");
    assert!(legacy_tier
        .iter()
        .all(|result| result.volume_id == "gv_test" || result.volume_id == "gv_fts_second"));

    let legacy_ties = with_legacy_fts_search_candidate(|| diagnostic_search_fts(&conn, "alpha", 2))
        .expect("run legacy equal-rank FTS tie query");
    assert_eq!(legacy_ties.len(), 2);

    let mut tie_statement = conn
        .prepare(
            "SELECT id FROM global_entries WHERE name_normalized = 'alpha.txt' AND volume_id IN ('gv_test', 'gv_fts_second') AND is_stale = 0 ORDER BY id ASC",
        )
        .expect("prepare enabled FTS tie ID query");
    let expected_tie_ids = tie_statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query enabled FTS tie IDs")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect enabled FTS tie IDs");
    drop(tie_statement);
    assert_eq!(expected_tie_ids.len(), 2);
    assert_eq!(
        legacy_ties
            .iter()
            .map(|result| result.id.clone())
            .collect::<Vec<_>>(),
        expected_tie_ids,
        "equal BM25 and mtime FTS results must retain ascending ID order"
    );

    let legacy_full = with_legacy_fts_search_candidate(|| db.search_global_entries("report", 2, 0))
        .expect("run legacy full search");
    let candidate_full =
        with_key_only_fts_search_candidate(|| db.search_global_entries("report", 2, 0))
            .expect("run key-only full search");
    assert_eq!(candidate_full, legacy_full);
    assert_eq!(
        with_legacy_fts_search_candidate(|| db.search_global_entries("report", 1, 1))
            .expect("legacy paged FTS search"),
        with_key_only_fts_search_candidate(|| db.search_global_entries("report", 1, 1))
            .expect("key-only paged FTS search")
    );

    let legacy_snapshot =
        with_legacy_fts_search_candidate(|| db.search_global_entries_snapshot("report", 2, 0))
            .expect("run legacy FTS repository snapshot");
    let candidate_snapshot =
        with_key_only_fts_search_candidate(|| db.search_global_entries_snapshot("report", 2, 0))
            .expect("run key-only FTS repository snapshot");
    assert_eq!(candidate_snapshot.results, legacy_snapshot.results);
    assert_eq!(
        candidate_snapshot.source_health,
        legacy_snapshot.source_health
    );
    assert_eq!(
        candidate_snapshot.source_revision,
        legacy_snapshot.source_revision
    );
    assert_eq!(
        candidate_snapshot.index_status,
        legacy_snapshot.index_status
    );

    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn global_search_snapshot_filters_disabled_sources_and_tracks_entry_revision() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open test database");
    db.upsert_global_volume(&test_volume())
        .expect("insert global volume");
    db.update_global_volume_state(
        "gv_test",
        INDEX_STATUS_READY,
        None,
        None,
        None,
        None,
        Some(31),
    )
    .expect("mark source ready");
    let document = test_entry(r"C:\Global\snapshot-note.txt", "snapshot-note.txt", false);
    let document_id = document.entry_id();
    db.upsert_global_entries_batch(std::slice::from_ref(&document))
        .expect("insert snapshot entry");

    let ready = db
        .search_global_entries_snapshot("snapshot", 20, 0)
        .expect("read consistent search snapshot");
    assert_eq!(ready.results.len(), 1);
    assert_eq!(ready.source_health.len(), 1);
    assert!(ready.source_health[0].enabled);
    assert_eq!(ready.source_health[0].status, INDEX_STATUS_READY);
    assert!(ready.index_status.collection_complete);
    assert!(!ready.source_revision.is_empty());

    db.set_global_volume_enabled("gv_test", false)
        .expect("disable source");
    let disabled = db
        .search_global_entries_snapshot("snapshot", 20, 0)
        .expect("read disabled source snapshot");
    assert!(disabled.results.is_empty());
    assert!(!disabled.source_health[0].enabled);
    assert_eq!(disabled.index_status.status, INDEX_STATUS_UNAVAILABLE);
    assert_ne!(ready.source_revision, disabled.source_revision);

    db.set_global_volume_enabled("gv_test", true)
        .expect("re-enable source");
    db.mark_global_entry_stale(&document_id)
        .expect("mark entry stale");
    let stale = db
        .search_global_entries_snapshot("snapshot", 20, 0)
        .expect("read stale entry snapshot");
    assert!(stale.results.is_empty());
    assert_ne!(disabled.source_revision, stale.source_revision);

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn global_search_snapshot_remains_source_consistent_during_status_rebuild_changes() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open test database");
    db.upsert_global_volume(&test_volume())
        .expect("insert global volume");
    db.update_global_volume_state(
        "gv_test",
        INDEX_STATUS_READY,
        None,
        None,
        None,
        None,
        Some(31),
    )
    .expect("mark source ready");
    let document = test_entry(
        r"C:\Global\concurrent-note.txt",
        "concurrent-note.txt",
        false,
    );
    db.upsert_global_entries_batch(std::slice::from_ref(&document))
        .expect("insert concurrent entry");

    let start = Arc::new(Barrier::new(2));
    let worker_db = db.clone();
    let worker_start = Arc::clone(&start);
    let worker = std::thread::spawn(move || {
        worker_start.wait();
        for index in 0..24 {
            let enabled = index % 2 == 0;
            worker_db
                .set_global_volume_enabled("gv_test", enabled)
                .expect("toggle source");
            worker_db
                .update_global_volume_state(
                    "gv_test",
                    if enabled {
                        INDEX_STATUS_READY
                    } else {
                        INDEX_STATUS_REBUILD_REQUIRED
                    },
                    None,
                    None,
                    None,
                    None,
                    Some(40 + index),
                )
                .expect("update source status");
        }
    });
    start.wait();
    for _ in 0..24 {
        let snapshot = db
            .search_global_entries_snapshot("concurrent", 20, 0)
            .expect("read concurrent snapshot");
        let source = &snapshot.source_health[0];
        assert!(snapshot
            .results
            .iter()
            .all(|result| result.volume_id == source.source_id && source.enabled));
        if snapshot.index_status.collection_complete {
            assert!(source.enabled);
            assert_eq!(source.status, INDEX_STATUS_READY);
        }
        assert!(!snapshot.source_revision.is_empty());
    }
    worker.join().expect("status worker");

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn disabled_ai_policy_creates_no_jobs_without_removing_global_search() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open test database");
    db.upsert_global_volume(&test_volume())
        .expect("insert global volume");
    let document = test_entry(r"C:\Global\Private\secret.txt", "secret.txt", false);
    db.upsert_global_entries_batch(std::slice::from_ref(&document))
        .expect("insert global document");
    db.add_managed_scope(AddManagedScopeRequest {
        path: r"C:\Global\Private".to_string(),
        global_entry_id: None,
        enabled: true,
        allow_local_ai: false,
        allow_cloud_ai: false,
    })
    .expect("add blocked scope");

    let results = db
        .search_global_entries("secret", 20, 0)
        .expect("global search remains available");
    assert_eq!(results.len(), 1);
    assert!(results[0].managed);
    let conn = Connection::open(&path).expect("inspect disabled AI policy");
    let job_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_jobs", [], |row| row.get(0))
        .expect("AI job count");
    assert_eq!(job_count, 0);
    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn managed_ai_queue_and_scope_eligibility_changes_wake_the_worker() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open test database");
    db.upsert_global_volume(&test_volume())
        .expect("insert global volume");
    let _scope = db
        .add_managed_scope(AddManagedScopeRequest {
            path: r"C:\Global\Managed".to_string(),
            global_entry_id: None,
            enabled: true,
            allow_local_ai: true,
            allow_cloud_ai: false,
        })
        .expect("add managed scope");
    let (wake_tx, wake_rx) = std::sync::mpsc::sync_channel(1);
    db.set_managed_ai_waker(wake_tx);
    db.set_managed_ai_work_wakes_armed(true);

    let indexed = test_entry(r"C:\Global\Managed\indexed.txt", "indexed.txt", false);
    db.upsert_global_entries_batch(std::slice::from_ref(&indexed))
        .expect("index entry and durably enqueue AI work");
    let pending_jobs: i64 = db
        .conn()
        .expect("database connection")
        .query_row(
            "SELECT COUNT(*) FROM ai_jobs WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )
        .expect("read durable pending jobs");
    assert_eq!(pending_jobs, 1);
    assert_eq!(wake_rx.try_recv(), Ok(()));

    let backfilled = test_entry(r"C:\Global\Backfill\saved.txt", "saved.txt", false);
    db.upsert_global_entries_batch(std::slice::from_ref(&backfilled))
        .expect("index entry outside managed scope");
    assert_eq!(
        wake_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty),
        "an entry without a matching scope must not wake the worker"
    );
    let backfill_scope = db
        .add_managed_scope(AddManagedScopeRequest {
            path: r"C:\Global\Backfill".to_string(),
            global_entry_id: None,
            enabled: true,
            allow_local_ai: true,
            allow_cloud_ai: false,
        })
        .expect("backfill new managed scope");
    assert_eq!(wake_rx.try_recv(), Ok(()));

    db.update_managed_scope_policy(UpdateManagedScopePolicyRequest {
        id: backfill_scope.id.clone(),
        enabled: Some(false),
        allow_local_ai: None,
        allow_cloud_ai: None,
    })
    .expect("disable managed scope policy");
    assert_eq!(wake_rx.try_recv(), Ok(()));
    db.update_managed_scope_policy(UpdateManagedScopePolicyRequest {
        id: backfill_scope.id.clone(),
        enabled: Some(true),
        allow_local_ai: None,
        allow_cloud_ai: None,
    })
    .expect("re-enable managed scope policy");
    assert_eq!(wake_rx.try_recv(), Ok(()));

    let reenabled_status: String = db
        .conn()
        .expect("database connection")
        .query_row(
            "SELECT status FROM ai_jobs WHERE managed_scope_id = ?1 ORDER BY created_at DESC LIMIT 1",
            [&backfill_scope.id],
            |row| row.get(0),
        )
        .expect("read re-enabled durable job status");
    assert_eq!(reenabled_status, "pending");
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn global_index_batch_wakes_only_for_new_or_reactivated_eligible_work() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open test database");
    db.upsert_global_volume(&test_volume())
        .expect("insert global volume");
    let _scope = db
        .add_managed_scope(AddManagedScopeRequest {
            path: r"C:\Global\Eligible".to_string(),
            global_entry_id: None,
            enabled: true,
            allow_local_ai: true,
            allow_cloud_ai: false,
        })
        .expect("add eligible scope");
    let _blocked_scope = db
        .add_managed_scope(AddManagedScopeRequest {
            path: r"C:\Global\Blocked".to_string(),
            global_entry_id: None,
            enabled: true,
            allow_local_ai: false,
            allow_cloud_ai: false,
        })
        .expect("add policy-blocked scope");
    let (wake_tx, wake_rx) = std::sync::mpsc::sync_channel(1);
    db.set_managed_ai_waker(wake_tx);
    db.set_managed_ai_work_wakes_armed(true);

    let outside_scope = test_entry(r"C:\Global\Outside\note.txt", "note.txt", false);
    db.upsert_global_entries_batch(std::slice::from_ref(&outside_scope))
        .expect("index entry without a matching scope");
    let directory = test_entry(r"C:\Global\Eligible\folder", "folder", true);
    db.upsert_global_entries_batch(std::slice::from_ref(&directory))
        .expect("index managed directory");
    let blocked = test_entry(r"C:\Global\Blocked\blocked.txt", "blocked.txt", false);
    db.upsert_global_entries_batch(std::slice::from_ref(&blocked))
        .expect("index policy-blocked entry");
    assert_eq!(
        wake_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty),
        "no-scope, directory, and blocked_by_policy entries must not wake the worker"
    );

    let eligible = test_entry(r"C:\Global\Eligible\work.txt", "work.txt", false);
    db.upsert_global_entries_batch(std::slice::from_ref(&eligible))
        .expect("create eligible pending work");
    assert_eq!(wake_rx.try_recv(), Ok(()));

    db.upsert_global_entries_batch(std::slice::from_ref(&eligible))
        .expect("re-upsert unchanged pending input");
    assert_eq!(
        wake_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty),
        "an existing current job must not produce another work wake"
    );

    let entry_id = eligible.entry_id();
    db.conn()
        .expect("database connection")
        .execute(
            "UPDATE ai_jobs SET status = 'stale' WHERE global_entry_id = ?1",
            [&entry_id],
        )
        .expect("mark current job stale");
    db.upsert_global_entries_batch(std::slice::from_ref(&eligible))
        .expect("reactivate stale eligible work");
    assert_eq!(wake_rx.try_recv(), Ok(()));

    db.conn()
        .expect("database connection")
        .execute(
            "UPDATE ai_jobs SET status = 'completed' WHERE global_entry_id = ?1",
            [&entry_id],
        )
        .expect("mark current job completed");
    db.upsert_global_entries_batch(std::slice::from_ref(&eligible))
        .expect("re-upsert completed input");
    assert_eq!(
        wake_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty),
        "an existing completed result must not produce another work wake"
    );

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn rebuild_intent_coalesces_without_changing_committed_source_health_or_entries() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open rebuild-intent database");
    let mut volume = test_volume();
    volume.index_status = INDEX_STATUS_READY.to_string();
    volume.journal_id = Some("journal-7".into());
    volume.journal_cursor = Some("cursor-42".into());
    volume.last_full_index_at = Some(100);
    volume.last_incremental_sync_at = Some(200);
    volume.entry_count = 1;
    db.upsert_global_volume(&volume).expect("seed source");
    let entry = test_entry(r"C:\Global\kept.txt", "kept.txt", false);
    db.upsert_global_entries_batch(std::slice::from_ref(&entry))
        .expect("seed active global entry");
    let before = db.get_global_volume(&volume.id).unwrap().unwrap();

    assert!(db.request_global_index_rebuild(&volume.id).unwrap());
    assert!(db.request_global_index_rebuild(&volume.id).unwrap());

    let after = db.get_global_volume(&volume.id).unwrap().unwrap();
    let operation = db
        .global_index_operation_state(&volume.id)
        .unwrap()
        .unwrap();
    let stale: i64 = db
        .conn()
        .unwrap()
        .query_row(
            "SELECT is_stale FROM global_entries WHERE id = ?1",
            [entry.entry_id()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(operation.rebuild_intent_revision, 1);
    assert_eq!(operation.rebuild_completed_revision, 0);
    assert!(operation.rebuild_pending());
    assert_eq!(after.index_status, before.index_status);
    assert_eq!(after.last_error, before.last_error);
    assert_eq!(after.journal_id, before.journal_id);
    assert_eq!(after.journal_cursor, before.journal_cursor);
    assert_eq!(after.last_full_index_at, before.last_full_index_at);
    assert_eq!(
        after.last_incremental_sync_at,
        before.last_incremental_sync_at
    );
    assert_eq!(after.entry_count, before.entry_count);
    assert_eq!(stale, 0);
}

#[test]
fn source_revision_fences_late_stale_and_checkpoint_writes_after_disable() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open source-revision database");
    let mut volume = test_volume();
    volume.index_status = INDEX_STATUS_READY.to_string();
    db.upsert_global_volume(&volume).expect("seed source");
    let entry = test_entry(r"C:\Global\late.txt", "late.txt", false);
    db.upsert_global_entries_batch(std::slice::from_ref(&entry))
        .expect("seed active global entry");
    let owner = db
        .claim_global_index_operation(
            &volume,
            "service-operation-before-disable",
            SourceOperationKind::Rebuild,
            SourceOperationRoute::Service,
        )
        .unwrap()
        .expect("admit source operation");

    db.set_global_volume_enabled(&volume.id, false)
        .expect("disable source and invalidate old owner");
    db.forget_local_global_index_operation(&owner);

    assert!(!db
        .update_global_volume_checkpoint_owned(&owner, Some("late-journal"), Some("late-cursor"))
        .unwrap());
    assert!(db
        .mark_global_entries_stale_for_volume_owned(&owner)
        .is_err());
    assert!(db
        .upsert_global_entries_batch_owned(
            &[test_entry(
                r"C:\Global\late-write.txt",
                "late-write.txt",
                false,
            )],
            &owner
        )
        .is_err());

    let current = db.get_global_volume(&volume.id).unwrap().unwrap();
    let operation = db
        .global_index_operation_state(&volume.id)
        .unwrap()
        .unwrap();
    let stale: i64 = db
        .conn()
        .unwrap()
        .query_row(
            "SELECT is_stale FROM global_entries WHERE id = ?1",
            [entry.entry_id()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!current.enabled);
    assert_eq!(current.journal_id, None);
    assert_eq!(current.journal_cursor, None);
    assert!(operation.revision > owner.revision);
    assert_eq!(operation.owner_id, None);
    assert_eq!(stale, 0);
}

#[test]
fn unknown_service_owner_cannot_be_replayed_by_direct_after_restart() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open operation-recovery database");
    let mut volume = test_volume();
    volume.index_status = INDEX_STATUS_READY.to_string();
    db.upsert_global_volume(&volume).expect("seed source");
    let admitted = db
        .claim_global_index_operation(
            &volume,
            "admitted-service-request",
            SourceOperationKind::Incremental,
            SourceOperationRoute::Service,
        )
        .unwrap()
        .expect("claim admitted Service request");
    assert!(db.mark_global_index_operation_unknown(&admitted).unwrap());

    let direct_replay = db
        .claim_global_index_operation(
            &volume,
            "direct-replay-after-restart",
            SourceOperationKind::Incremental,
            SourceOperationRoute::Direct,
        )
        .unwrap();
    let unknown_state = db
        .global_index_operation_state(&volume.id)
        .unwrap()
        .unwrap();
    assert!(direct_replay.is_none());
    assert_eq!(
        unknown_state.owner_id.as_deref(),
        Some("admitted-service-request")
    );
    assert_eq!(unknown_state.operation_state.as_deref(), Some("unknown"));

    let reconciled = db
        .claim_global_index_operation(
            &volume,
            "newly-admitted-service-request",
            SourceOperationKind::Incremental,
            SourceOperationRoute::Service,
        )
        .unwrap()
        .expect("a new Service admission reconciles the previous unknown owner");
    assert!(reconciled.revision > admitted.revision);
    assert!(db
        .finish_global_index_operation(
            &reconciled,
            INDEX_STATUS_READY,
            None,
            None,
            Some(300),
            false
        )
        .unwrap());
    let completed = db
        .global_index_operation_state(&volume.id)
        .unwrap()
        .unwrap();
    assert_eq!(completed.owner_id, None);
    assert_eq!(completed.operation_state, None);
}

#[test]
fn abandoned_direct_owner_is_recovered_when_its_process_is_gone() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open orphan-owner database");
    let volume = test_volume();
    db.upsert_global_volume(&volume).expect("seed source");
    seed_running_direct_operation_owner(
        &db,
        &volume,
        "crashed-direct-operation",
        i64::from(u32::MAX),
        1,
    );

    let replacement = db
        .claim_global_index_operation(
            &volume,
            "replacement-direct-operation",
            SourceOperationKind::Incremental,
            SourceOperationRoute::Direct,
        )
        .unwrap()
        .expect("a new Direct operation recovers an owner whose process exited");
    assert_eq!(replacement.revision, 2);
    assert!(db
        .finish_global_index_operation(
            &replacement,
            INDEX_STATUS_READY,
            None,
            None,
            Some(300),
            false,
        )
        .unwrap());
}

#[test]
fn live_direct_owner_is_not_reclaimed_when_precheck_confirms_its_process() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open live-owner database");
    let volume = test_volume();
    db.upsert_global_volume(&volume).expect("seed source");
    let owner_pid = i64::from(u32::MAX - 1);
    let owner_started_at = 42;
    seed_running_direct_operation_owner(
        &db,
        &volume,
        "live-direct-owner",
        owner_pid,
        owner_started_at,
    );

    let replacement = db
        .claim_global_index_operation_with_process_check_for_test(
            &volume,
            "replacement-for-live-owner",
            SourceOperationKind::Incremental,
            SourceOperationRoute::Direct,
            |pid, started_at| {
                assert_eq!((pid, started_at), (owner_pid, owner_started_at));
                Some(true)
            },
        )
        .unwrap();

    assert!(replacement.is_none());
    let state = db
        .global_index_operation_state(&volume.id)
        .unwrap()
        .unwrap();
    assert_eq!(state.revision, 1);
    assert_eq!(state.owner_id.as_deref(), Some("live-direct-owner"));
    assert_eq!(state.operation_state.as_deref(), Some("running"));
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn ambiguous_direct_owner_liveness_fails_closed() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open ambiguous-owner database");
    let volume = test_volume();
    db.upsert_global_volume(&volume).expect("seed source");
    seed_running_direct_operation_owner(
        &db,
        &volume,
        "unverifiable-direct-owner",
        i64::from(u32::MAX - 1),
        42,
    );

    let replacement = db
        .claim_global_index_operation_with_process_check_for_test(
            &volume,
            "replacement-for-unverifiable-owner",
            SourceOperationKind::Incremental,
            SourceOperationRoute::Direct,
            |_, _| None,
        )
        .unwrap();

    assert!(replacement.is_none());
    let state = db
        .global_index_operation_state(&volume.id)
        .unwrap()
        .unwrap();
    assert_eq!(state.revision, 1);
    assert_eq!(state.owner_id.as_deref(), Some("unverifiable-direct-owner"));
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn process_liveness_precheck_runs_without_an_immediate_writer_lock() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open process-precheck database");
    let process_probe_db = Database::open(&path).expect("open independent writer probe");
    let volume = test_volume();
    db.upsert_global_volume(&volume).expect("seed source");
    seed_running_direct_operation_owner(
        &db,
        &volume,
        "expired-direct-owner",
        i64::from(u32::MAX - 1),
        42,
    );

    let owner_id = volume.id.clone();
    let process_probe_closure_db = process_probe_db.clone();
    let recovered = db
        .claim_global_index_operation_with_process_check_for_test(
            &volume,
            "replacement-after-unlocked-precheck",
            SourceOperationKind::Incremental,
            SourceOperationRoute::Direct,
            move |_, _| {
                // A second connection must be able to obtain and commit an
                // IMMEDIATE write while the process check is running.
                let mut conn = process_probe_closure_db.conn().expect("probe connection");
                let transaction = conn
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .expect("process check runs outside the claim writer reservation");
                transaction
                    .execute(
                        "UPDATE global_volumes SET updated_at = updated_at + 1 WHERE id = ?1",
                        [&owner_id],
                    )
                    .expect("probe writer update");
                transaction.commit().expect("probe writer commit");
                Some(false)
            },
        )
        .unwrap()
        .expect("expired Direct owner can be recovered");

    assert_eq!(recovered.revision, 2);
    assert!(
        db.finish_global_index_operation(
            &recovered,
            INDEX_STATUS_READY,
            None,
            None,
            Some(300),
            false,
        )
        .unwrap()
    );
    drop(process_probe_db);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn operation_snapshot_change_after_precheck_rejects_claim() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open owner-snapshot database");
    let volume = test_volume();
    db.upsert_global_volume(&volume).expect("seed source");
    seed_running_direct_operation_owner(
        &db,
        &volume,
        "owner-revoked-during-precheck",
        i64::from(u32::MAX - 1),
        42,
    );

    let owner_id = volume.id.clone();
    let revision_db = db.clone();
    let replacement = db
        .claim_global_index_operation_with_process_check_for_test(
            &volume,
            "must-not-pass-stale-owner-snapshot",
            SourceOperationKind::Incremental,
            SourceOperationRoute::Direct,
            move |_, _| {
                let conn = revision_db.conn().expect("revision-revocation connection");
                conn.execute(
                    r#"
                    UPDATE global_volumes
                    SET operation_revision = operation_revision + 1,
                        operation_owner_id = NULL,
                        operation_owner_pid = NULL,
                        operation_owner_started_at = NULL,
                        operation_owner_route = NULL,
                        operation_kind = NULL,
                        operation_state = NULL
                    WHERE id = ?1
                    "#,
                    [&owner_id],
                )
                .expect("revoke the exact owner snapshot");
                Some(false)
            },
        )
        .unwrap();

    assert!(replacement.is_none());
    let state = db
        .global_index_operation_state(&volume.id)
        .unwrap()
        .unwrap();
    assert_eq!(state.revision, 2);
    assert_eq!(state.owner_id, None);
    assert_eq!(state.operation_state, None);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn concurrent_expired_owner_claims_have_one_revision_fenced_winner() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open concurrent-claim database");
    let volume = test_volume();
    db.upsert_global_volume(&volume).expect("seed source");
    seed_running_direct_operation_owner(
        &db,
        &volume,
        "expired-owner-for-concurrent-claims",
        i64::from(u32::MAX - 1),
        42,
    );

    let precheck_barrier = Arc::new(Barrier::new(2));
    let first_db = db.clone();
    let first_volume = volume.clone();
    let first_barrier = precheck_barrier.clone();
    let first_claim = std::thread::spawn(move || {
        first_db.claim_global_index_operation_with_process_check_for_test(
            &first_volume,
            "concurrent-claim-first",
            SourceOperationKind::Incremental,
            SourceOperationRoute::Direct,
            move |_, _| {
                first_barrier.wait();
                Some(false)
            },
        )
    });
    let second_db = db.clone();
    let second_volume = volume.clone();
    let second_claim = std::thread::spawn(move || {
        second_db.claim_global_index_operation_with_process_check_for_test(
            &second_volume,
            "concurrent-claim-second",
            SourceOperationKind::Incremental,
            SourceOperationRoute::Direct,
            move |_, _| {
                precheck_barrier.wait();
                Some(false)
            },
        )
    });

    let first = first_claim
        .join()
        .expect("first claim thread")
        .expect("first claim result");
    let second = second_claim
        .join()
        .expect("second claim thread")
        .expect("second claim result");
    assert_ne!(first.is_some(), second.is_some());
    let winner = first.or(second).expect("one claim wins the CAS");
    let state = db
        .global_index_operation_state(&volume.id)
        .unwrap()
        .unwrap();
    assert_eq!(state.revision, 2);
    assert_eq!(
        state.owner_id.as_deref(),
        Some(winner.operation_id.as_str())
    );
    assert!(db
        .finish_global_index_operation(&winner, INDEX_STATUS_READY, None, None, Some(301), false,)
        .unwrap());
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn local_operation_registration_cleans_up_on_commit_failure_and_cancel() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open local-registration database");
    let volume = test_volume();
    db.upsert_global_volume(&volume).expect("seed source");
    db.conn()
        .unwrap()
        .execute_batch(
            r#"
            CREATE TABLE claim_commit_parent(id INTEGER PRIMARY KEY);
            CREATE TABLE claim_commit_child(
                parent_id INTEGER REFERENCES claim_commit_parent(id)
                    DEFERRABLE INITIALLY DEFERRED
            );
            CREATE TRIGGER fail_claim_at_commit
            AFTER UPDATE OF operation_owner_id ON global_volumes
            WHEN NEW.operation_owner_id = 'claim-commit-failure'
            BEGIN
                INSERT INTO claim_commit_child(parent_id) VALUES (1);
            END;
            "#,
        )
        .expect("install deferred constraint to fail the claim commit");

    let failed_commit = db.claim_global_index_operation(
        &volume,
        "claim-commit-failure",
        SourceOperationKind::Incremental,
        SourceOperationRoute::Direct,
    );
    assert!(failed_commit.is_err(), "deferred FK fails during COMMIT");
    let after_failed_commit = db
        .global_index_operation_state(&volume.id)
        .unwrap()
        .unwrap();
    assert_eq!(after_failed_commit.revision, 0);
    assert_eq!(after_failed_commit.owner_id, None);

    db.conn()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_claim_at_commit;")
        .expect("remove commit failure trigger");
    let owner = db
        .claim_global_index_operation(
            &volume,
            "claim-commit-failure",
            SourceOperationKind::Incremental,
            SourceOperationRoute::Direct,
        )
        .unwrap()
        .expect("failed commit must release the local operation registration");

    db.forget_local_global_index_operation(&owner);
    let after_cancel = db
        .claim_global_index_operation(
            &volume,
            &owner.operation_id,
            SourceOperationKind::Incremental,
            SourceOperationRoute::Direct,
        )
        .unwrap()
        .expect("cancel must release the local registration for a fenced re-admission");
    assert!(after_cancel.revision > owner.revision);
    assert!(db
        .finish_global_index_operation(
            &after_cancel,
            INDEX_STATUS_READY,
            None,
            None,
            Some(301),
            false,
        )
        .unwrap());
    let after_finish = db
        .claim_global_index_operation(
            &volume,
            &owner.operation_id,
            SourceOperationKind::Incremental,
            SourceOperationRoute::Direct,
        )
        .unwrap()
        .expect("normal completion must release the local registration");
    assert!(db
        .finish_global_index_operation(
            &after_finish,
            INDEX_STATUS_READY,
            None,
            None,
            Some(302),
            false,
        )
        .unwrap());

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn source_operation_claims_are_independent_per_volume() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open independent-owner database");
    let first = test_volume();
    let mut second = test_volume();
    second.id = "gv_second".into();
    second.stable_volume_id = "second-volume".into();
    db.upsert_global_volume(&first).expect("seed first source");
    db.upsert_global_volume(&second)
        .expect("seed second source");

    let first_owner = db
        .claim_global_index_operation(
            &first,
            "first-source-operation",
            SourceOperationKind::Incremental,
            SourceOperationRoute::Service,
        )
        .unwrap()
        .expect("claim first source");

    // Simulate a second process that does not share the process-local registry:
    // the transaction must still prevent one operation ID owning two sources.
    db.forget_local_global_index_operation(&first_owner);
    assert!(db
        .claim_global_index_operation(
            &second,
            "first-source-operation",
            SourceOperationKind::Incremental,
            SourceOperationRoute::Service,
        )
        .unwrap()
        .is_none());
    let first_owner = db
        .claim_global_index_operation(
            &first,
            "first-source-operation",
            SourceOperationKind::Incremental,
            SourceOperationRoute::Service,
        )
        .unwrap()
        .expect("the admitted Service can reconcile its own prior source owner");
    let second_owner = db
        .claim_global_index_operation(
            &second,
            "second-source-operation",
            SourceOperationKind::Incremental,
            SourceOperationRoute::Service,
        )
        .unwrap()
        .expect("independent source can be admitted concurrently");
    assert_ne!(first_owner.volume_id, second_owner.volume_id);
    assert!(db
        .claim_global_index_operation(
            &first,
            "competing-first-source-operation",
            SourceOperationKind::Rebuild,
            SourceOperationRoute::Service,
        )
        .unwrap()
        .is_none());
}

#[test]
fn disabled_managed_scope_is_not_reported_as_active_management() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open test database");
    db.upsert_global_volume(&test_volume())
        .expect("insert global volume");
    let document = test_entry(r"C:\Global\Disabled\note.txt", "note.txt", false);
    db.upsert_global_entries_batch(std::slice::from_ref(&document))
        .expect("insert global document");

    let scope = db
        .add_managed_scope(AddManagedScopeRequest {
            path: r"C:\Global\Disabled".to_string(),
            global_entry_id: None,
            enabled: false,
            allow_local_ai: true,
            allow_cloud_ai: false,
        })
        .expect("add disabled scope");
    let result = db
        .search_global_entries("note", 20, 0)
        .expect("search disabled scope");
    assert_eq!(result.len(), 1);
    assert!(!result[0].managed);

    let conn = Connection::open(&path).expect("inspect disabled scope");
    let enabled_entries: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM managed_entries WHERE enabled = 1",
            [],
            |row| row.get(0),
        )
        .expect("enabled managed entries");
    assert_eq!(enabled_entries, 0);
    drop(conn);

    db.update_managed_scope_policy(UpdateManagedScopePolicyRequest {
        id: scope.id,
        enabled: Some(true),
        allow_local_ai: None,
        allow_cloud_ai: None,
    })
    .expect("enable scope");
    let result = db
        .search_global_entries("note", 20, 0)
        .expect("search enabled scope");
    assert!(result[0].managed);

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn overlapping_managed_scopes_keep_ai_jobs_isolated_by_scope() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open test database");
    db.upsert_global_volume(&test_volume())
        .expect("insert global volume");
    let document = test_entry(r"C:\Global\Shared\note.txt", "note.txt", false);
    db.upsert_global_entries_batch(std::slice::from_ref(&document))
        .expect("insert global document");

    db.add_managed_scope(AddManagedScopeRequest {
        path: r"C:\Global".to_string(),
        global_entry_id: None,
        enabled: true,
        allow_local_ai: true,
        allow_cloud_ai: false,
    })
    .expect("add broad managed scope");
    db.add_managed_scope(AddManagedScopeRequest {
        path: r"C:\Global\Shared".to_string(),
        global_entry_id: None,
        enabled: true,
        allow_local_ai: true,
        allow_cloud_ai: false,
    })
    .expect("add nested managed scope");

    let conn = Connection::open(&path).expect("inspect isolated jobs");
    let managed_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM managed_entries", [], |row| row.get(0))
        .expect("managed entry count");
    let job_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_jobs", [], |row| row.get(0))
        .expect("AI job count");
    let distinct_scope_count: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT managed_scope_id) FROM ai_jobs",
            [],
            |row| row.get(0),
        )
        .expect("AI job scope count");
    assert_eq!(managed_count, 2);
    assert_eq!(job_count, 2);
    assert_eq!(distinct_scope_count, 2);

    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn removing_the_last_managed_scope_clears_ai_state_but_keeps_global_entry() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open test database");
    db.upsert_global_volume(&test_volume())
        .expect("insert global volume");
    let document = test_entry(r"C:\Global\Managed\note.txt", "note.txt", false);
    db.upsert_global_entries_batch(std::slice::from_ref(&document))
        .expect("insert global document");
    let scope = db
        .add_managed_scope(AddManagedScopeRequest {
            path: r"C:\Global\Managed".to_string(),
            global_entry_id: None,
            enabled: true,
            allow_local_ai: true,
            allow_cloud_ai: false,
        })
        .expect("add managed scope");
    db.remove_managed_scope(&scope.id)
        .expect("remove managed scope");

    let results = db
        .search_global_entries("note", 20, 0)
        .expect("global search after scope removal");
    assert_eq!(results.len(), 1);
    assert!(!results[0].managed);
    let conn = Connection::open(&path).expect("inspect removed scope");
    let ai_state_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_analysis_state", [], |row| {
            row.get(0)
        })
        .expect("AI state count");
    let managed_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM managed_entries", [], |row| row.get(0))
        .expect("managed entry count");
    assert_eq!(ai_state_count, 0);
    assert_eq!(managed_count, 0);

    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[path = "tests/global_search_benchmark.rs"]
mod global_search_benchmark;

#[path = "tests/global_search_source_health.rs"]
mod global_search_source_health;
