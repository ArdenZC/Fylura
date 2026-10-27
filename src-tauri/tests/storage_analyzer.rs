#[cfg(windows)]
use std::sync::{atomic::AtomicBool, Arc};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn schema_18_adds_safe_trash_identity_columns() {
    let path = test_db_path();
    let db = Database::open(&path).expect("create current database");
    drop(db);
    let conn = rusqlite::Connection::open(&path).expect("open sqlite");
    // Test-only teardown mirrors support/performance_fixture.rs before a real
    // Schema18 downgrade. DROP of the virtual table also removes its FTS shadows.
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS global_index_generation_entries_ai;
         DROP TRIGGER IF EXISTS global_index_generation_entries_insert_guard;
         DROP TRIGGER IF EXISTS global_index_generation_entries_no_update;
         DROP TRIGGER IF EXISTS global_index_generation_entries_no_delete;
         DROP TABLE IF EXISTS global_index_generation_fts;
         DROP TABLE IF EXISTS global_index_generation_entries;
         DROP TABLE IF EXISTS global_index_generation_heads;
         DROP TABLE IF EXISTS global_index_generations;",
    )
    .expect("remove only Schema42 objects from historical Schema18 fixture");
    let remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name GLOB 'global_index_generation*' OR tbl_name GLOB 'global_index_generation*'",
            [],
            |row| row.get(0),
        )
        .expect("inspect Schema42 teardown including FTS shadows and indexes");
    assert_eq!(remaining, 0, "historical fixture retains Schema42 objects");
    conn.execute_batch(
        r#"
        ALTER TABLE cleanup_trash_items DROP COLUMN source_modified_ns;
        ALTER TABLE cleanup_trash_items DROP COLUMN source_platform_file_id;
        ALTER TABLE cleanup_trash_items DROP COLUMN source_quick_hash;
        ALTER TABLE cleanup_trash_items DROP COLUMN source_full_hash;
        ALTER TABLE cleanup_trash_items DROP COLUMN trash_modified_ns;
        ALTER TABLE cleanup_trash_items DROP COLUMN trash_platform_volume_id;
        ALTER TABLE cleanup_trash_items DROP COLUMN trash_platform_file_id;
        ALTER TABLE cleanup_trash_items DROP COLUMN trash_quick_hash;
        ALTER TABLE cleanup_trash_items DROP COLUMN trash_full_hash;
        ALTER TABLE cleanup_trash_items DROP COLUMN identity_status;
        DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents;
        ALTER TABLE files DROP COLUMN filesystem_observation_key;
        ALTER TABLE scan_roots DROP COLUMN library_change_revision;
        DROP INDEX IF EXISTS idx_library_source_root_claims_source_state;
        DROP INDEX IF EXISTS idx_library_source_root_claims_root_path;
        DROP INDEX IF EXISTS idx_library_source_root_claims_path_state;
        DROP INDEX IF EXISTS idx_library_source_physical_verified;
        DROP INDEX IF EXISTS idx_library_source_physical_lookup;
        DROP TABLE IF EXISTS library_source_root_claims;
        DROP TABLE IF EXISTS library_root_epoch_state;
        PRAGMA user_version = 18;
        "#,
    )
    .expect("downgrade fixture to schema 18");
    drop(conn);

    Database::open(&path).expect("migrate schema 18 database");
    let conn = rusqlite::Connection::open(&path).expect("inspect migrated database");
    let columns = conn
        .prepare("PRAGMA table_info(cleanup_trash_items)")
        .expect("prepare columns")
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query columns")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect columns");
    let version: i64 = conn
        .query_row("SELECT user_version FROM pragma_user_version", [], |row| {
            row.get(0)
        })
        .expect("schema version");

    assert_eq!(version, 44);
    assert!(columns.iter().any(|column| column == "source_modified_ns"));
    assert!(columns
        .iter()
        .any(|column| column == "source_platform_file_id"));
    assert!(columns
        .iter()
        .any(|column| column == "source_platform_volume_id"));
    assert!(columns.iter().any(|column| column == "source_quick_hash"));
    assert!(columns.iter().any(|column| column == "source_full_hash"));
    assert!(columns.iter().any(|column| column == "trash_modified_ns"));
    assert!(columns
        .iter()
        .any(|column| column == "trash_platform_volume_id"));
    assert!(columns
        .iter()
        .any(|column| column == "trash_platform_file_id"));
    assert!(columns.iter().any(|column| column == "trash_quick_hash"));
    assert!(columns.iter().any(|column| column == "trash_full_hash"));
    assert!(columns.iter().any(|column| column == "identity_status"));
}

#[test]
fn pending_cleanup_query_includes_recovery_manual_review_without_terminal_history() {
    let path = test_db_path();
    let db = Database::open(&path).expect("create recovery query database");
    let conn = rusqlite::Connection::open(&path).expect("open recovery query database");
    conn.execute(
        "INSERT INTO cleanup_trash_batches (id, created_at, total_items, total_size, status) VALUES ('recovery-batch', '1', 4, 4, 'pending')",
        [],
    )
    .expect("insert cleanup batch");
    for (id, status, identity, phase) in [
        ("pending", "pending", "pending", "prepared"),
        (
            "move-recovery",
            "manual_review",
            "pending_recovery",
            "manual_review",
        ),
        (
            "restore-recovery",
            "manual_review",
            "restore_pending_recovery",
            "source_cleanup_pending",
        ),
        ("terminal", "manual_review", "verified", "completed"),
    ] {
        conn.execute(
            r#"
            INSERT INTO cleanup_trash_items (
                id, batch_id, original_path, trash_path, name, size, moved_at,
                status, identity_status, operation_phase
            ) VALUES (?1, 'recovery-batch', ?2, ?3, ?4, 1, '1', ?5, ?6, ?7)
            "#,
            rusqlite::params![
                id,
                format!("C:/recovery/{id}.txt"),
                format!("C:/recovery/.zen-canvas-trash/{id}.txt"),
                id,
                status,
                identity,
                phase
            ],
        )
        .expect("insert cleanup recovery item");
    }
    drop(conn);

    let ids = db
        .pending_cleanup_trash_items()
        .expect("query pending cleanup recovery items")
        .into_iter()
        .map(|item| item.id)
        .collect::<std::collections::HashSet<_>>();
    assert!(ids.contains("pending"));
    assert!(ids.contains("move-recovery"));
    assert!(ids.contains("restore-recovery"));
    assert!(!ids.contains("terminal"));
}

use zen_canvas_tauri::db::Database;
use zen_canvas_tauri::storage_analyzer::{
    analyze_storage_roots_for_test, candidates_by_job_and_ids_for_test,
    classify_candidate_for_test, cleanup_preview_items_for_candidates, default_scan_roots_for_test,
    get_storage_cleanup_candidate_page_for_test, get_storage_cleanup_scan_status_for_test,
    is_forbidden_storage_path_for_test, is_main_window_label_for_test,
    mark_cleanup_candidates_consumed_for_test, preview_cleanup_operations_for_candidates,
    run_cleanup_restore_job_for_test, start_storage_cleanup_job_state_for_test,
    start_storage_cleanup_scan_for_test, store_completed_cleanup_analysis_for_test,
    validate_cleanup_roots_for_test, CleanupActionKind, CleanupRestoreJobStatus,
    CleanupRestoreState, CleanupRestoreTestOutcome, CleanupTier, StorageCandidate,
    StorageCleanupProgress, StorageCleanupState,
};
#[cfg(windows)]
use zen_canvas_tauri::storage_analyzer::{
    move_cleanup_candidates_to_safe_trash_for_candidates, preview_cleanup_restore_item_for_test,
    reconcile_pending_cleanup_journal, restore_cleanup_trash_items_for_db,
    restore_cleanup_trash_items_for_db_with_cancel_for_test,
};

#[test]
fn storage_analyzer_preserves_dependency_folder_policy() {
    let root = test_dir();
    let project = root.join("workspace").join("node_modules");
    write_file(&project.join("package").join("index.js"), 128);

    let analysis = analyze_storage_roots_for_test(vec![root.clone()], Vec::new())
        .expect("analyze storage roots");
    let candidate = analysis
        .candidates
        .iter()
        .find(|candidate| candidate.path.ends_with("node_modules"))
        .expect("node_modules candidate");

    assert_eq!(candidate.tier, CleanupTier::Safe);
    assert_eq!(candidate.suggested_action, CleanupActionKind::MoveToTrash);
    assert!(candidate.trash_allowed);
    assert!(candidate.selected_by_default);
    assert!(candidate
        .risk_note
        .as_deref()
        .unwrap_or("")
        .contains("dependency"));
    assert!(analysis.reclaimable_estimate >= candidate.size);
}

