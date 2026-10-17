use serde_json::json;

#[test]
fn schema_30_creates_analysis_ledger_without_fabricated_history() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open schema 30 database");
    let conn = db.conn().expect("database connection");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("schema version");
    assert_eq!(version, 42);
    for table in [
        "dedupe_authority_state",
        "analysis_runs",
        "analysis_run_detectors",
        "analysis_findings",
        "analysis_finding_evidence",
        "analysis_finding_decisions",
    ] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                params![table],
                |row| row.get(0),
            )
            .expect("analysis table lookup");
        assert_eq!(count, 1, "missing schema 30 table {table}");
    }
    let authority: (String, i64) = conn
        .query_row(
            "SELECT status, revision FROM dedupe_authority_state WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("authority singleton");
    assert_eq!(authority, ("rebuild_required".to_string(), 1));
    let history_counts: (i64, i64, i64, i64, i64) = conn
        .query_row(
            "SELECT (SELECT COUNT(*) FROM analysis_runs), (SELECT COUNT(*) FROM analysis_findings), (SELECT COUNT(*) FROM analysis_finding_evidence), (SELECT COUNT(*) FROM analysis_finding_decisions), (SELECT COUNT(*) FROM dedupe_runs)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .expect("history counts");
    assert_eq!(history_counts, (0, 0, 0, 0, 0));
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name IN ('operation_batches', 'operation_logs', 'cleanup_trash_batches', 'cleanup_trash_items')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("legacy journal tables"),
        4
    );
    drop(conn);
    drop(db);
    Database::open(&path).expect("schema 30 reopen is idempotent");
}

#[test]
fn schema_33_to_34_creates_content_ledger_without_rewriting_core_tables() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open current database");
    db.insert_file(InsertFileRequest {
        id: "schema-34-stable-file".into(),
        path: "/tmp/schema-34-stable-file.txt".into(),
        name: "schema-34-stable-file.txt".into(),
        extension: "txt".into(),
        size: 7,
        mtime: 11,
        ctime: 11,
        is_dir: false,
        state_code: 0,
    })
    .expect("seed stable file");
    let conn = db.conn().expect("open database connection");
    let files_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='files'",
            [],
            |row| row.get(0),
        )
        .expect("files schema");
    let journal_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='operation_logs'",
            [],
            |row| row.get(0),
        )
        .expect("journal schema");
    drop(conn);
    drop(db);

    let conn = Connection::open(&path).expect("open schema 33 fixture");
    crate::db::drop_source_writer_schema_for_test(&conn);
    conn.execute_batch(
        r#"
        DROP TRIGGER IF EXISTS content_artifacts_file_changed;
        DROP TABLE content_artifact_fts;
        DROP TABLE content_artifacts;
        DROP TABLE content_run_items;
        DROP TABLE content_runs;
        DROP TABLE content_scope_policies;
        DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 33;
        "#,
    )
    .expect("construct schema 33 fixture");
    drop(conn);

    let migrated = Database::open(&path).expect("migrate schema 33 to 34");
    let conn = migrated.conn().expect("inspect schema 34");
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("schema version"),
        42
    );
    for table in [
        "content_scope_policies",
        "content_runs",
        "content_run_items",
        "content_artifacts",
        "content_artifact_fts",
    ] {
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name=?1",
                params![table],
                |row| row.get::<_, i64>(0),
            )
            .expect("content table lookup"),
            1,
            "missing content table {table}"
        );
    }
    assert_eq!(
        conn.query_row(
            "SELECT id FROM files WHERE id='schema-34-stable-file'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("stable file id"),
        "schema-34-stable-file"
    );
    assert_eq!(
        conn.query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='files'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("files schema after"),
        files_sql
    );
    assert_eq!(
        conn.query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='operation_logs'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("journal schema after"),
        journal_sql
    );
    drop(conn);
    drop(migrated);
    Database::open(&path).expect("schema 34 reopen is idempotent");
}

#[test]
fn schema_33_to_34_conflict_rolls_back_content_ledger_atomically() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open current database");
    drop(db);
    let conn = Connection::open(&path).expect("open schema 33 conflict fixture");
    crate::db::drop_source_writer_schema_for_test(&conn);
    conn.execute_batch(
        r#"
        DROP TRIGGER IF EXISTS content_artifacts_file_changed;
        DROP TABLE content_artifact_fts;
        DROP TABLE content_artifacts;
        DROP TABLE content_run_items;
        DROP TABLE content_runs;
        DROP TABLE content_scope_policies;
        CREATE TABLE content_runs (id TEXT PRIMARY KEY, wrong TEXT);
        DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 33;
        "#,
    )
    .expect("construct conflicting schema 33 fixture");
    drop(conn);

    let error = match Database::open(&path) {
        Ok(_) => panic!("schema 34 migration must reject the conflicting content table"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("content_runs")
            || error.to_string().contains("no such column")
            || error.to_string().contains("schema_conflict"),
        "unexpected migration error: {error}"
    );
    let conn = Connection::open(&path).expect("inspect rolled back content fixture");
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("rolled back schema version"),
        33
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name IN ('content_scope_policies','content_run_items','content_artifacts')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("no partial content tables"),
        0
    );
}

#[test]
fn schema_29_to_30_conflict_rolls_back_analysis_migration_atomically() {
    let path = test_db_path();
    let db = Database::open(&path).expect("create schema 30 fixture");
    drop(db);
    let conn = Connection::open(&path).expect("open fixture");
    crate::db::drop_source_writer_schema_for_test(&conn);
    conn.execute_batch(
        r#"
        DROP TABLE analysis_finding_evidence;
        DROP TABLE analysis_findings;
        DROP TABLE analysis_run_detectors;
        DROP TABLE analysis_finding_decisions;
        DROP TABLE analysis_runs;
        DROP TABLE dedupe_authority_state;
        CREATE TABLE analysis_runs (id TEXT PRIMARY KEY);
        DROP TABLE IF EXISTS automation_trigger_state; DROP TABLE IF EXISTS automation_runs; DROP TABLE IF EXISTS automation_intents; ALTER TABLE files DROP COLUMN filesystem_observation_key; ALTER TABLE scan_roots DROP COLUMN library_change_revision; PRAGMA user_version = 29;
        "#,
    )
    .expect("create conflicting schema 29 fixture");
    drop(conn);

    let error = match Database::open(&path) {
        Ok(_) => panic!("schema 29 to 30 migration should reject the conflicting table"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("scope_hash"));
    let conn = Connection::open(&path).expect("inspect rolled back fixture");
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("schema version"),
        29
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('analysis_runs')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("conflicting table survives rollback"),
        1
    );
}

#[test]
fn schema_30_reopen_preserves_analysis_run_finding_evidence_and_decision() {
    let path = test_db_path();
    let root = test_dir();
    fs::write(root.join("candidate.txt"), b"candidate").expect("write candidate");
    let db = Database::open(&path).expect("open schema 30 persistence fixture");
    let detector_set = vec!["cleanup_heuristics_v1".to_string()];
    let detector_pairs = vec![("cleanup_heuristics_v1".to_string(), 1_i64)];
    let run = db
        .start_analysis_run(
            &StartAnalysisRunRequest {
                scope: AnalysisScopeRequest {
                    kind: "approved_cleanup_paths".to_string(),
                    root_ids: Vec::new(),
                    paths: vec![root.to_string_lossy().into_owned()],
                },
                detector_ids: detector_set,
                request_key: Some("analysis-reopen-persistence".to_string()),
            },
            &detector_pairs,
        )
        .expect("start persistence run")
        .run;
    complete_test_analysis_run(&db, &run.id, &run.scope_hash, "reopen-persistence-key");
    let finding = active_analysis_finding(&db, &run.id);
    db.set_analysis_finding_decision(
        &finding.finding_key,
        "acknowledged",
        None,
        Some("persisted review"),
        0,
    )
    .expect("persist analysis decision");
    drop(db);

    let reopened = Database::open(&path).expect("reopen schema 30 persistence fixture");
    let reopened_run = reopened.get_analysis_run(&run.id).expect("reopened run");
    assert_eq!(reopened_run.status, "completed");
    let reopened_finding = reopened
        .get_analysis_finding(&finding.id)
        .expect("reopened finding")
        .expect("finding persisted");
    assert_eq!(reopened_finding.decision.as_deref(), Some("acknowledged"));
    assert_eq!(
        reopened
            .list_analysis_finding_evidence(&finding.id)
            .expect("reopened evidence")
            .len(),
        1
    );
}

