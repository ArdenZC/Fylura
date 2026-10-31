use super::manual_rename_tests::Fixture;
use super::manual_transfer::*;
use super::*;
use crate::file_workspace::integration::types::BrowseEntryKindDto;
use crate::file_workspace::integration::{
    types::{BrowseOpenRequest, BrowseSessionRequest, BrowseStartEnumerationRequest},
    FileWorkspaceRuntime,
};
use crate::file_workspace::{EntryRef, WorkspacePlatform};
use crate::platform::macos::quick_look::MacThumbnailService;
fn runtime(f: &Fixture) -> FileWorkspaceRuntime {
    FileWorkspaceRuntime::new(
        f.db.clone(),
        MacThumbnailService::new(f.root.join("legacy-cache")),
        f.root.join("thumbnail-cache"),
    )
    .unwrap()
}
fn capture(runtime: &FileWorkspaceRuntime, f: &Fixture) -> Vec<TransferTicket> {
    runtime
        .capture_transfer(CaptureRequest {
            refs: vec![EntryRef::Managed {
                file_id: f.id.clone(),
            }],
            selection: None,
            release_ids: vec![],
            external_paths: vec![],
        })
        .unwrap()
}
fn preview(
    runtime: &FileWorkspaceRuntime,
    ids: Vec<String>,
    target: &Path,
    action: TransferAction,
    both: Vec<String>,
    skip: Vec<String>,
) -> TransferPreview {
    runtime
        .preview_transfer(TransferPreviewRequest {
            ticket_ids: ids,
            action,
            destination_directory: normalize_path(target),
            destination_entry: None,
            new_name: None,
            keep_both: both,
            skip,
        })
        .unwrap()
}
#[test]
fn u1_batch_collects_managed_and_disposed_browse_sources_then_copies_with_journal() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    fs::create_dir(f.root.join("other")).unwrap();
    fs::write(f.root.join("other/second.txt"), b"second bytes").unwrap();
    let browse = runtime
        .open_browse(BrowseOpenRequest {
            platform: if cfg!(windows) {
                WorkspacePlatform::Windows
            } else {
                WorkspacePlatform::Macos
            },
            routing_hint: normalize_path(&f.root.join("other")),
            display_hint: None,
            global_entry_id: None,
        })
        .unwrap();
    let page = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: browse.session_id.clone(),
            request_id: "capture".into(),
            path_ref: browse.root_path_ref,
            page_size: 32,
            query: Default::default(),
        })
        .unwrap();
    let reference = match page.entries[0].entry_ref.clone() {
        crate::file_workspace::BrowseEntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        } => EntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        },
    };
    let mut tickets = capture(&runtime, &f);
    tickets.extend(
        runtime
            .capture_transfer(CaptureRequest {
                refs: vec![reference],
                selection: None,
                release_ids: vec![],
                external_paths: vec![],
            })
            .unwrap(),
    );
    runtime
        .dispose_browse(BrowseSessionRequest {
            session_id: browse.session_id,
        })
        .unwrap();
    let target = f.root.join("target");
    fs::create_dir(&target).unwrap();
    let plan = preview(
        &runtime,
        tickets.iter().map(|t| t.id.clone()).collect(),
        &target,
        TransferAction::Copy,
        tickets.iter().map(|ticket| ticket.id.clone()).collect(),
        vec![],
    );
    assert!(plan.rows.iter().all(|r| r.error.is_none()));
    let request = runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: plan.id.clone(),
            confirmed: true,
        })
        .unwrap();
    assert!(runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: plan.id,
            confirmed: true
        })
        .is_err());
    let result = execute_moves_with_persistence(&f.db, request).unwrap();
    assert_eq!(result.logs.len(), 2);
    assert!(result
        .logs
        .iter()
        .all(|l| l.status == "success" && !l.can_restore));
    assert_eq!(
        fs::read(target.join("original.txt")).unwrap(),
        b"owned fixture"
    );
    assert_eq!(
        fs::read(target.join("second.txt")).unwrap(),
        b"second bytes"
    );
    assert_eq!(fs::read(&f.source).unwrap(), b"owned fixture");
    assert_eq!(f.db.get_operation_logs(Some(10)).unwrap().len(), 2);
    runtime.dispose();
}
#[test]
fn u1_conflict_keep_both_skip_and_cancel_do_not_overwrite() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let tickets = capture(&runtime, &f);
    let id = tickets[0].id.clone();
    let target = f.root.join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("original.txt"), b"existing destination").unwrap();
    let conflict = preview(
        &runtime,
        vec![id.clone()],
        &target,
        TransferAction::Copy,
        vec![],
        vec![],
    );
    assert_eq!(
        conflict.rows[0].error.as_deref(),
        Some("transfer_target_exists")
    );
    assert!(runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: conflict.id,
            confirmed: true
        })
        .is_err());
    let skipped = preview(
        &runtime,
        vec![id.clone()],
        &target,
        TransferAction::Copy,
        vec![],
        vec![id.clone()],
    );
    assert!(skipped.rows[0].skipped);
    assert!(skipped.rows[0].error.is_none());
    assert!(skipped.rows[0].preview.is_none());
    assert!(runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: skipped.id,
            confirmed: true,
        })
        .is_err());
    assert_eq!(
        fs::read(target.join("original.txt")).unwrap(),
        b"existing destination"
    );
    let both = preview(
        &runtime,
        vec![id.clone()],
        &target,
        TransferAction::Copy,
        vec![id],
        vec![],
    );
    assert_eq!(
        both.rows[0].preview.as_ref().unwrap().new_name,
        "original (2).txt"
    );
    let request = runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: both.id,
            confirmed: true,
        })
        .unwrap();
    let result = execute_moves_with_persistence_with_progress(
        &f.db,
        request,
        Arc::new(AtomicBool::new(true)),
        &NoopOperationProgressEmitter,
    )
    .unwrap();
    assert!(result.logs.iter().all(|l| l.status != "success"));
    assert_eq!(
        fs::read(target.join("original.txt")).unwrap(),
        b"existing destination"
    );
    assert!(!target.join("original (2).txt").exists());
    assert!(f.source.exists());
    runtime.dispose();
}