#[test]
fn default_storage_cleanup_roots_do_not_scan_entire_appdata_or_program_files() {
    let roots = default_scan_roots_for_test()
        .into_iter()
        .map(|path| {
            path.to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase()
        })
        .collect::<Vec<_>>();

    assert!(roots.iter().any(|path| path.contains("/downloads")));
    assert!(roots.iter().any(|path| path.contains("/desktop")));
    assert!(roots.iter().any(|path| path.contains("/documents")));
    assert!(roots.iter().any(|path| path.contains("/temp")));
    assert!(roots
        .iter()
        .any(|path| path.contains(".npm") || path.contains("npm-cache")));
    assert!(!roots.iter().any(|path| path.ends_with("/.cargo")));
    assert!(!roots.iter().any(|path| path.ends_with("/.m2")));
    assert!(!roots.iter().any(|path| path.ends_with("/.gradle")));
    assert!(roots
        .iter()
        .any(|path| path.ends_with("/.cargo/registry/cache")));
    assert!(roots.iter().any(|path| path.ends_with("/.m2/repository")));
    assert!(roots.iter().any(|path| path.ends_with("/.gradle/caches")));
    assert!(!roots.iter().any(|path| path.ends_with("/appdata/local")));
    assert!(!roots.iter().any(|path| path.ends_with("/appdata/roaming")));
    assert!(!roots.iter().any(|path| path.ends_with("/program files")));
    assert!(!roots
        .iter()
        .any(|path| path.ends_with("/program files (x86)")));
}

#[test]
fn storage_cleanup_scan_requires_explicit_user_roots() {
    let result = validate_cleanup_roots_for_test(Vec::new());

    assert!(result.is_err());
}

#[test]
fn storage_cleanup_scan_only_uses_user_selected_roots() {
    let selected_root = test_dir();
    let other_root = test_dir();
    let selected_cache = selected_root.join("project").join("node_modules");
    let other_cache = other_root.join("project").join("node_modules");
    write_file(&selected_cache.join("package").join("index.js"), 128);
    write_file(&other_cache.join("package").join("index.js"), 128);

    let analysis = analyze_storage_roots_for_test(vec![selected_root.clone()], Vec::new())
        .expect("analyze selected root");

    assert!(analysis
        .candidates
        .iter()
        .any(|candidate| candidate.path == selected_cache.to_string_lossy().replace('\\', "/")));
    assert!(!analysis
        .candidates
        .iter()
        .any(|candidate| candidate.path == other_cache.to_string_lossy().replace('\\', "/")));
}

#[test]
fn storage_cleanup_scan_rejects_protected_system_roots() {
    let root = if cfg!(windows) {
        "C:/Windows/System32"
    } else if cfg!(target_os = "macos") {
        "/System"
    } else {
        "/etc"
    };
    let result = validate_cleanup_roots_for_test(vec![root.to_string()]);

    assert!(result.is_err());
}

#[test]
fn storage_cleanup_rejects_unix_system_roots() {
    for path in ["/", "/usr", "/etc", "/var", "/bin", "/sbin"] {
        assert!(
            is_forbidden_storage_path_for_test(Path::new(path)),
            "expected {path} to be protected"
        );
    }

    #[cfg(target_os = "macos")]
    assert!(is_forbidden_storage_path_for_test(Path::new("/System")));
}

#[test]
fn storage_cleanup_scan_accepts_user_selected_temp_directory() {
    let root = test_dir();
    let result = validate_cleanup_roots_for_test(vec![root.to_string_lossy().into_owned()]);

    assert!(result.is_ok());
}

#[cfg(windows)]
#[test]
fn cleanup_roots_accept_extended_drive_spelling_after_normal_validation() {
    let root = test_dir();
    let extended = format!(r"\\?\{}", root.display());
    let ordinary = validate_cleanup_roots_for_test(vec![root.to_string_lossy().into_owned()])
        .expect("ordinary cleanup root");
    let extended_roots =
        validate_cleanup_roots_for_test(vec![extended]).expect("extended cleanup root");

    assert_eq!(extended_roots, ordinary);

    let malformed = r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy1".to_string();
    let error = validate_cleanup_roots_for_test(vec![malformed])
        .expect_err("device namespace must not enter cleanup validation");
    assert!(error.contains("unsupported Windows path prefix"));

    let escape = format!(r"\\?\{}\child\..", root.display());
    let error = validate_cleanup_roots_for_test(vec![escape])
        .expect_err("extended root traversal must remain rejected");
    assert!(error.contains("parent-directory traversal"));
}

#[test]
fn cleanup_roots_are_deduplicated_after_canonicalization() {
    let root = test_dir();
    let aliased = root.join(".");

    let roots = validate_cleanup_roots_for_test(vec![
        root.to_string_lossy().into_owned(),
        aliased.to_string_lossy().into_owned(),
    ])
    .expect("canonical cleanup roots");

    assert_eq!(roots, vec![root.canonicalize().expect("canonical root")]);
}

#[test]
fn cleanup_roots_reject_wildcards_and_parent_traversal_before_filesystem_access() {
    let root = test_dir();
    for invalid in [
        root.join("*"),
        root.join("?"),
        root.join("child").join(".."),
    ] {
        let error = validate_cleanup_roots_for_test(vec![invalid.to_string_lossy().into_owned()])
            .expect_err("invalid cleanup root syntax must be rejected");
        assert!(
            error.contains("unsupported path characters")
                || error.contains("parent-directory traversal"),
            "unexpected error: {error}"
        );
    }
}

#[cfg(unix)]
#[test]
fn cleanup_roots_reject_nul_before_filesystem_access() {
    let error = validate_cleanup_roots_for_test(vec!["/tmp/invalid\0root".to_string()])
        .expect_err("NUL cleanup root must be rejected");

    assert!(error.contains("unsupported path characters"));
}

#[test]
fn cleanup_root_symlinks_to_system_directories_are_rejected() {
    let root = test_dir();
    let link = root.join("root-link");
    let target = if cfg!(windows) {
        PathBuf::from("C:/Windows")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/System")
    } else {
        PathBuf::from("/etc")
    };
    create_directory_symlink_for_test(&target, &link)
        .expect("create protected cleanup-root symlink fixture");

    let result = validate_cleanup_roots_for_test(vec![link.to_string_lossy().into_owned()]);
    remove_directory_symlink_for_test(&link).expect("remove system directory symlink");
    let error = result.expect_err("symlink cleanup root must be rejected");

    assert!(error.to_ascii_lowercase().contains("symlink"));
}

#[test]
fn cleanup_root_symlink_to_current_temp_is_not_automatically_allowed() {
    let root = test_dir();
    let link = root.join("temp-link");
    create_directory_symlink_for_test(&std::env::temp_dir(), &link)
        .expect("create temp-root symlink fixture");

    let result = validate_cleanup_roots_for_test(vec![link.to_string_lossy().into_owned()]);
    remove_directory_symlink_for_test(&link).expect("remove temp directory symlink");
    assert!(result.is_err());
}

#[test]
fn current_user_recent_temp_file_is_not_default_safe_in_real_scan() {
    let root = current_temp_test_dir();
    let recent = root.join("recent.tmp");
    write_file(&recent, 128);

    let analysis =
        analyze_storage_roots_for_test(vec![root], Vec::new()).expect("scan current temp child");

    assert!(!analysis.candidates.iter().any(|candidate| {
        candidate.path == recent.to_string_lossy().replace('\\', "/")
            && candidate.tier == CleanupTier::Safe
            && candidate.selected_by_default
    }));
}

#[test]
fn current_user_old_temp_file_is_safe_in_real_scan() {
    let root = current_temp_test_dir();
    let old = root.join("old.tmp");
    write_file(&old, 128);
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&old)
        .expect("open old temp file");
    file.set_times(
        fs::FileTimes::new()
            .set_modified(SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60)),
    )
    .expect("age temp file");

    let analysis = analyze_storage_roots_for_test(vec![root], Vec::new())
        .expect("scan aged current temp child");
    let candidate = analysis
        .candidates
        .iter()
        .find(|candidate| candidate.path == old.to_string_lossy().replace('\\', "/"))
        .expect("old temp candidate");

    assert_eq!(candidate.tier, CleanupTier::Safe);
    assert!(candidate.trash_allowed);
    assert!(candidate.selected_by_default);
}

#[test]
fn temp_execution_revalidation_enforces_age_policy() {
    let root = current_temp_test_dir();
    let recent = root.join("recent-execution.tmp");
    let old = root.join("old-execution.tmp");
    write_file(&recent, 128);
    write_file(&old, 128);
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&old)
        .expect("open old execution temp file");
    file.set_times(
        fs::FileTimes::new()
            .set_modified(SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60)),
    )
    .expect("age execution temp file");

    assert!(zen_canvas_tauri::storage_analyzer::is_cleanup_execution_forbidden(&recent, None));
    assert!(!zen_canvas_tauri::storage_analyzer::is_cleanup_execution_forbidden(&old, None));
}

