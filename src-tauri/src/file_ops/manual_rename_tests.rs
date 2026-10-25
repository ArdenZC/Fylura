use super::*;
use crate::db::{
    CreateUserTagRequest, InsertFileRequest, LibrarySelectionV1, MutateFileUserTagsRequest,
    UserTagMutationOperation,
};

struct Fixture {
    root: PathBuf,
    db: Database,
    source: PathBuf,
    id: String,
}
impl Fixture {
    fn new() -> Self {
        Self::with_legacy_id(false)
    }
    fn with_legacy_id(legacy: bool) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp-tests")
            .join(format!("m1-r4-{}", uuid::Uuid::new_v4()));
        // The parent deliberately resembles a filename: it must never be
        // interpreted as a suggestion target and turn Rename into Move.
        fs::create_dir_all(root.join("folder.txt")).unwrap();
        let source = root.join("folder.txt/original.txt");
        fs::write(&source, b"owned fixture").unwrap();
        let db = Database::open(root.join("fixture.sqlite3")).unwrap();
        let id = if legacy {
            normalize_path(&source)
        } else {
            format!("zen-canvas-source:{}", uuid::Uuid::new_v4())
        };
        let metadata = fs::metadata(&source).unwrap();
        db.insert_file(InsertFileRequest {
            id: id.clone(),
            path: source.to_string_lossy().into_owned(),
            name: "original.txt".into(),
            extension: "txt".into(),
            size: metadata.len() as i64,
            mtime: metadata
                .modified()
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
            ctime: 0,
            is_dir: false,
            state_code: 0,
        })
        .unwrap();
        Self {
            root,
            db,
            source,
            id,
        }
    }
    fn selection(&self, preview: &crate::db::OperationPreviewDto) -> ExecuteMovesByIdRequest {
        let fingerprint = preview.operation_fingerprint.clone().unwrap();
        ExecuteMovesByIdRequest {
            operations: vec![OperationSelection {
                id: preview.id.clone(),
                file_id: self.id.clone(),
                operation_fingerprint: fingerprint.clone(),
                expected_revision: fingerprint,
                new_name: Some(preview.new_name.clone()),
            }],
        }
    }
    fn finish(self) {
        drop(self.db);
        fs::remove_dir_all(self.root).unwrap();
    }
}

#[test]
fn m1_r4_manual_rename_preview_execute_history_restore_preserves_file_and_tags() {
    for legacy in [false, true] {
        let fixture = Fixture::with_legacy_id(legacy);
        let db = &fixture.db;
        let tag = db
            .create_user_tag(CreateUserTagRequest {
                display_name: "Owned fixture".into(),
                color_token: "blue".into(),
            })
            .unwrap();
        db.mutate_file_user_tags(MutateFileUserTagsRequest {
            selection: LibrarySelectionV1::Explicit {
                file_ids: vec![fixture.id.clone()],
            },
            tag_ids: vec![tag.id.clone()],
            operation: UserTagMutationOperation::Add,
            expected_count: Some(1),
        })
        .unwrap();
        assert!(db
            .get_operation_previews_by_file_ids(std::slice::from_ref(&fixture.id))
            .unwrap()
            .is_empty());
        let preview = db
            .get_manual_rename_operation_preview(&fixture.id, "手动重命名")
            .unwrap();
        assert_eq!(preview.operation_type, "rename");
        assert_eq!(preview.new_name, "手动重命名.txt");
        assert_eq!(
            Path::new(&preview.target_path).parent(),
            fixture.source.parent()
        );
        assert!(preview.requires_confirmation);
        assert_eq!(preview.selected_by_default, Some(false));
        assert!(db.get_operation_logs(Some(10)).unwrap().is_empty());
        assert!(fixture.source.exists()); // Preview performs no filesystem write.
        let request = resolve_execute_selections(db, fixture.selection(&preview)).unwrap();
        let result = execute_moves_with_persistence(db, request).unwrap();
        assert_eq!(result.logs[0].status, "success");
        assert!(!fixture.source.exists());
        assert_eq!(fs::read(&preview.target_path).unwrap(), b"owned fixture");
        let renamed_id = if legacy {
            preview.target_path.clone()
        } else {
            fixture.id.clone()
        };
        let detail = db.get_file_library_detail(&renamed_id).unwrap();
        assert_eq!(detail.name, "手动重命名.txt");
        assert_eq!(detail.tags[0].id, tag.id);
        let logs = db.get_operation_logs(Some(10)).unwrap();
        assert_eq!(logs.len(), 1);
        assert!(logs[0].can_restore);
        let restorable = db
            .get_restorable_operation_logs_by_ids(std::slice::from_ref(&logs[0].id))
            .unwrap();
        let restored =
            restore_moves_with_persistence(db, RestoreMovesRequest { logs: restorable }).unwrap();
        assert_eq!(restored.restored, 1);
        assert_eq!(fs::read(&fixture.source).unwrap(), b"owned fixture");
        assert!(!Path::new(&preview.target_path).exists());
        let detail = db.get_file_library_detail(&fixture.id).unwrap();
        assert_eq!(detail.name, "original.txt");
        assert_eq!(detail.tags[0].id, tag.id);
        assert_eq!(
            db.get_operation_logs(Some(10)).unwrap()[0].restore_status,
            "restored"
        );
        fixture.finish();
    }
}