#[test]
fn fl20_skip_still_revalidates_ticket_identity() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let id = capture(&runtime, &f)[0].id.clone();
    let target = f.root.join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("original.txt"), b"existing destination").unwrap();
    fs::write(&f.source, b"source changed after capture").unwrap();

    let preview = preview(
        &runtime,
        vec![id.clone()],
        &target,
        TransferAction::Copy,
        vec![],
        vec![id],
    );

    assert!(!preview.rows[0].skipped);
    assert_eq!(
        preview.rows[0].error.as_deref(),
        Some("transfer_source_changed")
    );
    assert_eq!(
        fs::read(target.join("original.txt")).unwrap(),
        b"existing destination"
    );
    runtime.dispose();
}

#[test]
fn u1_cut_move_restore_preserves_identity_and_changed_source_or_destination_rejects() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let id = capture(&runtime, &f)[0].id.clone();
    let target = f.root.join("target");
    fs::create_dir(&target).unwrap();
    let plan = preview(
        &runtime,
        vec![id.clone()],
        &target,
        TransferAction::Move,
        vec![],
        vec![],
    );
    assert!(runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: plan.id.clone(),
            confirmed: false
        })
        .is_err());
    fs::rename(&target, f.root.join("retired-target")).unwrap();
    fs::create_dir(&target).unwrap();
    assert!(runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: plan.id,
            confirmed: true
        })
        .is_err());
    let plan = preview(
        &runtime,
        vec![id.clone()],
        &target,
        TransferAction::Move,
        vec![],
        vec![],
    );
    let result = execute_moves_with_persistence(
        &f.db,
        runtime
            .take_transfer(TransferExecuteRequest {
                preview_id: plan.id,
                confirmed: true,
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result.logs[0].status, "success");
    assert!(!f.source.exists());
    let restored =
        restore_moves_with_persistence(&f.db, RestoreMovesRequest { logs: result.logs }).unwrap();
    assert_eq!(restored.logs[0].restore_status, "restored");
    assert_eq!(fs::read(&f.source).unwrap(), b"owned fixture");
    fs::rename(&f.source, f.root.join("retired-source")).unwrap();
    fs::write(&f.source, b"replacement").unwrap();
    let stale = preview(
        &runtime,
        vec![id],
        &target,
        TransferAction::Copy,
        vec![],
        vec![],
    );
    assert_eq!(
        stale.rows[0].error.as_deref(),
        Some("transfer_source_changed")
    );
    runtime.dispose();
}

#[test]
fn u1_favorites_survive_runtime_recreation_and_unavailable_locations_are_hints_only() {
    use crate::file_workspace::integration::places::PlacesRequest;
    let f = Fixture::new();
    let runtime = runtime(&f);
    let folder = f.root.join("favorite");
    fs::create_dir(&folder).unwrap();
    let admitted = runtime
        .open_browse(BrowseOpenRequest {
            platform: if cfg!(windows) {
                WorkspacePlatform::Windows
            } else {
                WorkspacePlatform::Macos
            },
            routing_hint: normalize_path(&folder),
            display_hint: None,
            global_entry_id: None,
        })
        .unwrap();
    let saved = runtime
        .places(PlacesRequest {
            session_id: Some(admitted.session_id),
            path_ref: Some(admitted.root_path_ref),
            favorite: Some(true),
            remove_id: None,
        })
        .unwrap();
    assert!(saved[0].favorite && saved[0].current && !saved[0].unavailable);
    runtime.dispose();
    drop(runtime);
    fs::rename(&folder, f.root.join("retired-favorite")).unwrap();
    let next = FileWorkspaceRuntime::new(
        f.db.clone(),
        MacThumbnailService::new(f.root.join("legacy-next")),
        f.root.join("cache-next"),
    )
    .unwrap();
    let saved = next
        .places(PlacesRequest {
            session_id: None,
            path_ref: None,
            favorite: None,
            remove_id: None,
        })
        .unwrap();
    assert!(saved[0].favorite && saved[0].unavailable);
    assert!(next
        .open_browse(BrowseOpenRequest {
            platform: if cfg!(windows) {
                WorkspacePlatform::Windows
            } else {
                WorkspacePlatform::Macos
            },
            routing_hint: saved[0].routing_hint.clone(),
            display_hint: None,
            global_entry_id: None
        })
        .is_err());
    next.dispose();
}

#[test]
fn u1_partial_batch_cancellation_preserves_unprocessed_sources_and_durable_results() {
    struct CancelAtTen(Arc<AtomicBool>);
    impl OperationProgressEmitter for CancelAtTen {
        fn emit_progress(&self, payload: OperationProgressPayload) {
            if payload.processed >= 10 {
                self.0.store(true, Ordering::Relaxed);
            }
        }
    }
    let f = Fixture::new();
    let runtime = runtime(&f);
    for number in 0..10 {
        fs::write(
            f.source
                .parent()
                .unwrap()
                .join(format!("batch-{number}.txt")),
            b"batch owned bytes",
        )
        .unwrap();
    }
    let admitted = runtime
        .open_browse(BrowseOpenRequest {
            platform: if cfg!(windows) {
                WorkspacePlatform::Windows
            } else {
                WorkspacePlatform::Macos
            },
            routing_hint: normalize_path(f.source.parent().unwrap()),
            display_hint: None,
            global_entry_id: None,
        })
        .unwrap();
    let page = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: admitted.session_id,
            request_id: "batch".into(),
            path_ref: admitted.root_path_ref,
            page_size: 32,
            query: Default::default(),
        })
        .unwrap();
    let refs = page
        .entries
        .into_iter()
        .map(|entry| match entry.entry_ref {
            crate::file_workspace::BrowseEntryRef::Ephemeral {
                browse_session_id,
                entry_id,
            } => EntryRef::Ephemeral {
                browse_session_id,
                entry_id,
            },
        })
        .collect();
    let tickets = runtime
        .capture_transfer(CaptureRequest {
            refs,
            selection: None,
            release_ids: vec![],
            external_paths: vec![],
        })
        .unwrap();
    assert_eq!(tickets.len(), 11);
    let target = f.root.join("batch-target");
    fs::create_dir(&target).unwrap();
    let plan = preview(
        &runtime,
        tickets.iter().map(|t| t.id.clone()).collect(),
        &target,
        TransferAction::Copy,
        vec![],
        vec![],
    );
    let request = runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: plan.id,
            confirmed: true,
        })
        .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let result = execute_moves_with_persistence_with_progress(
        &f.db,
        request,
        Arc::clone(&cancel),
        &CancelAtTen(cancel),
    )
    .unwrap();
    assert_eq!(
        result
            .logs
            .iter()
            .filter(|log| log.status == "success")
            .count(),
        10
    );
    assert_eq!(
        result
            .logs
            .iter()
            .filter(|log| log.status == "skipped")
            .count(),
        1
    );
    assert_eq!(fs::read_dir(&target).unwrap().count(), 10);
    assert!(tickets
        .iter()
        .all(|ticket| Path::new(&ticket.path).exists()));
    assert_eq!(f.db.get_operation_logs(Some(20)).unwrap().len(), 11);
    runtime.dispose();
}

