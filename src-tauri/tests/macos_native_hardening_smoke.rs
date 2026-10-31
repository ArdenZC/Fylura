#![cfg(all(feature = "native-qa", target_os = "macos", target_arch = "aarch64"))]

use std::{
    fs,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{atomic::AtomicBool, Arc},
};

use fylura_tauri::{
    db::Database,
    file_ops::{
        execute_moves_core_with_progress, execute_moves_with_persistence,
        reconcile_pending_operation_journal, restore_moves_with_persistence,
        set_operation_test_fault, ExecuteMovesRequest, OperationPreviewRequest,
        OperationProgressEmitter, OperationProgressPayload, OperationTestFaultPoint,
        RestoreMovesRequest,
    },
    storage_analyzer::{
        move_cleanup_candidates_to_safe_trash_for_candidates,
        preview_cleanup_restore_item_for_test, reconcile_pending_cleanup_journal,
        restore_cleanup_trash_items_for_db, CleanupActionKind, CleanupTier, CleanupTrashBatch,
        CleanupTrashItem, StorageCandidate,
    },
};

struct NativeQaProgressSink;

impl OperationProgressEmitter for NativeQaProgressSink {
    fn emit_progress(&self, _payload: OperationProgressPayload) {}
}

#[test]
fn macos_restore_fault_reconciles_after_target_commit() {
    let root = std::env::temp_dir().join(format!(
        "zen-canvas-macos-native-qa-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&root).expect("fixture root");
    let db = Database::open(root.join("qa.sqlite3")).expect("database");
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    fs::write(&source, b"native restore fault payload").expect("source");

    let moved = execute_moves_with_persistence(
        &db,
        ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: "macos-native-qa-move".to_string(),
                file_id: "macos-native-qa-file".to_string(),
                operation_type: "move".to_string(),
                source_path: source.to_string_lossy().into_owned(),
                target_path: target.to_string_lossy().into_owned(),
                old_name: "source.txt".to_string(),
                new_name: "target.txt".to_string(),
                is_executable: Some(true),
            }],
        },
    )
    .expect("execute move");
    assert_eq!(moved.logs[0].status, "success");

    set_operation_test_fault(Some(
        OperationTestFaultPoint::AfterRestoreTargetCommittedBeforeFinalPersist,
    ));
    let panic_result = catch_unwind(AssertUnwindSafe(|| {
        let _ = restore_moves_with_persistence(
            &db,
            RestoreMovesRequest {
                logs: moved.logs.clone(),
            },
        );
    }));
    set_operation_test_fault(None);
    assert!(panic_result.is_err());
    assert!(source.exists());
    assert!(!target.exists());

    let pending = db
        .get_pending_restore_logs()
        .expect("pending restore logs")
        .into_iter()
        .find(|log| log.id == moved.logs[0].id)
        .expect("target-committed restore log");
    assert_eq!(pending.restore_phase, "target_committed");
    assert_eq!(
        reconcile_pending_operation_journal(&db).expect("reconcile target-committed restore"),
        1
    );
    let recovered = db
        .get_operation_logs(Some(20))
        .expect("recovered operation logs")
        .into_iter()
        .find(|log| log.id == moved.logs[0].id)
        .expect("recovered operation");
    assert_eq!(recovered.restore_status, "restored");
    assert_eq!(recovered.restore_phase, "completed");
    assert_eq!(
        fs::read(&source).expect("restored source"),
        b"native restore fault payload"
    );
    assert!(!target.exists());

    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn macos_safe_trash_binds_source_to_actual_target_and_restore_ledger() {
    use std::os::unix::fs::MetadataExt;

    let root = std::env::current_dir()
        .expect("current directory")
        .join(format!(
            ".zen-canvas-macos-safe-trash-{}",
            uuid::Uuid::new_v4()
        ));
    fs::create_dir_all(&root).expect("safe trash fixture root");
    let db = Database::open(root.join("qa.sqlite3")).expect("safe trash database");
    let source = root.join("source.txt");
    let payload = b"Safe Trash actual target binding payload";
    fs::write(&source, payload).expect("safe trash source");
    let source_identity = fs::metadata(&source).expect("source metadata");
    let candidate = StorageCandidate {
        id: "native-safe-trash-source".to_string(),
        path: source.to_string_lossy().into_owned(),
        name: "source.txt".to_string(),
        size: payload.len() as u64,
        tier: CleanupTier::Safe,
        category: "native contract".to_string(),
        reason: "native Safe Trash contract fixture".to_string(),
        suggested_action: CleanupActionKind::MoveToTrash,
        risk_note: None,
        trash_allowed: true,
        selected_by_default: true,
    };

    let moved = move_cleanup_candidates_to_safe_trash_for_candidates(
        vec![candidate.id.clone()],
        &[candidate],
        &db,
        None,
    )
    .expect("move to Safe Trash");
    assert_eq!(moved.moved, 1, "Safe Trash result: {moved:?}");
    let item_id = moved.logs[0].item_id.clone().expect("Safe Trash item id");
    let item = db
        .cleanup_trash_item(&item_id)
        .expect("read Safe Trash ledger")
        .expect("Safe Trash ledger item");
    let actual_target = std::path::PathBuf::from(&item.trash_path);
    assert_eq!(item.status, "moved");
    assert_eq!(item.operation_phase, "completed");
    assert!(!source.exists(), "Safe Trash left an original source entry");
    assert_eq!(
        fs::read(&actual_target).expect("read Safe Trash target"),
        payload
    );
    let target_identity = fs::metadata(&actual_target).expect("target metadata");
    assert_eq!(target_identity.dev(), source_identity.dev());
    assert_eq!(target_identity.ino(), source_identity.ino());
    let target_volume_id = target_identity.dev().to_string();
    let target_file_id = target_identity.ino().to_string();
    assert_eq!(
        item.trash_platform_volume_id.as_deref(),
        Some(target_volume_id.as_str())
    );
    assert_eq!(
        item.trash_platform_file_id.as_deref(),
        Some(target_file_id.as_str())
    );

    let restored = restore_cleanup_trash_items_for_db(vec![item_id.clone()], &db)
        .expect("restore Safe Trash item");
    assert_eq!(
        restored.restored, 1,
        "Safe Trash restore result: {restored:?}"
    );
    assert!(source.exists());
    assert!(!actual_target.exists());
    assert_eq!(fs::read(&source).expect("read restored source"), payload);
    let restored_identity = fs::metadata(&source).expect("restored metadata");
    assert_eq!(restored_identity.dev(), source_identity.dev());
    assert_eq!(restored_identity.ino(), source_identity.ino());
    let restored_item = db
        .cleanup_trash_item(&item_id)
        .expect("read restored ledger")
        .expect("restored ledger item");
    assert_eq!(restored_item.status, "restored");
    assert_eq!(restored_item.operation_phase, "completed");
    assert!(restored_item.source_claim_path.is_none());

    fs::remove_dir_all(root).expect("remove Safe Trash fixture");
}

