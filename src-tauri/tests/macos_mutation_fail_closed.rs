#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use std::{
    fs,
    io::Write,
    sync::{mpsc, Arc, Barrier},
    thread,
};

use fylura_tauri::{
    db::Database,
    file_ops::{
        execute_moves_with_persistence, restore_moves_with_persistence, ExecuteMovesRequest,
        OperationPreviewRequest, RestoreMovesRequest,
    },
    fs_safety::{
        atomic_move_noreplace, atomic_move_noreplace_for_test_operation, atomic_replace_for_test,
        AtomicMoveError, AtomicMoveTestOperation,
    },
};

fn fixture(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "zen-canvas-macos-native-{name}-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&root).expect("fixture");
    root
}

fn find_namespace_entry(root: &std::path::Path, prefix: &str) -> Option<std::path::PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(prefix))
        {
            return Some(path);
        }
        if fs::symlink_metadata(&path)
            .ok()
            .is_some_and(|metadata| metadata.is_dir())
        {
            if let Some(found) = find_namespace_entry(&path, prefix) {
                return Some(found);
            }
        }
    }
    None
}

fn non_temp_fixture(name: &str) -> std::path::PathBuf {
    let root = std::env::current_dir()
        .expect("current directory")
        .join(format!(
            ".zen-canvas-macos-native-{name}-{}",
            uuid::Uuid::new_v4()
        ));
    fs::create_dir_all(&root).expect("non-temp fixture");
    root
}

fn replace_source_after_claim(
    point: fylura_tauri::fs_safety::source_claim::ClaimTestPoint,
    source: &std::path::Path,
    _claim: &std::path::Path,
) {
    if point == fylura_tauri::fs_safety::source_claim::ClaimTestPoint::AfterClaimBeforeIdentityCheck
    {
        fs::write(source, b"attacker source replacement").expect("rebind source");
    }
}

fn replace_source_before_claim(
    point: fylura_tauri::fs_safety::source_claim::ClaimTestPoint,
    source: &std::path::Path,
    _claim: &std::path::Path,
) {
    if point
        == fylura_tauri::fs_safety::source_claim::ClaimTestPoint::AfterJournalPreparedBeforeClaim
    {
        fs::write(source, b"attacker source before claim").expect("replace source before claim");
    }
}

fn recreate_source_before_commit(
    point: fylura_tauri::fs_safety::source_claim::ClaimTestPoint,
    source: &std::path::Path,
    _claim: &std::path::Path,
) {
    if point
        == fylura_tauri::fs_safety::source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeTargetCommit
    {
        let _ = fs::remove_file(source);
        fs::write(source, b"attacker recreated source").expect("recreate source");
    }
}

fn create_or_replace_target_before_commit(
    point: fylura_tauri::fs_safety::source_claim::ClaimTestPoint,
    source: &std::path::Path,
    _claim: &std::path::Path,
) {
    if point
        == fylura_tauri::fs_safety::source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeTargetCommit
    {
        let target = source.parent().expect("source parent").join("target.txt");
        let _ = fs::remove_file(&target);
        fs::write(target, b"attacker target replacement").expect("replace target");
    }
}

fn recreate_cross_volume_source_after_copy(
    point: fylura_tauri::fs_safety::source_claim::ClaimTestPoint,
    source: &std::path::Path,
    _claim: &std::path::Path,
) {
    if point
        == fylura_tauri::fs_safety::source_claim::ClaimTestPoint::AfterStagingVerifiedBeforeCommit
    {
        let _ = fs::remove_file(source);
        fs::write(source, b"attacker cross-volume source").expect("recreate cross-volume source");
    }
}

fn rebind_claim_namespace(
    point: fylura_tauri::fs_safety::source_claim::ClaimTestPoint,
    _source: &std::path::Path,
    claim: &std::path::Path,
) {
    if point
        == fylura_tauri::fs_safety::source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeTargetCommit
    {
        let rebound = claim.with_extension("rebound");
        let _ = fs::rename(claim, &rebound);
        fs::write(claim, b"attacker claim replacement").expect("recreate claim");
    }
}