#[cfg(target_os = "macos")]
#[test]
fn storage_cleanup_only_accepts_the_current_macos_temp_root() {
    let current_temp_entry = current_temp_test_dir();
    assert!(!is_forbidden_storage_path_for_test(&current_temp_entry));
    fs::remove_dir_all(&current_temp_entry).expect("remove current temp fixture");

    for path in [
        "/var/folders/another-user/T/zen-canvas-test",
        "/private/var/folders/another-user/T/zen-canvas-test",
        "/private/var/db/zen-canvas-test",
    ] {
        assert!(
            is_forbidden_storage_path_for_test(Path::new(path)),
            "expected non-current macOS temp path {path} to remain protected"
        );
    }
}

#[test]
fn storage_cleanup_scan_job_can_report_progress_and_be_cancelled() {
    let root = test_dir();
    for index in 0..150 {
        write_file(&root.join(format!("file-{index}.tmp")), 32);
    }
    let state = StorageCleanupState::default();

    let job_id =
        start_storage_cleanup_scan_for_test(vec![root.clone()], &state).expect("start job");
    let mut started = get_storage_cleanup_scan_status_for_test(&job_id, &state).expect("status");
    for _ in 0..20 {
        if started.progress.scanned_entries > 0 || started.analysis.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
        started = get_storage_cleanup_scan_status_for_test(&job_id, &state).expect("status");
    }

    assert_eq!(started.job_id, job_id);
    assert!(matches!(started.status.as_str(), "running" | "completed"));
    assert!(started.progress.scanned_entries > 0 || started.analysis.is_some());

    zen_canvas_tauri::storage_analyzer::cancel_storage_cleanup_scan_for_test(&job_id, &state)
        .expect("cancel job");
    thread::sleep(Duration::from_millis(30));
    let cancelled = get_storage_cleanup_scan_status_for_test(&job_id, &state).expect("status");

    assert!(matches!(
        cancelled.status.as_str(),
        "cancelled" | "completed"
    ));
}

#[test]
fn storage_cleanup_progress_payload_serializes_camel_case() {
    let payload = StorageCleanupProgress {
        job_id: "job-1".to_string(),
        scanned_entries: 7,
        current_path: Some("C:/Users/Zen/file.tmp".to_string()),
        total_size: 99,
    };
    let value = serde_json::to_value(payload).expect("serialize progress");

    assert_eq!(value["jobId"], "job-1");
    assert_eq!(value["scannedEntries"], 7);
    assert_eq!(value["currentPath"], "C:/Users/Zen/file.tmp");
    assert_eq!(value["totalSize"], 99);
}

#[test]
fn cleanup_job_a_cannot_execute_job_b_candidates() {
    let state = StorageCleanupState::default();
    let (job_a, candidate_a) = completed_cleanup_job(&state, "project-a");
    let (job_b, candidate_b) = completed_cleanup_job(&state, "project-b");

    let resolved =
        candidates_by_job_and_ids_for_test(&state, &job_a, std::slice::from_ref(&candidate_a.id))
            .expect("resolve job A candidate");
    assert_eq!(resolved[0].id, candidate_a.id);

    let error =
        candidates_by_job_and_ids_for_test(&state, &job_a, std::slice::from_ref(&candidate_b.id))
            .expect_err("job B candidate must not resolve through job A");
    assert!(
        error.contains("does not belong"),
        "unexpected error: {error}"
    );
    assert_ne!(job_a, job_b);
}

#[test]
fn cleanup_new_job_does_not_change_old_job_page() {
    let state = StorageCleanupState::default();
    let (job_a, _) = completed_cleanup_job(&state, "project-a");
    let page_before = get_storage_cleanup_candidate_page_for_test(&state, &job_a, 0, 200)
        .expect("job A page before job B");

    let _ = completed_cleanup_job(&state, "project-b");
    let page_after = get_storage_cleanup_candidate_page_for_test(&state, &job_a, 0, 200)
        .expect("job A page after job B");

    let before_ids = page_before
        .candidates
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect::<Vec<_>>();
    let after_ids = page_after
        .candidates
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(after_ids, before_ids);
}

#[test]
fn cleanup_missing_candidate_fails_whole_request() {
    let state = StorageCleanupState::default();
    let (job_id, candidate) = completed_cleanup_job(&state, "project-a");

    let error = candidates_by_job_and_ids_for_test(
        &state,
        &job_id,
        &[candidate.id, "storage-missing".to_string()],
    )
    .expect_err("one missing ID must fail the whole request");

    assert!(
        error.contains("does not belong"),
        "unexpected error: {error}"
    );
}

#[test]
fn cleanup_duplicate_candidate_ids_are_rejected() {
    let state = StorageCleanupState::default();
    let (job_id, candidate) = completed_cleanup_job(&state, "project-a");

    let error =
        candidates_by_job_and_ids_for_test(&state, &job_id, &[candidate.id.clone(), candidate.id])
            .expect_err("duplicate IDs must be rejected");

    assert!(error.contains("duplicate"), "unexpected error: {error}");
}

#[test]
fn cleanup_consumed_candidate_is_rejected_before_a_second_execution() {
    let state = StorageCleanupState::default();
    let (job_id, candidate) = completed_cleanup_job(&state, "project-a");
    mark_cleanup_candidates_consumed_for_test(&state, &job_id, std::slice::from_ref(&candidate.id))
        .expect("mark consumed");

    let error =
        candidates_by_job_and_ids_for_test(&state, &job_id, std::slice::from_ref(&candidate.id))
            .expect_err("consumed candidate must fail before execution");
    assert!(
        error.contains("already consumed"),
        "unexpected error: {error}"
    );
}

#[test]
fn cleanup_running_and_cancelled_jobs_cannot_resolve_candidates() {
    let state = StorageCleanupState::default();
    start_storage_cleanup_job_state_for_test(&state, "job-running").expect("start job state");

    let running = candidates_by_job_and_ids_for_test(&state, "job-running", &[])
        .expect_err("running job must not resolve candidates");
    assert!(running.contains("not completed"));

    zen_canvas_tauri::storage_analyzer::cancel_storage_cleanup_scan_for_test("job-running", &state)
        .expect("cancel job state");
    let cancelled = candidates_by_job_and_ids_for_test(&state, "job-running", &[])
        .expect_err("cancelled job must not resolve candidates");
    assert!(cancelled.contains("not completed"));
}

#[test]
fn cleanup_candidate_resolution_preserves_request_order() {
    let state = StorageCleanupState::default();
    let (job_id, candidates) = completed_cleanup_job_with_candidates(&state, "ordered", 2);
    let requested = vec![candidates[1].id.clone(), candidates[0].id.clone()];

    let resolved = candidates_by_job_and_ids_for_test(&state, &job_id, &requested)
        .expect("resolve candidates in request order");
    let resolved_ids = resolved
        .into_iter()
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();

    assert_eq!(resolved_ids, requested);
}

#[test]
fn cleanup_candidate_ids_are_scoped_to_jobs() {
    let state = StorageCleanupState::default();
    let root = cleanup_candidate_root("same-path", 1);
    let job_a = start_storage_cleanup_scan_for_test(vec![root.clone()], &state).expect("job A");
    let candidate_a = wait_for_completed_cleanup_job(&state, &job_a).candidates[0].clone();
    let job_b = start_storage_cleanup_scan_for_test(vec![root], &state).expect("job B");
    let candidate_b = wait_for_completed_cleanup_job(&state, &job_b).candidates[0].clone();

    assert_ne!(candidate_a.id, candidate_b.id);
}

#[test]
fn cleanup_completion_payload_uses_job_scoped_candidate_ids() {
    let state = StorageCleanupState::default();
    let root = cleanup_candidate_root("completion-payload", 1);
    let analysis = analyze_storage_roots_for_test(vec![root], Vec::new()).expect("analysis");

    let completed = store_completed_cleanup_analysis_for_test(&state, "job-completed", analysis)
        .expect("store completed analysis");
    let page = get_storage_cleanup_candidate_page_for_test(&state, "job-completed", 0, 200)
        .expect("stored page");

    assert_eq!(completed.candidates[0].id, page.candidates[0].id);
}

#[test]
fn cleanup_evicted_job_cannot_resolve_candidates() {
    let state = StorageCleanupState::default();
    let (old_job, old_candidate) = completed_cleanup_job(&state, "oldest");
    for index in 0..8 {
        let _ = completed_cleanup_job(&state, &format!("newer-{index}"));
    }

    let error = candidates_by_job_and_ids_for_test(
        &state,
        &old_job,
        std::slice::from_ref(&old_candidate.id),
    )
    .expect_err("evicted job must not resolve candidates");

    assert!(error.contains("not found"), "unexpected error: {error}");
}

