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
    assert_eq!(tickets.len(), 2);
    assert!(tickets
        .iter()
        .any(|ticket| ticket.relative_path == "drop-source/root.txt"));
    assert!(tickets
        .iter()
        .any(|ticket| ticket.relative_path == "drop-source/nested/deeper/child.txt"));

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
    assert!(plan.rows.iter().any(|row| row
        .preview
        .as_ref()
        .is_some_and(|preview| Path::new(&preview.target_path)
            .ends_with("drop-source/nested/deeper/child.txt"))));

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
    assert_eq!(result.logs.len(), 2);
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
        keep_both: vec![],
        skip: vec![],
    });
    assert!(stale.is_err());
    runtime.dispose();
}
