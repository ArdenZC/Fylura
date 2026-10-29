use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use zen_canvas_tauri::db::Database;
use zen_canvas_tauri::global_index::{
    search_global_entries, GlobalEntryInput, GlobalVolume, INDEX_STATUS_READY,
    PROVIDER_WINDOWS_MFT_USN, SCOPE_AUTHORIZATION_AUTHORIZED,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn test_db_path() -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "zen-canvas-global-search-hardening-{}-{id}.db",
        std::process::id()
    ))
}

fn volume(id: &str, mount_path: &str, enabled: bool) -> GlobalVolume {
    GlobalVolume {
        id: id.to_string(),
        platform: "windows".to_string(),
        stable_volume_id: format!("stable-{id}"),
        display_name: id.to_string(),
        mount_path: mount_path.to_string(),
        filesystem_type: "ntfs".to_string(),
        drive_kind: "fixed".to_string(),
        enabled,
        scope_authorization_state: SCOPE_AUTHORIZATION_AUTHORIZED.to_string(),
        provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
        index_status: INDEX_STATUS_READY.to_string(),
        last_error: None,
        journal_id: None,
        journal_cursor: None,
        last_full_index_at: Some(1),
        last_incremental_sync_at: Some(1),
        entry_count: 0,
        created_at: 1,
        updated_at: 1,
    }
}

fn entry(volume_id: &str, path: &str, name: &str) -> GlobalEntryInput {
    GlobalEntryInput {
        volume_id: volume_id.to_string(),
        platform_file_id: format!("frn:{path}"),
        parent_platform_file_id: format!("frn:parent:{volume_id}"),
        name: name.to_string(),
        path: path.to_string(),
        extension: "txt".to_string(),
        is_directory: false,
        size: 1,
        created_at_fs: Some(1),
        modified_at_fs: Some(2),
        file_attributes: 0,
        is_hidden: false,
        is_system: false,
        source_provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
        last_seen_at: 2,
    }
}

#[test]
fn disabled_volume_entries_are_not_returned() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open database");
    db.upsert_global_volume(&volume("enabled", r"C:\", true))
        .expect("insert enabled volume");
    db.upsert_global_volume(&volume("disabled", r"D:\", false))
        .expect("insert disabled volume");
    db.upsert_global_entries_batch(&[
        entry("enabled", r"C:\Reports\report.txt", "report.txt"),
        entry("disabled", r"D:\Reports\report.txt", "report.txt"),
    ])
    .expect("insert entries");

    let results = search_global_entries(&db, "report", 20, 0).expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].volume_id, "enabled");

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn aggregate_status_ignores_disabled_volume_entries_and_errors() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open database");
    db.upsert_global_volume(&volume("enabled", r"C:\", true))
        .expect("insert enabled volume");
    let mut disabled = volume("disabled", r"D:\", false);
    disabled.last_error = Some("disabled_volume_failure".to_string());
    disabled.last_incremental_sync_at = Some(99);
    db.upsert_global_volume(&disabled)
        .expect("insert disabled volume");
    db.upsert_global_entries_batch(&[
        entry("enabled", r"C:\Reports\report.txt", "report.txt"),
        entry("disabled", r"D:\Reports\hidden.txt", "hidden.txt"),
    ])
    .expect("insert entries");

    let status = db.global_index_status().expect("global index status");
    assert_eq!(status.total_entries, 1);
    assert_eq!(status.processed_entries, 1);
    assert_eq!(status.indexed_volumes, 1);
    assert_eq!(status.ready_volumes, 1);
    assert!(status.collection_complete);
    assert_eq!(status.last_sync_at, Some(1));
    assert_eq!(status.last_error, None);

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn one_and_two_character_queries_are_prefix_bounded() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open database");
    db.upsert_global_volume(&volume("enabled", r"C:\", true))
        .expect("insert volume");
    db.upsert_global_entries_batch(&[
        entry("enabled", r"C:\Docs\报告.txt", "报告.txt"),
        entry("enabled", r"C:\Docs\周报.txt", "周报.txt"),
    ])
    .expect("insert entries");

    let prefix = search_global_entries(&db, "报", 20, 0).expect("prefix search");
    assert_eq!(prefix.len(), 1);
    assert_eq!(prefix[0].name, "报告.txt");

    let two = search_global_entries(&db, "报告", 20, 0).expect("two-character search");
    assert_eq!(two.len(), 1);
    assert_eq!(two[0].name, "报告.txt");

    drop(db);
    let _ = std::fs::remove_file(path);
}
