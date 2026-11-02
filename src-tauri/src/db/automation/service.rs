//! Short manual orchestration. The transaction publishes one receipt and one
//! existing Organization Plan atomically; bounded analysis admission follows.
use super::{repository::*, types::*};
use crate::ai::readiness::{managed_ai_readiness, AIReadinessState};
use crate::db::queries::{
    current_unix_seconds, library::current_library_revision,
    organization::materialize_organization_plan,
};
use crate::db::{
    AnalyzeOrganizationPlanItemsRequest, CreateOrganizationPlanRequestV1, Database, DbError,
    LibrarySelectionV1,
};
use rusqlite::{params, OptionalExtension};

// Existing semantic projection marks stale/unavailable assessments as blocked.
// Preserve those decisions; only admit their missing analysis through Organize.
const ANALYSIS_CANDIDATE: &str = "(item.validity='needs_analysis' OR (item.validity='blocked' AND item.blocking_code IN ('managed_ai_assessment_stale','managed_ai_assessment_unavailable','managed_ai_assessment_missing','managed_ai_model_changed','managed_ai_completed_job_binding_missing','managed_ai_assessment_schema_invalid','managed_ai_provider_disabled','managed_ai_provider_policy_invalid','managed_ai_scope_policy_unavailable')))";

impl Database {
    pub fn run_automation_intent_manual(
        &self,
        request: RunAutomationIntentV1,
    ) -> Result<AutomationRunV1, DbError> {
        if request.request_key.starts_with("auto:") {
            return Err(invalid("automation_request_namespace_reserved"));
        }
        self.run_automation_intent(request, None)
    }

    pub(super) fn run_automation_intent_automatic(
        &self,
        cause: &super::trigger_state::TriggerCause,
    ) -> Result<AutomationRunV1, DbError> {
        self.run_automation_intent(
            RunAutomationIntentV1 {
                version: 1,
                intent_id: cause.intent_id.clone(),
                expected_intent_revision: cause.intent_revision,
                request_key: cause.request_key.clone(),
            },
            Some(cause),
        )
    }

