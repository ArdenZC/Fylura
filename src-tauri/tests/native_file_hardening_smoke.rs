#![cfg(all(feature = "native-qa", windows))]

use fylura_tauri::{
    db::Database,
    file_ops::{
        execute_moves_with_persistence, native_qa_transfer_move_preview,
        reconcile_pending_operation_journal, restore_moves_with_persistence,
        set_operation_test_fault, ExecuteMovesRequest, OperationLogDto, OperationPreviewRequest,
        OperationTestFaultPoint, RestoreMovesRequest,
    },
    fs_safety::{
        atomic_move::test_faults::{self, AtomicFaultPoint},
        atomic_move_noreplace, capture_identity, claim_source,
        source_claim::{self, ClaimTestPoint},
        AtomicMoveError, VerifiedDirectory,
    },
    storage_analyzer::{
        classify_candidate_for_test, move_cleanup_candidates_to_safe_trash_for_candidates,
        preview_cleanup_operations_for_candidates, preview_cleanup_restore_item_for_test,
        reconcile_pending_cleanup_journal, restore_cleanup_trash_items_for_db, CleanupTrashItem,
        StorageCandidate,
    },
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    process::Command,
};

fn panic_after_restore_claim(point: ClaimTestPoint, _source: &Path, _claim: &Path) {
    if point == ClaimTestPoint::AfterClaimVerifiedBeforeTargetCommit {
        panic!("native restore source-claimed crash");
    }
}

fn replace_claim_before_identity_check(point: ClaimTestPoint, _source: &Path, claim: &Path) {
    if point == ClaimTestPoint::AfterClaimBeforeIdentityCheck {
        fs::remove_file(claim).expect("remove claim for identity mismatch");
        fs::write(claim, b"replacement claim").expect("write replacement claim");
    }
}

fn native_cleanup_candidate(root: &Path, case_name: &str, payload: &[u8]) -> StorageCandidate {
    let source = root.join(case_name).join("node_modules");
    fs::create_dir_all(source.join("package")).expect("create Safe Trash case");
    fs::write(source.join("package").join("index.js"), payload).expect("write Safe Trash case");
    classify_candidate_for_test(&source, payload.len() as u64)
}

fn cleanup_item(db: &Database, item_id: &str) -> CleanupTrashItem {
    db.list_cleanup_trash_batches()
        .expect("list cleanup batches")
        .into_iter()
        .flat_map(|batch| batch.items)
        .find(|item| item.id == item_id)
        .unwrap_or_else(|| panic!("cleanup item not found: {item_id}"))
}

fn ordinary_restore_fixture(
    db: &Database,
    root: &Path,
    run_id: &str,
    label: &str,
) -> (PathBuf, PathBuf, OperationLogDto) {
    let source = root.join(format!("ordinary-{label}-before.txt"));
    let target = root.join(format!("ordinary-{label}-after.txt"));
    fs::write(&source, format!("ordinary restore {label}")).expect("write ordinary restore source");
    let moved = execute_moves_with_persistence(
        db,
        ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: format!("native-ordinary-restore-{label}-{run_id}"),
                file_id: format!("native-ordinary-restore-file-{label}-{run_id}"),
                operation_type: "move".to_string(),
                source_path: source.to_string_lossy().into_owned(),
                target_path: target.to_string_lossy().into_owned(),
                old_name: format!("ordinary-{label}-before.txt"),
                new_name: format!("ordinary-{label}-after.txt"),
                is_executable: Some(true),
            }],
        },
    )
    .expect("execute ordinary restore fixture");
    assert_eq!(moved.logs[0].status, "success");
    (
        source,
        target,
        moved.logs.into_iter().next().expect("ordinary restore log"),
    )
}

fn native_copy_operation(
    run_id: &str,
    label: &str,
    source: &Path,
    target: &Path,
) -> OperationPreviewRequest {
    OperationPreviewRequest {
        id: format!("native-copy-{label}-{run_id}"),
        file_id: format!("native-copy-file-{label}-{run_id}"),
        operation_type: "copy".to_string(),
        source_path: source.to_string_lossy().into_owned(),
        target_path: target.to_string_lossy().into_owned(),
        old_name: source
            .file_name()
            .expect("copy source name")
            .to_string_lossy()
            .into_owned(),
        new_name: target
            .file_name()
            .expect("copy target name")
            .to_string_lossy()
            .into_owned(),
        is_executable: Some(true),
    }
}

#[derive(Serialize)]
struct SmokeManifest {
    schema: &'static str,
    fixture_root: String,
    database: String,
    source_volume: String,
    target_volume: String,
    cross_volume_exercised: bool,
    ordinary_same_volume_file_copy: &'static str,
    ordinary_same_volume_directory_copy: &'static str,
    ordinary_cross_volume_file_copy: &'static str,
    ordinary_cross_volume_directory_copy: &'static str,
    ordinary_same_volume_file_move: &'static str,
    ordinary_same_volume_directory_move: &'static str,
    ordinary_cross_volume_file_move: &'static str,
    windows_copy_audit_policy: &'static str,
    windows_sacl_inspection: &'static str,
    operation_move_restore: &'static str,
    rename: &'static str,
    target_conflict: &'static str,
    source_replacement: &'static str,
    target_parent_replacement: &'static str,
    staging_handle_copy_commit: &'static str,
    cross_volume_directory_move: &'static str,
    durability_failure: &'static str,
    safe_trash_durability_failure: &'static str,
    safe_trash_identity_mismatch: &'static str,
    safe_trash_source_cleanup_pending: &'static str,
    safe_trash_restore_source_claimed: &'static str,
    safe_trash_restore_target_committed: &'static str,
    ordinary_restore_source_claimed: &'static str,
    ordinary_restore_target_committed: &'static str,
    ordinary_restore_final_transaction: &'static str,
    ordinary_restore_target_replacement: &'static str,
    ordinary_restore_claim_replacement: &'static str,
    ordinary_restore_target_and_claim: &'static str,
    ordinary_restore_all_paths_missing: &'static str,
    ordinary_restore_claim_path: String,
    final_log_persistence_boundary: &'static str,
    claim_identity_reconciliation: &'static str,
    windows_nested_handle_relative: &'static str,
    system_trash_preview: &'static str,
    operation_reconciliation: &'static str,
    safe_trash_restore: &'static str,
    safe_trash_reconciliation: &'static str,
    sqlite_integrity: &'static str,
    canary_sha256_before: String,
    canary_sha256_after: String,
    canary_unchanged: bool,
    real_app_data_accessed: bool,
    fixture_cleaned: bool,
}

struct FixtureGuard {
    primary: PathBuf,
    secondary: Option<PathBuf>,
}

impl Drop for FixtureGuard {
    fn drop(&mut self) {
        remove_isolated_fixture_if_present(&self.primary);
        if let Some(secondary) = self.secondary.as_deref() {
            remove_isolated_fixture_if_present(secondary);
        }
    }
}