#[cfg(windows)]
#[test]
fn u1_locked_source_never_publishes_a_destination() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    let runtime = runtime(&f);
    let ticket = capture(&runtime, &f).remove(0);
    let target = f.root.join("locked-target");
    fs::create_dir(&target).unwrap();
    let plan = preview(
        &runtime,
        vec![ticket.id],
        &target,
        TransferAction::Copy,
        vec![],
        vec![],
    );
    let request = runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: plan.id,
            confirmed: true,
        })
        .unwrap();
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&f.source)
        .unwrap();
    let result = execute_moves_with_persistence(&f.db, request);
    assert!(
        result.is_err()
            || result
                .unwrap()
                .logs
                .iter()
                .all(|log| log.status != "success")
    );
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    drop(locked);
    assert_eq!(fs::read(&f.source).unwrap(), b"owned fixture");
    runtime.dispose();
}

#[cfg(target_os = "macos")]
#[test]
fn u1_macos_directory_copy_move_and_restore_reuse_the_existing_tree_transaction() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let parent = f.root.join("folders");
    let source = parent.join("tree");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("nested/content.txt"), b"owned directory bytes").unwrap();
    let admitted = runtime
        .open_browse(BrowseOpenRequest {
            platform: WorkspacePlatform::Macos,
            routing_hint: normalize_path(&parent),
            display_hint: None,
            global_entry_id: None,
        })
        .unwrap();
    let page = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: admitted.session_id,
            request_id: "folder".into(),
            path_ref: admitted.root_path_ref,
            page_size: 32,
            query: Default::default(),
        })
        .unwrap();
    let reference = match page
        .entries
        .into_iter()
        .find(|entry| entry.name == "tree")
        .unwrap()
        .entry_ref
    {
        crate::file_workspace::BrowseEntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        } => EntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        },
    };
    let ticket = runtime
        .capture_transfer(CaptureRequest {
            refs: vec![reference],
            selection: None,
            release_ids: vec![],
            external_paths: vec![],
        })
        .unwrap()
        .remove(0);
    for action in [TransferAction::Copy, TransferAction::Move] {
        let target = f.root.join(if action == TransferAction::Copy {
            "folder-copy"
        } else {
            "folder-move"
        });
        fs::create_dir(&target).unwrap();
        let plan = preview(
            &runtime,
            vec![ticket.id.clone()],
            &target,
            action,
            vec![],
            vec![],
        );
        assert!(plan.rows[0].error.is_none(), "{:?}", plan.rows[0].error);
        let executed = execute_moves_with_persistence(
            &f.db,
            runtime
                .take_transfer(TransferExecuteRequest {
                    preview_id: plan.id,
                    confirmed: true,
                })
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            executed.logs[0].status, "success",
            "{:?}",
            executed.logs[0].error_message
        );
        assert_eq!(
            fs::read(target.join("tree/nested/content.txt")).unwrap(),
            b"owned directory bytes"
        );
        if action == TransferAction::Move {
            assert!(!source.exists());
            let restored = restore_moves_with_persistence(
                &f.db,
                RestoreMovesRequest {
                    logs: executed.logs,
                },
            )
            .unwrap();
            assert_eq!(
                restored.logs[0].restore_status, "restored",
                "{:?}",
                restored.logs[0].restore_error
            );
        }
        assert_eq!(
            fs::read(source.join("nested/content.txt")).unwrap(),
            b"owned directory bytes"
        );
    }
    runtime.dispose();
}

#[test]
fn u1_skipped_same_name_does_not_reserve_a_batch_destination() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let other = f.root.join("other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("original.txt"), b"other source").unwrap();
    let browse = runtime
        .open_browse(BrowseOpenRequest {
            platform: if cfg!(windows) {
                WorkspacePlatform::Windows
            } else {
                WorkspacePlatform::Macos
            },
            routing_hint: normalize_path(&other),
            display_hint: None,
            global_entry_id: None,
        })
        .unwrap();
    let page = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: browse.session_id,
            request_id: "same-name".into(),
            path_ref: browse.root_path_ref,
            page_size: 32,
            query: Default::default(),
        })
        .unwrap();
    let reference = match page.entries[0].entry_ref.clone() {
        crate::file_workspace::BrowseEntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        } => EntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        },
    };
    let mut tickets = capture(&runtime, &f);
    tickets.extend(
        runtime
            .capture_transfer(CaptureRequest {
                refs: vec![reference],
                selection: None,
                release_ids: vec![],
                external_paths: vec![],
            })
            .unwrap(),
    );
    let target = f.root.join("target");
    fs::create_dir(&target).unwrap();
    let ids = tickets
        .iter()
        .map(|ticket| ticket.id.clone())
        .collect::<Vec<_>>();
    let conflict = preview(
        &runtime,
        ids.clone(),
        &target,
        TransferAction::Copy,
        vec![],
        vec![],
    );
    assert_eq!(
        conflict.rows[1].error.as_deref(),
        Some("transfer_target_exists")
    );
    let plan = preview(
        &runtime,
        ids,
        &target,
        TransferAction::Copy,
        vec![],
        vec![tickets[0].id.clone()],
    );
    assert!(plan.rows[0].skipped);
    assert!(plan.rows[1].error.is_none());
    assert_eq!(
        plan.rows[1].preview.as_ref().unwrap().new_name,
        "original.txt"
    );
    let execution = runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: plan.id,
            confirmed: true,
        })
        .unwrap();
    assert_eq!(execution.operations.len(), 1);
    assert_eq!(
        execution.operations[0].source_path,
        normalize_path(&other.join("original.txt").canonicalize().unwrap())
    );
    assert!(f.source.exists());
    assert!(!target.join("original.txt").exists());
}

