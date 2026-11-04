use super::one_off::*;
use super::*;
use crate::ai::{
    provider::{AIProvider, AIProviderError},
    schema::{AIChatRequest, AIConnectionTestResult, AIProviderKind, AIProviderPresetId},
    settings::{AISettings, AI_SETTINGS_KEY},
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fixture {
    db: Database,
    root: PathBuf,
    source: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("fylura-one-off-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("plan.sqlite3")).unwrap();
        let source = root.join("Downloads");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(
            source.join("invoice.txt"),
            b"real private content: never sent to AI",
        )
        .unwrap();
        std::fs::write(source.join("notes.txt"), b"notes").unwrap();
        std::fs::create_dir(source.join("Subfolder")).unwrap();
        let settings = AISettings {
            enabled: true,
            provider: AIProviderKind::Ollama,
            preset: AIProviderPresetId::Ollama,
            model: "fixture".into(),
            base_url: "http://127.0.0.1:11434".into(),
            ..Default::default()
        };
        db.conn()
            .unwrap()
            .execute(
                "INSERT OR REPLACE INTO app_settings(key,value) VALUES(?1,?2)",
                params![AI_SETTINGS_KEY, serde_json::to_string(&settings).unwrap()],
            )
            .unwrap();
        Self { db, root, source }
    }
    fn plan(&self) -> OrganizationPlanDto {
        let paths = std::fs::read_dir(&self.source)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        let captured = capture_folder(self.source.clone(), paths).unwrap();
        self.db
            .materialize_one_off_folder("fixture".into(), None, captured)
            .unwrap()
    }
    fn analyze(&self, plan: &OrganizationPlanDto) -> OrganizationPlanDto {
        let settings = crate::ai::settings::get_ai_settings_with_store(
            &self.db,
            &crate::ai::settings::InMemoryCredentialStore::default(),
        )
        .unwrap();
        let policy = "fixture-provider-policy".to_string();
        let provider = MetadataProvider(AtomicUsize::new(0));
        for _ in 0..2 {
            self.db
                .analyze_one_off_with_provider(
                    AnalyzeOrganizationPlanItemsRequest {
                        plan_id: plan.id.clone(),
                        expected_plan_revision: plan.revision,
                        item_ids: vec![],
                        one_off_consent: true,
                    },
                    &provider,
                    &settings,
                    &policy,
                    || true,
                )
                .unwrap();
        }
        assert_eq!(provider.0.load(Ordering::SeqCst), 2);
        self.db
            .refresh_organization_plan(OrganizationPlanRevisionRequest {
                plan_id: plan.id.clone(),
                expected_plan_revision: plan.revision,
            })
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct MetadataProvider(AtomicUsize);
impl AIProvider for MetadataProvider {
    fn chat_json(&self, request: AIChatRequest) -> Result<String, AIProviderError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let facts: serde_json::Value = serde_json::from_str(&request.messages[1].content).unwrap();
        assert_eq!(facts.as_object().unwrap().len(), 6);
        assert!(facts.get("path").is_none());
        assert!(facts.get("content").is_none());
        assert!(!request.messages[1].content.contains("real private content"));
        Ok(serde_json::json!({"version":1,"refId":facts["refId"],"fileType":"Document","purpose":"Work","lifecycle":"Reference","context":"Work documents","riskLevel":"Normal","suggestedAction":"Move","targetTemplate":"Documents","confidence":0.95,"reason":"Related work documents belong together.","keywords":["work"],"requiresConfirmation":false}).to_string())
    }
    fn test_connection(&self) -> Result<AIConnectionTestResult, AIProviderError> {
        unreachable!()
    }
}

#[test]
fn one_off_real_folder_plan_has_no_managed_or_global_registration_and_survives_reopen() {
    let f = Fixture::new();
    let plan = f.plan();
    assert_eq!(plan.source_kind, "one_off");
    assert_eq!(plan.materialized_count, 3);
    assert_eq!(plan.summary.needs_analysis, 2);
    assert_eq!(plan.summary.blocked, 1);
    let conn = f.db.conn().unwrap();
    for table in ["files", "scan_roots", "global_volumes", "global_entries"] {
        let count: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    drop(conn);
    let reopened = Database::open(f.root.join("plan.sqlite3")).unwrap();
    assert_eq!(
        reopened
            .get_organization_plan(&plan.id)
            .unwrap()
            .source_kind,
        "one_off"
    );
    assert_eq!(
        reopened.list_organization_plans().unwrap()[0].materialized_count,
        3
    );
}

#[test]
fn one_off_real_folder_rejects_missing_extra_duplicate_and_nested_sources() {
    let f = Fixture::new();
    assert!(capture_folder(f.source.clone(), vec![f.source.join("invoice.txt")]).is_err());
    assert!(capture_folder(f.source.clone(), vec![f.source.join("invoice.txt"); 2]).is_err());
    assert!(capture_folder(
        f.source.clone(),
        vec![f.source.join("Subfolder/nested.txt")]
    )
    .is_err());
    assert!(capture_folder(f.source.clone(), vec![f.source.join("invoice.txt"); 101]).is_err());
}

#[test]
fn one_off_metadata_analysis_groups_direct_files_and_keeps_subfolders_blocked() {
    let f = Fixture::new();
    let plan = f.analyze(&f.plan());
    assert_eq!(plan.summary.needs_analysis, 0);
    assert_eq!(plan.summary.blocked, 1);
    let groups =
        f.db.query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
            plan_id: plan.id.clone(),
            cursor: None,
            page_size: 100,
        })
        .unwrap();
    let group = groups
        .groups
        .iter()
        .find(|g| g.proposal_kind == "move")
        .unwrap();
    assert_eq!(group.item_count, 2);
    assert!(group.group_actions.can_accept_all);
    assert_eq!(
        group.sample_items[0].reason.as_deref(),
        Some("Related work documents belong together.")
    );
    let target =
        crate::path_identity::normalize_text_for_compare(group.target_directory.as_ref().unwrap());
    let root = crate::path_identity::normalize_for_compare(&f.source.canonicalize().unwrap());
    assert!(target.starts_with(&(root + "/")));
    assert!(f.source.join("invoice.txt").exists()); // Analysis never mutates.
}

#[test]
fn one_off_ai_consent_and_provider_are_required_without_a_managed_scope() {
    let f = Fixture::new();
    let plan = f.plan();
    let request = AnalyzeOrganizationPlanItemsRequest {
        plan_id: plan.id,
        expected_plan_revision: plan.revision,
        item_ids: vec![],
        one_off_consent: false,
    };
    assert!(f
        .db
        .analyze_one_off_organization_plan(request.clone())
        .unwrap_err()
        .to_string()
        .contains("consent_required"));
    f.db.conn()
        .unwrap()
        .execute("DELETE FROM app_settings WHERE key=?1", [AI_SETTINGS_KEY])
        .unwrap();
    let mut request = request;
    request.one_off_consent = true;
    assert!(f
        .db
        .analyze_one_off_organization_plan(request)
        .unwrap_err()
        .to_string()
        .contains("provider_not_ready"));
}

#[test]
fn one_off_snapshot_invalidates_changed_deleted_renamed_and_replaced_files() {
    let f = Fixture::new();
    let plan = f.analyze(&f.plan());
    let conn = f.db.conn().unwrap();
    let id: String = conn
        .query_row(
            "SELECT id FROM organization_plan_sources WHERE plan_id=?1 AND name='invoice.txt'",
            [&plan.id],
            |r| r.get(0),
        )
        .unwrap();
    let path = f.source.join("invoice.txt");
    std::fs::rename(&path, f.source.join("renamed.txt")).unwrap();
    assert!(load_source(&conn, &id).unwrap().is_none());
    std::fs::write(&path, b"a different object").unwrap();
    assert!(load_source(&conn, &id).unwrap().is_none());
    std::fs::remove_file(&path).unwrap();
    assert!(load_source(&conn, &id).unwrap().is_none());
    let fresh_id: String = conn
        .query_row(
            "SELECT id FROM organization_plan_sources WHERE plan_id=?1 AND name='notes.txt'",
            [&plan.id],
            |r| r.get(0),
        )
        .unwrap();
    std::fs::write(f.source.join("notes.txt"), b"changed content and size").unwrap();
    assert!(load_source(&conn, &fresh_id).unwrap().is_none());
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn one_off_native_filesystem_analysis_group_execute_journal_and_restore() {
    use crate::file_ops::*;
    struct Quiet;
    impl OperationProgressEmitter for Quiet {
        fn emit_progress(&self, _: OperationProgressPayload) {}
    }
    let f = Fixture::new();
    let plan = f.analyze(&f.plan());
    let groups =
        f.db.query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
            plan_id: plan.id.clone(),
            cursor: None,
            page_size: 100,
        })
        .unwrap();
    let group = groups
        .groups
        .iter()
        .find(|g| g.proposal_kind == "move")
        .unwrap();
    let plan =
        f.db.update_organization_plan_group_decision(UpdateOrganizationPlanGroupDecisionRequest {
            edited_target_template: None,
            plan_id: plan.id,
            group_id: group.group_id.clone(),
            expected_plan_revision: plan.revision,
            expected_projection_fingerprint: group.projection_fingerprint.clone(),
            expected_item_count: 2,
            decision: "accepted".into(),
        })
        .unwrap()
        .plan;
    let dry =
        f.db.get_organization_plan_dry_run(OrganizationPlanSelectionRequest {
            plan_id: plan.id.clone(),
            expected_plan_revision: plan.revision,
            item_ids: vec![],
            all_accepted: true,
        })
        .unwrap();
    assert_eq!(dry.executable_count, 2);
    let request = ExecuteOrganizationPlanRequest {
        plan_id: plan.id.clone(),
        expected_plan_revision: plan.revision,
        dry_run_fingerprint: dry.dry_run_fingerprint,
        item_ids: vec![],
        all_accepted: true,
        confirmed: true,
    };
    let dispatch = f.db.begin_organization_plan_execution(&request).unwrap();
    let result = execute_moves_with_persistence_with_progress_and_app_data(
        &f.db,
        ExecuteMovesRequest {
            operations: dispatch.selections.clone(),
        },
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        &Quiet,
        None,
        Some(dispatch.operation_batch_id.clone()),
    )
    .unwrap();
    assert!(
        result.logs.iter().all(|l| l.status == "success"),
        "one-off outcomes: {:?}",
        result.logs
    );
    let outcome =
        f.db.finalize_organization_plan_execution(&plan.id, &dispatch, &result.logs)
            .unwrap();
    assert_eq!(outcome.succeeded_count, 2);
    assert!(!f.source.join("invoice.txt").exists());
    assert!(f.source.join("Documents/invoice.txt").exists());
    let logs =
        f.db.get_restorable_operation_logs_by_ids(
            &result.logs.iter().map(|l| l.id.clone()).collect::<Vec<_>>(),
        )
        .unwrap();
    assert_eq!(logs.len(), 2);
    let restored = restore_moves_with_persistence(&f.db, RestoreMovesRequest { logs }).unwrap();
    assert_eq!(restored.restored, 2);
    assert_eq!(
        std::fs::read(f.source.join("invoice.txt")).unwrap(),
        b"real private content: never sent to AI"
    );
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn one_off_native_single_file_name_edit_execute_journal_and_restore() {
    use crate::file_ops::*;
    struct Quiet;
    impl OperationProgressEmitter for Quiet {
        fn emit_progress(&self, _: OperationProgressPayload) {}
    }

    let f = Fixture::new();
    let plan = f.analyze(&f.plan());
    let items =
        f.db.query_organization_plan_items(QueryOrganizationPlanItemsRequest {
            plan_id: plan.id.clone(),
            cursor: None,
            page_size: 100,
        })
        .unwrap();
    let invoice = items
        .items
        .iter()
        .find(|item| item.source_name_snapshot == "invoice.txt")
        .unwrap();
    let original_fingerprint = invoice.proposal_fingerprint.clone();
    assert_eq!(invoice.proposal_kind, "move");
    assert!(invoice
        .available_actions
        .iter()
        .any(|action| action == "edit_name"));

    let reviewed =
        f.db.update_organization_plan_decisions(UpdateOrganizationPlanDecisionsRequest {
            plan_id: plan.id.clone(),
            expected_plan_revision: plan.revision,
            safe_batch: false,
            mutations: vec![OrganizationDecisionMutation {
                item_id: invoice.id.clone(),
                expected_item_revision: invoice.revision,
                decision: "edited".into(),
                edited_filename: Some("reviewed-invoice.txt".into()),
            }],
        })
        .unwrap();
    assert_eq!(reviewed.revision, plan.revision + 1);
    let reviewed_item =
        f.db.query_organization_plan_items(QueryOrganizationPlanItemsRequest {
            plan_id: plan.id.clone(),
            cursor: None,
            page_size: 100,
        })
        .unwrap()
        .items
        .into_iter()
        .find(|item| item.id == invoice.id)
        .unwrap();
    assert_eq!(reviewed_item.decision, "edited");
    assert_eq!(
        reviewed_item.edited_name.as_deref(),
        Some("reviewed-invoice.txt")
    );
    assert_eq!(reviewed_item.proposal_fingerprint, original_fingerprint);

    let dry =
        f.db.get_organization_plan_dry_run(OrganizationPlanSelectionRequest {
            plan_id: plan.id.clone(),
            expected_plan_revision: reviewed.revision,
            item_ids: vec![invoice.id.clone()],
            all_accepted: false,
        })
        .unwrap();
    assert_eq!(dry.executable_count, 1, "dry run: {:?}", dry.items);
    assert_eq!(
        std::path::Path::new(&dry.items[0].to).file_name().unwrap(),
        "reviewed-invoice.txt"
    );

    let dispatch =
        f.db.begin_organization_plan_execution(&ExecuteOrganizationPlanRequest {
            plan_id: plan.id.clone(),
            expected_plan_revision: reviewed.revision,
            dry_run_fingerprint: dry.dry_run_fingerprint,
            item_ids: vec![invoice.id.clone()],
            all_accepted: false,
            confirmed: true,
        })
        .unwrap();
    let result = execute_moves_with_persistence_with_progress_and_app_data(
        &f.db,
        ExecuteMovesRequest {
            operations: dispatch.selections.clone(),
        },
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        &Quiet,
        None,
        Some(dispatch.operation_batch_id.clone()),
    )
    .unwrap();
    assert!(result.logs.iter().all(|log| log.status == "success"));
    assert_eq!(
        f.db.finalize_organization_plan_execution(&plan.id, &dispatch, &result.logs)
            .unwrap()
            .succeeded_count,
        1
    );
    let moved = f.source.join("Documents/reviewed-invoice.txt");
    assert!(moved.exists());
    let logs =
        f.db.get_restorable_operation_logs_by_ids(
            &result
                .logs
                .iter()
                .map(|log| log.id.clone())
                .collect::<Vec<_>>(),
        )
        .unwrap();
    assert_eq!(logs.len(), 1);
    let restored = restore_moves_with_persistence(&f.db, RestoreMovesRequest { logs }).unwrap();
    assert_eq!(restored.restored, 1);
    assert!(!moved.exists());
    assert_eq!(
        std::fs::read(f.source.join("invoice.txt")).unwrap(),
        b"real private content: never sent to AI"
    );
}

#[test]
fn one_off_group_folder_edit_is_bounded_and_does_not_approve_items() {
    let f = Fixture::new();
    let plan = f.analyze(&f.plan());
    let groups =
        f.db.query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
            plan_id: plan.id.clone(),
            cursor: None,
            page_size: 100,
        })
        .unwrap();
    let group = groups
        .groups
        .iter()
        .find(|g| g.proposal_kind == "move")
        .unwrap();
    let request = UpdateOrganizationPlanGroupDecisionRequest {
        plan_id: plan.id.clone(),
        group_id: group.group_id.clone(),
        expected_plan_revision: plan.revision,
        expected_projection_fingerprint: group.projection_fingerprint.clone(),
        expected_item_count: 2,
        decision: "undecided".into(),
        edited_target_template: Some("../outside".into()),
    };
    assert!(f
        .db
        .update_organization_plan_group_decision(request.clone())
        .unwrap_err()
        .to_string()
        .contains("folder_invalid"));
    let mut request = request;
    request.edited_target_template = Some("Project Documents".into());
    let edited =
        f.db.update_organization_plan_group_decision(request)
            .unwrap()
            .plan;
    assert_eq!(edited.summary.accepted, 0);
    assert_eq!(edited.summary.undecided, 3);
    let groups =
        f.db.query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
            plan_id: plan.id,
            cursor: None,
            page_size: 100,
        })
        .unwrap();
    let group = groups
        .groups
        .iter()
        .find(|g| g.proposal_kind == "move")
        .unwrap();
    assert_eq!(group.item_count, 2);
    assert!(group
        .target_directory
        .as_ref()
        .unwrap()
        .ends_with("Project Documents"));
    assert!(f.source.join("invoice.txt").exists());
    assert!(!f.source.join("Project Documents").exists());
}

