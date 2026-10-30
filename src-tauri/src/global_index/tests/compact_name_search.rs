use super::query::match_name;
use super::*;
use crate::global_index::models::{
    GlobalEntryInput, GlobalVolume, INDEX_STATUS_READY, PROVIDER_WINDOWS_MFT_USN,
    SCOPE_AUTHORIZATION_AUTHORIZED,
};
use crate::global_index::search::search_global_entries_on_connection;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn test_path() -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
    std::env::temp_dir().join(format!(
        "zen-canvas-compact-name-search-{}-{id}.db",
        std::process::id()
    ))
}

fn test_volume(id: &str, mount_path: &str) -> GlobalVolume {
    GlobalVolume {
        id: id.to_string(),
        platform: "windows".to_string(),
        stable_volume_id: format!("stable-{id}"),
        display_name: format!("Test {id}"),
        mount_path: mount_path.to_string(),
        filesystem_type: "ntfs".to_string(),
        drive_kind: "fixed".to_string(),
        enabled: true,
        scope_authorization_state: SCOPE_AUTHORIZATION_AUTHORIZED.to_string(),
        provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
        index_status: INDEX_STATUS_READY.to_string(),
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

fn test_entry(
    volume_id: &str,
    file_id: &str,
    name: &str,
    path: &str,
    mtime: i64,
) -> GlobalEntryInput {
    GlobalEntryInput {
        volume_id: volume_id.to_string(),
        platform_file_id: file_id.to_string(),
        parent_platform_file_id: String::new(),
        name: name.to_string(),
        path: path.to_string(),
        extension: name
            .rsplit_once('.')
            .map_or(String::new(), |(_, ext)| ext.to_lowercase()),
        is_directory: false,
        size: 1,
        created_at_fs: Some(1),
        modified_at_fs: Some(mtime),
        file_attributes: 0,
        is_hidden: false,
        is_system: false,
        source_provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
        last_seen_at: 1,
    }
}

fn cleanup(path: &Path) {
    let sidecar = PathBuf::from(format!("{}.compact-name-search", path.display()));
    let _ = std::fs::remove_dir_all(sidecar);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

fn independent_rank(name: &str, extension: &str, path: &str, query: &str) -> Option<u8> {
    let bounded = name.match_indices(query).any(|(start, matched)| {
        let before = name[..start].chars().next_back();
        let after = name[start + matched.len()..].chars().next();
        !before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
            && !after.is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
    });
    if name.contains(query) {
        Some(if bounded { 0 } else { 1 })
    } else if extension.contains(query) {
        Some(1)
    } else if path.contains(query) {
        Some(2)
    } else {
        None
    }
}

#[test]
fn word_boundaries_check_every_occurrence_and_unicode_characters() {
    assert!(query::has_bounded_occurrence("xpha-pha.txt", "pha"));
    assert!(!query::has_bounded_occurrence("alphabeta.txt", "pha"));
    assert_eq!(match_name("alpha-pha-report.txt", "pha"), Some(1));
    assert_eq!(match_name("alphareport.txt", "pha"), Some(2));
    assert_eq!(match_name("資料-計画-報告.txt", "計画"), Some(1));
    assert_eq!(match_name("東京計画案.txt", "計画"), Some(2));
}

#[test]
fn complete_top_k_and_offset_4016_match_independent_oracle() {
    let path = test_path();
    let db = crate::db::Database::open(&path).expect("open fixture database");
    let volume = test_volume("volume-main", r"F:\compact-fixture");
    db.upsert_global_volume(&volume)
        .expect("insert fixture volume");
    let second_volume = test_volume("volume-second", r"F:\compact-second");
    db.upsert_global_volume(&second_volume)
        .expect("insert second fixture volume");

    let entries = (0..4_200)
        .map(|index| {
            let name = format!("alpha-pha-{:03}.txt", index % 137);
            let file_path = format!(r"F:\compact-fixture\folder-{index}\{name}");
            let mut entry = test_entry(
                &volume.id,
                &format!("file-{index}"),
                &name,
                &file_path,
                (index % 31) as i64,
            );
            if index % 17 == 0 {
                entry.modified_at_fs = None;
            }
            entry
        })
        .collect::<Vec<_>>();
    let duplicate_name = test_entry(
        &second_volume.id,
        "same-name-other-volume",
        "alpha-pha-000.txt",
        r"F:\compact-second\alpha-pha-000.txt",
        99,
    );
    let mut all_entries = entries.clone();
    all_entries.push(duplicate_name.clone());
    assert_eq!(
        db.upsert_global_entries_batch(&all_entries)
            .expect("insert duplicate-name fixture"),
        all_entries.len()
    );
    db.enable_compact_name_search_for_test()
        .expect("install transactional change feed");
    db.rebuild_compact_name_search_for_test()
        .expect("build compact mmap index");

    let conn = db.conn().expect("read fixture for independent oracle");
    let mut oracle_statement = conn
        .prepare(
            "SELECT ge.id, ge.name_normalized, ge.extension, ge.path_normalized,
                    ge.modified_at_fs
             FROM global_entries ge JOIN global_volumes gv ON gv.id=ge.volume_id
             WHERE gv.enabled=1 AND gv.coverage_pending=0 AND ge.is_stale=0",
        )
        .expect("prepare oracle read");
    let mut rows = oracle_statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })
        .expect("read oracle rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect oracle rows");
    let query = "pha";
    rows.retain(|(_, name, extension, path, _)| {
        independent_rank(name, extension, path, query).is_some()
    });
    rows.sort_by(|left, right| {
        independent_rank(&left.1, &left.2, &left.3, query)
            .cmp(&independent_rank(&right.1, &right.2, &right.3, query))
            .then_with(|| right.4.cmp(&left.4))
            .then_with(|| left.0.cmp(&right.0))
    });
    let expected_window = rows
        .iter()
        .take(4_096)
        .map(|row| row.0.clone())
        .collect::<Vec<_>>();

    let first_page = db
        .search_global_entries(query, 80, 0)
        .expect("compact first page")
        .into_iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    assert_eq!(first_page, expected_window[..80]);

    let deep_page = db
        .search_global_entries(query, 80, 4_016)
        .expect("compact deep page")
        .into_iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    assert_eq!(deep_page, expected_window[4_016..4_096]);

    let snapshot = db
        .search_global_entries_snapshot(query, 80, 4_016)
        .expect("snapshot search through compact adapter");
    assert_eq!(
        snapshot
            .results
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        expected_window[4_016..4_096]
    );
    assert_eq!(
        db.compact_name_search().diagnostics().last_name_candidates,
        4_201
    );
    let diagnostics = db.compact_name_search().diagnostics();
    assert!(diagnostics.enabled);
    assert!(diagnostics.queries >= 3);
    assert!(diagnostics.accepted_queries >= 3);
    assert_eq!(diagnostics.fallbacks, 0);
    assert!(diagnostics.rebuilds >= 1);
    assert_eq!(diagnostics.last_overlay_events, 0);
    assert_eq!(diagnostics.last_path_candidates, 0);
    assert!(diagnostics.path_relation_complete);
    assert!(diagnostics.last_error.is_none());

    let expected_without_second = expected_window
        .iter()
        .filter(|id| *id != &duplicate_name.entry_id())
        .cloned()
        .collect::<Vec<_>>();
    db.set_global_volume_enabled(&second_volume.id, false)
        .expect("revoke second source");
    assert_eq!(
        db.search_global_entries(query, 80, 0)
            .expect("search after second source revocation")
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        expected_without_second[..80]
    );

    drop(oracle_statement);
    drop(conn);
    drop(db);
    cleanup(&path);
}

#[test]
fn overlay_updates_deletes_and_volume_revocation_are_snapshot_safe() {
    let path = test_path();
    let db = crate::db::Database::open(&path).expect("open fixture database");
    let volume = test_volume("volume-overlay", r"F:\overlay-fixture");
    db.upsert_global_volume(&volume)
        .expect("insert fixture volume");
    let baseline = test_entry(
        &volume.id,
        "stable-file-1",
        "alpha-pha-before.txt",
        r"F:\overlay-fixture\alpha-pha-before.txt",
        1,
    );
    let mut parent = test_entry(
        &volume.id,
        "folder-identity",
        "archive-folder",
        r"F:\overlay-fixture\archive-folder",
        2,
    );
    parent.is_directory = true;
    parent.extension.clear();
    parent.size = 0;
    let mut path_only = test_entry(
        &volume.id,
        "path-only-identity",
        "clean.txt",
        r"F:\overlay-fixture\archive-folder\clean.txt",
        2,
    );
    path_only.parent_platform_file_id = parent.platform_file_id.clone();
    let unicode = test_entry(
        &volume.id,
        "unicode-identity",
        "資料-計画-報告.txt",
        r"F:\overlay-fixture\資料-計画-報告.txt",
        4,
    );
    let punctuation = test_entry(
        &volume.id,
        "punctuation-identity",
        "doc-v2.9.txt",
        r"F:\overlay-fixture\doc-v2.9.txt",
        4,
    );
    db.upsert_global_entries_batch(&[
        baseline.clone(),
        parent.clone(),
        path_only.clone(),
        unicode.clone(),
        punctuation.clone(),
    ])
    .expect("insert baseline row");
    db.enable_compact_name_search_for_test()
        .expect("install transactional change feed");
    db.rebuild_compact_name_search_for_test()
        .expect("build compact mmap index");
    let path_page = db
        .search_global_entries("arc", 20, 0)
        .expect("path-only child search");
    assert_eq!(path_page.len(), 2);
    assert_eq!(path_page[0].id, parent.entry_id());
    assert_eq!(path_page[1].id, path_only.entry_id());
    assert_eq!(
        db.compact_name_search().diagnostics().last_path_candidates,
        1
    );
    assert_eq!(
        db.search_global_entries("計画", 20, 0)
            .expect("two-character Unicode search")
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        [unicode.entry_id().as_str()]
    );
    assert_eq!(
        db.search_global_entries("v2.9", 20, 0)
            .expect("punctuation substring search")
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        [punctuation.entry_id().as_str()]
    );
    assert!(db
        .search_global_entries("no-such-token", 20, 0)
        .expect("zero-hit search")
        .is_empty());
    let baseline_id = baseline.entry_id();

    let renamed = test_entry(
        &volume.id,
        "stable-file-1",
        "renamed-away.txt",
        r"F:\overlay-fixture\renamed-away.txt",
        3,
    );
    let added = test_entry(
        &volume.id,
        "stable-file-2",
        "new-pha-entry.txt",
        r"F:\overlay-fixture\new-pha-entry.txt",
        5,
    );
    db.mark_global_entry_stale(&baseline_id)
        .expect("retire renamed base identity");
    db.upsert_global_entries_batch(&[renamed, added.clone()])
        .expect("apply overlay rename and insert");
    let found = db
        .search_global_entries("pha", 20, 0)
        .expect("search current overlay");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, added.entry_id());
    assert_ne!(found[0].id, baseline_id);

    db.conn()
        .expect("open writer connection")
        .execute(
            "DELETE FROM global_entries WHERE id=?1",
            [&added.entry_id()],
        )
        .expect("delete overlay row");
    assert!(db
        .search_global_entries("pha", 20, 0)
        .expect("search after delete")
        .is_empty());

    let authorized = test_entry(
        &volume.id,
        "stable-file-3",
        "scope-pha.txt",
        r"F:\overlay-fixture\scope-pha.txt",
        8,
    );
    db.upsert_global_entries_batch(&[authorized])
        .expect("insert row for authorization check");
    db.set_global_volume_enabled(&volume.id, false)
        .expect("revoke source from search");
    assert!(db
        .search_global_entries("pha", 20, 0)
        .expect("search after source revocation")
        .is_empty());

    drop(db);
    cleanup(&path);
}

