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
    assert_eq!(
        rows.len(),
        1,
        "the historical row must be kept, not deleted"
    );
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

#[test]
fn capability_downgrade_recovery_requires_new_consent_and_preserves_history() {
    let (db, path) = fresh_db();
    let mut original = test_volume();
    original.journal_id = Some("journal-kept".into());
    original.journal_cursor = Some("9001".into());
    original.last_full_index_at = Some(111);
    db.upsert_global_volume(&original).unwrap();
    db.upsert_global_entries_batch(&[super::test_entry("Z:\\kept.txt", "kept.txt", false)])
        .unwrap();
    let conn = db.conn().unwrap();
    let entry: String = conn
        .query_row("SELECT id FROM global_entries", [], |r| r.get(0))
        .unwrap();
    conn.execute("INSERT INTO ai_analysis_state (global_entry_id,status,input_fingerprint,content_summary,updated_at) VALUES (?1,'completed','kept','summary',3)", [&entry]).unwrap();
    conn.execute("INSERT INTO managed_scopes (id,path,global_entry_id,allow_cloud_ai,created_at,updated_at) VALUES ('round7-scope','fixture',?1,1,1,2)", [&entry]).unwrap();
    conn.execute("INSERT INTO ai_jobs (id,global_entry_id,managed_scope_id,input_fingerprint,provider,status,created_at) VALUES ('round7-job',?1,'round7-scope','kept','local','completed',3)", [&entry]).unwrap();
    drop(conn);
    for cycle in 0..3 {
        let mut discovery = original.clone();
        discovery.enabled = false;
        discovery.scope_authorization_state = "not_authorized".into();
        discovery.provider = PROVIDER_WINDOWS_UNSUPPORTED.into();
        discovery.filesystem_type = "FAT32".into();
        db.upsert_global_volume(&discovery).unwrap();
        assert!(db
            .authorize_global_volume(&original.id, true, &discovery)
            .is_err());
        discovery.provider = PROVIDER_WINDOWS_MFT_USN.into();
        discovery.filesystem_type = "ntfs".into();
        discovery.mount_path = format!("fixture-letter-{cycle}");
        db.upsert_global_volume(&discovery).unwrap();
        let restored = db.get_global_volume(&original.id).unwrap().unwrap();
        assert_eq!(restored.provider, PROVIDER_WINDOWS_MFT_USN);
        assert!(!restored.enabled);
        assert_eq!(restored.scope_authorization_state, "not_authorized");
        assert!(db.set_global_volume_enabled(&original.id, true).is_err());
        assert!(db
            .authorize_global_volume(&original.id, false, &discovery)
            .is_err());
        db.authorize_global_volume(&original.id, true, &discovery)
            .unwrap();
        let approved = db.get_global_volume(&original.id).unwrap().unwrap();
        assert!(approved.enabled);
        assert_eq!(approved.scope_authorization_state, "authorized");
        assert_eq!(approved.stable_volume_id, original.stable_volume_id);
        assert_eq!(approved.journal_cursor, original.journal_cursor);
        assert_eq!(approved.journal_id, original.journal_id);
        assert_eq!(approved.last_full_index_at, Some(111));
        assert_eq!(approved.entry_count, 1);
        assert_eq!(
            db.conn()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM ai_jobs WHERE id='round7-job' AND global_entry_id=?1",
                    [&entry],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        eprintln!("round7 capability cycle={cycle}: unsupported/not_authorized/disabled -> mft/not_authorized/disabled -> explicit confirmation/authorized/enabled; id={} cursor={} entries={}", approved.id, approved.journal_cursor.as_deref().unwrap(), approved.entry_count);
        assert_eq!(
            db.conn()
                .unwrap()
                .query_row(
                    "SELECT content_summary FROM ai_analysis_state WHERE global_entry_id=?1",
                    [&entry],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "summary"
        );
    }
    drop(db);
    let reopened = Database::open(&path).unwrap();
    assert_eq!(
        reopened
            .get_global_volume(&original.id)
            .unwrap()
            .unwrap()
            .entry_count,
        1
    );
    drop(reopened);
    let _ = std::fs::remove_file(path);
}

#[test]
fn concurrent_demotion_between_enable_check_and_write_cannot_enable() {
    let (db, path) = fresh_db();
    let original = test_volume();
    db.upsert_global_volume(&original).unwrap();
    db.set_global_volume_enabled(&original.id, false).unwrap();
    let writer = Database::open(&path).unwrap();
    let (checked_tx, checked_rx) = std::sync::mpsc::sync_channel(0);
    let (done_tx, done_rx) = std::sync::mpsc::sync_channel(0);
    let id = original.id.clone();
    let enabler = std::thread::spawn(move || {
        writer.set_global_volume_enabled_before_update(&id, true, || {
            checked_tx.send(()).unwrap();
            done_rx.recv().unwrap();
        })
    });
    checked_rx.recv().unwrap();
    let mut demoted = original.clone();
    demoted.enabled = false;
    demoted.scope_authorization_state = "not_authorized".into();
    demoted.provider = PROVIDER_WINDOWS_UNSUPPORTED.into();
    db.upsert_global_volume(&demoted).unwrap();
    let revision = db
        .global_index_operation_state(&original.id)
        .unwrap()
        .unwrap()
        .revision;
    done_tx.send(()).unwrap();
    assert!(enabler.join().unwrap().is_err());
    let row = db.get_global_volume(&original.id).unwrap().unwrap();
    assert!(!row.enabled);
    assert_eq!(row.scope_authorization_state, "not_authorized");
    assert_eq!(
        db.global_index_operation_state(&original.id)
            .unwrap()
            .unwrap()
            .revision,
        revision
    );
    db.set_global_volume_enabled(&original.id, false).unwrap();
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn enable_disable_revision_changes_fence_owners_but_idempotent_enable_does_not() {
    let (db, path) = fresh_db();
    let original = test_volume();
    db.upsert_global_volume(&original).unwrap();
    let other = Database::open(&path).unwrap();
    let owner = db
        .claim_global_index_operation(
            &original,
            "round7-owner",
            crate::global_index::SourceOperationKind::Incremental,
            crate::global_index::SourceOperationRoute::Coordinator,
        )
        .unwrap()
        .unwrap();
    other.set_global_volume_enabled(&original.id, true).unwrap();
    assert_eq!(
        db.global_index_operation_state(&original.id)
            .unwrap()
            .unwrap()
            .owner_id
            .as_deref(),
        Some("round7-owner")
    );
    other
        .set_global_volume_enabled(&original.id, false)
        .unwrap();
    let disabled = db
        .global_index_operation_state(&original.id)
        .unwrap()
        .unwrap();
    assert_eq!(disabled.revision, owner.revision + 1);
    assert_eq!(disabled.owner_id, None);
    assert!(!db
        .finish_global_index_operation(&owner, "ready", None, Some(99), Some(99), false)
        .unwrap());
    other.set_global_volume_enabled(&original.id, true).unwrap();
    let enabled = db
        .global_index_operation_state(&original.id)
        .unwrap()
        .unwrap();
    assert_eq!(enabled.revision, disabled.revision + 1);
    assert_eq!(enabled.owner_id, None);
    db.forget_local_global_index_operation(&owner);
    drop(other);
    drop(db);
    let _ = std::fs::remove_file(path);
}