#[test]
fn storage_analyzer_marks_download_media_as_review_without_default_trash() {
    let candidate = classify_candidate_for_test(
        &PathBuf::from("C:/Users/zen/Downloads/movie.mp4"),
        50_000_000,
    );

    assert_eq!(candidate.tier, CleanupTier::Review);
    assert_eq!(candidate.suggested_action, CleanupActionKind::Reveal);
    assert!(!candidate.trash_allowed);
}

#[test]
fn storage_analyzer_keeps_program_files_and_database_like_paths_cautious() {
    let app_candidate = classify_candidate_for_test(
        &PathBuf::from("C:/Program Files/Example/app.exe"),
        50_000_000,
    );
    let db_candidate = classify_candidate_for_test(
        &PathBuf::from("C:/Users/zen/AppData/Roaming/Tencent/WeChat/msg.db"),
        50_000_000,
    );

    assert_eq!(app_candidate.tier, CleanupTier::Caution);
    assert_eq!(
        app_candidate.suggested_action,
        CleanupActionKind::UninstallAdvice
    );
    assert!(!app_candidate.trash_allowed);
    assert_eq!(db_candidate.tier, CleanupTier::Caution);
    assert!(!db_candidate.trash_allowed);
}

#[cfg(windows)]
#[test]
fn storage_analyzer_rejects_forbidden_system_and_app_database_paths() {
    let app_data = PathBuf::from("C:/Users/zen/AppData/Roaming/Startlan/Zen Canvas");

    assert!(is_forbidden_storage_path_for_test(Path::new(
        "C:/Windows/System32"
    )));
    assert!(is_forbidden_storage_path_for_test(Path::new(
        "C:/ProgramData"
    )));
    assert!(is_forbidden_storage_path_for_test(&app_data));
}

#[test]
fn storage_analyzer_records_forbidden_roots_as_denied_without_safe_candidates() {
    let system_root = if cfg!(windows) {
        "C:/Windows/System32"
    } else if cfg!(target_os = "macos") {
        "/System"
    } else {
        "/etc"
    };
    let analysis = analyze_storage_roots_for_test(vec![PathBuf::from(system_root)], Vec::new())
        .expect("analyze forbidden root");

    assert_eq!(analysis.total_size, 0);
    assert!(!analysis.denied_paths.is_empty());
    assert!(analysis
        .candidates
        .iter()
        .all(|candidate| candidate.tier != CleanupTier::Safe));
}

#[test]
fn storage_analyzer_marks_temp_and_package_caches_safe() {
    let temp_candidate = classify_candidate_for_test(
        &PathBuf::from("C:/Users/zen/AppData/Local/Temp/zen-cache.tmp"),
        500,
    );
    let cargo_candidate =
        classify_candidate_for_test(&PathBuf::from("C:/Users/zen/.cargo/registry/cache"), 500);

    assert_eq!(temp_candidate.tier, CleanupTier::Safe);
    assert!(temp_candidate.trash_allowed);
    assert!(temp_candidate.selected_by_default);
    assert_eq!(cargo_candidate.tier, CleanupTier::Safe);
    assert!(cargo_candidate.trash_allowed);
    assert!(cargo_candidate.selected_by_default);
}

#[test]
fn storage_analyzer_never_selects_developer_credentials_or_config() {
    for path in [
        "C:/Users/zen/.cargo/credentials.toml",
        "C:/Users/zen/.m2/settings.xml",
        "C:/Users/zen/.gradle/init.gradle",
    ] {
        let candidate = classify_candidate_for_test(Path::new(path), 500);
        assert_ne!(candidate.tier, CleanupTier::Safe, "{path}");
        assert!(!candidate.trash_allowed, "{path}");
        assert!(!candidate.selected_by_default, "{path}");
    }
}

#[test]
fn storage_analyzer_requires_review_for_output_names_without_safety_evidence() {
    for path in [
        "C:/Users/zen/project/build",
        "C:/Users/zen/project/dist",
        "C:/Users/zen/project/target",
        "F:/Coding/Zen-Canvas/Library/build",
        "C:/Users/zen/project/.next",
        "C:/Users/zen/project/.nuxt",
        "C:/Users/zen/project/.turbo",
        "C:/Users/zen/project/BuIlD",
    ] {
        let candidate = classify_candidate_for_test(&PathBuf::from(path), 500);

        assert_eq!(candidate.tier, CleanupTier::Review);
        assert_eq!(candidate.suggested_action, CleanupActionKind::Reveal);
        assert!(!candidate.trash_allowed);
        assert!(!candidate.selected_by_default);
    }
}

#[test]
fn storage_analyzer_does_not_return_overlapping_safe_parent_and_child_candidates() {
    let root = test_dir();
    let parent = root.join("node_modules");
    let child = parent.join("package").join("dist");
    write_file(&child.join("index.js"), 128);

    let analysis = analyze_storage_roots_for_test(vec![root], Vec::new())
        .expect("analyze nested cleanup candidates");
    let safe_paths = analysis
        .candidates
        .iter()
        .filter(|candidate| candidate.trash_allowed)
        .map(|candidate| candidate.path.as_str())
        .collect::<Vec<_>>();

    assert!(safe_paths.iter().any(|path| path.ends_with("node_modules")));
    assert!(!safe_paths.iter().any(|path| path.ends_with("package/dist")));
}

#[test]
fn cleanup_preview_items_only_include_safe_trash_allowed_candidates() {
    let safe =
        classify_candidate_for_test(&PathBuf::from("C:/Users/zen/.cargo/registry/cache"), 100);
    let review =
        classify_candidate_for_test(&PathBuf::from("C:/Users/zen/Downloads/movie.zip"), 100);
    let caution = classify_candidate_for_test(&PathBuf::from("C:/Program Files/Example"), 100);

    let preview = cleanup_preview_items_for_candidates(
        vec![safe.id.clone(), review.id.clone(), caution.id.clone()],
        &[safe, review, caution],
    )
    .expect("preview items");

    assert_eq!(preview.len(), 1);
    assert_eq!(preview[0].tier, CleanupTier::Safe);
    assert_eq!(preview[0].operation_type, "move_to_trash_preview");
    assert!(!preview[0].is_executable);
}

#[test]
fn cleanup_preview_items_reject_system_paths_even_if_client_marks_them_safe() {
    let system_path = if cfg!(windows) {
        "C:/Windows/Temp"
    } else if cfg!(target_os = "macos") {
        "/System/Library"
    } else {
        "/etc/zen-canvas"
    };
    let forged = StorageCandidate {
        id: "forged-system-safe".to_string(),
        path: system_path.to_string(),
        name: "Temp".to_string(),
        size: 100,
        tier: CleanupTier::Safe,
        category: "Forged".to_string(),
        reason: "Client supplied".to_string(),
        suggested_action: CleanupActionKind::MoveToTrash,
        risk_note: None,
        trash_allowed: true,
        selected_by_default: true,
    };

    let preview = cleanup_preview_items_for_candidates(vec![forged.id.clone()], &[forged])
        .expect("preview items");

    assert!(preview.is_empty());
}

#[test]
fn storage_analyzer_ignores_zen_canvas_safe_trash_paths() {
    let root = test_dir();
    let safe_trash = root.join(".zen-canvas-trash").join("items");
    write_file(
        &safe_trash
            .join("batch")
            .join("item")
            .join("node_modules")
            .join("x.js"),
        256,
    );

    let analysis =
        analyze_storage_roots_for_test(vec![root], Vec::new()).expect("analyze storage roots");

    assert!(analysis
        .candidates
        .iter()
        .all(|candidate| !candidate.path.contains(".zen-canvas-trash")));
}

#[test]
fn cleanup_operation_preview_only_includes_safe_trash_candidates() {
    let root = test_dir();
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);
    let review = classify_candidate_for_test(&root.join("Downloads").join("movie.mp4"), 128);
    let caution = classify_candidate_for_test(&PathBuf::from("C:/Program Files/Example"), 128);

    let preview = preview_cleanup_operations_for_candidates(
        vec![safe.id.clone(), review.id.clone(), caution.id.clone()],
        &[safe.clone(), review, caution],
        None,
    )
    .expect("cleanup operation preview");

    assert_eq!(preview.total, 1);
    assert_eq!(preview.previews.len(), 1);
    assert_eq!(preview.previews[0].operation_type, "move_to_trash");
    assert_eq!(preview.previews[0].source_path, safe.path);

    #[cfg(target_os = "macos")]
    {
        assert_eq!(preview.previews[0].target_path, "Zen Canvas Safe Trash");
        assert_eq!(preview.previews[0].is_executable, Some(true));
        assert_eq!(preview.previews[0].blocking_reason, None);
    }
    #[cfg(not(target_os = "macos"))]
    {
        assert_eq!(preview.previews[0].target_path, "Recycle Bin");
        assert_eq!(preview.previews[0].is_executable, Some(false));
        assert_eq!(
            preview.previews[0].blocking_reason.as_deref(),
            Some("system_trash_source_binding_unsupported")
        );
    }
    assert!(preview.previews[0].requires_confirmation);
    assert_eq!(preview.previews[0].suggested_action, "DeleteCandidate");
    assert_eq!(preview.previews[0].editable_new_name, Some(false));
    assert_eq!(preview.previews[0].will_create_parent, Some(false));
}