#[test]
fn owner_lease_prevents_another_process_from_removing_the_live_feed() {
    const CHILD_DATABASE_ENV: &str = "ZC_TEST_COMPACT_OWNER_CHILD_DB";
    if let Ok(child_path) = std::env::var(CHILD_DATABASE_ENV) {
        let child_db = crate::db::Database::open(&child_path)
            .expect("open database from an env-off secondary process");
        if std::env::var("ZC_TEST_COMPACT_OWNER_CHILD_MODE").as_deref() == Ok("recover") {
            assert!(child_db.compact_name_search().is_enabled());
            child_db
                .rebuild_compact_name_search_for_test()
                .expect("restart reacquires the released owner lease and rebuilds");
            return;
        }
        let child_entry = test_entry(
            "volume-cross-process",
            "child-stable-id",
            "childneedle.txt",
            r"F:\cross-process\childneedle.txt",
            3,
        );
        child_db
            .upsert_global_entries_batch(&[child_entry])
            .expect("write through the live owner feed from a second process");
        return;
    }

    let path = test_path();
    let db = crate::db::Database::open(&path).expect("open owner database");
    let volume = test_volume("volume-cross-process", r"F:\cross-process");
    db.upsert_global_volume(&volume)
        .expect("insert fixture source");
    db.upsert_global_entries_batch(&[test_entry(
        &volume.id,
        "base-stable-id",
        "base-entry.txt",
        r"F:\cross-process\base-entry.txt",
        1,
    )])
    .expect("insert base entry");
    db.enable_compact_name_search_for_test()
        .expect("acquire exclusive owner lease");
    db.rebuild_compact_name_search_for_test()
        .expect("build base under the owner lease");

    let output = std::process::Command::new(
        std::env::current_exe().expect("locate current Rust test executable"),
    )
    .args([
        "--exact",
        "global_index::compact_name_search::tests::owner_lease_prevents_another_process_from_removing_the_live_feed",
        "--nocapture",
    ])
    .env(CHILD_DATABASE_ENV, &path)
    .env_remove("ZC_COMPACT_NAME_SEARCH")
    .output()
    .expect("spawn an isolated secondary database opener");
    assert!(
        output.status.success(),
        "secondary process failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let result = db
        .search_global_entries("childneedle", 20, 0)
        .expect("the owner observes the second process feed batch");
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "childneedle.txt");
    let conn = db.conn().expect("inspect committed cross-process outbox");
    let (batch_rows, represented_changes): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(event_count), 0)
             FROM compact_name_search_changes
             WHERE seq > (SELECT base_generation FROM compact_name_search_state WHERE singleton=1)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("count committed batch event");
    assert_eq!((batch_rows, represented_changes), (1, 1));

    drop(conn);
    drop(db);

    let recovery = std::process::Command::new(
        std::env::current_exe().expect("locate current Rust test executable for recovery"),
    )
    .args([
        "--exact",
        "global_index::compact_name_search::tests::owner_lease_prevents_another_process_from_removing_the_live_feed",
        "--nocapture",
    ])
    .env(CHILD_DATABASE_ENV, &path)
    .env("ZC_TEST_COMPACT_OWNER_CHILD_MODE", "recover")
    .env("ZC_COMPACT_NAME_SEARCH", "1")
    .output()
    .expect("spawn candidate restart after owner release");
    assert!(
        recovery.status.success(),
        "candidate restart failed: stdout={} stderr={}",
        String::from_utf8_lossy(&recovery.stdout),
        String::from_utf8_lossy(&recovery.stderr)
    );
    cleanup(&path);
}