#[test]
fn one_off_schema_46_upgrade_preserves_legacy_plans_and_ddl_failure_rolls_back() {
    for fail in [false, true] {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::migrate(&conn).unwrap();
        conn.execute_batch("INSERT INTO organization_plans(id,title,status,source_kind,source_snapshot_revision,requested_count,materialized_count,planner_version,revision,created_at,updated_at) VALUES('legacy','Legacy Plan','ready','explicit',0,0,0,1,1,1,1); DROP TABLE organization_plan_sources; ALTER TABLE organization_plans DROP COLUMN one_off_folder_path; PRAGMA user_version=46;").unwrap();
        if fail {
            conn.execute_batch("CREATE TABLE organization_plan_sources(intruder TEXT)")
                .unwrap();
        }
        let result = crate::db::schema::migrate(&conn);
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        let title: String = conn
            .query_row(
                "SELECT title FROM organization_plans WHERE id='legacy'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(title, "Legacy Plan");
        if fail {
            assert!(result.is_err());
            assert_eq!(version, 46);
            assert!(conn
                .prepare("SELECT one_off_folder_path FROM organization_plans")
                .is_err());
        } else {
            result.unwrap();
            assert_eq!(version, 47);
            assert!(conn
                .prepare("SELECT one_off_folder_path FROM organization_plans")
                .is_ok());
        }
    }
}

#[test]
fn one_off_policy_revocation_prevents_assessment_publication() {
    let f = Fixture::new();
    let plan = f.plan();
    let provider = MetadataProvider(AtomicUsize::new(0));
    let settings = crate::ai::settings::get_ai_settings_with_store(
        &f.db,
        &crate::ai::settings::InMemoryCredentialStore::default(),
    )
    .unwrap();
    let calls = AtomicUsize::new(0);
    let result = f.db.analyze_one_off_with_provider(
        AnalyzeOrganizationPlanItemsRequest {
            plan_id: plan.id.clone(),
            expected_plan_revision: plan.revision,
            item_ids: vec![],
            one_off_consent: true,
        },
        &provider,
        &settings,
        "fixture-policy",
        || calls.fetch_add(1, Ordering::SeqCst) == 0,
    );
    assert!(result.unwrap_err().to_string().contains("policy_changed"));
    let count: i64 =
        f.db.conn()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM organization_plan_sources WHERE assessment_json IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn one_off_provider_changes_keep_saved_advice_provenance_and_require_review_and_confirmation() {
    let f = Fixture::new();
    let mut plan = f.analyze(&f.plan());
    let item =
        f.db.query_organization_plan_items(QueryOrganizationPlanItemsRequest {
            plan_id: plan.id.clone(),
            cursor: None,
            page_size: 100,
        })
        .unwrap()
        .items
        .into_iter()
        .find(|item| item.source_name_snapshot == "invoice.txt")
        .unwrap();
    assert_eq!(
        item.semantic_explanation
            .as_ref()
            .unwrap()
            .provider_kind
            .as_deref(),
        Some("ollama")
    );
    assert_eq!(
        item.semantic_explanation.as_ref().unwrap().model.as_deref(),
        Some("fixture")
    );
    let item_id = item.id.clone();
    let original_fingerprint = item.proposal_fingerprint.clone();

    let credentials = crate::ai::settings::InMemoryCredentialStore::default();
    let mut settings =
        crate::ai::settings::get_ai_settings_with_store(&f.db, &credentials).unwrap();
    settings.model = "new-model".into();
    {
        let conn = f.db.conn().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO app_settings(key,value) VALUES(?1,?2)",
            params![AI_SETTINGS_KEY, serde_json::to_string(&settings).unwrap()],
        )
        .unwrap();
    }
    plan =
        f.db.refresh_organization_plan(OrganizationPlanRevisionRequest {
            plan_id: plan.id.clone(),
            expected_plan_revision: plan.revision,
        })
        .unwrap();
    let current_item =
        f.db.query_organization_plan_items(QueryOrganizationPlanItemsRequest {
            plan_id: plan.id.clone(),
            cursor: None,
            page_size: 100,
        })
        .unwrap()
        .items
        .into_iter()
        .find(|item| item.id == item_id)
        .unwrap();
    assert_eq!(current_item.proposal_fingerprint, original_fingerprint);
    assert_eq!(
        current_item
            .semantic_explanation
            .as_ref()
            .unwrap()
            .model
            .as_deref(),
        Some("fixture")
    );
    assert_eq!(
        current_item
            .semantic_explanation
            .as_ref()
            .unwrap()
            .provider_settings_current,
        Some(false)
    );

    let reviewed =
        f.db.update_organization_plan_decisions(UpdateOrganizationPlanDecisionsRequest {
            plan_id: plan.id.clone(),
            expected_plan_revision: plan.revision,
            safe_batch: false,
            mutations: vec![OrganizationDecisionMutation {
                item_id: current_item.id.clone(),
                expected_item_revision: current_item.revision,
                decision: "accepted".into(),
                edited_filename: None,
            }],
        })
        .unwrap();
    settings.enabled = false;
    {
        let conn = f.db.conn().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO app_settings(key,value) VALUES(?1,?2)",
            params![AI_SETTINGS_KEY, serde_json::to_string(&settings).unwrap()],
        )
        .unwrap();
    }
    let new_analysis =
        f.db.analyze_one_off_organization_plan(AnalyzeOrganizationPlanItemsRequest {
            plan_id: plan.id.clone(),
            expected_plan_revision: reviewed.revision,
            item_ids: vec![current_item.id.clone()],
            one_off_consent: true,
        })
        .unwrap_err();
    assert!(new_analysis
        .to_string()
        .contains("one_off_ai_provider_not_ready"));

    plan =
        f.db.refresh_organization_plan(OrganizationPlanRevisionRequest {
            plan_id: plan.id.clone(),
            expected_plan_revision: reviewed.revision,
        })
        .unwrap();
    let after_disable =
        f.db.query_organization_plan_items(QueryOrganizationPlanItemsRequest {
            plan_id: plan.id.clone(),
            cursor: None,
            page_size: 100,
        })
        .unwrap()
        .items
        .into_iter()
        .find(|item| item.id == current_item.id)
        .unwrap();
    assert_eq!(after_disable.decision, "accepted");
    assert_eq!(
        after_disable
            .semantic_explanation
            .as_ref()
            .unwrap()
            .model
            .as_deref(),
        Some("fixture")
    );
    assert_eq!(
        after_disable
            .semantic_explanation
            .as_ref()
            .unwrap()
            .provider_settings_current,
        Some(false)
    );
    let dry =
        f.db.get_organization_plan_dry_run(OrganizationPlanSelectionRequest {
            plan_id: plan.id.clone(),
            expected_plan_revision: plan.revision,
            item_ids: vec![current_item.id.clone()],
            all_accepted: false,
        })
        .unwrap();
    assert_eq!(dry.executable_count, 1);
    assert!(f.source.join("invoice.txt").exists());
    assert!(f.source.join("notes.txt").exists());
    let unconfirmed =
        f.db.begin_organization_plan_execution(&ExecuteOrganizationPlanRequest {
            plan_id: plan.id,
            expected_plan_revision: dry.plan_revision,
            dry_run_fingerprint: dry.dry_run_fingerprint,
            item_ids: vec![after_disable.id],
            all_accepted: false,
            confirmed: false,
        })
        .err()
        .unwrap();
    assert!(unconfirmed
        .to_string()
        .contains("organization_execution_confirmation_required"));
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn one_off_native_cancel_reissue_partial_failure_and_restore_eligibility() {
    use crate::file_ops::*;
    struct Quiet;
    impl OperationProgressEmitter for Quiet {
        fn emit_progress(&self, _: OperationProgressPayload) {}
    }
    let f = Fixture::new();
    let plan = f.analyze(&f.plan());
    let group =
        f.db.query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
            plan_id: plan.id.clone(),
            cursor: None,
            page_size: 100,
        })
        .unwrap()
        .groups
        .into_iter()
        .find(|g| g.proposal_kind == "move")
        .unwrap();
    let mut plan =
        f.db.update_organization_plan_group_decision(UpdateOrganizationPlanGroupDecisionRequest {
            plan_id: plan.id.clone(),
            group_id: group.group_id,
            expected_plan_revision: plan.revision,
            expected_projection_fingerprint: group.projection_fingerprint,
            expected_item_count: 2,
            decision: "accepted".into(),
            edited_target_template: None,
        })
        .unwrap()
        .plan;
    for cancelled in [true, false] {
        let dry =
            f.db.get_organization_plan_dry_run(OrganizationPlanSelectionRequest {
                plan_id: plan.id.clone(),
                expected_plan_revision: plan.revision,
                item_ids: vec![],
                all_accepted: true,
            })
            .unwrap();
        assert_eq!(dry.executable_count, 2);
        let request = ExecuteOrganizationPlanRequest {
            plan_id: plan.id.clone(),
            expected_plan_revision: plan.revision,
            dry_run_fingerprint: dry.dry_run_fingerprint,
            item_ids: vec![],
            all_accepted: true,
            confirmed: true,
        };
        let dispatch = f.db.begin_organization_plan_execution(&request).unwrap();
        if !cancelled {
            let target = Path::new(&dispatch.selections[1].target_path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, b"unrelated conflicting data").unwrap();
        }
        let result = execute_moves_with_persistence_with_progress_and_app_data(
            &f.db,
            ExecuteMovesRequest {
                operations: dispatch.selections.clone(),
            },
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(cancelled)),
            &Quiet,
            None,
            Some(dispatch.operation_batch_id.clone()),
        )
        .unwrap();
        let outcome =
            f.db.finalize_organization_plan_execution(&plan.id, &dispatch, &result.logs)
                .unwrap();
        if cancelled {
            assert_eq!(outcome.succeeded_count, 0);
            assert_eq!(outcome.skipped_count, 2);
            assert!(result.logs.iter().all(|l| !l.can_restore));
            assert!(f.source.join("invoice.txt").exists());
            assert!(f.source.join("notes.txt").exists());
            plan =
                f.db.refresh_organization_plan(OrganizationPlanRevisionRequest {
                    plan_id: plan.id.clone(),
                    expected_plan_revision: outcome.plan.revision,
                })
                .unwrap();
        } else {
            assert_eq!(outcome.succeeded_count, 1);
            assert_eq!(outcome.failed_count, 1);
            assert_eq!(
                std::fs::read(&dispatch.selections[1].target_path).unwrap(),
                b"unrelated conflicting data"
            );
            assert!(Path::new(&dispatch.selections[1].source_path).exists());
            assert!(result.logs[0].can_restore);
            assert!(!result.logs[1].can_restore);
            let restore = restore_moves_with_persistence(
                &f.db,
                RestoreMovesRequest {
                    logs: f
                        .db
                        .get_restorable_operation_logs_by_ids(std::slice::from_ref(
                            &result.logs[0].id,
                        ))
                        .unwrap(),
                },
            )
            .unwrap();
            assert_eq!(restore.restored, 1);
        }
    }
}

