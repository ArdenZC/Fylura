use super::*;
use crate::global_index::models::*;
use crate::global_index::repository::{SourceOperationKind, SourceOperationRoute};
use std::{path::Path, sync::mpsc::sync_channel};

struct Fixture(std::path::PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn fixture() -> (Fixture, Database, GlobalVolume, GlobalEntryInput) {
    let path = Fixture(
        std::env::temp_dir().join(format!("zc380-generation-{}.sqlite", uuid::Uuid::new_v4())),
    );
    let db = Database::open(&path.0).unwrap();
    let mut volume = crate::global_index::tests::test_volume();
    volume.index_status = INDEX_STATUS_ERROR.into();
    volume.last_error = Some("committed_real_io_error".into());
    volume.journal_id = Some("old-journal".into());
    volume.journal_cursor = Some("old-cursor".into());
    volume.last_full_index_at = Some(10);
    db.upsert_global_volume(&volume).unwrap();
    let mut entry = GlobalEntryInput::from_path(
        &volume.id,
        Path::new("/legacy/original.txt"),
        &volume.provider,
    );
    entry.platform_file_id = "frn:42".into();
    entry.parent_platform_file_id = "frn:parent".into();
    db.upsert_global_entries_batch(std::slice::from_ref(&entry))
        .unwrap();
    db.add_managed_scope(AddManagedScopeRequest {
        path: "/legacy".into(),
        global_entry_id: None,
        enabled: true,
        allow_local_ai: true,
        allow_cloud_ai: false,
    })
    .unwrap();
    db.conn().unwrap().execute(
        "UPDATE ai_analysis_state SET user_corrected = 1, classification_json = '{\"user\":true}'",
        [],
    ).unwrap();
    (path, db, volume, entry)
}

fn claim(
    db: &Database,
    volume: &GlobalVolume,
    route: SourceOperationRoute,
) -> SourceOperationOwner {
    db.claim_global_index_operation(
        volume,
        &uuid::Uuid::new_v4().to_string(),
        SourceOperationKind::Rebuild,
        route,
    )
    .unwrap()
    .expect("actual admitted source claim")
}

// Compare complete committed rows, not only counts or an isolated fake model.
fn legacy_facts(db: &Database) -> Vec<Vec<String>> {
    let conn = db.conn().unwrap();
    [
        "global_volumes",
        "global_entries",
        "managed_entries",
        "managed_scopes",
        "ai_jobs",
        "ai_job_items",
        "ai_analysis_state",
        "files",
        "user_tags",
        "file_user_tags",
    ]
    .iter()
    .map(|table| {
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = stmt.column_count();
        stmt.query_map([], |row| {
            Ok((0..columns)
                .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                .collect::<Vec<_>>()
                .join("|"))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    })
    .collect()
}

fn candidate_count(db: &Database, id: &str) -> i64 {
    db.conn()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM global_index_generation_entries WHERE generation_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn candidate_partial_batch_failure_cancel_and_two_versions_leave_legacy_and_ai_unchanged() {
    let (_path, db, volume, original) = fixture();
    db.request_global_index_rebuild(&volume.id).unwrap();
    let owner = claim(&db, &volume, SourceOperationRoute::Service);
    // The existing admission changes source health. P3-A changes nothing after
    // that claim; it must also preserve a subsequently committed real error.
    db.update_global_volume_state_owned(
        &owner,
        INDEX_STATUS_ERROR,
        Some("committed_real_io_error"),
    )
    .unwrap();
    let before = legacy_facts(&db);
    let (wake_tx, wake_rx) = sync_channel(2);
    db.set_managed_ai_waker(wake_tx);
    db.set_managed_ai_work_wakes_armed(true);
    let candidate = db
        .begin_global_index_candidate(&owner, CaptureKind::Rebuild)
        .unwrap();
    let mut changed = original.clone();
    changed.path = "/legacy/candidate-only/original.txt".into();
    changed.size = 7788;
    assert_eq!(
        original.entry_id(),
        changed.entry_id(),
        "new metadata for the same native identity retains the logical ID"
    );
    db.stage_global_index_candidate_batch(&candidate, std::slice::from_ref(&changed))
        .unwrap();
    assert_eq!(legacy_facts(&db), before);
    assert!(
        wake_rx.try_recv().is_err(),
        "candidate storage cannot wake AI"
    );
    assert_eq!(
        db.search_global_entries("original", 20, 0).unwrap().len(),
        1
    );
    assert!(db
        .search_global_entries("candidate-only", 20, 0)
        .unwrap()
        .is_empty());
    // A real SQLite uniqueness failure after one row in batch 2 proves rollback
    // of that row and its physical FTS insertion, while batch 1 stays committed.
    let mut second = changed.clone();
    second.platform_file_id = "second-id".into();
    second.name = "rolledback-token.txt".into();
    assert!(db
        .stage_global_index_candidate_batch(&candidate, &[second.clone(), changed.clone()])
        .is_err());
    assert_eq!(candidate_count(&db, &candidate.id), 1);
    let mut wrong_provider = second.clone();
    wrong_provider.source_provider = "foreign-provider".into();
    assert!(db
        .stage_global_index_candidate_batch(&candidate, &[wrong_provider])
        .is_err());
    let fts: i64 = db.conn().unwrap().query_row(
        "SELECT COUNT(*) FROM global_index_generation_fts WHERE global_index_generation_fts MATCH 'rolledback'", [], |r| r.get(0)).unwrap();
    assert_eq!(fts, 0);
    assert_eq!(legacy_facts(&db), before);
    assert!(db
        .seal_global_index_candidate(
            &candidate,
            &generation_manifest(&[changed.clone(), second.clone()]).unwrap(),
            None,
            None
        )
        .is_err());
    let mut other = volume.clone();
    other.id = "independent-volume".into();
    other.stable_volume_id = "independent-stable".into();
    db.upsert_global_volume(&other).unwrap();
    let other_owner = claim(&db, &other, SourceOperationRoute::Coordinator);
    let other_candidate = db
        .begin_global_index_candidate(&other_owner, CaptureKind::Initial)
        .unwrap();
    let mut foreign = second.clone();
    foreign.volume_id = other.id.clone();
    assert!(db
        .stage_global_index_candidate_batch(&candidate, std::slice::from_ref(&foreign))
        .is_err());
    db.stage_global_index_candidate_batch(&other_candidate, &[foreign])
        .unwrap();
    let before_cancel = legacy_facts(&db);
    db.abort_global_index_candidate(&candidate, "cancelled")
        .unwrap();
    assert_eq!(legacy_facts(&db), before_cancel);
    let after_cancel = legacy_facts(&db);
    assert!(db
        .stage_global_index_candidate_batch(&candidate, std::slice::from_ref(&second))
        .is_err());
    assert!(db
        .seal_global_index_candidate(
            &candidate,
            &generation_manifest(std::slice::from_ref(&changed)).unwrap(),
            None,
            None
        )
        .is_err());
    assert!(
        db.begin_global_index_candidate(&owner, CaptureKind::Rebuild)
            .is_err(),
        "no same-revision ABA after abort"
    );
    assert_eq!(legacy_facts(&db), after_cancel);
    assert_eq!(candidate_count(&db, &other_candidate.id), 1);
    // A new admitted operation may store a second physical version of the same
    // logical ID. Neither version replaces the existing published legacy row.
    db.finish_global_index_operation(
        &owner,
        INDEX_STATUS_ERROR,
        Some("committed_real_io_error"),
        None,
        None,
        false,
    )
    .unwrap();
    let successor = claim(&db, &volume, SourceOperationRoute::Service);
    let v2 = db
        .begin_global_index_candidate(&successor, CaptureKind::Reconcile)
        .unwrap();
    let mut changed_again = changed.clone();
    changed_again.path = "/legacy/newer-version/original.txt".into();
    db.stage_global_index_candidate_batch(&v2, &[changed_again])
        .unwrap();
    let versions: i64 = db.conn().unwrap().query_row(
        "SELECT COUNT(DISTINCT version_rowid) FROM global_index_generation_entries WHERE logical_entry_id=?1",
        [changed.entry_id()], |r| r.get(0)).unwrap();
    assert_eq!(versions, 2);
    let conn = db.conn().unwrap();
    let mapped: i64 = conn.query_row(
        "SELECT COUNT(*) FROM global_index_generation_fts f JOIN global_index_generation_entries e
         ON e.version_rowid=f.rowid JOIN global_index_generations g ON g.generation_id=e.generation_id
         WHERE global_index_generation_fts MATCH 'newer' AND g.volume_id=?1", [&volume.id], |r| r.get(0)).unwrap();
    assert_eq!(mapped, 1);
    let heads: (String, Option<String>) = conn.query_row(
        "SELECT legacy_state,published_generation_id FROM global_index_generation_heads WHERE volume_id=?1", [&volume.id], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(heads, ("LEGACY_UNVERIFIED".into(), None));
    assert_eq!(
        db.get_global_entry(&original.entry_id())
            .unwrap()
            .unwrap()
            .name,
        original.name
    );
}

#[test]
fn candidate_restart_unknown_service_fence_recovery_seal_and_late_writes() {
    let (path, db, volume, original) = fixture();
    db.request_global_index_rebuild(&volume.id).unwrap();
    let owner = claim(&db, &volume, SourceOperationRoute::Service);
    let candidate = db
        .begin_global_index_candidate(&owner, CaptureKind::Rebuild)
        .unwrap();
    db.stage_global_index_candidate_batch(&candidate, std::slice::from_ref(&original))
        .unwrap();
    db.mark_global_index_operation_unknown(&owner).unwrap();
    db.forget_local_global_index_operation(&owner);
    drop(db);
    let db = Database::open(&path.0).unwrap();
    let before = legacy_facts(&db);
    assert!(db
        .claim_global_index_operation(
            &volume,
            "forbidden-direct",
            SourceOperationKind::Rebuild,
            SourceOperationRoute::Direct
        )
        .unwrap()
        .is_none());
    assert!(db
        .stage_global_index_candidate_batch(&candidate, std::slice::from_ref(&original))
        .is_err());
    assert!(
        db.recover_global_index_candidate(&owner).is_err(),
        "UNKNOWN is not authority"
    );
    assert_eq!(legacy_facts(&db), before);
    let successor = claim(&db, &volume, SourceOperationRoute::Service);
    assert!(db.recover_global_index_candidate(&successor).unwrap());
    assert!(
        !db.recover_global_index_candidate(&successor).unwrap(),
        "idempotent recovery"
    );
    assert!(db.abort_global_index_candidate(&candidate, "late").is_err());
    let fresh = db
        .begin_global_index_candidate(&successor, CaptureKind::Rebuild)
        .unwrap();
    assert!(
        db.recover_global_index_candidate(&successor).is_err(),
        "cannot recover current candidate"
    );
    db.stage_global_index_candidate_batch(&fresh, std::slice::from_ref(&original))
        .unwrap();
    let manifest = generation_manifest(std::slice::from_ref(&original)).unwrap();
    let mut wrong = manifest.clone();
    wrong.digest = "0".repeat(64);
    assert!(db
        .seal_global_index_candidate(&fresh, &wrong, None, None)
        .is_err());
    let pre_seal = legacy_facts(&db);
    db.seal_global_index_candidate(
        &fresh,
        &manifest,
        Some("candidate-journal"),
        Some("candidate-cursor"),
    )
    .unwrap();
    assert_eq!(legacy_facts(&db), pre_seal);
    assert!(db
        .stage_global_index_candidate_batch(&fresh, &[original])
        .is_err());
    assert!(db
        .abort_global_index_candidate(&fresh, "late-cancel")
        .is_err());
    assert!(db
        .seal_global_index_candidate(&fresh, &manifest, None, None)
        .is_err());
    let op = db
        .global_index_operation_state(&volume.id)
        .unwrap()
        .unwrap();
    assert_eq!(op.owner_id, Some(successor.operation_id.clone()));
    assert!(op.rebuild_pending(), "seal never completes live intent");
    let current = db.get_global_volume(&volume.id).unwrap().unwrap();
    assert_eq!(current.journal_cursor.as_deref(), Some("old-cursor"));
    db.forget_local_global_index_operation(&successor);
    drop(db);
    let db = Database::open(&path.0).unwrap();
    let sealed: (String, String) = db
        .conn()
        .unwrap()
        .query_row(
            "SELECT state, journal_cursor FROM global_index_generations WHERE generation_id=?1",
            [&fresh.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(sealed, ("sealed".into(), "candidate-cursor".into()));
    // Superseding a sealed-but-unpublished build detaches its pointer without
    // mutating its sealed manifest or granting publication.
    let next = claim(&db, &volume, SourceOperationRoute::Service);
    assert!(db.recover_global_index_candidate(&next).unwrap());
    let state: String = db
        .conn()
        .unwrap()
        .query_row(
            "SELECT state FROM global_index_generations WHERE generation_id=?1",
            [&fresh.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "sealed");
}
