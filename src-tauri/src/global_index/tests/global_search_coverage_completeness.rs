//! Regression coverage for the search "complete empty" claim.
//!
//! Review #5678861648 defect 1: `collection_complete` was derived from a
//! partial exclusion list, so an enabled source sitting in
//! `permission_required` (or any `spotlight_*` / `fsevents_unavailable` state)
//! still reported complete coverage. `globalSearchFeedback.ts` then rendered a
//! normal "no results" card for a search that had not actually covered the
//! volume.
//!
//! Completion is now defined positively: every enabled source must be `ready`.

use super::{test_db_path, test_volume};
use crate::db::Database;
use crate::global_index::{
    GlobalVolume, INDEX_STATUS_DATABASE_BUSY, INDEX_STATUS_DISCOVERED, INDEX_STATUS_ERROR,
    INDEX_STATUS_FSEVENTS_UNAVAILABLE, INDEX_STATUS_INDEXING, INDEX_STATUS_PAUSED,
    INDEX_STATUS_PERMISSION_REQUIRED, INDEX_STATUS_READY, INDEX_STATUS_REBUILD_REQUIRED,
    INDEX_STATUS_SERVICE_BUSY, INDEX_STATUS_SPOTLIGHT_EXTERNAL_NOT_INDEXED,
    INDEX_STATUS_SPOTLIGHT_NOT_INDEXED, INDEX_STATUS_SPOTLIGHT_UNAVAILABLE, INDEX_STATUS_SYNCING,
    INDEX_STATUS_UNAVAILABLE,
};

fn volume(id: &str, enabled: bool) -> GlobalVolume {
    let mut volume = test_volume();
    volume.id = id.to_string();
    volume.stable_volume_id = format!("stable-{id}");
    volume.mount_path = format!(r"C:\Global\{id}\");
    volume.enabled = enabled;
    volume.index_status = INDEX_STATUS_READY.to_string();
    volume
}

fn set_status(db: &Database, id: &str, status: &str) {
    db.update_global_volume_state(id, status, None, None, None, None, None)
        .expect("update volume status");
}

/// Every degraded status that must never be presented as complete coverage.
const DEGRADED_STATUSES: &[&str] = &[
    INDEX_STATUS_PERMISSION_REQUIRED,
    INDEX_STATUS_SPOTLIGHT_NOT_INDEXED,
    INDEX_STATUS_SPOTLIGHT_UNAVAILABLE,
    INDEX_STATUS_SPOTLIGHT_EXTERNAL_NOT_INDEXED,
    INDEX_STATUS_FSEVENTS_UNAVAILABLE,
    INDEX_STATUS_UNAVAILABLE,
    INDEX_STATUS_ERROR,
    INDEX_STATUS_PAUSED,
    INDEX_STATUS_REBUILD_REQUIRED,
    INDEX_STATUS_INDEXING,
    INDEX_STATUS_SYNCING,
    INDEX_STATUS_DISCOVERED,
    INDEX_STATUS_DATABASE_BUSY,
    INDEX_STATUS_SERVICE_BUSY,
];

#[test]
fn collection_complete_is_false_without_enabled_sources() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open db");

    let status = db.global_index_status().expect("status");
    assert_eq!(status.indexed_volumes, 0);
    assert!(
        !status.collection_complete,
        "no enabled source can never be complete coverage"
    );

    // A discovered-but-disabled source must not count either.
    db.upsert_global_volume(&volume("gv_disabled", false))
        .expect("upsert");
    let status = db.global_index_status().expect("status");
    assert_eq!(status.indexed_volumes, 0);
    assert!(!status.collection_complete);

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn collection_complete_is_true_for_a_single_ready_source() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open db");
    db.upsert_global_volume(&volume("gv_ready", true))
        .expect("upsert");

    let status = db.global_index_status().expect("status");
    assert_eq!(status.indexed_volumes, 1);
    assert_eq!(status.ready_volumes, 1);
    assert!(
        status.collection_complete,
        "a single ready source is complete coverage"
    );

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn collection_complete_rejects_each_degraded_status() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open db");
    db.upsert_global_volume(&volume("gv_single", true))
        .expect("upsert");

    for status_value in DEGRADED_STATUSES {
        set_status(&db, "gv_single", status_value);
        let status = db.global_index_status().expect("status");
        assert_eq!(status.indexed_volumes, 1, "status {status_value}");
        assert!(
            !status.collection_complete,
            "enabled source in `{status_value}` must not report complete coverage"
        );
    }

    // Returning to ready must restore the claim, so the rule is not just
    // "always false after the first degradation".
    set_status(&db, "gv_single", INDEX_STATUS_READY);
    assert!(
        db.global_index_status()
            .expect("status")
            .collection_complete,
        "recovery to ready must restore complete coverage"
    );

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn collection_complete_requires_all_mixed_sources_ready() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open db");
    db.upsert_global_volume(&volume("gv_a", true))
        .expect("upsert");
    db.upsert_global_volume(&volume("gv_b", true))
        .expect("upsert");

    let status = db.global_index_status().expect("status");
    assert_eq!(status.indexed_volumes, 2);
    assert_eq!(status.ready_volumes, 2);
    assert!(
        status.collection_complete,
        "both ready sources are complete"
    );

    // One of two sources degraded: still incomplete, and the aggregate status
    // must expose `partial` rather than a zero-result claim.
    set_status(&db, "gv_b", INDEX_STATUS_PERMISSION_REQUIRED);
    let status = db.global_index_status().expect("status");
    assert_eq!(status.indexed_volumes, 2);
    assert_eq!(status.ready_volumes, 1);
    assert!(
        !status.collection_complete,
        "mixed ready+permission is partial"
    );
    assert_eq!(status.status, "partial");

    // A disabled degraded source must not poison coverage.
    set_status(&db, "gv_b", INDEX_STATUS_READY);
    db.set_global_volume_enabled("gv_b", false)
        .expect("disable");
    set_status(&db, "gv_b", INDEX_STATUS_PERMISSION_REQUIRED);
    let status = db.global_index_status().expect("status");
    assert_eq!(status.indexed_volumes, 1);
    assert!(
        status.collection_complete,
        "disabled sources are outside the coverage claim"
    );

    drop(db);
    let _ = std::fs::remove_file(path);
}