#[test]
fn cleanup_operation_preview_rejects_system_and_app_data_paths() {
    let root = test_dir();
    let app_data = root.join("Zen Canvas");
    let app_data_child = app_data.join("cache");
    write_file(&app_data_child.join("owned.txt"), 64);
    let forged_app_data = StorageCandidate {
        id: "forged-app-data".to_string(),
        path: app_data_child.to_string_lossy().into_owned(),
        name: "cache".to_string(),
        size: 64,
        tier: CleanupTier::Safe,
        category: "Forged".to_string(),
        reason: "Client supplied".to_string(),
        suggested_action: CleanupActionKind::MoveToTrash,
        risk_note: None,
        trash_allowed: true,
        selected_by_default: true,
    };
    let forged_system = StorageCandidate {
        id: "forged-system".to_string(),
        path: "C:/Windows/System32".to_string(),
        name: "System32".to_string(),
        size: 64,
        tier: CleanupTier::Safe,
        category: "Forged".to_string(),
        reason: "Client supplied".to_string(),
        suggested_action: CleanupActionKind::MoveToTrash,
        risk_note: None,
        trash_allowed: true,
        selected_by_default: true,
    };

    let preview = preview_cleanup_operations_for_candidates(
        vec![forged_app_data.id.clone(), forged_system.id.clone()],
        &[forged_app_data, forged_system],
        Some(&app_data),
    )
    .expect("cleanup operation preview");

    assert!(preview.previews.is_empty());
    assert_eq!(preview.total, 0);
}

#[test]
#[cfg(windows)]
fn move_cleanup_candidates_to_safe_trash_records_and_restores_items() {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);

    let result = move_cleanup_candidates_to_safe_trash_for_candidates(
        vec![safe.id.clone()],
        std::slice::from_ref(&safe),
        &db,
        None,
    )
    .expect("move to safe trash");

    assert_eq!(result.moved, 1);
    assert_eq!(result.failed, 0);
    assert!(!safe_path.exists());
    let item = db.list_cleanup_trash_batches().expect("trash batches")[0].items[0].clone();
    assert_eq!(item.original_path, safe.path);
    assert_eq!(item.status, "moved");
    assert_eq!(item.identity_status, "verified");
    assert!(item.trash_quick_hash.is_some());
    if cfg!(target_os = "macos") {
        assert!(item.source_platform_volume_id.is_some());
    } else {
        assert!(item.source_platform_volume_id.is_none());
    }
    assert!(item.source_platform_file_id.is_some());
    assert!(item.trash_platform_volume_id.is_some());
    assert!(item.trash_platform_file_id.is_some());
    assert!(Path::new(&item.trash_path).starts_with(root.join(".zen-canvas-trash")));
    assert!(Path::new(&item.trash_path).exists());

    let restore =
        restore_cleanup_trash_items_for_db(vec![item.id.clone()], &db).expect("restore safe trash");

    assert_eq!(restore.restored, 1);
    assert!(safe_path.exists());
    let restored_item = db.list_cleanup_trash_batches().expect("trash batches")[0].items[0].clone();
    assert_eq!(restored_item.status, "restored");
    assert_eq!(restored_item.operation_phase, "completed");
    assert_eq!(restored_item.identity_status, "verified");
    assert!(restored_item.source_claim_path.is_none());
    assert!(restored_item.claim_created_at.is_none());
    assert!(restored_item.claim_platform_file_id.is_none());
    assert!(restored_item.claim_full_hash.is_none());
    let terminal_preview = preview_cleanup_restore_item_for_test(restored_item);
    assert!(!terminal_preview.can_restore);
    assert_eq!(
        terminal_preview.blocking_reason.as_deref(),
        Some("already restored")
    );
}

#[test]
#[cfg(windows)]
fn safe_trash_restore_final_transaction_failure_preserves_claim_for_manual_review() {
    let root = test_dir();
    let db_path = test_db_path();
    let db = Database::open(&db_path).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);
    move_cleanup_candidates_to_safe_trash_for_candidates(vec![safe.id.clone()], &[safe], &db, None)
        .expect("move to safe trash");
    let item = db.list_cleanup_trash_batches().expect("trash batches")[0].items[0].clone();
    let expected_claim_platform_id = item.trash_platform_file_id.clone();
    let expected_claim_hash = item.trash_full_hash.clone();

    let conn = rusqlite::Connection::open(&db_path).expect("open trigger database");
    conn.execute_batch(
        r#"
        CREATE TRIGGER reject_cleanup_restore_finalization
        BEFORE UPDATE OF status ON cleanup_trash_items
        WHEN NEW.status = 'restored'
        BEGIN
            SELECT RAISE(ABORT, 'injected Safe Trash finalization failure');
        END;
        "#,
    )
    .expect("install Safe Trash finalization trigger");
    drop(conn);

    let restore = restore_cleanup_trash_items_for_db(vec![item.id.clone()], &db)
        .expect("record Safe Trash finalization failure");
    let updated = db.list_cleanup_trash_batches().expect("updated batches")[0].items[0].clone();

    assert_eq!(restore.restored, 0);
    assert_eq!(restore.failed, 1);
    assert_eq!(updated.status, "manual_review");
    assert_eq!(updated.operation_phase, "target_committed");
    assert_eq!(updated.identity_status, "restore_pending_recovery");
    assert!(updated
        .message
        .as_deref()
        .unwrap_or_default()
        .starts_with("target_committed_durability_unknown:"));
    assert!(updated.source_claim_path.is_some());
    assert_eq!(updated.claim_platform_file_id, expected_claim_platform_id);
    assert_eq!(updated.claim_full_hash, expected_claim_hash);
    assert!(safe_path.exists());
}

#[test]
#[cfg(windows)]
fn pending_safe_trash_restore_reconciliation_finalizes_committed_target_and_clears_claim() {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);
    move_cleanup_candidates_to_safe_trash_for_candidates(vec![safe.id.clone()], &[safe], &db, None)
        .expect("move to safe trash");
    let mut item = db.list_cleanup_trash_batches().expect("trash batches")[0].items[0].clone();
    let claim_path = root.join(".missing-restore-claim");
    item.status = "pending".to_string();
    item.identity_status = "restore_pending".to_string();
    item.operation_phase = "completed".to_string();
    item.source_claim_path = Some(claim_path.to_string_lossy().replace('\\', "/"));
    item.claim_created_at = Some("1900000000000".to_string());
    item.claim_platform_file_id = item.trash_platform_file_id.clone();
    item.claim_full_hash = item.trash_full_hash.clone();
    db.update_cleanup_trash_item_status(&item)
        .expect("persist committed-target recovery fixture");
    fs::rename(&item.trash_path, &item.original_path).expect("recreate committed restore target");

    assert_eq!(
        reconcile_pending_cleanup_journal(&db).expect("reconcile committed target"),
        1
    );
    let updated = db.list_cleanup_trash_batches().expect("updated batches")[0].items[0].clone();
    assert_eq!(updated.status, "restored");
    assert_eq!(updated.operation_phase, "completed");
    assert_eq!(updated.identity_status, "verified");
    assert!(updated.source_claim_path.is_none());
    assert!(updated.claim_created_at.is_none());
    assert!(updated.claim_platform_file_id.is_none());
    assert!(updated.claim_full_hash.is_none());
    assert!(safe_path.exists());
}

#[cfg(windows)]
fn assert_pending_safe_trash_restore_a_phase_is_idempotent(phase: &str) {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);
    move_cleanup_candidates_to_safe_trash_for_candidates(vec![safe.id.clone()], &[safe], &db, None)
        .expect("move to safe trash");
    let mut item = db.list_cleanup_trash_batches().expect("trash batches")[0].items[0].clone();
    item.status = "pending".to_string();
    item.identity_status = "restore_pending".to_string();
    item.operation_phase = phase.to_string();
    item.source_claim_path = Some(
        root.join(".missing-restore-claim")
            .to_string_lossy()
            .replace('\\', "/"),
    );
    item.claim_created_at = Some("1900000000000".to_string());
    item.claim_platform_file_id = item.trash_platform_file_id.clone();
    item.claim_full_hash = item.trash_full_hash.clone();
    db.update_cleanup_trash_item_status(&item)
        .expect("persist committed target recovery fixture");
    fs::rename(&item.trash_path, &item.original_path).expect("recreate committed restore target");

    assert_eq!(
        reconcile_pending_cleanup_journal(&db).expect("reconcile committed target"),
        1
    );
    let updated = db.list_cleanup_trash_batches().expect("updated batches")[0].items[0].clone();
    assert_eq!(updated.status, "restored");
    assert_eq!(updated.operation_phase, "completed");
    assert_eq!(updated.identity_status, "verified");
    assert!(updated.source_claim_path.is_none());
    assert!(updated.claim_created_at.is_none());
    assert!(updated.claim_platform_file_id.is_none());
    assert!(updated.claim_full_hash.is_none());
    assert!(safe_path.exists());
    assert_eq!(
        reconcile_pending_cleanup_journal(&db).expect("reconcile restored target again"),
        0
    );
}

