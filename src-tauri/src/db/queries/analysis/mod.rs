use super::*;
use crate::ids::new_job_id;
use rusqlite::{
    params, params_from_iter, Connection, OptionalExtension, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

mod grouping;
mod projection;
pub use grouping::AnalysisFindingGroupDto;

use projection::{
    aggregate_path_is_same_or_child, analysis_decision_from_row, analysis_detector_from_row,
    analysis_evidence_from_row, analysis_finding_from_row, analysis_run_from_row,
    deterministic_finding_id, higher_risk_tier, load_analysis_detectors,
    managed_analysis_file_from_row, normalized_aggregate_path, parse_finding_cursor,
    query_analysis_run, query_dedupe_authority, tier_order, ANALYSIS_DETECTOR_SELECT,
    ANALYSIS_FINDING_SELECT, ANALYSIS_RUN_SELECT,
};

pub const ANALYSIS_REGISTRY_VERSION: i64 = 1;
pub const ANALYSIS_FINDING_DETAIL_LIMIT: usize = 1000;
pub const ANALYSIS_PRUNE_ROW_BUDGET: usize = 1000;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisScopeRequest {
    pub kind: String,
    #[serde(default)]
    pub root_ids: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartAnalysisRunRequest {
    pub scope: AnalysisScopeRequest,
    #[serde(default)]
    pub detector_ids: Vec<String>,
    #[serde(default)]
    pub request_key: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisRunDto {
    pub id: String,
    pub request_key: String,
    pub request_attempt: i64,
    pub scope: Value,
    pub scope_hash: String,
    pub source_snapshot: Value,
    pub source_snapshot_hash: String,
    pub detector_set: Vec<String>,
    pub detector_set_hash: String,
    pub status: String,
    pub phase: String,
    pub revision: i64,
    pub cancel_requested: bool,
    pub rerun_required: bool,
    pub detectors_total: i64,
    pub detectors_completed: i64,
    pub detectors_failed: i64,
    pub findings_staged: i64,
    pub findings_published: i64,
    pub safe_count: i64,
    pub review_count: i64,
    pub caution_count: i64,
    pub exact_reclaimable_bytes: i64,
    pub potential_reclaimable_bytes: i64,
    pub warning_count: i64,
    pub error_count: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub last_checkpoint_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisDetectorDto {
    pub run_id: String,
    pub detector_id: String,
    pub detector_version: i64,
    pub status: String,
    pub revision: i64,
    pub scanned_subjects: i64,
    pub findings_staged: i64,
    pub findings_published: i64,
    pub exact_reclaimable_bytes: i64,
    pub potential_reclaimable_bytes: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisFindingDto {
    pub id: String,
    pub finding_key: String,
    pub run_id: String,
    pub detector_id: String,
    pub detector_version: i64,
    pub scope_hash: String,
    pub status: String,
    pub tier: String,
    pub category: String,
    pub action_kind: String,
    pub title: String,
    pub reason: String,
    pub risk_note: Option<String>,
    pub confidence: String,
    pub size_bytes: i64,
    pub exact_reclaimable_bytes: Option<i64>,
    pub potential_reclaimable_bytes: i64,
    pub requires_confirmation: bool,
    pub executable: bool,
    pub primary_subject_kind: String,
    pub primary_subject_id: String,
    pub path_snapshot: Option<String>,
    pub identity_snapshot: Value,
    pub evidence_summary: Value,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub published_at: Option<i64>,
    pub stale_at: Option<i64>,
    pub decision: Option<String>,
    pub snoozed_until: Option<i64>,
    pub decision_revision: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisFindingEvidenceDto {
    pub id: String,
    pub finding_id: String,
    pub evidence_kind: String,
    pub subject_kind: String,
    pub subject_id: Option<String>,
    pub path_snapshot: Option<String>,
    pub value: Value,
    pub created_at: i64,
    pub is_current_assessment: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisFindingDecisionDto {
    pub finding_key: String,
    pub decision: String,
    pub snoozed_until: Option<i64>,
    pub note: Option<String>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisFindingPageDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_revision: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<AnalysisFindingGroupDto>>,
    pub findings: Vec<AnalysisFindingDto>,
    pub next_cursor: Option<String>,
    pub limit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DedupeAuthorityDto {
    pub revision: i64,
    pub status: String,
    pub last_authoritative_run_id: Option<String>,
    pub scope_hash: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct AnalysisAdmission {
    pub run: AnalysisRunDto,
    pub created: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct FindingEvidenceDraft {
    pub evidence_kind: String,
    pub subject_kind: String,
    pub subject_id: Option<String>,
    pub path_snapshot: Option<String>,
    pub value: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct FindingDraft {
    pub id: String,
    pub finding_key: String,
    pub detector_id: String,
    pub detector_version: i64,
    pub tier: String,
    pub category: String,
    pub action_kind: String,
    pub title: String,
    pub reason: String,
    pub risk_note: Option<String>,
    pub confidence: String,
    pub size_bytes: i64,
    pub exact_reclaimable_bytes: Option<i64>,
    pub potential_reclaimable_bytes: i64,
    pub requires_confirmation: bool,
    pub executable: bool,
    pub primary_subject_kind: String,
    pub primary_subject_id: String,
    pub path_snapshot: Option<String>,
    pub identity_snapshot: Value,
    pub evidence_summary: Value,
    pub evidence: Vec<FindingEvidenceDraft>,
}

#[derive(Debug, Clone)]
pub(crate) struct AnalysisAiFindingPrecondition {
    pub finding_id: String,
    pub finding_key: String,
    pub run_id: String,
    pub detector_id: String,
    pub detector_version: i64,
    pub detector_status: String,
    pub detector_revision: i64,
    pub scope_hash: String,
    pub expected_revision: i64,
    pub primary_subject_kind: String,
    pub primary_subject_id: String,
    pub path_snapshot: Option<String>,
    pub identity_snapshot_json: String,
}

#[derive(Debug, Clone)]
pub(crate) struct AnalysisAiAssessmentPublication {
    pub finding_id: String,
    pub requested_tier: String,
    pub requested_trash_allowed: bool,
    pub evidence: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct AnalysisAiPublicationBatch {
    pub run_id: String,
    pub expected_run_revision: i64,
    pub expected_source_snapshot_hash: String,
    pub expected_detector_set_hash: String,
    pub settings_key: String,
    pub expected_settings_revision: String,
    pub expected_candidate_set_fingerprint: String,
    pub preconditions: Vec<AnalysisAiFindingPrecondition>,
    pub assessments: Vec<AnalysisAiAssessmentPublication>,
}

#[derive(Debug, Clone)]
pub(crate) struct ManagedAnalysisFingerprint {
    pub identity_status: String,
    pub platform_kind: String,
    pub platform_volume_id: Option<String>,
    pub platform_file_id: Option<String>,
    pub physical_key: Option<String>,
    pub size: i64,
    pub modified_ns: Option<i64>,
    pub full_hash: Option<String>,
    pub fingerprint_status: String,
    pub revision: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct ManagedAnalysisFile {
    pub file_id: String,
    pub path: String,
    pub size: i64,
    pub mtime: i64,
    pub is_stale: bool,
    pub fingerprint: Option<ManagedAnalysisFingerprint>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct FindingCursor {
    pub version: i64,
    pub tier_order: i64,
    pub potential_reclaimable_bytes: i64,
    pub updated_at: i64,
    pub id: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AnalysisFindingFilter {
    pub run_id: Option<String>,
    pub detector_id: Option<String>,
    pub tier: Option<String>,
    pub category: Option<String>,
    pub decision: Option<String>,
    pub status: Option<String>,
    pub executable_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnalysisPublishOutcome {
    Completed,
    CompletedWithWarnings,
    Cancelled,
}

impl Database {
    pub(crate) fn start_analysis_run(
        &self,
        request: &StartAnalysisRunRequest,
        detectors: &[(String, i64)],
    ) -> Result<AnalysisAdmission, DbError> {
        let request_key = request
            .request_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| new_job_id("analysis-request"));
        self.admit_analysis_run(&request.scope, &request_key, detectors, false)
    }

    pub(crate) fn retry_analysis_run(
        &self,
        run_id: &str,
        detectors: &[(String, i64)],
    ) -> Result<AnalysisAdmission, DbError> {
        let run = self.get_analysis_run(run_id)?;
        if matches!(run.status.as_str(), "queued" | "running" | "cancelling") {
            return Err(DbError::Validation(
                "An active analysis run cannot be retried.".to_string(),
            ));
        }
        let scope = scope_request_from_value(&run.scope)?;
        self.admit_analysis_run(&scope, &run.request_key, detectors, true)
    }

    fn admit_analysis_run(
        &self,
        requested_scope: &AnalysisScopeRequest,
        request_key: &str,
        detectors: &[(String, i64)],
        force_retry: bool,
    ) -> Result<AnalysisAdmission, DbError> {
        if request_key.len() > 256 {
            return Err(DbError::Validation(
                "Analysis request key is too long.".to_string(),
            ));
        }
        if detectors.is_empty() {
            return Err(DbError::Validation(
                "At least one fixed analysis detector is required.".to_string(),
            ));
        }
        let mut detectors = detectors.to_vec();
        detectors.sort();
        detectors.dedup();

        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (scope, scope_hash) = canonical_analysis_scope(&tx, requested_scope)?;
        let detector_set = detectors
            .iter()
            .map(|(id, version)| format!("{id}:v{version}"))
            .collect::<Vec<_>>();
        let detector_set_json = serde_json::to_string(&detector_set)?;
        let detector_set_hash = blake3::hash(detector_set_json.as_bytes())
            .to_hex()
            .to_string();
        let scope_json = serde_json::to_string(&scope)?;
        let captured_at = current_unix_seconds();
        let (source_snapshot_json, source_snapshot_hash) =
            analysis_source_snapshot(&tx, &scope, Some(captured_at))?;

        let existing = tx
            .query_row(
                &format!("{ANALYSIS_RUN_SELECT} WHERE request_key = ?1 ORDER BY request_attempt DESC LIMIT 1"),
                params![request_key],
                analysis_run_from_row,
            )
            .optional()?;
        if let Some(existing) = existing {
            if !force_retry
                && existing.scope_hash == scope_hash
                && existing.detector_set_hash == detector_set_hash
            {
                tx.commit()?;
                return Ok(AnalysisAdmission {
                    run: existing,
                    created: false,
                });
            }
        }

        let active = tx
            .query_row(
                &format!("{ANALYSIS_RUN_SELECT} WHERE scope_hash = ?1 AND detector_set_hash = ?2 AND status IN ('queued', 'running', 'cancelling') ORDER BY created_at DESC LIMIT 1"),
                params![scope_hash, detector_set_hash],
                analysis_run_from_row,
            )
            .optional()?;
        if let Some(active) = active {
            if !force_retry {
                tx.execute(
                    "UPDATE analysis_runs SET rerun_required = 1, revision = revision + 1, updated_at = ?1 WHERE id = ?2 AND rerun_required = 0",
                    params![current_unix_seconds(), active.id],
                )?;
                let run = query_analysis_run(&tx, &active.id)?;
                tx.commit()?;
                return Ok(AnalysisAdmission {
                    run,
                    created: false,
                });
            }
            return Err(DbError::Validation(
                "An analysis run for this scope and detector set is still active.".to_string(),
            ));
        }

        let request_attempt: i64 = tx.query_row(
            "SELECT COALESCE(MAX(request_attempt), 0) + 1 FROM analysis_runs WHERE request_key = ?1",
            params![request_key],
            |row| row.get(0),
        )?;
        let now = captured_at;
        let id = new_job_id("analysis-run");
        tx.execute(
            r#"
            INSERT INTO analysis_runs (
                id, request_key, request_attempt, scope_json, scope_hash,
                source_snapshot_json, source_snapshot_hash, detector_set_json,
                detector_set_hash, status, phase, revision, detectors_total,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'queued', 'preparing', 1, ?10, ?11, ?11)
            "#,
            params![
                id,
                request_key,
                request_attempt,
                scope_json,
                scope_hash,
                source_snapshot_json,
                source_snapshot_hash,
                detector_set_json,
                detector_set_hash,
                i64::try_from(detectors.len()).unwrap_or(i64::MAX),
                now,
            ],
        )?;
        for (detector_id, detector_version) in detectors {
            tx.execute(
                "INSERT INTO analysis_run_detectors (run_id, detector_id, detector_version, status) VALUES (?1, ?2, ?3, 'queued')",
                params![id, detector_id, detector_version],
            )?;
        }
        let run = query_analysis_run(&tx, &id)?;
        tx.commit()?;
        Ok(AnalysisAdmission { run, created: true })
    }

    pub(crate) fn get_analysis_run(&self, run_id: &str) -> Result<AnalysisRunDto, DbError> {
        let conn = self.conn()?;
        query_analysis_run(&conn, run_id)
    }

    pub(crate) fn list_analysis_runs(&self, limit: usize) -> Result<Vec<AnalysisRunDto>, DbError> {
        let conn = self.conn()?;
        let limit = i64::try_from(limit.clamp(1, 200)).unwrap_or(200);
        let mut statement = conn.prepare(&format!(
            "{ANALYSIS_RUN_SELECT} ORDER BY created_at DESC, id DESC LIMIT ?1"
        ))?;
        let result = statement
            .query_map(params![limit], analysis_run_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(DbError::from);
        result
    }

    pub(crate) fn get_active_analysis_run(&self) -> Result<Option<AnalysisRunDto>, DbError> {
        let conn = self.conn()?;
        conn.query_row(
            &format!("{ANALYSIS_RUN_SELECT} WHERE status IN ('queued', 'running', 'cancelling') ORDER BY created_at DESC, id DESC LIMIT 1"),
            [],
            analysis_run_from_row,
        )
        .optional()
        .map_err(DbError::from)
    }

    pub(crate) fn claim_analysis_run(
        &self,
        run_id: &str,
    ) -> Result<Option<AnalysisRunDto>, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = query_analysis_run(&tx, run_id)?;
        if run.status != "queued" {
            tx.commit()?;
            return Ok(None);
        }
        let now = current_unix_seconds();
        let changed = tx.execute(
            "UPDATE analysis_runs SET status = 'running', phase = 'preparing', started_at = COALESCE(started_at, ?1), last_checkpoint_at = ?1, revision = revision + 1, updated_at = ?1 WHERE id = ?2 AND status = 'queued' AND revision = ?3",
            params![now, run_id, run.revision],
        )?;
        if changed != 1 {
            tx.commit()?;
            return Ok(None);
        }
        let updated = query_analysis_run(&tx, run_id)?;
        tx.commit()?;
        Ok(Some(updated))
    }

    pub(crate) fn request_analysis_cancellation(
        &self,
        run_id: &str,
    ) -> Result<AnalysisRunDto, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = query_analysis_run(&tx, run_id)?;
        if is_analysis_terminal(&run.status) {
            tx.commit()?;
            return Ok(run);
        }
        let now = current_unix_seconds();
        let changed = tx.execute(
            "UPDATE analysis_runs SET status = 'cancelling', cancel_requested = 1, revision = revision + 1, updated_at = ?1 WHERE id = ?2 AND revision = ?3 AND status IN ('queued', 'running', 'cancelling')",
            params![now, run_id, run.revision],
        )?;
        if changed != 1 {
            return Err(DbError::Validation(
                "Analysis cancellation lost durable revision ownership.".to_string(),
            ));
        }
        tx.execute(
            "UPDATE analysis_run_detectors SET status = CASE WHEN status IN ('queued', 'running') THEN 'cancelled' ELSE status END, revision = revision + 1 WHERE run_id = ?1 AND status IN ('queued', 'running')",
            params![run_id],
        )?;
        let updated = query_analysis_run(&tx, run_id)?;
        tx.commit()?;
        Ok(updated)
    }

    pub(crate) fn is_analysis_cancel_requested(&self, run_id: &str) -> Result<bool, DbError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT cancel_requested FROM analysis_runs WHERE id = ?1",
            params![run_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value != 0)
        .map_err(DbError::from)
    }

    pub fn recover_analysis_runs(&self) -> Result<usize, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = current_unix_seconds();
        let changed = tx.execute(
            "UPDATE analysis_runs SET status = 'interrupted', phase = 'completed', error_code = 'interrupted_on_startup', error_message = 'The previous process stopped while analysis was active.', finished_at = ?1, revision = revision + 1, updated_at = ?1 WHERE status IN ('queued', 'running', 'cancelling')",
            params![now],
        )?;
        tx.execute(
            "UPDATE analysis_run_detectors SET status = 'interrupted', revision = revision + 1, finished_at = ?1 WHERE status IN ('queued', 'running')",
            params![now],
        )?;
        tx.commit()?;
        Ok(changed)
    }

    pub fn prune_analysis_artifacts(&self) -> Result<usize, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM analysis_runs WHERE status IN ('queued', 'running', 'cancelling')",
            [],
            |row| row.get(0),
        )?;
        if active_count > 0 {
            tx.commit()?;
            return Ok(0);
        }
        let finding_cutoff = current_unix_seconds().saturating_sub(30 * 24 * 60 * 60);
        let run_cutoff = current_unix_seconds().saturating_sub(90 * 24 * 60 * 60);
        let decision_cutoff = current_unix_seconds().saturating_sub(180 * 24 * 60 * 60);
        let mut remaining = ANALYSIS_PRUNE_ROW_BUDGET;
        let mut deleted = 0usize;

        // Delete children first and charge every physical row to one global
        // budget.  We deliberately delete one row at a time so a foreign-key
        // cascade can never turn a nominal 1000-row pass into an unbounded
        // writer lock.
        if remaining > 0 {
            let mut statement = tx.prepare(
                "SELECT e.id FROM analysis_finding_evidence AS e JOIN analysis_findings AS f ON f.id = e.finding_id WHERE f.status IN ('staged', 'stale', 'superseded', 'discarded') AND f.updated_at <= ?1 ORDER BY e.created_at, e.id LIMIT ?2",
            )?;
            let ids = statement
                .query_map(params![finding_cutoff, remaining as i64], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?;
            drop(statement);
            for id in ids {
                if tx.execute(
                    "DELETE FROM analysis_finding_evidence WHERE id = ?1",
                    params![id],
                )? == 1
                {
                    deleted += 1;
                    remaining -= 1;
                    if remaining == 0 {
                        break;
                    }
                }
            }
        }
        if remaining > 0 {
            let mut statement = tx.prepare(
                "SELECT f.id FROM analysis_findings AS f WHERE f.status IN ('staged', 'stale', 'superseded', 'discarded') AND f.updated_at <= ?1 AND NOT EXISTS (SELECT 1 FROM analysis_finding_evidence AS e WHERE e.finding_id = f.id) ORDER BY f.updated_at, f.id LIMIT ?2",
            )?;
            let ids = statement
                .query_map(params![finding_cutoff, remaining as i64], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?;
            drop(statement);
            for id in ids {
                if tx.execute(
                    "DELETE FROM analysis_findings WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM analysis_finding_evidence WHERE finding_id = ?1)",
                    params![id],
                )? == 1
                {
                    deleted += 1;
                    remaining -= 1;
                    if remaining == 0 {
                        break;
                    }
                }
            }
        }
        // Decisions are keyed by stable finding identity so they may survive
        // a successful rerun.  Only prune an orphan after all finding rows
        // have gone, and charge it against the same global budget.
        if remaining > 0 {
            let mut statement = tx.prepare(
                "SELECT d.finding_key FROM analysis_finding_decisions AS d WHERE d.updated_at <= ?1 AND NOT EXISTS (SELECT 1 FROM analysis_findings AS f WHERE f.finding_key = d.finding_key) ORDER BY d.updated_at, d.finding_key LIMIT ?2",
            )?;
            let keys = statement
                .query_map(params![decision_cutoff, remaining as i64], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?;
            drop(statement);
            for key in keys {
                if tx.execute(
                    "DELETE FROM analysis_finding_decisions WHERE finding_key = ?1",
                    params![key],
                )? == 1
                {
                    deleted += 1;
                    remaining -= 1;
                    if remaining == 0 {
                        break;
                    }
                }
            }
        }
        // A run is deleted only after its detector children are explicitly
        // removed.  This makes the final run DELETE cascade-free.
        if remaining > 0 {
            let mut statement = tx.prepare(
                "SELECT d.run_id, d.detector_id FROM analysis_run_detectors AS d JOIN analysis_runs AS r ON r.id = d.run_id WHERE r.status IN ('completed', 'completed_with_warnings', 'cancelled', 'failed', 'interrupted') AND COALESCE(r.finished_at, r.updated_at) <= ?1 AND NOT EXISTS (SELECT 1 FROM analysis_findings AS f WHERE f.run_id = r.id) ORDER BY COALESCE(r.finished_at, r.updated_at), d.run_id, d.detector_id LIMIT ?2",
            )?;
            let detector_ids = statement
                .query_map(params![run_cutoff, remaining as i64], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            drop(statement);
            for (run_id, detector_id) in detector_ids {
                if tx.execute(
                    "DELETE FROM analysis_run_detectors WHERE run_id = ?1 AND detector_id = ?2 AND NOT EXISTS (SELECT 1 FROM analysis_findings WHERE run_id = ?1)",
                    params![run_id, detector_id],
                )? == 1
                {
                    deleted += 1;
                    remaining -= 1;
                    if remaining == 0 {
                        break;
                    }
                }
            }
        }
        if remaining > 0 {
            let mut statement = tx.prepare(
                "SELECT r.id FROM analysis_runs AS r WHERE r.status IN ('completed', 'completed_with_warnings', 'cancelled', 'failed', 'interrupted') AND COALESCE(r.finished_at, r.updated_at) <= ?1 AND NOT EXISTS (SELECT 1 FROM analysis_findings WHERE run_id = r.id) AND NOT EXISTS (SELECT 1 FROM analysis_run_detectors WHERE run_id = r.id) ORDER BY COALESCE(r.finished_at, r.updated_at), r.id LIMIT ?2",
            )?;
            let run_ids = statement
                .query_map(params![run_cutoff, remaining as i64], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?;
            drop(statement);
            for run_id in run_ids {
                if tx.execute(
                    "DELETE FROM analysis_runs WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM analysis_findings WHERE run_id = ?1) AND NOT EXISTS (SELECT 1 FROM analysis_run_detectors WHERE run_id = ?1)",
                    params![run_id],
                )? == 1
                {
                    deleted += 1;
                    remaining -= 1;
                    if remaining == 0 {
                        break;
                    }
                }
            }
        }
        tx.commit()?;
        Ok(deleted)
    }

    pub(crate) fn checkpoint_analysis_run(
        &self,
        run_id: &str,
        expected_revision: i64,
        phase: &str,
        warning_count: i64,
        error_count: i64,
    ) -> Result<AnalysisRunDto, DbError> {
        if !matches!(
            phase,
            "preparing" | "running_detectors" | "finalizing" | "completed"
        ) {
            return Err(DbError::Validation("Invalid analysis phase.".to_string()));
        }
        let conn = self.conn()?;
        let now = current_unix_seconds();
        let changed = conn.execute(
            "UPDATE analysis_runs SET phase = ?1, warning_count = ?2, error_count = ?3, last_checkpoint_at = ?4, revision = revision + 1, updated_at = ?4 WHERE id = ?5 AND revision = ?6 AND status IN ('queued', 'running', 'cancelling')",
            params![phase, warning_count, error_count, now, run_id, expected_revision],
        )?;
        if changed != 1 {
            return Err(DbError::Validation(
                "Analysis checkpoint rejected because the durable revision is stale.".to_string(),
            ));
        }
        query_analysis_run(&conn, run_id)
    }

    pub(crate) fn list_analysis_run_detectors(
        &self,
        run_id: &str,
    ) -> Result<Vec<AnalysisDetectorDto>, DbError> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(&format!(
            "{ANALYSIS_DETECTOR_SELECT} WHERE run_id = ?1 ORDER BY detector_id"
        ))?;
        let result = statement
            .query_map(params![run_id], analysis_detector_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(DbError::from);
        result
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_analysis_detector_status(
        &self,
        run_id: &str,
        detector_id: &str,
        expected_revision: i64,
        status: &str,
        scanned_subjects: i64,
        findings_staged: i64,
        exact_reclaimable_bytes: i64,
        potential_reclaimable_bytes: i64,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<AnalysisDetectorDto, DbError> {
        if !matches!(
            status,
            "queued"
                | "running"
                | "completed"
                | "completed_with_warnings"
                | "skipped"
                | "cancelled"
                | "failed"
                | "interrupted"
        ) {
            return Err(DbError::Validation(
                "Invalid analysis detector status.".to_string(),
            ));
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (run_revision, run_status, run_cancel_requested): (i64, String, i64) = tx.query_row(
            "SELECT revision, status, cancel_requested FROM analysis_runs WHERE id = ?1",
            params![run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if !matches!(run_status.as_str(), "queued" | "running" | "cancelling")
            || (run_cancel_requested != 0 && status == "running")
        {
            return Err(DbError::Validation(
                "Analysis detector transition was rejected by the durable run state.".to_string(),
            ));
        }
        let now = current_unix_seconds();
        let changed = tx.execute(
            "UPDATE analysis_run_detectors SET status = ?1, scanned_subjects = ?2, findings_staged = ?3, exact_reclaimable_bytes = ?4, potential_reclaimable_bytes = ?5, error_code = ?6, error_message = ?7, started_at = CASE WHEN ?1 = 'running' THEN COALESCE(started_at, ?8) ELSE started_at END, finished_at = CASE WHEN ?1 IN ('completed', 'completed_with_warnings', 'skipped', 'cancelled', 'failed', 'interrupted') THEN ?8 ELSE finished_at END, revision = revision + 1 WHERE run_id = ?9 AND detector_id = ?10 AND revision = ?11",
            params![status, scanned_subjects, findings_staged, exact_reclaimable_bytes, potential_reclaimable_bytes, error_code, error_message, now, run_id, detector_id, expected_revision],
        )?;
        if changed != 1 {
            return Err(DbError::Validation(
                "Analysis detector transition lost durable revision ownership.".to_string(),
            ));
        }
        let detector = tx.query_row(
            &format!("{ANALYSIS_DETECTOR_SELECT} WHERE run_id = ?1 AND detector_id = ?2"),
            params![run_id, detector_id],
            analysis_detector_from_row,
        )?;
        tx.execute(
            "UPDATE analysis_runs SET detectors_completed = (SELECT COUNT(*) FROM analysis_run_detectors WHERE run_id = ?1 AND status IN ('completed', 'completed_with_warnings', 'skipped')), detectors_failed = (SELECT COUNT(*) FROM analysis_run_detectors WHERE run_id = ?1 AND status IN ('failed', 'interrupted')), revision = revision + 1, updated_at = ?2 WHERE id = ?1 AND revision = ?3 AND status IN ('queued', 'running', 'cancelling')",
            params![run_id, now, run_revision],
        )?;
        if tx.changes() != 1 {
            return Err(DbError::Validation(
                "Analysis run aggregate transition lost durable revision ownership.".to_string(),
            ));
        }
        tx.commit()?;
        Ok(detector)
    }

    pub(crate) fn stage_analysis_findings(
        &self,
        run_id: &str,
        scope_hash: &str,
        drafts: &[FindingDraft],
    ) -> Result<usize, DbError> {
        if drafts.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (run_status, run_cancel_requested, run_revision): (String, i64, i64) = tx.query_row(
            "SELECT status, cancel_requested, revision FROM analysis_runs WHERE id = ?1",
            params![run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if run_status != "queued" && run_status != "running" || run_cancel_requested != 0 {
            return Err(DbError::Validation(
                "Findings can only be staged for an active analysis run.".to_string(),
            ));
        }
        let now = current_unix_seconds();
        for draft in drafts {
            validate_finding_draft(draft)?;
            // A stable finding_key carries decisions across reruns, while the
            // row identity must remain run-scoped so an unpublished rerun can
            // never overwrite the currently active finding row.
            let finding_id: String = tx
                .query_row(
                    "SELECT id FROM analysis_findings WHERE run_id = ?1 AND finding_key = ?2",
                    params![run_id, draft.finding_key],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or_else(|| {
                    if tx
                        .query_row(
                            "SELECT 1 FROM analysis_findings WHERE id = ?1",
                            params![draft.id],
                            |row| row.get::<_, i64>(0),
                        )
                        .optional()
                        .ok()
                        .flatten()
                        .is_some()
                    {
                        deterministic_finding_id(run_id, &draft.finding_key)
                    } else {
                        draft.id.clone()
                    }
                });
            tx.execute(
                r#"
                INSERT INTO analysis_findings (
                    id, finding_key, run_id, detector_id, detector_version, scope_hash,
                    status, tier, category, action_kind, title, reason, risk_note,
                    confidence, size_bytes, exact_reclaimable_bytes, potential_reclaimable_bytes,
                    requires_confirmation, executable, primary_subject_kind, primary_subject_id,
                    path_snapshot, identity_snapshot_json, evidence_summary_json,
                    revision, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'staged', ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, 1, ?24, ?24)
                ON CONFLICT(id) DO UPDATE SET
                    run_id = excluded.run_id,
                    detector_id = excluded.detector_id,
                    detector_version = excluded.detector_version,
                    scope_hash = excluded.scope_hash,
                    status = 'staged', tier = excluded.tier, category = excluded.category,
                    action_kind = excluded.action_kind, title = excluded.title,
                    reason = excluded.reason, risk_note = excluded.risk_note,
                    confidence = excluded.confidence, size_bytes = excluded.size_bytes,
                    exact_reclaimable_bytes = excluded.exact_reclaimable_bytes,
                    potential_reclaimable_bytes = excluded.potential_reclaimable_bytes,
                    requires_confirmation = excluded.requires_confirmation,
                    executable = excluded.executable,
                    primary_subject_kind = excluded.primary_subject_kind,
                    primary_subject_id = excluded.primary_subject_id,
                    path_snapshot = excluded.path_snapshot,
                    identity_snapshot_json = excluded.identity_snapshot_json,
                    evidence_summary_json = excluded.evidence_summary_json,
                    revision = analysis_findings.revision + 1,
                    updated_at = excluded.updated_at,
                    stale_at = NULL
                "#,
                params![
                    finding_id,
                    draft.finding_key,
                    run_id,
                    draft.detector_id,
                    draft.detector_version,
                    scope_hash,
                    draft.tier,
                    draft.category,
                    draft.action_kind,
                    draft.title,
                    draft.reason,
                    draft.risk_note,
                    draft.confidence,
                    draft.size_bytes,
                    draft.exact_reclaimable_bytes,
                    draft.potential_reclaimable_bytes,
                    bool_to_i64(draft.requires_confirmation),
                    bool_to_i64(draft.executable),
                    draft.primary_subject_kind,
                    draft.primary_subject_id,
                    draft.path_snapshot,
                    serde_json::to_string(&draft.identity_snapshot)?,
                    serde_json::to_string(&draft.evidence_summary)?,
                    now,
                ],
            )?;
            tx.execute(
                "DELETE FROM analysis_finding_evidence WHERE finding_id = ?1",
                params![finding_id],
            )?;
            for evidence in &draft.evidence {
                tx.execute(
                    "INSERT INTO analysis_finding_evidence (id, finding_id, evidence_kind, subject_kind, subject_id, path_snapshot, value_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![new_job_id("analysis-evidence"), finding_id, evidence.evidence_kind, evidence.subject_kind, evidence.subject_id, evidence.path_snapshot, serde_json::to_string(&evidence.value)?, now],
                )?;
            }
        }
        tx.execute(
            "UPDATE analysis_runs SET findings_staged = (SELECT COUNT(*) FROM analysis_findings WHERE run_id = ?1 AND status = 'staged'), revision = revision + 1, updated_at = ?2 WHERE id = ?1 AND revision = ?3 AND status IN ('queued', 'running') AND cancel_requested = 0",
            params![run_id, now, run_revision],
        )?;
        if tx.changes() != 1 {
            return Err(DbError::Validation(
                "Analysis staging lost durable run revision ownership.".to_string(),
            ));
        }
        tx.commit()?;
        Ok(drafts.len())
    }

    pub(crate) fn publish_analysis_run(
        &self,
        run_id: &str,
    ) -> Result<AnalysisPublishOutcome, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = query_analysis_run(&tx, run_id)?;
        if run.cancel_requested || run.status == "cancelling" {
            finish_analysis_run_tx(
                &tx,
                &run,
                "cancelled",
                false,
                Some("cancelled"),
                Some("Analysis run was cancelled before publication."),
            )?;
            tx.commit()?;
            return Ok(AnalysisPublishOutcome::Cancelled);
        }
        if !matches!(run.status.as_str(), "running" | "queued") {
            return Ok(if run.status == "completed" {
                AnalysisPublishOutcome::Completed
            } else {
                AnalysisPublishOutcome::CompletedWithWarnings
            });
        }
        let (_snapshot_json, snapshot_hash) = analysis_source_snapshot(&tx, &run.scope, None)?;
        if snapshot_hash != run.source_snapshot_hash {
            finish_analysis_run_tx(
                &tx,
                &run,
                "completed_with_warnings",
                true,
                Some("source_changed_during_run"),
                Some("The analysis source changed during detector execution; staged findings were retained as diagnostics and not published."),
            )?;
            tx.commit()?;
            return Ok(AnalysisPublishOutcome::CompletedWithWarnings);
        }

        let detector_rows = load_analysis_detectors(&tx, run_id)?;
        let mut has_warning = run.warning_count > 0;
        let mut has_failure = false;
        let now = current_unix_seconds();
        for detector in detector_rows {
            match detector.status.as_str() {
                "completed" | "completed_with_warnings" => {
                    has_warning |= detector.status == "completed_with_warnings";
                    tx.execute(
                        "UPDATE analysis_findings SET status = 'superseded', revision = revision + 1, updated_at = ?1 WHERE status = 'active' AND scope_hash = ?2 AND detector_id = ?3 AND detector_version = ?4",
                        params![now, run.scope_hash, detector.detector_id, detector.detector_version],
                    )?;
                    tx.execute(
                        "UPDATE analysis_findings SET status = 'active', published_at = ?1, revision = revision + 1, updated_at = ?1 WHERE run_id = ?2 AND detector_id = ?3 AND status = 'staged'",
                        params![now, run_id, detector.detector_id],
                    )?;
                    let published: i64 = tx.query_row(
                        "SELECT COUNT(*) FROM analysis_findings WHERE run_id = ?1 AND detector_id = ?2 AND status = 'active'",
                        params![run_id, detector.detector_id],
                        |row| row.get(0),
                    )?;
                    tx.execute(
                        "UPDATE analysis_run_detectors SET findings_published = ?1, revision = revision + 1 WHERE run_id = ?2 AND detector_id = ?3",
                        params![published, run_id, detector.detector_id],
                    )?;
                }
                "skipped" => {
                    has_warning = true;
                    tx.execute(
                        "UPDATE analysis_findings SET status = 'discarded', revision = revision + 1, updated_at = ?1 WHERE run_id = ?2 AND detector_id = ?3 AND status = 'staged'",
                        params![now, run_id, detector.detector_id],
                    )?;
                }
                "failed" | "interrupted" | "cancelled" => {
                    has_failure = true;
                    has_warning = true;
                    tx.execute(
                        "UPDATE analysis_findings SET status = 'discarded', revision = revision + 1, updated_at = ?1 WHERE run_id = ?2 AND detector_id = ?3 AND status = 'staged'",
                        params![now, run_id, detector.detector_id],
                    )?;
                }
                _ => {
                    has_failure = true;
                    has_warning = true;
                }
            }
        }
        let status = if has_warning || has_failure {
            "completed_with_warnings"
        } else {
            "completed"
        };
        let latest_run = query_analysis_run(&tx, run_id)?;
        finish_analysis_run_tx(
            &tx,
            &latest_run,
            status,
            latest_run.rerun_required,
            None,
            None,
        )?;
        tx.commit()?;
        Ok(if status == "completed" {
            AnalysisPublishOutcome::Completed
        } else {
            AnalysisPublishOutcome::CompletedWithWarnings
        })
    }

    pub(crate) fn list_analysis_findings(
        &self,
        filter: &AnalysisFindingFilter,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<AnalysisFindingPageDto, DbError> {
        let pooled = self.conn()?;
        let conn = pooled.unchecked_transaction()?;
        let limit = i64::try_from(limit.clamp(1, 200)).unwrap_or(200);
        let parsed_cursor = cursor.map(parse_finding_cursor).transpose()?;
        let mut clauses = vec!["1 = 1".to_string()];
        let mut values = Vec::<rusqlite::types::Value>::new();
        if let Some(run_id) = &filter.run_id {
            clauses.push("f.run_id = ?".to_string());
            values.push(run_id.clone().into());
        }
        if let Some(detector_id) = &filter.detector_id {
            clauses.push("f.detector_id = ?".to_string());
            values.push(detector_id.clone().into());
        }
        if let Some(tier) = &filter.tier {
            clauses.push("f.tier = ?".to_string());
            values.push(tier.clone().into());
        }
        if let Some(category) = &filter.category {
            clauses.push("f.category = ?".to_string());
            values.push(category.clone().into());
        }
        if let Some(status) = &filter.status {
            clauses.push("f.status = ?".to_string());
            values.push(status.clone().into());
        }
        if let Some(decision) = &filter.decision {
            clauses.push("COALESCE(CASE WHEN d.decision = 'snoozed' AND d.snoozed_until <= unixepoch() THEN 'open' ELSE d.decision END, 'open') = ?".to_string());
            values.push(decision.clone().into());
        }
        if filter.executable_only {
            clauses.push("f.executable = 1".to_string());
        }
        if let Some(cursor) = &parsed_cursor {
            values.extend([
                cursor.tier_order.into(),
                cursor.potential_reclaimable_bytes.into(),
                cursor.updated_at.into(),
                cursor.id.clone().into(),
            ]);
            clauses.push("(CASE f.tier WHEN 'safe' THEN 0 WHEN 'review' THEN 1 ELSE 2 END > ? OR (CASE f.tier WHEN 'safe' THEN 0 WHEN 'review' THEN 1 ELSE 2 END = ? AND (f.potential_reclaimable_bytes < ? OR (f.potential_reclaimable_bytes = ? AND (f.updated_at < ? OR (f.updated_at = ? AND f.id > ?))))))".to_string());
            // The expanded predicate uses duplicated placeholders; append them in order below.
            values.pop();
            values.pop();
            values.pop();
            values.pop();
            values.extend([
                cursor.tier_order.into(),
                cursor.tier_order.into(),
                cursor.potential_reclaimable_bytes.into(),
                cursor.potential_reclaimable_bytes.into(),
                cursor.updated_at.into(),
                cursor.updated_at.into(),
                cursor.id.clone().into(),
            ]);
        }
        let sql = format!(
            "{ANALYSIS_FINDING_SELECT} LEFT JOIN analysis_finding_decisions AS d ON d.finding_key = f.finding_key WHERE {} ORDER BY CASE f.tier WHEN 'safe' THEN 0 WHEN 'review' THEN 1 ELSE 2 END ASC, f.potential_reclaimable_bytes DESC, f.updated_at DESC, f.id ASC LIMIT ?",
            clauses.join(" AND ")
        );
        values.push((limit + 1).into());
        let mut statement = conn.prepare(&sql)?;
        let mut findings = statement
            .query_map(params_from_iter(values.iter()), analysis_finding_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = findings.len() > usize::try_from(limit).unwrap_or(200);
        if has_more {
            findings.truncate(usize::try_from(limit).unwrap_or(200));
        }
        let next_cursor = findings
            .last()
            .and_then(|finding| {
                has_more.then(|| {
                    serde_json::to_string(&FindingCursor {
                        version: 1,
                        tier_order: tier_order(&finding.tier),
                        potential_reclaimable_bytes: finding.potential_reclaimable_bytes,
                        updated_at: finding.updated_at,
                        id: finding.id.clone(),
                    })
                    .ok()
                })
            })
            .flatten();
        let context = filter
            .run_id
            .as_ref()
            .map(|id| {
                conn.query_row(
                    "SELECT revision,scope_json FROM analysis_runs WHERE id=?1",
                    [id],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                )
                .optional()
            })
            .transpose()?
            .flatten();
        let run_revision = context.as_ref().map(|(revision, _)| *revision);
        let is_cleanup = context.as_ref().is_some_and(|(_, scope)| {
            serde_json::from_str::<Value>(scope)
                .ok()
                .is_some_and(|scope| scope["kind"] == "approved_cleanup_paths")
        });
        let groups = if cursor.is_none() && is_cleanup {
            filter
                .run_id
                .as_ref()
                .map(|id| grouping::summarize(&conn, id))
                .transpose()?
        } else {
            None
        };
        Ok(AnalysisFindingPageDto {
            run_revision,
            groups,
            findings,
            next_cursor,
            limit,
        })
    }

    /// Offset pagination is kept only for the compatibility Storage Cleanup
    /// projection.  New analysis consumers use the cursor API above so a
    /// renderer cannot turn a large finding history into an unbounded query.
    pub(crate) fn list_analysis_findings_offset(
        &self,
        filter: &AnalysisFindingFilter,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<AnalysisFindingDto>, DbError> {
        let conn = self.conn()?;
        let limit = i64::try_from(limit.clamp(1, 500)).unwrap_or(500);
        let offset = i64::try_from(offset.min(1_000_000)).unwrap_or(1_000_000);
        let mut clauses = vec!["1 = 1".to_string()];
        let mut values = Vec::<rusqlite::types::Value>::new();
        if let Some(run_id) = &filter.run_id {
            clauses.push("f.run_id = ?".to_string());
            values.push(run_id.clone().into());
        }
        if let Some(detector_id) = &filter.detector_id {
            clauses.push("f.detector_id = ?".to_string());
            values.push(detector_id.clone().into());
        }
        if let Some(tier) = &filter.tier {
            clauses.push("f.tier = ?".to_string());
            values.push(tier.clone().into());
        }
        if let Some(category) = &filter.category {
            clauses.push("f.category = ?".to_string());
            values.push(category.clone().into());
        }
        if let Some(status) = &filter.status {
            clauses.push("f.status = ?".to_string());
            values.push(status.clone().into());
        }
        if let Some(decision) = &filter.decision {
            clauses.push("COALESCE(CASE WHEN d.decision = 'snoozed' AND d.snoozed_until <= unixepoch() THEN 'open' ELSE d.decision END, 'open') = ?".to_string());
            values.push(decision.clone().into());
        }
        if filter.executable_only {
            clauses.push("f.executable = 1".to_string());
        }
        let sql = format!(
            "{ANALYSIS_FINDING_SELECT} LEFT JOIN analysis_finding_decisions AS d ON d.finding_key = f.finding_key WHERE {} ORDER BY CASE f.tier WHEN 'safe' THEN 0 WHEN 'review' THEN 1 ELSE 2 END ASC, f.potential_reclaimable_bytes DESC, f.updated_at DESC, f.id ASC LIMIT ? OFFSET ?",
            clauses.join(" AND ")
        );
        values.push(limit.into());
        values.push(offset.into());
        let result = conn
            .prepare(&sql)?
            .query_map(params_from_iter(values.iter()), analysis_finding_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(DbError::from);
        result
    }

    pub(crate) fn count_analysis_findings_for_run(
        &self,
        run_id: &str,
        status: &str,
    ) -> Result<i64, DbError> {
        if !matches!(
            status,
            "staged" | "active" | "stale" | "superseded" | "discarded"
        ) {
            return Err(DbError::Validation("Invalid finding status.".to_string()));
        }
        let conn = self.conn()?;
        conn.query_row(
            "SELECT COUNT(*) FROM analysis_findings WHERE run_id = ?1 AND status = ?2",
            params![run_id, status],
            |row| row.get(0),
        )
        .map_err(DbError::from)
    }

    pub(crate) fn sum_analysis_finding_size_for_run(
        &self,
        run_id: &str,
        tier: &str,
    ) -> Result<i64, DbError> {
        if !matches!(tier, "safe" | "review" | "caution") {
            return Err(DbError::Validation("Invalid finding tier.".to_string()));
        }
        let conn = self.conn()?;
        conn.query_row(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM analysis_findings WHERE run_id = ?1 AND status = 'active' AND tier = ?2",
            params![run_id, tier],
            |row| row.get(0),
        )
        .map_err(DbError::from)
    }

    pub(crate) fn list_managed_files_for_analysis(
        &self,
        root_ids: &[String],
        minimum_size: i64,
    ) -> Result<Vec<ManagedAnalysisFile>, DbError> {
        if root_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn()?;
        let placeholders = vec!["?"; root_ids.len()].join(",");
        let sql = format!(
            "SELECT f.id, f.path, f.size, f.mtime, f.is_stale, fp.identity_status, fp.platform_kind, fp.platform_volume_id, fp.platform_file_id, fp.physical_key, fp.size, fp.modified_ns, fp.full_hash, fp.fingerprint_status, fp.revision FROM files AS f LEFT JOIN file_fingerprints AS fp ON fp.file_id = f.id WHERE f.is_dir = 0 AND f.is_stale = 0 AND f.size >= ? AND EXISTS (SELECT 1 FROM scan_roots AS r WHERE r.id IN ({placeholders}) AND r.enabled = 1 AND r.source_kind = 'file_library' AND (f.path = r.normalized_path OR f.path LIKE r.normalized_path || '/%' OR f.path LIKE r.normalized_path || '\\%')) ORDER BY f.size DESC, f.path COLLATE NOCASE ASC"
        );
        let mut values = Vec::<rusqlite::types::Value>::with_capacity(root_ids.len() + 1);
        values.push(minimum_size.into());
        values.extend(root_ids.iter().cloned().map(Into::into));
        let mut statement = conn.prepare(&sql)?;
        let result = statement
            .query_map(
                params_from_iter(values.iter()),
                managed_analysis_file_from_row,
            )?
            .collect::<Result<Vec<_>, _>>();
        result.map_err(DbError::from)
    }

    pub(crate) fn get_managed_file_for_analysis(
        &self,
        file_id: &str,
    ) -> Result<Option<ManagedAnalysisFile>, DbError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT f.id, f.path, f.size, f.mtime, f.is_stale, fp.identity_status, fp.platform_kind, fp.platform_volume_id, fp.platform_file_id, fp.physical_key, fp.size, fp.modified_ns, fp.full_hash, fp.fingerprint_status, fp.revision FROM files AS f LEFT JOIN file_fingerprints AS fp ON fp.file_id = f.id WHERE f.id = ?1",
            params![file_id],
            managed_analysis_file_from_row,
        )
        .optional()
        .map_err(DbError::from)
    }

    pub(crate) fn get_analysis_finding(
        &self,
        finding_id: &str,
    ) -> Result<Option<AnalysisFindingDto>, DbError> {
        let conn = self.conn()?;
        conn.query_row(
            &format!("{ANALYSIS_FINDING_SELECT} LEFT JOIN analysis_finding_decisions AS d ON d.finding_key = f.finding_key WHERE f.id = ?1"),
            params![finding_id],
            analysis_finding_from_row,
        )
        .optional()
        .map_err(DbError::from)
    }

    pub(crate) fn list_analysis_finding_evidence(
        &self,
        finding_id: &str,
    ) -> Result<Vec<AnalysisFindingEvidenceDto>, DbError> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT id, finding_id, evidence_kind, subject_kind, subject_id, path_snapshot, value_json, created_at FROM analysis_finding_evidence WHERE finding_id = ?1 ORDER BY created_at, id LIMIT ?2",
        )?;
        let result = statement
            .query_map(
                params![finding_id, ANALYSIS_FINDING_DETAIL_LIMIT as i64],
                analysis_evidence_from_row,
            )?
            .collect::<Result<Vec<_>, _>>()
            .map_err(DbError::from);
        result
    }

    pub(crate) fn set_analysis_finding_decision(
        &self,
        finding_key: &str,
        decision: &str,
        snoozed_until: Option<i64>,
        note: Option<&str>,
        expected_revision: i64,
    ) -> Result<AnalysisFindingDecisionDto, DbError> {
        if !matches!(decision, "open" | "acknowledged" | "dismissed" | "snoozed") {
            return Err(DbError::Validation("Invalid finding decision.".to_string()));
        }
        if decision == "snoozed" && snoozed_until.is_none() {
            return Err(DbError::Validation(
                "Snoozed findings require an expiry timestamp.".to_string(),
            ));
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = current_unix_seconds();
        let finding_exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM analysis_findings WHERE finding_key = ?1)",
            params![finding_key],
            |row| row.get(0),
        )?;
        if !finding_exists {
            return Err(DbError::Validation(
                "Finding decision requires a durable finding key.".to_string(),
            ));
        }
        let existing = tx
            .query_row(
                "SELECT revision FROM analysis_finding_decisions WHERE finding_key = ?1",
                params![finding_key],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        match existing {
            Some(revision) if revision != expected_revision => {
                return Err(DbError::Validation(
                    "Finding decision revision is stale.".to_string(),
                ))
            }
            None if expected_revision != 0 => {
                return Err(DbError::Validation(
                    "Finding decision revision is stale.".to_string(),
                ))
            }
            _ => {}
        }
        tx.execute(
            "INSERT INTO analysis_finding_decisions (finding_key, decision, snoozed_until, note, revision, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5) ON CONFLICT(finding_key) DO UPDATE SET decision = excluded.decision, snoozed_until = excluded.snoozed_until, note = excluded.note, revision = analysis_finding_decisions.revision + 1, updated_at = excluded.updated_at",
            params![finding_key, decision, snoozed_until, note, now],
        )?;
        let result = tx.query_row(
            "SELECT finding_key, decision, snoozed_until, note, revision, created_at, updated_at FROM analysis_finding_decisions WHERE finding_key = ?1",
            params![finding_key],
            analysis_decision_from_row,
        )?;
        tx.commit()?;
        Ok(result)
    }

    #[allow(dead_code)]
    pub(crate) fn get_analysis_finding_decision(
        &self,
        finding_key: &str,
    ) -> Result<Option<AnalysisFindingDecisionDto>, DbError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT finding_key, decision, snoozed_until, note, revision, created_at, updated_at FROM analysis_finding_decisions WHERE finding_key = ?1",
            params![finding_key],
            analysis_decision_from_row,
        )
        .optional()
        .map_err(DbError::from)
    }

    pub(crate) fn mark_analysis_finding_stale(&self, finding_id: &str) -> Result<(), DbError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE analysis_findings SET status = 'stale', stale_at = ?1, revision = revision + 1, updated_at = ?1 WHERE id = ?2 AND status = 'active'",
            params![current_unix_seconds(), finding_id],
        )?;
        if changed == 0 {
            return Err(DbError::Validation(
                "Analysis finding is no longer active.".to_string(),
            ));
        }
        Ok(())
    }

    /// Atomically publish only exact, uniquely covered AI assessments against
    /// the run, detector and finding revisions captured before provider work.
    /// AI may raise risk or disable an executable action, but cannot lower risk
    /// or grant filesystem authority.
    pub(crate) fn publish_analysis_ai_assessments_cas(
        &self,
        batch: &AnalysisAiPublicationBatch,
    ) -> Result<Vec<AnalysisFindingDto>, DbError> {
        if batch.preconditions.is_empty() || batch.assessments.is_empty() {
            return Ok(Vec::new());
        }
        if batch.assessments.iter().any(|assessment| {
            !matches!(
                assessment.requested_tier.as_str(),
                "safe" | "review" | "caution"
            )
        }) {
            return Err(DbError::Validation("Invalid AI finding tier.".to_string()));
        }
        let mut candidate_ids = batch
            .preconditions
            .iter()
            .map(|precondition| precondition.finding_id.clone())
            .collect::<Vec<_>>();
        candidate_ids.sort();
        candidate_ids.dedup();
        let candidate_set_json = serde_json::to_string(&candidate_ids)?;
        let candidate_set_fingerprint = blake3::hash(candidate_set_json.as_bytes())
            .to_hex()
            .to_string();
        if candidate_ids.len() != batch.preconditions.len()
            || candidate_set_fingerprint != batch.expected_candidate_set_fingerprint
        {
            return Err(DbError::Validation(
                "Cleanup AI candidate set changed; assessment was not published.".to_string(),
            ));
        }
        let mut assessment_ids = HashSet::new();
        if batch.assessments.iter().any(|assessment| {
            !candidate_ids.contains(&assessment.finding_id)
                || !assessment_ids.insert(assessment.finding_id.as_str())
        }) {
            return Err(DbError::Validation(
                "Cleanup AI response contains invalid candidate coverage.".to_string(),
            ));
        }
        for assessment in &batch.assessments {
            let Some(precondition) = batch
                .preconditions
                .iter()
                .find(|precondition| precondition.finding_id == assessment.finding_id)
            else {
                return Err(DbError::Validation(
                    "Cleanup AI response is not bound to its request manifest.".to_string(),
                ));
            };
            if !analysis_ai_coverage_matches(batch, precondition, assessment)? {
                return Err(DbError::Validation(
                    "Cleanup AI response is not exact current candidate coverage.".to_string(),
                ));
            }
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let run = query_analysis_run(&tx, &batch.run_id)?;
        if !matches!(run.status.as_str(), "completed" | "completed_with_warnings")
            || run.revision != batch.expected_run_revision
            || run.source_snapshot_hash != batch.expected_source_snapshot_hash
            || run.detector_set_hash != batch.expected_detector_set_hash
        {
            return Err(DbError::Validation(
                "Cleanup AI Analysis Run changed; assessment was not published.".to_string(),
            ));
        }

        let settings_value = tx
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                params![batch.settings_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if crate::db::app_setting_value_fingerprint(settings_value.as_deref())
            != batch.expected_settings_revision
        {
            return Err(DbError::Validation(
                "AI provider policy changed; cleanup assessment was not published.".to_string(),
            ));
        }

        let mut findings_by_id = HashMap::with_capacity(batch.preconditions.len());
        for precondition in &batch.preconditions {
            let detector_matches: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM analysis_run_detectors WHERE run_id = ?1 AND detector_id = ?2 AND detector_version = ?3 AND status = ?4 AND revision = ?5)",
                params![precondition.run_id, precondition.detector_id, precondition.detector_version, precondition.detector_status, precondition.detector_revision],
                |row| row.get(0),
            )?;
            let finding = tx
                .query_row(
                    &format!("{ANALYSIS_FINDING_SELECT} LEFT JOIN analysis_finding_decisions AS d ON d.finding_key = f.finding_key WHERE f.id = ?1"),
                    params![precondition.finding_id],
                    analysis_finding_from_row,
                )
                .optional()?
                .ok_or_else(|| DbError::Validation("Cleanup AI finding disappeared; assessment was not published.".to_string()))?;
            let identity_snapshot_json = serde_json::to_string(&finding.identity_snapshot)?;
            if !detector_matches
                || finding.status != "active"
                || finding.id != precondition.finding_id
                || finding.finding_key != precondition.finding_key
                || finding.run_id != precondition.run_id
                || finding.run_id != batch.run_id
                || finding.detector_id != precondition.detector_id
                || finding.detector_version != precondition.detector_version
                || finding.scope_hash != precondition.scope_hash
                || finding.revision != precondition.expected_revision
                || finding.primary_subject_kind != precondition.primary_subject_kind
                || finding.primary_subject_id != precondition.primary_subject_id
                || finding.path_snapshot != precondition.path_snapshot
                || identity_snapshot_json != precondition.identity_snapshot_json
            {
                return Err(DbError::Validation(
                    "Cleanup AI finding revision or identity is stale; assessment was not published.".to_string(),
                ));
            }
            findings_by_id.insert(precondition.finding_id.as_str(), finding);
        }

        let now = current_unix_seconds();
        let mut published_ids = Vec::with_capacity(batch.assessments.len());
        for publication in &batch.assessments {
            let finding = findings_by_id
                .get(publication.finding_id.as_str())
                .expect("assessment candidate was validated in the request manifest");
            let tier = higher_risk_tier(&finding.tier, &publication.requested_tier);
            let executable =
                finding.executable && publication.requested_trash_allowed && tier == "safe";
            let action_kind = if executable {
                finding.action_kind.clone()
            } else if finding.action_kind == "safe_trash_candidate" {
                "reveal".to_string()
            } else {
                finding.action_kind.clone()
            };
            let published_revision = finding.revision + 1;
            let mut assessment = publication.evidence.clone();
            if !assessment.is_object() {
                return Err(DbError::Validation(
                    "Cleanup AI assessment evidence is invalid.".to_string(),
                ));
            }
            if let Some(object) = assessment.as_object_mut() {
                object.insert(
                    "publication".to_string(),
                    json!({
                        "outcome": "published",
                        "compareAndSwap": "succeeded",
                        "findingId": finding.id,
                        "expectedRevision": finding.revision,
                        "publishedRevision": published_revision,
                        "analysisRunId": finding.run_id,
                        "detectorId": finding.detector_id,
                        "detectorVersion": finding.detector_version
                    }),
                );
            }
            let mut evidence_summary = finding.evidence_summary.clone();
            if !evidence_summary.is_object() {
                evidence_summary = json!({});
            }
            if let Some(object) = evidence_summary.as_object_mut() {
                object.insert("aiAssessment".to_string(), assessment.clone());
            }

            let changed = tx.execute(
                "UPDATE analysis_findings SET tier = ?1, action_kind = ?2, executable = ?3, evidence_summary_json = ?4, revision = revision + 1, updated_at = ?5 WHERE id = ?6 AND run_id = ?7 AND finding_key = ?8 AND detector_id = ?9 AND detector_version = ?10 AND scope_hash = ?11 AND primary_subject_kind = ?12 AND primary_subject_id = ?13 AND path_snapshot IS ?14 AND identity_snapshot_json = ?15 AND status = 'active' AND revision = ?16",
                params![tier, action_kind, bool_to_i64(executable), serde_json::to_string(&evidence_summary)?, now, finding.id, finding.run_id, finding.finding_key, finding.detector_id, finding.detector_version, finding.scope_hash, finding.primary_subject_kind, finding.primary_subject_id, finding.path_snapshot, serde_json::to_string(&finding.identity_snapshot)?, finding.revision],
            )?;
            if changed != 1 {
                return Err(DbError::Validation(
                    "Cleanup AI compare-and-swap was lost; assessment was not published."
                        .to_string(),
                ));
            }
            tx.execute(
                "INSERT INTO analysis_finding_evidence (id, finding_id, evidence_kind, subject_kind, subject_id, path_snapshot, value_json, created_at) VALUES (?1, ?2, 'ai_assessment', 'analysis_finding', ?2, ?3, ?4, ?5)",
                params![new_job_id("analysis-ai-evidence"), finding.id, finding.path_snapshot, serde_json::to_string(&assessment)?, now],
            )?;
            published_ids.push(finding.id.clone());
        }

        refresh_analysis_run_aggregate_tx(&tx, &batch.run_id, now)?;
        let mut results = Vec::with_capacity(published_ids.len());
        for finding_id in published_ids {
            results.push(tx.query_row(
                &format!("{ANALYSIS_FINDING_SELECT} LEFT JOIN analysis_finding_decisions AS d ON d.finding_key = f.finding_key WHERE f.id = ?1"),
                params![finding_id],
                analysis_finding_from_row,
            )?);
        }
        tx.commit()?;
        Ok(results)
    }

    pub(crate) fn fail_analysis_run(
        &self,
        run_id: &str,
        error_code: &str,
        error_message: &str,
    ) -> Result<AnalysisRunDto, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = query_analysis_run(&tx, run_id)?;
        if is_analysis_terminal(&run.status) {
            tx.commit()?;
            return Ok(run);
        }
        finish_analysis_run_tx(
            &tx,
            &run,
            "failed",
            false,
            Some(error_code),
            Some(error_message),
        )?;
        let updated = query_analysis_run(&tx, run_id)?;
        tx.commit()?;
        Ok(updated)
    }

    pub(crate) fn get_dedupe_authority(&self) -> Result<DedupeAuthorityDto, DbError> {
        let conn = self.conn()?;
        query_dedupe_authority(&conn)
    }
}

pub(crate) fn bump_dedupe_authority_tx(tx: &Transaction<'_>, status: &str) -> Result<(), DbError> {
    if !matches!(status, "healthy" | "rebuild_required" | "degraded") {
        return Err(DbError::Validation(
            "Invalid dedupe authority status.".to_string(),
        ));
    }
    tx.execute(
        "UPDATE dedupe_authority_state SET revision = revision + 1, status = ?1, updated_at = ?2 WHERE id = 1",
        params![status, current_unix_seconds()],
    )?;
    Ok(())
}

pub(crate) fn invalidate_analysis_findings_for_file_tx(
    tx: &Transaction<'_>,
    file_id: &str,
) -> Result<usize, DbError> {
    let now = current_unix_seconds();
    tx.execute(
        "UPDATE analysis_findings SET status = 'stale', stale_at = ?1, revision = revision + 1, updated_at = ?1 WHERE status = 'active' AND ((primary_subject_kind = 'managed_file' AND primary_subject_id = ?2) OR (primary_subject_kind = 'file' AND primary_subject_id = ?2))",
        params![now, file_id],
    )
    .map_err(DbError::from)
}

pub(crate) fn invalidate_analysis_findings_for_group_tx(
    tx: &Transaction<'_>,
    group_id: &str,
) -> Result<usize, DbError> {
    let now = current_unix_seconds();
    tx.execute(
        "UPDATE analysis_findings SET status = 'stale', stale_at = ?1, revision = revision + 1, updated_at = ?1 WHERE status = 'active' AND primary_subject_kind = 'duplicate_group' AND primary_subject_id = ?2",
        params![now, group_id],
    )
    .map_err(DbError::from)
}

pub(crate) fn invalidate_analysis_findings_for_root_tx(
    tx: &Transaction<'_>,
    root_path: &str,
) -> Result<usize, DbError> {
    let root = trim_trailing_path_separators(root_path);
    let escaped = root
        .replace('~', "~~")
        .replace('%', "~%")
        .replace('_', "~_");
    let now = current_unix_seconds();
    tx.execute(
        "UPDATE analysis_findings SET status = 'stale', stale_at = ?1, revision = revision + 1, updated_at = ?1 WHERE status = 'active' AND path_snapshot IS NOT NULL AND (path_snapshot = ?2 OR path_snapshot LIKE ?3 ESCAPE '~' OR path_snapshot LIKE ?4 ESCAPE '~')",
        params![now, root, format!("{escaped}/%"), format!("{escaped}\\%")],
    )
    .map_err(DbError::from)
}

fn canonical_analysis_scope(
    conn: &Connection,
    request: &AnalysisScopeRequest,
) -> Result<(Value, String), DbError> {
    let kind = match request.kind.trim() {
        "allManagedFileLibrary" | "all_managed_file_library" => "all_managed_file_library",
        "explicitEnabledScanRoots" | "explicit_enabled_scan_roots" => "explicit_enabled_scan_roots",
        "approvedCleanupPaths" | "approved_cleanup_paths" => "approved_cleanup_paths",
        _ => {
            return Err(DbError::Validation(
                "Unsupported analysis scope.".to_string(),
            ))
        }
    };
    let value = if kind == "approved_cleanup_paths" {
        let mut paths = request
            .paths
            .iter()
            .map(|path| path.trim().to_string())
            .filter(|path| !path.is_empty())
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        if paths.is_empty() {
            return Err(DbError::Validation(
                "Approved cleanup scope is empty.".to_string(),
            ));
        }
        json!({"kind": kind, "paths": paths})
    } else {
        let root_ids = if kind == "all_managed_file_library" {
            let mut statement = conn.prepare(
                "SELECT id FROM scan_roots WHERE enabled = 1 AND source_kind = 'file_library' ORDER BY id",
            )?;
            let result = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            result
        } else {
            let mut root_ids = request
                .root_ids
                .iter()
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect::<Vec<_>>();
            root_ids.sort();
            root_ids.dedup();
            root_ids
        };
        if root_ids.is_empty() {
            return Err(DbError::Validation(
                "No enabled managed roots are available.".to_string(),
            ));
        }
        for root_id in &root_ids {
            let enabled = conn
                .query_row(
                    "SELECT enabled FROM scan_roots WHERE id = ?1 AND source_kind = 'file_library'",
                    params![root_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            if enabled != Some(1) {
                return Err(DbError::Validation(
                    "Analysis scope contains a disabled or unknown managed root.".to_string(),
                ));
            }
        }
        json!({"kind": kind, "rootIds": root_ids})
    };
    let serialized = serde_json::to_string(&value)?;
    Ok((
        value,
        blake3::hash(serialized.as_bytes()).to_hex().to_string(),
    ))
}

fn scope_request_from_value(value: &Value) -> Result<AnalysisScopeRequest, DbError> {
    Ok(AnalysisScopeRequest {
        kind: value
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        root_ids: value
            .get("rootIds")
            .or_else(|| value.get("root_ids"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        paths: value
            .get("paths")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn analysis_source_snapshot(
    conn: &Connection,
    scope: &Value,
    captured_at: Option<i64>,
) -> Result<(String, String), DbError> {
    let kind = scope
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let value = if kind == "approved_cleanup_paths" {
        let paths = scope
            .get("paths")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .map(|path| {
                let metadata = std::fs::symlink_metadata(Path::new(&path)).ok();
                json!({
                    "path": path,
                    "exists": metadata.is_some(),
                    "isDirectory": metadata.as_ref().is_some_and(std::fs::Metadata::is_dir),
                    "size": metadata.as_ref().map(std::fs::Metadata::len),
                    "modified": metadata.as_ref().and_then(|item| item.modified().ok()).and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok()).and_then(|duration| i64::try_from(duration.as_nanos()).ok()),
                })
            })
            .collect::<Vec<_>>();
        json!({
            "scope": scope,
            "paths": paths,
            "exclusionPolicyVersion": 1,
            "detectorRegistryVersion": ANALYSIS_REGISTRY_VERSION,
            "capturedAt": captured_at,
        })
    } else {
        let roots = if kind == "all_managed_file_library" {
            let mut statement = conn.prepare(
                "SELECT id FROM scan_roots WHERE enabled = 1 AND source_kind = 'file_library' ORDER BY id",
            )?;
            let root_ids = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            root_ids.into_iter().map(Value::String).collect::<Vec<_>>()
        } else {
            scope
                .get("rootIds")
                .or_else(|| scope.get("root_ids"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let authority = conn.query_row(
            "SELECT revision, status, last_authoritative_run_id FROM dedupe_authority_state WHERE id = 1",
            [],
            |row| Ok(json!({"revision": row.get::<_, i64>(0)?, "status": row.get::<_, String>(1)?, "lastAuthoritativeRunId": row.get::<_, Option<String>>(2)?})),
        )?;
        let mut root_values = Vec::with_capacity(roots.len());
        for root in roots {
            let root_id = root.as_str().unwrap_or_default();
            root_values.push(conn.query_row(
                "SELECT id, enabled, last_successful_generation, watcher_revision, watcher_applied_revision, needs_reconciliation, watcher_rule_recovery_required, health_status FROM scan_roots WHERE id = ?1",
                params![root_id],
                |row| Ok(json!({
                    "rootId": row.get::<_, String>(0)?,
                    "enabled": row.get::<_, i64>(1)? != 0,
                    "lastSuccessfulGeneration": row.get::<_, Option<i64>>(2)?,
                    "watcherRevision": row.get::<_, i64>(3)?,
                    "watcherAppliedRevision": row.get::<_, i64>(4)?,
                    "needsReconciliation": row.get::<_, i64>(5)? != 0,
                    "watcherRuleRecoveryRequired": row.get::<_, i64>(6)? != 0,
                    "healthStatus": row.get::<_, String>(7)?,
                })),
            )?);
        }
        json!({
            "roots": root_values,
            "dedupeAuthority": authority,
            "detectorRegistryVersion": ANALYSIS_REGISTRY_VERSION,
            "capturedAt": captured_at,
        })
    };
    let serialized = serde_json::to_string(&value)?;
    let mut comparison_value = value;
    if let Some(object) = comparison_value.as_object_mut() {
        object.remove("capturedAt");
    }
    let comparison_serialized = serde_json::to_string(&comparison_value)?;
    Ok((
        serialized.clone(),
        blake3::hash(comparison_serialized.as_bytes())
            .to_hex()
            .to_string(),
    ))
}

fn validate_finding_draft(draft: &FindingDraft) -> Result<(), DbError> {
    if !matches!(draft.tier.as_str(), "safe" | "review" | "caution")
        || !matches!(draft.confidence.as_str(), "exact" | "estimated" | "unknown")
        || !matches!(
            draft.action_kind.as_str(),
            "reveal"
                | "review_duplicate_group"
                | "uninstall_advice"
                | "app_internal_cleanup"
                | "safe_trash_candidate"
                | "none"
        )
    {
        return Err(DbError::Validation(
            "Finding draft contains an invalid domain value.".to_string(),
        ));
    }
    if draft.detector_id == "duplicate_reclaimable_v1" && draft.executable {
        return Err(DbError::Validation(
            "Duplicate findings are never executable.".to_string(),
        ));
    }
    if draft.tier != "safe" && draft.executable {
        return Err(DbError::Validation(
            "Only Safe findings may be executable.".to_string(),
        ));
    }
    if draft.potential_reclaimable_bytes < 0 || draft.size_bytes < 0 {
        return Err(DbError::Validation(
            "Finding byte values cannot be negative.".to_string(),
        ));
    }
    Ok(())
}

fn is_analysis_terminal(status: &str) -> bool {
    matches!(
        status,
        "completed" | "completed_with_warnings" | "cancelled" | "failed" | "interrupted"
    )
}

fn finish_analysis_run_tx(
    tx: &Transaction<'_>,
    run: &AnalysisRunDto,
    status: &str,
    rerun_required: bool,
    error_code: Option<&str>,
    error_message: Option<&str>,
) -> Result<(), DbError> {
    if !is_analysis_terminal(status) {
        return Err(DbError::Validation(
            "Invalid analysis terminal status.".to_string(),
        ));
    }
    let now = current_unix_seconds();
    let safe_count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM analysis_findings WHERE run_id = ?1 AND status = 'active' AND tier = 'safe'",
        params![run.id],
        |row| row.get(0),
    )?;
    let review_count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM analysis_findings WHERE run_id = ?1 AND status = 'active' AND tier = 'review'",
        params![run.id],
        |row| row.get(0),
    )?;
    let caution_count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM analysis_findings WHERE run_id = ?1 AND status = 'active' AND tier = 'caution'",
        params![run.id],
        |row| row.get(0),
    )?;
    let (exact, potential) = analysis_reclaimable_totals_tx(tx, &run.id)?;
    let changed = tx.execute(
        "UPDATE analysis_runs SET status = ?1, phase = 'completed', rerun_required = ?2, findings_published = (SELECT COUNT(*) FROM analysis_findings WHERE run_id = ?3 AND status = 'active'), safe_count = ?4, review_count = ?5, caution_count = ?6, exact_reclaimable_bytes = ?7, potential_reclaimable_bytes = ?8, error_code = COALESCE(?9, error_code), error_message = COALESCE(?10, error_message), finished_at = ?11, last_checkpoint_at = ?11, revision = revision + 1, updated_at = ?11 WHERE id = ?3 AND revision = ?12 AND status IN ('queued', 'running', 'cancelling')",
        params![status, bool_to_i64(rerun_required), run.id, safe_count, review_count, caution_count, exact, potential, error_code, error_message, now, run.revision],
    )?;
    if changed != 1 {
        return Err(DbError::Validation(
            "Analysis terminal transition lost durable revision ownership.".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct ReclaimableSubject {
    stable_key: String,
    kind: String,
    path: Option<String>,
    identity_json: String,
    exact: i64,
    potential: i64,
}

#[derive(Debug, Clone)]
struct ExactPhysicalSubject {
    stable_key: String,
    paths: Vec<String>,
    bytes: i64,
}

fn analysis_reclaimable_totals_tx(tx: &Connection, run_id: &str) -> Result<(i64, i64), DbError> {
    analysis_reclaimable_totals_for_tier(tx, run_id, None)
}

fn analysis_reclaimable_totals_for_tier(
    tx: &Connection,
    run_id: &str,
    tier: Option<&str>,
) -> Result<(i64, i64), DbError> {
    let mut statement = tx.prepare(
        "SELECT primary_subject_kind, primary_subject_id, path_snapshot, identity_snapshot_json, exact_reclaimable_bytes, potential_reclaimable_bytes FROM analysis_findings WHERE run_id = ?1 AND status = 'active' AND (?2 IS NULL OR (tier = ?2 AND NOT EXISTS (SELECT 1 FROM analysis_finding_decisions d WHERE d.finding_key=analysis_findings.finding_key AND (d.decision='dismissed' OR (d.decision='snoozed' AND d.snoozed_until>unixepoch())))))",
    )?;
    let mut subjects = HashMap::<(String, String, String), ReclaimableSubject>::new();
    let rows = statement.query_map(params![run_id, tier], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<i64>>(4)?.unwrap_or(0).max(0),
            row.get::<_, i64>(5)?.max(0),
        ))
    })?;
    for row in rows {
        let (kind, subject_id, path, identity, exact, potential) = row?;
        let key = (kind, subject_id, identity.clone());
        let kind = key.0.clone();
        let stable_key = reclaimable_subject_key(&key.0, &key.1, &key.2, path.as_deref());
        subjects
            .entry(key)
            .and_modify(|subject| {
                subject.exact = subject.exact.max(exact);
                subject.potential = subject.potential.max(potential);
            })
            .or_insert(ReclaimableSubject {
                stable_key,
                kind,
                path,
                identity_json: identity,
                exact,
                potential,
            });
    }
    drop(statement);

    let mut with_paths = subjects
        .values()
        .filter(|subject| subject.path.is_some())
        .cloned()
        .collect::<Vec<_>>();
    with_paths.sort_by(|left, right| {
        normalized_aggregate_path(left.path.as_deref().unwrap_or_default())
            .len()
            .cmp(&normalized_aggregate_path(right.path.as_deref().unwrap_or_default()).len())
            .then_with(|| right.potential.cmp(&left.potential))
            .then_with(|| left.stable_key.cmp(&right.stable_key))
    });
    let mut retained_paths = Vec::<String>::new();
    let mut potential = 0_i64;
    for subject in with_paths {
        let path = normalized_aggregate_path(subject.path.as_deref().unwrap_or_default());
        if retained_paths
            .iter()
            .any(|parent| aggregate_path_is_same_or_child(&path, parent))
        {
            continue;
        }
        retained_paths.push(path);
        potential = potential.saturating_add(subject.potential);
    }
    for subject in subjects.values().filter(|subject| subject.path.is_none()) {
        potential = potential.saturating_add(subject.potential);
    }
    let mut exact_subjects = Vec::<ExactPhysicalSubject>::new();
    for subject in subjects.values().filter(|subject| subject.exact > 0) {
        if subject.kind == "duplicate_group" {
            exact_subjects.extend(authoritative_duplicate_exact_subjects_tx(tx, subject)?);
        } else {
            exact_subjects.push(ExactPhysicalSubject {
                stable_key: subject.stable_key.clone(),
                paths: subject.path.iter().cloned().collect(),
                bytes: subject.exact,
            });
        }
    }
    let exact = exact_physical_union(exact_subjects);
    Ok((exact, potential))
}

fn authoritative_duplicate_exact_subjects_tx(
    tx: &Connection,
    finding: &ReclaimableSubject,
) -> Result<Vec<ExactPhysicalSubject>, DbError> {
    let identity = serde_json::from_str::<Value>(&finding.identity_json).unwrap_or(Value::Null);
    let group_id = identity
        .get("groupId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let expected_hash = identity
        .get("fullHash")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(expected_revision) = identity.get("revision").and_then(Value::as_i64) else {
        return Ok(Vec::new());
    };
    if group_id.is_empty() || expected_hash.is_empty() {
        return Ok(Vec::new());
    }

    let group = tx
        .query_row(
            "SELECT status, full_hash, revision, physical_copy_count, exact_reclaimable_bytes, reclaimable_confidence FROM duplicate_groups WHERE id = ?1",
            params![group_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((status, full_hash, revision, physical_copy_count, group_exact, confidence)) = group
    else {
        return Ok(Vec::new());
    };
    if status != "active"
        || full_hash != expected_hash
        || revision != expected_revision
        || confidence != "exact"
        || physical_copy_count < 2
    {
        return Ok(Vec::new());
    }

    let mut statement = tx.prepare(
        "SELECT member.file_id, member.path_snapshot, member.physical_key, member.identity_status, member.size, f.is_stale
         FROM duplicate_group_members AS member
         JOIN files AS f ON f.id = member.file_id
         WHERE member.group_id = ?1
         ORDER BY member.path_snapshot COLLATE NOCASE, member.file_id",
    )?;
    let rows = statement.query_map(params![group_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)? != 0,
        ))
    })?;
    let mut physical = HashMap::<String, (String, String, i64, Vec<String>)>::new();
    for row in rows {
        let (file_id, path, physical_key, identity_status, size, is_stale) = row?;
        let Some(physical_key) = physical_key.filter(|value| !value.is_empty()) else {
            return Ok(Vec::new());
        };
        if is_stale || identity_status != "verified" || size <= 0 {
            return Ok(Vec::new());
        }
        let normalized_path = normalized_aggregate_path(&path);
        physical
            .entry(physical_key)
            .and_modify(|current| {
                current.3.push(normalized_path.clone());
                if (normalized_path.as_str(), file_id.as_str())
                    < (current.0.as_str(), current.1.as_str())
                {
                    current.0 = normalized_path.clone();
                    current.1 = file_id.clone();
                }
                if current.2 != size {
                    current.2 = -1;
                }
            })
            .or_insert((
                normalized_path.clone(),
                file_id,
                size,
                vec![normalized_path],
            ));
    }
    if physical.len() != usize::try_from(physical_copy_count).unwrap_or_default()
        || physical.values().any(|subject| subject.2 <= 0)
    {
        return Ok(Vec::new());
    }

    let mut physical = physical.into_iter().collect::<Vec<_>>();
    physical.sort_by(|left, right| {
        left.1
             .0
            .cmp(&right.1 .0)
            .then_with(|| left.1 .1.cmp(&right.1 .1))
            .then_with(|| left.0.cmp(&right.0))
    });
    let reclaimable = physical
        .into_iter()
        .skip(1)
        .map(|(physical_key, (_, _, bytes, mut paths))| {
            paths.sort();
            paths.dedup();
            ExactPhysicalSubject {
                stable_key: format!("physical:{physical_key}"),
                paths,
                bytes,
            }
        })
        .collect::<Vec<_>>();
    let resolved_exact = reclaimable
        .iter()
        .map(|subject| subject.bytes)
        .fold(0_i64, i64::saturating_add);
    if group_exact.unwrap_or_default().max(0) != resolved_exact || finding.exact != resolved_exact {
        return Ok(Vec::new());
    }
    Ok(reclaimable)
}

fn exact_physical_union(subjects: Vec<ExactPhysicalSubject>) -> i64 {
    let mut by_stable_key = HashMap::<String, ExactPhysicalSubject>::new();
    for mut subject in subjects.into_iter().filter(|subject| subject.bytes > 0) {
        subject.paths = subject
            .paths
            .into_iter()
            .map(|path| normalized_aggregate_path(&path))
            .filter(|path| !path.is_empty())
            .collect();
        subject.paths.sort();
        subject.paths.dedup();
        by_stable_key
            .entry(subject.stable_key.clone())
            .and_modify(|current| {
                current.bytes = current.bytes.max(subject.bytes);
                current.paths.extend(subject.paths.clone());
                current.paths.sort();
                current.paths.dedup();
            })
            .or_insert(subject);
    }

    let mut subjects = by_stable_key.into_values().collect::<Vec<_>>();
    subjects.sort_by(|left, right| {
        shortest_path_len(&left.paths)
            .cmp(&shortest_path_len(&right.paths))
            .then_with(|| right.bytes.cmp(&left.bytes))
            .then_with(|| left.stable_key.cmp(&right.stable_key))
    });
    let mut retained_paths = Vec::<String>::new();
    let mut retained_keys = HashSet::<String>::new();
    let mut exact = 0_i64;
    for subject in subjects {
        if !retained_keys.insert(subject.stable_key) {
            continue;
        }
        if subject.paths.iter().any(|path| {
            retained_paths
                .iter()
                .any(|parent| aggregate_path_is_same_or_child(path, parent))
        }) {
            continue;
        }
        retained_paths.extend(subject.paths);
        exact = exact.saturating_add(subject.bytes);
    }
    exact
}

fn shortest_path_len(paths: &[String]) -> usize {
    paths.iter().map(String::len).min().unwrap_or(usize::MAX)
}

fn analysis_ai_coverage_matches(
    batch: &AnalysisAiPublicationBatch,
    precondition: &AnalysisAiFindingPrecondition,
    assessment: &AnalysisAiAssessmentPublication,
) -> Result<bool, DbError> {
    let coverage = &assessment.evidence["coverage"];
    let assessment_value = &assessment.evidence["assessment"];
    let Some(requested_ids) = coverage["requestedCandidateIds"].as_array() else {
        return Ok(false);
    };
    let ids = requested_ids
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut sorted_ids = ids.clone();
    sorted_ids.sort();
    sorted_ids.dedup();
    let serialized_ids = serde_json::to_string(&sorted_ids)?;
    let candidate_set_fingerprint = blake3::hash(serialized_ids.as_bytes()).to_hex().to_string();
    let request_membership_valid = ids.len() == requested_ids.len()
        && sorted_ids.len() == ids.len()
        && ids.iter().all(|id| {
            batch
                .preconditions
                .iter()
                .any(|candidate| candidate.finding_id == *id)
        });
    let identity_snapshot: Value = serde_json::from_str(&precondition.identity_snapshot_json)?;
    let source_fingerprint = blake3::hash(&serde_json::to_vec(&identity_snapshot)?);
    let source_fingerprint = source_fingerprint.to_hex().to_string();
    let candidate_identity = json!({
        "findingId": precondition.finding_id,
        "findingKey": precondition.finding_key,
        "analysisRunId": precondition.run_id,
        "expectedRevision": precondition.expected_revision,
        "detectorId": precondition.detector_id,
        "detectorVersion": precondition.detector_version,
        "primarySubjectKind": precondition.primary_subject_kind,
        "primarySubjectId": precondition.primary_subject_id,
        "sourceFingerprint": source_fingerprint
    });
    let candidate_identity_fingerprint = blake3::hash(&serde_json::to_vec(&candidate_identity)?)
        .to_hex()
        .to_string();
    let expected_revision = precondition.expected_revision;
    let counts_match = coverage["requestedCount"].as_u64() == Some(ids.len() as u64)
        && coverage["returnedCount"]
            .as_u64()
            .is_some_and(|returned| returned > 0)
        && coverage["omittedCount"].as_u64().is_some()
        && coverage["duplicateCount"].as_u64().is_some()
        && coverage["unknownCount"].as_u64().is_some()
        && coverage["returnedCount"].as_u64().unwrap_or_default()
            + coverage["omittedCount"].as_u64().unwrap_or_default()
            + coverage["duplicateCount"].as_u64().unwrap_or_default()
            == ids.len() as u64;
    Ok(request_membership_valid
        && ids
            .iter()
            .filter(|id| *id == &assessment.finding_id)
            .count()
            == 1
        && candidate_set_fingerprint == coverage["candidateSetFingerprint"]
        && batch.expected_candidate_set_fingerprint == coverage["requestCandidateSetFingerprint"]
        && coverage["status"] == "returned_exactly_once"
        && coverage["candidateId"] == assessment.finding_id
        && coverage["returnedCandidateId"] == assessment.finding_id
        && assessment_value["candidateId"] == assessment.finding_id
        && coverage["analysisRunId"] == batch.run_id
        && coverage["analysisRunRevision"].as_i64() == Some(batch.expected_run_revision)
        && coverage["analysisSourceSnapshotHash"] == batch.expected_source_snapshot_hash
        && coverage["detectorSetHash"] == batch.expected_detector_set_hash
        && coverage["detectorId"] == precondition.detector_id
        && coverage["detectorVersion"].as_i64() == Some(precondition.detector_version)
        && coverage["detectorRevision"].as_i64() == Some(precondition.detector_revision)
        && coverage["expectedFindingRevision"].as_i64() == Some(expected_revision)
        && coverage["candidateIdentityFingerprint"] == candidate_identity_fingerprint
        && coverage["sourceFingerprint"] == source_fingerprint
        && coverage["providerPolicyRevision"] == batch.expected_settings_revision
        && counts_match)
}

fn refresh_analysis_run_aggregate_tx(
    tx: &Transaction<'_>,
    run_id: &str,
    now: i64,
) -> Result<AnalysisRunDto, DbError> {
    let safe_count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM analysis_findings WHERE run_id = ?1 AND status = 'active' AND tier = 'safe'",
        params![run_id],
        |row| row.get(0),
    )?;
    let review_count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM analysis_findings WHERE run_id = ?1 AND status = 'active' AND tier = 'review'",
        params![run_id],
        |row| row.get(0),
    )?;
    let caution_count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM analysis_findings WHERE run_id = ?1 AND status = 'active' AND tier = 'caution'",
        params![run_id],
        |row| row.get(0),
    )?;
    let (exact, potential) = analysis_reclaimable_totals_tx(tx, run_id)?;
    let changed = tx.execute(
        "UPDATE analysis_runs SET findings_published = (SELECT COUNT(*) FROM analysis_findings WHERE run_id = ?1 AND status = 'active'), safe_count = ?2, review_count = ?3, caution_count = ?4, exact_reclaimable_bytes = ?5, potential_reclaimable_bytes = ?6, revision = revision + 1, updated_at = ?7 WHERE id = ?1 AND status IN ('completed', 'completed_with_warnings', 'cancelled', 'failed', 'interrupted')",
        params![run_id, safe_count, review_count, caution_count, exact, potential, now],
    )?;
    if changed != 1 {
        return Err(DbError::Validation(
            "Analysis run aggregate refresh was rejected by the durable run state.".to_string(),
        ));
    }
    query_analysis_run(tx, run_id)
}

fn reclaimable_subject_key(
    subject_kind: &str,
    subject_id: &str,
    identity_json: &str,
    path: Option<&str>,
) -> String {
    let identity = serde_json::from_str::<Value>(identity_json).unwrap_or(Value::Null);
    if subject_kind == "duplicate_group" {
        return format!(
            "duplicate-group:{subject_id}:{}",
            identity
                .get("fullHash")
                .and_then(Value::as_str)
                .unwrap_or_default()
        );
    }
    let physical = identity
        .get("physical")
        .and_then(|value| value.get("physicalKey"))
        .and_then(Value::as_str)
        .or_else(|| {
            identity
                .get("live")
                .and_then(|value| value.get("physicalKey"))
                .and_then(Value::as_str)
        })
        .or_else(|| identity.get("physicalKey").and_then(Value::as_str));
    if let Some(physical) = physical.filter(|value| !value.is_empty()) {
        return format!("physical:{physical}");
    }
    format!(
        "{subject_kind}:{subject_id}:{}",
        normalized_aggregate_path(path.unwrap_or_default())
    )
}

#[cfg(test)]
mod exact_physical_union_tests {
    use super::{exact_physical_union, ExactPhysicalSubject};

    #[test]
    fn physical_union_is_insertion_order_independent_and_counts_each_subject_once() {
        let claims = vec![
            ExactPhysicalSubject {
                stable_key: "physical:shared".to_string(),
                paths: vec!["/root/shared.bin".to_string()],
                bytes: 100,
            },
            ExactPhysicalSubject {
                stable_key: "physical:unrelated".to_string(),
                paths: vec!["/root/unrelated.bin".to_string()],
                bytes: 50,
            },
            ExactPhysicalSubject {
                stable_key: "physical:shared".to_string(),
                paths: vec![
                    "/root/shared.bin".to_string(),
                    "/root/shared-hardlink.bin".to_string(),
                ],
                bytes: 100,
            },
        ];
        let mut reversed = claims.clone();
        reversed.reverse();

        assert_eq!(exact_physical_union(claims), 150);
        assert_eq!(exact_physical_union(reversed), 150);
    }
}
