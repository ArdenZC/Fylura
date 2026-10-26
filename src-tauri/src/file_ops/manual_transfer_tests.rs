use super::manual_rename_tests::Fixture;
use super::manual_transfer::*;
use super::*;
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
        vec![],
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
