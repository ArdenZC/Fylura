use super::manual_rename_tests::Fixture;
use super::*;
use crate::db::{
    CreateUserTagRequest, LibrarySelectionV1, MutateFileUserTagsRequest, OperationPreviewDto,
    UserTagMutationOperation,
};
use crate::fs_safety::{DirectoryIdentity, VerifiedDirectory};

fn selection(
    f: &Fixture,
    preview: &OperationPreviewDto,
    directory: &Path,
) -> ExecuteMovesByIdRequest {
    let mut selected = f.selection(preview);
    selected.operations[0].destination_directory = Some(normalize_path(directory));
    selected
}

fn destination(f: &Fixture) -> PathBuf {
    let directory = f.root.join("destination.txt");
    fs::create_dir(&directory).unwrap();
    directory
}

#[test]
fn m1_r4b_move_history_restore_preserves_contents_physical_identity_and_tags() {
    for legacy in [false, true] {
        let f = Fixture::with_legacy_id(legacy);
        let directory = destination(&f);
        let original = file_namespace_fingerprint(&f.source).unwrap();
        let tag =
            f.db.create_user_tag(CreateUserTagRequest {
                display_name: "Move fixture".into(),
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
        let preview =
            f.db.get_manual_move_operation_preview(
                &f.id,
                &normalize_path(&directory),
                legacy.then_some("chosen.txt"),
            )
            .unwrap();
        assert_eq!(
            preview.operation_type,
            if legacy { "move_rename" } else { "move" }
        );
        assert_eq!(
            Path::new(&preview.target_path).parent(),
            Some(directory.as_path())
        );
        assert!(preview.requires_confirmation);
        assert_eq!(preview.selected_by_default, Some(false));
        assert!(f.db.get_operation_logs(Some(10)).unwrap().is_empty());
        let request =
            resolve_execute_selections(&f.db, selection(&f, &preview, &directory)).unwrap();
        let result = execute_moves_with_persistence(&f.db, request).unwrap();
        assert_eq!(result.logs[0].status, "success");
        assert!(!f.source.exists());
        assert_eq!(fs::read(&preview.target_path).unwrap(), b"owned fixture");
        let moved = file_namespace_fingerprint(Path::new(&preview.target_path)).unwrap();
        assert_eq!(moved.platform_volume_id, original.platform_volume_id);
        assert_eq!(moved.platform_file_id, original.platform_file_id);
        let moved_id = if legacy { &preview.target_path } else { &f.id };
        let detail = f.db.get_file_library_detail(moved_id).unwrap();
        assert_eq!(detail.tags[0].id, tag.id);
        let history = f.db.get_operation_logs(Some(10)).unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].can_restore);
        let logs =
            f.db.get_restorable_operation_logs_by_ids(std::slice::from_ref(&history[0].id))
                .unwrap();
        let restored = restore_moves_with_persistence(&f.db, RestoreMovesRequest { logs }).unwrap();
        assert_eq!(restored.restored, 1);
        assert_eq!(fs::read(&f.source).unwrap(), b"owned fixture");
        assert!(!Path::new(&preview.target_path).exists());
        assert_eq!(
            file_namespace_fingerprint(&f.source)
                .unwrap()
                .platform_file_id,
            original.platform_file_id
        );
        assert_eq!(
            f.db.get_file_library_detail(&f.id).unwrap().tags[0].id,
            tag.id
        );
        assert_eq!(
            f.db.get_operation_logs(Some(10)).unwrap()[0].restore_status,
            "restored"
        );
        f.finish();
    }
}