#[test]
#[ignore = "native filesystem QA harness; run explicitly with --features 'desktop-runtime native-qa'"]
fn native_file_hardening_smoke() {
    let run_id = uuid::Uuid::new_v4().to_string();
    let fixture_root = env::temp_dir().join(format!("zen-canvas-final-hardening-v413-{run_id}"));
    let source_volume = fixture_root.join("source-volume");
    let target_volume = secondary_target_root(&fixture_root, &run_id);
    let secondary_fixture = (!target_volume.starts_with(&fixture_root)).then(|| {
        target_volume
            .parent()
            .expect("secondary fixture root")
            .to_path_buf()
    });
    let fixture_guard = FixtureGuard {
        primary: fixture_root.clone(),
        secondary: secondary_fixture,
    };
    let app_data = fixture_root.join("app-data");
    let database_dir = fixture_root.join("database");
    let canary_dir = fixture_root.join("canary");
    for path in [
        &source_volume,
        &target_volume,
        &app_data,
        &database_dir,
        &canary_dir,
    ] {
        fs::create_dir_all(path).expect("create isolated smoke directory");
    }
    let canary = canary_dir.join("do-not-touch.txt");
    fs::write(&canary, b"native-qa-canary").expect("write canary");
    let canary_sha256_before = sha256_file(&canary);
    let database_path = database_dir.join("native-qa.sqlite3");
    let db = Database::open(&database_path).expect("open isolated database");

    let source = source_volume.join("unicode-源-file.txt");
    let target = target_volume.join("unicode-目标-file.txt");
    fs::write(&source, b"bound native operation payload").expect("write operation source");
    let operation = OperationPreviewRequest {
        id: format!("native-operation-{run_id}"),
        file_id: format!("native-file-{run_id}"),
        operation_type: "move".to_string(),
        source_path: source.to_string_lossy().into_owned(),
        target_path: target.to_string_lossy().into_owned(),
        old_name: "unicode-源-file.txt".to_string(),
        new_name: "unicode-目标-file.txt".to_string(),
        is_executable: Some(true),
    };
    let moved = execute_moves_with_persistence(
        &db,
        ExecuteMovesRequest {
            operations: vec![operation],
        },
    )
    .expect("execute production persisted move");
    let operation_move_restore = if moved.logs[0].status == "success" {
        assert_eq!(
            fs::read(&target).expect("read target"),
            b"bound native operation payload"
        );
        let restored = restore_moves_with_persistence(
            &db,
            RestoreMovesRequest {
                logs: moved.logs.clone(),
            },
        )
        .expect("restore production persisted move");
        assert_eq!(restored.restored, 1);
        assert_eq!(
            fs::read(&source).expect("read restored source"),
            b"bound native operation payload"
        );
        "passed"
    } else {
        let error = moved.logs[0].error_message.as_deref().unwrap_or_default();
        assert!(
            error.contains("windows_sacl_"),
            "unexpected move refusal: {error}"
        );
        assert!(source.exists(), "safe refusal must retain source");
        assert!(!target.exists(), "safe refusal must not publish a target");
        eprintln!("UNVERIFIED: cross-volume file move safely refused because Windows SACL inspection was unavailable or an audit SACL was present");
        "unverified_windows_sacl_refusal_source_retained"
    };

    let ordinary_restore_root = source_volume.join("ordinary-restore-faults");
    fs::create_dir_all(&ordinary_restore_root).expect("create ordinary restore fault root");
    let (prepared_source, prepared_target, prepared_log) =
        ordinary_restore_fixture(&db, &ordinary_restore_root, &run_id, "prepared");
    set_operation_test_fault(Some(
        OperationTestFaultPoint::AfterRestoreJournalPreparedBeforeClaim,
    ));
    let prepared_panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = restore_moves_with_persistence(
            &db,
            RestoreMovesRequest {
                logs: vec![prepared_log.clone()],
            },
        );
    }));
    set_operation_test_fault(None);
    assert!(prepared_panic.is_err());
    let prepared_pending = db
        .get_pending_restore_logs()
        .expect("read prepared restore journal")
        .into_iter()
        .find(|log| log.id == prepared_log.id)
        .expect("prepared restore journal");
    assert_eq!(prepared_pending.restore_phase, "prepared");
    assert!(prepared_pending
        .restore_claim_path
        .as_deref()
        .is_some_and(|path| !Path::new(path).exists()));
    assert!(!prepared_source.exists());
    assert!(prepared_target.exists());
    assert_eq!(
        reconcile_pending_operation_journal(&db).expect("reconcile prepared restore"),
        1
    );
    let prepared_recovered = db
        .get_operation_logs(Some(200))
        .expect("read prepared restore result")
        .into_iter()
        .find(|log| log.id == prepared_log.id)
        .expect("prepared restore result");
    assert_eq!(prepared_recovered.restore_status, "not_restored");
    assert_eq!(prepared_recovered.restore_phase, "rolled_back");
    assert!(prepared_recovered.can_restore);

    let (claimed_source, claimed_target, claimed_log) =
        ordinary_restore_fixture(&db, &ordinary_restore_root, &run_id, "claimed");
    set_operation_test_fault(Some(
        OperationTestFaultPoint::AfterRestoreSourceClaimedBeforeTargetCommit,
    ));
    let claimed_panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = restore_moves_with_persistence(
            &db,
            RestoreMovesRequest {
                logs: vec![claimed_log.clone()],
            },
        );
    }));
    set_operation_test_fault(None);
    assert!(claimed_panic.is_err());
    let claimed_pending = db
        .get_pending_restore_logs()
        .expect("read source-claimed restore journal")
        .into_iter()
        .find(|log| log.id == claimed_log.id)
        .expect("source-claimed restore journal");
    assert_eq!(claimed_pending.restore_phase, "source_claimed");
    let ordinary_restore_claim_path = claimed_pending
        .restore_claim_path
        .clone()
        .expect("persisted ordinary restore claim path");
    assert!(Path::new(&ordinary_restore_claim_path).exists());
    assert!(!claimed_source.exists());
    assert!(!claimed_target.exists());
    assert_eq!(
        reconcile_pending_operation_journal(&db).expect("reconcile source-claimed restore"),
        1
    );
    let claimed_recovered = db
        .get_operation_logs(Some(200))
        .expect("read source-claimed restore result")
        .into_iter()
        .find(|log| log.id == claimed_log.id)
        .expect("source-claimed restore result");
    assert_eq!(claimed_recovered.status, "manual_review");
    assert_eq!(claimed_recovered.restore_phase, "source_claimed");
    assert_eq!(claimed_recovered.restore_status, "manual_review");
    assert!(claimed_recovered
        .restore_claim_path
        .as_deref()
        .is_some_and(|path| Path::new(path).exists()));
    assert!(!claimed_source.exists());
    assert!(!claimed_target.exists());

    let (committed_source, committed_target, committed_log) =
        ordinary_restore_fixture(&db, &ordinary_restore_root, &run_id, "committed");
    set_operation_test_fault(Some(
        OperationTestFaultPoint::AfterRestoreTargetCommittedBeforeFinalPersist,
    ));
    let committed_panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = restore_moves_with_persistence(
            &db,
            RestoreMovesRequest {
                logs: vec![committed_log.clone()],
            },
        );
    }));
    set_operation_test_fault(None);
    assert!(committed_panic.is_err());
    assert!(committed_source.exists());
    assert!(!committed_target.exists());
    let committed_pending = db
        .get_pending_restore_logs()
        .expect("read target-committed restore journal")
        .into_iter()
        .find(|log| log.id == committed_log.id)
        .expect("target-committed restore journal");
    assert_eq!(committed_pending.restore_phase, "target_committed");
    assert_eq!(
        reconcile_pending_operation_journal(&db).expect("reconcile target-committed restore"),
        1
    );
    let committed_recovered = db
        .get_operation_logs(Some(200))
        .expect("read target-committed restore result")
        .into_iter()
        .find(|log| log.id == committed_log.id)
        .expect("target-committed restore result");
    assert_eq!(committed_recovered.restore_status, "restored");
    assert_eq!(committed_recovered.restore_phase, "completed");

    let (completed_source, completed_target, completed_log) =
        ordinary_restore_fixture(&db, &ordinary_restore_root, &run_id, "completed");
    set_operation_test_fault(Some(
        OperationTestFaultPoint::AfterRestoreCompletedPhaseBeforeFinalTransaction,
    ));
    let completed_panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = restore_moves_with_persistence(
            &db,
            RestoreMovesRequest {
                logs: vec![completed_log.clone()],
            },
        );
    }));
    set_operation_test_fault(None);
    assert!(completed_panic.is_err());
    assert!(completed_source.exists());
    assert!(!completed_target.exists());
    let completed_pending = db
        .get_pending_restore_logs()
        .expect("read completed-phase restore journal")
        .into_iter()
        .find(|log| log.id == completed_log.id)
        .expect("completed-phase restore journal");
    assert_eq!(completed_pending.restore_phase, "completed");
    assert_eq!(
        reconcile_pending_operation_journal(&db).expect("reconcile completed restore"),
        1
    );
    let completed_recovered = db
        .get_operation_logs(Some(200))
        .expect("read completed-phase restore result")
        .into_iter()
        .find(|log| log.id == completed_log.id)
        .expect("completed-phase restore result");
    assert_eq!(completed_recovered.restore_status, "restored");
    assert_eq!(completed_recovered.restore_phase, "completed");

    let (target_replacement_source, target_replacement_target, target_replacement_log) =
        ordinary_restore_fixture(&db, &ordinary_restore_root, &run_id, "target-replacement");
    set_operation_test_fault(Some(
        OperationTestFaultPoint::AfterRestoreTargetCommittedBeforeFinalPersist,
    ));
    let target_replacement_panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = restore_moves_with_persistence(
            &db,
            RestoreMovesRequest {
                logs: vec![target_replacement_log.clone()],
            },
        );
    }));
    set_operation_test_fault(None);
    assert!(target_replacement_panic.is_err());
    assert!(target_replacement_source.exists());
    assert!(!target_replacement_target.exists());
    fs::write(&target_replacement_source, b"replaced restore target")
        .expect("replace committed restore target");
    assert_eq!(
        reconcile_pending_operation_journal(&db).expect("reconcile replaced restore target"),
        1
    );
    let target_replacement_recovered = db
        .get_operation_logs(Some(300))
        .expect("read replaced restore target result")
        .into_iter()
        .find(|log| log.id == target_replacement_log.id)
        .expect("replaced restore target result");
    assert_eq!(target_replacement_recovered.status, "manual_review");
    assert_eq!(
        target_replacement_recovered.restore_phase,
        "target_committed"
    );
    assert_eq!(
        target_replacement_recovered.restore_error.as_deref(),
        Some(
            "target_committed_identity_mismatch: restore target or source identity is mismatched or unreadable; do not auto retry."
        )
    );
    assert_eq!(
        fs::read(&target_replacement_source).expect("read replaced restore target"),
        b"replaced restore target"
    );

    let (claim_replacement_source, claim_replacement_target, claim_replacement_log) =
        ordinary_restore_fixture(&db, &ordinary_restore_root, &run_id, "claim-replacement");
    set_operation_test_fault(Some(
        OperationTestFaultPoint::AfterRestoreSourceClaimedBeforeTargetCommit,
    ));
    let claim_replacement_panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = restore_moves_with_persistence(
            &db,
            RestoreMovesRequest {
                logs: vec![claim_replacement_log.clone()],
            },
        );
    }));
    set_operation_test_fault(None);
    assert!(claim_replacement_panic.is_err());
    let claim_replacement_pending = db
        .get_pending_restore_logs()
        .expect("read claim replacement restore journal")
        .into_iter()
        .find(|log| log.id == claim_replacement_log.id)
        .expect("claim replacement restore journal");
    let claim_replacement_path = PathBuf::from(
        claim_replacement_pending
            .restore_claim_path
            .as_deref()
            .expect("claim replacement path"),
    );
    fs::remove_file(&claim_replacement_path).expect("remove original restore claim");
    fs::write(&claim_replacement_path, b"replacement claim")
        .expect("write replacement restore claim");
    assert_eq!(
        reconcile_pending_operation_journal(&db).expect("reconcile replaced restore claim"),
        1
    );
    let claim_replacement_recovered = db
        .get_operation_logs(Some(300))
        .expect("read replaced restore claim result")
        .into_iter()
        .find(|log| log.id == claim_replacement_log.id)
        .expect("replaced restore claim result");
    assert_eq!(claim_replacement_recovered.status, "manual_review");
    assert_eq!(claim_replacement_recovered.restore_phase, "source_claimed");
    assert_eq!(
        claim_replacement_recovered.restore_error.as_deref(),
        Some(
            "claim_identity_mismatch: persisted restore claim identity is mismatched or unreadable; do not auto retry."
        )
    );
    assert!(!claim_replacement_source.exists());
    assert!(!claim_replacement_target.exists());
    assert_eq!(
        fs::read(&claim_replacement_path).expect("read replacement restore claim"),
        b"replacement claim"
    );

    let (both_paths_source, both_paths_target, both_paths_log) =
        ordinary_restore_fixture(&db, &ordinary_restore_root, &run_id, "target-and-claim");
    set_operation_test_fault(Some(
        OperationTestFaultPoint::AfterRestoreSourceClaimedBeforeTargetCommit,
    ));
    let both_paths_panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = restore_moves_with_persistence(
            &db,
            RestoreMovesRequest {
                logs: vec![both_paths_log.clone()],
            },
        );
    }));
    set_operation_test_fault(None);
    assert!(both_paths_panic.is_err());
    let both_paths_pending = db
        .get_pending_restore_logs()
        .expect("read target and claim restore journal")
        .into_iter()
        .find(|log| log.id == both_paths_log.id)
        .expect("target and claim restore journal");
    let both_paths_claim = PathBuf::from(
        both_paths_pending
            .restore_claim_path
            .as_deref()
            .expect("target and claim path"),
    );
    fs::hard_link(&both_paths_claim, &both_paths_source)
        .expect("create identity-preserving target and claim fixture");
    assert!(!both_paths_target.exists());
    assert_eq!(
        reconcile_pending_operation_journal(&db).expect("reconcile target and claim"),
        1
    );
    let both_paths_recovered = db
        .get_operation_logs(Some(300))
        .expect("read target and claim result")
        .into_iter()
        .find(|log| log.id == both_paths_log.id)
        .expect("target and claim result");
    assert_eq!(both_paths_recovered.status, "manual_review");
    assert_eq!(both_paths_recovered.restore_phase, "source_cleanup_pending");
    assert_eq!(
        both_paths_recovered.restore_error.as_deref(),
        Some(
            "target_committed_source_cleanup_pending: restored target and restore claim both exist; do not auto retry source cleanup."
        )
    );
    assert!(both_paths_source.exists());
    assert!(both_paths_claim.exists());
    let reentry = restore_moves_with_persistence(
        &db,
        RestoreMovesRequest {
            logs: vec![both_paths_recovered.clone()],
        },
    )
    .expect_err("restore reentry must fail closed for target and claim");
    assert!(reentry.contains("restore_pending_reconciliation"));
    assert!(both_paths_source.exists());
    assert!(both_paths_claim.exists());

    let (all_missing_source, all_missing_target, all_missing_log) =
        ordinary_restore_fixture(&db, &ordinary_restore_root, &run_id, "all-paths-missing");
    set_operation_test_fault(Some(
        OperationTestFaultPoint::AfterRestoreSourceClaimedBeforeTargetCommit,
    ));
    let all_missing_panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = restore_moves_with_persistence(
            &db,
            RestoreMovesRequest {
                logs: vec![all_missing_log.clone()],
            },
        );
    }));
    set_operation_test_fault(None);
    assert!(all_missing_panic.is_err());
    let all_missing_pending = db
        .get_pending_restore_logs()
        .expect("read all missing restore journal")
        .into_iter()
        .find(|log| log.id == all_missing_log.id)
        .expect("all missing restore journal");
    let all_missing_claim = PathBuf::from(
        all_missing_pending
            .restore_claim_path
            .as_deref()
            .expect("all missing claim path"),
    );
    fs::remove_file(&all_missing_claim).expect("remove all missing claim");
    assert!(!all_missing_source.exists());
    assert!(!all_missing_target.exists());
    assert_eq!(
        reconcile_pending_operation_journal(&db).expect("reconcile all missing restore"),
        1
    );
    let all_missing_recovered = db
        .get_operation_logs(Some(300))
        .expect("read all missing restore result")
        .into_iter()
        .find(|log| log.id == all_missing_log.id)
        .expect("all missing restore result");
    assert_eq!(all_missing_recovered.status, "manual_review");
    assert_eq!(all_missing_recovered.restore_status, "manual_review");
    assert_eq!(all_missing_recovered.restore_phase, "target_committed");
    assert!(!all_missing_recovered.can_restore);
    assert!(all_missing_recovered
        .restore_error
        .as_deref()
        .is_some_and(|error| error.starts_with("target_committed_durability_unknown:")));

    let reconcile_source = source_volume.join("reconcile-source.txt");
    let reconcile_target = source_volume.join("reconcile-target.txt");
    let reconcile_operation_id = format!("native-reconcile-{run_id}");
    fs::write(&reconcile_source, b"reconciliation payload").expect("write reconcile source");
    let reconcile_move = execute_moves_with_persistence(
        &db,
        ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: reconcile_operation_id.clone(),
                file_id: format!("native-reconcile-file-{run_id}"),
                operation_type: "move".to_string(),
                source_path: reconcile_source.to_string_lossy().into_owned(),
                target_path: reconcile_target.to_string_lossy().into_owned(),
                old_name: "reconcile-source.txt".to_string(),
                new_name: "reconcile-target.txt".to_string(),
                is_executable: Some(true),
            }],
        },
    )
    .expect("execute reconciliation fixture");
    let mut interrupted = reconcile_move.logs[0].clone();
    let reconcile_log_id = interrupted.id.clone();
    interrupted.status = "pending".to_string();
    interrupted.operation_phase = "target_committed".to_string();
    db.save_operation_logs(&interrupted.batch_id, std::slice::from_ref(&interrupted))
        .expect("simulate interrupted operation journal");
    assert_eq!(
        reconcile_pending_operation_journal(&db).expect("reconcile operation"),
        1
    );
    let reconciled_operation = db
        .get_operation_logs(Some(200))
        .expect("read reconciled operation")
        .into_iter()
        .find(|log| log.id == reconcile_log_id)
        .expect("reconciled operation log");
    assert_eq!(reconciled_operation.status, "manual_review");
    assert_eq!(reconciled_operation.operation_phase, "target_committed");

    let rename_source = source_volume.join("rename-before.txt");
    let rename_target = source_volume.join("rename-after.txt");
    fs::write(&rename_source, b"native rename payload").expect("write rename source");
    let renamed = execute_moves_with_persistence(
        &db,
        ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: format!("native-rename-{run_id}"),
                file_id: format!("native-rename-file-{run_id}"),
                operation_type: "rename".to_string(),
                source_path: rename_source.to_string_lossy().into_owned(),
                target_path: rename_target.to_string_lossy().into_owned(),
                old_name: "rename-before.txt".to_string(),
                new_name: "rename-after.txt".to_string(),
                is_executable: Some(true),
            }],
        },
    )
    .expect("execute production persisted rename");
    assert_eq!(renamed.logs[0].status, "success");
    assert_eq!(
        fs::read(&rename_target).expect("renamed target"),
        b"native rename payload"
    );

    let conflict_source = source_volume.join("conflict-source.txt");
    let conflict_target = source_volume.join("conflict-target.txt");
    fs::write(&conflict_source, b"conflict source").expect("conflict source");
    fs::write(&conflict_target, b"competing target").expect("conflict target");
    assert!(matches!(
        atomic_move_noreplace(&conflict_source, &conflict_target, None, None),
        Err(AtomicMoveError::TargetExists)
    ));
    assert_eq!(
        fs::read(&conflict_source).expect("source retained"),
        b"conflict source"
    );
    assert_eq!(
        fs::read(&conflict_target).expect("target retained"),
        b"competing target"
    );

    let replacement_source = source_volume.join("replacement-source.txt");
    let replacement_target = source_volume.join("replacement-target.txt");
    fs::write(&replacement_source, b"replacement original").expect("replacement original");
    let replacement_expected =
        capture_identity(&replacement_source, None).expect("replacement expected identity");
    fs::write(&replacement_source, b"replacement changed!").expect("replace source");
    assert!(matches!(
        atomic_move_noreplace(
            &replacement_source,
            &replacement_target,
            Some(&replacement_expected),
            None,
        ),
        Err(AtomicMoveError::SourceChanged)
    ));
    assert_eq!(
        fs::read(&replacement_source).expect("replacement retained"),
        b"replacement changed!"
    );
    assert!(!replacement_target.exists());

    let bound_source = source_volume.join("bound-parent-source.txt");
    let bound_parent = source_volume.join("bound-parent");
    let displaced_parent = source_volume.join("bound-parent-displaced");
    fs::write(&bound_source, b"bound parent payload").expect("bound parent source");
    fs::create_dir(&bound_parent).expect("bound parent");
    let bound_expected = capture_identity(&bound_source, None).expect("bound source identity");
    let mut bound_claim = claim_source(&bound_source, &bound_expected, "native-bound-parent", None)
        .expect("bound source claim");
    let verified_parent =
        VerifiedDirectory::open_existing(&bound_parent).expect("verified target parent");
    fs::rename(&bound_parent, &displaced_parent).expect("displace target parent");
    fs::create_dir(&bound_parent).expect("replacement target parent");
    let parent_error = bound_claim
        .commit_to(verified_parent, std::ffi::OsStr::new("committed.txt"))
        .expect_err("replaced target parent must fail closed");
    assert!(parent_error
        .to_string()
        .contains("verified directory identity changed"));
    bound_claim
        .rollback_to_original()
        .expect("rollback bound source");
    assert_eq!(
        fs::read(&bound_source).expect("bound source retained"),
        b"bound parent payload"
    );
    assert!(!bound_parent.join("committed.txt").exists());
    assert!(!displaced_parent.join("committed.txt").exists());

    let durability_source = source_volume.join("durability-source.txt");
    let durability_target = source_volume.join("durability-target.txt");
    fs::write(&durability_source, b"durability payload").expect("durability source");
    test_faults::set_fault(Some(AtomicFaultPoint::TargetDurability));
    let durability = execute_moves_with_persistence(
        &db,
        ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: format!("native-durability-{run_id}"),
                file_id: format!("native-durability-file-{run_id}"),
                operation_type: "move".to_string(),
                source_path: durability_source.to_string_lossy().into_owned(),
                target_path: durability_target.to_string_lossy().into_owned(),
                old_name: "durability-source.txt".to_string(),
                new_name: "durability-target.txt".to_string(),
                is_executable: Some(true),
            }],
        },
    )
    .expect("durability execution result");
    test_faults::set_fault(None);
    assert_eq!(durability.logs[0].status, "manual_review");
    assert!(durability.logs[0]
        .error_message
        .as_deref()
        .is_some_and(|message| message.contains("target_committed_durability_unknown")));
    assert!(!durability_source.exists());
    assert_eq!(
        fs::read(&durability_target).expect("durability target"),
        b"durability payload"
    );

    let nested_parent = source_volume
        .join("nested-handle-relative")
        .join("Unicode-目录")
        .join("segment-abcdefghijklmnopqrstuvwxyz0123456789")
        .join("segment-результат-αβγ");
    fs::create_dir_all(&nested_parent).expect("create nested handle-relative fixture");
    let nested_source = nested_parent.join("源-данные.txt");
    let nested_target = nested_parent.join("目标-результат.txt");
    fs::write(&nested_source, b"nested handle-relative payload")
        .expect("write nested handle-relative source");
    let nested_result = execute_moves_with_persistence(
        &db,
        ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: format!("native-nested-handle-{run_id}"),
                file_id: format!("native-nested-handle-file-{run_id}"),
                operation_type: "move".to_string(),
                source_path: nested_source.to_string_lossy().into_owned(),
                target_path: nested_target.to_string_lossy().into_owned(),
                old_name: "源-данные.txt".to_string(),
                new_name: "目标-результат.txt".to_string(),
                is_executable: Some(true),
            }],
        },
    )
    .expect("execute nested handle-relative move");
    assert_eq!(nested_result.logs[0].status, "success");
    assert_eq!(
        fs::read(&nested_target).expect("read nested handle-relative target"),
        b"nested handle-relative payload"
    );

    let claim_mismatch_source = source_volume.join("claim-identity-source.txt");
    let claim_mismatch_target = source_volume.join("claim-identity-target.txt");
    fs::write(&claim_mismatch_source, b"claim identity source")
        .expect("write claim identity source");
    let claim_operation_id = format!("native-claim-identity-{run_id}");
    let claim_serial = source_claim::lock_claim_test_hooks();
    source_claim::set_claim_test_hook(Some(replace_claim_before_identity_check));
    let claim_result = execute_moves_with_persistence(
        &db,
        ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: claim_operation_id.clone(),
                file_id: format!("native-claim-identity-file-{run_id}"),
                operation_type: "move".to_string(),
                source_path: claim_mismatch_source.to_string_lossy().into_owned(),
                target_path: claim_mismatch_target.to_string_lossy().into_owned(),
                old_name: "claim-identity-source.txt".to_string(),
                new_name: "claim-identity-target.txt".to_string(),
                is_executable: Some(true),
            }],
        },
    )
    .expect("claim identity recovery result");
    let claim_log_id = claim_result.logs[0].id.clone();
    source_claim::set_claim_test_hook(None);
    drop(claim_serial);
    assert_eq!(claim_result.logs[0].status, "manual_review");
    assert_eq!(claim_result.logs[0].operation_phase, "source_claimed");
    assert!(!claim_mismatch_source.exists());
    assert!(!claim_mismatch_target.exists());
    let claim_path = PathBuf::from(
        claim_result.logs[0]
            .source_claim_path
            .as_deref()
            .expect("claim path in operation journal"),
    );
    assert_eq!(
        fs::read(&claim_path).expect("replacement claim remains for reconciliation"),
        b"replacement claim"
    );
    let mut claim_pending = claim_result.logs[0].clone();
    claim_pending.status = "pending".to_string();
    db.save_operation_logs(
        &claim_pending.batch_id,
        std::slice::from_ref(&claim_pending),
    )
    .expect("persist claim identity recovery as pending");
    assert_eq!(
        reconcile_pending_operation_journal(&db).expect("reconcile claim identity journal"),
        1
    );
    let claim_log = db
        .get_operation_logs(Some(200))
        .expect("read claim identity log")
        .into_iter()
        .find(|log| log.id == claim_log_id)
        .expect("claim identity log");
    assert_eq!(claim_log.status, "manual_review");
    assert_eq!(claim_log.operation_phase, "manual_review");

    let cross_volume_exercised = volume_prefix(&source_volume) != volume_prefix(&target_volume);
    let same_copy_parent = source_volume.join("ordinary-copy-destination");
    fs::create_dir_all(&same_copy_parent).expect("same-volume copy target parent");
    let same_file_source = source_volume.join("ordinary-copy-source.txt");
    let same_file_target = same_copy_parent.join("ordinary-copy-file.txt");
    fs::write(&same_file_source, b"ordinary same-volume copy canary")
        .expect("same-volume Copy source");
    let same_directory_source = source_volume.join("ordinary-copy-tree");
    fs::create_dir_all(same_directory_source.join("nested").join("empty"))
        .expect("same-volume copy tree");
    fs::write(
        same_directory_source.join("nested").join("payload.bin"),
        b"ordinary same-volume nested copy canary",
    )
    .expect("same-volume nested Copy source");
    let same_directory_target = same_copy_parent.join("ordinary-copy-tree");
    let same_copy_result = execute_moves_with_persistence(
        &db,
        ExecuteMovesRequest {
            operations: vec![
                native_copy_operation(&run_id, "same-file", &same_file_source, &same_file_target),
                native_copy_operation(
                    &run_id,
                    "same-directory",
                    &same_directory_source,
                    &same_directory_target,
                ),
            ],
        },
    )
    .expect("execute ordinary same-volume Copy fixtures");
    assert_eq!(same_copy_result.logs.len(), 2);
    assert!(same_copy_result
        .logs
        .iter()
        .all(|log| log.status == "success"));
    assert!(
        same_file_source.is_file(),
        "same-volume Copy must retain its source"
    );
    assert!(
        same_directory_source.is_dir(),
        "same-volume directory Copy must retain its source"
    );
    assert_eq!(
        sha256_file(&same_file_source),
        sha256_file(&same_file_target),
        "same-volume copied file content must match"
    );
    assert_eq!(
        sha256_file(&same_directory_source.join("nested").join("payload.bin")),
        sha256_file(&same_directory_target.join("nested").join("payload.bin")),
        "same-volume copied nested directory content must match"
    );
    assert!(same_directory_target.join("nested").join("empty").is_dir());

    let same_move_parent = source_volume.join("ordinary-move-destination");
    fs::create_dir_all(&same_move_parent).expect("same-volume Move destination parent");
    let same_move_file_source = source_volume.join("ordinary-move-source.txt");
    let same_move_file_target = same_move_parent.join("ordinary-move-source.txt");
    fs::write(&same_move_file_source, b"ordinary same-volume Move canary")
        .expect("same-volume Move file source");
    let same_move_directory_source = source_volume.join("ordinary-move-tree");
    fs::create_dir_all(same_move_directory_source.join("nested").join("empty"))
        .expect("same-volume Move tree");
    fs::write(
        same_move_directory_source
            .join("nested")
            .join("payload.bin"),
        b"ordinary same-volume nested Move canary",
    )
    .expect("same-volume nested Move file");
    let same_move_directory_target = same_move_parent.join("ordinary-move-tree");
    let same_move_file_hash = sha256_file(&same_move_file_source);
    let same_move_nested_hash = sha256_file(
        &same_move_directory_source
            .join("nested")
            .join("payload.bin"),
    );
    let same_move_result = execute_moves_with_persistence(
        &db,
        ExecuteMovesRequest {
            operations: vec![
                native_qa_transfer_move_preview(&same_move_file_source, &same_move_parent)
                    .expect("same-volume file Move Preview"),
                native_qa_transfer_move_preview(&same_move_directory_source, &same_move_parent)
                    .expect("same-volume directory Move Preview"),
            ],
        },
    )
    .expect("execute ordinary same-volume Move fixtures");
    assert_eq!(same_move_result.logs.len(), 2);
    assert!(same_move_result
        .logs
        .iter()
        .all(|log| log.status == "success"));
    assert!(same_move_result.logs.iter().all(|log| log.can_restore));
    assert_eq!(same_move_result.logs[0].operation_phase, "completed");
    assert_eq!(same_move_result.logs[1].operation_phase, "completed");
    assert!(!same_move_file_source.exists());
    assert!(!same_move_directory_source.exists());
    assert_eq!(sha256_file(&same_move_file_target), same_move_file_hash);
    assert_eq!(
        sha256_file(
            &same_move_directory_target
                .join("nested")
                .join("payload.bin")
        ),
        same_move_nested_hash
    );
    assert!(same_move_directory_target
        .join("nested")
        .join("empty")
        .is_dir());
    let same_move_restore = restore_moves_with_persistence(
        &db,
        RestoreMovesRequest {
            logs: same_move_result.logs,
        },
    )
    .expect("restore same-volume file and directory Move through History");
    assert_eq!(same_move_restore.restored, 2);
    assert_eq!(same_move_restore.failed, 0);
    assert!(same_move_file_source.is_file());
    assert!(same_move_directory_source
        .join("nested")
        .join("empty")
        .is_dir());
    assert_eq!(sha256_file(&same_move_file_source), same_move_file_hash);
    assert_eq!(
        sha256_file(
            &same_move_directory_source
                .join("nested")
                .join("payload.bin")
        ),
        same_move_nested_hash
    );
    assert!(!same_move_file_target.exists());
    assert!(!same_move_directory_target.exists());

    let ordinary_same_volume_file_copy = "passed_source_retained_content_hash_matched";
    let ordinary_same_volume_directory_copy = "passed_source_retained_nested_hash_matched";
    let mut ordinary_cross_volume_file_copy = "unverified_no_secondary_volume";
    let mut ordinary_cross_volume_directory_copy = "unverified_no_secondary_volume";
    let ordinary_same_volume_file_move = "passed_move_and_history_restore";
    let ordinary_same_volume_directory_move = "passed_move_and_history_restore";
    let mut ordinary_cross_volume_file_move = "unverified_no_secondary_volume";
    let mut windows_sacl_inspection = "unverified_no_secondary_volume";
    let mut cross_volume_directory_move = "unverified_no_secondary_volume";
    if cross_volume_exercised {
        let cross_copy_parent = target_volume.join("ordinary-copy-destination");
        fs::create_dir_all(&cross_copy_parent).expect("cross-volume copy target parent");
        let cross_file_source = source_volume.join("ordinary-cross-copy-source.txt");
        let cross_file_target = cross_copy_parent.join("ordinary-cross-copy-file.txt");
        fs::write(&cross_file_source, b"ordinary cross-volume copy canary")
            .expect("cross-volume Copy source");
        let cross_directory_source = source_volume.join("ordinary-cross-copy-tree");
        fs::create_dir_all(cross_directory_source.join("nested").join("empty"))
            .expect("cross-volume Copy tree");
        fs::write(
            cross_directory_source.join("nested").join("payload.bin"),
            b"ordinary cross-volume nested copy canary",
        )
        .expect("cross-volume nested Copy source");
        let cross_directory_target = cross_copy_parent.join("ordinary-cross-copy-tree");
        let cross_copy_result = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![
                    native_copy_operation(
                        &run_id,
                        "cross-file",
                        &cross_file_source,
                        &cross_file_target,
                    ),
                    native_copy_operation(
                        &run_id,
                        "cross-directory",
                        &cross_directory_source,
                        &cross_directory_target,
                    ),
                ],
            },
        )
        .expect("execute ordinary cross-volume Copy fixtures under the runner token");
        assert_eq!(cross_copy_result.logs.len(), 2);
        assert!(
            cross_copy_result
                .logs
                .iter()
                .all(|log| log.status == "success"),
            "ordinary Copy must not depend on SeSecurityPrivilege: {:?}",
            cross_copy_result
                .logs
                .iter()
                .map(|log| &log.error_message)
                .collect::<Vec<_>>()
        );
        assert!(
            cross_file_source.is_file(),
            "cross-volume Copy must retain its source"
        );
        assert!(
            cross_directory_source.is_dir(),
            "cross-volume directory Copy must retain its source"
        );
        assert_eq!(
            sha256_file(&cross_file_source),
            sha256_file(&cross_file_target),
            "cross-volume copied file content must match"
        );
        assert_eq!(
            sha256_file(&cross_directory_source.join("nested").join("payload.bin")),
            sha256_file(&cross_directory_target.join("nested").join("payload.bin")),
            "cross-volume copied nested directory content must match"
        );
        assert!(cross_directory_target.join("nested").join("empty").is_dir());
        ordinary_cross_volume_file_copy = "passed_source_retained_content_hash_matched";
        ordinary_cross_volume_directory_copy = "passed_source_retained_nested_hash_matched";

        let cross_move_file_source = source_volume.join("ordinary-cross-volume-move.txt");
        let cross_move_file_target = target_volume.join("ordinary-cross-volume-move.txt");
        fs::write(
            &cross_move_file_source,
            b"ordinary cross-volume Move canary",
        )
        .expect("cross-volume Move file source");
        let cross_move_file_hash = sha256_file(&cross_move_file_source);
        let cross_move_result = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![native_qa_transfer_move_preview(
                    &cross_move_file_source,
                    &target_volume,
                )
                .expect("cross-volume file Move Preview")],
            },
        )
        .expect("execute journaled cross-volume file Move");
        assert_eq!(cross_move_result.logs.len(), 1);
        if cross_move_result.logs[0].status == "success" {
            assert_eq!(cross_move_result.logs[0].operation_phase, "completed");
            assert!(cross_move_result.logs[0].can_restore);
            assert!(!cross_move_file_source.exists());
            assert_eq!(sha256_file(&cross_move_file_target), cross_move_file_hash);
            let restored = restore_moves_with_persistence(
                &db,
                RestoreMovesRequest {
                    logs: cross_move_result.logs,
                },
            )
            .expect("restore cross-volume file Move through History");
            assert_eq!(restored.restored, 1);
            assert_eq!(restored.failed, 0);
            assert!(cross_move_file_source.is_file());
            assert_eq!(sha256_file(&cross_move_file_source), cross_move_file_hash);
            assert!(!cross_move_file_target.exists());
            ordinary_cross_volume_file_move = "passed_move_and_history_restore";
        } else {
            let error = cross_move_result.logs[0]
                .error_message
                .as_deref()
                .unwrap_or_default();
            assert!(
                error.contains("windows_sacl_"),
                "unexpected ordinary cross-volume file Move refusal: {error}"
            );
            assert!(cross_move_file_source.is_file());
            assert_eq!(sha256_file(&cross_move_file_source), cross_move_file_hash);
            assert!(
                !cross_move_file_target.exists(),
                "cross-volume Move refusal must not publish a target"
            );
            ordinary_cross_volume_file_move = "unverified_sacl_refusal_source_retained";
        }

        let directory_source = source_volume.join("cross-volume-directory");
        let directory_target = target_volume.join("cross-volume-directory");
        fs::create_dir_all(directory_source.join("nested").join("empty"))
            .expect("cross-volume directory source and empty leaf");
        fs::write(directory_source.join("canary.txt"), b"directory canary")
            .expect("directory canary");
        fs::write(
            directory_source.join("nested").join("内容.bin"),
            b"nested cross-volume canary",
        )
        .expect("nested directory canary");
        let inheritance = Command::new("icacls")
            .arg(&directory_source)
            .args(["/inheritance:d"])
            .output()
            .expect("make the cross-volume source DACL explicit");
        assert!(
            inheritance.status.success(),
            "could not make the source DACL explicit: {}",
            String::from_utf8_lossy(&inheritance.stderr).trim()
        );
        let explicit_acl = Command::new("icacls")
            .arg(&directory_source)
            .args(["/grant", "*S-1-5-32-545:(OI)(CI)(RX)"])
            .output()
            .expect("add an explicit cross-volume source DACL");
        assert!(
            explicit_acl.status.success(),
            "could not add the explicit source DACL: {}",
            String::from_utf8_lossy(&explicit_acl.stderr).trim()
        );
        let canary_hash = sha256_file(&directory_source.join("canary.txt"));
        let nested_hash = sha256_file(&directory_source.join("nested").join("内容.bin"));
        let move_preview = native_qa_transfer_move_preview(&directory_source, &target_volume)
            .expect("build transfer preview for the isolated directory move");
        let canonical_directory_target = target_volume
            .canonicalize()
            .expect("canonicalize target volume")
            .join("cross-volume-directory");
        assert_eq!(
            move_preview.target_path,
            canonical_directory_target
                .to_string_lossy()
                .replace('\\', "/")
        );
        let move_result = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![move_preview],
            },
        )
        .expect("execute journaled cross-volume directory move");
        assert_eq!(move_result.logs.len(), 1);
        if move_result.logs[0].status == "success" {
            windows_sacl_inspection = "verified_absent_for_all_entries";
            assert_eq!(move_result.logs[0].operation_phase, "completed");
            assert!(move_result.logs[0].can_restore);
            assert!(!directory_source.exists());
            assert_eq!(
                sha256_file(&directory_target.join("canary.txt")),
                canary_hash,
                "published file contents must match the source hash"
            );
            assert_eq!(
                sha256_file(&directory_target.join("nested").join("内容.bin")),
                nested_hash,
                "published nested file contents must match the source hash"
            );
            assert!(directory_target.join("nested").join("empty").is_dir());

            let directory_restore = restore_moves_with_persistence(
                &db,
                RestoreMovesRequest {
                    logs: move_result.logs,
                },
            )
            .expect("restore cross-volume directory through History");
            assert_eq!(directory_restore.restored, 1);
            assert_eq!(directory_restore.failed, 0);
            assert_eq!(directory_restore.logs[0].restore_status, "restored");
            assert!(directory_source.join("nested").join("empty").is_dir());
            assert_eq!(
                sha256_file(&directory_source.join("canary.txt")),
                canary_hash
            );
            assert_eq!(
                sha256_file(&directory_source.join("nested").join("内容.bin")),
                nested_hash
            );
            assert!(!directory_target.exists());
            cross_volume_directory_move = "passed_move_and_history_restore";
        } else {
            let error = move_result.logs[0]
                .error_message
                .as_deref()
                .unwrap_or_default();
            assert!(
                error.contains("windows_sacl_"),
                "unexpected cross-volume refusal: {error}"
            );
            assert!(
                directory_source.exists(),
                "safe refusal must retain directory source"
            );
            assert!(
                !directory_target.exists(),
                "safe refusal must not publish directory target"
            );
            if error.contains("windows_sacl_transfer_unsupported") {
                windows_sacl_inspection = "verified_audit_sacl_present_refused";
            } else {
                windows_sacl_inspection = "unavailable_refused_before_mutation";
            }
            eprintln!("UNVERIFIED: cross-volume directory move safely refused; source and target state checked: {error}");
            cross_volume_directory_move = "unverified_sacl_refusal_source_retained";
        }
    }

    let cleanup_source = target_volume.join("node_modules");
    fs::create_dir_all(cleanup_source.join("package")).expect("create cleanup fixture");
    fs::write(
        cleanup_source.join("package").join("index.js"),
        b"safe trash payload",
    )
    .expect("write cleanup fixture");
    let cleanup = classify_candidate_for_test(&cleanup_source, b"safe trash payload".len() as u64);
    let system_trash_preview = preview_cleanup_operations_for_candidates(
        vec![cleanup.id.clone()],
        std::slice::from_ref(&cleanup),
        Some(&app_data),
    )
    .expect("preview system trash fallback");
    assert_eq!(system_trash_preview.previews.len(), 1);
    assert_eq!(system_trash_preview.previews[0].is_executable, Some(false));
    assert_eq!(
        system_trash_preview.previews[0].blocking_reason.as_deref(),
        Some("system_trash_source_binding_unsupported")
    );
    let cleanup_result = move_cleanup_candidates_to_safe_trash_for_candidates(
        vec![cleanup.id.clone()],
        std::slice::from_ref(&cleanup),
        &db,
        Some(&app_data),
    )
    .expect("move through production Safe Trash");
    assert_eq!(
        cleanup_result.moved,
        1,
        "{}",
        serde_json::to_string(&cleanup_result).expect("cleanup result JSON")
    );
    let mut trash_item =
        db.list_cleanup_trash_batches().expect("list Safe Trash")[0].items[0].clone();
    trash_item.status = "pending".to_string();
    trash_item.operation_phase = "prepared".to_string();
    db.update_cleanup_trash_item_status(&trash_item)
        .expect("simulate interrupted Safe Trash journal");
    assert!(reconcile_pending_cleanup_journal(&db).expect("reconcile Safe Trash") >= 1);
    let cleanup_restore = restore_cleanup_trash_items_for_db(vec![trash_item.id], &db)
        .expect("restore through production Safe Trash");
    assert_eq!(cleanup_restore.restored, 1);
    assert!(cleanup_source.exists());

    let safe_trash_cases_root = target_volume.join("safe-trash-hardening-cases");

    let safe_trash_durability =
        native_cleanup_candidate(&safe_trash_cases_root, "durability", b"durability case");
    let safe_trash_durability_result = {
        test_faults::set_fault(Some(AtomicFaultPoint::TargetDurability));
        let result = move_cleanup_candidates_to_safe_trash_for_candidates(
            vec![safe_trash_durability.id.clone()],
            std::slice::from_ref(&safe_trash_durability),
            &db,
            Some(&app_data),
        )
        .expect("Safe Trash durability fault result");
        test_faults::set_fault(None);
        result
    };
    assert_eq!(safe_trash_durability_result.moved, 0);
    assert_eq!(safe_trash_durability_result.failed, 1);
    assert!(safe_trash_durability_result.logs[0]
        .message
        .contains("target_committed_durability_unknown"));
    let safe_trash_durability_item_id = safe_trash_durability_result.logs[0]
        .item_id
        .clone()
        .expect("Safe Trash durability item id");
    let safe_trash_durability_item = cleanup_item(&db, &safe_trash_durability_item_id);
    assert_eq!(safe_trash_durability_item.status, "manual_review");
    assert_eq!(
        safe_trash_durability_item.operation_phase,
        "target_committed"
    );
    assert!(!Path::new(&safe_trash_durability_item.original_path).exists());
    assert!(Path::new(&safe_trash_durability_item.trash_path).exists());
    let mut safe_trash_durability_pending = safe_trash_durability_item.clone();
    safe_trash_durability_pending.status = "pending".to_string();
    db.update_cleanup_trash_item_status(&safe_trash_durability_pending)
        .expect("persist Safe Trash durability pending restart state");
    assert!(reconcile_pending_cleanup_journal(&db).expect("reconcile Safe Trash durability") >= 1);
    let safe_trash_durability_recovered = cleanup_item(&db, &safe_trash_durability_item_id);
    assert_eq!(safe_trash_durability_recovered.status, "manual_review");
    assert_eq!(
        safe_trash_durability_recovered.operation_phase,
        "target_committed"
    );

    let safe_trash_identity =
        native_cleanup_candidate(&safe_trash_cases_root, "identity", b"identity case");
    let safe_trash_identity_result = {
        test_faults::set_fault(Some(AtomicFaultPoint::TargetIdentity));
        let result = move_cleanup_candidates_to_safe_trash_for_candidates(
            vec![safe_trash_identity.id.clone()],
            std::slice::from_ref(&safe_trash_identity),
            &db,
            Some(&app_data),
        )
        .expect("Safe Trash identity fault result");
        test_faults::set_fault(None);
        result
    };
    assert_eq!(safe_trash_identity_result.moved, 0);
    assert_eq!(safe_trash_identity_result.failed, 1);
    assert!(safe_trash_identity_result.logs[0]
        .message
        .contains("target_committed_identity_mismatch"));
    let safe_trash_identity_item_id = safe_trash_identity_result.logs[0]
        .item_id
        .clone()
        .expect("Safe Trash identity item id");
    let safe_trash_identity_item = cleanup_item(&db, &safe_trash_identity_item_id);
    assert_eq!(safe_trash_identity_item.status, "manual_review");
    assert_eq!(safe_trash_identity_item.operation_phase, "target_committed");
    assert!(!Path::new(&safe_trash_identity_item.original_path).exists());
    assert!(Path::new(&safe_trash_identity_item.trash_path).exists());
    let mut safe_trash_identity_pending = safe_trash_identity_item.clone();
    safe_trash_identity_pending.status = "pending".to_string();
    db.update_cleanup_trash_item_status(&safe_trash_identity_pending)
        .expect("persist Safe Trash identity pending restart state");
    assert!(reconcile_pending_cleanup_journal(&db).expect("reconcile Safe Trash identity") >= 1);
    assert_eq!(
        cleanup_item(&db, &safe_trash_identity_item_id).status,
        "manual_review"
    );

    let safe_trash_source_cleanup = native_cleanup_candidate(
        &safe_trash_cases_root,
        "source-cleanup",
        b"source cleanup case",
    );
    let safe_trash_source_cleanup_result = {
        test_faults::set_fault(Some(AtomicFaultPoint::SourceCleanup));
        let result = move_cleanup_candidates_to_safe_trash_for_candidates(
            vec![safe_trash_source_cleanup.id.clone()],
            std::slice::from_ref(&safe_trash_source_cleanup),
            &db,
            Some(&app_data),
        )
        .expect("Safe Trash source cleanup fault result");
        test_faults::set_fault(None);
        result
    };
    assert_eq!(safe_trash_source_cleanup_result.moved, 0);
    assert_eq!(safe_trash_source_cleanup_result.failed, 1);
    assert!(safe_trash_source_cleanup_result.logs[0]
        .message
        .contains("target_committed_source_cleanup_pending"));
    let safe_trash_source_cleanup_item_id = safe_trash_source_cleanup_result.logs[0]
        .item_id
        .clone()
        .expect("Safe Trash source cleanup item id");
    let safe_trash_source_cleanup_item = cleanup_item(&db, &safe_trash_source_cleanup_item_id);
    assert_eq!(
        safe_trash_source_cleanup_item.operation_phase,
        "source_cleanup_pending"
    );
    assert_eq!(safe_trash_source_cleanup_item.status, "manual_review");
    assert!(!Path::new(&safe_trash_source_cleanup_item.original_path).exists());
    assert!(Path::new(&safe_trash_source_cleanup_item.trash_path).exists());
    let mut safe_trash_source_cleanup_pending = safe_trash_source_cleanup_item.clone();
    safe_trash_source_cleanup_pending.status = "pending".to_string();
    db.update_cleanup_trash_item_status(&safe_trash_source_cleanup_pending)
        .expect("persist Safe Trash source cleanup pending restart state");
    assert!(
        reconcile_pending_cleanup_journal(&db).expect("reconcile Safe Trash source cleanup") >= 1
    );
    assert_eq!(
        cleanup_item(&db, &safe_trash_source_cleanup_item_id).status,
        "manual_review"
    );

    let restore_source_claimed = native_cleanup_candidate(
        &safe_trash_cases_root,
        "restore-source-claimed",
        b"restore claim",
    );
    let restore_source_claimed_move = move_cleanup_candidates_to_safe_trash_for_candidates(
        vec![restore_source_claimed.id.clone()],
        std::slice::from_ref(&restore_source_claimed),
        &db,
        Some(&app_data),
    )
    .expect("move restore source-claimed case");
    let restore_source_claimed_item_id = restore_source_claimed_move.logs[0]
        .item_id
        .clone()
        .expect("restore source-claimed item id");
    let restore_source_claimed_item = cleanup_item(&db, &restore_source_claimed_item_id);
    let restore_claim_serial = source_claim::lock_claim_test_hooks();
    source_claim::set_claim_test_hook(Some(panic_after_restore_claim));
    let restore_claim_panic = catch_unwind(AssertUnwindSafe(|| {
        let _ =
            restore_cleanup_trash_items_for_db(vec![restore_source_claimed_item.id.clone()], &db);
    }));
    source_claim::set_claim_test_hook(None);
    drop(restore_claim_serial);
    assert!(restore_claim_panic.is_err());
    let restore_source_claimed_pending = cleanup_item(&db, &restore_source_claimed_item_id);
    assert_eq!(restore_source_claimed_pending.status, "pending");
    assert_eq!(
        restore_source_claimed_pending.operation_phase,
        "source_claimed"
    );
    assert_eq!(
        restore_source_claimed_pending.identity_status,
        "restore_pending"
    );
    assert!(!Path::new(&restore_source_claimed_pending.original_path).exists());
    assert!(!Path::new(&restore_source_claimed_pending.trash_path).exists());
    assert!(restore_source_claimed_pending
        .source_claim_path
        .as_deref()
        .is_some_and(|path| Path::new(path).exists()));
    assert!(reconcile_pending_cleanup_journal(&db).expect("reconcile restore source claim") >= 1);
    let restore_source_claimed_recovered = cleanup_item(&db, &restore_source_claimed_item_id);
    assert_eq!(restore_source_claimed_recovered.status, "manual_review");
    assert_eq!(
        restore_source_claimed_recovered.operation_phase,
        "source_claimed"
    );
    assert!(!preview_cleanup_restore_item_for_test(restore_source_claimed_recovered).can_restore);
    let restore_reentry =
        restore_cleanup_trash_items_for_db(vec![restore_source_claimed_item_id.clone()], &db)
            .expect("restore pending Safe Trash reentry result");
    assert_eq!(restore_reentry.restored, 0);
    assert_eq!(restore_reentry.failed, 1);
    assert!(restore_reentry.logs[0]
        .message
        .contains("restore_pending_reconciliation"));
    let restore_reentry_item = cleanup_item(&db, &restore_source_claimed_item_id);
    assert_eq!(restore_reentry_item.status, "manual_review");
    assert_eq!(restore_reentry_item.operation_phase, "source_claimed");
    assert!(restore_reentry_item
        .source_claim_path
        .as_deref()
        .is_some_and(|path| Path::new(path).exists()));

    let mut move_manual_recovery = safe_trash_durability_item.clone();
    move_manual_recovery.status = "manual_review".to_string();
    move_manual_recovery.identity_status = "pending_recovery".to_string();
    move_manual_recovery.operation_phase = "manual_review".to_string();
    db.update_cleanup_trash_item_status(&move_manual_recovery)
        .expect("persist Safe Trash manual recovery state");
    assert!(db
        .pending_cleanup_trash_items()
        .expect("query Safe Trash manual recovery state")
        .iter()
        .any(|item| item.id == move_manual_recovery.id));
    assert!(
        reconcile_pending_cleanup_journal(&db).expect("reconcile Safe Trash manual recovery") >= 1
    );
    let move_manual_recovered = cleanup_item(&db, &move_manual_recovery.id);
    assert_eq!(move_manual_recovered.status, "manual_review");
    assert_eq!(move_manual_recovered.identity_status, "pending_recovery");
    assert_eq!(move_manual_recovered.operation_phase, "manual_review");

    let restore_target_committed = native_cleanup_candidate(
        &safe_trash_cases_root,
        "restore-target-committed",
        b"restore target",
    );
    let restore_target_committed_move = move_cleanup_candidates_to_safe_trash_for_candidates(
        vec![restore_target_committed.id.clone()],
        std::slice::from_ref(&restore_target_committed),
        &db,
        Some(&app_data),
    )
    .expect("move restore target-committed case");
    let restore_target_committed_item_id = restore_target_committed_move.logs[0]
        .item_id
        .clone()
        .expect("restore target-committed item id");
    test_faults::set_fault(Some(AtomicFaultPoint::TargetDurability));
    let restore_target_committed_result =
        restore_cleanup_trash_items_for_db(vec![restore_target_committed_item_id.clone()], &db)
            .expect("restore target-committed fault result");
    test_faults::set_fault(None);
    assert_eq!(restore_target_committed_result.restored, 0);
    assert_eq!(restore_target_committed_result.failed, 1);
    assert!(restore_target_committed_result.logs[0]
        .message
        .contains("target_committed_durability_unknown"));
    let restore_target_committed_item = cleanup_item(&db, &restore_target_committed_item_id);
    assert_eq!(restore_target_committed_item.status, "manual_review");
    assert_eq!(
        restore_target_committed_item.operation_phase,
        "target_committed"
    );
    assert!(Path::new(&restore_target_committed_item.original_path).exists());
    assert!(!Path::new(&restore_target_committed_item.trash_path).exists());
    let mut restore_target_committed_pending = restore_target_committed_item.clone();
    restore_target_committed_pending.status = "pending".to_string();
    db.update_cleanup_trash_item_status(&restore_target_committed_pending)
        .expect("persist restore target committed pending restart state");
    assert!(
        reconcile_pending_cleanup_journal(&db).expect("reconcile restore target committed") >= 1
    );
    assert_eq!(
        cleanup_item(&db, &restore_target_committed_item_id).status,
        "restored"
    );
    let restore_target_committed_recovered = cleanup_item(&db, &restore_target_committed_item_id);
    assert_eq!(
        restore_target_committed_recovered.operation_phase,
        "completed"
    );
    assert_eq!(
        restore_target_committed_recovered.identity_status,
        "verified"
    );
    assert!(restore_target_committed_recovered
        .source_claim_path
        .is_none());

    let restore_source_cleanup = native_cleanup_candidate(
        &safe_trash_cases_root,
        "restore-source-cleanup",
        b"restore cleanup",
    );
    let restore_source_cleanup_move = move_cleanup_candidates_to_safe_trash_for_candidates(
        vec![restore_source_cleanup.id.clone()],
        std::slice::from_ref(&restore_source_cleanup),
        &db,
        Some(&app_data),
    )
    .expect("move restore source-cleanup case");
    let restore_source_cleanup_item_id = restore_source_cleanup_move.logs[0]
        .item_id
        .clone()
        .expect("restore source-cleanup item id");
    test_faults::set_fault(Some(AtomicFaultPoint::SourceCleanup));
    let restore_source_cleanup_result =
        restore_cleanup_trash_items_for_db(vec![restore_source_cleanup_item_id.clone()], &db)
            .expect("restore source-cleanup fault result");
    test_faults::set_fault(None);
    assert_eq!(restore_source_cleanup_result.restored, 0);
    assert_eq!(restore_source_cleanup_result.failed, 1);
    assert!(restore_source_cleanup_result.logs[0]
        .message
        .contains("target_committed_source_cleanup_pending"));
    let restore_source_cleanup_item = cleanup_item(&db, &restore_source_cleanup_item_id);
    assert_eq!(restore_source_cleanup_item.status, "manual_review");
    assert_eq!(
        restore_source_cleanup_item.operation_phase,
        "source_cleanup_pending"
    );
    assert!(Path::new(&restore_source_cleanup_item.original_path).exists());
    assert!(!Path::new(&restore_source_cleanup_item.trash_path).exists());
    let mut restore_source_cleanup_pending = restore_source_cleanup_item.clone();
    restore_source_cleanup_pending.status = "pending".to_string();
    db.update_cleanup_trash_item_status(&restore_source_cleanup_pending)
        .expect("persist restore source cleanup pending restart state");
    assert!(reconcile_pending_cleanup_journal(&db).expect("reconcile restore source cleanup") >= 1);
    assert_eq!(
        cleanup_item(&db, &restore_source_cleanup_item_id).status,
        "restored"
    );
    let restore_source_cleanup_recovered = cleanup_item(&db, &restore_source_cleanup_item_id);
    assert_eq!(
        restore_source_cleanup_recovered.operation_phase,
        "completed"
    );
    assert_eq!(restore_source_cleanup_recovered.identity_status, "verified");
    assert!(restore_source_cleanup_recovered.source_claim_path.is_none());

    let final_log_source = source_volume.join("final-log-source.txt");
    let final_log_target = source_volume.join("final-log-target.txt");
    let final_log_operation_id = format!("native-final-log-{run_id}");
    fs::write(&final_log_source, b"final log persistence payload").expect("write final-log source");
    set_operation_test_fault(Some(
        OperationTestFaultPoint::AfterCompletedPhaseBeforeFinalLogPersist,
    ));
    let final_log_result = execute_moves_with_persistence(
        &db,
        ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: final_log_operation_id.clone(),
                file_id: format!("native-final-log-file-{run_id}"),
                operation_type: "move".to_string(),
                source_path: final_log_source.to_string_lossy().into_owned(),
                target_path: final_log_target.to_string_lossy().into_owned(),
                old_name: "final-log-source.txt".to_string(),
                new_name: "final-log-target.txt".to_string(),
                is_executable: Some(true),
            }],
        },
    );
    set_operation_test_fault(None);
    assert!(final_log_result.is_err());
    assert!(!final_log_source.exists());
    assert_eq!(
        fs::read(&final_log_target).expect("final-log target"),
        b"final log persistence payload"
    );
    let final_log_pending = db
        .get_pending_operation_logs()
        .expect("read final-log pending journal")
        .into_iter()
        .find(|log| log.path_before == final_log_source.to_string_lossy())
        .expect("final-log pending record");
    assert_eq!(final_log_pending.operation_phase, "completed");
    assert_eq!(
        reconcile_pending_operation_journal(&db).expect("reconcile final-log boundary"),
        1
    );
    let final_log_recovered = db
        .get_operation_logs(Some(200))
        .expect("read final-log recovered record")
        .into_iter()
        .find(|log| log.path_before == final_log_source.to_string_lossy())
        .expect("final-log recovered record");
    assert_eq!(final_log_recovered.status, "success");
    assert_eq!(final_log_recovered.operation_phase, "completed");

    let sqlite_integrity = rusqlite::Connection::open(&database_path)
        .expect("open SQLite integrity connection")
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .expect("SQLite integrity_check");
    assert_eq!(sqlite_integrity, "ok");
    let manifest_path = env::temp_dir()
        .join("zen-canvas-native-qa-artifacts")
        .join("native-file-hardening-smoke.json");
    fs::create_dir_all(manifest_path.parent().expect("manifest parent"))
        .expect("create manifest directory");
    let fixture_text = fixture_root.to_string_lossy().into_owned();
    let database_text = database_path.to_string_lossy().into_owned();
    let source_text = source_volume.to_string_lossy().into_owned();
    let target_text = target_volume.to_string_lossy().into_owned();
    let canary_sha256_after = sha256_file(&canary);
    let canary_unchanged = canary_sha256_before == canary_sha256_after;
    assert!(canary_unchanged);
    drop(db);
    remove_isolated_fixture(&fixture_root);
    if !target_volume.starts_with(&fixture_root) {
        let secondary_root = target_volume.parent().expect("secondary fixture root");
        remove_isolated_fixture(secondary_root);
    }
    let manifest = SmokeManifest {
        schema: "zen-canvas-native-file-hardening-smoke/v5",
        fixture_root: fixture_text,
        database: database_text,
        source_volume: source_text,
        target_volume: target_text,
        cross_volume_exercised,
        ordinary_same_volume_file_copy,
        ordinary_same_volume_directory_copy,
        ordinary_cross_volume_file_copy,
        ordinary_cross_volume_directory_copy,
        ordinary_same_volume_file_move,
        ordinary_same_volume_directory_move,
        ordinary_cross_volume_file_move,
        windows_copy_audit_policy: "destination_parent_inherited_source_sacl_not_copied",
        windows_sacl_inspection,
        operation_move_restore,
        rename: "passed",
        target_conflict: "passed",
        source_replacement: "passed",
        target_parent_replacement: "passed",
        staging_handle_copy_commit: if cross_volume_directory_move
            == "passed_move_and_history_restore"
        {
            "passed"
        } else {
            "unverified_sacl_or_volume_capability"
        },
        cross_volume_directory_move,
        durability_failure: "passed",
        safe_trash_durability_failure: "passed",
        safe_trash_identity_mismatch: "passed",
        safe_trash_source_cleanup_pending: "passed",
        safe_trash_restore_source_claimed: "passed",
        safe_trash_restore_target_committed: "passed",
        ordinary_restore_source_claimed: "passed",
        ordinary_restore_target_committed: "passed",
        ordinary_restore_final_transaction: "passed",
        ordinary_restore_target_replacement: "passed",
        ordinary_restore_claim_replacement: "passed",
        ordinary_restore_target_and_claim: "passed",
        ordinary_restore_all_paths_missing: "passed",
        ordinary_restore_claim_path,
        final_log_persistence_boundary: "passed",
        claim_identity_reconciliation: "passed",
        windows_nested_handle_relative: "passed",
        system_trash_preview: "passed",
        operation_reconciliation: "passed",
        safe_trash_restore: "passed",
        safe_trash_reconciliation: "passed",
        sqlite_integrity: "ok",
        canary_sha256_before,
        canary_sha256_after,
        canary_unchanged,
        real_app_data_accessed: false,
        fixture_cleaned: !fixture_root.exists() && !target_volume.exists(),
    };
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");
    println!(
        "{}",
        serde_json::to_string(&manifest).expect("manifest line")
    );
    println!("manifest={}", manifest_path.display());
    drop(fixture_guard);
}