struct RewritingProvider(serde_json::Value);
impl AIProvider for RewritingProvider {
    fn chat_json(&self, request: AIChatRequest) -> Result<String, AIProviderError> {
        let raw = MetadataProvider(AtomicUsize::new(0)).chat_json(request)?;
        let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        for (key, patch) in self.0.as_object().unwrap() {
            value[key] = patch.clone();
        }
        Ok(value.to_string())
    }
    fn test_connection(&self) -> Result<AIConnectionTestResult, AIProviderError> {
        unreachable!()
    }
}
#[test]
fn one_off_uncertain_and_unanalyzable_items_are_not_approved_or_reported_as_analyzed() {
    for (patch, analyzed) in [
        (
            serde_json::json!({"confidence":0.3,"requiresConfirmation":true}),
            true,
        ),
        (serde_json::json!({"refId":"wrong-source"}), false),
    ] {
        let f = Fixture::new();
        let plan = f.plan();
        let settings = crate::ai::settings::get_ai_settings_with_store(
            &f.db,
            &crate::ai::settings::InMemoryCredentialStore::default(),
        )
        .unwrap();
        let result =
            f.db.analyze_one_off_with_provider(
                AnalyzeOrganizationPlanItemsRequest {
                    plan_id: plan.id.clone(),
                    expected_plan_revision: plan.revision,
                    item_ids: vec![],
                    one_off_consent: true,
                },
                &RewritingProvider(patch),
                &settings,
                "fixture-policy",
                || true,
            )
            .unwrap();
        assert_eq!(result.queued_count, i64::from(analyzed));
        let plan =
            f.db.refresh_organization_plan(OrganizationPlanRevisionRequest {
                plan_id: plan.id,
                expected_plan_revision: plan.revision,
            })
            .unwrap();
        assert_eq!(plan.summary.accepted, 0);
        if analyzed {
            let groups =
                f.db.query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                    plan_id: plan.id,
                    cursor: None,
                    page_size: 100,
                })
                .unwrap();
            assert!(groups
                .groups
                .iter()
                .any(|g| g.confidence_band == "low" && g.readiness == "requires-decision"));
        } else {
            assert_eq!(plan.summary.needs_analysis, 2);
        }
        assert!(f.source.join("invoice.txt").exists());
    }
}