#[test]
fn m1_r4b_conflict_rechoose_and_changes_after_consent_require_new_preview() {
    let f = Fixture::new();
    let directory = destination(&f);
    let target = directory.join("original.txt");
    fs::write(&target, b"existing target").unwrap();
    let blocked =
        f.db.get_manual_move_operation_preview(&f.id, &normalize_path(&directory), None)
            .unwrap();
    assert_eq!(
        blocked.blocking_reason.as_deref(),
        Some("manual_file_target_exists")
    );
    assert_eq!(blocked.is_executable, Some(false));
    assert!(resolve_execute_selections(&f.db, selection(&f, &blocked, &directory)).is_err());
    let renamed = f
        .db
        .get_manual_move_operation_preview(&f.id, &normalize_path(&directory), Some("chosen.txt"))
        .unwrap();
    assert_eq!(renamed.operation_type, "move_rename");
    let mut changed_name = selection(&f, &renamed, &directory);
    changed_name.operations[0].new_name = Some("another.txt".into());
    assert!(resolve_execute_selections(&f.db, changed_name).is_err());
    let other = f.root.join("other");
    fs::create_dir(&other).unwrap();
    assert!(resolve_execute_selections(&f.db, selection(&f, &renamed, &other)).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"existing target");
    fs::remove_file(&target).unwrap();
    let clear =
        f.db.get_manual_move_operation_preview(&f.id, &normalize_path(&directory), None)
            .unwrap();
    assert_ne!(clear.operation_fingerprint, blocked.operation_fingerprint);
    fs::write(&target, b"late target").unwrap();
    assert!(resolve_execute_selections(&f.db, selection(&f, &clear, &directory)).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"late target");
    assert!(f.source.exists());
    assert!(f.db.get_operation_logs(Some(10)).unwrap().is_empty());
    f.finish();
}

#[test]
fn m1_r4b_stale_source_same_size_mtime_and_replaced_destination_fail_closed() {
    let f = Fixture::new();
    let directory = destination(&f);
    let preview =
        f.db.get_manual_move_operation_preview(&f.id, &normalize_path(&directory), None)
            .unwrap();
    let request = resolve_execute_selections(&f.db, selection(&f, &preview, &directory)).unwrap();
    fs::rename(&directory, f.root.join("retained-destination")).unwrap();
    fs::create_dir(&directory).unwrap();
    assert!(resolve_execute_selections(&f.db, selection(&f, &preview, &directory)).is_err());
    assert!(execute_moves_with_persistence(&f.db, request).is_err());
    let fresh =
        f.db.get_manual_move_operation_preview(&f.id, &normalize_path(&directory), None)
            .unwrap();
    let source_request =
        resolve_execute_selections(&f.db, selection(&f, &fresh, &directory)).unwrap();
    let mtime = fs::metadata(&f.source).unwrap().modified().unwrap();
    fs::rename(&f.source, f.root.join("retained-source.txt")).unwrap();
    fs::write(&f.source, b"owned fixture").unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&f.source)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    assert!(resolve_execute_selections(&f.db, selection(&f, &fresh, &directory)).is_err());
    assert!(execute_moves_with_persistence(&f.db, source_request).is_err());
    assert!(f.db.get_operation_logs(Some(10)).unwrap().is_empty());
    assert!(!Path::new(&preview.target_path).exists());
    f.finish();
}

fn expected_source(f: &Fixture) -> crate::fs_safety::ExpectedFileIdentity {
    let value = file_identity_fingerprint(&f.source).unwrap();
    crate::fs_safety::ExpectedFileIdentity {
        size: value.size,
        modified_ns: value.modified_ns,
        platform_volume_id: value.platform_volume_id,
        platform_file_id: value.platform_file_id,
        sample_hash: value.quick_hash,
        full_hash: value.full_hash,
    }
}

#[test]
fn m1_r4b_atomic_destination_guard_rejects_reopen_race_and_cross_volume_before_claim() {
    let f = Fixture::new();
    let directory = destination(&f);
    let old: DirectoryIdentity = VerifiedDirectory::open_existing(&directory)
        .unwrap()
        .identity()
        .clone();
    let mut expected = expected_source(&f);
    let target = directory.join("original.txt");
    expected.platform_volume_id = Some("another-volume".into());
    let attempt = |identity: &crate::fs_safety::ExpectedFileIdentity| {
        crate::fs_safety::atomic_move::atomic_move_with_verified_destination(
            &f.source,
            &target,
            Some(identity),
            None,
            None,
            None,
            crate::fs_safety::atomic_move::AtomicMoveOperation::Move,
            None,
            Some(&old),
        )
    };
    assert!(matches!(
        attempt(&expected),
        Err(crate::fs_safety::AtomicMoveError::CrossDevice)
    ));
    fs::rename(&directory, f.root.join("retained-directory")).unwrap();
    fs::create_dir(&directory).unwrap();
    assert!(matches!(
        attempt(&expected_source(&f)),
        Err(crate::fs_safety::AtomicMoveError::TargetParentIdentityChanged)
    ));
    assert!(f.source.exists());
    assert!(!target.exists());
    f.finish();
}