#[test]
fn fl20_external_directory_copy_preserves_nested_paths_and_rejects_stale_tree_preview() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let source = f.root.join("drop-source");
    fs::create_dir_all(source.join("nested/deeper")).unwrap();
    fs::write(source.join("root.txt"), b"root bytes").unwrap();
    fs::write(source.join("nested/deeper/child.txt"), b"nested bytes").unwrap();
    let tickets = runtime
        .capture_transfer(CaptureRequest {
            refs: vec![],
            selection: None,
            release_ids: vec![],
            external_paths: vec![normalize_path(&source)],
        })
        .unwrap();
    assert_eq!(tickets.len(), 1);
    assert!(tickets[0].is_directory);
    assert_eq!(tickets[0].relative_path, "drop-source");

    let destination = f.root.join("destination");
    fs::create_dir(&destination).unwrap();
    let plan = preview(
        &runtime,
        tickets.iter().map(|ticket| ticket.id.clone()).collect(),
        &destination,
        TransferAction::Copy,
        vec![],
        vec![],
    );
    assert!(
        plan.rows.iter().all(|row| row.error.is_none()),
        "{:?}",
        plan.rows
            .iter()
            .map(|row| row.error.as_deref())
            .collect::<Vec<_>>()
    );
    assert!(plan.rows[0]
        .preview
        .as_ref()
        .is_some_and(|preview| Path::new(&preview.target_path).ends_with("drop-source")));

    let stale = source.join("late.txt");
    fs::write(&stale, b"arrived after preview").unwrap();
    let stale_plan = runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: plan.id,
            confirmed: true,
        })
        .unwrap_err();
    assert_eq!(stale_plan, "transfer_source_tree_changed");
    assert!(!destination.join("drop-source").exists());
    fs::remove_file(&stale).unwrap();

    let fresh = runtime
        .capture_transfer(CaptureRequest {
            refs: vec![],
            selection: None,
            release_ids: vec![],
            external_paths: vec![normalize_path(&source)],
        })
        .unwrap();
    let plan = preview(
        &runtime,
        fresh.iter().map(|ticket| ticket.id.clone()).collect(),
        &destination,
        TransferAction::Copy,
        vec![],
        vec![],
    );
    let request = runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: plan.id,
            confirmed: true,
        })
        .unwrap();
    let result = execute_moves_with_persistence(&f.db, request).unwrap();
    assert_eq!(result.logs.len(), 1);
    assert!(
        result
            .logs
            .iter()
            .all(|log| log.status == "success" && !log.can_restore),
        "unexpected external copy journal state: {:?}",
        result.logs
    );
    assert_eq!(
        fs::read(destination.join("drop-source/root.txt")).unwrap(),
        b"root bytes"
    );
    assert_eq!(
        fs::read(destination.join("drop-source/nested/deeper/child.txt")).unwrap(),
        b"nested bytes"
    );
    assert!(source.join("root.txt").exists());
    runtime.dispose();
}

#[cfg(windows)]
#[test]
fn fl20_cancelled_directory_copy_rolls_back_source_and_cleans_partial_stage_with_journal() {
    use crate::fs_safety::atomic_move::test_faults::{self, AtomicFaultPoint};

    let f = Fixture::new();
    let runtime = runtime(&f);
    let source = f.root.join("cancelled-folder");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("a.txt"),
        b"first file copied before deterministic cancellation",
    )
    .unwrap();
    fs::write(source.join("b.txt"), b"second file must remain uncopied").unwrap();
    let ticket = runtime
        .capture_transfer(CaptureRequest {
            refs: vec![],
            selection: None,
            release_ids: vec![],
            external_paths: vec![normalize_path(&source)],
        })
        .unwrap()
        .remove(0);
    let destination = f.root.join("cancelled-folder-target");
    fs::create_dir(&destination).unwrap();
    let plan = preview(
        &runtime,
        vec![ticket.id],
        &destination,
        TransferAction::Copy,
        vec![],
        vec![],
    );
    let request = runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: plan.id,
            confirmed: true,
        })
        .unwrap();

    test_faults::set_fault(Some(AtomicFaultPoint::CancelDirectoryCopyAfterEntry));
    let result = execute_moves_with_persistence(&f.db, request).unwrap();
    test_faults::set_fault(None);

    assert_eq!(result.logs.len(), 1);
    assert_eq!(result.logs[0].status, "skipped");
    assert_eq!(result.logs[0].operation_phase, "copying");
    assert!(source.join("a.txt").is_file());
    assert!(source.join("b.txt").is_file());
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
    assert!(fs::read_dir(f.root.as_path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".zen-canvas-claim-")));
    let history = f.db.get_operation_logs(Some(10)).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, "skipped");
    assert_eq!(history[0].operation_phase, "copying");
    assert_eq!(
        crate::path_identity::normalize_for_compare(Path::new(&history[0].source_path)),
        crate::path_identity::normalize_for_compare(&source)
    );
    assert_eq!(
        crate::path_identity::normalize_for_compare(Path::new(&history[0].target_path)),
        crate::path_identity::normalize_for_compare(&destination.join("cancelled-folder"))
    );
    runtime.dispose();
}

