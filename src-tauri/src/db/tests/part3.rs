    #[test]
    fn operation_log_tables_exist_after_migration() {
        let db = Database::open(test_db_path()).expect("open test database");
        let conn = Connection::open(db.path()).expect("open migrated database");

        let table_count: i64 = conn
            .query_row(
                r#"
                SELECT COUNT(*)
                FROM sqlite_schema
                WHERE type = 'table'
                  AND name IN ('operation_batches', 'operation_logs')
                "#,
                [],
                |row| row.get(0),
            )
            .expect("count operation tables");

        assert_eq!(table_count, 2);
    }

    #[test]
    fn get_operation_logs_returns_empty_array_for_empty_table() {
        let db = Database::open(test_db_path()).expect("open test database");

        let logs = db.get_operation_logs(Some(500)).expect("operation logs");

        assert!(logs.is_empty());
    }

    #[test]
    fn save_operation_logs_persists_success_log() {
        let db = Database::open(test_db_path()).expect("open test database");
        let log = operation_log("log-success", "batch-success", "success");

        db.save_operation_logs("batch-success", std::slice::from_ref(&log))
            .expect("save operation logs");
        let logs = db.get_operation_logs(Some(10)).expect("operation logs");

        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].id, log.id);
        assert_eq!(logs[0].batch_id, "batch-success");
        assert_eq!(logs[0].status, "success");
        assert!(logs[0].can_restore);
        assert_eq!(operation_batch_status(&db, "batch-success"), "success");
    }

    #[test]
    fn save_operation_logs_persists_failed_log() {
        let db = Database::open(test_db_path()).expect("open test database");
        let mut log = operation_log("log-failed", "batch-failed", "failed");
        log.error_message = Some("Source file does not exist.".to_string());
        log.can_undo = false;
        log.can_restore = false;

        db.save_operation_logs("batch-failed", &[log.clone()])
            .expect("save operation logs");
        let logs = db.get_operation_logs(Some(10)).expect("operation logs");

        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status, "failed");
        assert_eq!(
            logs[0].error_message.as_deref(),
            Some("Source file does not exist.")
        );
        assert!(!logs[0].can_restore);
        assert_eq!(
            operation_batch_status(&db, "batch-failed"),
            "partial_failed"
        );
    }

    #[test]
    fn save_operation_logs_persists_skipped_log() {
        let db = Database::open(test_db_path()).expect("open test database");
        let mut log = operation_log("log-skipped", "batch-skipped", "skipped");
        log.error_message = Some("Operation is not executable.".to_string());
        log.can_undo = false;
        log.can_restore = false;

        db.save_operation_logs("batch-skipped", &[log.clone()])
            .expect("save operation logs");
        let logs = db.get_operation_logs(Some(10)).expect("operation logs");

        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status, "skipped");
        assert_eq!(
            logs[0].error_message.as_deref(),
            Some("Operation is not executable.")
        );
        assert!(!logs[0].can_restore);
        assert_eq!(operation_batch_status(&db, "batch-skipped"), "skipped");
    }

    #[test]
    fn update_operation_restore_logs_marks_restored_log() {
        let db = Database::open(test_db_path()).expect("open test database");
        let mut log = operation_log("log-restored", "batch-restored", "success");
        db.save_operation_logs("batch-restored", &[log.clone()])
            .expect("save operation logs");

        log.can_undo = false;
        log.can_restore = false;
        log.restored_at = Some("1900000000999".to_string());
        log.restore_status = "restored".to_string();
        log.restore_error = None;
        db.update_operation_restore_logs(&[log])
            .expect("update restore logs");

        let logs = db.get_operation_logs(Some(10)).expect("operation logs");
        assert_eq!(logs[0].restore_status, "restored");
        assert!(!logs[0].can_restore);
        assert!(!logs[0].can_undo);
        assert_eq!(logs[0].restored_at.as_deref(), Some("1900000000999"));
        assert!(logs[0].restore_error.is_none());
    }

    #[test]
    fn update_operation_restore_logs_marks_failed_log() {
        let db = Database::open(test_db_path()).expect("open test database");
        let mut log = operation_log("log-restore-failed", "batch-restore-failed", "success");
        db.save_operation_logs("batch-restore-failed", &[log.clone()])
            .expect("save operation logs");

        log.restore_status = "failed".to_string();
        log.restore_error = Some("Target file already exists.".to_string());
        db.update_operation_restore_logs(&[log])
            .expect("update restore logs");

        let logs = db.get_operation_logs(Some(10)).expect("operation logs");
        assert_eq!(logs[0].restore_status, "failed");
        assert_eq!(
            logs[0].restore_error.as_deref(),
            Some("Target file already exists.")
        );
    }

    #[test]
    fn update_operation_restore_logs_marks_unavailable_log() {
        let db = Database::open(test_db_path()).expect("open test database");
        let mut log = operation_log("log-unavailable", "batch-unavailable", "skipped");
        log.can_undo = false;
        log.can_restore = false;
        db.save_operation_logs("batch-unavailable", &[log.clone()])
            .expect("save operation logs");

        log.restore_status = "unavailable".to_string();
        log.restore_error = Some("Only successful operations can be restored.".to_string());
        db.update_operation_restore_logs(&[log])
            .expect("update restore logs");

        let logs = db.get_operation_logs(Some(10)).expect("operation logs");
        assert_eq!(logs[0].restore_status, "unavailable");
        assert!(!logs[0].can_restore);
        assert_eq!(
            logs[0].restore_error.as_deref(),
            Some("Only successful operations can be restored.")
        );
    }

    #[test]
    fn restore_claim_journal_is_independent_and_terminal_manual_review_is_not_requeued() {
        let db = Database::open(test_db_path()).expect("open test database");
        let mut log = operation_log("log-restore-claim", "batch-restore-claim", "success");
        db.save_operation_logs("batch-restore-claim", std::slice::from_ref(&log))
            .expect("save restore claim log");

        log.restore_claim_path = Some("C:/fixture/.zen-canvas-claim-uuid".to_string());
        log.restore_claim_created_at = Some("1900000000000".to_string());
        log.restore_claim_platform_file_id = Some("platform-file".to_string());
        log.restore_claim_full_hash = Some("full-hash".to_string());
        db.prepare_operation_restores(std::slice::from_ref(&log))
            .expect("prepare restore claim journal");

        let pending = db
            .get_pending_restore_logs()
            .expect("read pending restore claim")
            .into_iter()
            .find(|item| item.id == log.id)
            .expect("pending restore claim row");
        assert_eq!(pending.restore_phase, "prepared");
        assert_eq!(
            pending.restore_claim_path.as_deref(),
            Some("C:/fixture/.zen-canvas-claim-uuid")
        );
        assert_eq!(pending.operation_phase, "completed");

        let mut manual = pending;
        manual.status = "manual_review".to_string();
        manual.restore_status = "manual_review".to_string();
        manual.restore_phase = "source_claimed".to_string();
        db.finalize_operation_restore_outcome(std::slice::from_ref(&manual))
            .expect("persist source-claimed manual review");
        assert!(db
            .get_pending_restore_logs()
            .expect("manual source-claimed restore is not retried")
            .is_empty());

        manual.restore_phase = "completed".to_string();
        db.finalize_operation_restore_outcome(std::slice::from_ref(&manual))
            .expect("persist final-transaction manual review");
        assert_eq!(db.get_pending_restore_logs().expect("requeue final transaction").len(), 1);
    }

    #[test]
    fn ordinary_restore_finalization_upserts_missing_index_row_and_fts() {
        let db = Database::open(test_db_path()).expect("open test database");
        let root = test_dir();
        let source = root.join("restore-source.txt");
        let target = root.join("restore-target.txt");
        fs::write(&source, "restore index upsert").expect("write source");

        let source_path = normalized_test_path(&source);
        let target_path = normalized_test_path(&target);
        let restored_path = source_path.clone();
        let mut log = operation_log("log-restore-index-upsert", "batch-restore-index", "success");
        log.source_path = source_path.clone();
        log.target_path = target_path.clone();
        log.path_before = source_path;
        log.path_after = target_path.clone();
        log.name_before = "restore-source.txt".to_string();
        log.name_after = "restore-target.txt".to_string();
        log.new_name = "restore-target.txt".to_string();
        let identity = crate::file_ops::file_identity_fingerprint(&source)
            .expect("capture restore identity");
        log.source_size = Some(identity.size);
        log.source_modified_ns = identity.modified_ns.map(|value| value.to_string());
        log.source_platform_file_id = identity.platform_file_id;
        log.source_quick_hash = identity.quick_hash;
        log.source_full_hash = identity.full_hash;
        db.save_operation_logs("batch-restore-index", std::slice::from_ref(&log))
            .expect("save restore index log");

        log.restore_status = "restored".to_string();
        log.restore_phase = "completed".to_string();
        log.restored_at = Some("1900000000123".to_string());
        db.finalize_successful_operation_restore(&log)
            .expect("finalize restore with missing index row");

        let conn = Connection::open(db.path()).expect("open finalized database");
        let indexed: (String, String, i64, i64) = conn
            .query_row(
                "SELECT id, path, size, is_stale FROM files WHERE path = ?1",
                params![restored_path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("restored file index row");
        assert_eq!(indexed.0, indexed.1);
        assert_eq!(indexed.2, 20);
        assert_eq!(indexed.3, 0);

        let fts_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM files_fts WHERE name = ?1",
                params!["restore-source.txt"],
                |row| row.get(0),
            )
            .expect("restored file FTS row");
        assert_eq!(fts_count, 1);

        let restored_log = db
            .get_operation_logs(Some(10))
            .expect("read finalized restore log")
            .into_iter()
            .find(|item| item.id == log.id)
            .expect("finalized restore log");
        assert_eq!(restored_log.status, "success");
        assert_eq!(restored_log.restore_status, "restored");
        assert_eq!(restored_log.restore_phase, "completed");
        assert!(!restored_log.can_restore);
    }

    #[test]
    fn retained_restore_keeps_source_uid_and_user_tag_on_the_same_object() {
        let db = Database::open(test_db_path()).expect("open retained restore database");
        let root = test_dir();
        let source = root.join("retained-before.txt");
        let target = root.join("retained-after.txt");
        fs::write(&source, "retained source payload").expect("write retained source");
        let source_path = normalized_test_path(&source);
        let target_path = normalized_test_path(&target);
        db.insert_file(crate::db::InsertFileRequest {
            id: source_path.clone(),
            path: source_path.clone(),
            name: "retained-before.txt".to_string(),
            extension: "txt".to_string(),
            size: 22,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("seed a preexisting legacy row");
        let legacy_tag = db
            .create_user_tag(CreateUserTagRequest {
                display_name: "Unassigned pre-upgrade tag".to_string(),
                color_token: "neutral".to_string(),
            })
            .expect("create legacy tag");
        db.mutate_file_user_tags(MutateFileUserTagsRequest {
            selection: LibrarySelectionV1::Explicit {
                file_ids: vec![source_path.clone()],
            },
            tag_ids: vec![legacy_tag.id.clone()],
            operation: UserTagMutationOperation::Add,
            expected_count: Some(1),
        })
        .expect("tag legacy row before first observation");
        let mut settings = crate::settings::get_app_settings(&db).expect("read app settings");
        settings.default_scan_folders = vec![crate::settings::ScanRootSetting {
            id: "retained-restore-root".to_string(),
            path: normalized_test_path(&root),
            label: "Retained restore root".to_string(),
            enabled: true,
            created_at: "2026-10-15T00:00:00.000Z".to_string(),
        }];
        crate::settings::save_app_settings(&db, &settings).expect("save retained restore root");
        db.sync_file_library_watcher_roots(&settings.default_scan_folders)
            .expect("sync retained restore root");
        let root_id = db
            .list_watcher_root_configs()
            .expect("list retained restore roots")
            .into_iter()
            .find(|config| config.path == normalized_test_path(&root))
            .expect("managed retained restore root")
            .id;

        let initial_revision = db
            .begin_watcher_revision(&root_id)
            .expect("begin retained source watcher revision")
            .expect("enabled retained source root")
            .watcher_revision;
        db.apply_watcher_exact_mutations(
            &root_id,
            initial_revision,
            std::slice::from_ref(&source_path),
            &std::collections::HashSet::new(),
        )
        .expect("publish retained source identity");
        let original_uid: String = Connection::open(db.path())
            .expect("open retained source identity database")
            .query_row(
                "SELECT source_uid FROM library_source_root_claims WHERE root_id = ?1 AND observed_path = ?2 AND claim_state = 'present'",
                params![root_id, source_path],
                |row| row.get(0),
            )
            .expect("read retained source uid");
        let current_file_id: String = Connection::open(db.path())
            .expect("open current neutral row database")
            .query_row(
                "SELECT id FROM files WHERE path = ?1 AND is_stale = 0",
                params![source_path],
                |row| row.get(0),
            )
            .expect("read current neutral row id");
        assert_ne!(current_file_id, source_path);
        let legacy_tag_count_on_current: i64 = Connection::open(db.path())
            .expect("open current tag isolation database")
            .query_row(
                "SELECT COUNT(*) FROM file_user_tags WHERE file_id = ?1 AND tag_id = ?2",
                params![current_file_id, legacy_tag.id],
                |row| row.get(0),
            )
            .expect("check old tag is not inherited");
        assert_eq!(legacy_tag_count_on_current, 0);
        let tag = db
            .create_user_tag(CreateUserTagRequest {
                display_name: "Retained restore tag".to_string(),
                color_token: "green".to_string(),
            })
            .expect("create retained restore tag");
        db.mutate_file_user_tags(MutateFileUserTagsRequest {
            selection: LibrarySelectionV1::Explicit {
                file_ids: vec![current_file_id.clone()],
            },
            tag_ids: vec![tag.id.clone()],
            operation: UserTagMutationOperation::Add,
            expected_count: Some(1),
        })
        .expect("tag retained source");

        let identity = crate::file_ops::file_identity_fingerprint(&source)
            .expect("capture retained source identity");
        fs::rename(&source, &target).expect("move source into operation target");
        assert!(db
            .update_file_after_successful_operation(
                &current_file_id,
                &source_path,
                &target_path,
                "retained-after.txt",
            )
            .expect("publish operation move")
        );
        let mut log = operation_log(
            "log-retained-restore",
            "batch-retained-restore",
            "success",
        );
        log.source_path = source_path.clone();
        log.target_path = target_path.clone();
        log.path_before = source_path.clone();
        log.path_after = target_path.clone();
        log.name_before = "retained-before.txt".to_string();
        log.name_after = "retained-after.txt".to_string();
        log.new_name = log.name_after.clone();
        log.source_size = Some(identity.size);
        log.source_modified_ns = identity.modified_ns.map(|value| value.to_string());
        log.source_platform_file_id = identity.platform_file_id;
        log.source_platform_volume_id = identity.platform_volume_id;
        log.source_quick_hash = identity.quick_hash;
        log.source_full_hash = identity.full_hash;
        db.save_operation_logs(&log.batch_id, std::slice::from_ref(&log))
            .expect("save retained restore operation journal");

        fs::rename(&target, &source).expect("restore the same physical source");
        log.restore_status = "restored".to_string();
        log.restore_phase = "completed".to_string();
        log.restored_at = Some("1900000000123".to_string());
        db.finalize_successful_operation_restore(&log)
            .expect("finalize retained source restore");

        let conn = Connection::open(db.path()).expect("inspect retained restore state");
        let (restored_uid, claim_state): (String, String) = conn
            .query_row(
                "SELECT claim.source_uid, claim.claim_state FROM library_source_root_claims AS claim WHERE claim.root_id = ?1 AND claim.observed_path = ?2 ORDER BY claim.root_epoch DESC LIMIT 1",
                params![root_id, source_path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read restored root claim");
        let restored_tags: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_user_tags WHERE file_id = ?1 AND tag_id = ?2",
                params![current_file_id, tag.id],
                |row| row.get(0),
            )
            .expect("count restored user tags");
        let retained_legacy_tags: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_user_tags WHERE file_id = ?1 AND tag_id = ?2",
                params![source_path, legacy_tag.id],
                |row| row.get(0),
            )
            .expect("count quarantined legacy tags");
        let restored_file: (String, i64) = conn
            .query_row(
                "SELECT id, is_stale FROM files WHERE path = ?1",
                params![source_path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read restored file row");
        assert_eq!(restored_uid, original_uid);
        assert_eq!(claim_state, "present");
        assert_eq!(restored_tags, 1);
        assert_eq!(retained_legacy_tags, 1);
        assert_eq!(restored_file, (current_file_id, 0));
    }

    #[test]
    fn migrated_legacy_path_upsert_rejects_replacement_before_managed_scan_and_keeps_facts_isolated() {
        let db_path = test_db_path();
        let db = Database::open(&db_path).expect("create schema 41 fixture");
        let root = test_dir();
        let source = root.join("legacy-before.txt");
        let target = root.join("legacy-after.txt");
        fs::write(&source, "replacement-physical-A").expect("write managed file");
        let source_path = normalized_test_path(&source);
        let target_path = normalized_test_path(&target);
        let metadata = fs::metadata(&source).expect("read managed file metadata");
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);
        db.insert_file(InsertFileRequest {
            id: source_path.clone(),
            path: source_path.clone(),
            name: "legacy-before.txt".to_string(),
            extension: "txt".to_string(),
            size: metadata.len() as i64,
            mtime,
            ctime: mtime,
            is_dir: false,
            state_code: 0,
        })
        .expect("seed pre-migration legacy row");
        let legacy_tag = db
            .create_user_tag(CreateUserTagRequest {
                display_name: "Migrated legacy tag".to_string(),
                color_token: "neutral".to_string(),
            })
            .expect("create historical tag");
        db.mutate_file_user_tags(MutateFileUserTagsRequest {
            selection: LibrarySelectionV1::Explicit {
                file_ids: vec![source_path.clone()],
            },
            tag_ids: vec![legacy_tag.id.clone()],
            operation: UserTagMutationOperation::Add,
            expected_count: Some(1),
        })
        .expect("attach tag before migration");

        let mut settings = crate::settings::get_app_settings(&db).expect("read app settings");
        settings.default_scan_folders = vec![crate::settings::ScanRootSetting {
            id: "migrated-legacy-operation-root".to_string(),
            path: normalized_test_path(&root),
            label: "Migrated legacy operation root".to_string(),
            enabled: true,
            created_at: "2026-10-16T00:00:00.000Z".to_string(),
        }];
        crate::settings::save_app_settings(&db, &settings).expect("save managed root setting");
        db.sync_file_library_watcher_roots(&settings.default_scan_folders)
            .expect("sync managed root");
        let root_id = db
            .list_watcher_root_configs()
            .expect("list managed roots")
            .into_iter()
            .find(|config| config.path == normalized_test_path(&root))
            .expect("find migrated legacy root")
            .id;

        drop(db);
        let schema39 = Connection::open(&db_path).expect("open historical schema fixture");
        crate::db::drop_source_writer_schema_for_test(&schema39);
        schema39
            .execute_batch("PRAGMA user_version = 39;")
            .expect("mark fixture as schema 39");
        drop(schema39);

        let db = Database::open(&db_path).expect("run production schema 39 to 41 migration");
        let root_text = crate::db::scan::normalize_scan_root_path(&root.to_string_lossy());
        let admission = db
            .admit_managed_scan(&crate::db::scan::ScanAdmissionOptions {
                request: crate::db::scan::ManagedScanRequest {
                    roots: vec![root_text],
                    request_key: Some("migrated-legacy-operation-scan".to_string()),
                    dedupe: false,
                },
                run_id_override: None,
            })
            .expect("admit the first post-migration managed scan");
        let claimed = db
            .claim_queued_scan_run(&admission.runs[0].id)
            .expect("claim post-migration managed scan");
        let entry = InsertFileRequest {
            id: source_path.clone(),
            path: source_path.clone(),
            name: "legacy-before.txt".to_string(),
            extension: "txt".to_string(),
            size: fs::metadata(&source)
                .expect("read file size before managed scan")
                .len() as i64,
            mtime,
            ctime: mtime,
            is_dir: false,
            state_code: 0,
        };
        let persisted = db
            .persist_scan_batch(
                &claimed.dto.id,
                claimed.dto.revision,
                claimed.root_revision,
                claimed.session_revision,
                &crate::db::scan::ScanBatchInput {
                    entries: std::slice::from_ref(&entry),
                    errors: &[],
                    scanned_files: 1,
                    scanned_directories: 0,
                    processed_bytes: entry.size,
                    warnings: 0,
                },
            )
            .expect("publish migrated physical file through Managed Scan");
        let reconciled = db
            .reconcile_missing(
                &persisted.dto.id,
                persisted.dto.revision,
                persisted.root_revision,
                persisted.session_revision,
            )
            .expect("reconcile post-migration managed scan");
        db.finalize_scan_run(
            &reconciled.dto.id,
            reconciled.dto.revision,
            reconciled.root_revision,
            reconciled.session_revision,
            &crate::db::scan::ScanFinalizeInput {
                terminal_status: "completed".to_string(),
                error_code: None,
                error_message: None,
                allow_stale_reconciliation: true,
                rule_recovery_succeeded: false,
            },
        )
        .expect("finalize post-migration managed scan");

        let conn = Connection::open(db.path()).expect("inspect migrated rows");
        let current_id: String = conn
            .query_row(
                "SELECT id FROM files WHERE path = ?1 AND is_stale = 0",
                params![source_path],
                |row| row.get(0),
            )
            .expect("read current neutral row");
        assert_ne!(current_id, source_path);
        let current_claims: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM library_source_root_claims \
                 WHERE root_id = ?1 AND observed_path = ?2 AND claim_state = 'present'",
                params![root_id, source_path],
                |row| row.get(0),
            )
            .expect("read current managed source claim");
        assert_eq!(current_claims, 1);
        let archived_row: (String, i64) = conn
            .query_row(
                "SELECT path, is_stale FROM files WHERE id = ?1",
                params![source_path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read archived historical row");
        assert!(archived_row.0.starts_with("zen-canvas://legacy-unassigned/"));
        assert_eq!(archived_row.1, 1);
        let retained_tag_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_user_tags WHERE file_id = ?1 AND tag_id = ?2",
                params![source_path, legacy_tag.id],
                |row| row.get(0),
            )
            .expect("count retained historical tag");
        assert_eq!(retained_tag_count, 1);
        drop(conn);

        // This is the real filesystem path upsert. Its request ID is the old
        // path-based primary key, while that key now belongs to the archived
        // tagged row and the active browse row has a SourceUID ID.
        assert_eq!(
            crate::db::upsert_files_by_paths_for_db(&db, std::slice::from_ref(&source_path))
                .expect("path upsert must target the active neutral row"),
            1
        );
        let conn = Connection::open(db.path()).expect("inspect path upsert result");
        let active_after_upsert: String = conn
            .query_row(
                "SELECT id FROM files WHERE path = ?1 AND is_stale = 0",
                params![source_path],
                |row| row.get(0),
            )
            .expect("read active row after path upsert");
        assert_eq!(active_after_upsert, current_id);
        let retained_tag_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_user_tags WHERE file_id = ?1 AND tag_id = ?2",
                params![source_path, legacy_tag.id],
                |row| row.get(0),
            )
            .expect("count tag after path upsert");
        assert_eq!(retained_tag_count, 1);
        drop(conn);

        let current_tag = db
            .create_user_tag(CreateUserTagRequest {
                display_name: "Current physical object tag".to_string(),
                color_token: "blue".to_string(),
            })
            .expect("create tag for the verified physical object");
        db.mutate_file_user_tags(MutateFileUserTagsRequest {
            selection: LibrarySelectionV1::Explicit {
                file_ids: vec![current_id.clone()],
            },
            tag_ids: vec![current_tag.id.clone()],
            operation: UserTagMutationOperation::Add,
            expected_count: Some(1),
        })
        .expect("attach a user fact to the current physical object");

        // A normal path Upsert remains valid when the same physical object is
        // still present, and it must leave that object's user fact attached.
        assert_eq!(
            crate::db::upsert_files_by_paths_for_db(&db, std::slice::from_ref(&source_path))
                .expect("same-object path upsert remains supported"),
            1
        );
        let conn = Connection::open(db.path()).expect("inspect same-object upsert");
        let current_tag_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_user_tags WHERE file_id = ?1 AND tag_id = ?2",
                params![current_id, current_tag.id],
                |row| row.get(0),
            )
            .expect("count current-object tag after same-object upsert");
        assert_eq!(current_tag_count, 1);
        drop(conn);

        let library = db
            .query_file_library_v2(crate::db::queries::library::FileQueryRequestV2 {
                version: 2,
                request_id: "migrated-legacy-path-upsert".to_string(),
                query: crate::db::queries::library::FileQuerySpecV2 {
                    scope: crate::db::queries::library::FileLibraryScopeV2::AllEnabledRoots,
                    text: None,
                    filters: Default::default(),
                    sort: Default::default(),
                },
                page_size: 50,
                cursor: None,
            })
            .expect("query migrated file through the active File Library reader");
        let visible_rows = library
            .files
            .iter()
            .filter(|file| file.id == current_id)
            .collect::<Vec<_>>();
        assert_eq!(visible_rows.len(), 1);
        assert_eq!(visible_rows[0].tag_count, 1);
        assert_eq!(visible_rows[0].tags.len(), 1);

        // Keep the old object alive outside the managed root so it cannot be
        // rediscovered at another path. Recreate the original path with a
        // different object that deliberately has the same size and mtime.
        let original_modified = fs::metadata(&source)
            .expect("read original file metadata")
            .modified()
            .expect("read original file modification time");
        let parked_source = root.with_file_name(format!(
            "{}-parked-replaced-object",
            root.file_name()
                .expect("test root has a final path component")
                .to_string_lossy()
        ));
        fs::rename(&source, &parked_source).expect("park original physical object outside root");
        fs::write(&source, "replacement-physical-B").expect("write replacement physical object");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&source)
            .expect("open replacement to preserve modification time")
            .set_times(std::fs::FileTimes::new().set_modified(original_modified))
            .expect("preserve same-path replacement modification time");
        let replacement_metadata = fs::metadata(&source).expect("read replacement metadata");
        assert_eq!(replacement_metadata.len(), metadata.len());
        assert_eq!(
            replacement_metadata
                .modified()
                .expect("read replacement modification time"),
            original_modified
        );

        let conn = Connection::open(db.path()).expect("snapshot before rejected path upsert");
        let file_before: (String, String, String, i64, i64, i64, i64) = conn
            .query_row(
                "SELECT id, path, name, size, mtime, is_stale, last_seen_at FROM files WHERE id = ?1",
                params![current_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .expect("read current row before rejected upsert");
        let attribution_before: (Option<String>, String, Option<String>, i64) = conn
            .query_row(
                "SELECT source_uid, claim_state, evidence_digest, decision_revision FROM library_legacy_attributions WHERE legacy_file_id = ?1 ORDER BY decision_revision DESC, rowid DESC LIMIT 1",
                params![current_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("read current SourceUID attribution before rejected upsert");
        let root_epoch_before: i64 = conn
            .query_row(
                "SELECT epoch FROM library_root_epoch_state WHERE root_id = ?1",
                params![root_id],
                |row| row.get(0),
            )
            .expect("read root epoch before rejected upsert");
        let root_claim_before: (String, i64, String, i64, i64) = conn
            .query_row(
                "SELECT source_uid, root_epoch, claim_state, owner_revision, observation_revision FROM library_source_root_claims WHERE root_id = ?1 AND observed_path = ?2 ORDER BY root_epoch DESC, rowid DESC LIMIT 1",
                params![root_id, source_path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .expect("read source root claim before rejected upsert");
        drop(conn);

        let error = crate::db::upsert_files_by_paths_for_db(
            &db,
            std::slice::from_ref(&source_path),
        )
        .expect_err("same-path replacement must not inherit the old SourceUID facts");
        assert!(error.to_string().contains("physical identity is missing, changed, or unverified"));
        assert!(error.to_string().contains("Managed Scan/Watcher reconciliation is required"));

        let conn = Connection::open(db.path()).expect("inspect rejected upsert state");
        let file_after: (String, String, String, i64, i64, i64, i64) = conn
            .query_row(
                "SELECT id, path, name, size, mtime, is_stale, last_seen_at FROM files WHERE id = ?1",
                params![current_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .expect("read current row after rejected upsert");
        let attribution_after: (Option<String>, String, Option<String>, i64) = conn
            .query_row(
                "SELECT source_uid, claim_state, evidence_digest, decision_revision FROM library_legacy_attributions WHERE legacy_file_id = ?1 ORDER BY decision_revision DESC, rowid DESC LIMIT 1",
                params![current_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("read current SourceUID attribution after rejected upsert");
        let root_epoch_after: i64 = conn
            .query_row(
                "SELECT epoch FROM library_root_epoch_state WHERE root_id = ?1",
                params![root_id],
                |row| row.get(0),
            )
            .expect("read root epoch after rejected upsert");
        let root_claim_after: (String, i64, String, i64, i64) = conn
            .query_row(
                "SELECT source_uid, root_epoch, claim_state, owner_revision, observation_revision FROM library_source_root_claims WHERE root_id = ?1 AND observed_path = ?2 ORDER BY root_epoch DESC, rowid DESC LIMIT 1",
                params![root_id, source_path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .expect("read source root claim after rejected upsert");
        let current_tag_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_user_tags WHERE file_id = ?1 AND tag_id = ?2",
                params![current_id, current_tag.id],
                |row| row.get(0),
            )
            .expect("count retained current-object tag");
        let legacy_tag_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_user_tags WHERE file_id = ?1 AND tag_id = ?2",
                params![source_path, legacy_tag.id],
                |row| row.get(0),
            )
            .expect("count retained historical legacy tag");
        drop(conn);
        assert_eq!(file_after, file_before);
        assert_eq!(attribution_after, attribution_before);
        assert_eq!(root_epoch_after, root_epoch_before);
        assert_eq!(root_claim_after, root_claim_before);
        assert_eq!(current_tag_count, 1);
        assert_eq!(legacy_tag_count, 1);

        // Existing Managed Scan publishes replacement B as a new neutral row.
        let replacement_mtime = replacement_metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);
        let replacement_entry = InsertFileRequest {
            id: source_path.clone(),
            path: source_path.clone(),
            name: "legacy-before.txt".to_string(),
            extension: "txt".to_string(),
            size: replacement_metadata.len() as i64,
            mtime: replacement_mtime,
            ctime: replacement_metadata
                .created()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(replacement_mtime),
            is_dir: false,
            state_code: 0,
        };
        let replacement_admission = db
            .admit_managed_scan(&crate::db::scan::ScanAdmissionOptions {
                request: crate::db::scan::ManagedScanRequest {
                    roots: vec![crate::db::scan::normalize_scan_root_path(
                        &root.to_string_lossy(),
                    )],
                    request_key: Some("migrated-legacy-replacement-scan".to_string()),
                    dedupe: false,
                },
                run_id_override: None,
            })
            .expect("admit Managed Scan for replacement object");
        let replacement_claimed = db
            .claim_queued_scan_run(&replacement_admission.runs[0].id)
            .expect("claim Managed Scan for replacement object");
        let replacement_persisted = db
            .persist_scan_batch(
                &replacement_claimed.dto.id,
                replacement_claimed.dto.revision,
                replacement_claimed.root_revision,
                replacement_claimed.session_revision,
                &crate::db::scan::ScanBatchInput {
                    entries: std::slice::from_ref(&replacement_entry),
                    errors: &[],
                    scanned_files: 1,
                    scanned_directories: 0,
                    processed_bytes: replacement_entry.size,
                    warnings: 0,
                },
            )
            .expect("publish replacement object through Managed Scan");
        let replacement_reconciled = db
            .reconcile_missing(
                &replacement_persisted.dto.id,
                replacement_persisted.dto.revision,
                replacement_persisted.root_revision,
                replacement_persisted.session_revision,
            )
            .expect("reconcile replacement Managed Scan");
        db.finalize_scan_run(
            &replacement_reconciled.dto.id,
            replacement_reconciled.dto.revision,
            replacement_reconciled.root_revision,
            replacement_reconciled.session_revision,
            &crate::db::scan::ScanFinalizeInput {
                terminal_status: "completed".to_string(),
                error_code: None,
                error_message: None,
                allow_stale_reconciliation: true,
                rule_recovery_succeeded: false,
            },
        )
        .expect("finalize replacement Managed Scan");

        let conn = Connection::open(db.path()).expect("inspect managed replacement rows");
        let replacement_id: String = conn
            .query_row(
                "SELECT id FROM files WHERE path = ?1 AND is_stale = 0",
                params![source_path],
                |row| row.get(0),
            )
            .expect("read active replacement row");
        let archived_original: (String, i64) = conn
            .query_row(
                "SELECT path, is_stale FROM files WHERE id = ?1",
                params![current_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read archived original SourceUID row");
        let replacement_tag_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_user_tags WHERE file_id = ?1 AND tag_id = ?2",
                params![replacement_id, current_tag.id],
                |row| row.get(0),
            )
            .expect("check replacement did not inherit original object's tag");
        let original_tag_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_user_tags WHERE file_id = ?1 AND tag_id = ?2",
                params![current_id, current_tag.id],
                |row| row.get(0),
            )
            .expect("check original object's tag stayed with its row");
        let replacement_source_uid: String = conn
            .query_row(
                "SELECT source_uid FROM library_legacy_attributions WHERE legacy_file_id = ?1 ORDER BY decision_revision DESC, rowid DESC LIMIT 1",
                params![replacement_id],
                |row| row.get(0),
            )
            .expect("read replacement SourceUID attribution");
        let original_source_uid: String = conn
            .query_row(
                "SELECT source_uid FROM library_legacy_attributions WHERE legacy_file_id = ?1 ORDER BY decision_revision DESC, rowid DESC LIMIT 1",
                params![current_id],
                |row| row.get(0),
            )
            .expect("read original SourceUID attribution");
        drop(conn);
        assert_ne!(replacement_id, current_id);
        assert!(archived_original.0.starts_with("zen-canvas://legacy-unassigned/"));
        assert_eq!(archived_original.1, 1);
        assert_eq!(replacement_tag_count, 0);
        assert_eq!(original_tag_count, 1);
        assert_ne!(replacement_source_uid, original_source_uid);

        let replacement_library = db
            .query_file_library_v2(crate::db::queries::library::FileQueryRequestV2 {
                version: 2,
                request_id: "migrated-legacy-replacement-visible".to_string(),
                query: crate::db::queries::library::FileQuerySpecV2 {
                    scope: crate::db::queries::library::FileLibraryScopeV2::AllEnabledRoots,
                    text: None,
                    filters: Default::default(),
                    sort: Default::default(),
                },
                page_size: 50,
                cursor: None,
            })
            .expect("query replacement through the active File Library reader");
        let replacement_rows = replacement_library
            .files
            .iter()
            .filter(|file| file.id == replacement_id)
            .collect::<Vec<_>>();
        assert_eq!(replacement_rows.len(), 1);
        assert_eq!(replacement_rows[0].tag_count, 0);
        assert!(replacement_rows[0].tags.is_empty());

        fs::rename(&source, &target).expect("perform actual filesystem rename");
        assert!(db
            .update_file_after_successful_operation(
                &source_path,
                &source_path,
                &target_path,
                "legacy-after.txt",
            )
            .expect("postflight resolves the active path row, not the archived ID"));

        let conn = Connection::open(db.path()).expect("inspect rename postflight rows");
        let moved_row: (String, i64) = conn
            .query_row(
                "SELECT id, is_stale FROM files WHERE path = ?1",
                params![target_path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read renamed active row");
        assert_eq!(moved_row, (replacement_id.clone(), 0));
        let retained_tag_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_user_tags WHERE file_id = ?1 AND tag_id = ?2",
                params![source_path, legacy_tag.id],
                |row| row.get(0),
            )
            .expect("count original tag after move");
        assert_eq!(retained_tag_count, 1);
        let inherited_tag_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_user_tags WHERE file_id = ?1 AND tag_id = ?2",
                params![current_id, legacy_tag.id],
                |row| row.get(0),
            )
            .expect("ensure old tag did not move with the current object");
        assert_eq!(inherited_tag_count, 0);
        let original_tag_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_user_tags WHERE file_id = ?1 AND tag_id = ?2",
                params![current_id, current_tag.id],
                |row| row.get(0),
            )
            .expect("ensure original object's tag remains on its archived row");
        assert_eq!(original_tag_count, 1);
        let replacement_tag_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_user_tags WHERE file_id = ?1 AND tag_id = ?2",
                params![replacement_id, current_tag.id],
                |row| row.get(0),
            )
            .expect("ensure original object's tag was not transferred to replacement");
        assert_eq!(replacement_tag_count, 0);
        drop(conn);

        let error = db
            .insert_file(InsertFileRequest {
                id: source_path.clone(),
                path: source_path,
                name: "legacy-before.txt".to_string(),
                extension: "txt".to_string(),
                size: 22,
                mtime,
                ctime: mtime,
                is_dir: false,
                state_code: 0,
            })
            .expect_err("an archived ID cannot be revived after its active row moved");
        assert!(error
            .to_string()
            .contains("Path upsert cannot revive a parked legacy row"));
    }

    #[test]
    fn replacement_restore_finalization_reconciles_both_indexed_objects() {
        let db = Database::open(test_db_path()).expect("open replacement restore database");
        let root = test_dir();
        let source = root.join("replacement-source.txt");
        let target = root.join("replacement-target.txt");
        fs::write(&source, "replacement source payload").expect("write replacement source");
        fs::write(&target, "replacement target payload").expect("write replacement target");

        let source_path = normalized_test_path(&source);
        let target_path = normalized_test_path(&target);
        let source_identity = crate::file_ops::file_identity_fingerprint(&source)
            .expect("capture replacement source identity");
        let target_identity = crate::file_ops::file_identity_fingerprint(&target)
            .expect("capture replacement target identity");
        let mut log = operation_log(
            "log-replacement-restore-index",
            "batch-replacement-restore-index",
            "success",
        );
        log.operation_type = "replace".to_string();
        log.source_path = source_path.clone();
        log.target_path = target_path.clone();
        log.path_before = source_path.clone();
        log.path_after = target_path.clone();
        log.name_before = "replacement-source.txt".to_string();
        log.name_after = "replacement-target.txt".to_string();
        log.new_name = log.name_after.clone();
        log.source_size = Some(source_identity.size);
        log.source_modified_ns = source_identity.modified_ns.map(|value| value.to_string());
        log.source_platform_file_id = source_identity.platform_file_id.clone();
        log.source_platform_volume_id = source_identity.platform_volume_id.clone();
        log.source_quick_hash = source_identity.quick_hash.clone();
        log.source_full_hash = source_identity.full_hash.clone();
        log.target_platform_file_id = target_identity.platform_file_id;
        log.target_platform_volume_id = target_identity.platform_volume_id;
        log.target_full_hash = target_identity.full_hash;
        log.restore_status = "restored".to_string();
        log.restore_phase = "completed".to_string();
        log.restored_at = Some("1900000000123".to_string());
        db.save_operation_logs(&log.batch_id, std::slice::from_ref(&log))
            .expect("save replacement restore log");

        let source_metadata = fs::metadata(&source).expect("replacement source metadata");
        db.insert_file(InsertFileRequest {
            id: target_path.clone(),
            path: target_path.clone(),
            name: log.name_after.clone(),
            extension: "txt".to_string(),
            size: source_metadata.len() as i64,
            mtime: source_metadata
                .modified()
                .expect("replacement source mtime")
                .duration_since(UNIX_EPOCH)
                .expect("replacement source mtime epoch")
                .as_secs() as i64,
            ctime: 0,
            is_dir: false,
            state_code: 0,
        })
        .expect("index replacement source at target path");

        db.finalize_successful_operation_restore(&log)
            .expect("finalize replacement restore index");

        let conn = Connection::open(db.path()).expect("open replacement index database");
        let rows: Vec<(String, String, i64)> = {
            let mut stmt = conn
                .prepare("SELECT path, name, size FROM files WHERE path IN (?1, ?2) ORDER BY path")
                .expect("prepare replacement index query");
            stmt.query_map(params![source_path, target_path], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .expect("query replacement index rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect replacement index rows")
        };
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].1, "replacement-source.txt");
        assert_eq!(rows[1].1, "replacement-target.txt");
        assert_eq!(rows[0].2, source_identity.size as i64);
        assert_eq!(rows[1].2, target_identity.size as i64);
    }

    #[test]
    fn ordinary_restore_finalization_rolls_back_index_when_journal_update_fails() {
        let db_path = test_db_path();
        let db = Database::open(&db_path).expect("open test database");
        let root = test_dir();
        let source = root.join("restore-atomic-source.txt");
        let target = root.join("restore-atomic-target.txt");
        fs::write(&source, "restore atomic boundary").expect("write source");

        let source_path = normalized_test_path(&source);
        let target_path = normalized_test_path(&target);
        let mut log = operation_log("log-restore-atomic", "batch-restore-atomic", "success");
        log.source_path = source_path.clone();
        log.target_path = target_path.clone();
        log.path_before = source_path;
        log.path_after = target_path;
        log.name_before = "restore-atomic-source.txt".to_string();
        log.name_after = "restore-atomic-target.txt".to_string();
        log.new_name = "restore-atomic-target.txt".to_string();
        let identity = crate::file_ops::file_identity_fingerprint(&source)
            .expect("capture atomic restore identity");
        log.source_size = Some(identity.size);
        log.source_modified_ns = identity.modified_ns.map(|value| value.to_string());
        log.source_platform_file_id = identity.platform_file_id;
        log.source_quick_hash = identity.quick_hash;
        log.source_full_hash = identity.full_hash;
        db.save_operation_logs("batch-restore-atomic", std::slice::from_ref(&log))
            .expect("save atomic restore log");

        let conn = Connection::open(&db_path).expect("open trigger database");
        conn.execute_batch(
            r#"
            CREATE TRIGGER reject_restore_final_journal
            BEFORE UPDATE OF restore_status ON operation_logs
            WHEN NEW.restore_status = 'restored'
            BEGIN
                SELECT RAISE(ABORT, 'injected restore final journal failure');
            END;
            "#,
        )
        .expect("install restore finalization trigger");
        drop(conn);

        log.restore_status = "restored".to_string();
        log.restore_phase = "completed".to_string();
        log.restored_at = Some("1900000000123".to_string());
        assert!(db.finalize_successful_operation_restore(&log).is_err());

        let conn = Connection::open(db.path()).expect("open rollback database");
        let indexed_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
            .expect("count rolled back index rows");
        assert_eq!(indexed_count, 0);
        let restore_status: String = conn
            .query_row(
                "SELECT restore_status FROM operation_logs WHERE id = ?1",
                params![log.id],
                |row| row.get(0),
            )
            .expect("read rolled back restore journal");
        assert_eq!(restore_status, "not_restored");
    }

    #[test]
    fn ordinary_restore_finalization_resolves_source_target_rows_without_broad_delete() {
        for case in ["source-only", "target-only", "same-row", "distinct-rows"] {
            let db = Database::open(test_db_path()).expect("open restore matrix database");
            let root = test_dir();
            let source = root.join(format!("{case}-before.txt"));
            let target = root.join(format!("{case}-after.txt"));
            fs::write(&source, format!("restore matrix {case}")).expect("write matrix source");
            let source_path = normalized_test_path(&source);
            let target_path = normalized_test_path(&target);
            let mut log = operation_log(
                &format!("log-restore-{case}"),
                &format!("batch-restore-{case}"),
                "success",
            );
            log.source_path = source_path.clone();
            log.target_path = target_path.clone();
            log.path_before = source_path.clone();
            log.path_after = target_path.clone();
            log.name_before = source.file_name().unwrap().to_string_lossy().into_owned();
            log.name_after = target.file_name().unwrap().to_string_lossy().into_owned();
            log.new_name = log.name_after.clone();
            let identity = crate::file_ops::file_identity_fingerprint(&source)
                .expect("capture matrix restore identity");
            log.source_size = Some(identity.size);
            log.source_modified_ns = identity.modified_ns.map(|value| value.to_string());
            log.source_platform_file_id = identity.platform_file_id;
            log.source_quick_hash = identity.quick_hash;
            log.source_full_hash = identity.full_hash;
            log.restore_status = "restored".to_string();
            log.restore_phase = "completed".to_string();
            log.restored_at = Some("1900000000123".to_string());
            db.save_operation_logs(&log.batch_id, std::slice::from_ref(&log))
                .expect("save matrix restore log");

            let size = fs::metadata(&source).expect("matrix metadata").len() as i64;
            let mtime = fs::metadata(&source)
                .expect("matrix metadata")
                .modified()
                .expect("matrix mtime")
                .duration_since(UNIX_EPOCH)
                .expect("matrix mtime epoch")
                .as_secs() as i64;
            let insert = |id: &str, path: &str, name: &str| {
                db.insert_file(InsertFileRequest {
                    id: id.to_string(),
                    path: path.to_string(),
                    name: name.to_string(),
                    extension: "txt".to_string(),
                    size,
                    mtime,
                    ctime: 0,
                    is_dir: false,
                    state_code: 0,
                })
                .expect("insert matrix index row");
            };
            match case {
                "source-only" => insert("source-row", &target_path, &log.name_after),
                "target-only" | "same-row" => insert("target-row", &source_path, &log.name_before),
                "distinct-rows" => {
                    insert("target-row", &source_path, &log.name_before);
                    insert("source-row", &target_path, &log.name_after);
                    let conn = Connection::open(db.path()).expect("open matrix metadata database");
                    conn.execute(
                        r#"
                        UPDATE files
                        SET purpose = 'SourcePurpose',
                            lifecycle = 'SourceLifecycle',
                            context = 'SourceContext',
                            risk_level = 'Sensitive',
                            suggested_action = 'Move',
                            suggested_target_path = 'source-target',
                            suggested_name = 'source-suggested.txt',
                            confidence = 0.93,
                            classification_reason = 'source reason',
                            classification_status = 'classified',
                            matched_rules = '["source-rule"]',
                            requires_confirmation = 1,
                            content_hash = 'source-content-hash',
                            last_classified_at = 123,
                            classified_rule_version = 'source-rule-version'
                        WHERE id = 'source-row'
                        "#,
                        [],
                    )
                    .expect("set source business metadata");
                }
                _ => unreachable!(),
            }

            db.finalize_successful_operation_restore(&log)
                .expect("finalize matrix restore");
            let conn = Connection::open(db.path()).expect("open finalized matrix database");
            let row_count: i64 = conn
                .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
                .expect("count matrix index rows");
            let fts_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM files_fts WHERE name = ?1",
                    params![log.name_before],
                    |row| row.get(0),
                )
                .expect("count matrix FTS rows");
            assert_eq!(row_count, 1, "row matrix case: {case}");
            assert_eq!(fts_count, 1, "FTS matrix case: {case}");
            if case == "distinct-rows" {
                #[derive(Debug)]
                struct PreservedSourceMetadata {
                    purpose: String,
                    lifecycle: String,
                    context: String,
                    risk_level: String,
                    suggested_action: String,
                    suggested_target_path: String,
                    suggested_name: String,
                    confidence: f64,
                    classification_reason: String,
                    classification_status: String,
                    matched_rules: String,
                    requires_confirmation: i64,
                    classified_rule_version: String,
                }
                let metadata: PreservedSourceMetadata = conn
                    .query_row(
                        r#"
                        SELECT purpose, lifecycle, context, risk_level, suggested_action,
                               suggested_target_path, suggested_name, confidence,
                               classification_reason, classification_status, matched_rules,
                               requires_confirmation, classified_rule_version
                        FROM files WHERE path = ?1
                        "#,
                        params![source_path],
                        |row| {
                            Ok(PreservedSourceMetadata {
                                purpose: row.get(0)?,
                                lifecycle: row.get(1)?,
                                context: row.get(2)?,
                                risk_level: row.get(3)?,
                                suggested_action: row.get(4)?,
                                suggested_target_path: row.get(5)?,
                                suggested_name: row.get(6)?,
                                confidence: row.get(7)?,
                                classification_reason: row.get(8)?,
                                classification_status: row.get(9)?,
                                matched_rules: row.get(10)?,
                                requires_confirmation: row.get(11)?,
                                classified_rule_version: row.get(12)?,
                            })
                        },
                    )
                    .expect("read preserved source metadata");
                assert_eq!(metadata.purpose, "SourcePurpose");
                assert_eq!(metadata.lifecycle, "SourceLifecycle");
                assert_eq!(metadata.context, "SourceContext");
                assert_eq!(metadata.risk_level, "Sensitive");
                assert_eq!(metadata.suggested_action, "Move");
                assert_eq!(metadata.suggested_target_path, "source-target");
                assert_eq!(metadata.suggested_name, "source-suggested.txt");
                assert_eq!(metadata.confidence, 0.93);
                assert_eq!(metadata.classification_reason, "source reason");
                assert_eq!(metadata.classification_status, "classified");
                assert_eq!(metadata.matched_rules, "[\"source-rule\"]");
                assert_eq!(metadata.requires_confirmation, 1);
                assert_eq!(metadata.classified_rule_version, "source-rule-version");
                let watcher_rows: i64 = conn
                    .query_row("SELECT COUNT(*) FROM files WHERE id = 'target-row'", [], |row| {
                        row.get(0)
                    })
                    .expect("count deleted watcher target row");
                assert_eq!(watcher_rows, 0);
            }
        }
    }

    #[test]
    fn ordinary_restore_reconciliation_rejects_reappeared_source_and_preserves_claim() {
        let db = Database::open(test_db_path()).expect("open reappeared-source database");
        let root = test_dir();
        let source = root.join("reappeared-before.txt");
        let after = root.join("reappeared-after.txt");
        fs::write(&source, "original restore payload").expect("write restore target");
        fs::write(&after, "replacement source payload").expect("write reappeared source");

        let source_identity = crate::file_ops::file_identity_fingerprint(&source)
            .expect("capture reappeared target identity");
        let mut log = operation_log(
            "log-restore-source-reappeared",
            "batch-restore-source-reappeared",
            "success",
        );
        log.source_path = normalized_test_path(&source);
        log.target_path = normalized_test_path(&after);
        log.path_before = log.source_path.clone();
        log.path_after = log.target_path.clone();
        log.name_before = "reappeared-before.txt".to_string();
        log.name_after = "reappeared-after.txt".to_string();
        log.new_name = log.name_after.clone();
        log.source_size = Some(source_identity.size);
        log.source_modified_ns = source_identity
            .modified_ns
            .map(|value| value.to_string());
        log.source_platform_file_id = source_identity.platform_file_id.clone();
        log.source_platform_volume_id = source_identity.platform_volume_id.clone();
        log.source_quick_hash = source_identity.quick_hash.clone();
        log.source_full_hash = source_identity.full_hash.clone();
        log.target_platform_file_id = source_identity.platform_file_id.clone();
        log.target_platform_volume_id = source_identity.platform_volume_id.clone();
        log.target_full_hash = source_identity.full_hash.clone();
        log.restore_status = "pending".to_string();
        log.restore_phase = "target_committed".to_string();
        log.restore_claim_path = Some(normalized_test_path(
            &root.join(".zen-canvas-claim-reappeared"),
        ));
        log.restore_claim_created_at = Some("1900000000123".to_string());
        log.restore_claim_platform_file_id = source_identity.platform_file_id.clone();
        log.restore_claim_platform_volume_id = source_identity.platform_volume_id.clone();
        log.restore_claim_full_hash = source_identity.full_hash.clone();
        db.save_operation_logs(&log.batch_id, std::slice::from_ref(&log))
            .expect("save reappeared restore log");

        let insert_index_row = |id: &str, path: &Path| {
            let metadata = fs::metadata(path).expect("reappeared metadata");
            db.insert_file(InsertFileRequest {
                id: id.to_string(),
                path: normalized_test_path(path),
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                extension: "txt".to_string(),
                size: metadata.len() as i64,
                mtime: metadata
                    .modified()
                    .expect("reappeared mtime")
                    .duration_since(UNIX_EPOCH)
                    .expect("reappeared mtime epoch")
                    .as_secs() as i64,
                ctime: 0,
                is_dir: false,
                state_code: 0,
            })
            .expect("insert reappeared index row");
        };
        insert_index_row("reappeared-target-row", &source);
        insert_index_row("reappeared-source-row", &after);

        assert_eq!(
            crate::file_ops::reconcile_pending_operation_journal(&db)
                .expect("reconcile reappeared restore"),
            1
        );
        let reconciled = db.get_operation_logs(Some(10)).expect("read reappeared log");
        let reconciled = reconciled
            .iter()
            .find(|candidate| candidate.id == log.id)
            .expect("reconciled reappeared log");
        assert_eq!(reconciled.restore_status, "manual_review");
        assert_eq!(reconciled.restore_phase, "source_cleanup_pending");
        assert_eq!(
            reconciled.restore_error.as_deref().unwrap_or_default(),
            "restore_source_path_reappeared: restore source path reappeared after the target commit; preserve the claim and review both paths"
        );
        assert_eq!(reconciled.restore_claim_path, log.restore_claim_path);
        assert_eq!(
            reconciled.restore_claim_platform_file_id,
            log.restore_claim_platform_file_id
        );
        assert_eq!(reconciled.restore_claim_full_hash, log.restore_claim_full_hash);

        let conn = Connection::open(db.path()).expect("open reappeared index database");
        let file_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
            .expect("count reappeared index rows");
        assert_eq!(file_count, 2);
        assert_eq!(fs::read_to_string(&after).expect("read reappeared source"), "replacement source payload");
    }

    #[test]
    fn ordinary_restore_finalization_rejects_target_identity_mismatch_before_index_write() {
        let db = Database::open(test_db_path()).expect("open identity-mismatch database");
        let root = test_dir();
        let source = root.join("identity-mismatch-before.txt");
        let after = root.join("identity-mismatch-after.txt");
        fs::write(&source, "original identity payload").expect("write identity target");
        let identity = crate::file_ops::file_identity_fingerprint(&source)
            .expect("capture identity-mismatch target");

        let mut log = operation_log(
            "log-restore-identity-mismatch",
            "batch-restore-identity-mismatch",
            "success",
        );
        log.source_path = normalized_test_path(&source);
        log.target_path = normalized_test_path(&after);
        log.path_before = log.source_path.clone();
        log.path_after = log.target_path.clone();
        log.name_before = "identity-mismatch-before.txt".to_string();
        log.name_after = "identity-mismatch-after.txt".to_string();
        log.new_name = log.name_after.clone();
        log.source_size = Some(identity.size);
        log.source_modified_ns = identity.modified_ns.map(|value| value.to_string());
        log.source_platform_file_id = identity.platform_file_id.clone();
        log.source_platform_volume_id = identity.platform_volume_id.clone();
        log.source_quick_hash = identity.quick_hash.clone();
        log.source_full_hash = identity.full_hash.clone();
        log.target_platform_file_id = identity.platform_file_id;
        log.target_platform_volume_id = identity.platform_volume_id;
        log.target_full_hash = identity.full_hash;
        db.save_operation_logs(&log.batch_id, std::slice::from_ref(&log))
            .expect("save identity-mismatch log");
        log.restore_status = "restored".to_string();
        log.restore_phase = "completed".to_string();
        log.restored_at = Some("1900000000123".to_string());

        fs::write(&source, "replacement identity payload").expect("replace identity target");
        let error = db
            .finalize_successful_operation_restore(&log)
            .expect_err("identity mismatch must block finalization")
            .to_string();
        assert!(error.starts_with("target_committed_identity_mismatch:"));

        let conn = Connection::open(db.path()).expect("open identity-mismatch database");
        let file_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
            .expect("count identity-mismatch index rows");
        assert_eq!(file_count, 0);
        let restore_status: String = conn
            .query_row(
                "SELECT restore_status FROM operation_logs WHERE id = ?1",
                params![log.id],
                |row| row.get(0),
            )
            .expect("read identity-mismatch journal");
        assert_eq!(restore_status, "not_restored");
    }

    #[test]
    fn build_fts_query_quotes_terms_without_breaking_chinese_or_punctuation() {
        let query = build_fts_query("项目\"报告 final-v1.pdf").expect("query");

        assert_eq!(query, "\"项目\"\"报告\" AND \"final-v1.pdf\"");
    }