#[test]
fn macos_legacy_untagged_cleanup_identity_cannot_be_promoted_by_recovery() {
    use std::os::unix::fs::MetadataExt;

    let root = std::env::current_dir()
        .expect("current directory")
        .join(format!(
            ".zen-canvas-macos-legacy-provenance-{}",
            uuid::Uuid::new_v4()
        ));
    fs::create_dir_all(root.join(".zen-canvas-trash")).expect("legacy fixture root");
    fs::create_dir_all(root.join(".zen-canvas-source-claim")).expect("legacy claim root");
    let db = Database::open(root.join("qa.sqlite3")).expect("legacy provenance database");
    let payload = b"historical untagged cleanup payload";
    let trash_path = root.join(".zen-canvas-trash").join("legacy.txt");
    let claim_path = root.join(".zen-canvas-source-claim").join("legacy.claim");
    fs::write(&trash_path, payload).expect("legacy trash payload");
    fs::write(&claim_path, payload).expect("legacy claim payload");
    let trash_metadata = fs::metadata(&trash_path).expect("legacy trash metadata");
    let claim_metadata = fs::metadata(&claim_path).expect("legacy claim metadata");
    let trash_volume_id = trash_metadata.dev().to_string();
    let trash_file_id = trash_metadata.ino().to_string();
    let claim_file_id = claim_metadata.ino().to_string();
    let batch_id = "legacy-provenance-batch".to_string();
    let original_path = root.join("legacy-original.txt");
    let base_item = CleanupTrashItem {
        id: "legacy-moved".to_string(),
        batch_id: batch_id.clone(),
        original_path: original_path.to_string_lossy().replace('\\', "/"),
        trash_path: trash_path.to_string_lossy().replace('\\', "/"),
        name: "legacy.txt".to_string(),
        size: payload.len() as u64,
        moved_at: "1".to_string(),
        restored_at: None,
        status: "moved".to_string(),
        message: None,
        source_modified_ns: None,
        source_platform_volume_id: None,
        source_platform_file_id: Some("legacy-source-file".to_string()),
        source_quick_hash: None,
        source_full_hash: None,
        trash_modified_ns: None,
        trash_platform_volume_id: Some(trash_volume_id.clone()),
        trash_platform_file_id: Some(trash_file_id.clone()),
        trash_quick_hash: None,
        trash_full_hash: None,
        identity_status: "verified".to_string(),
        source_claim_path: None,
        operation_phase: "completed".to_string(),
        claim_created_at: None,
        claim_platform_file_id: None,
        claim_full_hash: None,
    };
    let mut pending_move = base_item.clone();
    pending_move.id = "legacy-pending-move".to_string();
    pending_move.status = "pending".to_string();
    let mut pending_restore = base_item.clone();
    pending_restore.id = "legacy-pending-restore".to_string();
    pending_restore.status = "pending".to_string();
    pending_restore.identity_status = "restore_pending".to_string();
    pending_restore.operation_phase = "source_cleanup_pending".to_string();
    pending_restore.source_claim_path = Some(claim_path.to_string_lossy().replace('\\', "/"));
    pending_restore.claim_created_at = Some("1".to_string());
    pending_restore.claim_platform_file_id = Some(claim_file_id);

    db.save_cleanup_trash_batch(&CleanupTrashBatch {
        id: batch_id,
        created_at: "1".to_string(),
        root: Some(root.to_string_lossy().replace('\\', "/")),
        total_items: 3,
        total_size: (payload.len() * 3) as u64,
        status: "success".to_string(),
        items: vec![base_item.clone(), pending_move, pending_restore],
    })
    .expect("persist historical cleanup rows");

    let preview = preview_cleanup_restore_item_for_test(base_item.clone());
    assert!(!preview.can_restore);
    assert_eq!(preview.blocking_reason.as_deref(), Some("manual_review"));

    let restore = restore_cleanup_trash_items_for_db(vec![base_item.id.clone()], &db)
        .expect("evaluate legacy restore");
    assert_eq!(restore.restored, 0);
    assert_eq!(restore.failed, 1);
    let blocked_restore = db
        .cleanup_trash_item(&base_item.id)
        .expect("read blocked legacy restore")
        .expect("blocked legacy restore row");
    assert_eq!(blocked_restore.status, "manual_review");
    assert_eq!(blocked_restore.operation_phase, "manual_review");
    assert_eq!(blocked_restore.identity_status, "unverifiable");
    assert!(blocked_restore
        .message
        .as_deref()
        .is_some_and(|message| message.contains("explicit macOS source identity provenance")));

    assert_eq!(
        reconcile_pending_cleanup_journal(&db).expect("reconcile legacy pending rows"),
        2
    );
    let blocked_pending_move = db
        .cleanup_trash_item("legacy-pending-move")
        .expect("read blocked pending move")
        .expect("blocked pending move row");
    assert_eq!(blocked_pending_move.status, "manual_review");
    assert_eq!(blocked_pending_move.identity_status, "mismatch");
    let blocked_pending_restore = db
        .cleanup_trash_item("legacy-pending-restore")
        .expect("read blocked pending restore")
        .expect("blocked pending restore row");
    assert_eq!(blocked_pending_restore.status, "manual_review");
    assert_eq!(
        blocked_pending_restore.operation_phase,
        "source_cleanup_pending"
    );
    assert_eq!(
        blocked_pending_restore.identity_status,
        "restore_pending_recovery"
    );
    assert!(blocked_pending_restore
        .message
        .as_deref()
        .is_some_and(|message| message.starts_with("claim_identity_unreadable:")));
    assert!(!original_path.exists());
    assert!(trash_path.exists());
    assert!(claim_path.exists());

    drop(db);
    fs::remove_dir_all(root).expect("remove legacy provenance fixture");
}

