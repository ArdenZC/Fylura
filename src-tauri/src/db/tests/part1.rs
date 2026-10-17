    #[test]
    fn app_settings_defaults_include_search_scope_settings() {
        let settings = AppSettings::default();

        assert_eq!(settings.search_hotkey, "CmdOrCtrl+K");
        assert_eq!(settings.search_scope_mode, "all");
        assert!(settings.custom_search_roots.is_empty());
        assert_eq!(settings.organize_root_mode, OrganizeRootMode::CurrentFolder);
        assert_eq!(settings.organize_root_path, None);
    }

    #[test]
    fn app_settings_deserializes_legacy_json_with_search_scope_defaults() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "closeBehavior": "ask",
            "folderNamingLanguage": "en",
            "defaultScanFolders": [],
            "restoreRetentionDays": 30,
            "launchAtLogin": false
        }))
        .expect("legacy settings deserialize");

        assert_eq!(settings.search_hotkey, "CmdOrCtrl+K");
        assert_eq!(settings.search_scope_mode, "all");
        assert!(settings.custom_search_roots.is_empty());
        assert_eq!(settings.organize_root_mode, OrganizeRootMode::CurrentFolder);
        assert_eq!(settings.organize_root_path, None);
    }

    #[test]
    fn translate_template_uses_chinese_folder_segments() {
        assert_eq!(
            translate_template("20_Areas/Personal/Identity", "zh"),
            "20_领域/个人/证件"
        );
        assert_eq!(
            translate_template("40_Archive/{year}/Study", "zh"),
            "40_归档/{year}/学业"
        );
        assert_eq!(
            translate_template("90_Temporary/Installers", "zh"),
            "90_临时/安装包"
        );
        assert_eq!(
            translate_template("20_Areas/Projects", "en"),
            "20_Areas/Projects"
        );
    }

    #[test]
    fn execute_rules_on_inbox_uses_persisted_chinese_folder_naming_for_new_classifications() {
        let db = Database::open(test_db_path()).expect("open test database");
        let settings = AppSettings {
            folder_naming_language: "zh".to_string(),
            use_legacy_builtin_classification_rules: true,
            ..AppSettings::default()
        };
        save_app_settings(&db, &settings).expect("save app settings");
        insert_test_file(
            &db,
            "file-resume-zh",
            "resume_2026.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );

        db.execute_rules_on_inbox(Vec::new())
            .expect("execute rules");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");
        let file = page
            .files
            .iter()
            .find(|file| file.id == "file-resume-zh")
            .expect("classified file");

        assert!(file.suggested_target_path.contains("职业"));
        assert!(!file.suggested_target_path.contains("Career"));
    }

    #[test]
    fn folder_naming_language_change_rebuilds_existing_media_suggestion_path() {
        let db = Database::open(test_db_path()).expect("open test database");
        let mut settings = AppSettings {
            folder_naming_language: "en".to_string(),
            use_legacy_builtin_classification_rules: true,
            ..AppSettings::default()
        };
        save_app_settings(&db, &settings).expect("save english settings");
        insert_test_file(
            &db,
            "file-photo-language",
            "photo_001.jpg",
            "jpg",
            2_048,
            1_900_000_000,
        );

        db.execute_rules_on_inbox(Vec::new())
            .expect("execute english rules");
        let english_path = db
            .get_paged_files(Some(10), Some(0), None)
            .expect("english page")
            .files
            .into_iter()
            .find(|file| file.id == "file-photo-language")
            .expect("english file")
            .suggested_target_path
            .replace('\\', "/");

        settings.folder_naming_language = "zh".to_string();
        save_app_settings(&db, &settings).expect("save chinese settings");
        set_file_lifecycle(&db, "/test/virtual/documents/photo_001.jpg", "Inbox");
        let summary = db
            .execute_rules_for_paths(
                &["/test/virtual/documents/photo_001.jpg".to_string()],
                Vec::new(),
            )
            .expect("execute chinese rules");
        let chinese_path = db
            .get_paged_files(Some(10), Some(0), None)
            .expect("chinese page")
            .files
            .into_iter()
            .find(|file| file.id == "file-photo-language")
            .expect("chinese file")
            .suggested_target_path
            .replace('\\', "/");

        assert!(english_path.contains("Images"));
        assert_eq!(summary.updated, 1);
        assert!(chinese_path.contains("图片"));
        assert_ne!(english_path, chinese_path);
    }

    #[test]
    fn default_organize_root_places_documents_under_current_folder_without_zencanvas_or_inbox() {
        let db = Database::open(test_db_path()).expect("open test database");
        enable_legacy_builtin_rules(&db);
        insert_test_file_at_path(
            &db,
            "file-desktop-doc",
            "/tmp/Desktop/test.docx",
            "test.docx",
            "docx",
            2_048,
            1_900_000_000,
        );

        db.execute_rules_on_inbox(Vec::new())
            .expect("execute rules");
        let file = db
            .get_paged_files(Some(10), Some(0), None)
            .expect("page")
            .files
            .into_iter()
            .find(|file| file.id == "file-desktop-doc")
            .expect("classified file");

        assert_eq!(file.lifecycle, "Reference");
        assert_eq!(file.suggested_target_path, "/tmp/Desktop/Documents");
        assert!(!file.suggested_target_path.contains("ZenCanvas"));
        assert!(!file.suggested_target_path.contains("00_Inbox"));
    }

    #[test]
    fn zen_canvas_organize_root_mode_preserves_legacy_wrapper() {
        let db = Database::open(test_db_path()).expect("open test database");
        let settings = AppSettings {
            organize_root_mode: OrganizeRootMode::ZenCanvasFolder,
            use_legacy_builtin_classification_rules: true,
            ..AppSettings::default()
        };
        save_app_settings(&db, &settings).expect("save app settings");
        insert_test_file_at_path(
            &db,
            "file-legacy-doc",
            "/tmp/Desktop/legacy.docx",
            "legacy.docx",
            "docx",
            2_048,
            1_900_000_000,
        );

        db.execute_rules_on_inbox(Vec::new())
            .expect("execute rules");
        let path = db
            .get_paged_files(Some(10), Some(0), None)
            .expect("page")
            .files
            .into_iter()
            .find(|file| file.id == "file-legacy-doc")
            .expect("classified file")
            .suggested_target_path;

        assert_eq!(path, "/tmp/Desktop/ZenCanvas/Documents");
    }

    #[test]
    fn custom_organize_root_mode_uses_configured_root_for_all_targets() {
        let db = Database::open(test_db_path()).expect("open test database");
        let settings = AppSettings {
            organize_root_mode: OrganizeRootMode::CustomRoot,
            organize_root_path: Some("/tmp/Organized".to_string()),
            use_legacy_builtin_classification_rules: true,
            ..AppSettings::default()
        };
        save_app_settings(&db, &settings).expect("save app settings");
        insert_test_file_at_path(
            &db,
            "file-custom-doc",
            "/tmp/Desktop/custom.docx",
            "custom.docx",
            "docx",
            2_048,
            1_900_000_000,
        );

        db.execute_rules_on_inbox(Vec::new())
            .expect("execute rules");
        let path = db
            .get_paged_files(Some(10), Some(0), None)
            .expect("page")
            .files
            .into_iter()
            .find(|file| file.id == "file-custom-doc")
            .expect("classified file")
            .suggested_target_path;

        assert_eq!(path, "/tmp/Organized/Documents");
    }

    #[test]
    fn target_paths_are_normalized_to_forward_slashes_without_mixed_windows_separators() {
        let db = Database::open(test_db_path()).expect("open test database");
        enable_legacy_builtin_rules(&db);
        insert_test_file_at_path(
            &db,
            "file-windows-doc",
            "C:/Users/77588/Desktop/测试用.docx",
            "测试用.docx",
            "docx",
            2_048,
            1_900_000_000,
        );

        db.execute_rules_on_inbox(Vec::new())
            .expect("execute rules");
        let path = db
            .get_paged_files(Some(10), Some(0), None)
            .expect("page")
            .files
            .into_iter()
            .find(|file| file.id == "file-windows-doc")
            .expect("classified file")
            .suggested_target_path;

        assert_eq!(path, "C:/Users/77588/Desktop/Documents");
        assert!(!path.contains('\\'));
    }

    #[test]
    fn get_paged_files_returns_limit_and_offset() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(&db, "file-1", "report.pdf", "pdf", 2_048, 1_800_000_000);
        insert_test_file(&db, "file-2", "photo.jpg", "jpg", 4_096, 1_900_000_000);

        let page = db.get_paged_files(Some(1), Some(1), None).expect("page");

        assert_eq!(page.total, 2);
        assert_eq!(page.limit, 1);
        assert_eq!(page.offset, 1);
        assert_eq!(page.files.len(), 1);
        assert_eq!(page.files[0].name, "report.pdf");
    }

    #[test]
    fn migrated_schema_contains_common_library_performance_indexes() {
        let db = Database::open(test_db_path()).expect("open test database");
        let conn = Connection::open(db.path()).expect("open migrated database");
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'index' AND tbl_name = 'files'")
            .expect("prepare index query");
        let index_names = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query index names")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect index names");

        for expected in [
            "idx_files_active_mtime",
            "idx_files_lifecycle_mtime",
            "idx_files_action_mtime",
            "idx_files_review_mtime",
            "idx_files_risk_mtime",
            "idx_files_scope_path",
        ] {
            assert!(
                index_names.iter().any(|name| name == expected),
                "missing performance index {expected}; indexes were {index_names:?}"
            );
        }
    }

    #[test]
    fn schema_12_migrates_v11_non_trigram_fts_and_restores_triggers() {
        let path = test_db_path();
        {
            let db = Database::open(&path).expect("open current database");
            insert_test_file(
                &db,
                "file-cn-report",
                "项目报告2026_final.pdf",
                "pdf",
                2_048,
                1_900_000_000,
            );
        }
        {
            let conn = Connection::open(&path).expect("open database to simulate v11");
            crate::db::drop_source_writer_schema_for_test(&conn);
            conn.execute_batch(
                    r#"
                        DROP TRIGGER IF EXISTS files_ai;
                        DROP TRIGGER IF EXISTS files_ad;
                        DROP TRIGGER IF EXISTS files_au;
                        DROP TABLE IF EXISTS files_fts;

                        CREATE VIRTUAL TABLE files_fts USING fts5(
                            name,
                            path,
                            content='files',
                            content_rowid='rowid'
                        );
                        INSERT INTO files_fts(files_fts) VALUES('rebuild');

                        CREATE TRIGGER files_ai AFTER INSERT ON files BEGIN
                            INSERT INTO files_fts(rowid, name, path) VALUES (new.rowid, new.name, new.path);
                        END;
                        CREATE TRIGGER files_ad AFTER DELETE ON files BEGIN
                            INSERT INTO files_fts(files_fts, rowid, name, path)
                            VALUES('delete', old.rowid, old.name, old.path);
                        END;
                        CREATE TRIGGER files_au AFTER UPDATE ON files BEGIN
                            INSERT INTO files_fts(files_fts, rowid, name, path)
                            VALUES('delete', old.rowid, old.name, old.path);
                            INSERT INTO files_fts(rowid, name, path) VALUES (new.rowid, new.name, new.path);
                        END;

                        DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 11;
                        "#,
                )
                .expect("simulate v11 non-trigram fts");
        }

        let db = Database::open(&path).expect("migrate v11 database");
        let conn = Connection::open(&path).expect("inspect migrated database");
        let fts_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'files_fts'",
                [],
                |row| row.get(0),
            )
            .expect("fts definition");
        let trigger_count: i64 = conn
            .query_row(
                r#"
                        SELECT COUNT(*)
                        FROM sqlite_schema
                        WHERE type = 'trigger'
                          AND name IN ('files_ai', 'files_ad', 'files_au')
                        "#,
                [],
                |row| row.get(0),
            )
            .expect("trigger count");

        assert!(fts_sql.to_ascii_lowercase().contains("tokenize='trigram'"));
        assert_eq!(trigger_count, 3);
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
                .expect("schema version"),
            42
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name IN ('cleanup_trash_batches', 'cleanup_trash_items')",
                [],
                |row| row.get::<_, i64>(0)
            )
            .expect("cleanup trash tables"),
            2
        );
        assert_eq!(
            db.search_files("报告2026", Some(10))
                .expect("search migrated trigram")
                .len(),
            1
        );

        insert_test_file(
            &db,
            "file-cn-contract",
            "新增合同2026.pdf",
            "pdf",
            2_048,
            1_900_000_001,
        );
        assert_eq!(
            db.search_files("合同2026", Some(10))
                .expect("search inserted fts")
                .len(),
            1
        );

        conn.execute(
            r#"
                    UPDATE files
                    SET name = '更新合同2026.pdf',
                        path = '/test/virtual/documents/更新合同2026.pdf'
                    WHERE id = 'file-cn-contract'
                    "#,
            [],
        )
        .expect("update file row");
        assert_eq!(
            db.search_files("更新合同", Some(10))
                .expect("search updated fts")
                .len(),
            1
        );

        conn.execute("DELETE FROM files WHERE id = 'file-cn-contract'", [])
            .expect("delete file row");
        assert!(db
            .search_files("更新合同", Some(10))
            .expect("search deleted fts")
            .is_empty());
    }

    #[test]
    fn pooled_connections_use_performance_pragmas() {
        let db = Database::open(test_db_path()).expect("open test database");
        let conn = db.conn().expect("get pooled connection");

        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("journal mode");
        let synchronous: i64 = conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .expect("synchronous mode");
        let temp_store: i64 = conn
            .query_row("PRAGMA temp_store", [], |row| row.get(0))
            .expect("temp store");
        let mmap_size: i64 = conn
            .query_row("PRAGMA mmap_size", [], |row| row.get(0))
            .expect("mmap size");

        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        assert_eq!(synchronous, 1);
        assert_eq!(temp_store, 2);
        assert!(
            (2_000_000_000..=3_000_000_000).contains(&mmap_size),
            "expected a large mmap_size up to the requested 3GB, got {mmap_size}"
        );
    }

    #[test]
    fn database_rejects_schema_43_as_a_future_version() {
        let path = test_db_path();
        let db = Database::open(&path).expect("create database");
        drop(db);
        let conn = Connection::open(&path).expect("open sqlite");
        conn.execute_batch("PRAGMA user_version = 43;")
            .expect("set future version");
        drop(conn);

        let error = match Database::open(&path) {
            Ok(_) => panic!("expected future schema rejection"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("newer than this app supports"));
    }

    #[test]
    fn schema_30_to_31_creates_file_library_tables_without_changing_files_ids() {
        let path = test_db_path();
        let db = Database::open(&path).expect("create schema 31 database");
        db.insert_file(InsertFileRequest {
            id: "schema-30-legacy-file".to_string(),
            path: "/tmp/schema-30-legacy-file.txt".to_string(),
            name: "schema-30-legacy-file.txt".to_string(),
            extension: "txt".to_string(),
            size: 10,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("seed legacy file");
        drop(db);

        let conn = Connection::open(&path).expect("open schema 30 fixture");
        crate::db::drop_source_writer_schema_for_test(&conn);
        conn.execute_batch(
            r#"
            DROP TABLE file_user_tags;
            DROP TABLE user_tags;
            DROP TABLE library_saved_views;
            DROP TABLE library_query_state;
            DROP INDEX idx_library_files_modified;
            DROP INDEX idx_library_files_created;
            DROP INDEX idx_library_files_name;
            DROP INDEX idx_library_files_size;
            DROP INDEX idx_library_files_confidence;
            DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 30;
            "#,
        )
        .expect("create real schema 30 fixture");
        drop(conn);

        let migrated = Database::open(&path).expect("migrate schema 30 fixture");
        let conn = migrated.conn().expect("inspect schema 31 database");
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("schema version");
        assert_eq!(version, 42);
        for table in [
            "user_tags",
            "file_user_tags",
            "library_saved_views",
            "library_query_state",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                    params![table],
                    |row| row.get(0),
                )
                .expect("file library table lookup");
            assert_eq!(count, 1, "missing schema 31 table {table}");
        }
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'index' AND name IN ('idx_user_tags_name', 'idx_file_user_tags_tag_file', 'idx_library_saved_views_position')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("file library indexes"),
            3
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'index' AND name IN ('idx_library_files_modified', 'idx_library_files_created', 'idx_library_files_name', 'idx_library_files_size', 'idx_library_files_confidence')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("file library sort indexes"),
            5
        );
        assert_eq!(
            conn.query_row(
                "SELECT id FROM files WHERE id = 'schema-30-legacy-file'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("legacy files id"),
            "schema-30-legacy-file"
        );
        assert_eq!(
            conn.query_row(
                "SELECT revision FROM library_query_state WHERE singleton_id = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("library revision singleton"),
            1
        );
        drop(conn);
        drop(migrated);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn schema_31_to_32_creates_organization_ledger_and_revision_cas_without_changing_files_ids() {
        let path = test_db_path();
        let db = Database::open(&path).expect("create current database");
        db.insert_file(InsertFileRequest {
            id: "schema-31-stable-file-id".to_string(),
            path: "/tmp/schema-31-stable-file-id.txt".to_string(),
            name: "schema-31-stable-file-id.txt".to_string(),
            extension: "txt".to_string(),
            size: 10,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("seed stable file id");
        drop(db);

        let conn = Connection::open(&path).expect("open schema fixture");
        crate::db::drop_source_writer_schema_for_test(&conn);
        conn.execute_batch(
            r#"
            DROP TABLE organization_plan_items;
            DROP TABLE organization_plans;
            ALTER TABLE user_tags DROP COLUMN revision;
            ALTER TABLE library_saved_views DROP COLUMN revision;
            DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 31;
            "#,
        )
        .expect("construct real schema 31 fixture");
        drop(conn);

        let migrated = Database::open(&path).expect("migrate schema 31 to 32");
        let conn = migrated.conn().expect("inspect schema 32");
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .expect("schema version"),
            42
        );
        for table in ["organization_plans", "organization_plan_items"] {
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                    params![table],
                    |row| row.get::<_, i64>(0),
                )
                .expect("organization table"),
                1
            );
        }
        for table in ["user_tags", "library_saved_views"] {
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = 'revision' AND \"notnull\" = 1 AND dflt_value = '1'",
                    params![table],
                    |row| row.get::<_, i64>(0),
                )
                .expect("revision column"),
                1
            );
        }
        assert_eq!(
            conn.query_row(
                "SELECT id FROM files WHERE path = '/tmp/schema-31-stable-file-id.txt'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("stable file id"),
            "schema-31-stable-file-id"
        );
    }

    #[test]
    fn schema_31_to_32_conflict_rolls_back_atomically() {
        let path = test_db_path();
        let db = Database::open(&path).expect("create current database");
        drop(db);
        let conn = Connection::open(&path).expect("open schema fixture");
        crate::db::drop_source_writer_schema_for_test(&conn);
        conn.execute_batch(
            r#"
            DROP TABLE organization_plan_items;
            DROP TABLE organization_plans;
            CREATE TABLE idx_organization_plans_status_updated (conflict TEXT);
            DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 31;
            "#,
        )
        .expect("construct conflicting schema 31 fixture");
        drop(conn);

        let error = match Database::open(&path) {
            Ok(_) => panic!("duplicate revision columns must fail"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("already a table"),
            "unexpected migration error: {error}"
        );

        let conn = Connection::open(&path).expect("inspect rolled back fixture");
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .expect("schema version"),
            31
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name LIKE 'organization_plan%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("no partial organization tables"),
            0
        );
    }

    #[test]
    fn schema_32_to_33_adds_rule_catalog_and_proposals_without_touching_files_or_journals() {
        let path = test_db_path();
        let db = Database::open(&path).expect("create current database");
        db.insert_file(InsertFileRequest {
            id: "schema-32-stable-file-id".to_string(),
            path: "/tmp/schema-32-stable-file-id.txt".to_string(),
            name: "schema-32-stable-file-id.txt".to_string(),
            extension: "txt".to_string(),
            size: 10,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("seed stable file");
        drop(db);

        let conn = Connection::open(&path).expect("open schema 32 fixture");
        conn.execute(
            "INSERT INTO rules (
                id, name, source, enabled, priority, weight, root_operator,
                groups_json, action_json, created_at, updated_at
             ) VALUES (
                'schema-32-rule', 'Legacy rule', 'user', 1, 1, 1, 'AND',
                '[]', '{}', 'before', 'before'
             )",
            [],
        )
        .expect("seed legacy rule");
        let files_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'files'",
                [],
                |row| row.get(0),
            )
            .expect("files schema before");
        let operation_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'operation_logs'",
                [],
                |row| row.get(0),
            )
            .expect("operation schema before");
        crate::db::drop_source_writer_schema_for_test(&conn);
        conn.execute_batch(
            r#"
            DROP TABLE rule_proposals;
            DROP TABLE rule_catalog_state;
            ALTER TABLE rules DROP COLUMN origin_proposal_id;
            ALTER TABLE rules DROP COLUMN revision;
            ALTER TABLE rules DROP COLUMN ast_version;
            DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 32;
            "#,
        )
        .expect("construct real schema 32 fixture");
        drop(conn);

        let migrated = Database::open(&path).expect("migrate schema 32 to 33");
        let conn = migrated.conn().expect("inspect schema 33");
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .expect("schema version"),
            42
        );
        assert_eq!(
            conn.query_row(
                "SELECT ast_version, revision FROM rules WHERE id = 'schema-32-rule'",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .expect("legacy rule backfill"),
            (1, 1)
        );
        assert_eq!(
            conn.query_row(
                "SELECT revision FROM rule_catalog_state WHERE singleton_id = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("catalog singleton"),
            1
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM rule_proposals", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("empty proposals"),
            0
        );
        assert_eq!(
            conn.query_row(
                "SELECT id FROM files WHERE path = '/tmp/schema-32-stable-file-id.txt'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("stable file id"),
            "schema-32-stable-file-id"
        );
        assert_eq!(
            conn.query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'files'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("files schema after"),
            files_sql
        );
        assert_eq!(
            conn.query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'operation_logs'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("operation schema after"),
            operation_sql
        );
        drop(conn);
        drop(migrated);
        Database::open(&path).expect("schema 33 ensure is idempotent");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn schema_32_to_33_conflict_rolls_back_columns_tables_and_user_version() {
        let path = test_db_path();
        let db = Database::open(&path).expect("create current database");
        drop(db);
        let conn = Connection::open(&path).expect("open schema 32 conflict fixture");
        crate::db::drop_source_writer_schema_for_test(&conn);
        conn.execute_batch(
            r#"
            DROP TABLE rule_proposals;
            DROP TABLE rule_catalog_state;
            ALTER TABLE rules DROP COLUMN origin_proposal_id;
            ALTER TABLE rules DROP COLUMN revision;
            ALTER TABLE rules DROP COLUMN ast_version;
            CREATE TABLE rule_catalog_state (wrong_column TEXT);
            DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 32;
            "#,
        )
        .expect("construct conflicting schema 32 fixture");
        drop(conn);

        let error = match Database::open(&path) {
            Ok(_) => panic!("schema 33 migration must reject conflicting catalog"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("rule_catalog_state_schema_conflict"),
            "unexpected migration error: {error}"
        );
        let conn = Connection::open(&path).expect("inspect rolled back schema 32 fixture");
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .expect("rolled back version"),
            32
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('rules')
                 WHERE name IN ('ast_version', 'revision', 'origin_proposal_id')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("no partial rule columns"),
            0
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_schema
                 WHERE type = 'table' AND name = 'rule_proposals'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("no partial proposals"),
            0
        );
        drop(conn);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn schema_30_to_31_conflict_rolls_back_file_library_migration_atomically() {
        let path = test_db_path();
        let db = Database::open(&path).expect("create schema 31 database");
        db.insert_file(InsertFileRequest {
            id: "schema-30-rollback-file".to_string(),
            path: "/tmp/schema-30-rollback-file.txt".to_string(),
            name: "schema-30-rollback-file.txt".to_string(),
            extension: "txt".to_string(),
            size: 10,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("seed rollback file");
        drop(db);

        let conn = Connection::open(&path).expect("open schema 30 conflict fixture");
        crate::db::drop_source_writer_schema_for_test(&conn);
        conn.execute_batch(
            r#"
            DROP TABLE file_user_tags;
            DROP TABLE user_tags;
            DROP TABLE library_saved_views;
            DROP TABLE library_query_state;
            DROP INDEX idx_library_files_modified;
            DROP INDEX idx_library_files_created;
            DROP INDEX idx_library_files_name;
            DROP INDEX idx_library_files_size;
            DROP INDEX idx_library_files_confidence;
            CREATE TABLE user_tags (id TEXT PRIMARY KEY);
            DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 30;
            "#,
        )
        .expect("create conflicting schema 30 fixture");
        drop(conn);

        let error = match Database::open(&path) {
            Ok(_) => panic!("schema 31 migration should reject the conflicting user_tags table"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("normalized_name"));

        let conn = Connection::open(&path).expect("inspect rolled back schema 30 fixture");
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .expect("rolled back schema version"),
            30
        );
        assert_eq!(
            conn.query_row(
                "SELECT id FROM files WHERE id = 'schema-30-rollback-file'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("legacy file survives rollback"),
            "schema-30-rollback-file"
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name IN ('file_user_tags', 'library_saved_views', 'library_query_state')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("new tables remain absent after rollback"),
            0
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('user_tags')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("conflicting table survives rollback"),
            1
        );
        drop(conn);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn schema_27_fixture_adds_watcher_columns_without_rebuilding_the_ledger() {
        let path = test_db_path();
        let db = Database::open(&path).expect("create schema 28 database");
        drop(db);

        let conn = Connection::open(&path).expect("open schema 28 fixture");
        crate::db::drop_source_writer_schema_for_test(&conn);
        conn.execute_batch(
            r#"
            ALTER TABLE scan_roots DROP COLUMN watcher_last_error_message;
            ALTER TABLE scan_roots DROP COLUMN watcher_last_error_code;
            ALTER TABLE scan_roots DROP COLUMN watcher_last_applied_at;
            ALTER TABLE scan_roots DROP COLUMN watcher_last_event_at;
            ALTER TABLE scan_roots DROP COLUMN watcher_applied_revision;
            ALTER TABLE scan_roots DROP COLUMN watcher_revision;
            ALTER TABLE scan_runs DROP COLUMN watcher_revision_at_start;
            DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 27;
            "#,
        )
        .expect("create real schema 27 fixture");
        drop(conn);

        let migrated = Database::open(&path).expect("migrate schema 27 fixture");
        let conn = migrated.conn().expect("inspect migrated schema 27 fixture");
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("schema version");
        let ledger_tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name IN ('scan_roots', 'scan_runs', 'scan_seen', 'scan_run_errors')",
                [],
                |row| row.get(0),
            )
            .expect("scan ledger tables");
        conn.execute(
            "INSERT INTO scan_roots (id, normalized_path, display_name, created_at, updated_at) VALUES ('schema-27-root', '/tmp/schema-27-root', 'schema-27-root', 1, 1)",
            [],
        )
        .expect("insert migrated root");
        let watcher_defaults: (i64, i64) = conn
            .query_row(
                "SELECT watcher_revision, watcher_applied_revision FROM scan_roots WHERE id = 'schema-27-root'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("watcher defaults");
        let rule_recovery_required: i64 = conn
            .query_row(
                "SELECT watcher_rule_recovery_required FROM scan_roots WHERE id = 'schema-27-root'",
                [],
                |row| row.get(0),
            )
            .expect("rule recovery default");
        let fabricated_dedupe_rows: i64 = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM dedupe_runs) + (SELECT COUNT(*) FROM file_fingerprints) + (SELECT COUNT(*) FROM duplicate_groups)",
                [],
                |row| row.get(0),
            )
            .expect("dedupe backfill count");

        assert_eq!(version, 42);
        assert_eq!(ledger_tables, 4);
        assert_eq!(watcher_defaults, (0, 0));
        assert_eq!(rule_recovery_required, 0);
        assert_eq!(fabricated_dedupe_rows, 0);
    }

    #[test]
    fn schema_28_to_29_conflict_rolls_back_dedupe_migration_atomically() {
        let path = test_db_path();
        let db = Database::open(&path).expect("create schema 29 fixture");
        drop(db);

        let conn = Connection::open(&path).expect("open schema fixture");
        conn.execute(
            "INSERT INTO scan_roots (id, normalized_path, display_name, created_at, updated_at) VALUES ('schema-28-root', '/tmp/schema-28-root', 'schema-28-root', 1, 1)",
            [],
        )
        .expect("seed schema 28 scan ledger");
        crate::db::drop_source_writer_schema_for_test(&conn);
        conn.execute_batch(
            r#"
            DROP VIEW IF EXISTS active_duplicate_membership;
            DROP TABLE IF EXISTS duplicate_group_members;
            DROP TABLE IF EXISTS duplicate_groups;
            DROP TABLE IF EXISTS dedupe_run_errors;
            DROP TABLE IF EXISTS file_fingerprints;
            DROP TABLE IF EXISTS dedupe_runs;
            ALTER TABLE scan_roots DROP COLUMN watcher_rule_recovery_required;
            CREATE TABLE file_fingerprints (file_id TEXT PRIMARY KEY);
            DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 28;
            "#,
        )
        .expect("create conflicting schema 28 fixture");
        drop(conn);

        let error = match Database::open(&path) {
            Ok(_) => panic!("schema 29 migration should fail on the conflicting table"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("physical_key"));

        let conn = Connection::open(&path).expect("inspect rolled back schema 28 fixture");
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("schema version");
        let root_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM scan_roots WHERE id = 'schema-28-root'",
                [],
                |row| row.get(0),
            )
            .expect("scan root survives rollback");
        assert_eq!(version, 28);
        assert_eq!(root_count, 1);
        let watcher_flag_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('scan_roots') WHERE name = 'watcher_rule_recovery_required'",
                [],
                |row| row.get(0),
            )
            .expect("watcher flag column count");
        let conflicting_columns: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('file_fingerprints')",
                [],
                |row| row.get(0),
            )
            .expect("conflicting fingerprint columns");
        assert_eq!(watcher_flag_count, 0);
        assert_eq!(conflicting_columns, 1);
        let dedupe_table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'dedupe_runs'",
                [],
                |row| row.get(0),
            )
            .expect("dedupe table count");
        assert_eq!(dedupe_table_count, 0);
    }

    #[test]
    fn get_paged_files_filters_by_library_scope_roots() {
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

        let root_a = LibraryScope::Roots {
            roots: vec!["/tmp/root-a".to_string()],
        };
        let root_b = LibraryScope::Roots {
            roots: vec!["/tmp/root-b".to_string()],
        };
        let all = LibraryScope::All;

        let page_a = db
            .get_paged_files_in_scope(Some(10), Some(0), None, &root_a)
            .expect("root a page");
        let page_b = db
            .get_paged_files_in_scope(Some(10), Some(0), None, &root_b)
            .expect("root b page");
        let page_all = db
            .get_paged_files_in_scope(Some(10), Some(0), None, &all)
            .expect("all page");

        assert_eq!(page_a.total, 1);
        assert_eq!(page_a.files[0].name, "a.pdf");
        assert_eq!(page_b.total, 1);
        assert_eq!(page_b.files[0].name, "b.pdf");
        assert_eq!(page_all.total, 2);
    }

    #[test]
    fn get_paged_files_filters_review_files_and_search_query_together() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file_at_path(
            &db,
            "file-review-pdf",
            "/tmp/root-a/invoice_review.pdf",
            "invoice_review.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );
        insert_test_file_at_path(
            &db,
            "file-review-image",
            "/tmp/root-a/invoice_review.png",
            "invoice_review.png",
            "png",
            2_048,
            1_900_000_001,
        );
        insert_test_file_at_path(
            &db,
            "file-active-pdf",
            "/tmp/root-a/project_invoice.pdf",
            "project_invoice.pdf",
            "pdf",
            2_048,
            1_900_000_002,
        );
        set_file_review_state(
            &db,
            "/tmp/root-a/invoice_review.pdf",
            "Inbox",
            "Review",
            true,
        );
        set_file_review_state(
            &db,
            "/tmp/root-a/invoice_review.png",
            "Inbox",
            "Review",
            true,
        );
        set_file_review_state(
            &db,
            "/tmp/root-a/project_invoice.pdf",
            "Active",
            "Keep",
            false,
        );

        let page = db
            .get_paged_files_in_scope_with_filter(
                Some(10),
                Some(0),
                Some("pdf"),
                &LibraryScope::Roots {
                    roots: vec!["/tmp/root-a".to_string()],
                },
                Some(&FileLibraryFilter {
                    library_filter: Some(LibraryFilter::Review),
                }),
            )
            .expect("review pdf page");

        assert_eq!(page.total, 1);
        assert_eq!(page.files[0].id, "file-review-pdf");
    }

    #[test]
    fn get_paged_files_query_plan_is_available_for_benchmark_diagnostics() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file_at_path(
            &db,
            "file-review-pdf",
            "/tmp/root-a/invoice_review.pdf",
            "invoice_review.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );
        set_file_review_state(
            &db,
            "/tmp/root-a/invoice_review.pdf",
            "Inbox",
            "Review",
            true,
        );

        let plan = db
            .explain_paged_files_query_plan(
                Some("invoice"),
                &LibraryScope::Roots {
                    roots: vec!["/tmp/root-a".to_string()],
                },
                Some(&FileLibraryFilter {
                    library_filter: Some(LibraryFilter::Review),
                }),
            )
            .expect("query plan");

        assert!(
            plan.iter()
                .any(|line| line.contains("files_fts") || line.contains("idx_files")),
            "query plan should mention the FTS table or files indexes: {plan:?}"
        );
    }

    #[test]
    fn get_paged_files_filters_library_buckets() {
        let db = Database::open(test_db_path()).expect("open test database");
        let samples = [
            ("file-active", "active.txt", "txt"),
            ("file-reference", "reference.txt", "txt"),
            ("file-keep", "keep.txt", "txt"),
            ("file-archive", "archive.txt", "txt"),
            ("file-review", "review.txt", "txt"),
            ("file-delete-candidate", "delete-candidate.txt", "txt"),
            ("file-duplicate-a", "duplicate-a.txt", "txt"),
            ("file-duplicate-b", "duplicate-b.txt", "txt"),
            ("file-sensitive", "passport.pdf", "pdf"),
        ];
        for (index, (id, name, extension)) in samples.into_iter().enumerate() {
            insert_test_file_at_path(
                &db,
                id,
                &format!("/tmp/root-a/{name}"),
                name,
                extension,
                2_048,
                1_900_000_000 + index as i64,
            );
        }
        set_file_review_state(&db, "/tmp/root-a/active.txt", "Active", "Move", false);
        set_file_review_state(&db, "/tmp/root-a/reference.txt", "Reference", "Move", false);
        set_file_review_state(&db, "/tmp/root-a/keep.txt", "Inbox", "Keep", false);
        set_file_review_state(&db, "/tmp/root-a/archive.txt", "Archive", "Archive", false);
        set_file_review_state(&db, "/tmp/root-a/review.txt", "Inbox", "Review", true);
        set_file_review_state(
            &db,
            "/tmp/root-a/delete-candidate.txt",
            "Inbox",
            "DeleteCandidate",
            false,
        );
        set_file_review_state(&db, "/tmp/root-a/duplicate-a.txt", "Inbox", "Move", false);
        set_file_review_state(&db, "/tmp/root-a/duplicate-b.txt", "Inbox", "Move", false);
        set_file_review_state(&db, "/tmp/root-a/passport.pdf", "Sensitive", "Move", true);
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.execute(
            r#"
                    UPDATE files
                    SET content_hash = 'same-content'
                    WHERE path IN ('/tmp/root-a/duplicate-a.txt', '/tmp/root-a/duplicate-b.txt')
                    "#,
            [],
        )
        .expect("set duplicate content hash");
        publish_test_duplicate_group(
            &db,
            &["file-duplicate-a", "file-duplicate-b"],
            "same-content",
        );
        conn.execute(
            "UPDATE files SET risk_level = 'Sensitive' WHERE path = '/tmp/root-a/passport.pdf'",
            [],
        )
        .expect("set sensitive risk");
        let scope = LibraryScope::Roots {
            roots: vec!["/tmp/root-a".to_string()],
        };

        let active = db
            .get_paged_files_in_scope_with_filter(
                Some(10),
                Some(0),
                None,
                &scope,
                Some(&FileLibraryFilter {
                    library_filter: Some(LibraryFilter::Active),
                }),
            )
            .expect("active page");
        let archive = db
            .get_paged_files_in_scope_with_filter(
                Some(10),
                Some(0),
                None,
                &scope,
                Some(&FileLibraryFilter {
                    library_filter: Some(LibraryFilter::Archive),
                }),
            )
            .expect("archive page");
        let review = db
            .get_paged_files_in_scope_with_filter(
                Some(10),
                Some(0),
                None,
                &scope,
                Some(&FileLibraryFilter {
                    library_filter: Some(LibraryFilter::Review),
                }),
            )
            .expect("review page");
        let duplicate = db
            .get_paged_files_in_scope_with_filter(
                Some(10),
                Some(0),
                None,
                &scope,
                Some(&FileLibraryFilter {
                    library_filter: Some(LibraryFilter::Duplicate),
                }),
            )
            .expect("duplicate page");
        let sensitive = db
            .get_paged_files_in_scope_with_filter(
                Some(10),
                Some(0),
                None,
                &scope,
                Some(&FileLibraryFilter {
                    library_filter: Some(LibraryFilter::Sensitive),
                }),
            )
            .expect("sensitive page");

        assert_eq!(active.total, 3);
        assert!(active.files.iter().any(|file| file.id == "file-active"));
        assert!(active.files.iter().any(|file| file.id == "file-reference"));
        assert!(active.files.iter().any(|file| file.id == "file-keep"));
        assert_eq!(archive.total, 1);
        assert_eq!(archive.files[0].id, "file-archive");
        assert_eq!(review.total, 3);
        assert!(review.files.iter().any(|file| file.id == "file-review"));
        assert!(review
            .files
            .iter()
            .any(|file| file.id == "file-delete-candidate"));
        assert!(review.files.iter().any(|file| file.id == "file-sensitive"));
        assert_eq!(duplicate.total, 2);
        assert!(duplicate
            .files
            .iter()
            .all(|file| file.id == "file-duplicate-a" || file.id == "file-duplicate-b"));
        assert_eq!(sensitive.total, 1);
        assert_eq!(sensitive.files[0].id, "file-sensitive");
    }

    #[test]
    fn duplicate_flags_are_global_even_when_view_scope_is_limited() {
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
        let root_a_scope = LibraryScope::Roots {
            roots: vec!["/tmp/root-a".to_string()],
        };

        let root_a_page = db
            .get_paged_files_in_scope_with_filter(Some(10), Some(0), None, &root_a_scope, None)
            .expect("root a page");
        let root_a_duplicate_filter = db
            .get_paged_files_in_scope_with_filter(
                Some(10),
                Some(0),
                None,
                &root_a_scope,
                Some(&FileLibraryFilter {
                    library_filter: Some(LibraryFilter::Duplicate),
                }),
            )
            .expect("root a duplicate page");
        let root_a_stats = db
            .get_stats_summary_in_scope(&root_a_scope)
            .expect("root a stats");

        assert_eq!(root_a_page.total, 1);
        assert_eq!(root_a_page.files[0].id, "duplicate-root-a");
        assert!(root_a_page.files[0].is_duplicate);
        assert_eq!(root_a_duplicate_filter.total, 1);
        assert_eq!(root_a_duplicate_filter.files[0].id, "duplicate-root-a");
        assert_eq!(root_a_stats.duplicate_files, 1);
    }

    #[test]
    fn get_operation_previews_for_scope_uses_full_scope_not_first_page() {
        let db = Database::open(test_db_path()).expect("open test database");
        for index in 0..60 {
            let name = format!("project-{index:02}.txt");
            let path = format!("/tmp/root-a/{name}");
            insert_test_file_at_path(
                &db,
                &format!("file-{index:02}"),
                &path,
                &name,
                "txt",
                2_048,
                1_900_000_000 + index,
            );
            set_file_operation_suggestion(
                &db,
                &path,
                "Move",
                "/tmp/root-a/ZenCanvas/20_Areas/Projects",
                &name,
                "Normal",
                0.91,
                false,
            );
        }

        let result = db
            .get_operation_previews_for_scope(
                &LibraryScope::Roots {
                    roots: vec!["/tmp/root-a".to_string()],
                },
                None,
                Some(100),
                Some(0),
            )
            .expect("scope previews");

        assert_eq!(result.total, 60);
        assert_eq!(result.previews.len(), 60);
        assert!(!result.truncated);
        assert!(!result.has_more);
        #[cfg(target_os = "macos")]
        assert!(result.previews.iter().all(|preview| {
            preview.is_executable == Some(false)
                && preview
                    .blocking_reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("safe source retirement"))
        }));
        #[cfg(not(target_os = "macos"))]
        assert!(result
            .previews
            .iter()
            .all(|preview| preview.is_executable != Some(false)));
    }

    #[test]
    fn get_operation_previews_for_scope_reports_has_more_for_partial_pages() {
        let db = Database::open(test_db_path()).expect("open test database");
        for index in 0..3 {
            let name = format!("partial-{index}.txt");
            let path = format!("/tmp/root-a/{name}");
            insert_test_file_at_path(
                &db,
                &format!("file-partial-{index}"),
                &path,
                &name,
                "txt",
                2_048,
                1_900_000_000 + index,
            );
            set_file_operation_suggestion(
                &db,
                &path,
                "Move",
                "/tmp/root-a/ZenCanvas/20_Areas/Projects",
                &name,
                "Normal",
                0.91,
                false,
            );
        }

        let first = db
            .get_operation_previews_for_scope(
                &LibraryScope::Roots {
                    roots: vec!["/tmp/root-a".to_string()],
                },
                None,
                Some(2),
                Some(0),
            )
            .expect("first page");
        let second = db
            .get_operation_previews_for_scope(
                &LibraryScope::Roots {
                    roots: vec!["/tmp/root-a".to_string()],
                },
                None,
                Some(2),
                Some(2),
            )
            .expect("second page");

        assert_eq!(first.total, 3);
        assert_eq!(first.previews.len(), 2);
        assert!(first.truncated);
        assert!(first.has_more);
        assert_eq!(second.previews.len(), 1);
        assert!(!second.truncated);
        assert!(!second.has_more);
    }

    #[test]
    fn get_operation_previews_for_scope_marks_sensitive_files_blocked() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file_at_path(
            &db,
            "file-sensitive",
            "/tmp/root-a/passport.pdf",
            "passport.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );
        set_file_operation_suggestion(
            &db,
            "/tmp/root-a/passport.pdf",
            "Move",
            "/tmp/root-a/ZenCanvas/20_Areas/Identity",
            "passport.pdf",
            "Sensitive",
            0.95,
            true,
        );

        let result = db
            .get_operation_previews_for_scope(
                &LibraryScope::Roots {
                    roots: vec!["/tmp/root-a".to_string()],
                },
                None,
                Some(100),
                Some(0),
            )
            .expect("scope previews");
        let preview = result.previews.first().expect("sensitive preview");

        assert_eq!(result.total, 1);
        assert_eq!(preview.file_id, "file-sensitive");
        assert_eq!(preview.is_executable, Some(false));
        assert_eq!(preview.selected_by_default, Some(false));
        assert!(preview.requires_confirmation);
        assert!(preview
            .blocking_reason
            .as_deref()
            .unwrap_or_default()
            .contains("Sensitive"));
    }

    #[test]
    fn get_operation_previews_for_scope_blocks_existing_target_paths() {
        let root = test_dir();
        let inbox = root.join("inbox");
        let target_directory = root.join("organized").join("Media").join("Images");
        let source = inbox.join("image.png");
        let target = target_directory.join("image.png");
        fs::create_dir_all(&inbox).expect("create inbox");
        fs::create_dir_all(&target_directory).expect("create target directory");
        fs::write(&source, b"source").expect("write source");
        fs::write(&target, b"existing target").expect("write existing target");

        let db = Database::open(test_db_path()).expect("open test database");
        let source_text = normalized_test_path(&source);
        let inbox_text = normalized_test_path(&inbox);
        insert_test_file_at_path(
            &db,
            "file-target-conflict",
            &source_text,
            "image.png",
            "png",
            6,
            1_900_000_000,
        );
        set_file_operation_suggestion(
            &db,
            &source_text,
            "Move",
            &normalized_test_path(&target_directory),
            "image.png",
            "Normal",
            0.95,
            false,
        );

        let result = db
            .get_operation_previews_for_scope(
                &LibraryScope::Roots { roots: vec![inbox_text] },
                None,
                Some(100),
                Some(0),
            )
            .expect("scope previews");
        let preview = result.previews.first().expect("conflict preview");

        assert_eq!(preview.is_executable, Some(false));
        assert_eq!(preview.selected_by_default, Some(false));
        assert!(preview
            .blocking_reason
            .as_deref()
            .unwrap_or_default()
            .contains("already exists"));
        assert_eq!(fs::read(&target).expect("read target"), b"existing target");
    }

    #[test]
    fn operation_preview_preserves_indexed_extensions_and_blocks_legacy_changes() {
        let db = Database::open(test_db_path()).expect("open test database");
        let cases = [
            (
                "preview-lnk",
                "/tmp/root-a/Install_Package.lnk",
                "Install_Package.lnk",
                "lnk",
                "Install_Package",
                "Install_Package.lnk",
            ),
            (
                "preview-url",
                "/tmp/root-a/Website.url",
                "Website.url",
                "url",
                "Website_Archive",
                "Website_Archive.url",
            ),
            (
                "preview-appref",
                "/tmp/root-a/Product.appref-ms",
                "Product.appref-ms",
                "appref-ms",
                "Product_Archive.appref-ms",
                "Product_Archive.appref-ms",
            ),
            (
                "preview-pdf",
                "/tmp/root-a/Report.pdf",
                "Report.pdf",
                "pdf",
                "Report_2026",
                "Report_2026.pdf",
            ),
            (
                "preview-uppercase",
                "/tmp/root-a/My_Shortcut.LNK",
                "My_Shortcut.LNK",
                "lnk",
                "Renamed.lnk",
                "Renamed.LNK",
            ),
            (
                "preview-no-extension",
                "/tmp/root-a/README",
                "README",
                "",
                "README_Archive",
                "README_Archive",
            ),
            (
                "preview-dotfile",
                "/tmp/root-a/.gitignore",
                ".gitignore",
                "",
                ".gitignore",
                ".gitignore",
            ),
        ];

        for (id, path, name, extension, suggested_name, _expected_name) in cases {
            insert_test_file_at_path(
                &db,
                id,
                path,
                name,
                extension,
                2_048,
                1_900_000_000,
            );
            set_file_operation_suggestion(
                &db,
                path,
                "Move",
                "/tmp/root-a/ZenCanvas/20_Areas/Projects",
                suggested_name,
                "Normal",
                0.95,
                false,
            );
        }
        insert_test_file_at_path(
            &db,
            "preview-legacy-change",
            "/tmp/root-a/Install_Package_legacy.lnk",
            "Install_Package_legacy.lnk",
            "lnk",
            2_048,
            1_900_000_000,
        );
        set_file_operation_suggestion(
            &db,
            "/tmp/root-a/Install_Package_legacy.lnk",
            "Rename",
            "",
            "Install_Package_legacy.exe",
            "Normal",
            0.95,
            false,
        );

        let previews = db
            .get_operation_previews_for_scope(&LibraryScope::All, None, Some(100), Some(0))
            .expect("operation previews")
            .previews;

        for (id, _, _, _, _, expected_name) in cases {
            let preview = previews
                .iter()
                .find(|preview| preview.file_id == id)
                .expect("preview for case");
            assert_eq!(preview.new_name, expected_name, "{id}");
            #[cfg(target_os = "macos")]
            {
                assert_eq!(preview.is_executable, Some(false), "{id}");
                assert!(preview
                    .blocking_reason
                    .as_deref()
                    .unwrap_or_default()
                    .contains("safe source retirement"), "{id}");
            }
            #[cfg(not(target_os = "macos"))]
            assert_ne!(preview.is_executable, Some(false), "{id}");
        }

        let blocked = previews
            .iter()
            .find(|preview| preview.file_id == "preview-legacy-change")
            .expect("blocked legacy preview");
        assert_eq!(blocked.is_executable, Some(false));
        assert_eq!(blocked.selected_by_default, Some(false));
        assert!(blocked.requires_confirmation);
        assert!(blocked
            .blocking_reason
            .as_deref()
            .unwrap_or_default()
            .contains("Changing a file extension is not allowed"));
        assert_eq!(blocked.new_name, blocked.old_name);
    }

    #[test]
    fn get_stats_summary_aggregates_files_and_types() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(&db, "file-1", "report.pdf", "pdf", 2_048, 1_800_000_000);
        insert_test_file(&db, "file-2", "photo.jpg", "jpg", 4_096, 1_900_000_000);

        let stats = db.get_stats_summary().expect("stats");

        assert_eq!(stats.total_files, 2);
        assert_eq!(stats.total_size, 6_144);
        assert_eq!(stats.by_type.get("Document"), Some(&1));
        assert_eq!(stats.by_type.get("Image"), Some(&1));
        assert_eq!(stats.by_lifecycle.get("Inbox"), Some(&2));
    }

    #[test]
    fn get_stats_summary_filters_by_library_scope_roots() {
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

        let stats = db
            .get_stats_summary_in_scope(&LibraryScope::Roots {
                roots: vec!["/tmp/root-a".to_string()],
            })
            .expect("root a stats");

        assert_eq!(stats.total_files, 1);
        assert_eq!(stats.total_size, 2_048);
        assert_eq!(stats.by_type.get("Document"), Some(&1));
        assert_eq!(stats.by_lifecycle.get("Inbox"), Some(&1));
    }

    #[test]
    fn stats_summary_failure_rolls_back_transaction_state() {
        let db = Database::open(test_db_path()).expect("open test database");
        {
            let conn = db.conn().expect("get connection");
            conn.execute_batch("ALTER TABLE files RENAME TO files_broken")
                .expect("break stats query");
        }

        let error = db
            .get_stats_summary_in_scope(&LibraryScope::All)
            .expect_err("stats should fail against broken schema");
        assert!(
            error.to_string().contains("files"),
            "unexpected stats error: {error}"
        );

        let conn = db.conn().expect("get pooled connection after failed stats");
        assert!(
            conn.is_autocommit(),
            "failed stats query left the pooled SQLite connection inside a transaction"
        );
        conn.execute_batch("BEGIN IMMEDIATE; ROLLBACK;")
            .expect("connection accepts a new transaction after failed stats");
    }

    #[test]
    fn remove_files_by_paths_marks_file_stale() {
        let db = Database::open(test_db_path()).expect("open test database");
        db.insert_file(InsertFileRequest {
            id: "dir-project".to_string(),
            path: "/test/virtual/documents/project".to_string(),
            name: "project".to_string(),
            extension: String::new(),
            size: 0,
            mtime: 1_900_000_000,
            ctime: 0,
            is_dir: true,
            state_code: 0,
        })
        .expect("insert project directory");
        db.insert_file(InsertFileRequest {
            id: "file-ghost".to_string(),
            path: "/test/virtual/documents/project/ghost-report.pdf".to_string(),
            name: "ghost-report.pdf".to_string(),
            extension: "pdf".to_string(),
            size: 2_048,
            mtime: 1_900_000_001,
            ctime: 0,
            is_dir: false,
            state_code: 0,
        })
        .expect("insert child file");
        db.insert_file(InsertFileRequest {
            id: "file-survivor".to_string(),
            path: "/test/virtual/documents/project-other/survivor.pdf".to_string(),
            name: "survivor.pdf".to_string(),
            extension: "pdf".to_string(),
            size: 2_048,
            mtime: 1_900_000_002,
            ctime: 0,
            is_dir: false,
            state_code: 0,
        })
        .expect("insert sibling file");

        let removed = db
            .remove_files_by_paths(&["/test/virtual/documents/project".to_string()])
            .expect("remove paths");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");
        let ghost_search = db.search_files("ghost-report", Some(10)).expect("search");

        assert_eq!(removed, 2);
        assert_eq!(page.total, 1);
        assert_eq!(page.files[0].id, "file-survivor");
        assert!(ghost_search.is_empty());
        assert_eq!(
            stale_state(&db, "/test/virtual/documents/project"),
            Some((true, true))
        );
        assert_eq!(
            stale_state(&db, "/test/virtual/documents/project/ghost-report.pdf"),
            Some((true, true))
        );
    }

    #[test]
    fn mark_missing_files_stale_after_scan_marks_only_old_entries_under_root() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file_at_path(
            &db,
            "active-report",
            "/test/virtual/documents/project/active-report.txt",
            "active-report.txt",
            "txt",
            100,
            1_900_000_010,
        );
        insert_test_file_at_path(
            &db,
            "ghost-report",
            "/test/virtual/documents/project/ghost-report.txt",
            "ghost-report.txt",
            "txt",
            100,
            1_900_000_000,
        );
        insert_test_file_at_path(
            &db,
            "sibling-report",
            "/test/virtual/documents/project-other/sibling-report.txt",
            "sibling-report.txt",
            "txt",
            100,
            1_900_000_000,
        );
        let conn = Connection::open(db.path()).expect("open migrated database");
        conn.execute(
            "UPDATE files SET last_seen_at = 300 WHERE id = 'active-report'",
            [],
        )
        .expect("set active last_seen_at");
        conn.execute(
            "UPDATE files SET last_seen_at = 100 WHERE id IN ('ghost-report', 'sibling-report')",
            [],
        )
        .expect("set stale candidates last_seen_at");

        let marked = db
            .mark_missing_files_stale_after_scan("/test/virtual/documents/project", 200)
            .expect("mark missing stale");
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");
        let ghost_search = db.search_files("ghost-report", Some(10)).expect("search");

        assert_eq!(marked, 1);
        assert!(ghost_search.is_empty());
        assert_eq!(
            stale_state(&db, "/test/virtual/documents/project/active-report.txt"),
            Some((false, true))
        );
        assert_eq!(
            stale_state(&db, "/test/virtual/documents/project/ghost-report.txt"),
            Some((true, true))
        );
        assert_eq!(
            stale_state(
                &db,
                "/test/virtual/documents/project-other/sibling-report.txt"
            ),
            Some((false, true))
        );
        assert_eq!(page.total, 2);
    }

    #[test]
    fn insert_files_revives_stale_file() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(
            &db,
            "file-report",
            "report.pdf",
            "pdf",
            2_048,
            1_900_000_000,
        );
        db.remove_files_by_paths(&["/test/virtual/documents/report.pdf".to_string()])
            .expect("mark stale");
        assert_eq!(
            stale_state(&db, "/test/virtual/documents/report.pdf"),
            Some((true, true))
        );

        insert_test_file(
            &db,
            "file-report",
            "report.pdf",
            "pdf",
            4_096,
            1_900_000_100,
        );
        let page = db.get_paged_files(Some(10), Some(0), None).expect("page");

        assert_eq!(page.total, 1);
        assert_eq!(page.files[0].id, "file-report");
        assert_eq!(page.files[0].size, 4_096);
        assert_eq!(
            stale_state(&db, "/test/virtual/documents/report.pdf"),
            Some((false, true))
        );
    }

    #[test]
    fn search_files_excludes_stale_files() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(
            &db,
            "file-report",
            "report.txt",
            "txt",
            2_048,
            1_900_000_000,
        );

        let before = db.search_files("report", Some(10)).expect("search before");
        db.remove_files_by_paths(&["/test/virtual/documents/report.txt".to_string()])
            .expect("mark stale");
        let after = db.search_files("report", Some(10)).expect("search after");

        assert_eq!(before.len(), 1);
        assert!(after.is_empty());
        assert_eq!(
            stale_state(&db, "/test/virtual/documents/report.txt"),
            Some((true, true))
        );
    }

    #[test]
    fn optimize_search_index_returns_duration() {
        let db = Database::open(test_db_path()).expect("open test database");
        insert_test_file(
            &db,
            "file-report",
            "report.txt",
            "txt",
            2_048,
            1_900_000_000,
        );

        let duration_ms = db.optimize_search_index().expect("optimize search index");
        let results = db.search_files("report", Some(10)).expect("search");

        assert!(duration_ms <= 60_000);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "report.txt");
    }