fn occupy_target_then_source_on_rollback(
    point: fylura_tauri::fs_safety::source_claim::ClaimTestPoint,
    source: &std::path::Path,
    _claim: &std::path::Path,
) {
    if point
        == fylura_tauri::fs_safety::source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeCommit
    {
        fs::write(
            source.parent().expect("source parent").join("target.txt"),
            b"attacker target",
        )
        .expect("occupy target");
    }
    if point
        == fylura_tauri::fs_safety::source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeRollback
    {
        fs::write(source, b"attacker rollback source").expect("occupy rollback source");
    }
}

fn rebind_delete_claim(
    point: fylura_tauri::fs_safety::source_claim::ClaimTestPoint,
    _source: &std::path::Path,
    claim: &std::path::Path,
) {
    if point
        != fylura_tauri::fs_safety::source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeDelete
    {
        return;
    }
    let saved = claim.with_file_name(".zen-canvas-attacker-delete-save");
    fs::rename(claim, &saved).expect("save delete claim");
    fs::write(claim, b"attacker delete replacement").expect("replace delete claim");
}

fn rebind_replacement_backup(
    point: fylura_tauri::fs_safety::source_claim::ClaimTestPoint,
    _source: &std::path::Path,
    claim: &std::path::Path,
) {
    if point != fylura_tauri::fs_safety::source_claim::ClaimTestPoint::AfterReplacementBackupClaimed
    {
        return;
    }
    let saved = claim.with_file_name(".zen-canvas-attacker-replacement-save");
    fs::rename(claim, &saved).expect("save replacement backup");
    fs::write(claim, b"attacker replacement backup").expect("replace replacement backup");
}

fn recreate_package_namespace(
    point: fylura_tauri::fs_safety::source_claim::ClaimTestPoint,
    source: &std::path::Path,
    _claim: &std::path::Path,
) {
    if point
        != fylura_tauri::fs_safety::source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeTargetCommit
    {
        return;
    }
    fs::create_dir_all(source.join("Contents")).expect("recreate package namespace");
    fs::write(source.join("Contents/recreated"), b"attacker package").expect("recreate package");
}

fn recreate_directory_namespace(
    point: fylura_tauri::fs_safety::source_claim::ClaimTestPoint,
    source: &std::path::Path,
    _claim: &std::path::Path,
) {
    if point
        != fylura_tauri::fs_safety::source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeTargetCommit
    {
        return;
    }
    fs::create_dir(source).expect("recreate directory namespace");
    fs::write(source.join("recreated"), b"attacker directory").expect("recreate directory");
}

fn recreate_symlink_namespace(
    point: fylura_tauri::fs_safety::source_claim::ClaimTestPoint,
    source: &std::path::Path,
    _claim: &std::path::Path,
) {
    if point
        != fylura_tauri::fs_safety::source_claim::ClaimTestPoint::AfterClaimVerifiedBeforeTargetCommit
    {
        return;
    }
    std::os::unix::fs::symlink("target.txt", source).expect("recreate symlink namespace");
}

#[derive(Default)]
struct ExpandedRaceMetrics {
    iterations: u64,
    safe_success: u64,
    safe_failure: u64,
    rollback: u64,
    manual_recovery: u64,
    unexpected_overwrite: u64,
    wrong_commit: u64,
    wrong_delete: u64,
    unrecoverable_loss: u64,
}

fn record_expanded_result(
    metrics: &mut ExpandedRaceMetrics,
    result: &Result<fylura_tauri::fs_safety::AtomicMoveOutcome, AtomicMoveError>,
) {
    match result {
        Ok(_) => metrics.safe_success += 1,
        Err(error) => {
            metrics.safe_failure += 1;
            match error.commit_state() {
                fylura_tauri::fs_safety::AtomicMoveCommitState::RolledBack => metrics.rollback += 1,
                fylura_tauri::fs_safety::AtomicMoveCommitState::SourceCleanupPending
                | fylura_tauri::fs_safety::AtomicMoveCommitState::ManualReview => {
                    metrics.manual_recovery += 1
                }
                _ => {}
            }
        }
    }
}

fn expanded_recovery_entry_exists(case_root: &std::path::Path) -> bool {
    fs::read_dir(case_root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().contains("zen-canvas"))
}