#[test]
#[cfg(windows)]
fn pending_safe_trash_restore_target_committed_a_path_is_restored_without_retry() {
    assert_pending_safe_trash_restore_a_phase_is_idempotent("target_committed");
}

#[test]
#[cfg(windows)]
fn pending_safe_trash_restore_source_cleanup_pending_a_path_is_restored_without_retry() {
    assert_pending_safe_trash_restore_a_phase_is_idempotent("source_cleanup_pending");
}

#[test]
#[cfg(windows)]
fn cleanup_restore_batch_failure_rolls_back_item_and_batch_together() {
    let root = test_dir();
    let db_path = test_db_path();
    let db = Database::open(&db_path).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);
    move_cleanup_candidates_to_safe_trash_for_candidates(vec![safe.id.clone()], &[safe], &db, None)
        .expect("move to safe trash");
    let item = db.list_cleanup_trash_batches().expect("trash batches")[0].items[0].clone();

    let conn = rusqlite::Connection::open(&db_path).expect("open trigger database");
    conn.execute_batch(
        r#"
        CREATE TRIGGER reject_cleanup_restore_batch
        BEFORE UPDATE OF status ON cleanup_trash_batches
        BEGIN
            SELECT RAISE(ABORT, 'injected Safe Trash batch update failure');
        END;
        "#,
    )
    .expect("install batch failure trigger");
    drop(conn);

    let mut outcome = item.clone();
    outcome.status = "restored".to_string();
    outcome.identity_status = "verified".to_string();
    outcome.operation_phase = "completed".to_string();
    outcome.restored_at = Some("1900000000123".to_string());
    assert!(db.finalize_cleanup_restore_outcome(&outcome).is_err());

    let conn = rusqlite::Connection::open(db.path()).expect("open rollback database");
    let persisted_item: (String, String) = conn
        .query_row(
            "SELECT status, operation_phase FROM cleanup_trash_items WHERE id = ?1",
            rusqlite::params![item.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read rolled-back item");
    let persisted_batch: String = conn
        .query_row(
            "SELECT status FROM cleanup_trash_batches WHERE id = ?1",
            rusqlite::params![item.batch_id],
            |row| row.get(0),
        )
        .expect("read rolled-back batch");
    assert_eq!(
        persisted_item,
        ("moved".to_string(), "completed".to_string())
    );
    assert_eq!(persisted_batch, "success");
}

#[test]
#[cfg(windows)]
fn pending_safe_trash_restore_reconciliation_rolls_back_before_commit_and_remains_restorable() {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);
    move_cleanup_candidates_to_safe_trash_for_candidates(vec![safe.id.clone()], &[safe], &db, None)
        .expect("move to safe trash");
    let mut item = db.list_cleanup_trash_batches().expect("trash batches")[0].items[0].clone();
    item.status = "pending".to_string();
    item.identity_status = "restore_pending".to_string();
    item.operation_phase = "prepared".to_string();
    item.source_claim_path = Some(
        root.join(".missing-restore-claim")
            .to_string_lossy()
            .replace('\\', "/"),
    );
    item.claim_created_at = Some("1900000000000".to_string());
    item.claim_platform_file_id = item.trash_platform_file_id.clone();
    item.claim_full_hash = item.trash_full_hash.clone();
    db.update_cleanup_trash_item_status(&item)
        .expect("persist pre-commit recovery fixture");

    assert_eq!(
        reconcile_pending_cleanup_journal(&db).expect("reconcile pre-commit restore"),
        1
    );
    let rolled_back = db
        .list_cleanup_trash_batches()
        .expect("rolled-back batches")[0]
        .items[0]
        .clone();
    assert_eq!(rolled_back.status, "moved");
    assert_eq!(rolled_back.operation_phase, "rolled_back");
    assert_eq!(rolled_back.identity_status, "verified");
    assert!(rolled_back.source_claim_path.is_none());
    assert!(preview_cleanup_restore_item_for_test(rolled_back.clone()).can_restore);

    let restored = restore_cleanup_trash_items_for_db(vec![rolled_back.id], &db)
        .expect("restore rolled-back Safe Trash item");
    assert_eq!(restored.restored, 1);
    assert!(safe_path.exists());
}

#[test]
#[cfg(windows)]
fn pending_safe_trash_restore_reconciliation_preserves_all_claim_fields_when_every_path_is_missing()
{
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);
    move_cleanup_candidates_to_safe_trash_for_candidates(vec![safe.id.clone()], &[safe], &db, None)
        .expect("move to safe trash");
    let mut item = db.list_cleanup_trash_batches().expect("trash batches")[0].items[0].clone();
    let claim_path = root.join(".missing-restore-claim");
    item.status = "pending".to_string();
    item.identity_status = "restore_pending".to_string();
    item.operation_phase = "prepared".to_string();
    item.source_claim_path = Some(claim_path.to_string_lossy().replace('\\', "/"));
    item.claim_created_at = Some("1900000000000".to_string());
    item.claim_platform_file_id = item.trash_platform_file_id.clone();
    item.claim_full_hash = item.trash_full_hash.clone();
    let expected_claim_path = item.source_claim_path.clone();
    let expected_claim_created_at = item.claim_created_at.clone();
    let expected_claim_platform_id = item.claim_platform_file_id.clone();
    let expected_claim_hash = item.claim_full_hash.clone();
    db.update_cleanup_trash_item_status(&item)
        .expect("persist all-missing recovery fixture");
    fs::remove_dir_all(&item.trash_path).expect("remove Safe Trash path");

    assert_eq!(
        reconcile_pending_cleanup_journal(&db).expect("reconcile all-missing restore"),
        1
    );
    let updated = db.list_cleanup_trash_batches().expect("updated batches")[0].items[0].clone();
    assert_eq!(updated.status, "manual_review");
    assert_eq!(updated.operation_phase, "target_committed");
    assert_eq!(updated.identity_status, "restore_pending_recovery");
    assert!(updated
        .message
        .as_deref()
        .unwrap_or_default()
        .starts_with("target_committed_durability_unknown:"));
    assert_eq!(updated.source_claim_path, expected_claim_path);
    assert_eq!(updated.claim_created_at, expected_claim_created_at);
    assert_eq!(updated.claim_platform_file_id, expected_claim_platform_id);
    assert_eq!(updated.claim_full_hash, expected_claim_hash);
    assert!(!preview_cleanup_restore_item_for_test(updated).can_restore);
}

#[test]
#[cfg(windows)]
fn pending_safe_trash_journal_reconciles_a_completed_move_after_restart() {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);
    move_cleanup_candidates_to_safe_trash_for_candidates(vec![safe.id.clone()], &[safe], &db, None)
        .expect("move to safe trash");
    let mut item = db.list_cleanup_trash_batches().expect("batches")[0].items[0].clone();
    item.status = "pending".to_string();
    item.message = Some("simulate interrupted journal update".to_string());
    db.update_cleanup_trash_item_status(&item)
        .expect("mark pending");

    let reconciled = reconcile_pending_cleanup_journal(&db).expect("reconcile cleanup");
    let batch = &db.list_cleanup_trash_batches().expect("batches after")[0];

    assert_eq!(reconciled, 1);
    assert_eq!(batch.status, "success");
    assert_eq!(batch.items[0].status, "moved");
}

#[test]
#[cfg(windows)]
fn pending_safe_trash_rejects_replaced_trash_identity() {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);
    move_cleanup_candidates_to_safe_trash_for_candidates(vec![safe.id.clone()], &[safe], &db, None)
        .expect("move to safe trash");
    let mut item = db.list_cleanup_trash_batches().expect("batches")[0].items[0].clone();
    item.status = "pending".to_string();
    db.update_cleanup_trash_item_status(&item)
        .expect("mark pending");
    fs::remove_dir_all(&item.trash_path).expect("remove moved trash item");
    let replacement = Path::new(&item.trash_path).join("package").join("index.js");
    fs::create_dir_all(replacement.parent().expect("replacement parent"))
        .expect("create replacement parent");
    fs::write(&replacement, vec![0x7f; 128]).expect("write unrelated replacement");

    reconcile_pending_cleanup_journal(&db).expect("reconcile cleanup");
    let recovered = db.list_cleanup_trash_batches().expect("batches after")[0].items[0].clone();
    let restore = restore_cleanup_trash_items_for_db(vec![recovered.id.clone()], &db)
        .expect("reject unsafe restore");

    assert_eq!(recovered.status, "manual_review");
    assert!(recovered
        .message
        .as_deref()
        .unwrap_or_default()
        .contains("identity"));
    assert_eq!(restore.restored, 0);
    assert_eq!(restore.failed, 1);
    assert!(!Path::new(&recovered.original_path).exists());
}