    fn run_automation_intent(
        &self,
        request: RunAutomationIntentV1,
        cause: Option<&super::trigger_state::TriggerCause>,
    ) -> Result<AutomationRunV1, DbError> {
        if request.version != 1
            || request.request_key.trim().is_empty()
            || request.request_key.len() > 200
            || request.request_key.chars().any(char::is_control)
        {
            return Err(invalid("automation_request_invalid"));
        }
        let mut conn = self.conn()?;
        let mut tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(cause) = cause {
            Self::validate_automation_cause(&tx, cause)?;
        }
        let intent = load_intent(&tx, &request.intent_id)?;
        require_revision(&intent, request.expected_intent_revision)?;
        if !intent.enabled {
            return Err(invalid("automation_intent_paused"));
        }
        validate_contract(&AutomationIntentDraftV1 {
            title: intent.title.clone(),
            workflow_kind: intent.workflow_kind.clone(),
            scope_query: serde_json::to_value(&intent.scope_query)?,
            trigger: intent.trigger.clone(),
            policy: intent.policy.clone(),
            enabled: intent.enabled,
        })?;
        let previous = tx
            .query_row(
                &format!("SELECT {RUN_COLUMNS} FROM automation_runs WHERE request_key=?1"),
                [&request.request_key],
                run_row,
            )
            .optional()?;
        if let Some(previous) = previous {
            if previous.intent_id != intent.id || previous.intent_revision != intent.revision {
                return Err(invalid("automation_request_key_conflict"));
            }
            return Ok(previous);
        }
        let now = current_unix_seconds();
        let mut run = AutomationRunV1 {
            id: format!("automation-run-{}", uuid::Uuid::new_v4()),
            request_key: request.request_key,
            intent_id: intent.id.clone(),
            intent_revision: intent.revision,
            trigger_kind: cause.map_or("manual", |c| c.kind.as_str()).into(),
            trigger_context: cause.map_or_else(
                || serde_json::json!({"version":1,"kind":"manual"}),
                |c| c.context.clone(),
            ),
            scope_fingerprint: intent.scope_fingerprint.clone(),
            library_snapshot_revision: None,
            status: "blocked".into(),
            result_plan_id: None,
            queued_analysis_count: 0,
            requires_plan_refresh: false,
            analysis_blocker_code: None,
            error_code: None,
            created_at: now,
            completed_at: now,
        };
        let scope = canonical_scope(&tx, serde_json::to_value(&intent.scope_query)?);
        match scope {
            Err(_) => run.error_code = Some("automation_scope_unavailable".into()),
            Ok((query, _, fingerprint)) => {
                run.scope_fingerprint = fingerprint.clone();
                let revision = current_library_revision(&tx)?;
                run.library_snapshot_revision = Some(revision);
                let outstanding: Option<(Option<String>, String)> = if cause.is_some() {
                    tx.query_row("SELECT plan.status,run.result_plan_id FROM automation_runs run LEFT JOIN organization_plans plan ON plan.id=run.result_plan_id WHERE run.intent_id=?1 AND run.result_plan_id IS NOT NULL ORDER BY run.created_at DESC,run.rowid DESC LIMIT 1",[&intent.id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?
                } else {
                    None
                };
                if let Some((Some(status), plan_id)) = outstanding.filter(|(status, _)| {
                    status
                        .as_deref()
                        .is_some_and(|s| matches!(s, "draft" | "building" | "ready" | "executing"))
                }) {
                    let _ = status;
                    run.result_plan_id = Some(plan_id);
                    run.error_code = Some("automation_review_pending".into());
                } else {
                    let savepoint = tx.savepoint()?;
                    let plan = materialize_organization_plan(
                        &savepoint,
                        CreateOrganizationPlanRequestV1 {
                            version: 1,
                            request_id: run.id.clone(),
                            title: Some(intent.title),
                            source: LibrarySelectionV1::AllMatching {
                                query: Box::new(query),
                                query_fingerprint: fingerprint,
                                snapshot_revision: revision,
                                excluded_file_ids: vec![],
                            },
                            expected_count: None,
                        },
                    );
                    match plan {
                        Ok(plan) => {
                            run.result_plan_id = Some(plan.id);
                            let missing: i64 = savepoint.query_row(
                            &format!("SELECT COUNT(*) FROM organization_plan_items item WHERE item.plan_id=?1 AND {ANALYSIS_CANDIDATE}"),
                            [run.result_plan_id.as_deref().unwrap()], |row| row.get(0),
                        )?;
                            run.requires_plan_refresh = missing > 0;
                            savepoint.commit()?;
                            run.status = if run.requires_plan_refresh || plan.summary.blocked > 0 {
                                "blocked"
                            } else {
                                "completed"
                            }
                            .into();
                            // A crash after this commit preserves a reviewable plan and
                            // terminal receipt; retry never restarts orchestration.
                            run.analysis_blocker_code = run
                                .requires_plan_refresh
                                .then(|| "automation_analysis_admission_unconfirmed".into());
                        }
                        Err(_) => {
                            drop(savepoint);
                            run.status = "failed".into();
                            run.error_code = Some("automation_plan_materialization_failed".into());
                        }
                    }
                }
            }
        }
        tx.execute("INSERT INTO automation_runs (id,request_key,intent_id,intent_revision,trigger_kind,scope_fingerprint,library_snapshot_revision,status,result_plan_id,queued_analysis_count,requires_plan_refresh,analysis_blocker_code,error_code,created_at,completed_at,trigger_context_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",params![run.id,run.request_key,run.intent_id,run.intent_revision,run.trigger_kind,run.scope_fingerprint,run.library_snapshot_revision,run.status,run.result_plan_id,run.queued_analysis_count,i64::from(run.requires_plan_refresh),run.analysis_blocker_code,run.error_code,run.created_at,run.completed_at,serde_json::to_string(&run.trigger_context)?])?;
        tx.commit()?;
        drop(conn);
        if run.requires_plan_refresh {
            // No provider is constructed here. Fresh-work admission uses existing
            // readiness/consent truth and the existing bounded analysis API.
            match self.admit_automation_analysis(&run) {
                Ok((queued, blocker)) => {
                    run.queued_analysis_count = queued;
                    run.analysis_blocker_code = blocker;
                    run.status = if run.analysis_blocker_code.is_some() {
                        "blocked"
                    } else {
                        "completed"
                    }
                    .into();
                }
                Err(_) => {
                    run.analysis_blocker_code = Some("automation_analysis_admission_failed".into());
                }
            }
            run.completed_at = current_unix_seconds();
            self.conn()?.execute("UPDATE automation_runs SET status=?2,queued_analysis_count=?3,analysis_blocker_code=?4,completed_at=?5 WHERE id=?1",params![run.id,run.status,run.queued_analysis_count,run.analysis_blocker_code,run.completed_at])?;
        }
        Ok(run)
    }

    fn admit_automation_analysis(
        &self,
        run: &AutomationRunV1,
    ) -> Result<(i64, Option<String>), DbError> {
        let plan_id = run
            .result_plan_id
            .as_deref()
            .ok_or_else(|| invalid("automation_plan_missing"))?;
        let plan = self.get_organization_plan(plan_id)?;
        let conn = self.conn()?;
        let mut stmt=conn.prepare(&format!("SELECT item.id, f.path FROM organization_plan_items item JOIN files f ON f.id=item.file_id_snapshot WHERE item.plan_id=?1 AND {ANALYSIS_CANDIDATE} ORDER BY item.ordinal LIMIT 10000"))?;
        let candidates = stmt
            .query_map([plan_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        let scopes = self.list_managed_scopes()?;
        let mut readiness_by_scope = std::collections::HashMap::new();
        let mut eligible = Vec::new();
        let mut blocker = None;
        for (item, path) in candidates {
            let normalized = crate::global_index::models::normalize_path(&path);
            // Match the existing enqueue owner's most-specific enabled path,
            // including scopes that have not yet created a managed-entry link.
            let indexed: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM global_entries WHERE path_normalized=?1 AND is_stale=0)", [&normalized], |row| row.get(0))?;
            if !indexed {
                blocker.get_or_insert("managed_scope_missing".into());
                continue;
            }
            let scope_id = scopes
                .iter()
                .filter(|scope| {
                    let scope_path = crate::global_index::models::normalize_path(&scope.path);
                    let prefix = scope_path.trim_end_matches('/');
                    scope.enabled
                        && (normalized.trim_end_matches('/') == prefix
                            || normalized.starts_with(&format!("{prefix}/")))
                })
                .max_by(|left, right| {
                    left.path
                        .len()
                        .cmp(&right.path.len())
                        .then_with(|| right.id.cmp(&left.id))
                })
                .map(|scope| scope.id.clone());
            match scope_id {
                Some(id) => {
                    let readiness = readiness_by_scope
                        .entry(id.clone())
                        .or_insert_with(|| managed_ai_readiness(self, &id));
                    if readiness.state == AIReadinessState::Ready {
                        eligible.push(item);
                    } else if blocker.is_none() {
                        blocker = Some(readiness.reason.clone());
                    }
                }
                None => {
                    if blocker.is_none() {
                        blocker = Some("managed_scope_missing".into());
                    }
                }
            }
        }
        drop(conn);
        let mut queued = 0;
        for batch in eligible.chunks(100) {
            let result =
                match self.analyze_organization_plan_items(AnalyzeOrganizationPlanItemsRequest {
                    one_off_consent: false,
                    plan_id: plan_id.into(),
                    expected_plan_revision: plan.revision,
                    item_ids: batch.to_vec(),
                }) {
                    Ok(result) => result,
                    Err(_) => {
                        return Ok((queued, Some("automation_analysis_admission_failed".into())))
                    }
                };
            queued += result.queued_count;
        }
        Ok((queued, blocker))
    }
}