#[test]
#[ignore = "Task 03 analysis finding/WAL benchmark; invoked by npm run test:performance"]
fn performance_task03_analysis_100k_findings_and_wal_reader() {
    const FINDING_COUNT: usize = 100_000;
    let path = test_db_path();
    let db = Database::open(&path).expect("open schema 30 analysis benchmark database");
    let conn = db.conn().expect("analysis benchmark connection");
    conn.execute(
        "INSERT INTO analysis_runs (id, request_key, scope_json, scope_hash, source_snapshot_json, source_snapshot_hash, detector_set_json, detector_set_hash, status, phase, created_at, updated_at) VALUES ('analysis-performance-run', 'analysis-performance-request', '{\"kind\":\"all_managed_file_library\"}', 'analysis-performance-scope', '{}', 'analysis-performance-source', '[\"large_file_v1:v1\"]', 'analysis-performance-detectors', 'completed', 'completed', 1, 1)",
        [],
    )
    .expect("insert analysis benchmark run");
    let tx = conn
        .unchecked_transaction()
        .expect("start analysis finding fixture transaction");
    for index in 0..FINDING_COUNT {
        let tier = match index % 3 {
            0 => "safe",
            1 => "review",
            _ => "caution",
        };
        let size = 100 * 1024 * 1024 + index as i64;
        tx.execute(
            "INSERT INTO analysis_findings (id, finding_key, run_id, detector_id, detector_version, scope_hash, status, tier, category, action_kind, title, reason, confidence, size_bytes, potential_reclaimable_bytes, primary_subject_kind, primary_subject_id, path_snapshot, identity_snapshot_json, evidence_summary_json, created_at, updated_at, published_at) VALUES (?1, ?2, 'analysis-performance-run', 'large_file_v1', 1, 'analysis-performance-scope', 'active', ?3, 'large_file', 'reveal', 'Large file', 'Performance fixture', 'estimated', ?4, 0, 'managed_file', ?2, ?5, '{}', '{}', 1, 1, 1)",
            params![
                format!("analysis-performance-finding-{index:06}"),
                format!("analysis-performance-key-{index:06}"),
                tier,
                size,
                format!("/tmp/analysis-performance/file-{index:06}.bin")
            ],
        )
        .expect("insert analysis benchmark finding");
    }
    tx.execute(
        "INSERT INTO analysis_finding_decisions (finding_key, decision, note, revision, created_at, updated_at) VALUES ('analysis-performance-key-000000', 'acknowledged', 'benchmark decision', 1, 1, 1)",
        [],
    )
    .expect("insert analysis benchmark decision");
    tx.execute(
        "INSERT INTO analysis_finding_evidence (id, finding_id, evidence_kind, subject_kind, subject_id, path_snapshot, value_json, created_at) VALUES ('analysis-performance-evidence', 'analysis-performance-finding-000000', 'benchmark', 'managed_file', 'analysis-performance-key-000000', '/tmp/analysis-performance/file-000000.bin', '{}', 1)",
        [],
    )
    .expect("insert analysis benchmark evidence");
    tx.commit().expect("commit 100k analysis findings");
    drop(conn);

    let list_started = std::time::Instant::now();
    let page = db
        .list_analysis_findings(&AnalysisFindingFilter {
            run_id: Some("analysis-performance-run".to_string()),
            status: Some("active".to_string()),
            ..AnalysisFindingFilter::default()
        }, None, 500)
        .expect("page analysis findings");
    let list_elapsed = list_started.elapsed();
    assert_eq!(page.findings.len(), 200);

    let filter_started = std::time::Instant::now();
    let filtered_page = db
        .list_analysis_findings(
            &AnalysisFindingFilter {
                run_id: Some("analysis-performance-run".to_string()),
                status: Some("active".to_string()),
                tier: Some("review".to_string()),
                ..AnalysisFindingFilter::default()
            },
            None,
            200,
        )
        .expect("filter analysis findings");
    let filter_elapsed = filter_started.elapsed();
    assert_eq!(filtered_page.findings.len(), 200);

    let detail_started = std::time::Instant::now();
    let detail = db
        .get_analysis_finding("analysis-performance-finding-000000")
        .expect("detail analysis finding")
        .expect("analysis finding detail");
    let evidence = db
        .list_analysis_finding_evidence(&detail.id)
        .expect("analysis finding evidence detail");
    let detail_elapsed = detail_started.elapsed();
    assert_eq!(detail.decision.as_deref(), Some("acknowledged"));
    assert_eq!(evidence.len(), 1);

    let reader = Connection::open(&path).expect("open analysis WAL reader");
    reader
        .execute_batch("PRAGMA journal_mode = WAL;")
        .expect("enable analysis WAL reader mode");
    let reader_started = std::time::Instant::now();
    let count: i64 = reader
        .query_row("SELECT COUNT(*) FROM analysis_findings", [], |row| row.get(0))
        .expect("read analysis findings from WAL reader");
    let reader_elapsed = reader_started.elapsed();
    assert_eq!(count, FINDING_COUNT as i64);
    println!(
        "Task 03 analysis performance: findings={FINDING_COUNT}, page_200_ms={:.3}, filter_review_page_200_ms={:.3}, decision_detail_evidence_ms={:.3}, wal_reader_count_ms={:.3}",
        list_elapsed.as_secs_f64() * 1000.0,
        filter_elapsed.as_secs_f64() * 1000.0,
        detail_elapsed.as_secs_f64() * 1000.0,
        reader_elapsed.as_secs_f64() * 1000.0,
    );
    drop(reader);
    drop(db);
    let _ = fs::remove_file(path);
}

#[test]
#[ignore = "Task 03 10k finding publication benchmark; invoked by npm run test:performance"]
fn performance_task03_10k_finding_publication_transaction() {
    const FINDING_COUNT: usize = 10_000;
    let path = test_db_path();
    let root = test_dir();
    fs::write(root.join("publication-fixture.txt"), b"publication").expect("write fixture");
    let db = Database::open(&path).expect("open publication benchmark database");
    let detector_set = vec![("cleanup_heuristics_v1".to_string(), 1_i64)];
    let run = db
        .start_analysis_run(
            &StartAnalysisRunRequest {
                scope: AnalysisScopeRequest {
                    kind: "approved_cleanup_paths".to_string(),
                    root_ids: Vec::new(),
                    paths: vec![root.to_string_lossy().into_owned()],
                },
                detector_ids: vec!["cleanup_heuristics_v1".to_string()],
                request_key: Some("analysis-publication-performance".to_string()),
            },
            &detector_set,
        )
        .expect("start publication benchmark run")
        .run;
    db.claim_analysis_run(&run.id)
        .expect("claim publication benchmark run")
        .expect("publication benchmark run claimed");
    let detector = db
        .list_analysis_run_detectors(&run.id)
        .expect("publication benchmark detector")
        .pop()
        .expect("publication benchmark detector row");
    let running = db
        .set_analysis_detector_status(
            &run.id,
            &detector.detector_id,
            detector.revision,
            "running",
            FINDING_COUNT as i64,
            0,
            0,
            FINDING_COUNT as i64,
            None,
            None,
        )
        .expect("start publication benchmark detector");
    let drafts = (0..FINDING_COUNT)
        .map(|index| FindingDraft {
            id: format!("analysis-publication-finding-{index:05}"),
            finding_key: format!("analysis-publication-key-{index:05}"),
            detector_id: "cleanup_heuristics_v1".to_string(),
            detector_version: 1,
            tier: "review".to_string(),
            category: "benchmark".to_string(),
            action_kind: "reveal".to_string(),
            title: "Publication benchmark finding".to_string(),
            reason: "Bounded publication fixture".to_string(),
            risk_note: None,
            confidence: "estimated".to_string(),
            size_bytes: 1,
            exact_reclaimable_bytes: None,
            potential_reclaimable_bytes: 1,
            requires_confirmation: true,
            executable: false,
            primary_subject_kind: "approved_path".to_string(),
            primary_subject_id: format!("benchmark-subject-{index:05}"),
            path_snapshot: Some(format!("{}/publication-{index:05}.tmp", root.to_string_lossy())),
            identity_snapshot: json!({"index": index}),
            evidence_summary: json!({"benchmark": true}),
            evidence: Vec::new(),
        })
        .collect::<Vec<_>>();
    db.stage_analysis_findings(&run.id, &run.scope_hash, &drafts)
        .expect("stage publication benchmark findings");
    db.set_analysis_detector_status(
        &run.id,
        &detector.detector_id,
        running.revision,
        "completed",
        FINDING_COUNT as i64,
        FINDING_COUNT as i64,
        0,
        FINDING_COUNT as i64,
        None,
        None,
    )
    .expect("finish publication benchmark detector");
    let started = std::time::Instant::now();
    assert_eq!(
        db.publish_analysis_run(&run.id)
            .expect("publish 10k findings"),
        AnalysisPublishOutcome::Completed
    );
    let elapsed = started.elapsed();
    assert_eq!(
        db.count_analysis_findings_for_run(&run.id, "active")
            .expect("published finding count"),
        FINDING_COUNT as i64
    );
    println!(
        "Task 03 publication performance: findings={FINDING_COUNT}, publication_10k_ms={:.3}",
        elapsed.as_secs_f64() * 1000.0
    );
    drop(db);
    let _ = fs::remove_file(path);
}