fn sha256_file(path: &Path) -> String {
    let bytes = fs::read(path).expect("read SHA-256 input");
    format!("{:x}", Sha256::digest(bytes))
}

fn secondary_target_root(fixture_root: &Path, run_id: &str) -> PathBuf {
    let source_prefix = volume_prefix(fixture_root);
    if let Ok(configured) = env::var("ZEN_CANVAS_NATIVE_QA_SECONDARY_TEMP") {
        let configured = PathBuf::from(configured);
        if configured.is_absolute() && volume_prefix(&configured) != source_prefix {
            return configured
                .join(format!("zen-canvas-final-hardening-v413-{run_id}"))
                .join("target-volume");
        }
    }
    for drive in b'C'..=b'Z' {
        let candidate_root = PathBuf::from(format!(
            "{}:\\zen-canvas-final-hardening-v413-{run_id}",
            drive as char
        ));
        if volume_prefix(&candidate_root) == source_prefix {
            continue;
        }
        let candidate = candidate_root.join("target-volume");
        if fs::create_dir_all(&candidate).is_ok() {
            return candidate;
        }
    }
    fixture_root.join("target-volume")
}

fn volume_prefix(path: &Path) -> String {
    path.components()
        .next()
        .map(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn remove_isolated_fixture(path: &Path) {
    let text = path.to_string_lossy().to_ascii_lowercase();
    assert!(text.contains("zen-canvas-final-hardening-v413-"));
    assert!(path.parent().is_some());
    if path.exists() {
        fs::remove_dir_all(path).expect("remove isolated native QA fixture");
    }
}

fn remove_isolated_fixture_if_present(path: &Path) {
    let text = path.to_string_lossy().to_ascii_lowercase();
    if text.contains("zen-canvas-final-hardening-v413-") && path.parent().is_some() && path.exists()
    {
        let _ = fs::remove_dir_all(path);
    }
}