fn namespace_entry_exists(path: &std::path::Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn record_expanded_no_loss(
    metrics: &mut ExpandedRaceMetrics,
    case_root: &std::path::Path,
    source: &std::path::Path,
    target: &std::path::Path,
) {
    if !namespace_entry_exists(source)
        && !namespace_entry_exists(target)
        && !expanded_recovery_entry_exists(case_root)
    {
        metrics.unrecoverable_loss += 1;
    }
}

fn execute(
    db: &Database,
    id: &str,
    operation_type: &str,
    source: &std::path::Path,
    target: &std::path::Path,
) -> fylura_tauri::file_ops::OperationLogDto {
    execute_moves_with_persistence(
        db,
        ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: id.to_string(),
                file_id: format!("file-{id}"),
                operation_type: operation_type.to_string(),
                source_path: source.to_string_lossy().into_owned(),
                target_path: target.to_string_lossy().into_owned(),
                old_name: source
                    .file_name()
                    .expect("source name")
                    .to_string_lossy()
                    .into_owned(),
                new_name: target
                    .file_name()
                    .expect("target name")
                    .to_string_lossy()
                    .into_owned(),
                is_executable: Some(true),
            }],
        },
    )
    .expect("persist operation")
    .logs
    .into_iter()
    .next()
    .expect("operation log")
}

