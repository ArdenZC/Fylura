    fn enable_legacy_builtin_rules(db: &Database) {
        let settings = AppSettings {
            use_legacy_builtin_classification_rules: true,
            ..AppSettings::default()
        };
        save_app_settings(db, &settings).expect("enable legacy builtin classification rules");
    }

    fn insert_test_file(
        db: &Database,
        id: &str,
        name: &str,
        extension: &str,
        size: i64,
        mtime: i64,
    ) {
        db.insert_file(InsertFileRequest {
            id: id.to_string(),
            path: format!("/test/virtual/documents/{name}"),
            name: name.to_string(),
            extension: extension.to_string(),
            size,
            mtime,
            ctime: 0,
            is_dir: false,
            state_code: 0,
        })
        .expect("insert file");
    }

    fn insert_test_file_at_path(
        db: &Database,
        id: &str,
        path: &str,
        name: &str,
        extension: &str,
        size: i64,
        mtime: i64,
    ) {
        db.insert_file(InsertFileRequest {
            id: id.to_string(),
            path: path.to_string(),
            name: name.to_string(),
            extension: extension.to_string(),
            size,
            mtime,
            ctime: 0,
            is_dir: false,
            state_code: 0,
        })
        .expect("insert scoped file");
    }

    fn publish_test_duplicate_group(db: &Database, file_ids: &[&str], full_hash: &str) {
        assert!(file_ids.len() > 1, "duplicate groups need at least two files");
        let conn = Connection::open(db.path()).expect("open migrated database");
        let run_id = format!("test-dedupe-run-{full_hash}");
        let group_id = format!("test-duplicate-group-{full_hash}");
        conn.execute(
            r#"
            INSERT INTO dedupe_runs (
                id, request_key, request_attempt, scope_json, scope_hash,
                scope_snapshot_json, scope_snapshot_hash, status, phase,
                revision, finished_at, created_at, updated_at
            ) VALUES (?1, ?2, 1, '{"kind":"test"}', ?3, '[]', ?4,
                      'completed', 'completed', 1, 1, 1, 1)
            "#,
            params![run_id, format!("test-request-{full_hash}"), full_hash, full_hash],
        )
        .expect("insert test dedupe run");

        let first_size: i64 = conn
            .query_row(
                "SELECT size FROM files WHERE id = ?1",
                params![file_ids[0]],
                |row| row.get(0),
            )
            .expect("read first duplicate size");
        let member_count = i64::try_from(file_ids.len()).expect("member count fits i64");
        let reclaimable = first_size * (member_count - 1);
        conn.execute(
            r#"
            INSERT INTO duplicate_groups (
                id, size_each, full_hash, full_hash_algorithm, full_hash_version,
                member_count, physical_copy_count, hardlink_alias_count,
                exact_reclaimable_bytes, potential_reclaimable_bytes,
                reclaimable_confidence, status, last_built_run_id, revision,
                created_at, updated_at, last_verified_at
            ) VALUES (?1, ?2, ?3, 'blake3', 1, ?4, ?4, 0, ?5, ?5,
                      'exact', 'active', ?6, 1, 1, 1, 1)
            "#,
            params![group_id, first_size, full_hash, member_count, reclaimable, run_id],
        )
        .expect("insert test duplicate group");

        for file_id in file_ids {
            let (path, size, mtime): (String, i64, i64) = conn
                .query_row(
                    "SELECT path, size, mtime FROM files WHERE id = ?1",
                    params![file_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .expect("read duplicate member");
            conn.execute(
                r#"
                INSERT INTO duplicate_group_members (
                    group_id, file_id, path_snapshot, physical_key,
                    identity_status, is_hardlink_alias, size, modified_ns, verified_at
                ) VALUES (?1, ?2, ?3, NULL, 'path_only', 0, ?4, ?5, 1)
                "#,
                params![group_id, file_id, path, size, mtime],
            )
            .expect("insert test duplicate member");
        }
    }

    fn operation_log(id: &str, batch_id: &str, status: &str) -> OperationLogDto {
        let success = status == "success";
        OperationLogDto {
            id: id.to_string(),
            batch_id: batch_id.to_string(),
            operation_type: "move".to_string(),
            source_path: "/tmp/source.txt".to_string(),
            target_path: "/tmp/target.txt".to_string(),
            old_name: "source.txt".to_string(),
            new_name: "target.txt".to_string(),
            status: status.to_string(),
            error_message: None,
            created_at: "1900000000123".to_string(),
            can_undo: success,
            path_before: "/tmp/source.txt".to_string(),
            path_after: "/tmp/target.txt".to_string(),
            name_before: "source.txt".to_string(),
            name_after: "target.txt".to_string(),
            can_restore: success,
            restored_at: None,
            restore_status: "not_restored".to_string(),
            restore_error: None,
            source_size: None,
            source_modified_ns: None,
            source_platform_file_id: None,
            source_platform_volume_id: None,
            source_quick_hash: None,
            source_full_hash: None,
            target_platform_file_id: None,
            target_platform_volume_id: None,
            target_full_hash: None,
            source_claim_path: None,
            operation_phase: "completed".to_string(),
            claim_created_at: None,
            claim_platform_file_id: None,
            claim_platform_volume_id: None,
            claim_full_hash: None,
            restore_claim_path: None,
            restore_phase: "idle".to_string(),
            restore_claim_created_at: None,
            restore_claim_platform_file_id: None,
            restore_claim_platform_volume_id: None,
            restore_claim_full_hash: None,
        }
    }

    fn operation_batch_status(db: &Database, batch_id: &str) -> String {
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.query_row(
            "SELECT status FROM operation_batches WHERE id = ?1",
            params![batch_id],
            |row| row.get(0),
        )
        .expect("operation batch status")
    }

    fn stale_state(db: &Database, path: &str) -> Option<(bool, bool)> {
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.query_row(
            "SELECT is_stale, last_seen_at FROM files WHERE path = ?1",
            params![path],
            |row| Ok((row.get::<_, i64>(0)? != 0, row.get::<_, i64>(1)? > 0)),
        )
        .optional()
        .expect("stale state")
    }

    fn file_classification(db: &Database, path: &str) -> Option<(String, String, bool)> {
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.query_row(
            "SELECT purpose, lifecycle, is_stale FROM files WHERE path = ?1",
            params![path],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? != 0,
                ))
            },
        )
        .optional()
        .expect("file classification")
    }

    fn set_file_lifecycle(db: &Database, path: &str, lifecycle: &str) {
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.execute(
            "UPDATE files SET lifecycle = ?2 WHERE path = ?1",
            params![path, lifecycle],
        )
        .expect("set lifecycle");
    }

    fn set_file_review_state(
        db: &Database,
        path: &str,
        lifecycle: &str,
        suggested_action: &str,
        requires_confirmation: bool,
    ) {
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.execute(
            r#"
            UPDATE files
            SET lifecycle = ?2,
                suggested_action = ?3,
                requires_confirmation = ?4
            WHERE path = ?1
            "#,
            params![
                path,
                lifecycle,
                suggested_action,
                if requires_confirmation { 1 } else { 0 }
            ],
        )
        .expect("set file review state");
    }

    #[allow(clippy::too_many_arguments)]
    fn set_file_operation_suggestion(
        db: &Database,
        path: &str,
        suggested_action: &str,
        suggested_target_path: &str,
        suggested_name: &str,
        risk_level: &str,
        confidence: f64,
        requires_confirmation: bool,
    ) {
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.execute(
            r#"
            UPDATE files
            SET suggested_action = ?2,
                suggested_target_path = ?3,
                suggested_name = ?4,
                risk_level = ?5,
                confidence = ?6,
                requires_confirmation = ?7,
                classification_reason = 'test suggestion',
                classification_status = 'classified'
            WHERE path = ?1
            "#,
            params![
                path,
                suggested_action,
                suggested_target_path,
                suggested_name,
                risk_level,
                confidence,
                if requires_confirmation { 1 } else { 0 }
            ],
        )
        .expect("set file operation suggestion");
    }

    fn set_file_mtime(db: &Database, path: &str, mtime: i64) {
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.execute(
            "UPDATE files SET mtime = ?2 WHERE path = ?1",
            params![path, mtime],
        )
        .expect("set mtime");
    }

    fn set_file_size(db: &Database, path: &str, size: i64) {
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.execute(
            "UPDATE files SET size = ?2 WHERE path = ?1",
            params![path, size],
        )
        .expect("set size");
    }

    fn classification_fingerprint(db: &Database, path: &str) -> Option<(i64, String, i64, i64)> {
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.query_row(
            r#"
            SELECT last_classified_at,
                   classified_rule_version,
                   last_classified_mtime,
                   last_classified_size
            FROM files
            WHERE path = ?1
            "#,
            params![path],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .expect("classification fingerprint")
    }

    fn name_contains_rule(id: &str, name: &str, purpose: &str) -> Rule {
        Rule {
            id: id.to_string(),
            name: name.to_string(),
            source: "user".into(),
            enabled: true,
            priority: 200.0,
            weight: 100.0,
            root_operator: "OR".into(),
            groups: vec![RuleConditionGroup {
                id: format!("{id}-group"),
                operator: "AND".into(),
                conditions: vec![RuleCondition {
                    id: format!("{id}-condition"),
                    field: "name".into(),
                    operator: "contains".into(),
                    value: Value::String("special".to_string()),
                }],
            }],
            action: RuleAction {
                purpose: Some(purpose.into()),
                lifecycle: Some("Inbox".into()),
                context: Some("D1 Test".to_string()),
                risk_level: Some("Normal".into()),
                suggested_action: Some("Keep".into()),
                target_template: None,
                rename_template: None,
            },
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn user_rule_for_persistence(id: &str, name: &str, priority: f64) -> Rule {
        Rule {
            id: id.to_string(),
            name: name.to_string(),
            source: "user".into(),
            enabled: true,
            priority,
            weight: 42.5,
            root_operator: "AND".into(),
            groups: vec![RuleConditionGroup {
                id: format!("{id}-group"),
                operator: "AND".into(),
                conditions: vec![RuleCondition {
                    id: format!("{id}-condition"),
                    field: "extension".into(),
                    operator: "equals".into(),
                    value: Value::String("pdf".to_string()),
                }],
            }],
            action: RuleAction {
                purpose: Some("Document".into()),
                lifecycle: Some("Archive".into()),
                context: Some("Persistence Test".to_string()),
                risk_level: Some("Normal".into()),
                suggested_action: Some("Move".into()),
                target_template: Some("{home}/Archive".to_string()),
                rename_template: Some("{name}".to_string()),
            },
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn assert_rule_matches(actual: &Rule, expected: &Rule) {
        assert_eq!(actual.id, expected.id);
        assert_eq!(actual.name, expected.name);
        assert_eq!(actual.source.as_str(), "user");
        assert_eq!(actual.enabled, expected.enabled);
        assert_eq!(actual.priority, expected.priority);
        assert_eq!(actual.weight, expected.weight);
        assert_eq!(actual.root_operator, expected.root_operator);
        assert_eq!(
            serde_json::to_value(&actual.groups).expect("actual groups json"),
            serde_json::to_value(&expected.groups).expect("expected groups json")
        );
        assert_eq!(
            serde_json::to_value(&actual.action).expect("actual action json"),
            serde_json::to_value(&expected.action).expect("expected action json")
        );
        assert!(!actual.created_at.is_empty());
        assert!(!actual.updated_at.is_empty());
    }

    fn insert_system_rule_row(db: &Database, id: &str) {
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.execute(
            r#"
            INSERT INTO rules (
                id,
                name,
                source,
                enabled,
                priority,
                weight,
                root_operator,
                groups_json,
                action_json,
                created_at,
                updated_at
            )
            VALUES (?1, 'System Rule', 'system', 1, 1000, 100, 'AND', '[]', '{}', '2026-06-21T00:00:00Z', '2026-06-21T00:00:00Z')
            "#,
            params![id],
        )
        .expect("insert system rule row");
    }

    fn rule_source_by_id(db: &Database, id: &str) -> Option<String> {
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.query_row(
            "SELECT source FROM rules WHERE id = ?1",
            params![id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .expect("rule source")
    }

    fn test_db_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let counter = TEST_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "zen-canvas-db-test-{}-{nonce}-{counter}.sqlite3",
            std::process::id()
        ))
    }

    fn test_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let counter = TEST_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = crate::path_filter::managed_test_temp_dir().join(format!(
            "zen-canvas-db-file-test-{}-{nonce}-{counter}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("test dir");
        dir
    }

    #[test]
    fn temporary_database_paths_are_unique_across_parallel_tests() {
        let paths = (0..64)
            .map(|_| std::thread::spawn(test_db_path))
            .map(|handle| handle.join().expect("join path generator"))
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(paths.len(), 64);
    }

    fn normalized_test_path(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }
