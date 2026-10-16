use super::*;

#[test]
fn actual_watcher_publication_changes_clock_only_for_filesystem_truth() {
    let f = Fixture::new();
    let db = f.db();
    let root = db.path().parent().unwrap();
    let root_path = crate::db::scan::normalize_scan_root_path(&root.to_string_lossy());
    db.conn()
        .unwrap()
        .execute(
            "UPDATE scan_roots SET normalized_path=?1 WHERE id='root'",
            [&root_path],
        )
        .unwrap();
    let file = root.join("disposable.txt");
    std::fs::write(&file, b"one").unwrap();
    let normalized = crate::db::scan::normalize_scan_root_path(&file.to_string_lossy());
    let clock = || {
        db.conn()
            .unwrap()
            .query_row(
                "SELECT library_change_revision FROM scan_roots WHERE id='root'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
    };
    let publish = |paths: &[String]| {
        let revision = db
            .begin_watcher_revision("root")
            .unwrap()
            .expect("enabled watcher root");
        db.apply_watcher_exact_mutations(
            "root",
            revision.watcher_revision,
            paths,
            &std::collections::HashSet::new(),
        )
        .unwrap()
    };
    publish(std::slice::from_ref(&normalized));
    assert_eq!(clock(), 1);
    let _ = std::fs::read(&file).unwrap();
    publish(std::slice::from_ref(&normalized));
    assert_eq!(clock(), 1);
    // Semantic/presentation publication and global query revision remain
    // separate from the scanner/watcher-owned filesystem clock.
    db.conn().unwrap().execute("UPDATE files SET purpose='work',classification_status='classified',suggested_name='user suggestion' WHERE id=?1",[&normalized]).unwrap();
    db.conn()
        .unwrap()
        .execute("UPDATE library_query_state SET revision=revision+1", [])
        .unwrap();
    publish(std::slice::from_ref(&normalized));
    assert_eq!(clock(), 1);
    std::fs::write(&file, b"changed size").unwrap();
    publish(std::slice::from_ref(&normalized));
    assert_eq!(clock(), 2);
    // One transaction with two changed paths increments this root only once.
    let renamed = root.join("renamed.txt");
    std::fs::rename(&file, &renamed).unwrap();
    let renamed = crate::db::scan::normalize_scan_root_path(&renamed.to_string_lossy());
    publish(&[normalized.clone(), renamed.clone()]);
    assert_eq!(clock(), 3);
    std::fs::remove_file(root.join("renamed.txt")).unwrap();
    publish(std::slice::from_ref(&renamed));
    assert_eq!(clock(), 4);
    publish(std::slice::from_ref(&renamed));
    assert_eq!(clock(), 4);
    std::fs::write(root.join("renamed.txt"), b"new identity").unwrap();
    publish(std::slice::from_ref(&renamed));
    assert_eq!(clock(), 5);
    assert_eq!(
        db.conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM operation_batches", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn watcher_detects_replaced_physical_identity_with_equal_size_and_mtime() {
    let f = Fixture::new();
    let db = f.db();
    let root = db.path().parent().unwrap();
    db.conn()
        .unwrap()
        .execute(
            "UPDATE scan_roots SET normalized_path=?1 WHERE id='root'",
            [crate::db::scan::normalize_scan_root_path(
                &root.to_string_lossy(),
            )],
        )
        .unwrap();
    let file = root.join("identity.txt");
    std::fs::write(&file, b"same").unwrap();
    let path = crate::db::scan::normalize_scan_root_path(&file.to_string_lossy());
    let publish = || {
        let revision = db
            .begin_watcher_revision("root")
            .unwrap()
            .expect("enabled watcher root");
        db.apply_watcher_exact_mutations(
            "root",
            revision.watcher_revision,
            std::slice::from_ref(&path),
            &std::collections::HashSet::new(),
        )
        .unwrap()
    };
    publish();
    let before = crate::fs_safety::capture_physical_identity(&file).unwrap();
    let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
    let replacement = root.join("replacement.txt");
    std::fs::write(&replacement, b"same").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    std::fs::remove_file(&file).unwrap();
    std::fs::rename(&replacement, &file).unwrap();
    let after = crate::fs_safety::capture_physical_identity(&file).unwrap();
    assert_ne!(before.physical_key, after.physical_key);
    assert_eq!(before.size, after.size);
    assert_eq!(before.modified_ns, after.modified_ns);
    // Match the platform creation timestamp as well: identity is the only
    // represented filesystem difference left when the publication compares.
    let ctime = std::fs::metadata(&file)
        .unwrap()
        .created()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|t| t.as_secs() as i64)
        .unwrap_or(0);
    db.conn()
        .unwrap()
        .execute(
            "UPDATE files SET ctime=?2 WHERE path=?1",
            params![path, ctime],
        )
        .unwrap();
    publish();
    assert_eq!(
        db.conn()
            .unwrap()
            .query_row(
                "SELECT library_change_revision FROM scan_roots WHERE id='root'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    publish();
    assert_eq!(
        db.conn()
            .unwrap()
            .query_row(
                "SELECT library_change_revision FROM scan_roots WHERE id='root'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
}

#[test]
fn actual_tag_and_semantic_metadata_publication_do_not_trigger() {
    use crate::db::{
        CreateUserTagRequest, LibrarySelectionV1, MutateFileUserTagsRequest,
        UserTagMutationOperation,
    };
    let f = Fixture::new();
    f.file();
    f.set_watcher_roots(vec![watcher_root_setting("settings-root", "/tmp")]);
    intent(
        &f,
        trigger(json!({"version":2,"kind":"managed_scope_change"})),
    );
    let tag = f
        .db()
        .create_user_tag(CreateUserTagRequest {
            display_name: "Review".into(),
            color_token: "blue".into(),
        })
        .unwrap();
    f.db()
        .mutate_file_user_tags(MutateFileUserTagsRequest {
            selection: LibrarySelectionV1::Explicit {
                file_ids: vec!["file".into()],
            },
            tag_ids: vec![tag.id],
            operation: UserTagMutationOperation::Add,
            expected_count: Some(1),
        })
        .unwrap();
    crate::db::queries::organization::tests::seed_current_managed_semantics_for_file(
        f.db(),
        "file",
    );
    assert!(f.db().select_automation_trigger(100).unwrap().0.is_none());
    assert_eq!(
        f.db()
            .conn()
            .unwrap()
            .query_row(
                "SELECT library_change_revision FROM scan_roots WHERE id='root'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert!(f.db().list_automation_runs(None).unwrap().is_empty());
}
