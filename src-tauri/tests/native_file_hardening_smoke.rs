#![cfg(all(feature = "native-qa", windows))]

use fylura_tauri::{
    db::Database,
    file_ops::{
        execute_moves_with_persistence, native_qa_transfer_copy_preview,
        native_qa_transfer_move_preview, reconcile_pending_operation_journal,
        restore_moves_with_persistence, set_operation_test_fault, ExecuteMovesRequest,
        OperationLogDto, OperationPreviewRequest, OperationTestFaultPoint, RestoreMovesRequest,
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

struct TokenEvidence {
    identity: String,
    groups: String,
    privileges: String,
    is_administrator: bool,
    security_privilege_enabled: bool,
}

fn capture_token_evidence() -> TokenEvidence {
    fn whoami(argument: &str) -> String {
        let output = Command::new("whoami")
            .arg(argument)
            .output()
            .unwrap_or_else(|error| panic!("run whoami {argument}: {error}"));
        assert!(
            output.status.success(),
            "whoami {argument} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
    let identity = whoami("/user");
    let groups = whoami("/groups");
    let privileges = whoami("/priv");
    let is_administrator = groups.contains("S-1-5-32-544");
    let security_privilege_enabled = privileges.lines().any(|line| {
        line.contains("SeSecurityPrivilege") && line.to_ascii_lowercase().contains("enabled")
    });
    TokenEvidence {
        identity,
        groups,
        privileges,
        is_administrator,
        security_privilege_enabled,
    }
}

#[derive(Serialize)]
struct SmokeManifest {
    schema: &'static str,
    fixture_root: String,
    database: String,
    source_volume: String,
    source_volume_id: String,
    target_volume: String,
    target_volume_id: String,
    cross_volume_exercised: bool,
    token_evidence_path: String,
    token_identity: String,
    token_is_administrator: bool,
    token_security_privilege_enabled: bool,
    token_privileges: String,
    ordinary_same_volume_file_copy: &'static str,
    ordinary_same_volume_directory_copy: &'static str,
    ordinary_cross_volume_file_copy: &'static str,
    ordinary_cross_volume_directory_copy: &'static str,
    ordinary_same_volume_file_move: &'static str,
    ordinary_same_volume_directory_move: &'static str,
    ordinary_cross_volume_file_move: &'static str,
    windows_cross_volume_directory_acl: &'static str,
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
    let token = capture_token_evidence();
    let require_standard_user = env::var_os("ZC_NATIVE_QA_REQUIRE_STANDARD_USER").is_some();
    if require_standard_user {
        assert!(
            !token.is_administrator,
            "native filesystem smoke must run as a standard user; groups: {}",
            token.groups
        );
        assert!(
            !token.security_privilege_enabled,
            "ordinary Move must not require enabled SeSecurityPrivilege: {}",
            token.privileges
        );
    }
    let artifact_root = env::var_os("ZC_NATIVE_QA_ARTIFACT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join("zen-canvas-native-qa-artifacts"));
    let token_evidence_path = artifact_root.join("native-file-hardening-token.txt");
    fs::create_dir_all(&artifact_root).expect("create native QA evidence directory");
    fs::write(
        &token_evidence_path,
        format!(
            "USER\n{}\nGROUPS\n{}\nPRIVILEGES\n{}\n",
            token.identity, token.groups, token.privileges
        ),
    )
    .expect("write actual test token evidence");
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
    assert_eq!(
        moved.logs[0].status, "success",
        "ordinary same-volume Move must succeed: {:?}",
        moved.logs[0].error_message
    );
    assert_eq!(moved.logs[0].operation_phase, "completed");
    assert!(moved.logs[0].can_restore);
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
    let operation_move_restore = "passed";

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

    let source_volume_id = physical_volume_id(&source_volume);
    let target_volume_id = physical_volume_id(&target_volume);
    let cross_volume_exercised = source_volume_id != target_volume_id;
    if env::var_os("ZC_NATIVE_QA_REQUIRE_CROSS_VOLUME").is_some() {
        assert!(
            cross_volume_exercised,
            "required native QA paths must be on distinct volumes: source={} ({}) target={} ({})",
            source_volume.display(),
            source_volume_id,
            target_volume.display(),
            target_volume_id
        );
    }
    let same_copy_parent = source_volume.join("ordinary-copy-destination");
    fs::create_dir_all(&same_copy_parent).expect("same-volume copy target parent");
    let same_file_source = source_volume.join("ordinary-copy-file.txt");
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
                native_qa_transfer_copy_preview(&same_file_source, &same_copy_parent)
                    .expect("same-volume file Copy Preview"),
                native_qa_transfer_copy_preview(&same_directory_source, &same_copy_parent)
                    .expect("same-volume directory Copy Preview"),
            ],
        },
    )
    .expect("execute ordinary same-volume Copy fixtures");
    assert_eq!(same_copy_result.logs.len(), 2);
    assert!(
        same_copy_result
            .logs
            .iter()
            .all(|log| log.status == "success"),
        "same-volume Copy must execute through a fresh transfer Preview: {:?}",
        same_copy_result
            .logs
            .iter()
            .map(|log| (&log.status, &log.error_message))
            .collect::<Vec<_>>()
    );
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
    let mut windows_cross_volume_directory_acl = "unverified_no_secondary_volume";
    let mut cross_volume_directory_move = "unverified_no_secondary_volume";
    if cross_volume_exercised {
        let cross_copy_parent = target_volume.join("ordinary-copy-destination");
        fs::create_dir_all(&cross_copy_parent).expect("cross-volume copy target parent");
        let cross_file_source = source_volume.join("ordinary-cross-copy-file.txt");
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
                    native_qa_transfer_copy_preview(&cross_file_source, &cross_copy_parent)
                        .expect("cross-volume file Copy Preview"),
                    native_qa_transfer_copy_preview(&cross_directory_source, &cross_copy_parent)
                        .expect("cross-volume directory Copy Preview"),
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
        assert_eq!(
            cross_move_result.logs[0].status, "success",
            "ordinary standard-user cross-volume file Move must succeed: {:?}",
            cross_move_result.logs[0].error_message
        );
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

        let cancelled_source = source_volume.join("cross-volume-cancelled-directory");
        let cancelled_target = target_volume.join("cross-volume-cancelled-directory");
        fs::create_dir_all(cancelled_source.join("nested").join("empty"))
            .expect("create cross-volume cancellation source");
        fs::write(cancelled_source.join("a.bin"), vec![b'a'; 256 * 1024])
            .expect("write first cross-volume cancellation source file");
        let mut cancelled_permissions = fs::metadata(cancelled_source.join("a.bin"))
            .expect("read cancellation source permissions")
            .permissions();
        cancelled_permissions.set_readonly(true);
        fs::set_permissions(cancelled_source.join("a.bin"), cancelled_permissions)
            .expect("mark first cancellation source file read-only");
        fs::write(cancelled_source.join("nested").join("b.bin"), b"not copied")
            .expect("write second cross-volume cancellation source file");
        let cancelled_hash = sha256_file(&cancelled_source.join("a.bin"));
        let cancelled_preview = native_qa_transfer_move_preview(&cancelled_source, &target_volume)
            .expect("preview cross-volume directory Move cancellation fixture");
        test_faults::set_fault(Some(AtomicFaultPoint::CancelDirectoryCopyAfterEntry));
        let cancelled_result = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![cancelled_preview],
            },
        )
        .expect("execute injected cross-volume directory cancellation");
        test_faults::set_fault(None);
        assert_eq!(cancelled_result.logs.len(), 1);
        assert_eq!(cancelled_result.logs[0].status, "skipped");
        assert_eq!(cancelled_result.logs[0].operation_phase, "copying");
        assert!(cancelled_source.join("nested").join("empty").is_dir());
        assert_eq!(sha256_file(&cancelled_source.join("a.bin")), cancelled_hash);
        assert!(cancelled_source.join("nested").join("b.bin").is_file());
        assert!(!cancelled_target.exists());
        assert!(fs::read_dir(&target_volume)
            .expect("inspect cross-volume staging cleanup")
            .all(|entry| !entry
                .expect("read target directory entry")
                .file_name()
                .to_string_lossy()
                .starts_with(".zen-canvas-stage-")));

        let out_of_space_source = source_volume.join("cross-volume-space-failure-directory");
        let out_of_space_target = target_volume.join("cross-volume-space-failure-directory");
        fs::create_dir_all(out_of_space_source.join("nested"))
            .expect("create space-failure source");
        fs::write(
            out_of_space_source.join("payload.bin"),
            vec![b'x'; 512 * 1024],
        )
        .expect("write space-failure payload");
        let out_of_space_hash = sha256_file(&out_of_space_source.join("payload.bin"));
        let out_of_space_preview =
            native_qa_transfer_move_preview(&out_of_space_source, &target_volume)
                .expect("preview injected cross-volume space failure");
        test_faults::set_fault(Some(AtomicFaultPoint::OutOfSpaceDuringCopy));
        let out_of_space_result = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![out_of_space_preview],
            },
        )
        .expect("execute injected cross-volume space failure");
        test_faults::set_fault(None);
        assert_eq!(out_of_space_result.logs.len(), 1);
        assert_eq!(out_of_space_result.logs[0].status, "failed");
        assert!(out_of_space_result.logs[0]
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("injected_out_of_space_during_copy"));
        assert_eq!(
            sha256_file(&out_of_space_source.join("payload.bin")),
            out_of_space_hash,
            "a staging write failure must retain the source"
        );
        assert!(!out_of_space_target.exists());
        assert!(fs::read_dir(&target_volume)
            .expect("inspect out-of-space staging cleanup")
            .all(|entry| !entry
                .expect("read target directory entry")
                .file_name()
                .to_string_lossy()
                .starts_with(".zen-canvas-stage-")));

        let conflict_source = source_volume.join("cross-volume-conflict.txt");
        let conflict_parent = target_volume.join("conflict-destination");
        fs::create_dir_all(&conflict_parent).expect("create conflict destination");
        let conflict_target = conflict_parent.join("cross-volume-conflict.txt");
        fs::write(&conflict_source, b"source must survive target conflict")
            .expect("write conflicting Move source");
        fs::write(&conflict_target, b"pre-existing target must survive")
            .expect("write pre-existing Move target");
        let conflict_source_hash = sha256_file(&conflict_source);
        let conflict_target_hash = sha256_file(&conflict_target);
        let conflict_result = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![native_qa_transfer_move_preview(
                    &conflict_source,
                    &conflict_parent,
                )
                .expect("preview cross-volume no-overwrite Move")],
            },
        )
        .expect("execute cross-volume no-overwrite Move");
        assert_eq!(conflict_result.logs[0].status, "failed");
        let conflict_error = conflict_result.logs[0]
            .error_message
            .as_deref()
            .unwrap_or_default();
        assert!(
            conflict_error.contains("already exists") || conflict_error.contains("target_exists"),
            "a real target collision must be reported without replacement; got {conflict_error:?}"
        );
        assert_eq!(sha256_file(&conflict_source), conflict_source_hash);
        assert_eq!(sha256_file(&conflict_target), conflict_target_hash);

        let stream_source = source_volume.join("cross-volume-stream.txt");
        let stream_target = target_volume.join("cross-volume-stream.txt");
        fs::write(&stream_source, b"ordinary file stream base data")
            .expect("write alternate-stream fixture");
        let mut stream_path = stream_source.as_os_str().to_os_string();
        stream_path.push(":fylura-qa");
        let stream_path = PathBuf::from(stream_path);
        fs::write(&stream_path, b"alternate stream payload").expect("write alternate data stream");
        let stream_result = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![
                    native_qa_transfer_move_preview(&stream_source, &target_volume)
                        .expect("preview unsupported alternate-stream Move"),
                ],
            },
        )
        .expect("execute unsupported alternate-stream Move");
        assert_eq!(stream_result.logs[0].status, "failed");
        assert!(stream_result.logs[0]
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("windows_special_metadata_unsupported:alternate_data_stream"));
        assert!(stream_source.is_file());
        assert_eq!(
            fs::read(&stream_path).expect("retained alternate data stream"),
            b"alternate stream payload"
        );
        assert!(!stream_target.exists());

        let denied_parent = target_volume.join("permission-denied-destination");
        fs::create_dir_all(&denied_parent).expect("create permission-failure target parent");
        let denied_source = source_volume.join("cross-volume-permission-failure-directory");
        fs::create_dir_all(denied_source.join("nested")).expect("create denied Move source");
        fs::write(
            denied_source.join("nested").join("payload.bin"),
            b"permission failure",
        )
        .expect("write denied Move source payload");
        let denied_hash = sha256_file(&denied_source.join("nested").join("payload.bin"));
        // Preview must be captured while the destination is usable. Revoke
        // write access afterward so execution revalidation, rather than
        // Preview construction, observes the ordinary-user permission error.
        let denied_preview = native_qa_transfer_move_preview(&denied_source, &denied_parent)
            .expect("preview cross-volume permission failure");
        let denied = Command::new("icacls")
            .arg(&denied_parent)
            .args(["/deny", "*S-1-5-32-545:(WD,AD)"])
            .output()
            .expect("inject ordinary-user target write denial after Preview");
        assert!(
            denied.status.success(),
            "could not inject target permission failure: {}",
            String::from_utf8_lossy(&denied.stderr).trim()
        );
        let denied_result = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![denied_preview],
            },
        );
        let remove_deny = Command::new("icacls")
            .arg(&denied_parent)
            .args(["/remove:d", "*S-1-5-32-545"])
            .output()
            .expect("remove ordinary-user target write denial");
        assert!(
            remove_deny.status.success(),
            "could not restore isolated target ACL: {}",
            String::from_utf8_lossy(&remove_deny.stderr).trim()
        );
        let denied_result = denied_result.expect("execute cross-volume permission failure");
        assert_eq!(denied_result.logs[0].status, "failed");
        assert!(denied_source.join("nested").join("payload.bin").is_file());
        assert_eq!(
            sha256_file(&denied_source.join("nested").join("payload.bin")),
            denied_hash,
            "a target permission error must restore the source namespace"
        );
        assert!(!denied_parent
            .join("cross-volume-permission-failure-directory")
            .exists());
        assert_eq!(fs::read_dir(&denied_parent).unwrap().count(), 0);

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
        let readonly_file = directory_source.join("nested").join("readonly.txt");
        fs::write(&readonly_file, b"read-only metadata canary")
            .expect("write read-only metadata canary");
        let mut readonly_permissions = fs::metadata(&readonly_file)
            .expect("read source file permissions")
            .permissions();
        readonly_permissions.set_readonly(true);
        fs::set_permissions(&readonly_file, readonly_permissions)
            .expect("mark source file read-only");
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
        let source_only_acl_marker = "S-1-5-21-1111111111-2222222222-3333333333-4444";
        let explicit_acl = Command::new("icacls")
            .arg(&directory_source)
            .args(["/grant", &format!("*{source_only_acl_marker}:(OI)(CI)(RX)")])
            .output()
            .expect("add an explicit source-only ACL marker");
        assert!(
            explicit_acl.status.success(),
            "could not add the source-only ACL marker: {}",
            String::from_utf8_lossy(&explicit_acl.stderr).trim()
        );
        let source_acl = Command::new("icacls")
            .arg(&directory_source)
            .output()
            .expect("read explicit source ACL");
        assert!(source_acl.status.success());
        let source_acl = String::from_utf8_lossy(&source_acl.stdout).into_owned();
        assert!(source_acl.contains(source_only_acl_marker));
        assert!(
            !source_acl.contains("(I)"),
            "the source fixture must have an explicit, non-inherited ACL: {source_acl}"
        );
        let canary_hash = sha256_file(&directory_source.join("canary.txt"));
        let nested_hash = sha256_file(&directory_source.join("nested").join("内容.bin"));
        let readonly_hash = sha256_file(&readonly_file);
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
        assert_eq!(
            move_result.logs[0].status, "success",
            "ordinary standard-user cross-volume directory Move must succeed: {:?}",
            move_result.logs[0].error_message
        );
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
        assert_eq!(
            sha256_file(&directory_target.join("nested").join("readonly.txt")),
            readonly_hash
        );
        assert!(
            fs::metadata(directory_target.join("nested").join("readonly.txt"))
                .expect("read target read-only metadata")
                .permissions()
                .readonly()
        );
        assert!(directory_target.join("nested").join("empty").is_dir());
        let target_acl = Command::new("icacls")
            .arg(&directory_target)
            .output()
            .expect("read target ACL inherited from destination parent");
        assert!(target_acl.status.success());
        let target_acl = String::from_utf8_lossy(&target_acl.stdout).into_owned();
        assert!(
            target_acl.contains("(I)"),
            "new target directory must use destination-parent ACL inheritance: {target_acl}"
        );
        assert!(
            !target_acl.contains(source_only_acl_marker),
            "the source's explicit ACL marker must not be copied to the target: {target_acl}"
        );
        windows_cross_volume_directory_acl =
            "passed_destination_parent_inherited_source_acl_not_copied";

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
        assert_eq!(sha256_file(&readonly_file), readonly_hash);
        assert!(fs::metadata(&readonly_file)
            .expect("read restored read-only source metadata")
            .permissions()
            .readonly());
        assert!(!directory_target.exists());
        cross_volume_directory_move = "passed_move_and_history_restore";

        let pending_source = source_volume.join("cross-volume-cleanup-pending-directory");
        let pending_target = target_volume.join("cross-volume-cleanup-pending-directory");
        fs::create_dir_all(pending_source.join("nested")).expect("create cleanup-pending source");
        fs::write(
            pending_source.join("nested").join("payload.bin"),
            b"cleanup pending",
        )
        .expect("write cleanup-pending payload");
        let pending_hash = sha256_file(&pending_source.join("nested").join("payload.bin"));
        let pending_preview = native_qa_transfer_move_preview(&pending_source, &target_volume)
            .expect("preview cleanup-pending cross-volume Move");
        test_faults::set_fault(Some(AtomicFaultPoint::SourceCleanup));
        let pending_result = execute_moves_with_persistence(
            &db,
            ExecuteMovesRequest {
                operations: vec![pending_preview],
            },
        )
        .expect("execute injected post-publication cleanup failure");
        test_faults::set_fault(None);
        let pending_log = pending_result.logs.first().expect("cleanup-pending log");
        assert_eq!(pending_log.status, "manual_review");
        assert_eq!(pending_log.operation_phase, "source_cleanup_pending");
        assert!(pending_log
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("target_committed_source_cleanup_pending"));
        assert!(!pending_source.exists());
        assert!(pending_target.is_dir());
        assert_eq!(
            sha256_file(&pending_target.join("nested").join("payload.bin")),
            pending_hash
        );
        let pending_claim = PathBuf::from(
            pending_log
                .source_claim_path
                .as_deref()
                .expect("persisted source claim path"),
        );
        assert!(pending_claim.is_dir());
        assert_eq!(
            sha256_file(&pending_claim.join("nested").join("payload.bin")),
            pending_hash,
            "both committed target and claimed source remain intact"
        );
        let mut interrupted_log = pending_log.clone();
        interrupted_log.status = "pending".to_string();
        db.save_operation_logs(
            &interrupted_log.batch_id,
            std::slice::from_ref(&interrupted_log),
        )
        .expect("persist interrupted cleanup-pending state");
        assert_eq!(
            reconcile_pending_operation_journal(&db)
                .expect("reconcile cross-volume cleanup-pending Move"),
            1
        );
        let recovered_pending_log = db
            .get_operation_logs(Some(300))
            .expect("read recovered cleanup-pending operation")
            .into_iter()
            .find(|log| log.id == pending_log.id)
            .expect("recovered cleanup-pending operation");
        assert_eq!(recovered_pending_log.status, "manual_review");
        assert_eq!(
            recovered_pending_log.operation_phase,
            "source_cleanup_pending"
        );
        assert!(pending_claim.exists());
        assert!(pending_target.exists());
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
    let manifest_path = artifact_root.join("native-file-hardening-smoke.json");
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
        schema: "zen-canvas-native-file-hardening-smoke/v6",
        fixture_root: fixture_text,
        database: database_text,
        source_volume: source_text,
        source_volume_id,
        target_volume: target_text,
        target_volume_id,
        cross_volume_exercised,
        token_evidence_path: token_evidence_path.to_string_lossy().into_owned(),
        token_identity: token.identity,
        token_is_administrator: token.is_administrator,
        token_security_privilege_enabled: token.security_privilege_enabled,
        token_privileges: token.privileges,
        ordinary_same_volume_file_copy,
        ordinary_same_volume_directory_copy,
        ordinary_cross_volume_file_copy,
        ordinary_cross_volume_directory_copy,
        ordinary_same_volume_file_move,
        ordinary_same_volume_directory_move,
        ordinary_cross_volume_file_move,
        windows_cross_volume_directory_acl,
        operation_move_restore,
        rename: "passed",
        target_conflict: "passed",
        source_replacement: "passed",
        target_parent_replacement: "passed",
        staging_handle_copy_commit: "passed",
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
    let source_volume_id = physical_volume_id(
        fixture_root
            .parent()
            .expect("source temp root has a parent"),
    );
    if let Ok(configured) = env::var("ZEN_CANVAS_NATIVE_QA_SECONDARY_TEMP") {
        let configured = PathBuf::from(configured);
        if configured.is_absolute() && fs::create_dir_all(&configured).is_ok() {
            if physical_volume_id(&configured) != source_volume_id {
                return configured
                    .join(format!("zen-canvas-final-hardening-v413-{run_id}"))
                    .join("target-volume");
            }
        }
    }
    for drive in b'C'..=b'Z' {
        let candidate_root = PathBuf::from(format!("{}:\\fylura-native-qa", drive as char));
        if fs::create_dir_all(&candidate_root).is_ok()
            && physical_volume_id(&candidate_root) != source_volume_id
        {
            return candidate_root
                .join(format!("zen-canvas-final-hardening-v413-{run_id}"))
                .join("target-volume");
        }
    }
    fixture_root.join("target-volume")
}

fn physical_volume_id(path: &Path) -> String {
    VerifiedDirectory::open_existing(path)
        .unwrap_or_else(|error| panic!("open volume identity path {}: {error}", path.display()))
        .identity()
        .volume_id
        .clone()
}

fn remove_isolated_fixture(path: &Path) {
    let text = path.to_string_lossy().to_ascii_lowercase();
    assert!(text.contains("zen-canvas-final-hardening-v413-"));
    assert!(path.parent().is_some());
    if path.exists() {
        clear_fixture_readonly(path).expect("clear read-only fixture attributes");
        fs::remove_dir_all(path).expect("remove isolated native QA fixture");
    }
}

fn remove_isolated_fixture_if_present(path: &Path) {
    let text = path.to_string_lossy().to_ascii_lowercase();
    if text.contains("zen-canvas-final-hardening-v413-") && path.parent().is_some() && path.exists()
    {
        let _ = clear_fixture_readonly(path);
        let _ = fs::remove_dir_all(path);
    }
}

fn clear_fixture_readonly(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        for entry in fs::read_dir(path)? {
            clear_fixture_readonly(&entry?.path())?;
        }
    }
    if metadata.permissions().readonly() {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}
