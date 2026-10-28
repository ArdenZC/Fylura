//! Review #5678861648 defect 2 — consent must not survive a volume identity or
//! capability change.
//!
//! `upsert_global_volume` used to update only descriptive columns, keeping
//! `enabled` and `scope_authorization_state` from the previous row. A source
//! whose stable id was reused (serial collision, GUID->serial drift, a cloud
//! mount replacing a local disk on the same letter) could therefore inherit
//! whole-volume authorization it never received.

use super::{test_db_path, test_volume};
use crate::db::Database;
use crate::global_index::models::{PROVIDER_WINDOWS_MFT_USN, PROVIDER_WINDOWS_UNSUPPORTED};

/// `test_db_path` is deterministic per process, so a re-run inside the same
/// temp directory would reopen a stale database. Start from a clean file.
fn fresh_db() -> (Database, std::path::PathBuf) {
    let path = test_db_path();
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let db = Database::open(&path).expect("open db");
    (db, path)
}

#[test]
fn capability_change_resets_consent_but_keeps_the_row() {
    let (db, path) = fresh_db();

    let mut authorized = test_volume();
    authorized.scope_authorization_state = "authorized".to_string();
    authorized.enabled = true;
    authorized.provider = PROVIDER_WINDOWS_MFT_USN.to_string();
    db.upsert_global_volume(&authorized).expect("upsert");

    let rows = db.list_global_volumes().expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].scope_authorization_state, "authorized");
    assert!(rows[0].enabled);

    // Same stable id, but a fresh discovery now reports a different
    // capability: this is what a virtual/cloud mount or an identity downgrade
    // looks like. Discovery always emits the un-authorized default, so the
    // upsert must adopt it instead of keeping the old consent.
    let mut downgraded = authorized.clone();
    downgraded.provider = PROVIDER_WINDOWS_UNSUPPORTED.to_string();
    downgraded.filesystem_type = "FAT32".to_string();
    downgraded.scope_authorization_state = "not_authorized".to_string();
    downgraded.enabled = false;
    downgraded.index_status = "unavailable".to_string();
    db.upsert_global_volume(&downgraded).expect("re-upsert");

    let rows = db.list_global_volumes().expect("list");
    assert_eq!(rows.len(), 1, "the historical row must be kept, not deleted");
    assert_eq!(
        rows[0].scope_authorization_state, "not_authorized",
        "a capability change must not carry whole-volume consent forward"
    );
    assert!(
        !rows[0].enabled,
        "a capability change must not leave the source enabled for indexing"
    );
    assert_eq!(rows[0].provider, PROVIDER_WINDOWS_UNSUPPORTED);

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn reupsert_with_identical_capability_keeps_consent() {
    let (db, path) = fresh_db();

    let mut authorized = test_volume();
    authorized.scope_authorization_state = "authorized".to_string();
    authorized.enabled = true;
    db.upsert_global_volume(&authorized).expect("upsert");

    // A plain re-mount (letter unchanged, same capability) must stay authorized.
    db.upsert_global_volume(&authorized).expect("re-upsert");

    let rows = db.list_global_volumes().expect("list");
    assert_eq!(rows[0].scope_authorization_state, "authorized");
    assert!(rows[0].enabled);

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn remount_on_a_new_letter_keeps_consent_for_the_same_identity() {
    let (db, path) = fresh_db();

    let mut authorized = test_volume();
    authorized.scope_authorization_state = "authorized".to_string();
    authorized.enabled = true;
    db.upsert_global_volume(&authorized).expect("upsert");

    let mut remounted = authorized.clone();
    remounted.mount_path = r"D:\Global\".to_string();
    db.upsert_global_volume(&remounted).expect("re-upsert");

    let rows = db.list_global_volumes().expect("list");
    assert_eq!(rows.len(), 1, "same stable identity must stay one row");
    assert_eq!(rows[0].mount_path, r"D:\Global\");
    assert_eq!(
        rows[0].scope_authorization_state, "authorized",
        "the same physical volume keeps its consent when only the letter changes"
    );

    drop(db);
    let _ = std::fs::remove_file(path);
}
