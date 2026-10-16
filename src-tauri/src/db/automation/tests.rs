use super::*;
use crate::db::{Database, InsertFileRequest};
use crate::settings::ScanRootSetting;
use rusqlite::params;
use serde_json::json;

pub(super) struct Fixture {
    db: Option<Database>,
    root: std::path::PathBuf,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let root = std::env::temp_dir().join(format!("pm02a-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("test.sqlite3")).unwrap();
        db.conn().unwrap().execute("INSERT INTO scan_roots(id,normalized_path,display_name,source_kind,enabled,health_status,current_generation,needs_reconciliation,created_at,updated_at) VALUES ('root','/tmp','Test','file_library',1,'healthy',1,0,1,1)",[]).unwrap();
        Self { db: Some(db), root }
    }
    pub(super) fn db(&self) -> &Database {
        self.db.as_ref().unwrap()
    }
    pub(super) fn set_watcher_roots(&self, roots: Vec<ScanRootSetting>) {
        self.persist_watcher_settings(roots.clone());
        self.db()
            .sync_file_library_watcher_roots(&roots)
            .expect("sync watcher roots");
    }
    pub(super) fn persist_watcher_settings(&self, roots: Vec<ScanRootSetting>) {
        let mut settings = crate::settings::get_app_settings(self.db()).expect("read settings");
        settings.default_scan_folders = roots;
        crate::settings::save_app_settings(self.db(), &settings).expect("persist watcher roots");
    }
    pub(super) fn file(&self) {
        self.db()
            .insert_file(InsertFileRequest {
                id: "file".into(),
                path: "/tmp/pm02a-source.txt".into(),
                name: "pm02a-source.txt".into(),
                extension: "txt".into(),
                size: 10,
                mtime: 1,
                ctime: 1,
                is_dir: false,
                state_code: 0,
            })
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        drop(self.db.take());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
pub(super) fn watcher_root_setting(id: &str, path: &str) -> ScanRootSetting {
    ScanRootSetting {
        id: id.to_string(),
        path: path.to_string(),
        label: id.to_string(),
        enabled: true,
        created_at: "2026-10-01T00:00:00.000Z".to_string(),
    }
}
pub(super) fn draft() -> AutomationIntentDraftV1 {
    serde_json::from_value(json!({"title":"Prepare review","workflowKind":"organize_plan","scopeQuery":{"scope":{"kind":"all_enabled_roots"},"text":"  "},"trigger":{"version":2,"kind":"manual"},"policy":{"version":1,"review":"required","autoExecute":false},"enabled":true})).unwrap()
}
pub(super) fn request(i: &AutomationIntentV1, key: &str) -> RunAutomationIntentV1 {
    RunAutomationIntentV1 {
        version: 1,
        intent_id: i.id.clone(),
        expected_intent_revision: i.revision,
        request_key: key.into(),
    }
}

#[test]
fn intent_crud_cas_and_archive_retain_history() {
    let f = Fixture::new();
    let db = f.db();
    let first = db.create_automation_intent(draft()).unwrap();
    assert_eq!(first.revision, 1);
    assert_eq!(db.list_automation_intents().unwrap().len(), 1);
    let updated = db
        .update_automation_intent(AutomationIntentUpdateV1 {
            intent_id: first.id.clone(),
            expected_revision: 1,
            draft: draft(),
        })
        .unwrap();
    assert_eq!(updated.revision, 2);
    assert!(db
        .update_automation_intent(AutomationIntentUpdateV1 {
            intent_id: first.id.clone(),
            expected_revision: 1,
            draft: draft()
        })
        .is_err());
    let paused = db
        .set_automation_intent_enabled(AutomationIntentEnabledV1 {
            intent_id: first.id.clone(),
            expected_revision: 2,
            enabled: false,
        })
        .unwrap();
    assert_eq!(paused.revision, 3);
    assert!(db
        .run_automation_intent_manual(request(&paused, "paused"))
        .is_err());
    let enabled = db
        .set_automation_intent_enabled(AutomationIntentEnabledV1 {
            intent_id: first.id.clone(),
            expected_revision: 3,
            enabled: true,
        })
        .unwrap();
    let run = db
        .run_automation_intent_manual(request(&enabled, "archive-history"))
        .unwrap();
    let archived = db
        .archive_automation_intent(AutomationIntentRevisionV1 {
            intent_id: enabled.id.clone(),
            expected_revision: 4,
        })
        .unwrap();
    assert!(archived.archived_at.is_some());
    assert!(db.list_automation_intents().unwrap().is_empty());
    assert!(db
        .run_automation_intent_manual(request(&archived, "archived"))
        .is_err());
    assert_eq!(
        db.list_automation_runs(Some(&archived.id)).unwrap()[0].id,
        run.id
    );
}
#[test]
fn invalid_contracts_unknown_fields_and_ephemeral_scopes_fail_closed() {
    let f = Fixture::new();
    let db = f.db();
    for value in ["other", "cleanup"] {
        let mut d = draft();
        d.workflow_kind = value.into();
        assert!(db.create_automation_intent(d).is_err());
    }
    let mut d = draft();
    d.trigger.kind = "scheduled".into();
    assert!(db.create_automation_intent(d).is_err());
    let mut d = draft();
    d.policy.auto_execute = true;
    assert!(db.create_automation_intent(d).is_err());
    let mut d = draft();
    d.policy.review = "optional".into();
    assert!(db.create_automation_intent(d).is_err());
    for scope in [
        json!({"kind":"current_scan","scanSessionId":"session"}),
        json!({"kind":"browse","path":"/tmp"}),
        json!({"kind":"all_enabled_roots","snapshotRevision":1}),
        json!({"kind":"roots","scanRootIds":[]}),
        json!({"kind":"roots","scanRootIds":["missing"]}),
    ] {
        let mut d = draft();
        d.scope_query = json!({"scope":scope});
        assert!(db.create_automation_intent(d).is_err());
    }
    assert!(serde_json::from_value::<RunAutomationIntentV1>(json!({"version":1,"intentId":"x","expectedIntentRevision":1,"requestKey":"k","snapshotRevision":1})).is_err());
    assert!(serde_json::from_value::<AutomationPolicyV1>(
        json!({"version":1,"review":"required","autoExecute":false,"shell":"echo"})
    )
    .is_err());
}
#[test]
fn canonical_scope_is_stable_and_revalidated_without_widening() {
    let f = Fixture::new();
    let db = f.db();
    let mut d = draft();
    d.scope_query = json!({"scope":{"kind":"roots","scanRootIds":["root","root"]},"text":"   "});
    let i = db.create_automation_intent(d).unwrap();
    let (_, _, fingerprint) =
        crate::db::canonicalize_file_query_spec(i.scope_query.clone()).unwrap();
    assert_eq!(i.scope_fingerprint, fingerprint);
    db.conn()
        .unwrap()
        .execute("UPDATE scan_roots SET enabled=0 WHERE id='root'", [])
        .unwrap();
    let r = db
        .run_automation_intent_manual(request(&i, "disabled-root"))
        .unwrap();
    assert_eq!(r.status, "blocked");
    assert!(r.result_plan_id.is_none());
    db.conn()
        .unwrap()
        .execute("DELETE FROM scan_roots WHERE id='root'", [])
        .unwrap();
    assert!(db
        .run_automation_intent_manual(request(&i, "removed-root"))
        .unwrap()
        .result_plan_id
        .is_none());
    assert!(db.list_organization_plans().unwrap().is_empty());
}
#[test]
fn duplicate_requests_and_concurrent_retries_create_one_plan() {
    let f = Fixture::new();
    f.file();
    let db = f.db();
    let i = db.create_automation_intent(draft()).unwrap();
    let first = db
        .run_automation_intent_manual(request(&i, "same-key"))
        .unwrap();
    let retry = db
        .run_automation_intent_manual(request(&i, "same-key"))
        .unwrap();
    assert_eq!(first.id, retry.id);
    assert_eq!(first.result_plan_id, retry.result_plan_id);
    assert_eq!(db.list_organization_plans().unwrap().len(), 1);
    let other = db.create_automation_intent(draft()).unwrap();
    assert!(db
        .run_automation_intent_manual(request(&other, "same-key"))
        .is_err());
    let mut stale = request(&i, "new-stale");
    stale.expected_intent_revision += 1;
    assert!(db.run_automation_intent_manual(stale).is_err());
    assert_eq!(db.list_automation_runs(None).unwrap().len(), 1);
    let a = db.clone();
    let b = db.clone();
    let ra = request(&i, "concurrent");
    let rb = request(&i, "concurrent");
    let left = std::thread::spawn(move || a.run_automation_intent_manual(ra).unwrap());
    let right = std::thread::spawn(move || b.run_automation_intent_manual(rb).unwrap());
    assert_eq!(left.join().unwrap().id, right.join().unwrap().id);
    assert_eq!(db.list_organization_plans().unwrap().len(), 2);
}
#[test]
fn every_new_run_uses_fresh_snapshot_and_stops_at_plan() {
    let f = Fixture::new();
    let db = f.db();
    let i = db.create_automation_intent(draft()).unwrap();
    let a = db
        .run_automation_intent_manual(request(&i, "before"))
        .unwrap();
    f.file();
    let b = db
        .run_automation_intent_manual(request(&i, "after"))
        .unwrap();
    assert!(b.library_snapshot_revision > a.library_snapshot_revision);
    let plan = db
        .get_organization_plan(b.result_plan_id.as_deref().unwrap())
        .unwrap();
    assert_eq!(plan.materialized_count, 1);
    assert_eq!(plan.source_kind, "all_matching");
    assert_eq!(
        plan.source_query_fingerprint.as_ref(),
        Some(&b.scope_fingerprint)
    );
    assert_eq!(
        plan.source_snapshot_revision,
        b.library_snapshot_revision.unwrap()
    );
    assert!(plan.active_execution_id.is_none());
    assert_eq!(plan.summary.accepted, 0);
    assert_eq!(
        db.conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM operation_batches", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn missing_semantics_keeps_reviewable_plan_without_rules_fallback() {
    let f = Fixture::new();
    f.file();
    let db = f.db();
    db.conn().unwrap().execute("UPDATE files SET classification_status='classified',suggested_action='Rename',suggested_name='rule-name.txt' WHERE id='file'",[]).unwrap();
    let i = db.create_automation_intent(draft()).unwrap();
    let r = db
        .run_automation_intent_manual(request(&i, "missing"))
        .unwrap();
    assert_eq!(r.status, "blocked");
    assert_eq!(r.queued_analysis_count, 0);
    assert!(r.requires_plan_refresh);
    assert_eq!(
        r.analysis_blocker_code.as_deref(),
        Some("managed_scope_missing")
    );
    let p = db
        .get_organization_plan(r.result_plan_id.as_deref().unwrap())
        .unwrap();
    assert_eq!(p.summary.needs_analysis, 1);
    assert_eq!(p.summary.accepted, 0);
}
#[test]
fn current_assessment_needs_no_credential_or_new_enqueue() {
    let f = Fixture::new();
    f.file();
    let db = f.db();
    crate::db::queries::organization::tests::seed_current_managed_semantics_for_file(db, "file");
    assert_ne!(
        crate::ai::readiness::provider_readiness(db).state,
        crate::ai::readiness::AIReadinessState::Ready
    );
    let i = db.create_automation_intent(draft()).unwrap();
    let r = db
        .run_automation_intent_manual(request(&i, "current"))
        .unwrap();
    assert_eq!(r.status, "completed");
    assert_eq!(r.queued_analysis_count, 0);
    assert!(!r.requires_plan_refresh);
    assert!(r.analysis_blocker_code.is_none());
    let p = db
        .get_organization_plan(r.result_plan_id.as_deref().unwrap())
        .unwrap();
    assert_eq!(p.summary.needs_analysis, 0);
}
#[test]
fn stale_assessment_checks_fresh_readiness_and_keeps_plan() {
    let f = Fixture::new();
    f.file();
    let db = f.db();
    crate::db::queries::organization::tests::seed_current_managed_semantics_for_file(db, "file");
    db.conn().unwrap().execute("UPDATE ai_analysis_state SET status='stale' WHERE global_entry_id='organization-test-global-file'",[]).unwrap();
    let i = db.create_automation_intent(draft()).unwrap();
    let r = db
        .run_automation_intent_manual(request(&i, "stale"))
        .unwrap();
    assert_eq!(r.status, "blocked");
    assert!(r.requires_plan_refresh);
    assert!(r.analysis_blocker_code.is_some());
    assert!(r.result_plan_id.is_some());
    assert_eq!(r.queued_analysis_count, 0);
}
#[test]
fn schema36_migration_preserves_populated35_and_constraints() {
    let f = Fixture::new();
    f.file();
    let db_path = f.root.join("old35.sqlite3");
    let db = Database::open(&db_path).unwrap();
    let conn = db.conn().unwrap();
    crate::db::drop_source_writer_schema_for_test(&conn);
    conn.execute(
        "INSERT INTO app_settings(key,value) VALUES ('preserved','content')",
        [],
    )
    .unwrap();
    conn.execute_batch(
        "DROP TABLE automation_trigger_state; DROP TABLE automation_runs; DROP TABLE automation_intents; ALTER TABLE scan_roots DROP COLUMN library_change_revision; ALTER TABLE files DROP COLUMN filesystem_observation_key; PRAGMA user_version=35;",
    )
    .unwrap();
    drop(conn);
    drop(db);
    let migrated = Database::open(&db_path).unwrap();
    let conn = migrated.conn().unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        40
    );
    assert_eq!(
        conn.query_row(
            "SELECT value FROM app_settings WHERE key='preserved'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "content"
    );
    let tables:i64=conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('automation_intents','automation_runs')",[],|r|r.get(0)).unwrap();
    assert_eq!(tables, 2);
    drop(conn);
    drop(migrated);
    Database::open(&db_path).unwrap();
    let i = f.db().create_automation_intent(draft()).unwrap();
    for (column, value) in [
        ("workflow_kind", "cleanup"),
        ("trigger_json", r#"{"version":1,"kind":"scheduled"}"#),
        (
            "policy_json",
            r#"{"version":1,"review":"required","autoExecute":true}"#,
        ),
    ] {
        assert!(f
            .db()
            .conn()
            .unwrap()
            .execute(
                &format!("UPDATE automation_intents SET {column}=?2 WHERE id=?1"),
                params![i.id, value]
            )
            .is_err());
    }
}

#[test]
fn fresh_analysis_ready_uses_existing_queue_only_and_retry_does_not_enqueue() {
    let f = Fixture::new();
    f.file();
    let db = f.db();
    crate::db::queries::organization::tests::seed_current_managed_semantics_for_file(db, "file");
    let settings = crate::ai::settings::AISettings {
        enabled: true,
        provider: crate::ai::schema::AIProviderKind::Ollama,
        preset: crate::ai::schema::AIProviderPresetId::Ollama,
        base_url: "http://127.0.0.1:11434".into(),
        model: "llama3.2".into(),
        ..Default::default()
    };
    let conn = db.conn().unwrap();
    conn.execute(
        "UPDATE app_settings SET value=?2 WHERE key=?1",
        params![
            crate::ai::settings::AI_SETTINGS_KEY,
            serde_json::to_string(&settings).unwrap()
        ],
    )
    .unwrap();
    conn.execute(
        "UPDATE managed_scopes SET allow_local_ai=1, allow_cloud_ai=0",
        [],
    )
    .unwrap();
    conn.execute("DELETE FROM ai_analysis_state", []).unwrap();
    conn.execute("DELETE FROM ai_job_items", []).unwrap();
    conn.execute("DELETE FROM ai_jobs", []).unwrap();
    drop(conn);
    let i = db.create_automation_intent(draft()).unwrap();
    let run = db
        .run_automation_intent_manual(request(&i, "local-ready"))
        .unwrap();
    assert_eq!(run.status, "completed");
    assert_eq!(run.queued_analysis_count, 1);
    assert!(run.requires_plan_refresh);
    assert!(run.analysis_blocker_code.is_none());
    let retry = db
        .run_automation_intent_manual(request(&i, "local-ready"))
        .unwrap();
    assert_eq!(run.id, retry.id);
    let conn = db.conn().unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM ai_jobs WHERE status='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM operation_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    // Crash window: queue admission committed, final receipt update did not.
    // Its outcome stays explicitly unconfirmed; retry must not admit again.
    conn.execute("UPDATE automation_runs SET status='blocked',queued_analysis_count=0,analysis_blocker_code='automation_analysis_admission_unconfirmed' WHERE id=?1",[&run.id]).unwrap();
    drop(conn);
    let interrupted = db
        .run_automation_intent_manual(request(&i, "local-ready"))
        .unwrap();
    assert_eq!(
        interrupted.analysis_blocker_code.as_deref(),
        Some("automation_analysis_admission_unconfirmed")
    );
    assert_eq!(interrupted.result_plan_id, run.result_plan_id);
    assert_eq!(
        db.conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM ai_jobs WHERE status='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn fresh_schema_and_exact_indexes_are_idempotent() {
    let f = Fixture::new();
    let conn = f.db().conn().unwrap();
    let mut statement = conn.prepare("SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_automation_%' ORDER BY name").unwrap();
    let indexes = statement
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        indexes,
        vec![
            "idx_automation_intents_active",
            "idx_automation_runs_intent",
            "idx_automation_runs_recent",
            "idx_automation_trigger_event_due",
            "idx_automation_trigger_schedule_due"
        ]
    );
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        40
    );
}

#[test]
fn fresh_analysis_blocks_on_current_disabled_provider_missing_model_and_scope_policy() {
    for (case, enabled, model, local_policy, expected) in [
        ("disabled", false, "llama3.2", true, "provider_disabled"),
        ("missing-model", true, "", true, "provider_model_missing"),
        (
            "local-policy",
            true,
            "llama3.2",
            false,
            "managed_local_ai_policy_disabled",
        ),
    ] {
        let f = Fixture::new();
        f.file();
        let db = f.db();
        crate::db::queries::organization::tests::seed_current_managed_semantics_for_file(
            db, "file",
        );
        let settings = crate::ai::settings::AISettings {
            enabled,
            provider: crate::ai::schema::AIProviderKind::Ollama,
            preset: if case == "missing-model" {
                crate::ai::schema::AIProviderPresetId::CustomOpenAICompatible
            } else {
                crate::ai::schema::AIProviderPresetId::Ollama
            },
            base_url: "http://127.0.0.1:11434".into(),
            model: model.into(),
            ..Default::default()
        };
        let conn = db.conn().unwrap();
        conn.execute(
            "UPDATE app_settings SET value=?2 WHERE key=?1",
            params![
                crate::ai::settings::AI_SETTINGS_KEY,
                serde_json::to_string(&settings).unwrap()
            ],
        )
        .unwrap();
        conn.execute(
            "UPDATE managed_scopes SET allow_local_ai=?1, allow_cloud_ai=0",
            [i64::from(local_policy)],
        )
        .unwrap();
        conn.execute("DELETE FROM ai_analysis_state", []).unwrap();
        drop(conn);
        let i = db.create_automation_intent(draft()).unwrap();
        let run = db.run_automation_intent_manual(request(&i, case)).unwrap();
        assert_eq!(run.status, "blocked", "{case}");
        assert_eq!(
            run.analysis_blocker_code.as_deref(),
            Some(expected),
            "{case}"
        );
        assert_eq!(run.queued_analysis_count, 0);
        assert!(run.result_plan_id.is_some());
        assert!(run.requires_plan_refresh);
    }
}

#[test]
fn conflicting_key_after_intent_revision_change_never_materializes_another_plan() {
    let f = Fixture::new();
    let db = f.db();
    let i = db.create_automation_intent(draft()).unwrap();
    db.run_automation_intent_manual(request(&i, "bound-revision"))
        .unwrap();
    let updated = db
        .update_automation_intent(AutomationIntentUpdateV1 {
            intent_id: i.id.clone(),
            expected_revision: 1,
            draft: draft(),
        })
        .unwrap();
    assert!(db
        .run_automation_intent_manual(request(&updated, "bound-revision"))
        .unwrap_err()
        .to_string()
        .contains("automation_request_key_conflict"));
    assert_eq!(db.list_organization_plans().unwrap().len(), 1);
    assert_eq!(db.list_automation_runs(None).unwrap()[0].intent_revision, 1);
}

#[test]
fn empty_schema35_upgrades_without_history_or_extra_automation_tables() {
    let f = Fixture::new();
    let path = f.root.join("empty35.sqlite3");
    let db = Database::open(&path).unwrap();
    let conn = db.conn().unwrap();
    crate::db::drop_source_writer_schema_for_test(&conn);
    conn.execute_batch(
        "DROP TABLE automation_trigger_state; DROP TABLE automation_runs; DROP TABLE automation_intents; ALTER TABLE scan_roots DROP COLUMN library_change_revision; ALTER TABLE files DROP COLUMN filesystem_observation_key; PRAGMA user_version=35;",
    )
        .unwrap();
    drop(conn);
    drop(db);
    let db = Database::open(&path).unwrap();
    let conn = db.conn().unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        40
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE 'automation_%'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        3
    );
    assert_eq!(db.list_automation_runs(None).unwrap().len(), 0);
}

#[test]
fn latest_receipt_uses_insertion_order_for_equal_second_timestamps() {
    let f = Fixture::new();
    let db = f.db();
    let intent = db.create_automation_intent(draft()).unwrap();
    let first = db
        .run_automation_intent_manual(request(&intent, "earlier"))
        .unwrap();
    let last = db
        .run_automation_intent_manual(request(&intent, "later"))
        .unwrap();
    db.conn()
        .unwrap()
        .execute("UPDATE automation_runs SET created_at=1", [])
        .unwrap();
    assert_ne!(first.id, last.id);
    assert_eq!(
        db.list_automation_runs(Some(&intent.id)).unwrap()[0].id,
        last.id
    );
}