#[test]
fn fl20_external_directory_conflict_requires_skip_or_keep_both_without_overwrite() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let source = f.root.join("folder-conflict");
    fs::create_dir_all(source.join("nested/empty")).unwrap();
    fs::write(source.join("nested/item.txt"), b"incoming tree").unwrap();
    let ticket = runtime
        .capture_transfer(CaptureRequest {
            refs: vec![],
            selection: None,
            release_ids: vec![],
            external_paths: vec![normalize_path(&source)],
        })
        .unwrap()
        .remove(0);
    let destination = f.root.join("folder-conflict-target");
    fs::create_dir(&destination).unwrap();
    fs::create_dir(destination.join("folder-conflict")).unwrap();
    fs::write(
        destination.join("folder-conflict/existing.txt"),
        b"keep existing",
    )
    .unwrap();

    let conflict = preview(
        &runtime,
        vec![ticket.id.clone()],
        &destination,
        TransferAction::Copy,
        vec![],
        vec![],
    );
    assert_eq!(
        conflict.rows[0].error.as_deref(),
        Some("transfer_target_exists")
    );
    let skipped = preview(
        &runtime,
        vec![ticket.id.clone()],
        &destination,
        TransferAction::Copy,
        vec![],
        vec![ticket.id.clone()],
    );
    assert!(skipped.rows[0].skipped);
    assert!(skipped.rows[0].error.is_none());
    assert!(skipped.rows[0].preview.is_none());
    assert_eq!(
        fs::read(destination.join("folder-conflict/existing.txt")).unwrap(),
        b"keep existing"
    );

    let keep_both = preview(
        &runtime,
        vec![ticket.id.clone()],
        &destination,
        TransferAction::Copy,
        vec![ticket.id],
        vec![],
    );
    assert!(
        keep_both.rows[0].error.is_none(),
        "{:?}",
        keep_both.rows[0].error
    );
    assert!(keep_both.rows[0]
        .preview
        .as_ref()
        .is_some_and(|operation| operation.new_name == "folder-conflict (2)"));
    let result = execute_moves_with_persistence(
        &f.db,
        runtime
            .take_transfer(TransferExecuteRequest {
                preview_id: keep_both.id,
                confirmed: true,
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        result.logs[0].status, "success",
        "{:?}",
        result.logs[0].error_message
    );
    assert_eq!(
        fs::read(destination.join("folder-conflict/existing.txt")).unwrap(),
        b"keep existing"
    );
    assert_eq!(
        fs::read(destination.join("folder-conflict (2)/nested/item.txt")).unwrap(),
        b"incoming tree"
    );
    assert!(destination
        .join("folder-conflict (2)/nested/empty")
        .is_dir());
    assert!(source.join("nested/item.txt").is_file());
    runtime.dispose();
}

#[test]
fn fl20_external_directory_copy_preserves_empty_directories_and_empty_roots() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let source = f.root.join("empty-source");
    fs::create_dir_all(source.join("nested/deeper")).unwrap();
    fs::write(
        source.join("nested/content.txt"),
        b"content beside empty branch",
    )
    .unwrap();
    let tickets = runtime
        .capture_transfer(CaptureRequest {
            refs: vec![],
            selection: None,
            release_ids: vec![],
            external_paths: vec![normalize_path(&source)],
        })
        .unwrap();
    assert_eq!(tickets.len(), 1);
    let destination = f.root.join("empty-destination");
    fs::create_dir(&destination).unwrap();
    let plan = preview(
        &runtime,
        vec![tickets[0].id.clone()],
        &destination,
        TransferAction::Copy,
        vec![],
        vec![],
    );
    assert!(plan.rows[0].error.is_none(), "{:?}", plan.rows[0].error);
    let result = execute_moves_with_persistence(
        &f.db,
        runtime
            .take_transfer(TransferExecuteRequest {
                preview_id: plan.id,
                confirmed: true,
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        result.logs[0].status, "success",
        "{:?}",
        result.logs[0].error_message
    );
    assert!(destination.join("empty-source/nested/deeper").is_dir());
    assert_eq!(
        fs::read(destination.join("empty-source/nested/content.txt")).unwrap(),
        b"content beside empty branch"
    );
    assert!(source.join("nested/deeper").is_dir());

    let empty = f.root.join("completely-empty");
    fs::create_dir(&empty).unwrap();
    let empty_ticket = runtime
        .capture_transfer(CaptureRequest {
            refs: vec![],
            selection: None,
            release_ids: vec![],
            external_paths: vec![normalize_path(&empty)],
        })
        .unwrap()
        .remove(0);
    let empty_plan = preview(
        &runtime,
        vec![empty_ticket.id],
        &destination,
        TransferAction::Copy,
        vec![],
        vec![],
    );
    assert!(
        empty_plan.rows[0].error.is_none(),
        "{:?}",
        empty_plan.rows[0].error
    );
    let empty_result = execute_moves_with_persistence(
        &f.db,
        runtime
            .take_transfer(TransferExecuteRequest {
                preview_id: empty_plan.id,
                confirmed: true,
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        empty_result.logs[0].status, "success",
        "{:?}",
        empty_result.logs[0].error_message
    );
    assert!(destination.join("completely-empty").is_dir());
    assert_eq!(
        fs::read_dir(destination.join("completely-empty"))
            .unwrap()
            .count(),
        0
    );
    assert!(empty.is_dir());
    runtime.dispose();
}

#[cfg(windows)]
#[test]
fn fl20_windows_external_directory_move_is_explicitly_rejected_without_mutation() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let source = f.root.join("move-source");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("nested/content.txt"), b"source remains").unwrap();
    let ticket = runtime
        .capture_transfer(CaptureRequest {
            refs: vec![],
            selection: None,
            release_ids: vec![],
            external_paths: vec![normalize_path(&source)],
        })
        .unwrap()
        .remove(0);
    let destination = f.root.join("move-target");
    fs::create_dir(&destination).unwrap();
    let plan = preview(
        &runtime,
        vec![ticket.id],
        &destination,
        TransferAction::Move,
        vec![],
        vec![],
    );
    assert_eq!(
        plan.rows[0].error.as_deref(),
        Some("transfer_windows_directory_move_unsupported")
    );
    assert!(source.join("nested/content.txt").is_file());
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
    runtime.dispose();
}

#[test]
fn fl20_external_transfer_rejects_a_replaced_destination_root() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let source = f.root.join("external-root-identity.txt");
    fs::write(&source, b"destination identity guard").unwrap();
    let tickets = runtime
        .capture_transfer(CaptureRequest {
            refs: vec![],
            selection: None,
            release_ids: vec![],
            external_paths: vec![normalize_path(&source)],
        })
        .unwrap();
    let destination = f.root.join("replaceable-destination");
    fs::create_dir(&destination).unwrap();
    let plan = preview(
        &runtime,
        vec![tickets[0].id.clone()],
        &destination,
        TransferAction::Copy,
        vec![],
        vec![],
    );
    let original = f.root.join("replaceable-destination-original");
    let replacement = f.root.join("replacement-destination");
    fs::create_dir(&replacement).unwrap();
    fs::rename(&destination, &original).unwrap();
    fs::rename(&replacement, &destination).unwrap();

    let error = runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: plan.id,
            confirmed: true,
        })
        .unwrap_err();
    assert_eq!(error, "operation_preview_stale");
    assert_eq!(fs::read(&source).unwrap(), b"destination identity guard");
    assert!(!destination.join("external-root-identity.txt").exists());
    runtime.dispose();
}