#[test]
fn m1_r4b_invalid_unavailable_readonly_and_cancelled_move_keeps_source() {
    let f = Fixture::new();
    let directory = destination(&f);
    for path in [
        f.source.clone(),
        f.root.join("missing"),
        f.source.parent().unwrap().to_path_buf(),
        directory.join("../destination.txt"),
    ] {
        assert!(f
            .db
            .get_manual_move_operation_preview(&f.id, &normalize_path(&path), None)
            .is_err());
    }
    let preview =
        f.db.get_manual_move_operation_preview(&f.id, &normalize_path(&directory), None)
            .unwrap();
    let original = fs::metadata(&f.source).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&f.source, readonly).unwrap();
    assert!(resolve_execute_selections(&f.db, selection(&f, &preview, &directory)).is_err());
    fs::set_permissions(&f.source, original).unwrap();
    let request = resolve_execute_selections(&f.db, selection(&f, &preview, &directory)).unwrap();
    let cancelled = execute_moves_with_persistence_with_progress(
        &f.db,
        request,
        Arc::new(AtomicBool::new(true)),
        &NoopOperationProgressEmitter,
    )
    .unwrap();
    assert_eq!(cancelled.logs[0].status, "skipped");
    assert!(f.source.exists());
    assert!(!Path::new(&preview.target_path).exists());
    fs::remove_file(&f.source).unwrap();
    assert!(f
        .db
        .get_manual_move_operation_preview(&f.id, &normalize_path(&directory), None)
        .is_err());
    f.finish();
}

#[test]
fn m1_r4b_destination_link_boundary_is_rejected() {
    let f = Fixture::new();
    let directory = destination(&f);
    let alias = f.root.join("linked-destination");
    #[cfg(target_os = "macos")]
    std::os::unix::fs::symlink(&directory, &alias).unwrap();
    #[cfg(windows)]
    assert!(std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&alias)
        .arg(&directory)
        .output()
        .unwrap()
        .status
        .success());
    assert!(f
        .db
        .get_manual_move_operation_preview(&f.id, &normalize_path(&alias), None)
        .is_err());
    #[cfg(windows)]
    fs::remove_dir(&alias).unwrap();
    #[cfg(target_os = "macos")]
    fs::remove_file(&alias).unwrap();
    assert!(f.source.exists());
    f.finish();
}

#[cfg(windows)]
#[test]
fn m1_r4b_windows_sharing_denial_refuses_source_and_destination() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    let directory = destination(&f);
    let preview =
        f.db.get_manual_move_operation_preview(&f.id, &normalize_path(&directory), None)
            .unwrap();
    let handle = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&f.source)
        .unwrap();
    let result = resolve_execute_selections(&f.db, selection(&f, &preview, &directory))
        .and_then(|request| execute_moves_with_persistence(&f.db, request));
    if let Ok(result) = result {
        assert!(result.logs.iter().all(|log| log.status != "success"));
    }
    assert!(!Path::new(&preview.target_path).exists());
    drop(handle);
    let directory_lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .custom_flags(0x02000000)
        .open(&directory)
        .unwrap();
    assert!(f
        .db
        .get_manual_move_operation_preview(&f.id, &normalize_path(&directory), None)
        .is_err());
    drop(directory_lock);
    assert_eq!(fs::read(&f.source).unwrap(), b"owned fixture");
    f.finish();
}

#[cfg(target_os = "macos")]
#[test]
fn m1_r4b_macos_unwritable_destination_does_not_retire_source() {
    use std::os::unix::fs::PermissionsExt;
    assert_ne!(
        unsafe { libc::geteuid() },
        0,
        "permission-denial fixture requires an unprivileged user"
    );
    let f = Fixture::new();
    let directory = destination(&f);
    let preview =
        f.db.get_manual_move_operation_preview(&f.id, &normalize_path(&directory), None)
            .unwrap();
    let original = fs::metadata(&directory).unwrap().permissions();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o555)).unwrap();
    let result = resolve_execute_selections(&f.db, selection(&f, &preview, &directory))
        .and_then(|request| execute_moves_with_persistence(&f.db, request));
    fs::set_permissions(&directory, original).unwrap();
    if let Ok(result) = result {
        assert!(result.logs.iter().all(|log| log.status != "success"));
    }
    assert!(f.source.exists());
    assert!(!Path::new(&preview.target_path).exists());
    f.finish();
}
