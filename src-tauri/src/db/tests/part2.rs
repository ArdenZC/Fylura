    #[test]
    fn run_search_index_optimize_reports_success() {
        let db = Database::open(test_db_path()).expect("open test database");

        let report = run_search_index_optimize("scan_complete", &db);

        assert_eq!(report.trigger, "scan_complete");
        assert!(report.success);
        assert!(report.duration_ms <= 60_000);
        assert!(report.error.is_none());
    }

    #[test]
    fn stats_excludes_stale_files() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(
            &db,
            "file-report",
            "report.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );

        let before = db.get_stats_summary().expect("stats before");
        db.remove_files_by_paths(&["/test/virtual/documents/report.pdf".to_string()])
            .expect("mark stale");
        let after = db.get_stats_summary().expect("stats after");

        assert_eq!(before.total_files, 1);
        assert_eq!(before.total_size, 2_048);
        assert_eq!(after.total_files, 0);
        assert_eq!(after.total_size, 0);
        assert_eq!(
            stale_state(&db, "/test/virtual/documents/report.pdf"),
            Some((true, true))
        );
    }

    #[test]
    fn execute_rules_ignores_stale_files() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(
            &db,
            "file-resume-stale",
            "resume_2026.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );
        db.remove_files_by_paths(&["/test/virtual/documents/resume_2026.pdf".to_string()])
            .expect("mark stale");

        let summary = db
            .execute_rules_on_inbox(Vec::new())
            .expect("execute rules");
        let row = file_classification(&db, "/test/virtual/documents/resume_2026.pdf")
            .expect("stale file still exists");

        assert_eq!(summary.scanned, 0);
        assert_eq!(summary.updated, 0);
        assert_eq!(row, ("Unknown".to_string(), "Inbox".to_string(), true));
    }

    #[test]
    fn execute_rules_for_paths_classifies_only_target_paths() {
        let db = Database::open(test_db_path()).expect("open test database");
        enable_legacy_builtin_rules(&db);
        insert_test_file(
            &db,
            "file-resume-target",
            "resume_2026.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );
        insert_test_file(
            &db,
            "file-invoice-untouched",
            "invoice_apple.pdf",
            "pdf",
            2_048,
            1_900_000_001,
        );

        let summary = db
            .execute_rules_for_paths(
                &["/test/virtual/documents/resume_2026.pdf".to_string()],
                Vec::new(),
            )
            .expect("execute targeted rules");
        let target =
            file_classification(&db, "/test/virtual/documents/resume_2026.pdf").expect("target file");
        let untouched = file_classification(&db, "/test/virtual/documents/invoice_apple.pdf")
            .expect("untouched file");

        assert_eq!(summary.scanned, 1);
        assert_eq!(summary.updated, 1);
        assert_eq!(
            target,
            ("Career".to_string(), "Reference".to_string(), false)
        );
        assert_eq!(
            untouched,
            ("Unknown".to_string(), "Inbox".to_string(), false)
        );
    }

    #[test]
    fn unverified_neutral_source_rows_never_receive_automatic_classification() {
        let db = Database::open(test_db_path()).expect("open test database");
        enable_legacy_builtin_rules(&db);
        let path = "/test/virtual/documents/resume_2026.pdf";
        let file_id = "zen-canvas-source:source-uid:epoch:1";
        insert_test_file_at_path(
            &db,
            file_id,
            path,
            "resume_2026.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.execute(
            "UPDATE files SET suggested_action = 'Keep', requires_confirmation = 1, classification_status = 'unclassified', classification_reason = 'Legacy facts remain unassigned to this current filesystem entry.' WHERE id = ?1",
            params![file_id],
        )
        .expect("set neutral source defaults");
        drop(conn);

        let targeted = db
            .execute_rules_for_paths(&[path.to_string()], Vec::new())
            .expect("execute watcher targeted rules");
        let scoped = db
            .execute_rules_for_scope_with_mode(
                &LibraryScope::Roots {
                    roots: vec!["/test/virtual/documents".to_string()],
                },
                Vec::new(),
                RuleExecutionMode::AllChangedOrRuleChanged,
            )
            .expect("execute reconciliation rules");
        let inbox = db
            .execute_rules_on_inbox(Vec::new())
            .expect("execute inbox rules");

        assert_eq!(targeted.scanned, 0);
        assert_eq!(targeted.updated, 0);
        assert_eq!(scoped.scanned, 0);
        assert_eq!(scoped.updated, 0);
        assert_eq!(inbox.scanned, 0);
        assert_eq!(inbox.updated, 0);
        let (purpose, lifecycle, suggested_action, requires_confirmation, classification_status): (
            String,
            String,
            String,
            i64,
            String,
        ) = Connection::open(db.path())
            .expect("open migrated database")
            .query_row(
                "SELECT purpose, lifecycle, suggested_action, requires_confirmation, classification_status FROM files WHERE id = ?1",
                params![file_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .expect("neutral source row remains available");
        assert_eq!(purpose, "Unknown");
        assert_eq!(lifecycle, "Inbox");
        assert_eq!(suggested_action, "Keep");
        assert_eq!(requires_confirmation, 1);
        assert_eq!(classification_status, "unclassified");
    }

    #[test]
    fn execute_rules_for_scope_classifies_only_scoped_roots() {
        let db = Database::open(test_db_path()).expect("open test database");
        enable_legacy_builtin_rules(&db);
        insert_test_file_at_path(
            &db,
            "file-root-a-resume",
            "/tmp/root-a/resume_2026.pdf",
            "resume_2026.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );
        insert_test_file_at_path(
            &db,
            "file-root-b-invoice",
            "/tmp/root-b/invoice_apple.pdf",
            "invoice_apple.pdf",
            "pdf",
            4_096,
            1_900_000_001,
        );

        let summary = db
            .execute_rules_for_scope(
                &LibraryScope::Roots {
                    roots: vec!["/tmp/root-a".to_string()],
                },
                Vec::new(),
            )
            .expect("execute scoped rules");
        let root_a = file_classification(&db, "/tmp/root-a/resume_2026.pdf").expect("root a file");
        let root_b = file_classification(&db, "/tmp/root-b/invoice_apple.pdf").expect("root b file");

        assert_eq!(summary.scanned, 1);
        assert_eq!(summary.updated, 1);
        assert_eq!(
            root_a,
            ("Career".to_string(), "Reference".to_string(), false)
        );
        assert_eq!(root_b, ("Unknown".to_string(), "Inbox".to_string(), false));
    }

    #[test]
    fn execute_rules_for_scope_v2_loads_only_backend_enabled_catalog_rules() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file_at_path(
            &db,
            "v2-authoritative-file",
            "/tmp/v2-authoritative-root/report.pdf",
            "report.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.execute(
            "INSERT INTO scan_roots (
                id, normalized_path, display_name, source_kind, enabled,
                health_status, needs_reconciliation,
                watcher_rule_recovery_required, watcher_revision,
                watcher_applied_revision, created_at, updated_at
             ) VALUES (
                'v2-authoritative-root', '/tmp/v2-authoritative-root',
                'v2-authoritative-root', 'file_library', 1, 'healthy', 0,
                0, 0, 0, 1, 1
             )",
            [],
        )
        .expect("insert managed root");
        let created = db
            .create_user_rule_v2(CreateUserRuleV2Request {
                version: 2,
                request_id: "v2-authoritative-create".to_string(),
                expected_catalog_revision: 1,
                draft: RuleDraftV2 {
                    name: "PDF project rule".to_string(),
                    priority: 200.0,
                    weight: 90.0,
                    root_operator: "AND".to_string(),
                    groups: vec![RuleGroupDraftV2 {
                        operator: "AND".to_string(),
                        conditions: vec![RuleConditionDraftV2 {
                            field: "extension".to_string(),
                            operator: "equals".to_string(),
                            value: Value::String("pdf".to_string()),
                        }],
                    }],
                    action: RuleActionDraftV2 {
                        purpose: Some("Project".to_string()),
                        lifecycle: Some("Inbox".to_string()),
                        ..Default::default()
                    },
                },
            })
            .expect("create disabled V2 rule");
        assert!(!created.rule.rule.enabled);
        let disabled = db
            .execute_rules_for_scope_v2(ExecuteRulesForScopeV2Request {
                scope: FileLibraryScopeV2::Roots {
                    scan_root_ids: vec!["v2-authoritative-root".to_string()],
                },
                mode: RuleExecutionMode::AllChangedOrRuleChanged,
                expected_catalog_revision: created.catalog_revision,
                confirmed: true,
            })
            .expect("execute with disabled catalog rule");
        assert_eq!(disabled.catalog_revision, 2);
        assert_eq!(
            file_classification(&db, "/tmp/v2-authoritative-root/report.pdf")
                .expect("disabled classification")
                .0,
            "Unknown"
        );

        let enabled = db
            .set_user_rule_enabled_v2(SetUserRuleEnabledV2Request {
                rule_id: created.rule.rule.id.clone(),
                expected_rule_revision: created.rule.revision,
                expected_catalog_revision: created.catalog_revision,
                enabled: true,
            })
            .expect("enable V2 rule");
        let result = db
            .execute_rules_for_scope_v2(ExecuteRulesForScopeV2Request {
                scope: FileLibraryScopeV2::Roots {
                    scan_root_ids: vec!["v2-authoritative-root".to_string()],
                },
                mode: RuleExecutionMode::AllChangedOrRuleChanged,
                expected_catalog_revision: enabled.catalog_revision,
                confirmed: true,
            })
            .expect("execute backend-loaded enabled rule");
        assert_eq!(result.catalog_revision, 3);
        assert!(!result.classification_version.is_empty());
        assert_eq!(result.summary.updated, 1);
        assert_eq!(
            file_classification(&db, "/tmp/v2-authoritative-root/report.pdf")
                .expect("enabled classification")
                .0,
            "Project"
        );

        let stale = db
            .execute_rules_for_scope_v2(ExecuteRulesForScopeV2Request {
                scope: FileLibraryScopeV2::Roots {
                    scan_root_ids: vec!["v2-authoritative-root".to_string()],
                },
                mode: RuleExecutionMode::InboxOnly,
                expected_catalog_revision: 2,
                confirmed: true,
            })
            .expect_err("stale renderer catalog must fail closed");
        assert!(stale.to_string().contains("rule_catalog_revision_conflict"));
    }

    #[test]
    fn execute_rules_for_scope_uses_global_duplicate_flags() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file_at_path(
            &db,
            "duplicate-root-a",
            "/tmp/root-a/shared-copy.txt",
            "shared-copy.txt",
            "txt",
            2_048,
            1_900_000_000,
        );
        insert_test_file_at_path(
            &db,
            "duplicate-root-b",
            "/tmp/root-b/shared-copy.txt",
            "shared-copy.txt",
            "txt",
            2_048,
            1_900_000_001,
        );
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.execute(
            r#"
                    UPDATE files
                    SET content_hash = 'same-global-content'
                    WHERE id IN ('duplicate-root-a', 'duplicate-root-b')
                    "#,
            [],
        )
        .expect("set duplicate content hash");
        publish_test_duplicate_group(
            &db,
            &["duplicate-root-a", "duplicate-root-b"],
            "same-global-content",
        );
        let duplicate_rule = Rule {
            id: "global-duplicate-rule".to_string(),
            name: "Global Duplicate Rule".to_string(),
            source: "user".into(),
            enabled: true,
            priority: 500.0,
            weight: 100.0,
            root_operator: "AND".into(),
            groups: vec![RuleConditionGroup {
                id: "global-duplicate-rule-group".to_string(),
                operator: "AND".into(),
                conditions: vec![RuleCondition {
                    id: "global-duplicate-rule-condition".to_string(),
                    field: "is_duplicate".into(),
                    operator: "equals".into(),
                    value: Value::String("true".to_string()),
                }],
            }],
            action: RuleAction {
                purpose: Some("Duplicate Review".into()),
                lifecycle: Some("Inbox".into()),
                context: Some("D1 Duplicate".to_string()),
                risk_level: Some("Normal".into()),
                suggested_action: Some("Review".into()),
                target_template: None,
                rename_template: None,
            },
            created_at: String::new(),
            updated_at: String::new(),
        };

        let summary = db
            .execute_rules_for_scope_with_mode(
                &LibraryScope::Roots {
                    roots: vec!["/tmp/root-a".to_string()],
                },
                vec![duplicate_rule],
                RuleExecutionMode::AllChangedOrRuleChanged,
            )
            .expect("execute scoped rules");
        let root_a = file_classification(&db, "/tmp/root-a/shared-copy.txt").expect("root a file");
        let root_b = file_classification(&db, "/tmp/root-b/shared-copy.txt").expect("root b file");

        assert_eq!(summary.scanned, 1);
        assert_eq!(summary.updated, 1);
        assert_eq!(
            root_a,
            ("Duplicate Review".to_string(), "Inbox".to_string(), false)
        );
        assert_eq!(root_b, ("Unknown".to_string(), "Inbox".to_string(), false));
    }

    #[test]
    fn execute_rules_for_scope_inbox_only_skips_already_classified_files() {
        let db = Database::open(test_db_path()).expect("open test database");
        enable_legacy_builtin_rules(&db);
        insert_test_file_at_path(
            &db,
            "file-root-a-resume",
            "/tmp/root-a/resume_2026.pdf",
            "resume_2026.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );
        db.execute_rules_for_scope_with_mode(
            &LibraryScope::Roots {
                roots: vec!["/tmp/root-a".to_string()],
            },
            Vec::new(),
            RuleExecutionMode::InboxOnly,
        )
        .expect("initial scoped rules");
        let summary = db
            .execute_rules_for_scope_with_mode(
                &LibraryScope::Roots {
                    roots: vec!["/tmp/root-a".to_string()],
                },
                vec![name_contains_rule(
                    "special-resume-project",
                    "Resume Project",
                    "Project",
                )],
                RuleExecutionMode::InboxOnly,
            )
            .expect("inbox only rules");

        assert_eq!(summary.scanned, 0);
        assert_eq!(summary.updated, 0);
    }

    #[test]
    fn execute_rules_for_scope_all_changed_reclassifies_when_rule_version_changes() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file_at_path(
            &db,
            "file-root-a-special",
            "/tmp/root-a/special_project.txt",
            "special_project.txt",
            "txt",
            2_048,
            1_900_000_000,
        );
        db.execute_rules_for_scope_with_mode(
            &LibraryScope::Roots {
                roots: vec!["/tmp/root-a".to_string()],
            },
            vec![name_contains_rule(
                "special-project",
                "Special Project",
                "Project",
            )],
            RuleExecutionMode::AllChangedOrRuleChanged,
        )
        .expect("first scoped rules");
        let summary = db
            .execute_rules_for_scope_with_mode(
                &LibraryScope::Roots {
                    roots: vec!["/tmp/root-a".to_string()],
                },
                vec![name_contains_rule(
                    "special-career",
                    "Special Career",
                    "Career",
                )],
                RuleExecutionMode::AllChangedOrRuleChanged,
            )
            .expect("changed scoped rules");
        let row = file_classification(&db, "/tmp/root-a/special_project.txt").expect("classified file");

        assert_eq!(summary.scanned, 1);
        assert_eq!(summary.updated, 1);
        assert_eq!(row, ("Career".to_string(), "Inbox".to_string(), false));
    }

    #[test]
    fn rule_recovery_scope_preserves_user_correction() {
        let db = Database::open(test_db_path()).expect("open test database");
        let path = "/tmp/root-a/user-corrected.txt";
        insert_test_file_at_path(
            &db,
            "file-root-a-user-corrected",
            path,
            "user-corrected.txt",
            "txt",
            2_048,
            1_900_000_000,
        );
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.execute(
            r#"
            UPDATE files
            SET purpose = 'Manual Purpose',
                lifecycle = 'Inbox',
                classification_status = 'classified',
                matched_rules = '["user_correction"]',
                last_classified_at = 1,
                classified_rule_version = 'obsolete-rule-version'
            WHERE path = ?1
            "#,
            params![path],
        )
        .expect("persist user correction");

        let summary = db
            .execute_rules_for_scope_with_mode(
                &LibraryScope::Roots {
                    roots: vec!["/tmp/root-a".to_string()],
                },
                vec![name_contains_rule(
                    "recovery-rule",
                    "Recovery Rule",
                    "Recovered",
                )],
                RuleExecutionMode::AllChangedOrRuleChanged,
            )
            .expect("execute recovery rules");
        let row = file_classification(&db, path).expect("user-corrected file");

        assert_eq!(summary.scanned, 1);
        assert_eq!(summary.updated, 0);
        assert_eq!(summary.skipped, 1);
        assert_eq!(row, ("Manual Purpose".to_string(), "Inbox".to_string(), false));
    }

    #[test]
    fn execute_rules_for_scope_all_changed_skips_unchanged_files_with_same_rules() {
        let db = Database::open(test_db_path()).expect("open test database");
        let rules = vec![name_contains_rule(
            "special-project",
            "Special Project",
            "Project",
        )];
        insert_test_file_at_path(
            &db,
            "file-root-a-special",
            "/tmp/root-a/special_project.txt",
            "special_project.txt",
            "txt",
            2_048,
            1_900_000_000,
        );
        db.execute_rules_for_scope_with_mode(
            &LibraryScope::Roots {
                roots: vec!["/tmp/root-a".to_string()],
            },
            rules.clone(),
            RuleExecutionMode::AllChangedOrRuleChanged,
        )
        .expect("first scoped rules");
        let summary = db
            .execute_rules_for_scope_with_mode(
                &LibraryScope::Roots {
                    roots: vec!["/tmp/root-a".to_string()],
                },
                rules,
                RuleExecutionMode::AllChangedOrRuleChanged,
            )
            .expect("unchanged scoped rules");

        assert_eq!(summary.scanned, 1);
        assert_eq!(summary.updated, 0);
        assert_eq!(summary.skipped, 1);
    }

    #[test]
    fn execute_rules_for_paths_ignores_missing_paths() {
        let db = Database::open(test_db_path()).expect("open test database");

        let summary = db
            .execute_rules_for_paths(
                &["/test/virtual/documents/missing.pdf".to_string()],
                Vec::new(),
            )
            .expect("execute targeted rules");

        assert_eq!(summary.scanned, 0);
        assert_eq!(summary.updated, 0);
        assert_eq!(summary.needs_confirmation, 0);
    }

    #[test]
    fn execute_rules_for_paths_ignores_stale_files() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(
            &db,
            "file-resume-stale-target",
            "resume_2026.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );
        db.remove_files_by_paths(&["/test/virtual/documents/resume_2026.pdf".to_string()])
            .expect("mark stale");

        let summary = db
            .execute_rules_for_paths(
                &["/test/virtual/documents/resume_2026.pdf".to_string()],
                Vec::new(),
            )
            .expect("execute targeted rules");
        let row = file_classification(&db, "/test/virtual/documents/resume_2026.pdf")
            .expect("stale file still exists");

        assert_eq!(summary.scanned, 0);
        assert_eq!(summary.updated, 0);
        assert_eq!(row, ("Unknown".to_string(), "Inbox".to_string(), true));
    }

    #[test]
    fn execute_rules_for_paths_ignores_non_inbox_files() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(
            &db,
            "file-resume-archive",
            "resume_2026.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );
        set_file_lifecycle(&db, "/test/virtual/documents/resume_2026.pdf", "Archive");

        let summary = db
            .execute_rules_for_paths(
                &["/test/virtual/documents/resume_2026.pdf".to_string()],
                Vec::new(),
            )
            .expect("execute targeted rules");
        let row =
            file_classification(&db, "/test/virtual/documents/resume_2026.pdf").expect("archive file");

        assert_eq!(summary.scanned, 0);
        assert_eq!(summary.updated, 0);
        assert_eq!(row, ("Unknown".to_string(), "Archive".to_string(), false));
    }

    #[test]
    fn execute_rules_on_inbox_skips_unchanged_files() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(
            &db,
            "file-special",
            "special_plain.txt",
            "txt",
            2_048,
            1_900_000_000,
        );

        let rules = vec![name_contains_rule("special-rule", "Special", "Project")];
        let first = db
            .execute_rules_on_inbox(rules.clone())
            .expect("first rules");
        let second = db.execute_rules_on_inbox(rules).expect("second rules");

        assert_eq!(first.scanned, 1);
        assert_eq!(first.updated, 1);
        assert_eq!(first.skipped, 0);
        assert_eq!(second.scanned, 1);
        assert_eq!(second.updated, 0);
        assert_eq!(second.skipped, 1);
    }

    #[test]
    fn execute_rules_on_inbox_reclassifies_when_rule_changes() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(
            &db,
            "file-special",
            "special_project.txt",
            "txt",
            2_048,
            1_900_000_000,
        );

        let first = db
            .execute_rules_on_inbox(vec![name_contains_rule(
                "special-rule-a",
                "Special A",
                "Project",
            )])
            .expect("first rules");
        let first_fingerprint =
            classification_fingerprint(&db, "/test/virtual/documents/special_project.txt")
                .expect("first fingerprint");
        let second = db
            .execute_rules_on_inbox(vec![name_contains_rule(
                "special-rule-b",
                "Special B",
                "Career",
            )])
            .expect("second rules");
        let second_fingerprint =
            classification_fingerprint(&db, "/test/virtual/documents/special_project.txt")
                .expect("second fingerprint");

        assert_eq!(first.updated, 1);
        assert_eq!(second.scanned, 1);
        assert_eq!(second.updated, 1);
        assert_eq!(second.skipped, 0);
        assert_ne!(first_fingerprint.1, second_fingerprint.1);
    }

    #[test]
    fn execute_rules_on_inbox_reclassifies_when_file_mtime_changes() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(
            &db,
            "file-special",
            "special_plain.txt",
            "txt",
            2_048,
            1_900_000_000,
        );

        let rules = vec![name_contains_rule("special-rule", "Special", "Project")];
        db.execute_rules_on_inbox(rules.clone())
            .expect("first rules");
        set_file_mtime(
            &db,
            "/test/virtual/documents/special_plain.txt",
            1_900_000_100,
        );
        let second = db.execute_rules_on_inbox(rules).expect("second rules");
        let fingerprint = classification_fingerprint(&db, "/test/virtual/documents/special_plain.txt")
            .expect("fingerprint");

        assert_eq!(second.scanned, 1);
        assert_eq!(second.updated, 1);
        assert_eq!(second.skipped, 0);
        assert_eq!(fingerprint.2, 1_900_000_100);
    }

    #[test]
    fn execute_rules_on_inbox_reclassifies_when_file_size_changes() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(
            &db,
            "file-special",
            "special_plain.txt",
            "txt",
            2_048,
            1_900_000_000,
        );

        let rules = vec![name_contains_rule("special-rule", "Special", "Project")];
        db.execute_rules_on_inbox(rules.clone())
            .expect("first rules");
        set_file_size(&db, "/test/virtual/documents/special_plain.txt", 4_096);
        let second = db.execute_rules_on_inbox(rules).expect("second rules");
        let fingerprint = classification_fingerprint(&db, "/test/virtual/documents/special_plain.txt")
            .expect("fingerprint");

        assert_eq!(second.scanned, 1);
        assert_eq!(second.updated, 1);
        assert_eq!(second.skipped, 0);
        assert_eq!(fingerprint.3, 4_096);
    }

    #[test]
    fn execute_rules_for_paths_sets_classification_fingerprint() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(
            &db,
            "file-target",
            "target.tmp",
            "tmp",
            4_096,
            1_900_000_000,
        );

        let summary = db
            .execute_rules_for_paths(
                &["/test/virtual/documents/target.tmp".to_string()],
                Vec::new(),
            )
            .expect("targeted rules");
        let fingerprint =
            classification_fingerprint(&db, "/test/virtual/documents/target.tmp").expect("fingerprint");

        assert_eq!(summary.scanned, 1);
        assert_eq!(summary.updated, 1);
        assert_eq!(summary.skipped, 0);
        assert!(fingerprint.0 > 0);
        assert!(!fingerprint.1.is_empty());
        assert_eq!(fingerprint.2, 1_900_000_000);
        assert_eq!(fingerprint.3, 4_096);
    }

    #[test]
    fn rule_version_is_stable_for_same_rules() {
        let rules = vec![
            name_contains_rule("special-rule-a", "Special A", "Project"),
            name_contains_rule("special-rule-b", "Special B", "Career"),
        ];

        let first = rule_version_for_rules(&rules).expect("first version");
        let second = rule_version_for_rules(&rules).expect("second version");

        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
    }

    #[test]
    fn rule_version_ignores_user_rule_order() {
        let rule_a = name_contains_rule("special-rule-a", "Special A", "Project");
        let rule_b = name_contains_rule("special-rule-b", "Special B", "Career");

        let first = rule_version_for_rules(&[rule_a.clone(), rule_b.clone()]).expect("first version");
        let second = rule_version_for_rules(&[rule_b, rule_a]).expect("second version");

        assert_eq!(first, second);
    }

    #[test]
    fn rule_version_changes_when_rule_content_changes() {
        let mut changed_rule = name_contains_rule("special-rule-a", "Special A", "Project");
        let original = rule_version_for_rules(&[changed_rule.clone()]).expect("original version");

        changed_rule.weight = 50.0;
        let changed = rule_version_for_rules(&[changed_rule]).expect("changed version");

        assert_ne!(original, changed);
    }

    #[test]
    fn save_user_rule_round_trips_rule() {
        let db = Database::open(test_db_path()).expect("open test database");
        let rule = user_rule_for_persistence("rule-round-trip", "Round Trip", 300.0);

        let saved = db.save_user_rule(rule.clone()).expect("save user rule");
        let rules = db.get_user_rules().expect("get user rules");

        assert_eq!(saved.id, rule.id);
        assert_eq!(saved.source.as_str(), "user");
        assert_eq!(rules.len(), 1);
        assert_rule_matches(&rules[0], &saved);
        assert_eq!(rules[0].groups.len(), 1);
        assert_eq!(rules[0].groups[0].operator.as_str(), "AND");
        assert_eq!(rules[0].groups[0].conditions[0].field.as_str(), "extension");
        assert_eq!(
            rules[0].groups[0].conditions[0].value,
            Value::String("pdf".to_string())
        );
        assert_eq!(rules[0].action.lifecycle.as_deref(), Some("Archive"));
        assert_eq!(
            rules[0].action.target_template.as_deref(),
            Some("{home}/Archive")
        );
    }

    #[test]
    fn save_user_rule_forces_source_user() {
        let db = Database::open(test_db_path()).expect("open test database");
        let mut rule = user_rule_for_persistence("rule-source", "Source", 200.0);
        rule.source = "system".into();

        let saved = db.save_user_rule(rule).expect("save user rule");
        let rules = db.get_user_rules().expect("get user rules");

        assert_eq!(saved.source.as_str(), "user");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].source.as_str(), "user");
    }

    #[test]
    fn save_user_rule_rejects_empty_conditions() {
        let db = Database::open(test_db_path()).expect("open test database");
        let mut rule = user_rule_for_persistence("rule-empty", "Empty", 200.0);
        rule.groups[0].conditions[0].value = Value::String("   ".to_string());

        let error = db.save_user_rule(rule).expect_err("reject empty condition");

        assert!(error.to_string().contains("condition value is required"));
    }

    #[test]
    fn save_user_rule_rejects_incompatible_condition_matrix_values() {
        let cases = [
            ("size", "contains", Value::String("100".to_string())),
            ("modified_at", "olderThanDays", Value::Number(serde_json::Number::from_f64(1.5).expect("number"))),
            ("file_type", "contains", Value::String("Image".to_string())),
            ("risk_level", "equals", Value::String("invalid-risk".to_string())),
            ("is_duplicate", "is", Value::String("true".to_string()))
        ];

        for (index, (field, operator, value)) in cases.into_iter().enumerate() {
            let db = Database::open(test_db_path()).expect("open test database");
            let mut rule = user_rule_for_persistence(&format!("rule-matrix-{index}"), "Matrix", 200.0);
            rule.groups[0].conditions[0].field = field.into();
            rule.groups[0].conditions[0].operator = operator.into();
            rule.groups[0].conditions[0].value = value;

            db.save_user_rule(rule).expect_err("reject incompatible condition");
        }
    }

    #[test]
    fn save_user_rule_rejects_invalid_domain_values() {
        let db = Database::open(test_db_path()).expect("open test database");

        let mut purpose = user_rule_for_persistence("rule-purpose", "Purpose", 200.0);
        purpose.action.purpose = Some("not-a-purpose".into());
        assert!(db
            .save_user_rule(purpose)
            .expect_err("reject invalid purpose")
            .to_string()
            .contains("purpose is invalid"));

        let mut lifecycle = user_rule_for_persistence("rule-lifecycle", "Lifecycle", 200.0);
        lifecycle.action.lifecycle = Some("not-a-lifecycle".into());
        assert!(db
            .save_user_rule(lifecycle)
            .expect_err("reject invalid lifecycle")
            .to_string()
            .contains("lifecycle is invalid"));

        let mut risk = user_rule_for_persistence("rule-risk", "Risk", 200.0);
        risk.action.risk_level = Some("not-a-risk".into());
        assert!(db
            .save_user_rule(risk)
            .expect_err("reject invalid risk")
            .to_string()
            .contains("risk level is invalid"));

        let mut root = user_rule_for_persistence("rule-root-domain", "Root", 200.0);
        root.root_operator = "XOR".into();
        assert!(db
            .save_user_rule(root)
            .expect_err("reject invalid root operator")
            .to_string()
            .contains("root operator is invalid"));

        let mut group = user_rule_for_persistence("rule-group-domain", "Group", 200.0);
        group.groups[0].operator = "XOR".into();
        assert!(db
            .save_user_rule(group)
            .expect_err("reject invalid group operator")
            .to_string()
            .contains("group operator is invalid"));

        let mut condition =
            user_rule_for_persistence("rule-condition-domain", "Condition", 200.0);
        condition.groups[0].conditions[0].field = "bad-field".into();
        condition.groups[0].conditions[0].operator = "bad-operator".into();
        assert!(db
            .save_user_rule(condition)
            .expect_err("reject invalid condition domains")
            .to_string()
            .contains("condition field is invalid"));
    }

    #[test]
    fn legacy_invalid_rule_values_load_as_unknown() {
        let path = test_db_path();
        let db = Database::open(&path).expect("open test database");
        db.save_user_rule(user_rule_for_persistence("rule-legacy-enum", "Legacy", 200.0))
            .expect("save valid rule");
        let conn = Connection::open(db.path()).expect("open sqlite");
        conn.execute(
            "UPDATE rules SET root_operator = 'XOR', groups_json = ?2, action_json = ?3 WHERE id = ?1",
            params![
                "rule-legacy-enum",
                r#"[{"id":"legacy-group","operator":"XOR","conditions":[{"id":"legacy-condition","field":"bad-field","operator":"bad-operator","value":"pdf"}]}]"#,
                r#"{"purpose":"bad-purpose","lifecycle":"bad-lifecycle","risk_level":"bad-risk","suggested_action":"bad-action"}"#
            ],
        )
        .expect("inject legacy invalid values");
        crate::db::drop_source_writer_schema_for_test(&conn);
        conn.execute_batch("DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 19;")
            .expect("downgrade enum migration fixture");
        drop(conn);
        drop(db);

        let db = Database::open(&path).expect("migrate invalid rule domains");
        let rule = db.get_user_rules().expect("load migrated rules").remove(0);

        assert_eq!(rule.action.purpose.as_deref(), Some("Unknown"));
        assert_eq!(rule.action.lifecycle.as_deref(), Some("Unknown"));
        assert_eq!(rule.action.risk_level.as_deref(), Some("Unknown"));
        assert_eq!(rule.action.suggested_action.as_deref(), Some("Unknown"));
        assert_eq!(rule.root_operator, RuleOperator::Unknown);
        assert_eq!(rule.groups[0].operator, RuleOperator::Unknown);
        assert_eq!(rule.groups[0].conditions[0].field, ConditionField::Unknown);
        assert_eq!(
            rule.groups[0].conditions[0].operator,
            ConditionOperator::Unknown
        );
        let conn = Connection::open(db.path()).expect("inspect migrated sqlite");
        let stored: String = conn
            .query_row(
                "SELECT action_json FROM rules WHERE id = 'rule-legacy-enum'",
                [],
                |row| row.get(0),
            )
            .expect("read migrated action json");
        assert!(!stored.contains("bad-purpose"));
        assert_eq!(stored.matches("Unknown").count(), 4);
        let (root_operator, groups_json): (String, String) = conn
            .query_row(
                "SELECT root_operator, groups_json FROM rules WHERE id = 'rule-legacy-enum'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read migrated rule domains");
        assert_eq!(root_operator, "UNKNOWN");
        assert!(groups_json.contains("unknown"));
        assert!(!groups_json.contains("XOR"));
        assert!(!groups_json.contains("bad-field"));
    }

    #[test]
    fn save_user_rule_rejects_unsafe_move_target() {
        let db = Database::open(test_db_path()).expect("open test database");
        let mut rule = user_rule_for_persistence("rule-target", "Target", 200.0);
        rule.action.suggested_action = Some("Move".into());
        rule.action.target_template = Some("../Secrets".to_string());

        let error = db.save_user_rule(rule).expect_err("reject unsafe target");

        assert!(error.to_string().contains("target template"));
    }

    #[test]
    fn save_user_rule_rejects_unsafe_rename_template() {
        let db = Database::open(test_db_path()).expect("open test database");
        let mut rule = user_rule_for_persistence("rule-rename", "Rename", 200.0);
        rule.action.rename_template = Some("../outside.txt".to_string());

        let error = db.save_user_rule(rule).expect_err("reject rename template");

        assert!(error.to_string().contains("rename template is unsafe"));
    }

    #[test]
    fn operation_restore_lookup_only_returns_restorable_database_logs() {
        let db = Database::open(test_db_path()).expect("open test database");
        let success = operation_log("log-success", "batch-restore", "success");
        let failed = operation_log("log-failed", "batch-restore", "failed");
        db.save_operation_logs("batch-restore", &[success, failed])
            .expect("save logs");

        let logs = db
            .get_restorable_operation_logs_by_ids(&[
                "log-failed".to_string(),
                "log-success".to_string(),
                "log-missing".to_string(),
            ])
            .expect("load restorable logs");

        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].id, "log-success");
    }

    #[test]
    fn save_user_rule_updates_existing_rule() {
        let db = Database::open(test_db_path()).expect("open test database");
        let mut rule = user_rule_for_persistence("rule-update", "Before", 100.0);
        db.save_user_rule(rule.clone()).expect("save initial rule");

        rule.name = "After".to_string();
        rule.enabled = false;
        rule.action.lifecycle = Some("TrashReview".into());
        let saved = db.save_user_rule(rule.clone()).expect("update rule");
        let rules = db.get_user_rules().expect("get user rules");

        assert_eq!(rules.len(), 1);
        assert_eq!(saved.name, "After");
        assert!(!rules[0].enabled);
        assert_eq!(rules[0].action.lifecycle.as_deref(), Some("TrashReview"));
    }

    #[test]
    fn get_user_rules_orders_by_priority() {
        let db = Database::open(test_db_path()).expect("open test database");
        db.save_user_rule(user_rule_for_persistence("rule-low", "Low", 10.0))
            .expect("save low rule");
        db.save_user_rule(user_rule_for_persistence("rule-high", "High", 900.0))
            .expect("save high rule");
        db.save_user_rule(user_rule_for_persistence("rule-mid", "Mid", 100.0))
            .expect("save mid rule");

        let rules = db.get_user_rules().expect("get user rules");
        let ids = rules
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["rule-high", "rule-mid", "rule-low"]);
    }

    #[test]
    fn delete_user_rule_removes_user_rule() {
        let db = Database::open(test_db_path()).expect("open test database");
        db.save_user_rule(user_rule_for_persistence("rule-delete", "Delete", 100.0))
            .expect("save user rule");

        let deleted = db
            .delete_user_rule("rule-delete")
            .expect("delete user rule");
        let rules = db.get_user_rules().expect("get user rules");

        assert!(deleted);
        assert!(rules.iter().all(|rule| rule.id != "rule-delete"));
    }

    #[test]
    fn delete_user_rule_ignores_missing_rule() {
        let db = Database::open(test_db_path()).expect("open test database");

        let deleted = db
            .delete_user_rule("missing-rule")
            .expect("delete missing rule");

        assert!(!deleted);
    }

    #[test]
    fn delete_user_rule_does_not_delete_non_user_rule() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_system_rule_row(&db, "system-rule");

        let deleted = db
            .delete_user_rule("system-rule")
            .expect("delete system rule");
        let source = rule_source_by_id(&db, "system-rule").expect("system rule row");

        assert!(!deleted);
        assert_eq!(source, "system");
    }

    #[test]
    fn upsert_files_by_paths_inserts_new_file() {
        let db = Database::open(test_db_path()).expect("open test database");
        let root = test_dir();
        let file = root.join("new-report.txt");
        fs::write(&file, "hello").expect("write file");

        let upserted = upsert_files_by_paths_for_db(&db, &[file.to_string_lossy().into_owned()])
            .expect("upsert paths");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");

        assert_eq!(upserted, 1);
        assert_eq!(page.total, 1);
        assert_eq!(page.files[0].name, "new-report.txt");
        assert_eq!(page.files[0].path, normalized_test_path(&file));
        assert!(!page.files[0].is_stale);
    }

    #[test]
    fn upsert_files_by_paths_updates_modified_file() {
        let db = Database::open(test_db_path()).expect("open test database");
        let root = test_dir();
        let file = root.join("notes.txt");
        fs::write(&file, "old").expect("write file");
        let path = normalized_test_path(&file);
        db.insert_file(InsertFileRequest {
            id: path.clone(),
            path: path.clone(),
            name: "notes.txt".to_string(),
            extension: "txt".to_string(),
            size: 3,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("insert old file");
        fs::write(&file, "new content").expect("modify file");

        let upserted = upsert_files_by_paths_for_db(&db, &[file.to_string_lossy().into_owned()])
            .expect("upsert paths");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");

        assert_eq!(upserted, 1);
        assert_eq!(page.total, 1);
        assert_eq!(page.files[0].size, 11);
        assert!(!page.files[0].modified_at.is_empty());
    }

    #[test]
    fn upsert_files_by_paths_revives_stale_file() {
        let db = Database::open(test_db_path()).expect("open test database");
        let root = test_dir();
        let file = root.join("revived.txt");
        fs::write(&file, "hello").expect("write file");
        let path = normalized_test_path(&file);
        db.insert_file(InsertFileRequest {
            id: path.clone(),
            path: path.clone(),
            name: "revived.txt".to_string(),
            extension: "txt".to_string(),
            size: 5,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("insert file");
        db.remove_files_by_paths(std::slice::from_ref(&path))
            .expect("mark stale");

        let upserted = upsert_files_by_paths_for_db(&db, &[file.to_string_lossy().into_owned()])
            .expect("upsert paths");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");

        assert_eq!(upserted, 1);
        assert_eq!(page.total, 1);
        assert_eq!(page.files[0].path, path);
        assert_eq!(stale_state(&db, &page.files[0].path), Some((false, true)));
    }

    #[test]
    fn upsert_files_by_paths_ignores_missing_path() {
        let db = Database::open(test_db_path()).expect("open test database");
        let root = test_dir();
        let missing = root.join("missing.txt");

        let upserted = upsert_files_by_paths_for_db(&db, &[missing.to_string_lossy().into_owned()])
            .expect("upsert paths");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");

        assert_eq!(upserted, 0);
        assert_eq!(page.total, 0);
    }

    #[test]
    fn upsert_files_by_paths_recursively_indexes_directory_contents() {
        let db = Database::open(test_db_path()).expect("open test database");
        let root = test_dir();
        let imported = root.join("imported");
        let nested = imported.join("nested");
        let ignored = imported.join("node_modules");
        fs::create_dir_all(&nested).expect("nested dir");
        fs::create_dir_all(&ignored).expect("ignored dir");
        fs::write(imported.join("top-level.txt"), "top").expect("write top-level file");
        fs::write(nested.join("deep-note.md"), "deep").expect("write nested file");
        fs::write(ignored.join("package-file.js"), "ignored").expect("write ignored file");

        let upserted = upsert_files_by_paths_for_db(&db, &[imported.to_string_lossy().into_owned()])
            .expect("upsert directory");
        let page = db.get_paged_files(Some(20), Some(0), None).expect("page");
        let names = page
            .files
            .iter()
            .map(|file| file.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(upserted, 4);
        assert!(names.contains(&"imported"));
        assert!(names.contains(&"nested"));
        assert!(names.contains(&"top-level.txt"));
        assert!(names.contains(&"deep-note.md"));
        assert!(!names.contains(&"node_modules"));
        assert!(!names.contains(&"package-file.js"));
    }

    #[test]
    fn upsert_files_by_paths_with_optional_optimize_handles_large_batch() {
        let db = Database::open(test_db_path()).expect("open test database");
        let root = test_dir();
        let mut paths = Vec::with_capacity(OPTIMIZE_AFTER_UPSERT_THRESHOLD);
        for index in 0..OPTIMIZE_AFTER_UPSERT_THRESHOLD {
            let file = root.join(format!("large-upsert-{index:04}.txt"));
            fs::write(&file, format!("file {index}")).expect("write file");
            paths.push(file.to_string_lossy().into_owned());
        }

        let upserted = upsert_files_by_paths_with_optional_optimize(&db, &paths).expect("upsert paths");
        let page = db.get_paged_files(Some(1), Some(0), None).expect("page");

        assert_eq!(upserted, OPTIMIZE_AFTER_UPSERT_THRESHOLD);
        assert_eq!(page.total, OPTIMIZE_AFTER_UPSERT_THRESHOLD as i64);
    }

    #[test]
    fn execute_rules_updates_inbox_resume_classification() {
        let db = Database::open(test_db_path()).expect("open test database");
        enable_legacy_builtin_rules(&db);
        insert_test_file(
            &db,
            "file-resume",
            "resume_2026.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );

        let summary = db
            .execute_rules_on_inbox(Vec::new())
            .expect("execute rules");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");
        let file = page
            .files
            .iter()
            .find(|file| file.id == "file-resume")
            .expect("classified file");

        assert_eq!(summary.scanned, 1);
        assert_eq!(summary.updated, 1);
        assert_eq!(file.purpose, "Career");
        assert_eq!(file.lifecycle, "Reference");
        assert_eq!(file.suggested_action, "Move");
        assert!(file.confidence >= 0.8);
        assert!(file
            .matched_rules
            .iter()
            .any(|rule| rule == "legacy_builtin:system_career"));
        assert_ne!(
            file.classification_reason,
            "Indexed by Fylura Tauri backend."
        );
    }

    #[test]
    fn execute_rules_classifies_common_real_world_files_into_actionable_suggestions() {
        let db = Database::open(test_db_path()).expect("open test database");
        enable_legacy_builtin_rules(&db);
        let samples = [
            (
                "file-photo",
                "photo_001.jpg",
                "jpg",
                "Media",
                "Media/Images",
            ),
            (
                "file-screenshot",
                "截图_桌面.png",
                "png",
                "Media",
                "Media/Screenshots",
            ),
            ("file-video", "vacation.mp4", "mp4", "Media", "Media/Videos"),
            ("file-audio", "podcast.mp3", "mp3", "Media", "Media/Audio"),
            ("file-lecture", "lecture_notes.pdf", "pdf", "Study", "Study"),
            (
                "file-thesis",
                "毕业论文最终版.docx",
                "docx",
                "Study",
                "Study",
            ),
            (
                "file-budget",
                "budget.xlsx",
                "xlsx",
                "Finance",
                "Sensitive/Finance",
            ),
            (
                "file-slides",
                "slides.pptx",
                "pptx",
                "Work",
                "Presentations",
            ),
            ("file-archive", "archive.zip", "zip", "Archive", "Archives"),
            (
                "file-package",
                "package.json",
                "json",
                "Project",
                "Projects",
            ),
            (
                "file-new-text",
                "新建文本文档.txt",
                "txt",
                "Temporary",
                "Temporary",
            ),
        ];

        for (id, name, extension, _, _) in samples {
            insert_test_file(&db, id, name, extension, 2_048, 1_900_000_000);
        }

        db.execute_rules_on_inbox(Vec::new())
            .expect("execute rules");
        let page = db.get_paged_files(Some(50), Some(0), None).expect("page");

        for (_, name, _, expected_purpose, target_fragment) in samples {
            let file = page
                .files
                .iter()
                .find(|file| file.name == name)
                .unwrap_or_else(|| panic!("classified sample {name}"));
            let normalized_target = file.suggested_target_path.replace('\\', "/");

            assert_eq!(file.classification_status, "classified", "{name}");
            assert_eq!(file.purpose, expected_purpose, "{name}");
            assert_ne!(file.suggested_action, "Keep", "{name}");
            assert!(!file.suggested_target_path.is_empty(), "{name}");
            assert!(
                normalized_target.contains(target_fragment),
                "{name} target was {}",
                file.suggested_target_path
            );
            assert!(!normalized_target.contains("00_Inbox"), "{name}");
            assert!(!normalized_target.contains("20_Areas"), "{name}");
            assert!(!normalized_target.contains("90_Temporary"), "{name}");
        }
    }

    #[test]
    fn execute_rules_routes_teacher_and_work_documents_by_chinese_keywords() {
        let db = Database::open(test_db_path()).expect("open test database");
        enable_legacy_builtin_rules(&db);
        let samples = [
            (
                "file-teaching-plan",
                "数据库项目化课程教案.docx",
                "docx",
                "Teaching",
                "Teaching",
                "Move",
                "Normal",
            ),
            (
                "file-practice",
                "Scala实训复习题.pdf",
                "pdf",
                "Teaching",
                "Teaching",
                "Move",
                "Normal",
            ),
            (
                "file-software-test",
                "软件测试学生试卷.docx",
                "docx",
                "Teaching",
                "Teaching",
                "Move",
                "Normal",
            ),
            (
                "file-work",
                "会议通知方案.docx",
                "docx",
                "Work",
                "Work/Documents",
                "Move",
                "Normal",
            ),
            (
                "file-invoice-cn",
                "发票账单.pdf",
                "pdf",
                "Finance",
                "Sensitive/Finance",
                "Review",
                "Sensitive",
            ),
            (
                "file-identity-cn",
                "身份证扫描件.pdf",
                "pdf",
                "Identity",
                "Sensitive/Identity",
                "Review",
                "Sensitive",
            ),
        ];

        for (id, name, extension, _, _, _, _) in samples {
            insert_test_file(&db, id, name, extension, 2_048, 1_900_000_000);
        }

        db.execute_rules_on_inbox(Vec::new())
            .expect("execute rules");
        let page = db.get_paged_files(Some(50), Some(0), None).expect("page");

        for (_, name, _, expected_purpose, target_fragment, action, risk) in samples {
            let file = page
                .files
                .iter()
                .find(|file| file.name == name)
                .unwrap_or_else(|| panic!("classified sample {name}"));
            let normalized_target = file.suggested_target_path.replace('\\', "/");

            assert_eq!(file.purpose, expected_purpose, "{name}");
            assert_eq!(file.suggested_action, action, "{name}");
            assert_eq!(file.risk_level, risk, "{name}");
            assert!(
                normalized_target.contains(target_fragment),
                "{name} target was {}",
                file.suggested_target_path
            );
            assert!(!normalized_target.contains("00_Inbox"), "{name}");
        }
    }

    #[test]
    fn desktop_and_downloads_source_signal_does_not_force_inbox_target() {
        let db = Database::open(test_db_path()).expect("open test database");
        enable_legacy_builtin_rules(&db);
        insert_test_file_at_path(
            &db,
            "file-downloads-doc",
            "/tmp/Downloads/普通文档.docx",
            "普通文档.docx",
            "docx",
            2_048,
            1_900_000_000,
        );
        insert_test_file_at_path(
            &db,
            "file-downloads-unknown",
            "/tmp/Downloads/random.bin",
            "random.bin",
            "bin",
            2_048,
            1_900_000_000,
        );

        db.execute_rules_on_inbox(Vec::new())
            .expect("execute rules");
        let page = db.get_paged_files(Some(50), Some(0), None).expect("page");
        let document = page
            .files
            .iter()
            .find(|file| file.id == "file-downloads-doc")
            .expect("document");
        let unknown = page
            .files
            .iter()
            .find(|file| file.id == "file-downloads-unknown")
            .expect("unknown");

        assert_eq!(document.suggested_target_path, "/tmp/Downloads/Documents");
        assert!(!document.suggested_target_path.contains("00_Inbox"));
        assert!(unknown.suggested_target_path.is_empty());
        assert_eq!(unknown.suggested_action, "Keep");
    }

    #[test]
    fn file_records_expose_classification_status_for_unclassified_and_classified_files() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(
            &db,
            "file-status-resume",
            "resume_2026.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );

        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");
        let unclassified = page
            .files
            .iter()
            .find(|file| file.id == "file-status-resume")
            .expect("unclassified file");

        assert_eq!(unclassified.classification_status, "unclassified");

        db.execute_rules_on_inbox(Vec::new())
            .expect("execute rules");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");
        let classified = page
            .files
            .iter()
            .find(|file| file.id == "file-status-resume")
            .expect("classified file");

        assert_eq!(classified.classification_status, "classified");
    }

    #[test]
    fn execute_rules_marks_finance_files_sensitive_review_only() {
        let db = Database::open(test_db_path()).expect("open test database");
        enable_legacy_builtin_rules(&db);
        insert_test_file(
            &db,
            "file-invoice",
            "invoice_apple.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );

        let summary = db
            .execute_rules_on_inbox(Vec::new())
            .expect("execute rules");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");
        let file = page
            .files
            .iter()
            .find(|file| file.id == "file-invoice")
            .expect("classified file");

        assert_eq!(summary.needs_confirmation, 1);
        assert_eq!(file.purpose, "Finance");
        assert_eq!(file.risk_level, "Sensitive");
        assert_eq!(file.suggested_action, "Review");
        assert!(file.requires_confirmation);
    }

    #[test]
    fn execute_rules_accepts_user_rules_that_override_builtins() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(
            &db,
            "file-resume-project",
            "resume_2026.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );

        let user_rule = Rule {
            id: "user_resume_project".to_string(),
            name: "Resume project override".to_string(),
            source: "user".into(),
            enabled: true,
            priority: 150.0,
            weight: 96.0,
            root_operator: "OR".into(),
            groups: vec![RuleConditionGroup {
                id: "resume_project_group".to_string(),
                operator: "AND".into(),
                conditions: vec![RuleCondition {
                    id: "resume_name".to_string(),
                    field: "name".into(),
                    operator: "contains".into(),
                    value: Value::String("resume".to_string()),
                }],
            }],
            action: RuleAction {
                purpose: Some("Project".into()),
                lifecycle: Some("Active".into()),
                context: Some("Override".to_string()),
                risk_level: Some("Normal".into()),
                suggested_action: Some("Move".into()),
                target_template: Some("20_Areas/Projects".to_string()),
                rename_template: None,
            },
            created_at: String::new(),
            updated_at: String::new(),
        };

        db.execute_rules_on_inbox(vec![user_rule])
            .expect("execute rules");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");
        let file = page
            .files
            .iter()
            .find(|file| file.id == "file-resume-project")
            .expect("classified file");

        assert_eq!(file.purpose, "Project");
        assert_eq!(file.lifecycle, "Active");
        assert!(file
            .matched_rules
            .iter()
            .any(|rule| rule == "rule:user_resume_project"));
    }

    #[test]
    fn search_files_matches_chinese_substrings_with_trigram() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(
            &db,
            "file-cn",
            "项目报告2026_final.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );

        let results = db.search_files("报告2026", Some(10)).expect("search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "项目报告2026_final.pdf");
    }

    #[test]
    fn search_files_uses_like_fallback_for_cjk_short_terms() {
        let db = Database::open(test_db_path()).expect("open test database");
        let samples = [
            ("file-thesis", "毕业论文最终版.docx", "docx"),
            ("file-report", "课程报告.pdf", "pdf"),
            ("file-screenshot", "截图_桌面.png", "png"),
            ("file-photo", "照片001.jpg", "jpg"),
            ("file-java", "Java学习笔记.md", "md"),
        ];
        for (id, name, extension) in samples {
            insert_test_file(&db, id, name, extension, 2_048, 1_900_000_000);
        }

        for (query, expected_name) in [
            ("论文", "毕业论文最终版.docx"),
            ("报告", "课程报告.pdf"),
            ("截图", "截图_桌面.png"),
            ("照片", "照片001.jpg"),
            ("学习", "Java学习笔记.md"),
            ("Java", "Java学习笔记.md"),
        ] {
            let names = db
                .search_files(query, Some(10))
                .expect("search")
                .into_iter()
                .map(|file| file.name)
                .collect::<Vec<_>>();

            assert!(
                names.iter().any(|name| name == expected_name),
                "{query} returned {names:?}"
            );
        }
    }

    #[test]
    fn get_paged_files_uses_like_fallback_for_cjk_short_terms() {
        let db = Database::open(test_db_path()).expect("open test database");
        let samples = [
            ("file-thesis", "毕业论文最终版.docx", "docx"),
            ("file-report", "课程报告.pdf", "pdf"),
            ("file-screenshot", "截图_桌面.png", "png"),
            ("file-photo", "照片001.jpg", "jpg"),
            ("file-java", "Java学习笔记.md", "md"),
        ];
        for (id, name, extension) in samples {
            insert_test_file(&db, id, name, extension, 2_048, 1_900_000_000);
        }

        for (query, expected_name) in [
            ("论文", "毕业论文最终版.docx"),
            ("报告", "课程报告.pdf"),
            ("截图", "截图_桌面.png"),
            ("照片", "照片001.jpg"),
            ("学习", "Java学习笔记.md"),
            ("Java", "Java学习笔记.md"),
        ] {
            let page = db
                .get_paged_files(Some(10), Some(0), Some(query))
                .expect("page search");
            let names = page
                .files
                .into_iter()
                .map(|file| file.name)
                .collect::<Vec<_>>();

            assert!(
                names.iter().any(|name| name == expected_name),
                "{query} returned {names:?}"
            );
        }
    }

    #[test]
    fn search_files_filters_by_library_scope_roots() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file_at_path(
            &db,
            "file-root-a",
            "/tmp/root-a/a.pdf",
            "a.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );
        insert_test_file_at_path(
            &db,
            "file-root-b",
            "/tmp/root-b/b.pdf",
            "b.pdf",
            "pdf",
            4_096,
            1_900_000_001,
        );

        let results = db
            .search_files_in_scope(
                "pdf",
                Some(10),
                &LibraryScope::Roots {
                    roots: vec!["/tmp/root-a".to_string()],
                },
            )
            .expect("search root a");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "a.pdf");
    }