#[test]
fn fl20_external_file_move_is_previewed_journaled_and_restored_without_indexing() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let source = f.root.join("external-move.txt");
    fs::write(&source, b"external source bytes").unwrap();
    let tickets = runtime
        .capture_transfer(CaptureRequest {
            refs: vec![],
            selection: None,
            release_ids: vec![],
            external_paths: vec![normalize_path(&source)],
        })
        .unwrap();
    let destination = f.root.join("destination");
    fs::create_dir(&destination).unwrap();
    let plan = preview(
        &runtime,
        vec![tickets[0].id.clone()],
        &destination,
        TransferAction::Move,
        vec![],
        vec![],
    );
    let request = runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: plan.id,
            confirmed: true,
        })
        .unwrap();
    let result = execute_moves_with_persistence(&f.db, request).unwrap();
    assert_eq!(result.logs[0].status, "success");
    assert!(result.logs[0].can_restore);
    assert!(!source.exists());
    assert_eq!(
        fs::read(destination.join("external-move.txt")).unwrap(),
        b"external source bytes"
    );
    let restore =
        restore_moves_with_persistence(&f.db, RestoreMovesRequest { logs: result.logs }).unwrap();
    assert_eq!(restore.logs[0].restore_status, "restored");
    assert_eq!(fs::read(&source).unwrap(), b"external source bytes");
    assert!(!destination.join("external-move.txt").exists());
    runtime.dispose();
}

#[test]
fn fl20_releasing_external_candidates_invalidates_unconfirmed_previews() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let source = f.root.join("external-cancel.txt");
    fs::write(&source, b"must remain unchanged").unwrap();
    let tickets = runtime
        .capture_transfer(CaptureRequest {
            refs: vec![],
            selection: None,
            release_ids: vec![],
            external_paths: vec![normalize_path(&source)],
        })
        .unwrap();
    let destination = f.root.join("cancel-destination");
    fs::create_dir(&destination).unwrap();
    let plan = preview(
        &runtime,
        vec![tickets[0].id.clone()],
        &destination,
        TransferAction::Copy,
        vec![],
        vec![],
    );
    runtime
        .capture_transfer(CaptureRequest {
            refs: vec![],
            selection: None,
            release_ids: vec![tickets[0].id.clone()],
            external_paths: vec![],
        })
        .unwrap();

    let error = runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: plan.id,
            confirmed: true,
        })
        .unwrap_err();
    assert_eq!(error, "operation_preview_stale");
    assert_eq!(fs::read(&source).unwrap(), b"must remain unchanged");
    assert!(!destination.join("external-cancel.txt").exists());
    runtime.dispose();
}

#[test]
fn fl20_releasing_a_repeated_external_capture_preserves_the_basket_ticket() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let source = f.root.join("shared-source.txt");
    fs::write(&source, b"basket owner survives external cancellation").unwrap();
    let capture = || {
        runtime
            .capture_transfer(CaptureRequest {
                refs: vec![],
                selection: None,
                release_ids: vec![],
                external_paths: vec![normalize_path(&source)],
            })
            .unwrap()
            .remove(0)
    };

    let basket_ticket = capture();
    let cancelled_external_ticket = capture();
    assert_ne!(basket_ticket.id, cancelled_external_ticket.id);
    runtime
        .capture_transfer(CaptureRequest {
            refs: vec![],
            selection: None,
            release_ids: vec![cancelled_external_ticket.id],
            external_paths: vec![],
        })
        .unwrap();

    let destination = f.root.join("basket-destination");
    fs::create_dir(&destination).unwrap();
    let plan = preview(
        &runtime,
        vec![basket_ticket.id],
        &destination,
        TransferAction::Copy,
        vec![],
        vec![],
    );
    assert!(plan.rows[0].error.is_none(), "{:?}", plan.rows[0].error);
    let request = runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: plan.id,
            confirmed: true,
        })
        .unwrap();
    let result = execute_moves_with_persistence(&f.db, request).unwrap();
    assert_eq!(result.logs[0].status, "success");
    assert_eq!(
        fs::read(&source).unwrap(),
        b"basket owner survives external cancellation"
    );
    assert_eq!(
        fs::read(destination.join("shared-source.txt")).unwrap(),
        b"basket owner survives external cancellation"
    );
    runtime.dispose();
}

#[test]
fn fl20_browse_folder_row_is_a_current_backend_resolved_destination() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let source = f.root.join("external-source.txt");
    fs::write(&source, b"external bytes").unwrap();
    let tickets = runtime
        .capture_transfer(CaptureRequest {
            refs: vec![],
            selection: None,
            release_ids: vec![],
            external_paths: vec![normalize_path(&source)],
        })
        .unwrap();
    let destination = f.root.join("drop-target");
    fs::create_dir(&destination).unwrap();
    let browse = runtime
        .open_browse(BrowseOpenRequest {
            platform: if cfg!(windows) {
                WorkspacePlatform::Windows
            } else {
                WorkspacePlatform::Macos
            },
            routing_hint: normalize_path(&f.root),
            display_hint: None,
            global_entry_id: None,
        })
        .unwrap();
    let page = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: browse.session_id.clone(),
            request_id: "external-destination".into(),
            path_ref: browse.root_path_ref.clone(),
            page_size: 64,
            query: Default::default(),
        })
        .unwrap();
    let row = page
        .entries
        .iter()
        .find(|entry| entry.name == "drop-target")
        .unwrap();
    assert_eq!(row.kind, BrowseEntryKindDto::Directory);
    let plan = runtime
        .preview_transfer(TransferPreviewRequest {
            ticket_ids: vec![tickets[0].id.clone()],
            action: TransferAction::Copy,
            destination_directory: String::new(),
            destination_entry: Some(row.entry_ref.clone()),
            new_name: None,
            keep_both: vec![],
            skip: vec![],
        })
        .unwrap();
    assert_eq!(
        plan.destination_directory,
        normalize_path(&destination.canonicalize().unwrap())
    );

    runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: browse.session_id.clone(),
            request_id: "external-destination-refresh".into(),
            path_ref: browse.root_path_ref,
            page_size: 64,
            query: Default::default(),
        })
        .unwrap();
    let stale = runtime.preview_transfer(TransferPreviewRequest {
        ticket_ids: vec![tickets[0].id.clone()],
        action: TransferAction::Copy,
        destination_directory: String::new(),
        destination_entry: Some(row.entry_ref.clone()),
        new_name: None,
        keep_both: vec![],
        skip: vec![],
    });
    assert!(stale.is_err());
    runtime.dispose();
}

fn preview_named(
    runtime: &FileWorkspaceRuntime,
    ids: Vec<String>,
    target: &Path,
    action: TransferAction,
    new_name: Option<&str>,
) -> TransferPreview {
    runtime
        .preview_transfer(TransferPreviewRequest {
            ticket_ids: ids,
            action,
            destination_directory: normalize_path(target),
            destination_entry: None,
            new_name: new_name.map(str::to_string),
            keep_both: vec![],
            skip: vec![],
        })
        .unwrap()
}

fn capture_named(runtime: &FileWorkspaceRuntime, directory: &Path, name: &str) -> TransferTicket {
    let browse = runtime
        .open_browse(BrowseOpenRequest {
            platform: if cfg!(windows) {
                WorkspacePlatform::Windows
            } else {
                WorkspacePlatform::Macos
            },
            routing_hint: normalize_path(directory),
            display_hint: None,
            global_entry_id: None,
        })
        .unwrap();
    let page = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: browse.session_id.clone(),
            request_id: format!("capture-{name}"),
            path_ref: browse.root_path_ref,
            page_size: 64,
            query: Default::default(),
        })
        .unwrap();
    let reference = match page
        .entries
        .into_iter()
        .find(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("fixture entry {name:?} was not enumerated"))
        .entry_ref
    {
        crate::file_workspace::BrowseEntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        } => EntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        },
    };
    let ticket = runtime
        .capture_transfer(CaptureRequest {
            refs: vec![reference],
            selection: None,
            release_ids: vec![],
            external_paths: vec![],
        })
        .unwrap()
        .remove(0);
    runtime
        .dispose_browse(BrowseSessionRequest {
            session_id: browse.session_id,
        })
        .unwrap();
    ticket
}

fn fixture_file_hash(path: &Path) -> blake3::Hash {
    blake3::hash(&fs::read(path).unwrap())
}

#[test]
fn fl24_create_folder_uses_preview_confirm_and_non_restorable_journal() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let folder = f.root.join("created-folder");
    let request = |name: &str| TransferPreviewRequest {
        ticket_ids: vec![],
        action: TransferAction::CreateDirectory,
        destination_directory: normalize_path(&f.root),
        destination_entry: None,
        new_name: Some(name.into()),
        keep_both: vec![],
        skip: vec![],
    };
    let plan = runtime.preview_transfer(request("created-folder")).unwrap();
    assert_eq!(plan.rows.len(), 1);
    assert_eq!(
        plan.rows[0]
            .preview
            .as_ref()
            .map(|preview| preview.operation_type.as_str()),
        Some("create_directory")
    );
    assert!(!folder.exists(), "Preview must not create the folder");

    let execution = runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: plan.id,
            confirmed: true,
        })
        .unwrap();
    let result = execute_moves_with_persistence(&f.db, execution).unwrap();
    assert_eq!(result.logs.len(), 1);
    assert_eq!(result.logs[0].operation_type, "create_directory");
    assert_eq!(result.logs[0].status, "success");
    assert!(!result.logs[0].can_restore);
    assert!(folder.is_dir());
    let journal = f.db.get_operation_logs(Some(10)).unwrap();
    assert_eq!(journal.len(), 1);
    assert_eq!(journal[0].operation_type, "create_directory");
    assert_eq!(journal[0].operation_phase, "completed");
    assert!(!journal[0].can_restore);

    // An already present destination is rejected before a preview is issued.
    let conflict = runtime.preview_transfer(request("created-folder")).unwrap();
    assert_eq!(
        conflict.rows[0].error.as_deref(),
        Some("transfer_target_exists")
    );

    // A destination created after the preview cannot be replaced on confirm.
    let late_folder = f.root.join("late-conflict");
    let late_plan = runtime.preview_transfer(request("late-conflict")).unwrap();
    fs::create_dir(&late_folder).unwrap();
    assert!(runtime
        .take_transfer(TransferExecuteRequest {
            preview_id: late_plan.id,
            confirmed: true,
        })
        .is_err());
    assert!(late_folder.is_dir());
    assert_eq!(fs::read_dir(&late_folder).unwrap().count(), 0);
    assert_eq!(f.db.get_operation_logs(Some(10)).unwrap().len(), 1);
    runtime.dispose();
}

#[test]
fn fl24_folder_rename_conflict_and_restore_preserve_fixture_bytes() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let source = f.root.join("source-folder");
    let payload = source.join("nested").join("payload.bin");
    fs::create_dir_all(payload.parent().unwrap()).unwrap();
    fs::write(&payload, b"directory fixture bytes").unwrap();
    let original_hash = fixture_file_hash(&payload);
    let rename_ticket = capture_named(&runtime, &f.root, "source-folder");

    let conflicting_target = f.root.join("renamed-folder");
    fs::create_dir(&conflicting_target).unwrap();
    let conflict = preview_named(
        &runtime,
        vec![rename_ticket.id.clone()],
        &f.root,
        TransferAction::Rename,
        Some("renamed-folder"),
    );
    assert_eq!(
        conflict.rows[0].error.as_deref(),
        Some("transfer_target_exists")
    );
    fs::remove_dir(&conflicting_target).unwrap();

    let rename_plan = preview_named(
        &runtime,
        vec![rename_ticket.id.clone()],
        &f.root,
        TransferAction::Rename,
        Some("renamed-folder"),
    );
    assert!(
        rename_plan.rows[0].error.is_none(),
        "unexpected rename preview error: {:?}",
        rename_plan.rows[0].error
    );
    assert_eq!(
        rename_plan.rows[0].preview.as_ref().unwrap().operation_type,
        "rename"
    );
    let renamed = f.root.join("renamed-folder");
    let renamed_result = execute_moves_with_persistence(
        &f.db,
        runtime
            .take_transfer(TransferExecuteRequest {
                preview_id: rename_plan.id,
                confirmed: true,
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        renamed_result.logs[0].status, "success",
        "rename failed: {:?}",
        renamed_result.logs[0].error_message
    );
    assert!(renamed_result.logs[0].can_restore);
    let renamed_payload = renamed.join("nested").join("payload.bin");
    assert_eq!(fixture_file_hash(&renamed_payload), original_hash);
    let restored_rename = restore_moves_with_persistence(
        &f.db,
        RestoreMovesRequest {
            logs: renamed_result.logs,
        },
    )
    .unwrap();
    assert_eq!(restored_rename.logs[0].restore_status, "restored");
    assert_eq!(fixture_file_hash(&payload), original_hash);
    runtime.dispose();
}

#[test]
fn fl24_stale_folder_identity_cannot_be_renamed_after_external_path_change() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let folder = f.root.join("stale-folder");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("content.txt"), b"unchanged fixture").unwrap();
    let ticket = capture_named(&runtime, &f.root, "stale-folder");
    fs::rename(&folder, f.root.join("moved-folder")).unwrap();

    let stale = preview_named(
        &runtime,
        vec![ticket.id],
        &f.root,
        TransferAction::Rename,
        Some("replacement-folder"),
    );
    assert!(matches!(
        stale.rows[0].error.as_deref(),
        Some("transfer_source_changed" | "source_missing")
    ));
    assert!(!f.root.join("replacement-folder").exists());
    runtime.dispose();
}

