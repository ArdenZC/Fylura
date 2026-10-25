use super::manual_rename_tests::Fixture;
use super::*;
use crate::db::{
    scan::{ManagedScanRequest, ScanAdmissionOptions, ScanFinalizeInput},
    CreateUserTagRequest, LibrarySelectionV1, MutateFileUserTagsRequest, UserTagMutationOperation,
};
use rusqlite::{params, Connection};

fn managed_fixture() -> (Fixture, String, String) {
    let mut f = Fixture::with_legacy_id(true);
    let library = normalize_path(f.source.parent().unwrap());
    let mut settings = crate::settings::get_app_settings(&f.db).unwrap();
    settings.default_scan_folders = vec![crate::settings::ScanRootSetting {
        id: "h1-library".into(),
        path: library.clone(),
        label: "H1 disposable library".into(),
        enabled: true,
        created_at: "2026-10-25T00:00:00.000Z".into(),
    }];
    crate::settings::save_app_settings(&f.db, &settings).unwrap();
    f.db.sync_file_library_watcher_roots(&settings.default_scan_folders)
        .unwrap();
    let root_id = f
        .db
        .list_watcher_root_configs()
        .unwrap()
        .into_iter()
        .find(|root| root.path == library)
        .unwrap()
        .id;
    let revision = f.db.begin_watcher_revision(&root_id).unwrap().unwrap();
    f.db.apply_watcher_exact_mutations(
        &root_id,
        revision.watcher_revision,
        &[normalize_path(&f.source)],
        &std::collections::HashSet::new(),
    )
    .unwrap();
    let conn = Connection::open(f.db.path()).unwrap();
    let uid = conn
        .query_row(
            "SELECT source_uid FROM library_source_root_claims WHERE root_id = ?1 AND claim_state = 'present'",
            params![root_id],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    f.id = conn
        .query_row(
            "SELECT id FROM files WHERE path = ?1 AND is_stale = 0",
            params![normalize_path(&f.source)],
            |row| row.get(0),
        )
        .unwrap();
    drop(conn);
    (f, library, uid)
}

fn scan_empty_library(db: &Database, library: &str) {
    let admission = db
        .admit_managed_scan(&ScanAdmissionOptions {
            request: ManagedScanRequest {
                roots: vec![library.into()],
                request_key: None,
                dedupe: false,
            },
            run_id_override: None,
        })
        .unwrap();
    let run = db.claim_queued_scan_run(&admission.runs[0].id).unwrap();
    let run = db
        .reconcile_missing(
            &run.dto.id,
            run.dto.revision,
            run.root_revision,
            run.session_revision,
        )
        .unwrap();
    db.finalize_scan_run(
        &run.dto.id,
        run.dto.revision,
        run.root_revision,
        run.session_revision,
        &ScanFinalizeInput {
            terminal_status: "completed".into(),
            error_code: None,
            error_message: None,
            allow_stale_reconciliation: true,
            rule_recovery_succeeded: false,
        },
    )
    .unwrap();
}

#[test]
fn m1_h1_outside_root_move_restore_reobserves_retired_uid_and_fails_closed() {
    for retire in [false, true] {
        for fault in ["none", "journal", "source_digest", "fingerprint"] {
            let (f, library, uid) = managed_fixture();
            let original = file_identity_fingerprint(&f.source).unwrap();
            let tag = f
                .db
                .create_user_tag(CreateUserTagRequest {
                    display_name: "H1 retained fact".into(),
                    color_token: "blue".into(),
                })
                .unwrap();
            f.db.mutate_file_user_tags(MutateFileUserTagsRequest {
                selection: LibrarySelectionV1::Explicit {
                    file_ids: vec![f.id.clone()],
                },
                tag_ids: vec![tag.id.clone()],
                operation: UserTagMutationOperation::Add,
                expected_count: Some(1),
            })
            .unwrap();
            let destination = f.root.join("Destination");
            fs::create_dir(&destination).unwrap();
            assert!(!destination.starts_with(Path::new(&library)));
            let preview = f
                .db
                .get_manual_move_operation_preview(&f.id, &normalize_path(&destination), None)
                .unwrap();
            let mut selected = f.selection(&preview);
            selected.operations[0].destination_directory = Some(normalize_path(&destination));
            let request = resolve_execute_selections(&f.db, selected).unwrap();
            let moved = execute_moves_with_persistence(&f.db, request).unwrap();
            assert_eq!(moved.logs[0].status, "success");
            assert!(moved.logs[0].error_message.is_none());
            assert!(!f.source.exists());
            assert_eq!(
                file_identity_fingerprint(Path::new(&preview.target_path))
                    .unwrap()
                    .platform_volume_id,
                original.platform_volume_id
            );
            if retire {
                scan_empty_library(&f.db, &library);
            }
            let conn = Connection::open(f.db.path()).unwrap();
            let state: String = conn
                .query_row(
                    "SELECT verification_state FROM library_source_incarnations WHERE source_uid = ?1",
                    params![uid],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(state, if retire { "retired" } else { "verified" });
            if fault == "journal" {
                conn.execute_batch(
                    "CREATE TRIGGER reject_h1_final_journal BEFORE UPDATE OF restore_status ON operation_logs
                     WHEN NEW.restore_status = 'restored' BEGIN
                     SELECT RAISE(ABORT, 'injected H1 final journal failure'); END;",
                )
                .unwrap();
            } else if fault == "source_digest" {
                conn.execute(
                    "UPDATE library_source_incarnations SET verification_digest = 'injected-mismatch' WHERE source_uid = ?1",
                    params![uid],
                )
                .unwrap();
            }
            let mut logs = f
                .db
                .get_restorable_operation_logs_by_ids(&[moved.logs[0].id.clone()])
                .unwrap();
            if fault == "fingerprint" {
                logs[0].target_full_hash = Some("injected-mismatch".into());
            }
            let result = restore_moves_with_persistence(&f.db, RestoreMovesRequest { logs }).unwrap();
            // This is the fresh backend History read, not the command's cached projection.
            let history = f.db.get_operation_logs(Some(10)).unwrap();
            assert_eq!(history[0].restore_status, result.logs[0].restore_status);
            assert_eq!(history[0].restore_phase, result.logs[0].restore_phase);
            assert_eq!(history[0].restore_error, result.logs[0].restore_error);
            assert!(!history[0].can_restore);
            if fault == "none" {
                assert_eq!(result.restored, 1, "retire={retire}: {:?}", result.logs);
                assert_eq!(history[0].restore_status, "restored");
                assert_eq!(history[0].restore_phase, "completed");
                assert!(history[0].restored_at.is_some());
                assert!(history[0].restore_claim_path.is_none());
                let restored = file_identity_fingerprint(&f.source).unwrap();
                assert_eq!(restored.platform_file_id, original.platform_file_id);
                assert_eq!(restored.platform_volume_id, original.platform_volume_id);
                assert_eq!(restored.full_hash, original.full_hash);
                assert_eq!(fs::read(&f.source).unwrap(), b"owned fixture");
                assert!(!Path::new(&preview.target_path).exists());
                assert_eq!(f.db.get_file_library_detail(&f.id).unwrap().tags[0].id, tag.id);
                let restored_uid: String = conn
                    .query_row(
                        "SELECT source_uid FROM library_source_incarnations WHERE source_uid = ?1 AND verification_state = 'verified'",
                        params![uid],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(restored_uid, uid);
            } else {
                assert_eq!(result.restored, 0);
                assert_eq!(result.failed, 1);
                assert_eq!(history[0].restore_status, "manual_review");
                assert!(history[0].restored_at.is_none());
                if fault != "fingerprint" {
                    assert!(history[0].restore_claim_path.is_some());
                    assert_eq!(history[0].restore_phase, "target_committed");
                    assert!(restore_requires_reconciliation(&history[0]));
                    assert_eq!(fs::read(&f.source).unwrap(), b"owned fixture");
                    assert!(!Path::new(&preview.target_path).exists());
                    assert!(restore_moves_with_persistence(
                        &f.db,
                        RestoreMovesRequest { logs: history }
                    )
                    .is_err());
                } else {
                    // A mismatch before claiming leaves the object at Destination.
                    assert_eq!(history[0].restore_phase, "manual_review");
                    assert!(!f.source.exists());
                    assert_eq!(fs::read(&preview.target_path).unwrap(), b"owned fixture");
                }
            }
            drop(conn);
            f.finish();
        }
    }
}