#[test]
fn m1_r4_manual_rename_conflict_and_changed_name_cannot_reuse_consent() {
    let fixture = Fixture::new();
    let preview = fixture
        .db
        .get_manual_rename_operation_preview(&fixture.id, "next.txt")
        .unwrap();
    let mut forged = fixture.selection(&preview);
    forged.operations[0].new_name = Some("other.txt".into());
    assert!(resolve_execute_selections(&fixture.db, forged).is_err());
    fs::write(&preview.target_path, b"conflicting owned file").unwrap();
    assert!(resolve_execute_selections(&fixture.db, fixture.selection(&preview)).is_err());
    let blocked = fixture
        .db
        .get_manual_rename_operation_preview(&fixture.id, "next.txt")
        .unwrap();
    assert_eq!(blocked.is_executable, Some(false));
    assert!(resolve_execute_selections(&fixture.db, fixture.selection(&blocked)).is_err());
    assert!(fixture.db.get_operation_logs(Some(10)).unwrap().is_empty());
    assert_eq!(fs::read(&fixture.source).unwrap(), b"owned fixture");
    assert_eq!(
        fs::read(&preview.target_path).unwrap(),
        b"conflicting owned file"
    );
    fixture.finish();
}

#[test]
fn m1_r4_manual_rename_replacement_with_same_size_and_mtime_is_stale() {
    let fixture = Fixture::new();
    let preview = fixture
        .db
        .get_manual_rename_operation_preview(&fixture.id, "next.txt")
        .unwrap();
    let modified = fs::metadata(&fixture.source).unwrap().modified().unwrap();
    fs::rename(&fixture.source, fixture.root.join("retained-original.txt")).unwrap();
    fs::write(&fixture.source, b"owned fixture").unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&fixture.source)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert_eq!(
        resolve_execute_selections(&fixture.db, fixture.selection(&preview)).unwrap_err(),
        "operation_preview_stale"
    );
    assert!(fixture.source.exists());
    assert!(!Path::new(&preview.target_path).exists());
    assert!(fixture.db.get_operation_logs(Some(10)).unwrap().is_empty());
    fixture.finish();
}

#[test]
fn m1_r4_manual_rename_readonly_missing_and_invalid_names_fail_closed() {
    let fixture = Fixture::new();
    for name in [
        "",
        "../outside.txt",
        "child/name.txt",
        "next.exe",
        "original.txt",
    ] {
        assert!(
            fixture
                .db
                .get_manual_rename_operation_preview(&fixture.id, name)
                .is_err(),
            "{name}"
        );
    }
    let preview = fixture
        .db
        .get_manual_rename_operation_preview(&fixture.id, "next.txt")
        .unwrap();
    let original = fs::metadata(&fixture.source).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&fixture.source, readonly).unwrap();
    assert!(fixture
        .db
        .get_manual_rename_operation_preview(&fixture.id, "next.txt")
        .is_err());
    assert!(resolve_execute_selections(&fixture.db, fixture.selection(&preview)).is_err());
    fs::set_permissions(&fixture.source, original).unwrap();
    fs::remove_file(&fixture.source).unwrap();
    assert!(fixture
        .db
        .get_manual_rename_operation_preview(&fixture.id, "next.txt")
        .is_err());
    assert!(resolve_execute_selections(&fixture.db, fixture.selection(&preview)).is_err());
    assert!(fixture.db.get_operation_logs(Some(10)).unwrap().is_empty());
    fixture.finish();
}

#[test]
fn m1_r4_manual_rename_cancellation_keeps_original_file() {
    let fixture = Fixture::new();
    let preview = fixture
        .db
        .get_manual_rename_operation_preview(&fixture.id, "next.txt")
        .unwrap();
    let request = resolve_execute_selections(&fixture.db, fixture.selection(&preview)).unwrap();
    let result = execute_moves_with_persistence_with_progress(
        &fixture.db,
        request,
        Arc::new(AtomicBool::new(true)),
        &NoopOperationProgressEmitter,
    )
    .unwrap();
    assert_eq!(result.logs.len(), 1);
    assert!(result
        .logs
        .iter()
        .all(|log| log.status != "success" && !log.can_restore));
    assert!(fixture.source.exists());
    assert!(!Path::new(&preview.target_path).exists());
    fixture.finish();
}

#[cfg(windows)]
#[test]
fn m1_r4_manual_rename_windows_sharing_denial_keeps_source() {
    use std::os::windows::fs::OpenOptionsExt;
    let fixture = Fixture::new();
    let initial = fixture
        .db
        .get_manual_rename_operation_preview(&fixture.id, "next.txt")
        .unwrap();
    let handle = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&fixture.source)
        .unwrap();
    // Windows can still expose read-only metadata through a sharing lock.
    // The safety boundary is admission/journal/handle-based mutation, not stat.
    let preview = fixture
        .db
        .get_manual_rename_operation_preview(&fixture.id, "next.txt")
        .unwrap_or(initial);
    let denied = resolve_execute_selections(&fixture.db, fixture.selection(&preview))
        .and_then(|request| execute_moves_with_persistence(&fixture.db, request));
    if let Ok(result) = denied {
        assert!(!result.logs.is_empty());
        assert!(result
            .logs
            .iter()
            .all(|log| log.status != "success" && !log.can_restore));
    }
    assert!(!Path::new(&preview.target_path).exists());
    assert!(fixture
        .db
        .get_operation_logs(Some(10))
        .unwrap()
        .iter()
        .all(|log| log.status != "success"));
    drop(handle);
    assert_eq!(fs::read(&fixture.source).unwrap(), b"owned fixture");
    fixture.finish();
}