#[test]
fn analysis_prune_uses_one_global_child_first_row_budget_and_wal_reader() {
    const RETENTION_FIXTURE_ROWS: usize = 1100;
    let path = test_db_path();
    let db = Database::open(&path).expect("open analysis prune budget database");
    let mut conn = db.conn().expect("open analysis prune fixture connection");
    let tx = conn
        .transaction()
        .expect("start analysis prune fixture transaction");
    tx.execute(
        "INSERT INTO analysis_runs (id, request_key, scope_json, scope_hash, source_snapshot_json, source_snapshot_hash, detector_set_json, detector_set_hash, status, phase, created_at, updated_at, finished_at) VALUES ('analysis-prune-budget-run', 'analysis-prune-budget-request', '{\"kind\":\"approved_cleanup_paths\"}', 'analysis-prune-budget-scope', '{}', 'analysis-prune-budget-source', '[\"cleanup_heuristics_v1:v1\"]', 'analysis-prune-budget-detectors', 'completed', 'completed', 1, 1, 1)",
        [],
    )
    .expect("insert old terminal analysis run");
    for index in 0..RETENTION_FIXTURE_ROWS {
        let finding_id = format!("analysis-prune-finding-{index:04}");
        let finding_key = format!("analysis-prune-key-{index:04}");
        let evidence_id = format!("analysis-prune-evidence-{index:04}");
        tx.execute(
            "INSERT INTO analysis_findings (id, finding_key, run_id, detector_id, detector_version, scope_hash, status, tier, category, action_kind, title, reason, confidence, size_bytes, potential_reclaimable_bytes, requires_confirmation, executable, primary_subject_kind, primary_subject_id, path_snapshot, identity_snapshot_json, evidence_summary_json, revision, created_at, updated_at) VALUES (?1, ?2, 'analysis-prune-budget-run', 'cleanup_heuristics_v1', 1, 'analysis-prune-budget-scope', 'stale', 'review', 'retention_fixture', 'reveal', 'Retention fixture', 'Retention fixture', 'estimated', 1, 1, 1, 0, 'approved_path', ?2, ?3, '{}', '{}', 1, 1, 1)",
            params![finding_id, finding_key, format!("/tmp/analysis-prune/{index:04}.bin")],
        )
        .expect("insert old analysis finding");
        tx.execute(
            "INSERT INTO analysis_finding_evidence (id, finding_id, evidence_kind, subject_kind, subject_id, path_snapshot, value_json, created_at) VALUES (?1, ?2, 'retention_fixture', 'approved_path', ?3, ?3, '{}', 1)",
            params![evidence_id, finding_id, format!("/tmp/analysis-prune/{index:04}.bin")],
        )
        .expect("insert old analysis evidence");
    }
    tx.commit().expect("commit analysis prune fixture");
    drop(conn);

    let deleted = db
        .prune_analysis_artifacts()
        .expect("prune analysis artifacts");
    assert_eq!(deleted, 1000, "one global budget must cap physical row deletes");

    let reader = Connection::open(&path).expect("open WAL reader after analysis prune");
    reader
        .busy_timeout(std::time::Duration::from_secs(5))
        .expect("set analysis prune reader timeout");
    assert_eq!(
        reader
            .query_row("SELECT COUNT(*) FROM analysis_findings", [], |row| row
                .get::<_, i64>(0))
            .expect("read retained findings from WAL"),
        RETENTION_FIXTURE_ROWS as i64,
    );
    assert_eq!(
        reader
            .query_row("SELECT COUNT(*) FROM analysis_finding_evidence", [], |row| row
                .get::<_, i64>(0))
            .expect("read retained evidence from WAL"),
        (RETENTION_FIXTURE_ROWS - 1000) as i64,
    );
    assert_eq!(
        reader
            .query_row(
                "SELECT COUNT(*) FROM analysis_runs WHERE id = 'analysis-prune-budget-run'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("verify child-bearing run remains"),
        1,
    );
}

#[test]
fn analysis_runs_are_idempotent_revisioned_and_retryable_without_overwriting_active_findings() {
    let root = test_dir();
    fs::write(root.join("candidate.txt"), b"candidate").expect("write candidate");
    let db = Database::open(test_db_path()).expect("open analysis database");
    let detector_set = vec![("cleanup_heuristics_v1".to_string(), 1_i64)];
    let request = |key: &str| StartAnalysisRunRequest {
        scope: AnalysisScopeRequest {
            kind: "approved_cleanup_paths".to_string(),
            root_ids: Vec::new(),
            paths: vec![root.to_string_lossy().into_owned()],
        },
        detector_ids: vec!["cleanup_heuristics_v1".to_string()],
        request_key: Some(key.to_string()),
    };

    let first = db
        .start_analysis_run(&request("analysis-idempotency"), &detector_set)
        .expect("start first analysis")
        .run;
    let repeated = db
        .start_analysis_run(&request("analysis-idempotency"), &detector_set)
        .expect("repeat start")
        .run;
    assert_eq!(first.id, repeated.id);
    let coalesced = db
        .start_analysis_run(&request("analysis-other-request"), &detector_set)
        .expect("coalesce active analysis")
        .run;
    assert_eq!(coalesced.id, first.id);
    assert!(coalesced.rerun_required);

    let claimed = db
        .claim_analysis_run(&first.id)
        .expect("claim first analysis")
        .expect("queued run claimed");
    assert!(db
        .checkpoint_analysis_run(
            &first.id,
            claimed.revision.saturating_sub(1),
            "running_detectors",
            0,
            0,
        )
        .is_err());
    complete_test_analysis_run(&db, &first.id, &first.scope_hash, "stable-finding-key");
    let first_active = active_analysis_finding(&db, &first.id);
    db.set_analysis_finding_decision("stable-finding-key", "dismissed", None, None, 0)
        .expect("dismiss finding");

    let retry = db
        .retry_analysis_run(&first.id, &detector_set)
        .expect("retry terminal analysis")
        .run;
    assert_eq!(retry.request_attempt, 2);
    let retry_claimed = db
        .claim_analysis_run(&retry.id)
        .expect("claim retry")
        .expect("retry claimed");
    let detector = db
        .list_analysis_run_detectors(&retry.id)
        .expect("retry detectors")
        .pop()
        .expect("retry detector");
    let running = db
        .set_analysis_detector_status(
            &retry.id,
            &detector.detector_id,
            detector.revision,
            "running",
            1,
            0,
            0,
            0,
            None,
            None,
        )
        .expect("start retry detector");
    let draft = test_finding_draft("draft-id-retry", "stable-finding-key");
    db.stage_analysis_findings(&retry.id, &retry_claimed.scope_hash, &[draft])
        .expect("stage retry finding");
    db.set_analysis_detector_status(
        &retry.id,
        &detector.detector_id,
        running.revision,
        "completed",
        1,
        1,
        0,
        8,
        None,
        None,
    )
    .expect("finish retry detector");
    assert_eq!(active_analysis_finding(&db, &first.id).status, "active");
    db.publish_analysis_run(&retry.id)
        .expect("publish retry");
    let retry_active = active_analysis_finding(&db, &retry.id);
    assert_ne!(first_active.id, retry_active.id);
    assert_eq!(retry_active.decision.as_deref(), Some("dismissed"));
    assert_eq!(
        db.count_analysis_findings_for_run(&first.id, "active")
            .expect("old active count"),
        0
    );
}

#[test]
fn analysis_ai_assessment_refreshes_run_aggregate_revision_and_durable_evidence() {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open AI aggregate refresh database");
    let detector_set = vec!["cleanup_heuristics_v1".to_string()];
    let detector_pairs = vec![("cleanup_heuristics_v1".to_string(), 1_i64)];
    let run = db
        .start_analysis_run(
            &StartAnalysisRunRequest {
                scope: AnalysisScopeRequest {
                    kind: "approved_cleanup_paths".to_string(),
                    root_ids: Vec::new(),
                    paths: vec![root.to_string_lossy().into_owned()],
                },
                detector_ids: detector_set,
                request_key: Some("analysis-ai-aggregate-refresh".to_string()),
            },
            &detector_pairs,
        )
        .expect("start AI aggregate run")
        .run;
    complete_test_analysis_run(&db, &run.id, &run.scope_hash, "ai-aggregate-key");
    let before_run = db.get_analysis_run(&run.id).expect("read AI run before assessment");
    let before_finding = active_analysis_finding(&db, &run.id);
    let assessed = publish_test_analysis_ai_assessment(
        &db,
            &before_finding.id,
            "review",
            false,
            &json!({"source": "targeted-ai-test", "tier": "review"}),
        );
    let after_run = db.get_analysis_run(&run.id).expect("read AI run after assessment");
    assert_eq!(assessed.revision, before_finding.revision + 1);
    assert_eq!(after_run.revision, before_run.revision + 1);
    assert_eq!(after_run.findings_published, 1);
    assert_eq!(after_run.review_count, 1);
    assert_eq!(after_run.safe_count, 0);
    assert_eq!(after_run.exact_reclaimable_bytes, before_run.exact_reclaimable_bytes);
    assert_eq!(after_run.potential_reclaimable_bytes, before_run.potential_reclaimable_bytes);
    assert_eq!(
        db.list_analysis_finding_evidence(&assessed.id)
            .expect("list AI evidence")
            .iter()
            .filter(|evidence| evidence.evidence_kind == "ai_assessment")
            .count(),
        1
    );
}

#[test]
fn analysis_finding_decision_requires_zero_for_new_row_and_current_revision_for_updates() {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open finding decision CAS database");
    let detector_set = vec!["cleanup_heuristics_v1".to_string()];
    let detector_pairs = vec![("cleanup_heuristics_v1".to_string(), 1_i64)];
    let run = db
        .start_analysis_run(
            &StartAnalysisRunRequest {
                scope: AnalysisScopeRequest {
                    kind: "approved_cleanup_paths".to_string(),
                    root_ids: Vec::new(),
                    paths: vec![root.to_string_lossy().into_owned()],
                },
                detector_ids: detector_set,
                request_key: Some("analysis-decision-cas".to_string()),
            },
            &detector_pairs,
        )
        .expect("start finding decision CAS run")
        .run;
    complete_test_analysis_run(&db, &run.id, &run.scope_hash, "decision-cas-key");
    let finding = active_analysis_finding(&db, &run.id);
    assert!(db
        .set_analysis_finding_decision(&finding.finding_key, "acknowledged", None, None, 1)
        .is_err());
    let first = db
        .set_analysis_finding_decision(&finding.finding_key, "acknowledged", None, None, 0)
        .expect("create finding decision with explicit zero revision");
    let second = db
        .set_analysis_finding_decision(
            &finding.finding_key,
            "dismissed",
            None,
            Some("current revision update"),
            first.revision,
        )
        .expect("update finding decision with current revision");
    assert_eq!(second.revision, first.revision + 1);
    assert!(db
        .set_analysis_finding_decision(
            &finding.finding_key,
            "open",
            None,
            None,
            first.revision,
        )
        .is_err());
}

#[test]
fn managed_file_mutation_invalidates_managed_file_findings_in_the_same_transaction() {
    let root = test_dir();
    let db = Database::open(test_db_path()).expect("open managed finding invalidation database");
    let detector_set = vec!["cleanup_heuristics_v1".to_string()];
    let detector_pairs = vec![("cleanup_heuristics_v1".to_string(), 1_i64)];
    let run = db
        .start_analysis_run(
            &StartAnalysisRunRequest {
                scope: AnalysisScopeRequest {
                    kind: "approved_cleanup_paths".to_string(),
                    root_ids: Vec::new(),
                    paths: vec![root.to_string_lossy().into_owned()],
                },
                detector_ids: detector_set,
                request_key: Some("analysis-managed-file-invalidation".to_string()),
            },
            &detector_pairs,
        )
        .expect("start managed finding invalidation run")
        .run;
    db.claim_analysis_run(&run.id)
        .expect("claim managed finding invalidation run")
        .expect("managed finding invalidation run claimed");
    let detector = db
        .list_analysis_run_detectors(&run.id)
        .expect("list managed finding invalidation detector")
        .into_iter()
        .next()
        .expect("managed finding invalidation detector");
    let running = db
        .set_analysis_detector_status(
            &run.id,
            &detector.detector_id,
            detector.revision,
            "running",
            1,
            0,
            0,
            0,
            None,
            None,
        )
        .expect("start managed finding invalidation detector");
    let mut draft = test_finding_draft("managed-invalidation", "managed-invalidation-key");
    draft.primary_subject_kind = "managed_file".to_string();
    draft.primary_subject_id = "managed-file-invalidation-id".to_string();
    db.stage_analysis_findings(&run.id, &run.scope_hash, &[draft])
        .expect("stage managed finding invalidation finding");
    db.set_analysis_detector_status(
        &run.id,
        &running.detector_id,
        running.revision,
        "completed",
        1,
        1,
        8,
        8,
        None,
        None,
    )
    .expect("complete managed finding invalidation detector");
    db.publish_analysis_run(&run.id)
        .expect("publish managed finding invalidation run");
    assert_eq!(
        active_analysis_finding(&db, &run.id).status,
        "active",
        "fixture must publish before mutation invalidation"
    );

    let mut conn = db.conn().expect("open managed finding invalidation transaction");
    let tx = conn
        .transaction()
        .expect("start managed finding invalidation transaction");
    assert_eq!(
        crate::db::invalidate_analysis_findings_for_file_tx(
            &tx,
            "managed-file-invalidation-id",
        )
        .expect("invalidate managed finding"),
        1
    );
    tx.commit().expect("commit managed finding invalidation");
    drop(conn);
    assert_eq!(
        db.get_analysis_finding("managed-invalidation")
            .expect("read invalidated managed finding")
            .expect("invalidated managed finding exists")
            .status,
        "stale",
        "file mutation must make the durable managed finding non-executable"
    );
}

#[test]
fn failed_detector_publication_preserves_the_previous_active_findings() {
    let root = test_dir();
    fs::write(root.join("candidate.txt"), b"candidate").expect("write candidate");
    let db = Database::open(test_db_path()).expect("open analysis database");
    let detector_set = vec![("cleanup_heuristics_v1".to_string(), 1_i64)];
    let request = StartAnalysisRunRequest {
        scope: AnalysisScopeRequest {
            kind: "approved_cleanup_paths".to_string(),
            root_ids: Vec::new(),
            paths: vec![root.to_string_lossy().into_owned()],
        },
        detector_ids: vec!["cleanup_heuristics_v1".to_string()],
        request_key: Some("analysis-failed-preserve".to_string()),
    };
    let first = db
        .start_analysis_run(&request, &detector_set)
        .expect("start baseline analysis")
        .run;
    complete_test_analysis_run(&db, &first.id, &first.scope_hash, "preserved-finding-key");
    let previous = active_analysis_finding(&db, &first.id);

    let failed = db
        .retry_analysis_run(&first.id, &detector_set)
        .expect("start failed retry")
        .run;
    db.claim_analysis_run(&failed.id)
        .expect("claim failed retry")
        .expect("failed retry claimed");
    let detector = db
        .list_analysis_run_detectors(&failed.id)
        .expect("failed detector row")
        .pop()
        .expect("failed detector");
    db.set_analysis_detector_status(
        &failed.id,
        &detector.detector_id,
        detector.revision,
        "failed",
        0,
        0,
        0,
        0,
        Some("detector_io_error"),
        Some("The fixed detector could not read its source snapshot."),
    )
    .expect("record detector failure");

    assert_eq!(
        db.publish_analysis_run(&failed.id)
            .expect("publish failed retry"),
        AnalysisPublishOutcome::CompletedWithWarnings
    );
    let final_failed = db.get_analysis_run(&failed.id).expect("failed final run");
    assert_eq!(final_failed.status, "completed_with_warnings");
    assert_eq!(
        active_analysis_finding(&db, &first.id).id,
        previous.id,
        "a failed detector must not supersede the prior active finding"
    );
    assert_eq!(
        db.count_analysis_findings_for_run(&failed.id, "active")
            .expect("failed retry active count"),
        0
    );
    assert_eq!(
        db.count_analysis_findings_for_run(&failed.id, "staged")
            .expect("failed retry staged count"),
        0
    );
}

#[test]
fn one_failed_detector_still_publishes_successful_detector_findings_with_warning() {
    let root = test_dir();
    fs::write(root.join("candidate.txt"), b"candidate").expect("write candidate");
    let db = Database::open(test_db_path()).expect("open partial detector database");
    let detector_set = vec![
        ("cleanup_heuristics_v1".to_string(), 1_i64),
        ("large_file_v1".to_string(), 1_i64),
    ];
    let run = db
        .start_analysis_run(
            &StartAnalysisRunRequest {
                scope: AnalysisScopeRequest {
                    kind: "approved_cleanup_paths".to_string(),
                    root_ids: Vec::new(),
                    paths: vec![root.to_string_lossy().into_owned()],
                },
                detector_ids: detector_set.iter().map(|(id, _)| id.clone()).collect(),
                request_key: Some("analysis-partial-detector".to_string()),
            },
            &detector_set,
        )
        .expect("start partial detector run")
        .run;
    db.claim_analysis_run(&run.id)
        .expect("claim partial detector run")
        .expect("partial detector run claimed");
    let detectors = db
        .list_analysis_run_detectors(&run.id)
        .expect("list partial detectors");
    let successful = detectors
        .iter()
        .find(|detector| detector.detector_id == "cleanup_heuristics_v1")
        .expect("successful detector row")
        .clone();
    let failed = detectors
        .iter()
        .find(|detector| detector.detector_id == "large_file_v1")
        .expect("failed detector row")
        .clone();
    let running = db
        .set_analysis_detector_status(
            &run.id,
            &successful.detector_id,
            successful.revision,
            "running",
            1,
            0,
            0,
            0,
            None,
            None,
        )
        .expect("start successful detector");
    db.stage_analysis_findings(
        &run.id,
        &run.scope_hash,
        &[test_finding_draft("partial-success", "partial-success-key")],
    )
    .expect("stage successful detector finding");
    db.set_analysis_detector_status(
        &run.id,
        &successful.detector_id,
        running.revision,
        "completed",
        1,
        1,
        0,
        8,
        None,
        None,
    )
    .expect("finish successful detector");
    db.set_analysis_detector_status(
        &run.id,
        &failed.detector_id,
        failed.revision,
        "failed",
        0,
        0,
        0,
        0,
        Some("test_detector_failure"),
        Some("The large-file detector fixture failed."),
    )
    .expect("record failed detector");
    assert_eq!(
        db.publish_analysis_run(&run.id)
            .expect("publish partial detector run"),
        AnalysisPublishOutcome::CompletedWithWarnings
    );
    let final_run = db.get_analysis_run(&run.id).expect("partial final run");
    assert_eq!(final_run.status, "completed_with_warnings");
    assert_eq!(final_run.detectors_completed, 1);
    assert_eq!(final_run.detectors_failed, 1);
    assert_eq!(
        db.count_analysis_findings_for_run(&run.id, "active")
            .expect("successful active finding"),
        1
    );
}

#[test]
fn expired_snooze_is_projected_as_open_without_erasing_the_decision_fact() {
    let root = test_dir();
    fs::write(root.join("candidate.txt"), b"candidate").expect("write candidate");
    let db = Database::open(test_db_path()).expect("open analysis database");
    let detector_set = vec![("cleanup_heuristics_v1".to_string(), 1_i64)];
    let run = db
        .start_analysis_run(
            &StartAnalysisRunRequest {
                scope: AnalysisScopeRequest {
                    kind: "approved_cleanup_paths".to_string(),
                    root_ids: Vec::new(),
                    paths: vec![root.to_string_lossy().into_owned()],
                },
                detector_ids: vec!["cleanup_heuristics_v1".to_string()],
                request_key: Some("analysis-expired-snooze".to_string()),
            },
            &detector_set,
        )
        .expect("start snooze analysis")
        .run;
    complete_test_analysis_run(&db, &run.id, &run.scope_hash, "snooze-finding-key");
    let finding = active_analysis_finding(&db, &run.id);
    let decision = db
        .set_analysis_finding_decision(
            &finding.finding_key,
            "snoozed",
            Some(current_unix_seconds().saturating_sub(1)),
            Some("expired fixture"),
            0,
        )
        .expect("store expired snooze");
    assert_eq!(decision.decision, "snoozed");

    let hydrated = db
        .get_analysis_finding(&finding.id)
        .expect("get snoozed finding")
        .expect("snoozed finding exists");
    assert_eq!(hydrated.decision.as_deref(), Some("open"));
    assert!(hydrated.snoozed_until.is_some());
    let open_page = db
        .list_analysis_findings(
            &AnalysisFindingFilter {
                decision: Some("open".to_string()),
                ..AnalysisFindingFilter::default()
            },
            None,
            20,
        )
        .expect("filter expired snooze as open");
    assert_eq!(open_page.findings.len(), 1);
    let snoozed_page = db
        .list_analysis_findings(
            &AnalysisFindingFilter {
                decision: Some("snoozed".to_string()),
                ..AnalysisFindingFilter::default()
            },
            None,
            20,
        )
        .expect("filter expired snooze");
    assert!(snoozed_page.findings.is_empty());
}

#[test]
fn cancelled_and_source_changed_analysis_runs_never_publish_staged_findings() {
    let root = test_dir();
    fs::write(root.join("candidate.txt"), b"candidate").expect("write candidate");
    let db = Database::open(test_db_path()).expect("open analysis database");
    let detector_set = vec![("cleanup_heuristics_v1".to_string(), 1_i64)];
    let make_request = |key: &str| StartAnalysisRunRequest {
        scope: AnalysisScopeRequest {
            kind: "approved_cleanup_paths".to_string(),
            root_ids: Vec::new(),
            paths: vec![root.to_string_lossy().into_owned()],
        },
        detector_ids: vec!["cleanup_heuristics_v1".to_string()],
        request_key: Some(key.to_string()),
    };
    let cancelled = db
        .start_analysis_run(&make_request("analysis-cancel"), &detector_set)
        .expect("start cancellation run")
        .run;
    db.claim_analysis_run(&cancelled.id)
        .expect("claim cancellation run")
        .expect("cancellation run claimed");
    db.stage_analysis_findings(
        &cancelled.id,
        &cancelled.scope_hash,
        &[test_finding_draft("cancelled-draft", "cancelled-key")],
    )
    .expect("stage before cancellation request");
    db.request_analysis_cancellation(&cancelled.id)
        .expect("request cancellation");
    assert_eq!(
        db.publish_analysis_run(&cancelled.id)
            .expect("cancel publication"),
        AnalysisPublishOutcome::Cancelled
    );
    assert_eq!(
        db.count_analysis_findings_for_run(&cancelled.id, "active")
            .expect("cancelled active count"),
        0
    );
    assert_eq!(
        db.count_analysis_findings_for_run(&cancelled.id, "staged")
            .expect("cancelled staged diagnostic count"),
        1
    );

    let changed = db
        .start_analysis_run(&make_request("analysis-source-change"), &detector_set)
        .expect("start source change run")
        .run;
    db.claim_analysis_run(&changed.id)
        .expect("claim source change run")
        .expect("source change run claimed");
    fs::create_dir(root.join("new-entry")).expect("change approved root");
    let detector = db
        .list_analysis_run_detectors(&changed.id)
        .expect("source change detector")
        .pop()
        .expect("source change detector row");
    let running = db
        .set_analysis_detector_status(
            &changed.id,
            &detector.detector_id,
            detector.revision,
            "running",
            1,
            0,
            0,
            0,
            None,
            None,
        )
        .expect("start source detector");
    db.stage_analysis_findings(
        &changed.id,
        &changed.scope_hash,
        &[test_finding_draft("changed-draft", "changed-key")],
    )
    .expect("stage source changed finding");
    db.set_analysis_detector_status(
        &changed.id,
        &detector.detector_id,
        running.revision,
        "completed",
        1,
        1,
        0,
        8,
        None,
        None,
    )
    .expect("finish source detector");
    assert_eq!(
        db.publish_analysis_run(&changed.id)
            .expect("source change publication"),
        AnalysisPublishOutcome::CompletedWithWarnings
    );
    let changed_run = db.get_analysis_run(&changed.id).expect("changed run");
    assert_eq!(changed_run.status, "completed_with_warnings");
    assert!(changed_run.rerun_required);
    assert_eq!(
        db.count_analysis_findings_for_run(&changed.id, "active")
            .expect("changed active count"),
        0
    );
    assert_eq!(
        db.count_analysis_findings_for_run(&changed.id, "staged")
            .expect("changed staged diagnostic count"),
        1
    );
}

#[test]
fn managed_scope_root_set_change_prevents_analysis_publication() {
    let db = Database::open(test_db_path()).expect("open managed analysis database");
    let conn = db.conn().expect("database connection");
    conn.execute(
        "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, created_at, updated_at) VALUES ('analysis-managed-root-a', '/tmp/analysis-managed-root-a', 'root-a', 'file_library', 1, 'healthy', 1, 1)",
        [],
    )
    .expect("insert initial managed root");
    drop(conn);

    let detector_set = vec![("large_file_v1".to_string(), 1_i64)];
    let run = db
        .start_analysis_run(
            &StartAnalysisRunRequest {
                scope: AnalysisScopeRequest {
                    kind: "all_managed_file_library".to_string(),
                    root_ids: Vec::new(),
                    paths: Vec::new(),
                },
                detector_ids: vec!["large_file_v1".to_string()],
                request_key: Some("analysis-managed-root-set-change".to_string()),
            },
            &detector_set,
        )
        .expect("start managed analysis")
        .run;
    db.claim_analysis_run(&run.id)
        .expect("claim managed analysis")
        .expect("managed analysis claimed");
    let detector = db
        .list_analysis_run_detectors(&run.id)
        .expect("managed detector row")
        .pop()
        .expect("managed detector");
    let running = db
        .set_analysis_detector_status(
            &run.id,
            &detector.detector_id,
            detector.revision,
            "running",
            0,
            0,
            0,
            0,
            None,
            None,
        )
        .expect("start managed detector");
    let conn = db.conn().expect("database connection after claim");
    conn.execute(
        "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, created_at, updated_at) VALUES ('analysis-managed-root-b', '/tmp/analysis-managed-root-b', 'root-b', 'file_library', 1, 'healthy', 1, 1)",
        [],
    )
    .expect("insert newly enabled managed root");
    drop(conn);
    db.set_analysis_detector_status(
        &run.id,
        &detector.detector_id,
        running.revision,
        "completed",
        0,
        0,
        0,
        0,
        None,
        None,
    )
    .expect("finish managed detector");
    assert_eq!(
        db.publish_analysis_run(&run.id)
            .expect("publish changed managed scope"),
        AnalysisPublishOutcome::CompletedWithWarnings
    );
    let final_run = db.get_analysis_run(&run.id).expect("final managed run");
    assert_eq!(final_run.error_code.as_deref(), Some("source_changed_during_run"));
    assert!(final_run.rerun_required);
}

#[test]
fn startup_recovery_interrupts_runs_and_retains_staged_diagnostics_without_publishing() {
    let root = test_dir();
    fs::write(root.join("candidate.txt"), b"candidate").expect("write candidate");
    let db = Database::open(test_db_path()).expect("open recovery analysis database");
    let detector_set = vec![("cleanup_heuristics_v1".to_string(), 1_i64)];
    let run = db
        .start_analysis_run(
            &StartAnalysisRunRequest {
                scope: AnalysisScopeRequest {
                    kind: "approved_cleanup_paths".to_string(),
                    root_ids: Vec::new(),
                    paths: vec![root.to_string_lossy().into_owned()],
                },
                detector_ids: vec!["cleanup_heuristics_v1".to_string()],
                request_key: Some("analysis-startup-recovery".to_string()),
            },
            &detector_set,
        )
        .expect("start recovery run")
        .run;
    db.claim_analysis_run(&run.id)
        .expect("claim recovery run")
        .expect("recovery run claimed");
    db.stage_analysis_findings(
        &run.id,
        &run.scope_hash,
        &[test_finding_draft("recovery-draft", "recovery-key")],
    )
    .expect("stage recovery diagnostic");
    assert_eq!(db.recover_analysis_runs().expect("recover analysis runs"), 1);
    let interrupted = db.get_analysis_run(&run.id).expect("interrupted run");
    assert_eq!(interrupted.status, "interrupted");
    assert_eq!(
        db.count_analysis_findings_for_run(&run.id, "staged")
            .expect("retained interrupted diagnostic"),
        1
    );
    assert_eq!(
        db.publish_analysis_run(&run.id)
            .expect("terminal interrupted publication"),
        AnalysisPublishOutcome::CompletedWithWarnings
    );
    assert_eq!(
        db.count_analysis_findings_for_run(&run.id, "active")
            .expect("interrupted active count"),
        0
    );
}

#[test]
fn analysis_totals_do_not_double_count_overlapping_subjects() {
    let root = test_dir();
    fs::create_dir(root.join("child")).expect("create nested analysis directory");
    let db = Database::open(test_db_path()).expect("open totals analysis database");
    let detector_set = vec![("cleanup_heuristics_v1".to_string(), 1_i64)];
    let run = db
        .start_analysis_run(
            &StartAnalysisRunRequest {
                scope: AnalysisScopeRequest {
                    kind: "approved_cleanup_paths".to_string(),
                    root_ids: Vec::new(),
                    paths: vec![root.to_string_lossy().into_owned()],
                },
                detector_ids: vec!["cleanup_heuristics_v1".to_string()],
                request_key: Some("analysis-overlap-totals".to_string()),
            },
            &detector_set,
        )
        .expect("start totals run")
        .run;
    db.claim_analysis_run(&run.id)
        .expect("claim totals run")
        .expect("totals run claimed");
    let detector = db
        .list_analysis_run_detectors(&run.id)
        .expect("totals detector")
        .pop()
        .expect("totals detector row");
    let running = db
        .set_analysis_detector_status(
            &run.id,
            &detector.detector_id,
            detector.revision,
            "running",
            2,
            0,
            0,
            0,
            None,
            None,
        )
        .expect("start totals detector");
    let root_path = root.to_string_lossy().into_owned();
    let child_path = root.join("child").to_string_lossy().into_owned();
    let mut parent = test_finding_draft("overlap-parent", "overlap-parent-key");
    parent.primary_subject_id = root_path.clone();
    parent.path_snapshot = Some(root_path.clone());
    parent.identity_snapshot = json!({"size": 100, "identity": "parent"});
    parent.size_bytes = 100;
    parent.exact_reclaimable_bytes = Some(100);
    parent.potential_reclaimable_bytes = 100;
    let mut child = test_finding_draft("overlap-child", "overlap-child-key");
    child.primary_subject_id = child_path.clone();
    child.path_snapshot = Some(child_path);
    child.identity_snapshot = json!({"size": 50, "identity": "child"});
    child.size_bytes = 50;
    child.exact_reclaimable_bytes = Some(50);
    child.potential_reclaimable_bytes = 50;
    db.stage_analysis_findings(&run.id, &run.scope_hash, &[parent, child])
        .expect("stage overlapping findings");
    db.set_analysis_detector_status(
        &run.id,
        &detector.detector_id,
        running.revision,
        "completed",
        2,
        2,
        150,
        150,
        None,
        None,
    )
    .expect("finish totals detector");
    db.publish_analysis_run(&run.id)
        .expect("publish totals run");
    let final_run = db.get_analysis_run(&run.id).expect("final totals run");
    assert_eq!(final_run.exact_reclaimable_bytes, 100);
    assert_eq!(final_run.potential_reclaimable_bytes, 100);
}

#[test]
fn analysis_totals_retain_duplicate_exact_bytes_when_large_file_shares_path() {
    let root = test_dir();
    let keeper_path = root.join("keeper.bin").to_string_lossy().into_owned();
    let keeper_alias_path = root.join("keeper-alias.bin").to_string_lossy().into_owned();
    let shared_path = root.join("shared.bin").to_string_lossy().into_owned();
    let other_copy_path = root.join("other-copy.bin").to_string_lossy().into_owned();
    let unrelated_path = root.join("unrelated.bin").to_string_lossy().into_owned();
    let db_path = test_db_path();
    let db = Database::open(&db_path).expect("open duplicate exact aggregation database");
    insert_test_file_at_path(
        &db,
        "shared-duplicate-keeper",
        &keeper_path,
        "keeper.bin",
        "bin",
        100,
        1,
    );
    insert_test_file_at_path(
        &db,
        "shared-duplicate-keeper-alias",
        &keeper_alias_path,
        "keeper-alias.bin",
        "bin",
        100,
        1,
    );
    insert_test_file_at_path(
        &db,
        "shared-large-file-id",
        &shared_path,
        "shared.bin",
        "bin",
        100,
        1,
    );
    insert_test_file_at_path(
        &db,
        "shared-duplicate-other-copy",
        &other_copy_path,
        "other-copy.bin",
        "bin",
        100,
        1,
    );
    insert_test_file_at_path(
        &db,
        "shared-safe-unrelated",
        &unrelated_path,
        "unrelated.bin",
        "bin",
        50,
        1,
    );
    let conn = db.conn().expect("open duplicate exact root connection");
    conn.execute(
        "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, created_at, updated_at) VALUES ('analysis-exact-root', ?1, 'analysis-exact-root', 'file_library', 1, 'healthy', 1, 1)",
        params![root.to_string_lossy().into_owned()],
    )
    .expect("insert duplicate exact managed root");
    conn.execute(
        "INSERT INTO dedupe_runs (id, request_key, request_attempt, scope_json, scope_hash, scope_snapshot_json, scope_snapshot_hash, status, phase, revision, finished_at, created_at, updated_at) VALUES ('shared-duplicate-run', 'shared-duplicate-request', 1, '{\"kind\":\"allEnabledScanRoots\"}', 'shared-scope', '[]', 'shared-snapshot', 'completed', 'completed', 1, 1, 1, 1)",
        [],
    )
    .expect("insert authoritative duplicate run");
    conn.execute(
        "INSERT INTO duplicate_groups (id, size_each, full_hash, full_hash_algorithm, full_hash_version, member_count, physical_copy_count, hardlink_alias_count, exact_reclaimable_bytes, potential_reclaimable_bytes, reclaimable_confidence, status, last_built_run_id, revision, created_at, updated_at, last_verified_at) VALUES ('shared-duplicate-group', 100, 'shared-duplicate-hash', 'blake3', 1, 4, 3, 1, 200, 200, 'exact', 'active', 'shared-duplicate-run', 1, 1, 1, 1)",
        [],
    )
    .expect("insert authoritative duplicate group");
    for (file_id, path, physical_key, is_alias) in [
        (
            "shared-duplicate-keeper",
            keeper_path.as_str(),
            "physical-keeper",
            0,
        ),
        (
            "shared-duplicate-keeper-alias",
            keeper_alias_path.as_str(),
            "physical-keeper",
            1,
        ),
        (
            "shared-large-file-id",
            shared_path.as_str(),
            "physical-reclaimable",
            0,
        ),
        (
            "shared-duplicate-other-copy",
            other_copy_path.as_str(),
            "physical-other-copy",
            0,
        ),
    ] {
        conn.execute(
            "INSERT INTO duplicate_group_members (group_id, file_id, path_snapshot, physical_key, identity_status, is_hardlink_alias, size, modified_ns, verified_at) VALUES ('shared-duplicate-group', ?1, ?2, ?3, 'verified', ?4, 100, 1, 1)",
            params![file_id, path, physical_key, is_alias],
        )
        .expect("insert authoritative duplicate member");
    }
    drop(conn);

    let detector_set = vec![
        ("duplicate_reclaimable_v1".to_string(), 1_i64),
        ("large_file_v1".to_string(), 1_i64),
        ("cleanup_heuristics_v1".to_string(), 1_i64),
    ];
    let run = db
        .start_analysis_run(
            &StartAnalysisRunRequest {
                scope: AnalysisScopeRequest {
                    kind: "all_managed_file_library".to_string(),
                    root_ids: Vec::new(),
                    paths: Vec::new(),
                },
                detector_ids: detector_set.iter().map(|(id, _)| id.clone()).collect(),
                request_key: Some("analysis-exact-shared-path".to_string()),
            },
            &detector_set,
        )
        .expect("start duplicate exact aggregation run")
        .run;
    db.claim_analysis_run(&run.id)
        .expect("claim duplicate exact aggregation run")
        .expect("duplicate exact aggregation run claimed");
    let detectors = db
        .list_analysis_run_detectors(&run.id)
        .expect("list duplicate exact detectors");
    let mut running = Vec::new();
    for detector in detectors {
        running.push(
            db.set_analysis_detector_status(
                &run.id,
                &detector.detector_id,
                detector.revision,
                "running",
                1,
                0,
                0,
                0,
                None,
                None,
            )
            .expect("start duplicate exact detector"),
        );
    }

    let mut duplicate = test_finding_draft("shared-duplicate", "shared-duplicate-key");
    duplicate.detector_id = "duplicate_reclaimable_v1".to_string();
    duplicate.tier = "review".to_string();
    duplicate.action_kind = "review_duplicate_group".to_string();
    duplicate.executable = false;
    duplicate.primary_subject_kind = "duplicate_group".to_string();
    duplicate.primary_subject_id = "shared-duplicate-group".to_string();
    duplicate.path_snapshot = Some(shared_path.clone());
    duplicate.identity_snapshot = json!({
        "groupId": "shared-duplicate-group",
        "fullHash": "shared-duplicate-hash",
        "revision": 1
    });
    duplicate.exact_reclaimable_bytes = Some(200);
    duplicate.potential_reclaimable_bytes = 200;

    let mut large_file = test_finding_draft("shared-large-file", "shared-large-file-key");
    large_file.detector_id = "large_file_v1".to_string();
    large_file.tier = "review".to_string();
    large_file.action_kind = "reveal".to_string();
    large_file.executable = false;
    large_file.primary_subject_kind = "managed_file".to_string();
    large_file.primary_subject_id = "shared-large-file-id".to_string();
    large_file.path_snapshot = Some(shared_path.clone());
    large_file.identity_snapshot = json!({
        "physical": {"physicalKey": "shared-large-file-physical"},
        "size": 100
    });
    large_file.exact_reclaimable_bytes = None;
    large_file.potential_reclaimable_bytes = 100;

    let mut same_physical_safe =
        test_finding_draft("shared-safe-exact", "shared-safe-exact-key");
    same_physical_safe.detector_id = "cleanup_heuristics_v1".to_string();
    same_physical_safe.primary_subject_kind = "approved_path".to_string();
    same_physical_safe.primary_subject_id = shared_path.clone();
    same_physical_safe.path_snapshot = Some(shared_path.clone());
    same_physical_safe.identity_snapshot = json!({
        "physical": {"physicalKey": "physical-reclaimable"},
        "size": 100
    });
    same_physical_safe.exact_reclaimable_bytes = Some(100);
    same_physical_safe.potential_reclaimable_bytes = 100;

    let mut unrelated_safe =
        test_finding_draft("unrelated-safe-exact", "unrelated-safe-exact-key");
    unrelated_safe.detector_id = "cleanup_heuristics_v1".to_string();
    unrelated_safe.primary_subject_kind = "approved_path".to_string();
    unrelated_safe.primary_subject_id = unrelated_path.clone();
    unrelated_safe.path_snapshot = Some(unrelated_path);
    unrelated_safe.identity_snapshot = json!({
        "physical": {"physicalKey": "physical-unrelated"},
        "size": 50
    });
    unrelated_safe.size_bytes = 50;
    unrelated_safe.exact_reclaimable_bytes = Some(50);
    unrelated_safe.potential_reclaimable_bytes = 50;

    db.stage_analysis_findings(
        &run.id,
        &run.scope_hash,
        &[
            unrelated_safe,
            large_file,
            same_physical_safe,
            duplicate,
        ],
    )
        .expect("stage duplicate and large-file findings at shared path");
    for detector in running {
        db.set_analysis_detector_status(
            &run.id,
            &detector.detector_id,
            detector.revision,
            "completed",
            1,
            1,
            match detector.detector_id.as_str() {
                "duplicate_reclaimable_v1" => 200,
                "cleanup_heuristics_v1" => 150,
                _ => 0,
            },
            match detector.detector_id.as_str() {
                "duplicate_reclaimable_v1" => 200,
                "cleanup_heuristics_v1" => 150,
                _ => 100,
            },
            None,
            None,
        )
        .expect("complete duplicate exact detector");
    }
    db.publish_analysis_run(&run.id)
        .expect("publish duplicate exact aggregation run");
    let final_run = db
        .get_analysis_run(&run.id)
        .expect("read duplicate exact aggregation run");
    assert_eq!(final_run.exact_reclaimable_bytes, 250);
    assert_eq!(final_run.potential_reclaimable_bytes, 250);

    let duplicate_finding_id: String = db
        .conn()
        .expect("open AI aggregate connection")
        .query_row(
            "SELECT id FROM analysis_findings WHERE run_id = ?1 AND finding_key = 'shared-duplicate-key' AND status = 'active'",
            params![run.id],
            |row| row.get(0),
        )
        .expect("read duplicate finding for AI refresh");
    publish_test_analysis_ai_assessment(
        &db,
        &duplicate_finding_id,
        "review",
        false,
        &json!({"source": "physical-union-refresh"}),
    );
    let refreshed = db
        .get_analysis_run(&run.id)
        .expect("read refreshed physical union");
    assert_eq!(refreshed.exact_reclaimable_bytes, 250);
    assert_eq!(refreshed.potential_reclaimable_bytes, 250);

    drop(db);
    let reopened = Database::open(db_path).expect("reopen physical union database");
    let hydrated = reopened
        .get_analysis_run(&run.id)
        .expect("hydrate physical union after reopen");
    assert_eq!(hydrated.exact_reclaimable_bytes, 250);
    assert_eq!(hydrated.potential_reclaimable_bytes, 250);

    reopened
        .conn()
        .expect("open stale group connection")
        .execute(
            "UPDATE duplicate_groups SET status = 'stale', revision = revision + 1 WHERE id = 'shared-duplicate-group'",
            [],
        )
        .expect("stale duplicate authority");
    publish_test_analysis_ai_assessment(
        &reopened,
        &duplicate_finding_id,
        "review",
        false,
        &json!({"source": "stale-group-refresh"}),
    );
    let stale_group_refresh = reopened
        .get_analysis_run(&run.id)
        .expect("read aggregate with stale duplicate group");
    assert_eq!(stale_group_refresh.exact_reclaimable_bytes, 150);
}

#[test]
fn dedupe_authority_is_global_and_diagnostic_root_runs_cannot_replace_it() {
    let db = Database::open(test_db_path()).expect("open dedupe authority database");
    let conn = db.conn().expect("database connection");
    for (id, path) in [("authority-root-a", "/tmp/authority-root-a"), ("authority-root-b", "/tmp/authority-root-b")] {
        conn.execute(
            "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, current_generation, needs_reconciliation, created_at, updated_at) VALUES (?1, ?2, ?1, 'file_library', 1, 'healthy', 0, 0, 1, 1)",
            params![id, path],
        )
        .expect("insert authority root");
    }
    drop(conn);

    let diagnostic = db
        .start_dedupe_run(&StartDedupeRunRequest {
            scope: DedupeScopeRequest {
                kind: "explicitEnabledScanRoots".to_string(),
                root_ids: vec!["authority-root-a".to_string()],
            },
            request_key: Some("diagnostic-authority-test".to_string()),
            parent_scan_session_id: None,
        })
        .expect("start diagnostic root run")
        .run;
    assert_eq!(diagnostic.publication_mode, "diagnostic");
    db.claim_dedupe_run(&diagnostic.id)
        .expect("claim diagnostic run")
        .expect("diagnostic claimed");
    db.publish_dedupe_groups(&diagnostic.id, &[], &DedupeCheckpoint::default())
        .expect("diagnostic publication");
    assert_eq!(db.get_dedupe_authority().expect("authority").status, "rebuild_required");

    let authoritative = db
        .start_dedupe_run(&StartDedupeRunRequest {
            scope: DedupeScopeRequest {
                kind: "allManagedFileLibrary".to_string(),
                root_ids: Vec::new(),
            },
            request_key: Some("global-authority-test".to_string()),
            parent_scan_session_id: None,
        })
        .expect("start global run")
        .run;
    assert_eq!(authoritative.publication_mode, "authoritative");
    db.claim_dedupe_run(&authoritative.id)
        .expect("claim global run")
        .expect("global claimed");
    db.publish_dedupe_groups(&authoritative.id, &[], &DedupeCheckpoint::default())
        .expect("publish global run");
    assert_eq!(db.get_dedupe_authority().expect("healthy authority").status, "healthy");

    let before = db
        .get_dedupe_authority()
        .expect("authority revision before unhealthy root");
    let conn = db.conn().expect("database connection");
    conn.execute(
        "UPDATE scan_roots SET health_status = 'degraded', needs_reconciliation = 1 WHERE id = 'authority-root-a'",
        [],
    )
    .expect("mark root unhealthy");
    drop(conn);
    let blocked = db
        .start_dedupe_run(&StartDedupeRunRequest {
            scope: DedupeScopeRequest {
                kind: "allManagedFileLibrary".to_string(),
                root_ids: Vec::new(),
            },
            request_key: Some("global-authority-blocked".to_string()),
            parent_scan_session_id: None,
        })
        .expect("start blocked global run")
        .run;
    db.claim_dedupe_run(&blocked.id)
        .expect("claim blocked run")
        .expect("blocked run claimed");
    db.publish_dedupe_groups(&blocked.id, &[], &DedupeCheckpoint::default())
        .expect("blocked global publication");
    let after = db.get_dedupe_authority().expect("blocked authority");
    assert_eq!(after.status, "rebuild_required");
    assert!(after.revision > before.revision);
}

fn complete_test_analysis_run(db: &Database, run_id: &str, scope_hash: &str, finding_key: &str) {
    if db.get_analysis_run(run_id).expect("read test analysis").status == "queued" {
        db.claim_analysis_run(run_id)
            .expect("claim test analysis")
            .expect("test analysis claimed");
    }
    let detector = db
        .list_analysis_run_detectors(run_id)
        .expect("test detector")
        .pop()
        .expect("test detector row");
    let running = db
        .set_analysis_detector_status(
            run_id,
            &detector.detector_id,
            detector.revision,
            "running",
            1,
            0,
            0,
            0,
            None,
            None,
        )
        .expect("start test detector");
    db.stage_analysis_findings(
        run_id,
        scope_hash,
        &[test_finding_draft("draft-id", finding_key)],
    )
    .expect("stage test finding");
    db.set_analysis_detector_status(
        run_id,
        &detector.detector_id,
        running.revision,
        "completed",
        1,
        1,
        8,
        8,
        None,
        None,
    )
    .expect("finish test detector");
    assert_eq!(
        db.publish_analysis_run(run_id).expect("publish test run"),
        AnalysisPublishOutcome::Completed
    );
}

fn active_analysis_finding(db: &Database, run_id: &str) -> AnalysisFindingDto {
    let filter = AnalysisFindingFilter {
        run_id: Some(run_id.to_string()),
        status: Some("active".to_string()),
        ..AnalysisFindingFilter::default()
    };
    db.list_analysis_findings(&filter, None, 20)
        .expect("active finding page")
        .findings
        .into_iter()
        .next()
        .expect("active analysis finding")
}

fn publish_test_analysis_ai_assessment(
    db: &Database,
    finding_id: &str,
    requested_tier: &str,
    requested_trash_allowed: bool,
    test_evidence: &serde_json::Value,
) -> AnalysisFindingDto {
    let finding = db
        .get_analysis_finding(finding_id)
        .expect("read test AI finding")
        .expect("test AI finding exists");
    let run = db
        .get_analysis_run(&finding.run_id)
        .expect("read test AI run");
    let detector = db
        .list_analysis_run_detectors(&run.id)
        .expect("read test AI detector")
        .into_iter()
        .find(|detector| detector.detector_id == finding.detector_id)
        .expect("test AI detector exists");
    let candidate_ids = vec![finding.id.clone()];
    let candidate_set_json = serde_json::to_string(&candidate_ids).expect("serialize candidate set");
    let candidate_set_fingerprint = blake3::hash(candidate_set_json.as_bytes()).to_hex().to_string();
    let source_fingerprint = blake3::hash(
        &serde_json::to_vec(&finding.identity_snapshot).expect("serialize source identity"),
    )
    .to_hex()
    .to_string();
    let candidate_identity = json!({
        "findingId": finding.id,
        "findingKey": finding.finding_key,
        "analysisRunId": finding.run_id,
        "expectedRevision": finding.revision,
        "detectorId": finding.detector_id,
        "detectorVersion": finding.detector_version,
        "primarySubjectKind": finding.primary_subject_kind,
        "primarySubjectId": finding.primary_subject_id,
        "sourceFingerprint": source_fingerprint
    });
    let candidate_identity_fingerprint = blake3::hash(
        &serde_json::to_vec(&candidate_identity).expect("serialize candidate identity"),
    )
    .to_hex()
    .to_string();
    let settings_value = db
        .conn()
        .expect("read AI settings")
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![crate::ai::settings::AI_SETTINGS_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .expect("query AI settings");
    let settings_revision = app_setting_value_fingerprint(settings_value.as_deref());
    let evidence = json!({
        "schema": "cleanup_ai_assessment_v1",
        "coverage": {
            "status": "returned_exactly_once",
            "candidateId": finding.id,
            "returnedCandidateId": finding.id,
            "requestId": "test-request",
            "batchId": "test-batch",
            "requestedCandidateIds": candidate_ids,
            "candidateSetFingerprint": candidate_set_fingerprint,
            "requestCandidateSetFingerprint": candidate_set_fingerprint,
            "requestedCount": 1,
            "returnedCount": 1,
            "omittedCount": 0,
            "duplicateCount": 0,
            "unknownCount": 0,
            "analysisRunId": run.id,
            "analysisRunRevision": run.revision,
            "analysisSourceSnapshotHash": run.source_snapshot_hash,
            "detectorSetHash": run.detector_set_hash,
            "detectorId": detector.detector_id,
            "detectorVersion": detector.detector_version,
            "detectorRevision": detector.revision,
            "expectedFindingRevision": finding.revision,
            "candidateIdentityFingerprint": candidate_identity_fingerprint,
            "sourceFingerprint": source_fingerprint,
            "providerPolicyRevision": settings_revision
        },
        "assessment": {
            "candidateId": finding.id,
            "testEvidence": test_evidence
        }
    });
    let precondition = AnalysisAiFindingPrecondition {
        finding_id: finding.id.clone(),
        finding_key: finding.finding_key.clone(),
        run_id: finding.run_id.clone(),
        detector_id: finding.detector_id.clone(),
        detector_version: finding.detector_version,
        detector_status: detector.status,
        detector_revision: detector.revision,
        scope_hash: finding.scope_hash.clone(),
        expected_revision: finding.revision,
        primary_subject_kind: finding.primary_subject_kind.clone(),
        primary_subject_id: finding.primary_subject_id.clone(),
        path_snapshot: finding.path_snapshot.clone(),
        identity_snapshot_json: serde_json::to_string(&finding.identity_snapshot)
            .expect("serialize finding identity"),
    };
    let batch = AnalysisAiPublicationBatch {
        run_id: run.id,
        expected_run_revision: run.revision,
        expected_source_snapshot_hash: run.source_snapshot_hash,
        expected_detector_set_hash: run.detector_set_hash,
        settings_key: crate::ai::settings::AI_SETTINGS_KEY.to_string(),
        expected_settings_revision: settings_revision,
        expected_candidate_set_fingerprint: candidate_set_fingerprint,
        preconditions: vec![precondition],
        assessments: vec![AnalysisAiAssessmentPublication {
            finding_id: finding.id,
            requested_tier: requested_tier.to_string(),
            requested_trash_allowed,
            evidence,
        }],
    };
    db.publish_analysis_ai_assessments_cas(&batch)
        .expect("publish test AI assessment")
        .into_iter()
        .next()
        .expect("test AI assessment was published")
}

fn test_finding_draft(id: &str, finding_key: &str) -> FindingDraft {
    FindingDraft {
        id: id.to_string(),
        finding_key: finding_key.to_string(),
        detector_id: "cleanup_heuristics_v1".to_string(),
        detector_version: 1,
        tier: "safe".to_string(),
        category: "temp_cache".to_string(),
        action_kind: "safe_trash_candidate".to_string(),
        title: "Test cleanup candidate".to_string(),
        reason: "Test finding for durable publication.".to_string(),
        risk_note: None,
        confidence: "exact".to_string(),
        size_bytes: 8,
        exact_reclaimable_bytes: Some(8),
        potential_reclaimable_bytes: 8,
        requires_confirmation: true,
        executable: true,
        primary_subject_kind: "approved_path".to_string(),
        primary_subject_id: "test-subject".to_string(),
        path_snapshot: None,
        identity_snapshot: json!({"test": true}),
        evidence_summary: json!({"test": true}),
        evidence: vec![FindingEvidenceDraft {
            evidence_kind: "test".to_string(),
            subject_kind: "approved_path".to_string(),
            subject_id: Some("test-subject".to_string()),
            path_snapshot: None,
            value: json!({"source": "unit-test"}),
        }],
    }
}