#[test]
fn tracked_writer_batches_are_atomic_and_crash_rollback_keeps_the_feed_contiguous() {
    let path = test_path();
    let db = crate::db::Database::open(&path).expect("open batch-feed fixture");
    let volume = test_volume("volume-batch-feed", r"F:\batch-feed");
    db.upsert_global_volume(&volume)
        .expect("insert feed source");
    let entries = (0..64)
        .map(|index| {
            test_entry(
                &volume.id,
                &format!("batch-stable-{index:03}"),
                &format!("batchmarker-{index:03}.txt"),
                &format!(r"F:\batch-feed\batchmarker-{index:03}.txt"),
                index,
            )
        })
        .collect::<Vec<_>>();
    db.upsert_global_entries_batch(&entries)
        .expect("insert initial rows");
    db.enable_compact_name_search_for_test()
        .expect("install leased feed");
    db.rebuild_compact_name_search_for_test()
        .expect("build immutable base");
    let modified = entries
        .iter()
        .cloned()
        .map(|mut entry| {
            entry.modified_at_fs = Some(entry.modified_at_fs.unwrap_or_default() + 100);
            entry
        })
        .collect::<Vec<_>>();
    db.upsert_global_entries_batch(&modified)
        .expect("write one production batch through the feed");

    let conn = db.conn().expect("inspect compact batch event");
    let (rows, event_count, json_count): (i64, i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(event_count), 0),
                    COALESCE(SUM(json_array_length(batch_ids)), 0)
             FROM compact_name_search_changes
             WHERE seq > (SELECT base_generation FROM compact_name_search_state WHERE singleton=1)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read single atomic outbox batch");
    assert_eq!((rows, event_count, json_count), (1, 64, 64));
    drop(conn);

    let results = db
        .search_global_entries("batchmarker", 80, 0)
        .expect("search all rows from the batched overlay");
    assert_eq!(
        results
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        modified
            .iter()
            .rev()
            .map(GlobalEntryInput::entry_id)
            .collect::<Vec<_>>()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );

    let mut writer = db.conn().expect("lease rollback test connection");
    let tx = writer
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .expect("begin simulated interrupted writer transaction");
    assert!(crate::global_index::repository::begin_compact_feed_batch(
        &tx,
        db.compact_feed_batching.load(AtomicOrdering::Acquire)
    )
    .expect("mark batch transaction"));
    let id = modified[0].entry_id();
    tx.execute(
        "UPDATE global_entries SET modified_at_fs=999999 WHERE id=?1",
        [&id],
    )
    .expect("change row under transaction marker");
    drop(tx); // SQLite rolls back both the row change and batch marker.
    let (stored_mtime, batch_active): (i64, i64) = writer
        .query_row(
            "SELECT e.modified_at_fs,s.batch_active FROM global_entries e
             CROSS JOIN compact_name_search_state s WHERE e.id=?1 AND s.singleton=1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("verify rollback restored row and marker");
    assert_eq!(stored_mtime, modified[0].modified_at_fs.unwrap());
    assert_eq!(batch_active, 0);
    drop(writer);

    let after_rollback = db
        .search_global_entries("batchmarker-000", 20, 0)
        .expect("search after atomic rollback");
    assert_eq!(after_rollback.len(), 1);
    drop(db);
    cleanup(&path);
}

#[test]
fn changed_trigger_set_invalidates_manifest_before_rebuild() {
    let path = test_path();
    let db = crate::db::Database::open(&path).expect("open fixture database");
    let volume = test_volume("volume-trigger", r"F:\trigger-fixture");
    db.upsert_global_volume(&volume)
        .expect("insert fixture volume");
    db.upsert_global_entries_batch(&[test_entry(
        &volume.id,
        "stable-trigger-file",
        "alpha-pha.txt",
        r"F:\trigger-fixture\alpha-pha.txt",
        1,
    )])
    .expect("insert fixture entry");
    db.enable_compact_name_search_for_test()
        .expect("install transactional change feed");
    db.rebuild_compact_name_search_for_test()
        .expect("build compact mmap index");
    db.conn()
        .expect("open schema writer")
        .execute_batch("DROP TRIGGER compact_name_search_entries_au")
        .expect("simulate a missing persistent trigger");
    db.compact_name_search()
        .install_change_tracking()
        .expect("repair and invalidate a changed trigger set");
    let base_generation = db
        .conn()
        .expect("read invalidated manifest")
        .query_row(
            "SELECT base_generation FROM compact_name_search_state WHERE singleton=1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("read invalidated generation");
    assert_eq!(base_generation, -1);
    db.rebuild_compact_name_search_for_test()
        .expect("rebuild after change-feed repair");
    assert_eq!(db.search_global_entries("pha", 20, 0).unwrap().len(), 1);

    drop(db);
    cleanup(&path);
}

#[test]
fn oversized_change_feed_invalidates_the_base_and_retains_only_the_latest_event() {
    let path = test_path();
    let db = crate::db::Database::open(&path).expect("open fixture database");
    let volume = test_volume("volume-feed-cap", r"F:\feed-cap-fixture");
    db.upsert_global_volume(&volume)
        .expect("insert fixture volume");
    let entry = test_entry(
        &volume.id,
        "stable-feed-cap-file",
        "alpha-pha.txt",
        r"F:\feed-cap-fixture\alpha-pha.txt",
        1,
    );
    let entry_id = entry.entry_id();
    db.upsert_global_entries_batch(&[entry])
        .expect("insert fixture entry");
    db.enable_compact_name_search_for_test()
        .expect("install bounded transactional change feed");
    db.rebuild_compact_name_search_for_test()
        .expect("build compact mmap index");

    db.conn()
        .expect("set overflow boundary")
        .execute(
            "UPDATE compact_name_search_state
             SET change_floor=base_generation-65536 WHERE singleton=1",
            [],
        )
        .expect("set near-cap feed floor");
    db.conn()
        .expect("open source writer")
        .execute(
            "UPDATE global_entries SET modified_at_fs=2 WHERE id=?1",
            [&entry_id],
        )
        .expect("emit the overflowing source change");

    let conn = db.conn().expect("inspect capped feed");
    let base_generation = conn
        .query_row(
            "SELECT base_generation FROM compact_name_search_state WHERE singleton=1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("read invalidated generation");
    let retained_events = conn
        .query_row(
            "SELECT COUNT(*) FROM compact_name_search_changes",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("count retained feed events");
    assert_eq!(base_generation, -1);
    assert_eq!(retained_events, 1);
    drop(conn);

    db.rebuild_compact_name_search_for_test()
        .expect("rebuild from capped feed snapshot");
    assert_eq!(db.search_global_entries("pha", 20, 0).unwrap().len(), 1);

    drop(db);
    cleanup(&path);
}

#[test]
fn corrupt_or_version_mismatched_index_falls_back_and_rebuilds_at_same_generation() {
    let path = test_path();
    let db = crate::db::Database::open(&path).expect("open fixture database");
    let volume = test_volume("volume-corrupt", r"F:\corrupt-fixture");
    db.upsert_global_volume(&volume)
        .expect("insert fixture volume");
    db.upsert_global_entries_batch(&[test_entry(
        &volume.id,
        "stable-corrupt-file",
        "alpha-pha.txt",
        r"F:\corrupt-fixture\alpha-pha.txt",
        1,
    )])
    .expect("insert fixture entry");
    db.enable_compact_name_search_for_test()
        .expect("install transactional change feed");
    db.rebuild_compact_name_search_for_test()
        .expect("build compact mmap index");

    let manager = db.compact_name_search();
    let index_name = db
        .conn()
        .expect("read current manifest")
        .query_row(
            "SELECT file_name FROM compact_name_search_state WHERE singleton=1",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("read mapped filename");
    let index_path = manager.sidecar_dir.join(index_name);

    {
        let mut conn = db.conn().expect("open version-check connection");
        let tx = conn.transaction().expect("begin version-check snapshot");
        tx.execute(
            "UPDATE compact_name_search_state SET format_version=0 WHERE singleton=1",
            [],
        )
        .expect("simulate a manifest format mismatch");
        let compact = manager
            .search_page_inner(&tx, "pha", 20, 0, 20, &HashSet::new())
            .expect("check mismatched manifest");
        assert!(compact.is_none());
        assert_eq!(
            search_global_entries_on_connection(&tx, "pha", 20, 0)
                .expect("authoritative SQLite fallback")
                .len(),
            1
        );
        tx.rollback().expect("discard test-only manifest mismatch");
    }

    *manager
        .loaded
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(&index_path)
        .expect("open index for corruption fixture");
    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(0)).expect("seek to index header");
    std::io::Write::write_all(&mut file, b"X").expect("corrupt index magic");
    file.sync_all().expect("flush corruption fixture");
    drop(file);

    {
        let mut conn = db.conn().expect("open corrupt-index snapshot");
        let tx = conn.transaction().expect("begin corrupt-index snapshot");
        assert!(manager
            .search_page_inner(&tx, "pha", 20, 0, 20, &HashSet::new())
            .expect("reject corrupt mmap")
            .is_none());
        assert_eq!(
            search_global_entries_on_connection(&tx, "pha", 20, 0)
                .expect("fallback after corrupt mmap")
                .len(),
            1
        );
        tx.rollback().expect("end corrupt-index snapshot");
    }

    db.rebuild_compact_name_search_for_test()
        .expect("replace corrupt index at the same change generation");
    assert_eq!(
        db.search_global_entries("pha", 20, 0)
            .expect("search rebuilt index")
            .len(),
        1
    );

    drop(db);
    cleanup(&path);
}