#[cfg(unix)]
#[test]
fn one_off_parent_alias_is_resolved_before_group_target_and_journal_derivation() {
    let f = Fixture::new();
    let alias = f.root.join("selected-parent-alias");
    std::os::unix::fs::symlink(&f.root, &alias).unwrap();
    let selected = alias.join("Downloads");
    let paths = std::fs::read_dir(&selected)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let captured = capture_folder(selected, paths).unwrap();
    let plan =
        f.db.materialize_one_off_folder("alias".into(), None, captured)
            .unwrap();
    let page =
        f.db.query_organization_plan_items(QueryOrganizationPlanItemsRequest {
            plan_id: plan.id.clone(),
            cursor: None,
            page_size: 100,
        })
        .unwrap();
    let root = f.source.canonicalize().unwrap();
    for item in page.items {
        assert_eq!(
            Path::new(&item.source_path_snapshot).parent(),
            Some(root.as_path())
        );
    }
    let plan = f.analyze(&plan);
    let groups =
        f.db.query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
            plan_id: plan.id,
            cursor: None,
            page_size: 100,
        })
        .unwrap();
    assert!(groups.groups.iter().any(|g| g
        .target_directory
        .as_ref()
        .is_some_and(|path| Path::new(path) == root.join("Documents"))));
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn organization_transfer_root_allows_regular_files_into_missing_children_only() {
    use crate::file_ops::manual_transfer::{make_operation, revalidate, TransferAction};
    use std::fs;
    let f = Fixture::new();
    let source = f.root.join("one-off.txt");
    fs::write(&source, b"selected file").unwrap();
    let root = f.root.canonicalize().unwrap();
    let source = source.canonicalize().unwrap();
    let target = root.join("Documents/one-off.txt");
    let operation =
        make_operation("source", &source, &root, &target, TransferAction::Move).unwrap();
    revalidate(&operation).unwrap();
    assert!(!root.join("Documents").exists()); // Preview has no filesystem side effect.
    assert!(make_operation(
        "source",
        &source,
        &root,
        &root.join("one-off.txt"),
        TransferAction::Move
    )
    .is_err());
    let directory = root.join("Subfolder");
    fs::create_dir(&directory).unwrap();
    assert!(make_operation(
        "directory",
        &directory,
        &root,
        &directory.join("nested"),
        TransferAction::Move
    )
    .is_err());
}