#[test]
#[cfg(windows)]
fn restore_safe_trash_rejects_identity_replaced_after_successful_move() {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);
    move_cleanup_candidates_to_safe_trash_for_candidates(vec![safe.id.clone()], &[safe], &db, None)
        .expect("move to safe trash");
    let item = db.list_cleanup_trash_batches().expect("batches")[0].items[0].clone();
    assert_eq!(item.status, "moved");
    fs::remove_dir_all(&item.trash_path).expect("remove moved trash item");
    let replacement = Path::new(&item.trash_path).join("package").join("index.js");
    fs::create_dir_all(replacement.parent().expect("replacement parent"))
        .expect("create replacement parent");
    fs::write(&replacement, vec![0x55; 128]).expect("write unrelated replacement");

    let restore = restore_cleanup_trash_items_for_db(vec![item.id.clone()], &db)
        .expect("reject replaced trash item");
    let updated = db.list_cleanup_trash_batches().expect("updated batches")[0].items[0].clone();

    assert_eq!(restore.restored, 0);
    assert_eq!(restore.failed, 1);
    assert_eq!(updated.status, "manual_review");
    assert!(updated
        .message
        .as_deref()
        .unwrap_or_default()
        .contains("identity"));
    assert!(!Path::new(&item.original_path).exists());
}

#[test]
#[cfg(windows)]
fn move_cleanup_candidates_to_safe_trash_rejects_review_caution_missing_and_system_paths() {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let review = classify_candidate_for_test(&root.join("Downloads").join("movie.mp4"), 128);
    let caution = classify_candidate_for_test(&PathBuf::from("C:/Program Files/Example"), 128);
    let forged_system = StorageCandidate {
        id: "forged-system".to_string(),
        path: "C:/Windows/System32".to_string(),
        name: "System32".to_string(),
        size: 64,
        tier: CleanupTier::Safe,
        category: "Forged".to_string(),
        reason: "Client supplied".to_string(),
        suggested_action: CleanupActionKind::MoveToTrash,
        risk_note: None,
        trash_allowed: true,
        selected_by_default: true,
    };

    let result = move_cleanup_candidates_to_safe_trash_for_candidates(
        vec![
            review.id.clone(),
            caution.id.clone(),
            forged_system.id.clone(),
            "missing".to_string(),
        ],
        &[review, caution, forged_system],
        &db,
        None,
    )
    .expect("move to safe trash");

    assert_eq!(result.moved, 0);
    assert_eq!(result.skipped, 4);
    assert!(db
        .list_cleanup_trash_batches()
        .expect("trash batches")
        .is_empty());
}

#[test]
#[cfg(windows)]
fn restore_cleanup_trash_items_blocks_conflicts_and_marks_missing_trash_paths() {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);
    let result = move_cleanup_candidates_to_safe_trash_for_candidates(
        vec![safe.id.clone()],
        &[safe],
        &db,
        None,
    )
    .expect("move to safe trash");
    assert_eq!(result.moved, 1);
    write_file(&safe_path.join("conflict.txt"), 16);
    let item = db.list_cleanup_trash_batches().expect("trash batches")[0].items[0].clone();

    let conflict =
        restore_cleanup_trash_items_for_db(vec![item.id.clone()], &db).expect("restore conflict");

    assert_eq!(conflict.restored, 0);
    assert_eq!(conflict.conflicts, 1);
    assert!(safe_path.exists());
    let conflicted_item = db
        .list_cleanup_trash_batches()
        .expect("trash batches after conflict")
        .into_iter()
        .flat_map(|batch| batch.items)
        .find(|candidate| candidate.id == item.id)
        .expect("conflicted item");
    assert_eq!(conflicted_item.status, "manual_review");
    assert_eq!(conflicted_item.operation_phase, "manual_review");
    assert_eq!(conflicted_item.identity_status, "restore_pending_recovery");
    assert!(conflicted_item
        .message
        .as_deref()
        .unwrap_or_default()
        .starts_with("manual_review_required:"));

    let trash_path = PathBuf::from(item.trash_path);
    if trash_path.exists() {
        if trash_path.is_dir() {
            fs::remove_dir_all(&trash_path).expect("remove trash path");
        } else {
            fs::remove_file(&trash_path).expect("remove trash path");
        }
    }
    fs::remove_dir_all(&safe_path).expect("remove conflict");

    let missing_path = root.join("second-project").join("node_modules");
    write_file(&missing_path.join("package").join("index.js"), 128);
    let missing_safe = classify_candidate_for_test(&missing_path, 128);
    let missing_move = move_cleanup_candidates_to_safe_trash_for_candidates(
        vec![missing_safe.id.clone()],
        &[missing_safe],
        &db,
        None,
    )
    .expect("move missing fixture to safe trash");
    assert_eq!(missing_move.moved, 1);
    let missing_item = db
        .list_cleanup_trash_batches()
        .expect("trash batches for missing fixture")
        .into_iter()
        .flat_map(|batch| batch.items)
        .find(|candidate| candidate.original_path.contains("second-project"))
        .expect("missing fixture item");
    let missing_trash_path = PathBuf::from(&missing_item.trash_path);
    if missing_trash_path.exists() {
        if missing_trash_path.is_dir() {
            fs::remove_dir_all(&missing_trash_path).expect("remove missing trash path");
        } else {
            fs::remove_file(&missing_trash_path).expect("remove missing trash path");
        }
    }
    let missing = restore_cleanup_trash_items_for_db(vec![missing_item.id.clone()], &db)
        .expect("restore missing");

    assert_eq!(missing.restored, 0);
    assert_eq!(missing.missing, 1);
    let missing_item = db
        .list_cleanup_trash_batches()
        .expect("trash batches after missing")
        .into_iter()
        .flat_map(|batch| batch.items)
        .find(|candidate| candidate.id == missing_item.id)
        .expect("updated missing fixture item");
    assert_eq!(missing_item.status, "missing");
}

#[test]
#[cfg(windows)]
fn cleanup_restore_preview_marks_filesystem_conflicts_and_missing_sources() {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);
    move_cleanup_candidates_to_safe_trash_for_candidates(vec![safe.id.clone()], &[safe], &db, None)
        .expect("move to safe trash");
    let item = db.list_cleanup_trash_batches().expect("trash batches")[0].items[0].clone();

    let executable = preview_cleanup_restore_item_for_test(item.clone());
    assert!(executable.can_restore);
    assert_eq!(executable.blocking_reason, None);

    write_file(&safe_path.join("conflict.txt"), 16);
    let conflict = preview_cleanup_restore_item_for_test(item.clone());
    assert!(!conflict.can_restore);
    assert_eq!(conflict.blocking_reason.as_deref(), Some("conflict"));

    fs::remove_dir_all(&safe_path).expect("remove conflict path");
    let trash_path = PathBuf::from(&item.trash_path);
    if trash_path.is_dir() {
        fs::remove_dir_all(&trash_path).expect("remove trash path");
    } else if trash_path.exists() {
        fs::remove_file(&trash_path).expect("remove trash path");
    }
    let missing = preview_cleanup_restore_item_for_test(item);
    assert!(!missing.can_restore);
    assert_eq!(missing.blocking_reason.as_deref(), Some("missing"));
}

#[test]
#[cfg(windows)]
fn cleanup_restore_preview_and_execution_reject_replacement_after_preview() {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);
    move_cleanup_candidates_to_safe_trash_for_candidates(vec![safe.id.clone()], &[safe], &db, None)
        .expect("move to safe trash");
    let item = db.list_cleanup_trash_batches().expect("trash batches")[0].items[0].clone();
    assert!(preview_cleanup_restore_item_for_test(item.clone()).can_restore);

    fs::remove_dir_all(&item.trash_path).expect("remove previewed trash item");
    let replacement = Path::new(&item.trash_path).join("package").join("index.js");
    fs::create_dir_all(replacement.parent().expect("replacement parent"))
        .expect("create replacement parent");
    fs::write(&replacement, vec![0x44; 128]).expect("write same-size replacement");

    let second_preview = preview_cleanup_restore_item_for_test(item.clone());
    assert!(!second_preview.can_restore);
    assert_eq!(
        second_preview.blocking_reason.as_deref(),
        Some("replacement_detected")
    );
    let restore = restore_cleanup_trash_items_for_db(vec![item.id], &db)
        .expect("block replacement after preview");
    assert_eq!(restore.restored, 0);
    assert_eq!(restore.failed, 1);
    assert!(!safe_path.exists());
}

