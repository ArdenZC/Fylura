use super::*;
use crate::{
    db::scan::WatcherReconciliationAdmission, dedupe::DedupeJobManager, scanner::ScanJobManager,
};

fn wait_healthy(db: &Database, root_id: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let root = db.get_scan_root_health(Some(root_id), None).unwrap();
        if !root.needs_reconciliation && root.active_run_id.is_none() {
            assert_eq!(root.health_status, "healthy");
            assert_eq!(root.watcher_revision, root.watcher_applied_revision);
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "unsettled root: {root:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

// Included by the existing normal-PR issue375_ backend lane. No watcher is
// started: this deliberately covers missed events, with no restart/manual scan.
#[test]
fn issue375_m1_w2_move_restore_wakes_admissible_root_in_same_session() {
    for wake in [false, true] {
        let (f, library, uid) = super::restore_finalization_tests::managed_fixture();
        let root_id =
            f.db.managed_root_ids_for_operation_paths(&[library.clone()])
                .unwrap()[0]
                .clone();
        let app = tauri::test::mock_app();
        let jobs = ScanJobManager::default();
        let dedupe_jobs = DedupeJobManager::default();
        app.manage(jobs.clone());
        app.manage(dedupe_jobs.clone());
        let initial = crate::scanner::start_performance_managed_scan(
            app.handle().clone(),
            f.db.clone(),
            jobs.clone(),
            dedupe_jobs.clone(),
            PathBuf::from(&library),
            format!("w2-initial-{}", uuid::Uuid::new_v4()),
            "background",
        )
        .unwrap();
        initial.worker.join().unwrap();
        wait_healthy(&f.db, &root_id);
        let identity = file_identity_fingerprint(&f.source).unwrap();
        let destination = f.root.join("Destination");
        fs::create_dir(&destination).unwrap();
        let preview =
            f.db.get_manual_move_operation_preview(&f.id, &normalize_path(&destination), None)
                .unwrap();
        let mut selected = f.selection(&preview);
        selected.operations[0].destination_directory = Some(normalize_path(&destination));
        let request = resolve_execute_selections(&f.db, selected).unwrap();
        let paths = vec![normalize_path(&f.source), preview.target_path.clone()];
        // Duplicate endpoints resolve to exactly one root; the unmanaged sibling
        // Destination never becomes a scan root.
        assert_eq!(
            f.db.managed_root_ids_for_operation_paths(&paths).unwrap(),
            vec![root_id.clone()]
        );
        #[cfg(windows)]
        assert_eq!(
            f.db.managed_root_ids_for_operation_paths(&[format!("//?/{}", paths[0])])
                .unwrap(),
            vec![root_id.clone()]
        );
        let execute = || execute_moves_with_persistence(&f.db, request);
        let moved = if wake {
            super::reconciliation::with_reconciliation_wake(app.handle(), &f.db, &paths, execute)
        } else {
            execute()
        }
        .unwrap();
        assert_eq!(moved.logs[0].status, "success");
        if wake {
            wait_healthy(&f.db, &root_id);
        }
        let logs =
            f.db.get_restorable_operation_logs_by_ids(&[moved.logs[0].id.clone()])
                .unwrap();
        // Reuse the existing write hook to hold a real legacy scan's lease
        // after enumeration. Restore must join it, then completion must wake
        // reconciliation for the newly restored file missed by that snapshot.
        let held = if wake {
            use crate::db::scan::{set_managed_scan_write_test_hook, ManagedScanWriteTestPoint};
            fs::write(Path::new(&library).join("lease-anchor.txt"), b"anchor").unwrap();
            let job_id = format!("issue345-d4-w2-{}", uuid::Uuid::new_v4());
            let expected = job_id.clone();
            let (entered_tx, entered_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let release_rx = std::sync::Mutex::new(release_rx);
            let once = AtomicBool::new(false);
            set_managed_scan_write_test_hook(Some(Arc::new(move |id, point| {
                if id == expected
                    && point == ManagedScanWriteTestPoint::AfterCommit
                    && !once.swap(true, Ordering::SeqCst)
                {
                    entered_tx.send(()).unwrap();
                    release_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(20))
                        .unwrap();
                }
            })));
            let scan = crate::scanner::start_performance_managed_scan(
                app.handle().clone(),
                f.db.clone(),
                jobs.clone(),
                dedupe_jobs.clone(),
                PathBuf::from(&library),
                job_id,
                "background",
            )
            .unwrap();
            entered_rx.recv_timeout(Duration::from_secs(15)).unwrap();
            Some((scan, release_tx))
        } else {
            None
        };
        let restore = || restore_moves_with_persistence(&f.db, RestoreMovesRequest { logs });
        let restored = if wake {
            super::reconciliation::with_reconciliation_wake(app.handle(), &f.db, &paths, restore)
        } else {
            restore()
        }
        .unwrap();
        assert_eq!(restored.restored, 1, "{:?}", restored.logs);
        if let Some((scan, release)) = held {
            let pending = f.db.get_scan_root_health(Some(&root_id), None).unwrap();
            assert!(pending.needs_reconciliation);
            assert_eq!(pending.active_run_id.as_deref(), Some(scan.run_id.as_str()));
            release.send(()).unwrap();
            scan.worker.join().unwrap();
            crate::db::scan::set_managed_scan_write_test_hook(None);
        }
        if !wake {
            // Model a delivered exact watcher batch as well: equal counters
            // cannot clear the mutation fence or supply the missing dispatch.
            let revision = f.db.begin_watcher_revision(&root_id).unwrap().unwrap();
            f.db.complete_watcher_revision(&root_id, revision.watcher_revision)
                .unwrap();
            let root = f.db.get_scan_root_health(Some(&root_id), None).unwrap();
            assert!(root.needs_reconciliation);
            assert_eq!(root.health_status, "reconciliation_required");
            assert_eq!(root.watcher_revision, root.watcher_applied_revision);
            assert!(root.active_run_id.is_none());
            let admission =
                f.db.next_watcher_reconciliation_admission(
                    &root_id,
                    root.watcher_revision,
                    crate::db::current_unix_seconds(),
                )
                .unwrap();
            eprintln!("W2 old/no-wake root={root:?} admission={admission:?}");
            assert!(matches!(
                admission,
                WatcherReconciliationAdmission::Start { .. }
            ));
            // Cleanup via the same production wake, after recording the old
            // unresolved state. This is not part of the new-path acceptance.
            super::reconciliation::with_reconciliation_wake(app.handle(), &f.db, &paths, || Ok(()))
                .unwrap();
        }
        wait_healthy(&f.db, &root_id);
        let final_root = f.db.get_scan_root_health(Some(&root_id), None).unwrap();
        eprintln!("W2 wake={wake} final={final_root:?}");
        let after = file_identity_fingerprint(&f.source).unwrap();
        assert_eq!(after.platform_file_id, identity.platform_file_id);
        assert_eq!(after.platform_volume_id, identity.platform_volume_id);
        assert_eq!(after.full_hash, identity.full_hash);
        assert!(!Path::new(&preview.target_path).exists());
        let conn = rusqlite::Connection::open(f.db.path()).unwrap();
        let verified: String = conn
            .query_row(
                "SELECT verification_state FROM library_source_incarnations WHERE source_uid = ?1",
                [&uid],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(verified, "verified");
        let history = f.db.get_operation_logs(Some(10)).unwrap();
        assert_eq!(history[0].status, "success");
        assert_eq!(history[0].restore_status, "restored");
        assert_eq!(history[0].restore_phase, "completed");
        drop(conn);
        drop(app);
        f.finish();
    }
}