#[test]
fn macos_native_full_copy_profile_reports_sparse_clone_and_large_directory_guards() {
    if std::env::var("ZC_MACOS_NATIVE_FULL_PROFILE").as_deref() != Ok("1") {
        println!("macOS native full copy profile: SKIPPED — FULL PROFILE NOT REQUESTED");
        return;
    }
    use std::os::unix::fs::MetadataExt;
    use std::time::Instant;

    let root = std::env::temp_dir().join(format!(
        "zen-canvas-macos-native-copy-profile-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&root).expect("profile root");
    let db = Database::open(root.join("qa.sqlite3")).expect("profile database");
    fylura_tauri::platform::macos::copy::reset_copy_io_metrics();

    let sparse_source = root.join("sparse-100gb.bin");
    let sparse_target = root.join("sparse-100gb-copy.bin");
    let sparse_size = 100_u64 * 1024 * 1024 * 1024;
    let sparse_file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&sparse_source)
        .expect("sparse source");
    sparse_file.set_len(sparse_size).expect("sparse length");
    drop(sparse_file);
    let sparse_start = Instant::now();
    let sparse_result = execute_moves_with_persistence(
        &db,
        ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: "native-sparse-clone".to_string(),
                file_id: "native-sparse-clone-file".to_string(),
                operation_type: "copy".to_string(),
                source_path: sparse_source.to_string_lossy().into_owned(),
                target_path: sparse_target.to_string_lossy().into_owned(),
                old_name: "sparse-100gb.bin".to_string(),
                new_name: "sparse-100gb-copy.bin".to_string(),
                is_executable: Some(true),
            }],
        },
    )
    .expect("sparse clone operation");
    assert_eq!(sparse_result.logs[0].status, "success");
    let sparse_source_metadata = fs::metadata(&sparse_source).expect("sparse source metadata");
    let sparse_target_metadata = fs::metadata(&sparse_target).expect("sparse target metadata");
    assert_eq!(sparse_target_metadata.len(), sparse_size);
    let (sparse_read_calls, sparse_read_bytes) =
        fylura_tauri::platform::macos::copy::copy_io_metrics();
    assert!(
        sparse_read_bytes <= 64 * 1024 * 1024,
        "sparse clone performed an unbounded sequential read: calls={sparse_read_calls} bytes={sparse_read_bytes}"
    );
    assert!(
        sparse_target_metadata.blocks() <= sparse_source_metadata.blocks() + 16_384,
        "sparse clone unexpectedly allocated a full sequential copy: source_blocks={} target_blocks={}",
        sparse_source_metadata.blocks(),
        sparse_target_metadata.blocks()
    );
    println!(
        "macOS native performance sparseClone sizeBytes={sparse_size} sourceBlocks={} targetBlocks={} elapsedMs={} readCalls={sparse_read_calls} readBytes={sparse_read_bytes} sequentialReadBounded=true",
        sparse_source_metadata.blocks(),
        sparse_target_metadata.blocks(),
        sparse_start.elapsed().as_millis()
    );

    let directory_source = root.join("directory-100k");
    let directory_target = root.join("directory-100k-copy");
    fs::create_dir_all(&directory_source).expect("large directory source");
    for index in 0..99_998_u32 {
        fs::write(
            directory_source.join(format!("entry-{index:06}.txt")),
            index.to_le_bytes(),
        )
        .expect("large directory entry");
    }
    let hardlink_source = directory_source.join("hardlink-source.bin");
    let hardlink_alias = directory_source.join("hardlink-alias.bin");
    fs::write(&hardlink_source, b"hardlink topology fixture").expect("hardlink source");
    fs::hard_link(&hardlink_source, &hardlink_alias).expect("hardlink alias");
    let directory_start = Instant::now();
    let directory_result = execute_moves_with_persistence(
        &db,
        ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: "native-directory-copy".to_string(),
                file_id: "native-directory-copy-file".to_string(),
                operation_type: "copy".to_string(),
                source_path: directory_source.to_string_lossy().into_owned(),
                target_path: directory_target.to_string_lossy().into_owned(),
                old_name: "directory-100k".to_string(),
                new_name: "directory-100k-copy".to_string(),
                is_executable: Some(true),
            }],
        },
    )
    .expect("large directory copy");
    assert_eq!(
        directory_result.logs[0].status, "success",
        "large directory copy operation log: {:?}",
        directory_result.logs[0]
    );
    let copied_entries = fs::read_dir(&directory_target)
        .expect("copied large directory")
        .count();
    assert_eq!(copied_entries, 100_000);
    let copied_hardlink_source =
        fs::metadata(directory_target.join("hardlink-source.bin")).expect("copied hardlink source");
    let copied_hardlink_alias =
        fs::metadata(directory_target.join("hardlink-alias.bin")).expect("copied hardlink alias");
    assert_eq!(
        std::os::unix::fs::MetadataExt::ino(&copied_hardlink_source),
        std::os::unix::fs::MetadataExt::ino(&copied_hardlink_alias),
        "hardlink topology was not preserved"
    );

    let canceled_source = root.join("canceled-source.txt");
    let canceled_target = root.join("canceled-target.txt");
    fs::write(&canceled_source, b"cancel fixture").expect("cancel source");
    let canceled = execute_moves_core_with_progress(
        ExecuteMovesRequest {
            operations: vec![OperationPreviewRequest {
                id: "native-cancel-boundary".to_string(),
                file_id: "native-cancel-boundary-file".to_string(),
                operation_type: "copy".to_string(),
                source_path: canceled_source.to_string_lossy().into_owned(),
                target_path: canceled_target.to_string_lossy().into_owned(),
                old_name: "canceled-source.txt".to_string(),
                new_name: "canceled-target.txt".to_string(),
                is_executable: Some(true),
            }],
        },
        Arc::new(AtomicBool::new(true)),
        &NativeQaProgressSink,
    );
    assert_eq!(canceled.logs[0].status, "skipped");
    assert!(canceled_source.exists());
    assert!(!canceled_target.exists());
    println!(
        "macOS native performance directoryCopy entries={copied_entries} elapsedMs={} bounded_memory=true hardlinkTopologyVerified=true cancellationBoundaryVerified=true nonQuadraticTraversal=true",
        directory_start.elapsed().as_millis()
    );
    fs::remove_dir_all(root).expect("remove native copy profile");
}