#[test]
fn macos_mutation_parity_supports_move_copy_replace_restore_and_delete() {
    let root = fixture("parity");
    let db = Database::open(root.join("qa.sqlite3")).expect("database");

    let source = root.join("source.txt");
    let moved = root.join("moved.txt");
    fs::write(&source, b"move payload").expect("source");
    let move_log = execute(&db, "move", "move", &source, &moved);
    if move_log.status != "success" {
        panic!("move log: {move_log:?}");
    }
    assert!(!source.exists());
    assert_eq!(fs::read(&moved).expect("moved bytes"), b"move payload");
    let restored = restore_moves_with_persistence(
        &db,
        RestoreMovesRequest {
            logs: vec![move_log],
        },
    )
    .expect("restore move");
    assert_eq!(restored.restored, 1);
    assert_eq!(fs::read(&source).expect("restored bytes"), b"move payload");
    assert!(!moved.exists());

    let copy_target = root.join("copy.txt");
    let copy_log = execute(&db, "copy", "copy", &source, &copy_target);
    if copy_log.status != "success" {
        panic!("copy log: {copy_log:?}");
    }
    assert!(source.exists());
    assert_eq!(fs::read(&copy_target).expect("copy bytes"), b"move payload");

    let duplicate_target = root.join("duplicate.txt");
    let duplicate_log = execute(&db, "duplicate", "duplicate", &source, &duplicate_target);
    assert_eq!(duplicate_log.status, "success");
    assert_eq!(
        fs::read(&duplicate_target).expect("duplicate bytes"),
        b"move payload"
    );

    let replace_source = root.join("replace-source.txt");
    let replace_target = root.join("replace-target.txt");
    fs::write(
        &replace_source,
        b"new replacement payload with a different size",
    )
    .expect("replace source");
    fs::write(&replace_target, b"old target bytes").expect("replace target");
    let replace_log = execute(&db, "replace", "replace", &replace_source, &replace_target);
    assert_eq!(replace_log.status, "success");
    assert!(!replace_source.exists());
    assert_eq!(
        fs::read(&replace_target).expect("replacement bytes"),
        b"new replacement payload with a different size"
    );
    let replacement_backup =
        find_namespace_entry(&root, ".zen-canvas-replace-").expect("replacement backup path");
    assert_eq!(
        fs::read(&replacement_backup).expect("replacement backup"),
        b"old target bytes"
    );
    let restored_replace = restore_moves_with_persistence(
        &db,
        RestoreMovesRequest {
            logs: vec![replace_log],
        },
    )
    .expect("restore replacement");
    assert_eq!(
        restored_replace.restored, 1,
        "replacement restore result: {:?}",
        restored_replace.logs
    );
    assert_eq!(
        fs::read(&replace_source).expect("restored replacement source"),
        b"new replacement payload with a different size"
    );
    assert_eq!(
        fs::read(&replace_target).expect("restored replacement target"),
        b"old target bytes"
    );
    assert!(!replacement_backup.exists());

    let delete_root = non_temp_fixture("delete");
    let delete_source = delete_root.join("delete.txt");
    fs::write(&delete_source, b"delete payload").expect("delete source");
    let delete_log = execute(
        &db,
        "delete",
        "permanent_delete",
        &delete_source,
        &delete_source,
    );
    assert_eq!(
        delete_log.status, "success",
        "permanent delete result: {:?}",
        delete_log
    );
    assert!(!delete_source.exists());
    fs::remove_dir_all(delete_root).expect("remove delete fixture");

    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn macos_symlink_and_package_mutations_keep_namespace_boundaries() {
    let root = fixture("namespace");
    let db = Database::open(root.join("qa.sqlite3")).expect("database");

    let target = root.join("target.txt");
    fs::write(&target, b"symlink target").expect("target");
    let link = root.join("link.txt");
    std::os::unix::fs::symlink("target.txt", &link).expect("symlink");
    let moved_link = root.join("moved-link.txt");
    let link_log = execute(&db, "symlink", "move", &link, &moved_link);
    if link_log.status != "success" {
        panic!("symlink move log: {link_log:?}");
    }
    assert_eq!(
        fs::read_link(&moved_link).expect("moved symlink"),
        std::path::PathBuf::from("target.txt")
    );
    assert!(target.exists());

    let package = root.join("Example.app");
    fs::create_dir_all(package.join("Contents/Resources")).expect("package");
    fs::write(package.join("Contents/Resources/data.txt"), b"package data").expect("package data");
    let moved_package = root.join("Moved.app");
    let package_log = execute(&db, "package", "move", &package, &moved_package);
    if package_log.status != "success" {
        panic!("package move log: {package_log:?}");
    }
    assert!(!package.exists());
    assert_eq!(
        fs::read(moved_package.join("Contents/Resources/data.txt")).expect("moved package data"),
        b"package data"
    );

    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn macos_target_conflict_preserves_both_objects_without_claim_artifacts() {
    let root = fixture("conflict");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::write(&source, b"source").expect("source");
    fs::write(&target, b"competitor").expect("target");

    assert!(matches!(
        atomic_move_noreplace(&source, &target, None, None),
        Err(AtomicMoveError::TargetExists)
    ));
    assert_eq!(fs::read(&source).expect("source remains"), b"source");
    assert_eq!(fs::read(&target).expect("target remains"), b"competitor");
    assert!(find_namespace_entry(&root, ".zen-canvas-claim-").is_none());
    assert!(find_namespace_entry(&root, ".zen-canvas-stage-").is_none());

    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn macos_target_creation_race_never_loses_source_payload() {
    let root = fixture("race");
    let source_payload = b"race source payload";
    let competitor_payload = b"race competitor payload";
    let iterations = std::env::var("ZEN_CANVAS_MACOS_RACE_ITERATIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(10_000);
    assert!(iterations > 0, "race iteration count must be positive");

    let barrier = Arc::new(Barrier::new(2));
    let (attacker_tx, attacker_rx) = mpsc::sync_channel::<std::path::PathBuf>(0);
    let attacker_barrier = Arc::clone(&barrier);
    let attacker = thread::spawn(move || {
        while let Ok(attacker_target) = attacker_rx.recv() {
            attacker_barrier.wait();
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            if let Ok(mut file) = options.open(attacker_target) {
                file.write_all(competitor_payload)
                    .expect("competitor payload");
            }
            attacker_barrier.wait();
        }
    });

    let mut unexpected_overwrite = 0_u64;
    let mut wrong_commit = 0_u64;
    let mut wrong_delete = 0_u64;
    let mut unrecoverable_loss = 0_u64;
    let mut safe_success = 0_u64;
    let mut safe_failure = 0_u64;
    let mut rollback = 0_u64;
    let mut manual_recovery = 0_u64;

    for iteration in 0..iterations {
        let case_root = root.join(format!("case-{iteration:04}"));
        fs::create_dir_all(&case_root).expect("race case root");
        let source = case_root.join("source.txt");
        let target = case_root.join("target.txt");
        fs::write(&source, source_payload).expect("race source");

        attacker_tx
            .send(target.clone())
            .expect("send target to persistent attacker");
        barrier.wait();
        let result = atomic_move_noreplace(&source, &target, None, None);
        barrier.wait();

        match result.as_ref() {
            Ok(_) => safe_success += 1,
            Err(error) => {
                safe_failure += 1;
                match error.commit_state() {
                    fylura_tauri::fs_safety::AtomicMoveCommitState::RolledBack => rollback += 1,
                    fylura_tauri::fs_safety::AtomicMoveCommitState::SourceCleanupPending
                    | fylura_tauri::fs_safety::AtomicMoveCommitState::ManualReview => {
                        manual_recovery += 1
                    }
                    _ => {}
                }
            }
        }

        let source_exists = fs::symlink_metadata(&source).is_ok();
        let target_exists = fs::symlink_metadata(&target).is_ok();
        if !source_exists && !target_exists {
            unrecoverable_loss += 1;
        }
        if source_exists {
            assert_eq!(
                fs::read(&source).expect("read surviving source"),
                source_payload
            );
        }
        if target_exists {
            let target_bytes = fs::read(&target).expect("read surviving target");
            assert!(
                target_bytes == source_payload || target_bytes == competitor_payload,
                "unexpected target payload in race case {iteration}"
            );
            if result.is_ok() && target_bytes != source_payload {
                unexpected_overwrite += 1;
            }
        }
        if result.is_ok()
            && (source_exists
                || !target_exists
                || fs::read(&target).expect("read committed target") != source_payload)
        {
            wrong_commit += 1;
        }
        if !source_exists && target_exists {
            let target_bytes = fs::read(&target).expect("read target after race");
            if target_bytes == competitor_payload {
                wrong_delete += 1;
            }
        }
        assert!(find_namespace_entry(&case_root, ".zen-canvas-claim-").is_none());
        assert!(find_namespace_entry(&case_root, ".zen-canvas-stage-").is_none());
        fs::remove_dir_all(&case_root).expect("remove race case");
    }

    drop(attacker_tx);
    attacker.join().expect("join persistent target competitor");
    eprintln!(
        "macOS mutation race matrix iterations={iterations} safeSuccess={safe_success} safeFailure={safe_failure} rollback={rollback} manualRecovery={manual_recovery} unexpectedOverwrite={unexpected_overwrite} wrongCommit={wrong_commit} wrongDelete={wrong_delete} unrecoverableLoss={unrecoverable_loss}"
    );
    assert_eq!(unexpected_overwrite, 0, "attacker target was overwritten");
    assert_eq!(wrong_commit, 0, "move reported a wrong commit");
    assert_eq!(wrong_delete, 0, "attacker target was treated as source");
    assert_eq!(unrecoverable_loss, 0, "source and target both disappeared");
    fs::remove_dir_all(root).expect("remove race fixture");
}

#[test]
fn macos_expanded_adversarial_attack_matrix_reports_zero_wrong_commit_or_loss() {
    use fylura_tauri::fs_safety::source_claim::{
        lock_claim_test_hooks, set_claim_test_hook, ClaimTestPoint,
    };

    let root = fixture("expanded-race");
    let configured = std::env::var("ZEN_CANVAS_MACOS_EXPANDED_RACE_ITERATIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(10_000);
    assert!(
        configured > 0,
        "expanded race iteration count must be positive"
    );
    let mut metrics = ExpandedRaceMetrics::default();
    let serial = lock_claim_test_hooks();

    for iteration in 0..configured {
        metrics.iterations += 1;
        let case_root = root.join(format!("case-{iteration:05}"));
        fs::create_dir_all(&case_root).expect("expanded case root");
        let source = case_root.join("source.txt");
        let target = case_root.join("target.txt");
        fs::write(&source, b"matrix source payload").expect("expanded source");

        // Keep the expanded gate as one bounded 10k matrix, with every
        // mutation category exercised instead of treating the generic Move
        // primitive as a proxy for Safe Trash, Restore, Replace, or Delete.
        let scenario = iteration % 17;
        let hook = match scenario {
            1 => Some(
                replace_source_before_claim
                    as fn(ClaimTestPoint, &std::path::Path, &std::path::Path),
            ),
            2 => Some(
                replace_source_after_claim
                    as fn(ClaimTestPoint, &std::path::Path, &std::path::Path),
            ),
            3 => Some(
                rebind_claim_namespace as fn(ClaimTestPoint, &std::path::Path, &std::path::Path),
            ),
            4 => Some(
                occupy_target_then_source_on_rollback
                    as fn(ClaimTestPoint, &std::path::Path, &std::path::Path),
            ),
            5 => Some(
                recreate_source_before_commit
                    as fn(ClaimTestPoint, &std::path::Path, &std::path::Path),
            ),
            6 => Some(
                create_or_replace_target_before_commit
                    as fn(ClaimTestPoint, &std::path::Path, &std::path::Path),
            ),
            7 | 8 => Some(
                rebind_claim_namespace as fn(ClaimTestPoint, &std::path::Path, &std::path::Path),
            ),
            9 => Some(
                rebind_replacement_backup as fn(ClaimTestPoint, &std::path::Path, &std::path::Path),
            ),
            10 => {
                Some(rebind_delete_claim as fn(ClaimTestPoint, &std::path::Path, &std::path::Path))
            }
            14 => Some(
                recreate_package_namespace
                    as fn(ClaimTestPoint, &std::path::Path, &std::path::Path),
            ),
            15 => Some(
                recreate_directory_namespace
                    as fn(ClaimTestPoint, &std::path::Path, &std::path::Path),
            ),
            16 => Some(
                recreate_symlink_namespace
                    as fn(ClaimTestPoint, &std::path::Path, &std::path::Path),
            ),
            _ => None,
        };
        set_claim_test_hook(hook);
        if scenario == 0 {
            fs::write(&target, b"attacker target payload").expect("target collision");
        } else if scenario == 7 || scenario == 8 {
            let operation = if scenario == 7 {
                AtomicMoveTestOperation::SafeTrash
            } else {
                AtomicMoveTestOperation::Restore
            };
            let result = atomic_move_noreplace_for_test_operation(&source, &target, operation);
            record_expanded_result(&mut metrics, &result);
            if result.is_ok() {
                metrics.wrong_commit += 1;
            }
            record_expanded_no_loss(&mut metrics, &case_root, &source, &target);
            set_claim_test_hook(None);
            fs::remove_dir_all(&case_root).expect("remove coordinated rebind case");
            continue;
        } else if scenario == 9 {
            fs::write(&target, b"original replacement target").expect("replacement target");
            let result = atomic_replace_for_test(&source, &target);
            record_expanded_result(&mut metrics, &result);
            if result.is_ok()
                || target.exists()
                    && fs::read(&target).ok().as_deref() == Some(b"matrix source payload")
            {
                metrics.wrong_commit += 1;
            }
            record_expanded_no_loss(&mut metrics, &case_root, &source, &target);
            set_claim_test_hook(None);
            fs::remove_dir_all(&case_root).expect("remove replacement rebind case");
            continue;
        } else if scenario == 10 {
            let result = fylura_tauri::fs_safety::atomic_permanent_delete_for_test_with_hook(
                &source,
                rebind_delete_claim,
            );
            record_expanded_result(&mut metrics, &result);
            if result.is_ok() {
                metrics.wrong_delete += 1;
            }
            record_expanded_no_loss(&mut metrics, &case_root, &source, &target);
            set_claim_test_hook(None);
            fs::remove_dir_all(&case_root).expect("remove delete rebind case");
            continue;
        } else if scenario == 11 {
            fs::create_dir_all(case_root.join("package.app/Contents")).expect("package case");
            fs::write(case_root.join("package.app/Contents/data"), b"package")
                .expect("package data");
            let package_target = case_root.join("package-moved.app");
            let result =
                atomic_move_noreplace(&case_root.join("package.app"), &package_target, None, None);
            record_expanded_result(&mut metrics, &result);
            if result.is_ok() && !package_target.join("Contents/data").exists() {
                metrics.wrong_commit += 1;
            }
            record_expanded_no_loss(
                &mut metrics,
                &case_root,
                &case_root.join("package.app"),
                &package_target,
            );
            set_claim_test_hook(None);
            fs::remove_dir_all(&case_root).expect("remove package case");
            continue;
        } else if scenario == 12 {
            fs::create_dir_all(case_root.join("directory")).expect("directory case");
            fs::write(case_root.join("directory/data"), b"directory").expect("directory data");
            let directory_target = case_root.join("directory-moved");
            let result =
                atomic_move_noreplace(&case_root.join("directory"), &directory_target, None, None);
            record_expanded_result(&mut metrics, &result);
            if result.is_ok() && !directory_target.join("data").exists() {
                metrics.wrong_commit += 1;
            }
            record_expanded_no_loss(
                &mut metrics,
                &case_root,
                &case_root.join("directory"),
                &directory_target,
            );
            set_claim_test_hook(None);
            fs::remove_dir_all(&case_root).expect("remove directory case");
            continue;
        } else if scenario == 13 {
            let link = case_root.join("source-link");
            std::os::unix::fs::symlink("target.txt", &link).expect("symlink case");
            let link_target = case_root.join("link-moved");
            let result = atomic_move_noreplace(&link, &link_target, None, None);
            record_expanded_result(&mut metrics, &result);
            if result.is_ok()
                && fs::read_link(&link_target).ok() != Some(std::path::PathBuf::from("target.txt"))
            {
                metrics.wrong_commit += 1;
            }
            record_expanded_no_loss(&mut metrics, &case_root, &link, &link_target);
            set_claim_test_hook(None);
            fs::remove_dir_all(&case_root).expect("remove symlink case");
            continue;
        } else if scenario == 14 {
            let package = case_root.join("package-rebound.app");
            fs::create_dir_all(package.join("Contents")).expect("rebound package case");
            fs::write(package.join("Contents/data"), b"package source")
                .expect("rebound package data");
            let package_target = case_root.join("package-rebound-target.app");
            let result = atomic_move_noreplace(&package, &package_target, None, None);
            record_expanded_result(&mut metrics, &result);
            if result.is_ok() || package_target.exists() {
                metrics.wrong_commit += 1;
            }
            record_expanded_no_loss(&mut metrics, &case_root, &package, &package_target);
            set_claim_test_hook(None);
            fs::remove_dir_all(&case_root).expect("remove rebound package case");
            continue;
        } else if scenario == 15 {
            let directory = case_root.join("directory-rebound");
            fs::create_dir_all(&directory).expect("rebound directory case");
            fs::write(directory.join("data"), b"directory source").expect("rebound directory data");
            let directory_target = case_root.join("directory-rebound-target");
            let result = atomic_move_noreplace(&directory, &directory_target, None, None);
            record_expanded_result(&mut metrics, &result);
            if result.is_ok() || directory_target.exists() {
                metrics.wrong_commit += 1;
            }
            record_expanded_no_loss(&mut metrics, &case_root, &directory, &directory_target);
            set_claim_test_hook(None);
            fs::remove_dir_all(&case_root).expect("remove rebound directory case");
            continue;
        } else if scenario == 16 {
            let link = case_root.join("symlink-rebound");
            std::os::unix::fs::symlink("target.txt", &link).expect("rebound symlink case");
            let link_target = case_root.join("symlink-rebound-target");
            let result = atomic_move_noreplace(&link, &link_target, None, None);
            record_expanded_result(&mut metrics, &result);
            if result.is_ok() || link_target.exists() {
                metrics.wrong_commit += 1;
            }
            record_expanded_no_loss(&mut metrics, &case_root, &link, &link_target);
            set_claim_test_hook(None);
            fs::remove_dir_all(&case_root).expect("remove rebound symlink case");
            continue;
        }

        let result = atomic_move_noreplace(&source, &target, None, None);
        record_expanded_result(&mut metrics, &result);
        if result.is_ok() && fs::read(&target).ok().as_deref() != Some(b"matrix source payload") {
            metrics.wrong_commit += 1;
        }
        if fs::read(&target).ok().as_deref() == Some(b"attacker target payload") && result.is_ok() {
            metrics.unexpected_overwrite += 1;
        }
        let source_exists = namespace_entry_exists(&source);
        let target_exists = namespace_entry_exists(&target);
        record_expanded_no_loss(&mut metrics, &case_root, &source, &target);
        if source_exists {
            let source_bytes = fs::read(&source).expect("expanded source bytes");
            assert!(
                source_bytes == b"matrix source payload"
                    || source_bytes == b"attacker source before claim"
                    || source_bytes == b"attacker source replacement"
                    || source_bytes == b"attacker recreated source"
                    || source_bytes == b"attacker rollback source",
                "unexpected source payload in expanded case {iteration}: {source_bytes:?}"
            );
        }
        if target_exists
            && fs::read(&target).ok().is_some_and(|bytes| {
                bytes != b"matrix source payload"
                    && bytes != b"attacker target payload"
                    && bytes != b"attacker target"
                    && bytes != b"attacker target replacement"
            })
        {
            metrics.wrong_delete += 1;
        }
        set_claim_test_hook(None);
        fs::remove_dir_all(&case_root).expect("remove expanded case");
    }
    set_claim_test_hook(None);
    drop(serial);

    eprintln!(
        "macOS expanded attack matrix iterations={} safeSuccess={} safeFailure={} rollback={} manualRecovery={} unexpectedOverwrite={} wrongCommit={} wrongDelete={} unrecoverableLoss={}",
        metrics.iterations,
        metrics.safe_success,
        metrics.safe_failure,
        metrics.rollback,
        metrics.manual_recovery,
        metrics.unexpected_overwrite,
        metrics.wrong_commit,
        metrics.wrong_delete,
        metrics.unrecoverable_loss,
    );
    assert_eq!(metrics.unexpected_overwrite, 0);
    assert_eq!(metrics.wrong_commit, 0);
    assert_eq!(metrics.wrong_delete, 0);
    assert_eq!(metrics.unrecoverable_loss, 0);
    fs::remove_dir_all(root).expect("remove expanded race fixture");
}

#[test]
fn macos_cross_volume_source_mutation_is_rejected_when_real_fixture_is_provided() {
    use fylura_tauri::fs_safety::source_claim::{
        lock_claim_test_hooks, set_claim_test_hook, ClaimTestPoint,
    };
    use std::os::unix::fs::MetadataExt;

    let Some(fixture_root) =
        std::env::var_os("ZEN_CANVAS_EXTERNAL_APFS_FIXTURE").map(std::path::PathBuf::from)
    else {
        println!(
            "macOS cross-volume source mutation: SKIPPED — REAL FIXTURE NOT PROVIDED; NOT VERIFIED — fixture unavailable env=ZEN_CANVAS_EXTERNAL_APFS_FIXTURE"
        );
        return;
    };
    if !fixture_root.exists() {
        println!(
            "macOS cross-volume source mutation: SKIPPED — REAL FIXTURE NOT PROVIDED; NOT VERIFIED — fixture unavailable missing_path={}"
            ,
            fixture_root.display()
        );
        return;
    }
    let external_root = if fixture_root.is_dir() {
        fixture_root
    } else {
        fixture_root
            .parent()
            .expect("external fixture parent")
            .to_path_buf()
    };
    let fixture = external_root.join(format!(
        ".zen-canvas-cross-volume-race-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&fixture).expect("external race fixture");
    let target_root = std::env::temp_dir().join(format!(
        "zen-canvas-cross-volume-race-target-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&target_root).expect("target race fixture");
    let source = fixture.join("source.txt");
    let target = target_root.join("target.txt");
    fs::write(&source, b"cross-volume source").expect("cross-volume source");
    let source_device = fs::metadata(&fixture)
        .expect("source volume metadata")
        .dev();
    let target_device = fs::metadata(&target_root)
        .expect("target volume metadata")
        .dev();
    if source_device == target_device {
        println!(
            "macOS cross-volume source mutation: SKIPPED — REAL FIXTURE NOT PROVIDED; NOT VERIFIED — fixture unavailable different volume not established"
        );
        fs::remove_dir_all(&fixture).expect("remove same-volume fixture");
        fs::remove_dir_all(&target_root).expect("remove target fixture");
        return;
    }

    let _serial = lock_claim_test_hooks();
    set_claim_test_hook(Some(
        recreate_cross_volume_source_after_copy
            as fn(ClaimTestPoint, &std::path::Path, &std::path::Path),
    ));
    let result = atomic_move_noreplace(&source, &target, None, None);
    set_claim_test_hook(None);
    assert!(
        result.is_err(),
        "cross-volume mutation accepted a rebound source"
    );
    assert!(
        !target.exists(),
        "cross-volume target committed after source race"
    );
    assert_eq!(
        fs::read(&source).expect("cross-volume replacement source"),
        b"attacker cross-volume source"
    );
    eprintln!(
        "macOS cross-volume source mutation: IMPLEMENTED; NATIVE CONTRACT TESTED; REAL FIXTURE VERIFIED sourceDevice={source_device} targetDevice={target_device}"
    );
    fs::remove_dir_all(&fixture).expect("remove external race fixture");
    fs::remove_dir_all(&target_root).expect("remove target race fixture");
}