#[test]
fn fl24_folder_safe_trash_restores_from_journal_and_preserves_hash() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let folder = f.root.join("trash-folder");
    let payload = folder.join("nested").join("content.bin");
    fs::create_dir_all(payload.parent().unwrap()).unwrap();
    fs::write(&payload, b"trash restore fixture").unwrap();
    let expected_hash = fixture_file_hash(&payload);
    let ticket = capture_named(&runtime, &f.root, "trash-folder");
    let plan = preview_named(
        &runtime,
        vec![ticket.id],
        &f.root,
        TransferAction::Trash,
        None,
    );
    assert!(plan.rows[0].error.is_none(), "{:?}", plan.rows[0].error);
    let result = execute_moves_with_persistence(
        &f.db,
        runtime
            .take_transfer(TransferExecuteRequest {
                preview_id: plan.id,
                confirmed: true,
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        result.logs[0].status, "success",
        "Safe Trash failed: {:?}",
        result.logs[0].error_message
    );
    assert!(!folder.exists());
    let trashed = Path::new(&result.logs[0].path_after)
        .join("nested")
        .join("content.bin");
    assert_eq!(fixture_file_hash(&trashed), expected_hash);
    assert!(result.logs[0].can_restore);
    let restored =
        restore_moves_with_persistence(&f.db, RestoreMovesRequest { logs: result.logs }).unwrap();
    assert_eq!(restored.logs[0].restore_status, "restored");
    assert_eq!(fixture_file_hash(&payload), expected_hash);
    assert_eq!(fs::read(&payload).unwrap(), b"trash restore fixture");
    runtime.dispose();
}

#[test]
fn fl24_safe_trash_sources_and_destinations_are_reserved_for_history_restore() {
    let f = Fixture::new();
    let runtime = runtime(&f);
    let trash_root = f.root.join(".zen-canvas-trash");
    fs::create_dir_all(&trash_root).unwrap();
    let trashed_path = trash_root.join("protected-item.txt");
    fs::write(&trashed_path, b"journal-owned bytes").unwrap();
    let trashed_ticket = capture_named(&runtime, &trash_root, "protected-item.txt");

    for action in [
        TransferAction::Copy,
        TransferAction::Move,
        TransferAction::Trash,
    ] {
        let plan = preview(
            &runtime,
            vec![trashed_ticket.id.clone()],
            &f.root,
            action,
            vec![],
            vec![],
        );
        assert!(
            plan.rows[0]
                .error
                .as_deref()
                .is_some_and(|error| error.to_ascii_lowercase().contains("protected")),
            "{action:?} must reject a Safe Trash source during Preview: {:?}",
            plan.rows[0].error
        );
    }
    let rename = preview_named(
        &runtime,
        vec![trashed_ticket.id],
        &trash_root,
        TransferAction::Rename,
        Some("renamed-item.txt"),
    );
    assert!(
        rename.rows[0]
            .error
            .as_deref()
            .is_some_and(|error| error.to_ascii_lowercase().contains("protected")),
        "Safe Trash rename preview should fail with a protected-path reason: {:?}",
        rename.rows[0].error
    );
    let trash_folder_ticket = capture_named(&runtime, &f.root, ".zen-canvas-trash");
    let rename_trash_root = preview_named(
        &runtime,
        vec![trash_folder_ticket.id],
        &f.root,
        TransferAction::Rename,
        Some("renamed-trash"),
    );
    assert!(
        rename_trash_root.rows[0]
            .error
            .as_deref()
            .is_some_and(|error| error.to_ascii_lowercase().contains("protected")),
        "Safe Trash root rename preview should fail with a protected-path reason: {:?}",
        rename_trash_root.rows[0].error
    );
    assert!(trash_root.is_dir());
    assert_eq!(fs::read(&trashed_path).unwrap(), b"journal-owned bytes");

    let regular_ticket = capture(&runtime, &f).remove(0);
    let destination_preview = runtime
        .preview_transfer(TransferPreviewRequest {
            ticket_ids: vec![regular_ticket.id],
            action: TransferAction::Copy,
            destination_directory: normalize_path(&trash_root),
            destination_entry: None,
            new_name: None,
            keep_both: vec![],
            skip: vec![],
        })
        .unwrap();
    assert!(destination_preview.rows[0]
        .error
        .as_deref()
        .is_some_and(|error| error.to_ascii_lowercase().contains("protected")));

    let create_error = runtime
        .preview_transfer(TransferPreviewRequest {
            ticket_ids: vec![],
            action: TransferAction::CreateDirectory,
            destination_directory: normalize_path(&trash_root),
            destination_entry: None,
            new_name: Some("child".into()),
            keep_both: vec![],
            skip: vec![],
        })
        .err()
        .unwrap();
    assert!(create_error.to_ascii_lowercase().contains("protected"));
    assert_eq!(fs::read(&trashed_path).unwrap(), b"journal-owned bytes");
    runtime.dispose();
}

#[cfg(windows)]
#[test]
fn fl24_windows_extended_prefix_is_allowed_without_allowing_wildcards() {
    assert!(validate_cleanup_path_syntax(Path::new(r"\\?\D:\fixtures\ordinary-folder")).is_ok());
    assert!(validate_cleanup_path_syntax(Path::new(r"\\?\D:\fixtures\wild?card-folder")).is_err());
}