#[test]
#[cfg(windows)]
fn cleanup_restore_rejects_same_content_with_a_different_platform_identity() {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);
    move_cleanup_candidates_to_safe_trash_for_candidates(vec![safe.id.clone()], &[safe], &db, None)
        .expect("move to safe trash");
    let item = db.list_cleanup_trash_batches().expect("trash batches")[0].items[0].clone();
    let original_bytes = fs::read(Path::new(&item.trash_path).join("package").join("index.js"))
        .expect("read moved bytes");
    fs::remove_dir_all(&item.trash_path).expect("remove moved object");
    let replacement = Path::new(&item.trash_path).join("package").join("index.js");
    fs::create_dir_all(replacement.parent().expect("replacement parent"))
        .expect("create replacement parent");
    fs::write(&replacement, original_bytes).expect("write identical replacement");

    let preview = preview_cleanup_restore_item_for_test(item.clone());
    let restore = restore_cleanup_trash_items_for_db(vec![item.id], &db)
        .expect("reject different platform identity");

    assert!(!preview.can_restore);
    assert_eq!(
        preview.blocking_reason.as_deref(),
        Some("replacement_detected")
    );
    assert_eq!(restore.restored, 0);
    assert_eq!(restore.failed, 1);
    assert!(!safe_path.exists());
}

#[test]
#[cfg(windows)]
fn legacy_safe_trash_identity_is_manual_review_only() {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);
    move_cleanup_candidates_to_safe_trash_for_candidates(vec![safe.id.clone()], &[safe], &db, None)
        .expect("move to safe trash");
    let mut item = db.list_cleanup_trash_batches().expect("trash batches")[0].items[0].clone();
    item.identity_status = "legacy_unverified".to_string();
    item.trash_modified_ns = None;
    item.trash_platform_volume_id = None;
    item.trash_platform_file_id = None;
    item.trash_quick_hash = None;
    db.update_cleanup_trash_item_status(&item)
        .expect("persist legacy identity fixture");

    let preview = preview_cleanup_restore_item_for_test(item.clone());
    let restore = restore_cleanup_trash_items_for_db(vec![item.id], &db)
        .expect("block legacy identity restore");

    assert!(!preview.can_restore);
    assert_eq!(preview.blocking_reason.as_deref(), Some("manual_review"));
    assert_eq!(restore.restored, 0);
    assert_eq!(restore.failed, 1);
    assert!(!safe_path.exists());
}

#[test]
#[cfg(windows)]
fn cleanup_restore_cancellation_skips_remaining_items_without_moving_files() {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open db");
    let safe_path = root.join("node_modules");
    write_file(&safe_path.join("package").join("index.js"), 128);
    let safe = classify_candidate_for_test(&safe_path, 128);
    move_cleanup_candidates_to_safe_trash_for_candidates(vec![safe.id.clone()], &[safe], &db, None)
        .expect("move to safe trash");
    let item = db.list_cleanup_trash_batches().expect("trash batches")[0].items[0].clone();
    let cancel = Arc::new(AtomicBool::new(true));

    let result = restore_cleanup_trash_items_for_db_with_cancel_for_test(
        vec![item.id.clone()],
        &db,
        Arc::clone(&cancel),
    )
    .expect("canceled restore");

    assert_eq!(result.restored, 0);
    assert_eq!(result.canceled, 1);
    assert_eq!(result.logs[0].status, "canceled");
    assert!(!safe_path.exists());
    assert!(Path::new(&item.trash_path).exists());
    assert!(cancel.load(Ordering::Relaxed));
}

#[test]
fn cleanup_restore_jobs_release_running_state_on_all_terminal_paths() {
    let state = CleanupRestoreState::default();
    state
        .start_job_for_test("running")
        .expect("start running job");
    assert_eq!(
        state.status_for_test("running"),
        Some(CleanupRestoreJobStatus::Running)
    );
    assert!(state.start_job_for_test("second").is_err());
    assert!(state.cancel_job_for_test("running").is_ok());
    assert!(state.cancel_job_for_test("running").is_ok());
    state
        .finish_job_for_test("running", CleanupRestoreJobStatus::Canceled)
        .expect("finish canceled job");
    assert_eq!(
        state.status_for_test("running"),
        Some(CleanupRestoreJobStatus::Canceled)
    );
    assert!(state.cancel_job_for_test("running").is_ok());
    assert!(state.cancel_job_for_test("forged-storage-job").is_err());

    run_cleanup_restore_job_for_test(&state, "completed", CleanupRestoreTestOutcome::Completed)
        .expect("completed job");
    run_cleanup_restore_job_for_test(&state, "failed", CleanupRestoreTestOutcome::Failed)
        .expect("failed job");
    assert_eq!(
        state.status_for_test("failed"),
        Some(CleanupRestoreJobStatus::Failed)
    );
    assert!(
        run_cleanup_restore_job_for_test(&state, "panic", CleanupRestoreTestOutcome::Panic)
            .is_err()
    );
    assert_eq!(
        state.status_for_test("panic"),
        Some(CleanupRestoreJobStatus::Failed)
    );

    for index in 0..10 {
        let id = format!("retained-{index}");
        run_cleanup_restore_job_for_test(&state, &id, CleanupRestoreTestOutcome::Completed)
            .expect("retained job");
    }
    assert!(state.status_for_test("retained-0").is_none());
    assert_eq!(
        state.status_for_test("retained-9"),
        Some(CleanupRestoreJobStatus::Completed)
    );
}

#[test]
fn cleanup_restore_cancellation_requires_the_main_window_label() {
    assert!(is_main_window_label_for_test("main"));
    assert!(!is_main_window_label_for_test("secondary"));
}

fn test_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let counter = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::current_dir()
        .expect("current test directory")
        .join("target")
        .join("storage-analyzer-tests")
        .join(format!(
            "zen-canvas-storage-analyzer-test-{}-{nonce}-{counter}",
            std::process::id()
        ));
    fs::create_dir_all(&dir).expect("test dir");
    dir
}

fn current_temp_test_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let counter = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "zen-canvas-current-temp-test-{}-{nonce}-{counter}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("current temp test dir");
    dir
}

fn test_db_path() -> PathBuf {
    test_dir().join("zen-canvas-storage-cleanup.sqlite3")
}

fn write_file(path: &Path, size: usize) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent dir");
    }
    fs::write(path, vec![b'x'; size]).expect("write file");
}

#[cfg(windows)]
fn create_directory_symlink_for_test(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(unix)]
fn create_directory_symlink_for_test(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn remove_directory_symlink_for_test(link: &Path) -> std::io::Result<()> {
    fs::remove_dir(link)
}

#[cfg(unix)]
fn remove_directory_symlink_for_test(link: &Path) -> std::io::Result<()> {
    fs::remove_file(link)
}

fn cleanup_candidate_root(label: &str, count: usize) -> PathBuf {
    let root = test_dir();
    for index in 0..count {
        write_file(
            &root
                .join(format!("{label}-{index}"))
                .join("node_modules")
                .join("package")
                .join("index.js"),
            128,
        );
    }
    root
}

fn completed_cleanup_job(state: &StorageCleanupState, label: &str) -> (String, StorageCandidate) {
    let (job_id, candidates) = completed_cleanup_job_with_candidates(state, label, 1);
    (job_id, candidates[0].clone())
}

fn completed_cleanup_job_with_candidates(
    state: &StorageCleanupState,
    label: &str,
    count: usize,
) -> (String, Vec<StorageCandidate>) {
    let root = cleanup_candidate_root(label, count);
    let job_id = start_storage_cleanup_scan_for_test(vec![root], state).expect("start cleanup job");
    let analysis = wait_for_completed_cleanup_job(state, &job_id);
    assert_eq!(analysis.candidates.len(), count);
    (job_id, analysis.candidates)
}

fn wait_for_completed_cleanup_job(
    state: &StorageCleanupState,
    job_id: &str,
) -> zen_canvas_tauri::storage_analyzer::StorageAnalysis {
    for _ in 0..200 {
        let status = get_storage_cleanup_scan_status_for_test(job_id, state).expect("job status");
        match status.status.as_str() {
            "completed" => return status.analysis.expect("completed analysis"),
            "failed" => panic!("cleanup job failed: {:?}", status.error),
            "cancelled" => panic!("cleanup job was cancelled"),
            _ => thread::sleep(Duration::from_millis(5)),
        }
    }
    panic!("cleanup job did not complete: {job_id}");
}
