//! Task 06 durable organization-plan repository.
//!
//! Plans are review artifacts. Paths and operation kinds are derived from the
//! indexed file authority and are never accepted from renderer requests.

use super::files::operation_preview_from_indexed;
use super::library::{
    canonicalize_file_query_spec, clear_temp_selection_ids, current_library_revision,
    file_matches_authoritative_query_scope, files_matching_authoritative_query_scope,
    selection_where, FileLibraryScopeV2, FileLibrarySortV2, FileQueryFiltersV2, FileQuerySpecV2,
    LibrarySelectionV1,
};
use super::*;
use crate::path_identity::normalize_text_for_compare;
use rusqlite::{params, params_from_iter, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::time::Instant;

mod materialize;
pub(crate) use materialize::materialize_organization_plan;
mod cursor;
mod projection;
mod queries;
mod semantic;
mod validation;

#[cfg(test)]
use cursor::{
    decode_group_cursor, decode_item_cursor, encode_group_cursor, encode_item_cursor,
    OrganizationGroupCursor, OrganizationItemCursor,
};
use projection::{
    load_indexed_files_for_projection, load_organization_plan_group_projection,
    load_organization_plan_items_for_projection, organization_group_id, organization_group_key,
};
use semantic::{current_organization_projection, current_organization_proposal};
use validation::{
    normalize_decision, normalize_group_decision, organization_item_action_error,
    validate_edited_filename, validate_id, validate_organization_page_size,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrganizationReviewReason {
    LowConfidence,
    SensitiveFile,
    NonNormalRisk,
    PossibleDuplicate,
    RequiresConfirmation,
    TargetDirectoryCreation,
    TargetCollision,
    SourceChanged,
    ProposalChanged,
    ManagedScopeChanged,
    MissingPreview,
    UnsupportedOperation,
    UnsafeFilename,
    ExtensionChangeBlocked,
}

impl OrganizationReviewReason {
    const fn code(self) -> &'static str {
        match self {
            Self::LowConfidence => "low_confidence",
            Self::SensitiveFile => "sensitive_file",
            Self::NonNormalRisk => "non_normal_risk",
            Self::PossibleDuplicate => "possible_duplicate",
            Self::RequiresConfirmation => "requires_confirmation",
            Self::TargetDirectoryCreation => "target_directory_creation",
            Self::TargetCollision => "target_collision",
            Self::SourceChanged => "source_changed",
            Self::ProposalChanged => "proposal_changed",
            Self::ManagedScopeChanged => "managed_scope_changed",
            Self::MissingPreview => "missing_preview",
            Self::UnsupportedOperation => "unsupported_operation",
            Self::UnsafeFilename => "unsafe_filename",
            Self::ExtensionChangeBlocked => "extension_change_blocked",
        }
    }
}

const ORGANIZATION_PLAN_VERSION: i32 = 1;
const ORGANIZATION_PLAN_MAX_ITEMS: usize = 10_000;
const ORGANIZATION_EXECUTION_MAX_ITEMS: usize = 1_000;
const ORGANIZATION_PAGE_MAX: u32 = 200;
const ORGANIZATION_GROUP_SAMPLE_MAX: usize = 3;
const ORGANIZATION_GROUP_CURSOR_VERSION: i32 = 1;
const ORGANIZATION_GROUP_PROJECTION_VERSION: &str = "organization-groups-projection-v1";

#[cfg(test)]
thread_local! {
    static ORGANIZATION_FULL_PROJECTION_COUNT: Cell<usize> = const { Cell::new(0) };
    static ORGANIZATION_EXECUTION_PROFILE: RefCell<Option<HashMap<&'static str, Vec<f64>>>> =
        const { RefCell::new(None) };
}

#[cfg(test)]
struct OrganizationExecutionProfileTimer {
    phase: &'static str,
    started: Option<Instant>,
}

#[cfg(test)]
impl OrganizationExecutionProfileTimer {
    fn new(phase: &'static str) -> Self {
        let active = ORGANIZATION_EXECUTION_PROFILE.with(|profile| profile.borrow().is_some());
        Self {
            phase,
            started: active.then(Instant::now),
        }
    }
}

#[cfg(test)]
impl Drop for OrganizationExecutionProfileTimer {
    fn drop(&mut self) {
        let Some(started) = self.started else {
            return;
        };
        let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
        ORGANIZATION_EXECUTION_PROFILE.with(|profile| {
            if let Some(samples) = profile.borrow_mut().as_mut() {
                samples.entry(self.phase).or_default().push(elapsed_ms);
            }
        });
    }
}

#[cfg(test)]
fn start_organization_execution_profile() {
    ORGANIZATION_EXECUTION_PROFILE.with(|profile| {
        profile.replace(Some(HashMap::new()));
    });
}

#[cfg(test)]
fn take_organization_execution_profile() -> HashMap<&'static str, Vec<f64>> {
    ORGANIZATION_EXECUTION_PROFILE
        .with(|profile| profile.borrow_mut().take())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct CreateOrganizationPlanRequestV1 {
    pub version: i32,
    pub request_id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub source: LibrarySelectionV1,
    #[serde(default)]
    pub expected_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPlanDto {
    pub id: String,
    pub title: String,
    pub status: String,
    pub source_kind: String,
    pub source_query_fingerprint: Option<String>,
    pub source_snapshot_revision: i64,
    pub requested_count: i64,
    pub materialized_count: i64,
    pub planner_version: i64,
    pub revision: i64,
    pub active_execution_id: Option<String>,
    pub active_operation_batch_id: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error_detail: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub ready_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub summary: OrganizationPlanSummaryDto,
    pub effective_summary: Option<OrganizationPlanEffectiveSummaryDto>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPlanSummaryDto {
    pub undecided: i64,
    pub accepted: i64,
    pub kept: i64,
    pub edited: i64,
    pub needs_analysis: i64,
    pub needs_review: i64,
    pub pending_review: i64,
    pub reviewed: i64,
    pub ready: i64,
    pub blocked: i64,
    pub stale: i64,
    pub executing: i64,
    pub executed: i64,
    pub failed: i64,
    pub skipped: i64,
    pub remaining_executable: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPlanEffectiveSummaryDto {
    pub ready: i64,
    pub reviewed: i64,
    pub pending_review: i64,
    pub blocked: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationSemanticExplanationDto {
    pub assessment_fingerprint: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPlanItemDto {
    pub id: String,
    pub plan_id: String,
    pub ordinal: i64,
    pub file_id_snapshot: String,
    pub source_path_snapshot: String,
    pub source_name_snapshot: String,
    pub source_size_snapshot: i64,
    pub source_mtime_snapshot: i64,
    pub source_is_dir_snapshot: bool,
    pub proposal_fingerprint: String,
    /// Ephemeral explanation from the current bound SemanticAssessmentV1.
    /// It is omitted whenever the current proposal no longer matches this item.
    pub semantic_explanation: Option<OrganizationSemanticExplanationDto>,
    pub proposal_kind: String,
    pub proposed_target_directory: String,
    pub proposed_name: String,
    pub proposed_target_path: String,
    pub decision: String,
    pub edited_name: Option<String>,
    pub validity: String,
    pub review_state: String,
    pub effective_readiness: String,
    pub confidence: f64,
    pub risk_level: String,
    pub requires_confirmation: bool,
    pub blocking_code: Option<String>,
    pub blocking_detail: Option<String>,
    pub authoritative_preview_id: Option<String>,
    pub review_reasons: Vec<String>,
    pub available_actions: Vec<String>,
    pub operation_log_id: Option<String>,
    pub execution_id: Option<String>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct QueryOrganizationPlanItemsRequest {
    pub plan_id: String,
    #[serde(default)]
    pub cursor: Option<String>,
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPlanItemPageDto {
    pub plan_id: String,
    pub plan_revision: i64,
    pub items: Vec<OrganizationPlanItemDto>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct QueryOrganizationPlanGroupsRequest {
    pub plan_id: String,
    #[serde(default)]
    pub cursor: Option<String>,
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPlanGroupSampleDto {
    pub item_id: String,
    pub source_name: String,
    pub source_path: String,
    pub proposed_name: String,
    pub decision: String,
    pub validity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationReviewReasonCountDto {
    pub reason: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPlanGroupSummaryDto {
    pub group_id: String,
    pub plan_id: String,
    pub label: String,
    pub target_directory: Option<String>,
    pub proposal_kind: String,
    pub readiness: String,
    pub risk_level: String,
    pub item_count: i64,
    pub total_bytes: i64,
    pub accepted_count: i64,
    pub excluded_count: i64,
    pub stale_count: i64,
    pub conflict_count: i64,
    pub confidence_band: String,
    pub review_reason_counts: Vec<OrganizationReviewReasonCountDto>,
    pub available_actions: Vec<String>,
    pub group_actions: OrganizationPlanGroupActionsDto,
    pub projection_fingerprint: String,
    pub sample_items: Vec<OrganizationPlanGroupSampleDto>,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPlanGroupActionsDto {
    pub can_accept_all: bool,
    pub can_keep_all: bool,
    pub can_clear_all: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPlanGroupPageDto {
    pub plan_id: String,
    pub plan_revision: i64,
    pub groups: Vec<OrganizationPlanGroupSummaryDto>,
    pub effective_summary: OrganizationPlanEffectiveSummaryDto,
    pub projection_fingerprint: String,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct QueryOrganizationPlanGroupItemsRequest {
    pub plan_id: String,
    pub group_id: String,
    #[serde(default)]
    pub cursor: Option<String>,
    pub expected_projection_fingerprint: String,
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPlanGroupItemPageDto {
    pub plan_id: String,
    pub group_id: String,
    pub plan_revision: i64,
    pub projection_fingerprint: String,
    pub items: Vec<OrganizationPlanItemDto>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOrganizationPlanGroupDecisionRequest {
    pub plan_id: String,
    pub group_id: String,
    pub expected_plan_revision: i64,
    pub expected_projection_fingerprint: String,
    pub expected_item_count: i64,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOrganizationPlanGroupDecisionResultDto {
    pub plan: OrganizationPlanDto,
    pub group: Option<OrganizationPlanGroupSummaryDto>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationDecisionMutation {
    pub item_id: String,
    pub expected_item_revision: i64,
    pub decision: String,
    #[serde(default)]
    pub edited_filename: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOrganizationPlanDecisionsRequest {
    pub plan_id: String,
    pub expected_plan_revision: i64,
    #[serde(default)]
    pub safe_batch: bool,
    pub mutations: Vec<OrganizationDecisionMutation>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPlanRevisionRequest {
    pub plan_id: String,
    pub expected_plan_revision: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct DeleteOrganizationPlanRequest {
    pub plan_id: String,
    pub expected_plan_revision: i64,
    pub confirmed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeOrganizationPlanItemsRequest {
    pub plan_id: String,
    pub expected_plan_revision: i64,
    #[serde(default)]
    pub item_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeOrganizationPlanItemsResult {
    pub plan_id: String,
    pub queued_count: i64,
    pub requires_refresh: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPlanSelectionRequest {
    pub plan_id: String,
    pub expected_plan_revision: i64,
    #[serde(default)]
    pub item_ids: Vec<String>,
    #[serde(default)]
    pub all_accepted: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteOrganizationPlanRequest {
    pub plan_id: String,
    pub expected_plan_revision: i64,
    pub dry_run_fingerprint: String,
    #[serde(default)]
    pub item_ids: Vec<String>,
    #[serde(default)]
    pub all_accepted: bool,
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteOrganizationPlanResultDto {
    pub plan: OrganizationPlanDto,
    pub execution_id: String,
    pub operation_batch_id: String,
    pub attempted_count: i64,
    pub succeeded_count: i64,
    pub failed_count: i64,
    pub skipped_count: i64,
}

pub(crate) struct OrganizationExecutionDispatch {
    pub execution_id: String,
    pub operation_batch_id: String,
    pub item_ids: Vec<String>,
    pub selections: Vec<crate::file_ops::OperationPreviewRequest>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationDryRunItemDto {
    pub item_id: String,
    pub operation_kind: String,
    pub from: String,
    pub to: String,
    pub edited_filename: Option<String>,
    pub parent_directory_to_create: Option<String>,
    pub collision: bool,
    pub cross_volume: bool,
    pub risk_level: String,
    pub requires_confirmation: bool,
    pub source_health: String,
    pub authoritative_preview_id: Option<String>,
    pub executable: bool,
    pub blocking_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPlanDryRunDto {
    pub plan_id: String,
    pub plan_revision: i64,
    pub selected_count: i64,
    pub executable_count: i64,
    pub blocked_count: i64,
    pub stale_count: i64,
    pub total_bytes: i64,
    pub operation_kinds: Vec<String>,
    pub items: Vec<OrganizationDryRunItemDto>,
    pub execution_batch_limit: usize,
    pub dry_run_fingerprint: String,
}

impl Database {
    pub fn recover_organization_plans(&self) -> Result<usize, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let now = current_unix_seconds();
        let mut changed = tx.execute(
            "UPDATE organization_plans SET status = 'failed', revision = revision + 1,
                    updated_at = ?1, last_error_code = 'organization_build_interrupted',
                    last_error_detail = 'Plan publication was interrupted before ready.'
             WHERE status = 'building'",
            params![now],
        )?;
        let executing = {
            let mut stmt = tx.prepare(
                "SELECT id, active_execution_id, active_operation_batch_id
                 FROM organization_plans WHERE status = 'executing' ORDER BY id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for (plan_id, execution_id, batch_id) in executing {
            let (Some(execution_id), Some(batch_id)) = (execution_id, batch_id) else {
                tx.execute(
                    "UPDATE organization_plans SET status = 'failed',
                            active_execution_id = NULL, active_operation_batch_id = NULL,
                            revision = revision + 1, updated_at = ?2,
                            last_error_code = 'organization_execution_owner_missing',
                            last_error_detail = 'Execution owner metadata was incomplete.'
                     WHERE id = ?1",
                    params![plan_id, now],
                )?;
                changed += 1;
                continue;
            };
            let log_count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM operation_logs WHERE batch_id = ?1",
                params![batch_id],
                |row| row.get(0),
            )?;
            if log_count == 0 {
                tx.execute(
                    "UPDATE organization_plan_items SET validity = 'ready', execution_id = NULL,
                            revision = revision + 1, updated_at = ?2
                     WHERE plan_id = ?1 AND execution_id = ?3 AND validity = 'executing'",
                    params![plan_id, now, execution_id],
                )?;
                tx.execute(
                    "UPDATE organization_plans SET status = 'ready',
                            active_execution_id = NULL, active_operation_batch_id = NULL,
                            revision = revision + 1, updated_at = ?2,
                            last_error_code = 'organization_execution_not_journaled',
                            last_error_detail = 'No operation journal was created; review before retry.'
                     WHERE id = ?1",
                    params![plan_id, now],
                )?;
                changed += 1;
                continue;
            }
            let executing_items = {
                let mut stmt = tx.prepare(
                    "SELECT id, authoritative_preview_id FROM organization_plan_items
                     WHERE plan_id = ?1 AND execution_id = ?2 AND validity = 'executing'",
                )?;
                let rows = stmt.query_map(params![plan_id, execution_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            let mut unknown = 0_i64;
            for (item_id, preview_id) in executing_items {
                let Some(preview_id) = preview_id else {
                    unknown += 1;
                    continue;
                };
                let log = tx
                    .query_row(
                        "SELECT id, status, operation_phase FROM operation_logs
                         WHERE batch_id = ?1 AND id LIKE ?2 ORDER BY created_at DESC LIMIT 1",
                        params![batch_id, format!("%-{preview_id}")],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                            ))
                        },
                    )
                    .optional()?;
                let Some((log_id, status, phase)) = log else {
                    unknown += 1;
                    continue;
                };
                let validity = match status.as_str() {
                    "success" if phase == "completed" => "executed",
                    "failed" => "failed",
                    "skipped" => "skipped",
                    "pending" | "manual_review" => "executing",
                    _ => "executing",
                };
                tx.execute(
                    "UPDATE organization_plan_items SET validity = ?2,
                            operation_log_id = ?3, revision = revision + 1, updated_at = ?4
                     WHERE id = ?1",
                    params![item_id, validity, log_id, now],
                )?;
            }
            let projection = project_organization_plan(&tx, &plan_id, unknown > 0, now)?;
            let (error_code, error_detail) = if unknown > 0 {
                (
                    Some("organization_journal_mapping_unknown"),
                    Some("One or more journal rows could not be mapped; manual review required."),
                )
            } else {
                (None, None)
            };
            tx.execute(
                "UPDATE organization_plans SET status = ?2,
                        active_execution_id = CASE WHEN ?2 = 'executing' THEN active_execution_id ELSE NULL END,
                        active_operation_batch_id = CASE WHEN ?2 = 'executing' THEN active_operation_batch_id ELSE NULL END,
                        revision = revision + 1, updated_at = ?3, completed_at = ?4,
                        last_error_code = ?5, last_error_detail = ?6 WHERE id = ?1",
                params![
                    plan_id,
                    projection.status,
                    now,
                    projection.completed_at,
                    error_code,
                    error_detail
                ],
            )?;
            changed += 1;
        }
        tx.commit()?;
        Ok(changed)
    }

    pub fn prune_organization_plans(&self) -> Result<usize, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let cutoff = current_unix_seconds().saturating_sub(30 * 24 * 60 * 60);
        let ids = {
            let mut stmt = tx.prepare(
                "WITH terminal AS (
                    SELECT p.id, p.updated_at
                    FROM organization_plans AS p
                    WHERE p.status IN ('completed', 'cancelled', 'failed')
                      AND p.active_execution_id IS NULL
                      AND p.active_operation_batch_id IS NULL
                      AND NOT EXISTS (
                        SELECT 1 FROM organization_plan_items AS i
                        WHERE i.plan_id = p.id AND i.validity = 'executing'
                      )
                      AND NOT EXISTS (
                        SELECT 1
                        FROM organization_plan_items AS i
                        JOIN operation_logs AS l ON l.id = i.operation_log_id
                        WHERE i.plan_id = p.id
                          AND (l.status IN ('pending', 'manual_review')
                               OR l.operation_phase NOT IN ('completed', 'failed'))
                      )
                 ),
                 ranked AS (
                    SELECT id, updated_at,
                           ROW_NUMBER() OVER (ORDER BY updated_at DESC, id) AS terminal_rank
                    FROM terminal
                 ),
                 candidates AS (
                    SELECT id, updated_at FROM ranked WHERE updated_at < ?1
                    UNION
                    SELECT id, updated_at FROM ranked WHERE terminal_rank > 100
                 )
                 SELECT id FROM candidates ORDER BY updated_at, id LIMIT 20",
            )?;
            let rows = stmt.query_map(params![cutoff], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for id in &ids {
            tx.execute(
                "DELETE FROM organization_plan_items WHERE plan_id = ?1",
                params![id],
            )?;
            tx.execute("DELETE FROM organization_plans WHERE id = ?1", params![id])?;
        }
        tx.commit()?;
        Ok(ids.len())
    }

    pub fn create_organization_plan(
        &self,
        request: CreateOrganizationPlanRequestV1,
    ) -> Result<OrganizationPlanDto, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let plan = materialize_organization_plan(&tx, request)?;
        tx.commit()?;
        Ok(plan)
    }

    pub fn update_organization_plan_group_decision(
        &self,
        request: UpdateOrganizationPlanGroupDecisionRequest,
    ) -> Result<UpdateOrganizationPlanGroupDecisionResultDto, DbError> {
        let plan_id = validate_id(&request.plan_id, "organization_plan_id_invalid")?;
        let group_id = validate_id(&request.group_id, "organization_group_id_invalid")?;
        let decision = normalize_group_decision(&request.decision)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        require_plan_revision_and_status(
            &tx,
            &plan_id,
            request.expected_plan_revision,
            &["ready", "stale", "partially_completed"],
        )?;
        let projection =
            load_organization_plan_group_projection(&tx, &plan_id, request.expected_plan_revision)?;
        let current_group = projection
            .groups
            .iter()
            .find(|group| group.group_id == group_id);
        let Some(current_group) = current_group else {
            return Err(DbError::Validation(
                "organization_group_changed".to_string(),
            ));
        };
        if current_group.item_count != request.expected_item_count
            || current_group.projection_fingerprint != request.expected_projection_fingerprint
        {
            return Err(DbError::Validation(
                "organization_group_changed".to_string(),
            ));
        }
        let members = projection
            .items
            .into_iter()
            .filter(|projection| {
                organization_group_id(&plan_id, &organization_group_key(&projection.item))
                    == group_id
            })
            .map(|projection| projection.item)
            .collect::<Vec<_>>();
        if members.is_empty() {
            return Err(DbError::Validation(
                "organization_group_changed".to_string(),
            ));
        }

        if members
            .iter()
            .any(|item| matches!(item.validity.as_str(), "executing" | "executed"))
        {
            return Err(DbError::Validation(
                "organization_group_changed".to_string(),
            ));
        }
        let action_available = match decision {
            "accepted" => current_group.group_actions.can_accept_all,
            "kept" => current_group.group_actions.can_keep_all,
            "undecided" => current_group.group_actions.can_clear_all,
            _ => {
                return Err(DbError::Validation(
                    "organization_group_decision_invalid".to_string(),
                ));
            }
        };
        if !action_available {
            return Err(DbError::Validation(
                "organization_group_action_not_available".to_string(),
            ));
        }
        let now = current_unix_seconds();
        for item in members {
            let updated = tx.execute(
                "UPDATE organization_plan_items SET decision = ?1, edited_name = NULL,
                        revision = revision + 1, updated_at = ?2
                 WHERE id = ?3 AND plan_id = ?4 AND revision = ?5
                   AND validity NOT IN ('executing', 'executed')",
                params![decision, now, item.id, plan_id, item.revision],
            )?;
            if updated != 1 {
                return Err(DbError::Validation(
                    "organization_group_changed".to_string(),
                ));
            }
        }
        let updated_plan = tx.execute(
            "UPDATE organization_plans SET
                    status = CASE
                        WHEN status = 'stale'
                         AND NOT EXISTS (
                            SELECT 1 FROM organization_plan_items
                            WHERE plan_id = ?1 AND validity = 'stale'
                         )
                        THEN 'ready' ELSE status END,
                    revision = revision + 1, updated_at = ?3
             WHERE id = ?1 AND revision = ?2",
            params![plan_id, request.expected_plan_revision, now],
        )?;
        if updated_plan != 1 {
            return Err(DbError::Validation(
                "organization_plan_revision_conflict".to_string(),
            ));
        }
        let plan = load_plan(&tx, &plan_id)?;
        tx.commit()?;
        Ok(UpdateOrganizationPlanGroupDecisionResultDto { plan, group: None })
    }

    pub fn update_organization_plan_decisions(
        &self,
        request: UpdateOrganizationPlanDecisionsRequest,
    ) -> Result<OrganizationPlanDto, DbError> {
        if request.mutations.is_empty() || request.mutations.len() > ORGANIZATION_PLAN_MAX_ITEMS {
            return Err(DbError::Validation(
                "organization_decision_batch_invalid".to_string(),
            ));
        }
        let plan_id = validate_id(&request.plan_id, "organization_plan_id_invalid")?;
        let mut unique = HashSet::new();
        if request
            .mutations
            .iter()
            .any(|mutation| !unique.insert(mutation.item_id.as_str()))
        {
            return Err(DbError::Validation(
                "organization_decision_duplicate_item".to_string(),
            ));
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        require_plan_revision_and_status(
            &tx,
            &plan_id,
            request.expected_plan_revision,
            &["ready", "stale", "partially_completed"],
        )?;
        let projected_items = if request.mutations.len() >= 32 {
            Some(
                load_organization_plan_items_for_projection(&tx, &plan_id)?
                    .into_iter()
                    .map(|item| (item.id.clone(), item))
                    .collect::<HashMap<_, _>>(),
            )
        } else {
            None
        };
        let now = current_unix_seconds();
        for mutation in request.mutations {
            let item_id = validate_id(&mutation.item_id, "organization_item_id_invalid")?;
            let decision = normalize_decision(&mutation.decision)?;
            let current_item = match projected_items.as_ref() {
                Some(items) => items.get(&item_id).cloned().ok_or_else(|| {
                    DbError::Validation("organization_item_not_found".to_string())
                })?,
                None => load_organization_plan_item_for_projection(&tx, &plan_id, &item_id)?,
            };
            if current_item.revision != mutation.expected_item_revision {
                return Err(DbError::Validation(
                    "organization_item_revision_conflict".to_string(),
                ));
            }
            match decision {
                "accepted" => {
                    if request.safe_batch {
                        require_safe_batch_item_projection(&current_item)?;
                    } else if !current_item
                        .available_actions
                        .iter()
                        .any(|action| action == "accept_suggestion")
                    {
                        return Err(DbError::Validation(
                            organization_item_action_error("accept_suggestion").to_string(),
                        ));
                    }
                }
                "edited" => {
                    if !current_item
                        .available_actions
                        .iter()
                        .any(|action| action == "edit_name")
                    {
                        return Err(DbError::Validation(
                            organization_item_action_error("edit_name").to_string(),
                        ));
                    }
                }
                "kept" => {
                    if !current_item
                        .available_actions
                        .iter()
                        .any(|action| action == "keep")
                    {
                        return Err(DbError::Validation(
                            organization_item_action_error("keep").to_string(),
                        ));
                    }
                }
                "undecided" if current_item.decision != "undecided" => {
                    if !current_item
                        .available_actions
                        .iter()
                        .any(|action| action == "clear_decision")
                    {
                        return Err(DbError::Validation(
                            organization_item_action_error("clear_decision").to_string(),
                        ));
                    }
                }
                "undecided" => {}
                _ => unreachable!("normalize_decision returns only durable decisions"),
            }
            if request.safe_batch && decision != "accepted" {
                return Err(DbError::Validation(
                    "organization_safe_batch_accept_only".to_string(),
                ));
            }
            let edited_name = if decision == "edited" {
                Some(validate_edited_filename(
                    &tx,
                    &plan_id,
                    &item_id,
                    mutation.edited_filename.as_deref(),
                )?)
            } else {
                None
            };
            let resolves_target_collision = decision == "edited"
                && matches!(
                    current_item.blocking_code.as_deref(),
                    Some("target_collision" | "organization_target_collision")
                );
            let updated = tx.execute(
                "UPDATE organization_plan_items SET decision = ?1, edited_name = ?2,
                        validity = CASE WHEN ?3 THEN 'needs_review' ELSE validity END,
                        blocking_code = CASE WHEN ?3 THEN NULL ELSE blocking_code END,
                        revision = revision + 1, updated_at = ?4
                 WHERE id = ?5 AND plan_id = ?6 AND revision = ?7
                   AND validity NOT IN ('executing', 'executed')",
                params![
                    decision,
                    edited_name,
                    bool_to_i64(resolves_target_collision),
                    now,
                    item_id,
                    plan_id,
                    mutation.expected_item_revision
                ],
            )?;
            if updated != 1 {
                return Err(DbError::Validation(
                    "organization_item_revision_conflict".to_string(),
                ));
            }
        }
        tx.execute(
            "UPDATE organization_plans SET
                    status = CASE
                        WHEN status = 'stale'
                         AND NOT EXISTS (
                            SELECT 1 FROM organization_plan_items
                            WHERE plan_id = ?1 AND validity = 'stale'
                         )
                        THEN 'ready' ELSE status END,
                    revision = revision + 1, updated_at = ?3
             WHERE id = ?1 AND revision = ?2",
            params![plan_id, request.expected_plan_revision, now],
        )?;
        let plan = load_plan(&tx, &plan_id)?;
        tx.commit()?;
        Ok(plan)
    }

    pub fn refresh_organization_plan(
        &self,
        request: OrganizationPlanRevisionRequest,
    ) -> Result<OrganizationPlanDto, DbError> {
        let plan_id = validate_id(&request.plan_id, "organization_plan_id_invalid")?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        require_plan_revision_and_status(
            &tx,
            &plan_id,
            request.expected_plan_revision,
            &["ready", "stale", "partially_completed"],
        )?;
        let source_query = match load_plan_source_query(&tx, &plan_id) {
            Ok(query) => query,
            Err(_) => {
                mark_plan_scope_stale(
                    &tx,
                    &plan_id,
                    request.expected_plan_revision,
                    "organization_source_provenance_invalid",
                )?;
                let plan = load_plan(&tx, &plan_id)?;
                tx.commit()?;
                return Ok(plan);
            }
        };
        let item_ids = {
            let mut stmt = tx.prepare(
                "SELECT id, file_id_snapshot, proposal_fingerprint, decision
                 FROM organization_plan_items WHERE plan_id = ?1
                   AND validity NOT IN ('executing', 'executed') ORDER BY ordinal, id",
            )?;
            let rows = stmt.query_map(params![plan_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let now = current_unix_seconds();
        let mut any_stale = false;
        for (item_id, file_id, old_fingerprint, decision) in item_ids {
            let row = load_indexed_file_by_id(&tx, &file_id)?;
            let Some(row) = row else {
                any_stale = true;
                tx.execute(
                    "UPDATE organization_plan_items SET validity = 'stale',
                            blocking_code = 'source_missing',
                            blocking_detail = 'The indexed source is no longer available.',
                            revision = revision + 1, updated_at = ?2 WHERE id = ?1",
                    params![item_id, now],
                )?;
                continue;
            };
            match file_matches_authoritative_query_scope(&tx, &source_query, &file_id) {
                Ok(true) => {}
                Ok(false) | Err(_) => {
                    any_stale = true;
                    tx.execute(
                        "UPDATE organization_plan_items SET validity = 'stale',
                                blocking_code = 'managed_scope_unavailable',
                                blocking_detail = 'The source is no longer in the current healthy managed scope.',
                                decision = 'undecided', edited_name = NULL,
                                revision = revision + 1, updated_at = ?2 WHERE id = ?1",
                        params![item_id, now],
                    )?;
                    continue;
                }
            }
            let proposal = current_organization_proposal(&tx, &row)?;
            let changed = proposal.fingerprint != old_fingerprint;
            let validity = if changed && matches!(decision.as_str(), "accepted" | "edited") {
                any_stale = true;
                "needs_review"
            } else {
                proposal.validity.as_str()
            };
            let reset_review = changed && matches!(decision.as_str(), "accepted" | "edited");
            tx.execute(
                "UPDATE organization_plan_items SET proposal_fingerprint = ?2,
                        proposal_kind = ?3, proposed_target_directory = ?4,
                        proposed_name = ?5, proposed_target_path = ?6, validity = ?7,
                        confidence = ?8, risk_level = ?9, requires_confirmation = ?10,
                        blocking_code = ?11, blocking_detail = ?12,
                        authoritative_preview_id = ?13, revision = revision + 1,
                        decision = CASE WHEN ?14 THEN 'undecided' ELSE decision END,
                        edited_name = CASE WHEN ?14 THEN NULL ELSE edited_name END,
                        updated_at = ?15 WHERE id = ?1",
                params![
                    item_id,
                    proposal.fingerprint,
                    proposal.kind,
                    proposal.target_directory,
                    proposal.name,
                    proposal.target_path,
                    validity,
                    proposal.confidence,
                    proposal.risk,
                    i64::from(proposal.requires_confirmation),
                    proposal.blocking_code,
                    proposal.blocking_detail,
                    proposal.preview_id,
                    reset_review,
                    now
                ],
            )?;
        }
        let status = if any_stale { "stale" } else { "ready" };
        tx.execute(
            "UPDATE organization_plans SET status = ?3, revision = revision + 1,
                    updated_at = ?4, last_error_code = NULL, last_error_detail = NULL
             WHERE id = ?1 AND revision = ?2",
            params![plan_id, request.expected_plan_revision, status, now],
        )?;
        let plan = load_plan(&tx, &plan_id)?;
        tx.commit()?;
        Ok(plan)
    }

    pub fn cancel_organization_plan(
        &self,
        request: OrganizationPlanRevisionRequest,
    ) -> Result<OrganizationPlanDto, DbError> {
        let plan_id = validate_id(&request.plan_id, "organization_plan_id_invalid")?;
        let conn = self.conn()?;
        let updated = conn.execute(
            "UPDATE organization_plans SET status = 'cancelled', revision = revision + 1,
                    updated_at = ?3 WHERE id = ?1 AND revision = ?2
                    AND status IN ('draft', 'building', 'ready', 'stale')",
            params![
                plan_id,
                request.expected_plan_revision,
                current_unix_seconds()
            ],
        )?;
        if updated != 1 {
            return Err(DbError::Validation(
                "organization_plan_revision_conflict".to_string(),
            ));
        }
        load_plan(&conn, &plan_id)
    }

    pub fn delete_organization_plan(
        &self,
        request: DeleteOrganizationPlanRequest,
    ) -> Result<bool, DbError> {
        if !request.confirmed {
            return Err(DbError::Validation(
                "organization_plan_delete_confirmation_required".to_string(),
            ));
        }
        let plan_id = validate_id(&request.plan_id, "organization_plan_id_invalid")?;
        let conn = self.conn()?;
        let changed = conn.execute(
            "DELETE FROM organization_plans WHERE id = ?1 AND revision = ?2
             AND status IN ('completed', 'cancelled', 'failed')
             AND active_operation_batch_id IS NULL",
            params![plan_id, request.expected_plan_revision],
        )?;
        if changed != 1 {
            return Err(DbError::Validation(
                "organization_plan_delete_blocked".to_string(),
            ));
        }
        Ok(true)
    }

    pub fn analyze_organization_plan_items(
        &self,
        request: AnalyzeOrganizationPlanItemsRequest,
    ) -> Result<AnalyzeOrganizationPlanItemsResult, DbError> {
        if request.item_ids.len() > 100 {
            return Err(DbError::Validation(
                "organization_ai_batch_too_large".to_string(),
            ));
        }
        let plan_id = validate_id(&request.plan_id, "organization_plan_id_invalid")?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        require_plan_revision_and_status(
            &tx,
            &plan_id,
            request.expected_plan_revision,
            &["ready", "stale", "partially_completed"],
        )?;
        let requested = request
            .item_ids
            .into_iter()
            .map(|id| validate_id(&id, "organization_item_id_invalid"))
            .collect::<Result<HashSet<_>, _>>()?;
        let (candidate_sql, candidate_params) = if requested.is_empty() {
            (
                "SELECT id, file_id_snapshot FROM organization_plan_items
                 WHERE plan_id = ? AND validity = 'needs_analysis' ORDER BY ordinal, id LIMIT 101"
                    .to_string(),
                vec![plan_id.clone()],
            )
        } else {
            (
                format!(
                    "SELECT id, file_id_snapshot FROM organization_plan_items
                     WHERE plan_id = ? AND id IN ({}) ORDER BY ordinal, id LIMIT 101",
                    std::iter::repeat_n("?", requested.len())
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                std::iter::once(plan_id.clone())
                    .chain(requested.iter().cloned())
                    .collect(),
            )
        };
        let mut stmt = tx.prepare(&candidate_sql)?;
        let candidates = stmt
            .query_map(params_from_iter(candidate_params.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        let file_ids = candidates
            .into_iter()
            .take(100)
            .map(|(_, file_id)| file_id)
            .collect::<Vec<_>>();
        let (queued_count, has_eligible_pending_work) =
            crate::global_index::enqueue_managed_ai_for_library_files(&tx, &file_ids)?;
        tx.commit()?;
        if has_eligible_pending_work {
            self.notify_managed_ai_work();
        }
        Ok(AnalyzeOrganizationPlanItemsResult {
            plan_id,
            queued_count: queued_count as i64,
            requires_refresh: true,
        })
    }

    pub fn get_organization_plan_dry_run(
        &self,
        request: OrganizationPlanSelectionRequest,
    ) -> Result<OrganizationPlanDryRunDto, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let dry_run = build_organization_dry_run(&tx, &request)?;
        tx.commit()?;
        Ok(dry_run)
    }

    pub(crate) fn begin_organization_plan_execution(
        &self,
        request: &ExecuteOrganizationPlanRequest,
    ) -> Result<OrganizationExecutionDispatch, DbError> {
        #[cfg(test)]
        let _total_profile = OrganizationExecutionProfileTimer::new("execution.total");
        if !request.confirmed {
            return Err(DbError::Validation(
                "organization_execution_confirmation_required".to_string(),
            ));
        }
        let execution_id = format!("organization-execution-{}", uuid::Uuid::new_v4());
        let operation_batch_id = format!("organization-operation-{}", uuid::Uuid::new_v4());
        let mut conn = {
            #[cfg(test)]
            let _profile = OrganizationExecutionProfileTimer::new("execution.connection_checkout");
            self.conn()?
        };
        let tx = {
            #[cfg(test)]
            let _profile = OrganizationExecutionProfileTimer::new("execution.transaction_begin");
            conn.transaction()?
        };
        {
            #[cfg(test)]
            let _profile = OrganizationExecutionProfileTimer::new("execution.revision_guard");
            require_plan_revision_and_status(
                &tx,
                &request.plan_id,
                request.expected_plan_revision,
                &["ready", "partially_completed"],
            )?;
        }
        let live_dry_run = {
            #[cfg(test)]
            let _profile = OrganizationExecutionProfileTimer::new("execution.live_dry_run");
            build_organization_dry_run(
                &tx,
                &OrganizationPlanSelectionRequest {
                    plan_id: request.plan_id.clone(),
                    expected_plan_revision: request.expected_plan_revision,
                    item_ids: request.item_ids.clone(),
                    all_accepted: request.all_accepted,
                },
            )?
        };
        if live_dry_run.dry_run_fingerprint != request.dry_run_fingerprint {
            return Err(DbError::Validation(
                "organization_dry_run_expired".to_string(),
            ));
        }
        let executable_items = {
            #[cfg(test)]
            let _profile =
                OrganizationExecutionProfileTimer::new("execution.select_executable_items");
            live_dry_run
                .items
                .iter()
                .filter(|item| item.executable)
                .take(ORGANIZATION_EXECUTION_MAX_ITEMS)
                .collect::<Vec<_>>()
        };
        if executable_items.is_empty() {
            return Err(DbError::Validation(
                "organization_execution_no_executable_items".to_string(),
            ));
        }
        let executable_ids = {
            #[cfg(test)]
            let _profile = OrganizationExecutionProfileTimer::new("execution.collect_item_ids");
            executable_items
                .iter()
                .map(|item| item.item_id.clone())
                .collect::<Vec<_>>()
        };
        let now = current_unix_seconds();
        let mut selections = Vec::with_capacity(executable_ids.len());
        for live_item in executable_items {
            let item_id = &live_item.item_id;
            let (file_id, source_name, validity, decision): (String, String, String, String) = {
                #[cfg(test)]
                let _profile =
                    OrganizationExecutionProfileTimer::new("execution.item_snapshot_select");
                tx.query_row(
                    "SELECT file_id_snapshot, source_name_snapshot, validity, decision
                 FROM organization_plan_items WHERE id = ?1 AND plan_id = ?2",
                    params![item_id, request.plan_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )?
            };
            if !matches!(validity.as_str(), "ready" | "needs_review")
                || !matches!(decision.as_str(), "accepted" | "edited")
            {
                return Err(DbError::Validation(
                    "organization_execution_item_changed".to_string(),
                ));
            }
            {
                #[cfg(test)]
                let _profile = OrganizationExecutionProfileTimer::new("execution.selection_build");
                selections.push(crate::file_ops::OperationPreviewRequest {
                    id: live_item.authoritative_preview_id.clone().ok_or_else(|| {
                        DbError::Validation("organization_preview_missing".to_string())
                    })?,
                    file_id,
                    operation_type: live_item.operation_kind.clone(),
                    source_path: live_item.from.clone(),
                    target_path: live_item.to.clone(),
                    old_name: source_name,
                    new_name: std::path::Path::new(&live_item.to)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or_else(|| {
                            DbError::Validation("organization_target_name_invalid".to_string())
                        })?
                        .to_string(),
                    is_executable: Some(true),
                });
            }
            let claimed = {
                #[cfg(test)]
                let _profile =
                    OrganizationExecutionProfileTimer::new("execution.item_claim_update");
                tx.execute(
                    "UPDATE organization_plan_items SET validity = 'executing',
                            execution_id = ?2, revision = revision + 1, updated_at = ?3
                     WHERE id = ?1 AND validity IN ('ready', 'needs_review')",
                    params![item_id, execution_id, now],
                )?
            };
            if claimed != 1 {
                return Err(DbError::Validation(
                    "organization_execution_item_changed".to_string(),
                ));
            }
        }
        let updated = {
            #[cfg(test)]
            let _profile = OrganizationExecutionProfileTimer::new("execution.plan_claim_update");
            tx.execute(
                "UPDATE organization_plans SET status = 'executing',
                        active_execution_id = ?3, active_operation_batch_id = ?4,
                        revision = revision + 1, updated_at = ?5
                 WHERE id = ?1 AND revision = ?2
                   AND status IN ('ready', 'partially_completed')",
                params![
                    request.plan_id,
                    request.expected_plan_revision,
                    execution_id,
                    operation_batch_id,
                    now
                ],
            )?
        };
        if updated != 1 {
            return Err(DbError::Validation(
                "organization_plan_revision_conflict".to_string(),
            ));
        }
        {
            #[cfg(test)]
            let _profile = OrganizationExecutionProfileTimer::new("execution.transaction_commit");
            tx.commit()?;
        }
        Ok(OrganizationExecutionDispatch {
            execution_id,
            operation_batch_id,
            item_ids: executable_ids,
            selections,
        })
    }

    pub(crate) fn finalize_organization_plan_execution(
        &self,
        plan_id: &str,
        dispatch: &OrganizationExecutionDispatch,
        logs: &[crate::file_ops::OperationLogDto],
    ) -> Result<ExecuteOrganizationPlanResultDto, DbError> {
        if dispatch.item_ids.len() != logs.len() {
            return Err(DbError::Validation(
                "organization_execution_log_mapping_invalid".to_string(),
            ));
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let owner: Option<(String, String)> = tx
            .query_row(
                "SELECT active_execution_id, active_operation_batch_id
                 FROM organization_plans WHERE id = ?1 AND status = 'executing'",
                params![plan_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if owner
            != Some((
                dispatch.execution_id.clone(),
                dispatch.operation_batch_id.clone(),
            ))
        {
            return Err(DbError::Validation(
                "organization_execution_owner_mismatch".to_string(),
            ));
        }
        let now = current_unix_seconds();
        let mut succeeded = 0_i64;
        let mut failed = 0_i64;
        let mut skipped = 0_i64;
        for (item_id, log) in dispatch.item_ids.iter().zip(logs.iter()) {
            let validity = match log.status.as_str() {
                "success" => {
                    succeeded += 1;
                    "executed"
                }
                "skipped" => {
                    skipped += 1;
                    "skipped"
                }
                _ => {
                    failed += 1;
                    "failed"
                }
            };
            tx.execute(
                "UPDATE organization_plan_items SET validity = ?2,
                        operation_log_id = ?3, revision = revision + 1, updated_at = ?4
                 WHERE id = ?1 AND execution_id = ?5 AND validity = 'executing'",
                params![item_id, validity, log.id, now, dispatch.execution_id],
            )?;
        }
        let projection = project_organization_plan(&tx, plan_id, false, now)?;
        tx.execute(
            "UPDATE organization_plans SET status = ?2, active_execution_id = NULL,
                    active_operation_batch_id = NULL, revision = revision + 1,
                    updated_at = ?3, completed_at = ?4,
                    last_error_code = CASE WHEN ?5 > 0 THEN 'organization_items_failed' ELSE NULL END,
                    last_error_detail = CASE WHEN ?5 > 0 THEN CAST(?5 AS TEXT) || ' item(s) failed' ELSE NULL END
             WHERE id = ?1 AND active_execution_id = ?6",
            params![
                plan_id,
                projection.status,
                now,
                projection.completed_at,
                failed,
                dispatch.execution_id
            ],
        )?;
        let plan = load_plan(&tx, plan_id)?;
        tx.commit()?;
        Ok(ExecuteOrganizationPlanResultDto {
            plan,
            execution_id: dispatch.execution_id.clone(),
            operation_batch_id: dispatch.operation_batch_id.clone(),
            attempted_count: logs.len() as i64,
            succeeded_count: succeeded,
            failed_count: failed,
            skipped_count: skipped,
        })
    }

    pub(crate) fn fail_unjournaled_organization_execution(
        &self,
        plan_id: &str,
        dispatch: &OrganizationExecutionDispatch,
        error: &str,
    ) -> Result<(), DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let journal_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM operation_logs WHERE batch_id = ?1",
            params![dispatch.operation_batch_id],
            |row| row.get(0),
        )?;
        if journal_count > 0 {
            return Ok(());
        }
        let now = current_unix_seconds();
        tx.execute(
            "UPDATE organization_plan_items SET validity = 'ready', execution_id = NULL,
                    revision = revision + 1, updated_at = ?2
             WHERE plan_id = ?1 AND execution_id = ?3 AND validity = 'executing'",
            params![plan_id, now, dispatch.execution_id],
        )?;
        tx.execute(
            "UPDATE organization_plans SET status = 'failed',
                    active_execution_id = NULL, active_operation_batch_id = NULL,
                    revision = revision + 1, updated_at = ?2,
                    last_error_code = 'organization_execution_dispatch_failed',
                    last_error_detail = ?3
             WHERE id = ?1 AND active_execution_id = ?4",
            params![
                plan_id,
                now,
                error.chars().take(512).collect::<String>(),
                dispatch.execution_id
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[derive(Debug)]
struct OrganizationPlanProjection {
    status: &'static str,
    completed_at: Option<i64>,
}

fn project_organization_plan(
    conn: &rusqlite::Connection,
    plan_id: &str,
    unknown_mapping: bool,
    now: i64,
) -> Result<OrganizationPlanProjection, DbError> {
    if unknown_mapping {
        return Ok(OrganizationPlanProjection {
            status: "stale",
            completed_at: None,
        });
    }
    let (remaining, unresolved, failed_or_skipped): (i64, i64, i64) = conn.query_row(
        "SELECT
            COALESCE(SUM(CASE
                WHEN decision IN ('accepted', 'edited')
                 AND validity IN ('ready', 'needs_review')
                THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN validity = 'executing' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN validity IN ('failed', 'skipped') THEN 1 ELSE 0 END), 0)
         FROM organization_plan_items WHERE plan_id = ?1",
        params![plan_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if unresolved > 0 {
        Ok(OrganizationPlanProjection {
            status: "executing",
            completed_at: None,
        })
    } else if remaining > 0 || failed_or_skipped > 0 {
        Ok(OrganizationPlanProjection {
            status: "partially_completed",
            completed_at: None,
        })
    } else {
        Ok(OrganizationPlanProjection {
            status: "completed",
            completed_at: Some(now),
        })
    }
}

#[derive(Debug)]
struct Proposal {
    fingerprint: String,
    kind: String,
    target_directory: String,
    name: String,
    target_path: String,
    validity: String,
    confidence: f64,
    risk: String,
    requires_confirmation: bool,
    blocking_code: Option<String>,
    blocking_detail: Option<String>,
    preview_id: Option<String>,
}

fn proposal_from_preview(
    source_path: &str,
    source_name: &str,
    classification_status: &str,
    suggested_action: &str,
    preview: Option<OperationPreviewDto>,
) -> Proposal {
    let mut proposal = if matches!(suggested_action, "DeleteCandidate" | "Review") {
        Proposal {
            fingerprint: String::new(),
            kind: "blocked".to_string(),
            target_directory: parent_directory(source_path),
            name: source_name.to_string(),
            target_path: source_path.to_string(),
            validity: "blocked".to_string(),
            confidence: 0.0,
            risk: "Caution".to_string(),
            requires_confirmation: true,
            blocking_code: Some("cleanup_review_required".to_string()),
            blocking_detail: Some(
                "Delete and cleanup candidates must use the existing Cleanup review flow."
                    .to_string(),
            ),
            preview_id: None,
        }
    } else if let Some(preview) = preview {
        // Schema 34 intentionally keeps Organization Plan proposal_kind to
        // move/rename/keep/blocked. Copy, Duplicate and Replace are real
        // Operation Preview capabilities, but they do not get smuggled into
        // the durable Organization Plan enum without an approved schema
        // migration. They remain available from their owning file-operation
        // surface and appear here as a truthful blocked projection.
        let dedicated_file_operation = matches!(
            preview.operation_type.as_str(),
            "copy" | "duplicate" | "replace" | "permanent_delete"
        );
        let validity = if dedicated_file_operation || preview.is_executable != Some(true) {
            "blocked"
        } else if preview.risk_level != "Normal"
            || preview.confidence < 0.8
            || preview.is_duplicate
            || preview.requires_confirmation
            || preview.will_create_parent == Some(true)
        {
            "needs_review"
        } else {
            "ready"
        };
        let blocking_code = if dedicated_file_operation {
            Some("organization_unsupported_operation".to_string())
        } else {
            organization_preview_blocking_code(&preview)
        };
        Proposal {
            fingerprint: String::new(),
            kind: if dedicated_file_operation {
                "blocked".to_string()
            } else {
                preview.operation_type
            },
            target_directory: parent_directory(&preview.target_path),
            name: preview.new_name,
            target_path: preview.target_path,
            validity: validity.to_string(),
            confidence: preview.confidence,
            risk: preview.risk_level,
            requires_confirmation: preview.requires_confirmation,
            blocking_code,
            blocking_detail: preview.blocking_reason,
            preview_id: Some(preview.id),
        }
    } else if classification_status == "unclassified" {
        Proposal {
            fingerprint: String::new(),
            kind: "keep".to_string(),
            target_directory: parent_directory(source_path),
            name: source_name.to_string(),
            target_path: source_path.to_string(),
            validity: "needs_analysis".to_string(),
            confidence: 0.0,
            risk: "Unknown".to_string(),
            requires_confirmation: true,
            blocking_code: None,
            blocking_detail: None,
            preview_id: None,
        }
    } else {
        Proposal {
            fingerprint: String::new(),
            kind: "keep".to_string(),
            target_directory: parent_directory(source_path),
            name: source_name.to_string(),
            target_path: source_path.to_string(),
            validity: "ready".to_string(),
            confidence: 1.0,
            risk: "Normal".to_string(),
            requires_confirmation: false,
            blocking_code: None,
            blocking_detail: None,
            preview_id: None,
        }
    };
    proposal.fingerprint = proposal_fingerprint(&proposal, source_path);
    proposal
}

fn organization_preview_blocking_code(preview: &OperationPreviewDto) -> Option<String> {
    if preview.is_executable == Some(true) {
        return None;
    }
    if preview.editable_new_name == Some(false) {
        return Some("extension_change_blocked".to_string());
    }
    if preview_has_target_collision(preview) {
        return Some("target_collision".to_string());
    }
    if preview.risk_level.eq_ignore_ascii_case("Sensitive") {
        return Some("sensitive_file".to_string());
    }
    Some("unsupported_operation".to_string())
}

fn proposal_fingerprint(proposal: &Proposal, source_path: &str) -> String {
    blake3::hash(
        [
            source_path,
            &proposal.kind,
            &proposal.target_directory,
            &proposal.name,
            &proposal.target_path,
            &proposal.validity,
            &proposal.confidence.to_bits().to_string(),
            &proposal.risk,
            if proposal.requires_confirmation {
                "1"
            } else {
                "0"
            },
            proposal.blocking_code.as_deref().unwrap_or(""),
            proposal.preview_id.as_deref().unwrap_or(""),
        ]
        .join("\0")
        .as_bytes(),
    )
    .to_hex()
    .to_string()
}

fn validate_create_request(request: &CreateOrganizationPlanRequestV1) -> Result<(), DbError> {
    if request.version != ORGANIZATION_PLAN_VERSION
        || request.request_id.trim().is_empty()
        || request.request_id.chars().count() > 128
        || request.title.as_ref().is_some_and(|title| {
            title.chars().count() > 128 || title.chars().any(|c| c.is_control())
        })
        || request.expected_count.is_some_and(|count| count < 0)
    {
        return Err(DbError::Validation(
            "organization_plan_request_invalid".to_string(),
        ));
    }
    Ok(())
}

fn require_safe_batch_item_projection(item: &OrganizationPlanItemDto) -> Result<(), DbError> {
    let target = std::path::Path::new(&item.proposed_target_path);
    let target_parent_exists = target.parent().is_some_and(std::path::Path::exists);
    let collision = normalize_text_for_compare(&item.proposed_target_path)
        != normalize_text_for_compare(&item.source_path_snapshot)
        && target.exists();
    let safe = item
        .available_actions
        .iter()
        .any(|action| action == "accept_suggestion")
        && item.validity == "ready"
        && item.risk_level == "Normal"
        && item.confidence >= 0.8
        && !item.requires_confirmation
        && !paths_cross_volume(&item.source_path_snapshot, &item.proposed_target_path)
        && target_parent_exists
        && !collision
        && item.blocking_code.is_none()
        && item.authoritative_preview_id.is_some();
    if !safe {
        return Err(DbError::Validation(
            "organization_safe_batch_item_blocked".to_string(),
        ));
    }
    Ok(())
}

fn load_organization_plan_item_for_projection(
    conn: &rusqlite::Connection,
    plan_id: &str,
    item_id: &str,
) -> Result<OrganizationPlanItemDto, DbError> {
    let mut item = conn
        .query_row(
            "SELECT id, plan_id, ordinal, file_id_snapshot, source_path_snapshot,
                    source_name_snapshot, source_size_snapshot, source_mtime_snapshot,
                    source_is_dir_snapshot, proposal_fingerprint, proposal_kind,
                    proposed_target_directory, proposed_name, proposed_target_path,
                    decision, edited_name, validity, confidence, risk_level,
                    requires_confirmation, blocking_code, blocking_detail,
                    authoritative_preview_id, operation_log_id, execution_id, revision,
                    created_at, updated_at
             FROM organization_plan_items WHERE plan_id = ?1 AND id = ?2",
            params![plan_id, item_id],
            item_from_row,
        )
        .optional()?
        .ok_or_else(|| DbError::Validation("organization_item_not_found".to_string()))?;
    let scope = load_organization_scope_projection(conn, plan_id);
    let scope_memberships =
        organization_scope_memberships(conn, &scope, std::slice::from_ref(&item.file_id_snapshot));
    let scope_membership = scope_memberships
        .get(&item.file_id_snapshot)
        .copied()
        .unwrap_or(Some(false));
    let current_file = load_indexed_file_by_id(conn, &item.file_id_snapshot)?;
    decorate_organization_item_metadata_with_file(
        conn,
        &mut item,
        current_file.as_ref(),
        scope_membership,
    )?;
    Ok(item)
}

fn require_plan_revision_and_status(
    conn: &rusqlite::Connection,
    plan_id: &str,
    revision: i64,
    allowed_statuses: &[&str],
) -> Result<(), DbError> {
    let status = conn
        .query_row(
            "SELECT status FROM organization_plans WHERE id = ?1 AND revision = ?2",
            params![plan_id, revision],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| DbError::Validation("organization_plan_revision_conflict".to_string()))?;
    if !allowed_statuses.contains(&status.as_str()) {
        return Err(DbError::Validation(
            "organization_plan_status_invalid".to_string(),
        ));
    }
    Ok(())
}

fn load_plan(conn: &rusqlite::Connection, plan_id: &str) -> Result<OrganizationPlanDto, DbError> {
    let mut plan = conn
        .query_row(
            "SELECT id, title, status, source_kind, source_query_fingerprint,
                source_snapshot_revision, requested_count, materialized_count,
                planner_version, revision, active_execution_id,
                active_operation_batch_id, last_error_code, last_error_detail,
                created_at, updated_at, ready_at, completed_at
         FROM organization_plans WHERE id = ?1",
            params![plan_id],
            plan_from_row,
        )
        .optional()?
        .ok_or_else(|| DbError::Validation("organization_plan_not_found".to_string()))?;
    plan.summary = load_plan_summary(conn, plan_id)?;
    Ok(plan)
}

fn plan_from_row(row: &Row<'_>) -> rusqlite::Result<OrganizationPlanDto> {
    Ok(OrganizationPlanDto {
        id: row.get(0)?,
        title: row.get(1)?,
        status: row.get(2)?,
        source_kind: row.get(3)?,
        source_query_fingerprint: row.get(4)?,
        source_snapshot_revision: row.get(5)?,
        requested_count: row.get(6)?,
        materialized_count: row.get(7)?,
        planner_version: row.get(8)?,
        revision: row.get(9)?,
        active_execution_id: row.get(10)?,
        active_operation_batch_id: row.get(11)?,
        last_error_code: row.get(12)?,
        last_error_detail: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
        ready_at: row.get(16)?,
        completed_at: row.get(17)?,
        summary: OrganizationPlanSummaryDto::default(),
        effective_summary: None,
    })
}

fn item_from_row(row: &Row<'_>) -> rusqlite::Result<OrganizationPlanItemDto> {
    let decision = row.get::<_, String>(14)?;
    let validity = row.get::<_, String>(16)?;
    let review_state = match (validity.as_str(), decision.as_str()) {
        ("needs_review", "undecided") => "needs_review",
        ("needs_review", "accepted" | "edited" | "kept") => "reviewed",
        ("ready", _) => "ready",
        ("blocked", _) => "blocked",
        (other, _) => other,
    }
    .to_string();
    Ok(OrganizationPlanItemDto {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        ordinal: row.get(2)?,
        file_id_snapshot: row.get(3)?,
        source_path_snapshot: row.get(4)?,
        source_name_snapshot: row.get(5)?,
        source_size_snapshot: row.get(6)?,
        source_mtime_snapshot: row.get(7)?,
        source_is_dir_snapshot: row.get::<_, i64>(8)? != 0,
        proposal_fingerprint: row.get(9)?,
        semantic_explanation: None,
        proposal_kind: row.get(10)?,
        proposed_target_directory: row.get(11)?,
        proposed_name: row.get(12)?,
        proposed_target_path: row.get(13)?,
        decision,
        edited_name: row.get(15)?,
        validity,
        review_state,
        effective_readiness: "blocked".to_string(),
        confidence: row.get(17)?,
        risk_level: row.get(18)?,
        requires_confirmation: row.get::<_, i64>(19)? != 0,
        blocking_code: row.get(20)?,
        blocking_detail: row.get(21)?,
        authoritative_preview_id: row.get(22)?,
        review_reasons: Vec::new(),
        available_actions: Vec::new(),
        operation_log_id: row.get(23)?,
        execution_id: row.get(24)?,
        revision: row.get(25)?,
        created_at: row.get(26)?,
        updated_at: row.get(27)?,
    })
}

#[derive(Debug)]
enum OrganizationScopeProjection {
    Query(Box<FileQuerySpecV2>),
    Unavailable,
}

fn load_organization_scope_projection(
    conn: &rusqlite::Connection,
    plan_id: &str,
) -> OrganizationScopeProjection {
    match load_plan_source_query(conn, plan_id) {
        Ok(query) => OrganizationScopeProjection::Query(Box::new(query)),
        Err(_) => OrganizationScopeProjection::Unavailable,
    }
}

fn organization_scope_memberships(
    conn: &rusqlite::Connection,
    scope: &OrganizationScopeProjection,
    file_ids: &[String],
) -> HashMap<String, Option<bool>> {
    let mut memberships = HashMap::with_capacity(file_ids.len());
    match scope {
        OrganizationScopeProjection::Query(query) => {
            const SCOPE_ID_CHUNK: usize = 500;
            let mut matching_ids = HashSet::new();
            let mut scope_available = true;
            for chunk in file_ids.chunks(SCOPE_ID_CHUNK) {
                match files_matching_authoritative_query_scope(conn, query.as_ref(), chunk) {
                    Ok(ids) => matching_ids.extend(ids),
                    Err(_) => {
                        scope_available = false;
                        break;
                    }
                }
            }
            for file_id in file_ids {
                memberships.insert(
                    file_id.clone(),
                    Some(scope_available && matching_ids.contains(file_id)),
                );
            }
        }
        OrganizationScopeProjection::Unavailable => {
            for file_id in file_ids {
                memberships.insert(file_id.clone(), Some(false));
            }
        }
    }
    memberships
}

fn decorate_organization_item_metadata_with_file(
    conn: &rusqlite::Connection,
    item: &mut OrganizationPlanItemDto,
    current_file: Option<&IndexedFileRow>,
    managed_scope_membership: Option<bool>,
) -> Result<(), DbError> {
    let live_projection = current_file
        .map(|file| current_organization_projection(conn, file))
        .transpose()?;
    let (live_proposal, preview, semantic_explanation) =
        live_projection.map_or((None, None, None), |projection| {
            (
                Some(projection.proposal),
                projection.preview,
                projection.semantic_explanation,
            )
        });
    let live_proposal = live_proposal.as_ref();
    let live_proposal_changed = live_proposal
        .as_ref()
        .is_some_and(|proposal| proposal.fingerprint != item.proposal_fingerprint);
    item.semantic_explanation = if live_proposal_changed {
        None
    } else {
        semantic_explanation
    };
    let mut reasons = Vec::new();
    let blocking_code = item.blocking_code.as_deref().unwrap_or("");
    let source_unchanged = current_file.is_some_and(|file| {
        normalize_text_for_compare(&file.path)
            == normalize_text_for_compare(&item.source_path_snapshot)
            && file.size == item.source_size_snapshot
            && file.mtime == item.source_mtime_snapshot
            && file.is_dir == item.source_is_dir_snapshot
    });

    if matches!(
        item.validity.as_str(),
        "needs_review" | "blocked" | "stale" | "failed" | "skipped"
    ) {
        if item.confidence < 0.8 {
            push_review_reason(&mut reasons, OrganizationReviewReason::LowConfidence);
        }
        if item.risk_level.eq_ignore_ascii_case("Sensitive") {
            push_review_reason(&mut reasons, OrganizationReviewReason::SensitiveFile);
        }
        if !item.risk_level.trim().is_empty() && !item.risk_level.eq_ignore_ascii_case("Normal") {
            push_review_reason(&mut reasons, OrganizationReviewReason::NonNormalRisk);
        }
        if item.requires_confirmation {
            push_review_reason(&mut reasons, OrganizationReviewReason::RequiresConfirmation);
        }
    }

    if let Some(preview) = preview.as_ref() {
        if preview.is_duplicate {
            push_review_reason(&mut reasons, OrganizationReviewReason::PossibleDuplicate);
        }
        if preview.will_create_parent == Some(true) {
            push_review_reason(
                &mut reasons,
                OrganizationReviewReason::TargetDirectoryCreation,
            );
        }
        for reason in preview_review_reasons(preview) {
            push_review_reason(&mut reasons, reason);
        }
    }

    if let Some(reason) = review_reason_from_blocking_code(blocking_code) {
        push_review_reason(&mut reasons, reason);
    }

    if !source_unchanged || current_file.is_none() {
        push_review_reason(&mut reasons, OrganizationReviewReason::SourceChanged);
    }
    if managed_scope_membership == Some(false) {
        push_review_reason(&mut reasons, OrganizationReviewReason::ManagedScopeChanged);
    }
    if live_proposal_changed {
        push_review_reason(&mut reasons, OrganizationReviewReason::ProposalChanged);
    }
    if item.validity == "stale" && reasons.is_empty() {
        push_review_reason(&mut reasons, OrganizationReviewReason::SourceChanged);
    }
    if item.proposal_kind != "keep"
        && (item.authoritative_preview_id.is_none() || preview.is_none())
    {
        push_review_reason(&mut reasons, OrganizationReviewReason::MissingPreview);
    }
    if item.proposal_kind != "keep"
        && item.authoritative_preview_id.is_some()
        && preview.as_ref().is_some_and(|current| {
            Some(current.id.as_str()) != item.authoritative_preview_id.as_deref()
        })
    {
        push_review_reason(&mut reasons, OrganizationReviewReason::ProposalChanged);
    }
    if !matches!(
        item.proposal_kind.as_str(),
        "move" | "rename" | "move_rename" | "keep"
    ) {
        push_review_reason(&mut reasons, OrganizationReviewReason::UnsupportedOperation);
    }
    if item.validity == "needs_review" && reasons.is_empty() {
        push_review_reason(&mut reasons, OrganizationReviewReason::RequiresConfirmation);
    }
    if item.validity == "blocked" && reasons.is_empty() {
        push_review_reason(&mut reasons, OrganizationReviewReason::UnsupportedOperation);
    }

    let terminal = matches!(item.validity.as_str(), "executing" | "executed");
    let supported_operation = matches!(
        item.proposal_kind.as_str(),
        "move" | "rename" | "move_rename"
    );
    let preview_available = item
        .authoritative_preview_id
        .as_deref()
        .zip(preview.as_ref())
        .is_some_and(|(expected_id, current)| expected_id == current.id);
    let preview_is_executable = preview
        .as_ref()
        .is_some_and(|current| current.is_executable == Some(true));
    let preview_is_editable = preview
        .as_ref()
        .is_some_and(|current| current.editable_new_name == Some(true));
    let hard_blocked = !source_unchanged
        || current_file.is_none()
        || managed_scope_membership == Some(false)
        || live_proposal_changed
        || item.blocking_code.as_ref().is_some_and(|code| {
            matches!(
                code.as_str(),
                "source_identity_changed"
                    | "source_missing"
                    | "managed_scope_unavailable"
                    | "managed_scope_membership_changed"
                    | "live_proposal_changed"
                    | "proposal_changed"
                    | "organization_edited_filename_invalid"
                    | "organization_unsupported_operation"
                    | "cleanup_review_required"
            )
        });
    let reviewable_validity = matches!(item.validity.as_str(), "ready" | "needs_review");
    let editable_collision = item.validity == "blocked"
        && reasons
            .iter()
            .any(|reason| reason == OrganizationReviewReason::TargetCollision.code());
    let can_edit = !terminal
        && preview_available
        && supported_operation
        && preview_is_editable
        && !hard_blocked
        && (reviewable_validity || editable_collision);
    let can_accept = !terminal
        && preview_available
        && preview_is_executable
        && supported_operation
        && reviewable_validity
        && !hard_blocked
        && item.blocking_code.is_none()
        && !item.risk_level.eq_ignore_ascii_case("Sensitive")
        && item.decision == "undecided";
    let mut actions = Vec::new();
    if !terminal {
        if can_accept {
            actions.push("accept_suggestion".to_string());
        }
        if can_edit {
            actions.push("edit_name".to_string());
            actions.push("view_preview".to_string());
        }
        actions.push("keep".to_string());
        if item.decision != "undecided" {
            actions.push("clear_decision".to_string());
        }
        if item.validity == "needs_review" && item.decision == "undecided" {
            actions.push("defer".to_string());
        }
    }

    item.review_reasons = reasons;
    item.available_actions = actions;
    item.effective_readiness = organization_effective_readiness(
        item,
        OrganizationEffectiveReadinessFacts {
            source_present: current_file.is_some(),
            source_unchanged,
            managed_scope_membership,
            live_proposal_changed,
            terminal,
            supported_operation,
            preview_available,
            preview_is_executable,
            can_edit,
        },
    )
    .to_string();
    Ok(())
}

struct OrganizationEffectiveReadinessFacts {
    source_present: bool,
    source_unchanged: bool,
    managed_scope_membership: Option<bool>,
    live_proposal_changed: bool,
    terminal: bool,
    supported_operation: bool,
    preview_available: bool,
    preview_is_executable: bool,
    can_edit: bool,
}

fn organization_effective_readiness(
    item: &OrganizationPlanItemDto,
    facts: OrganizationEffectiveReadinessFacts,
) -> &'static str {
    if facts.terminal
        || !facts.source_present
        || !facts.source_unchanged
        || facts.managed_scope_membership == Some(false)
        || facts.live_proposal_changed
        || (item.blocking_code.is_some()
            && !(facts.can_edit
                && item.blocking_code.as_deref().is_some_and(|code| {
                    matches!(code, "target_collision" | "organization_target_collision")
                })))
        || !matches!(item.validity.as_str(), "ready" | "needs_review")
        || (!facts.supported_operation && item.proposal_kind != "keep")
        || (item.proposal_kind != "keep" && !facts.preview_available)
        || (item.proposal_kind != "keep" && !facts.preview_is_executable && !facts.can_edit)
    {
        return "blocked";
    }
    if item.validity == "needs_review" {
        if item.decision == "undecided"
            && item.available_actions.iter().any(|action| {
                matches!(
                    action.as_str(),
                    "accept_suggestion" | "edit_name" | "keep" | "defer"
                )
            })
        {
            return "requires-decision";
        }
        if matches!(item.decision.as_str(), "accepted" | "edited" | "kept") {
            return "reviewed";
        }
        return "blocked";
    }
    if item.proposal_kind == "keep" || facts.preview_is_executable {
        return "ready";
    }
    "blocked"
}

fn push_review_reason(reasons: &mut Vec<String>, reason: OrganizationReviewReason) {
    let code = reason.code();
    if !reasons.iter().any(|current| current == code) {
        reasons.push(code.to_string());
    }
}

fn review_reason_from_blocking_code(value: &str) -> Option<OrganizationReviewReason> {
    match value {
        "source_identity_changed" | "source_missing" => {
            Some(OrganizationReviewReason::SourceChanged)
        }
        "managed_scope_unavailable" | "managed_scope_membership_changed" => {
            Some(OrganizationReviewReason::ManagedScopeChanged)
        }
        "live_proposal_changed" | "proposal_changed" => {
            Some(OrganizationReviewReason::ProposalChanged)
        }
        "organization_target_collision" | "target_collision" => {
            Some(OrganizationReviewReason::TargetCollision)
        }
        "organization_edited_filename_invalid" | "unsafe_filename" => {
            Some(OrganizationReviewReason::UnsafeFilename)
        }
        "extension_change_blocked" => Some(OrganizationReviewReason::ExtensionChangeBlocked),
        "organization_unsupported_operation" | "cleanup_review_required" => {
            Some(OrganizationReviewReason::UnsupportedOperation)
        }
        _ => None,
    }
}

fn preview_review_reasons(preview: &OperationPreviewDto) -> Vec<OrganizationReviewReason> {
    let mut reasons = Vec::new();
    if preview.editable_new_name == Some(false) {
        reasons.push(OrganizationReviewReason::ExtensionChangeBlocked);
    }
    if preview_has_target_collision(preview) {
        reasons.push(OrganizationReviewReason::TargetCollision);
    }
    if preview.is_executable == Some(false) && preview.risk_level.eq_ignore_ascii_case("Sensitive")
    {
        reasons.push(OrganizationReviewReason::SensitiveFile);
    }
    if preview.is_executable != Some(true)
        && !reasons.iter().any(|reason| {
            matches!(
                reason,
                OrganizationReviewReason::ExtensionChangeBlocked
                    | OrganizationReviewReason::TargetCollision
                    | OrganizationReviewReason::SensitiveFile
            )
        })
    {
        reasons.push(OrganizationReviewReason::UnsupportedOperation);
    }
    reasons
}

fn preview_has_target_collision(preview: &OperationPreviewDto) -> bool {
    preview.is_executable == Some(false)
        && preview.editable_new_name == Some(true)
        && preview.target_parent_exists == Some(true)
        && preview.will_create_parent == Some(false)
        && !preview.risk_level.eq_ignore_ascii_case("Sensitive")
        && matches!(
            preview.operation_type.as_str(),
            "move" | "rename" | "move_rename"
        )
        && normalize_text_for_compare(&preview.source_path)
            != normalize_text_for_compare(&preview.target_path)
}

fn load_plan_summary(
    conn: &rusqlite::Connection,
    plan_id: &str,
) -> Result<OrganizationPlanSummaryDto, DbError> {
    conn.query_row(
        "SELECT
            COALESCE(SUM(CASE WHEN decision = 'undecided' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN decision = 'accepted' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN decision = 'kept' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN decision = 'edited' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN validity = 'needs_analysis' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN validity = 'needs_review' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE
                WHEN validity = 'needs_review' AND decision = 'undecided'
                THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE
                WHEN validity = 'needs_review'
                 AND decision IN ('accepted', 'edited', 'kept')
                THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN validity = 'ready' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN validity = 'blocked' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN validity = 'stale' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN validity = 'executing' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN validity = 'executed' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN validity = 'failed' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN validity = 'skipped' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE
                WHEN decision IN ('accepted', 'edited')
                 AND validity IN ('ready', 'needs_review')
                THEN 1 ELSE 0 END), 0)
         FROM organization_plan_items WHERE plan_id = ?1",
        params![plan_id],
        |row| {
            Ok(OrganizationPlanSummaryDto {
                undecided: row.get(0)?,
                accepted: row.get(1)?,
                kept: row.get(2)?,
                edited: row.get(3)?,
                needs_analysis: row.get(4)?,
                needs_review: row.get(5)?,
                pending_review: row.get(6)?,
                reviewed: row.get(7)?,
                ready: row.get(8)?,
                blocked: row.get(9)?,
                stale: row.get(10)?,
                executing: row.get(11)?,
                executed: row.get(12)?,
                failed: row.get(13)?,
                skipped: row.get(14)?,
                remaining_executable: row.get(15)?,
            })
        },
    )
    .map_err(DbError::from)
}

fn load_indexed_file_by_id(
    conn: &rusqlite::Connection,
    file_id: &str,
) -> Result<Option<IndexedFileRow>, DbError> {
    conn.query_row(
        "SELECT f.id, f.path, f.name, f.extension, f.size, f.mtime, f.ctime, f.is_dir, f.state_code,
                f.file_type, f.purpose, f.lifecycle, f.context, f.risk_level, f.suggested_action,
                f.suggested_target_path, f.suggested_name, f.confidence, f.classification_reason,
                f.classification_status, f.matched_rules, f.requires_confirmation, f.content_hash,
                EXISTS (SELECT 1 FROM active_duplicate_membership AS membership
                        WHERE membership.file_id = f.id),
                f.is_stale, f.last_seen_at, f.last_classified_at, f.classified_rule_version,
                f.last_classified_mtime, f.last_classified_size
         FROM files AS f WHERE f.id = ?1 AND f.is_stale = 0",
        params![file_id],
        indexed_file_from_row,
    )
    .optional()
    .map_err(DbError::from)
}

fn load_plan_source_query(
    conn: &rusqlite::Connection,
    plan_id: &str,
) -> Result<FileQuerySpecV2, DbError> {
    let (source_kind, source_query_json, stored_fingerprint): (
        String,
        Option<String>,
        Option<String>,
    ) = conn.query_row(
        "SELECT source_kind, source_query_spec_json, source_query_fingerprint
         FROM organization_plans WHERE id = ?1",
        params![plan_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if source_kind == "explicit" {
        return Ok(FileQuerySpecV2 {
            scope: FileLibraryScopeV2::AllEnabledRoots,
            text: None,
            filters: FileQueryFiltersV2::default(),
            sort: FileLibrarySortV2::default(),
        });
    }
    if source_kind != "all_matching" {
        return Err(DbError::Validation(
            "organization_source_provenance_invalid".to_string(),
        ));
    }
    let raw = source_query_json
        .ok_or_else(|| DbError::Validation("organization_source_provenance_invalid".to_string()))?;
    let query: FileQuerySpecV2 = serde_json::from_str(&raw)?;
    let (query, _, fingerprint) = canonicalize_file_query_spec(query)?;
    if stored_fingerprint.as_deref() != Some(fingerprint.as_str()) {
        return Err(DbError::Validation(
            "organization_source_provenance_invalid".to_string(),
        ));
    }
    Ok(query)
}

fn mark_plan_scope_stale(
    conn: &rusqlite::Connection,
    plan_id: &str,
    expected_revision: i64,
    code: &str,
) -> Result<(), DbError> {
    let now = current_unix_seconds();
    conn.execute(
        "UPDATE organization_plan_items SET validity = 'stale',
                blocking_code = ?2,
                blocking_detail = 'The plan source is no longer a valid healthy managed scope.',
                decision = 'undecided', edited_name = NULL,
                revision = revision + 1, updated_at = ?3
         WHERE plan_id = ?1 AND validity NOT IN ('executing', 'executed')",
        params![plan_id, code, now],
    )?;
    let changed = conn.execute(
        "UPDATE organization_plans SET status = 'stale', revision = revision + 1,
                updated_at = ?4, last_error_code = ?3,
                last_error_detail = 'Managed source provenance or root authority is unavailable.'
         WHERE id = ?1 AND revision = ?2",
        params![plan_id, expected_revision, code, now],
    )?;
    if changed != 1 {
        return Err(DbError::Validation(
            "organization_plan_revision_conflict".to_string(),
        ));
    }
    Ok(())
}

fn build_organization_dry_run(
    conn: &rusqlite::Connection,
    request: &OrganizationPlanSelectionRequest,
) -> Result<OrganizationPlanDryRunDto, DbError> {
    let plan_id = {
        #[cfg(test)]
        let _profile = OrganizationExecutionProfileTimer::new("dry_run.validate_plan_id");
        validate_id(&request.plan_id, "organization_plan_id_invalid")?
    };
    {
        #[cfg(test)]
        let _profile = OrganizationExecutionProfileTimer::new("dry_run.revision_status_query");
        require_plan_revision_and_status(
            conn,
            &plan_id,
            request.expected_plan_revision,
            &["ready", "partially_completed"],
        )?;
    }
    let source_query = {
        #[cfg(test)]
        let _profile = OrganizationExecutionProfileTimer::new("dry_run.source_query_load");
        load_plan_source_query(conn, &plan_id)?
    };
    let selected = {
        #[cfg(test)]
        let _profile = OrganizationExecutionProfileTimer::new("dry_run.selected_items_query");
        selected_plan_items(conn, request)?
    };
    let file_ids = selected
        .iter()
        .map(|item| item.file_id_snapshot.clone())
        .collect::<Vec<_>>();
    let current_files = {
        #[cfg(test)]
        let _profile = OrganizationExecutionProfileTimer::new("dry_run.current_files_batch_load");
        load_indexed_files_for_projection(conn, &file_ids)?
    };
    let scope_memberships = {
        const SCOPE_ID_CHUNK: usize = 500;
        let mut memberships = HashMap::with_capacity(file_ids.len());
        for chunk in file_ids.chunks(SCOPE_ID_CHUNK) {
            let matching_ids = {
                #[cfg(test)]
                let _profile =
                    OrganizationExecutionProfileTimer::new("dry_run.scope_membership_query");
                files_matching_authoritative_query_scope(conn, &source_query, chunk)
            };
            match matching_ids {
                Ok(matching_ids) => {
                    for file_id in chunk {
                        memberships.insert(file_id.clone(), Ok(matching_ids.contains(file_id)));
                    }
                }
                Err(_) => {
                    for file_id in chunk {
                        memberships.insert(file_id.clone(), Err(()));
                    }
                }
            }
        }
        memberships
    };
    let mut items = Vec::with_capacity(selected.len());
    let mut kinds = HashSet::new();
    let mut total_bytes = 0_i64;
    let mut executable_count = 0_i64;
    let mut blocked_count = 0_i64;
    let mut stale_count = 0_i64;
    let mut fingerprint_parts = vec![plan_id.clone(), request.expected_plan_revision.to_string()];

    for item in selected {
        let current = current_files.get(&item.file_id_snapshot).cloned();
        let mut source_health = "healthy";
        let mut blocking_code = item.blocking_code.clone();
        let mut live_proposal = None;
        let mut final_target = item.proposed_target_path.clone();
        let mut collision = false;
        let mut invalid_filename = false;
        let mut classification_inputs = Vec::new();

        if let Some(row) = current.as_ref() {
            let source_unchanged = normalize_text_for_compare(&row.path)
                == normalize_text_for_compare(&item.source_path_snapshot)
                && row.size == item.source_size_snapshot
                && row.mtime == item.source_mtime_snapshot
                && row.is_dir == item.source_is_dir_snapshot;
            if !source_unchanged {
                source_health = "stale";
                blocking_code = Some("source_identity_changed".to_string());
            }
            let scope_match = scope_memberships
                .get(&item.file_id_snapshot)
                .copied()
                .unwrap_or(Err(()));
            match scope_match {
                Ok(true) => {}
                Ok(false) => {
                    source_health = "invalid_scope";
                    blocking_code = Some("managed_scope_membership_changed".to_string());
                }
                Err(_) => {
                    source_health = "invalid_scope";
                    blocking_code = Some("managed_scope_unavailable".to_string());
                }
            }
            classification_inputs.extend([
                row.file_type.clone(),
                row.purpose.clone(),
                row.lifecycle.clone(),
                row.context.clone(),
                row.risk_level.clone(),
                row.suggested_action.clone(),
                row.suggested_target_path.clone(),
                row.suggested_name.clone(),
                row.confidence.to_bits().to_string(),
                row.classification_reason.clone(),
                row.classification_status.clone(),
                row.matched_rules.clone(),
                row.requires_confirmation.to_string(),
                row.content_hash.clone(),
                row.ctime.to_string(),
                row.last_classified_at.to_string(),
                row.classified_rule_version.clone(),
                row.last_classified_mtime.to_string(),
                row.last_classified_size.to_string(),
            ]);
            let proposal = {
                #[cfg(test)]
                let _profile =
                    OrganizationExecutionProfileTimer::new("dry_run.current_proposal_query");
                current_organization_proposal(conn, row)?
            };
            if proposal.fingerprint != item.proposal_fingerprint {
                source_health = "stale";
                blocking_code = Some("live_proposal_changed".to_string());
            }
            final_target = if let Some(edited) = item.edited_name.as_deref() {
                if crate::file_ops::validate_safe_file_name(edited).is_err() {
                    invalid_filename = true;
                    blocking_code = Some("organization_edited_filename_invalid".to_string());
                }
                std::path::Path::new(&proposal.target_directory)
                    .join(edited)
                    .to_string_lossy()
                    .to_string()
            } else {
                proposal.target_path.clone()
            };
            collision = {
                #[cfg(test)]
                let _profile = OrganizationExecutionProfileTimer::new(
                    "dry_run.target_collision_filesystem_check",
                );
                normalize_text_for_compare(&final_target) != normalize_text_for_compare(&row.path)
                    && std::path::Path::new(&final_target).exists()
            };
            if collision {
                blocking_code = Some("organization_target_collision".to_string());
            }
            let edited_collision_resolved = item.edited_name.is_some()
                && proposal.blocking_code.as_deref() == Some("target_collision")
                && !collision
                && !invalid_filename;
            if edited_collision_resolved
                && matches!(
                    blocking_code.as_deref(),
                    Some("target_collision" | "organization_target_collision")
                )
            {
                blocking_code = None;
            }
            live_proposal = Some(proposal);
        } else {
            source_health = "missing";
            blocking_code = Some("source_missing".to_string());
        }

        let (operation_kind, risk_level, requires_confirmation, preview_id, mut live_validity) =
            live_proposal.as_ref().map_or_else(
                || {
                    (
                        item.proposal_kind.clone(),
                        item.risk_level.clone(),
                        item.requires_confirmation,
                        None,
                        "stale".to_string(),
                    )
                },
                |proposal| {
                    (
                        proposal.kind.clone(),
                        proposal.risk.clone(),
                        proposal.requires_confirmation,
                        proposal.preview_id.clone(),
                        proposal.validity.clone(),
                    )
                },
            );
        if item.edited_name.is_some()
            && live_proposal.as_ref().is_some_and(|proposal| {
                proposal.blocking_code.as_deref() == Some("target_collision")
            })
            && !collision
            && !invalid_filename
        {
            live_validity = "needs_review".to_string();
        }
        let explicitly_reviewed = matches!(item.decision.as_str(), "accepted" | "edited")
            && matches!(item.validity.as_str(), "ready" | "needs_review");
        let supported_operation =
            matches!(operation_kind.as_str(), "move" | "rename" | "move_rename");
        let executable = explicitly_reviewed
            && source_health == "healthy"
            && matches!(live_validity.as_str(), "ready" | "needs_review")
            && supported_operation
            && preview_id.is_some()
            && !collision
            && !invalid_filename
            && blocking_code.is_none();
        if executable {
            executable_count += 1;
            total_bytes = total_bytes.saturating_add(item.source_size_snapshot);
            kinds.insert(operation_kind.clone());
        } else {
            blocked_count += 1;
            if matches!(source_health, "stale" | "missing" | "invalid_scope") {
                stale_count += 1;
            }
        }
        let parent_directory_to_create = {
            #[cfg(test)]
            let _profile =
                OrganizationExecutionProfileTimer::new("dry_run.parent_directory_filesystem_check");
            std::path::Path::new(&final_target)
                .parent()
                .filter(|path| !path.exists())
                .map(|path| path.to_string_lossy().to_string())
        };
        let cross_volume = paths_cross_volume(&item.source_path_snapshot, &final_target);
        #[cfg(test)]
        let _fingerprint_profile =
            OrganizationExecutionProfileTimer::new("dry_run.fingerprint_and_result_build");
        let canonical_item_fingerprint = blake3::hash(
            [
                vec![
                    item.id.clone(),
                    item.file_id_snapshot.clone(),
                    item.revision.to_string(),
                    item.decision.clone(),
                    item.edited_name.clone().unwrap_or_default(),
                    item.proposal_fingerprint.clone(),
                    source_health.to_string(),
                    operation_kind.clone(),
                    final_target.clone(),
                    risk_level.clone(),
                    requires_confirmation.to_string(),
                    collision.to_string(),
                    cross_volume.to_string(),
                    preview_id.clone().unwrap_or_default(),
                    current
                        .as_ref()
                        .map(|row| row.path.clone())
                        .unwrap_or_default(),
                    current
                        .as_ref()
                        .map(|row| row.size.to_string())
                        .unwrap_or_default(),
                    current
                        .as_ref()
                        .map(|row| row.mtime.to_string())
                        .unwrap_or_default(),
                    current
                        .as_ref()
                        .map(|row| row.is_dir.to_string())
                        .unwrap_or_default(),
                ],
                classification_inputs,
            ]
            .concat()
            .join("\0")
            .as_bytes(),
        )
        .to_hex()
        .to_string();
        fingerprint_parts.push(canonical_item_fingerprint);
        items.push(OrganizationDryRunItemDto {
            item_id: item.id,
            operation_kind,
            from: current
                .as_ref()
                .map(|row| row.path.clone())
                .unwrap_or(item.source_path_snapshot),
            to: final_target,
            edited_filename: item.edited_name,
            parent_directory_to_create,
            collision,
            cross_volume,
            risk_level,
            requires_confirmation,
            source_health: source_health.to_string(),
            authoritative_preview_id: preview_id,
            executable,
            blocking_code,
        });
    }

    let dry_run_fingerprint = blake3::hash(fingerprint_parts.join("\0").as_bytes())
        .to_hex()
        .to_string();
    let mut operation_kinds = kinds.into_iter().collect::<Vec<_>>();
    operation_kinds.sort();
    Ok(OrganizationPlanDryRunDto {
        plan_id,
        plan_revision: request.expected_plan_revision,
        selected_count: items.len() as i64,
        executable_count,
        blocked_count,
        stale_count,
        total_bytes,
        operation_kinds,
        items,
        execution_batch_limit: ORGANIZATION_EXECUTION_MAX_ITEMS,
        dry_run_fingerprint,
    })
}

fn selected_plan_items(
    conn: &rusqlite::Connection,
    request: &OrganizationPlanSelectionRequest,
) -> Result<Vec<OrganizationPlanItemDto>, DbError> {
    if request.all_accepted != request.item_ids.is_empty() {
        return Err(DbError::Validation(
            "organization_selection_invalid".to_string(),
        ));
    }
    let sql = "SELECT id, plan_id, ordinal, file_id_snapshot, source_path_snapshot,
            source_name_snapshot, source_size_snapshot, source_mtime_snapshot,
            source_is_dir_snapshot, proposal_fingerprint, proposal_kind,
            proposed_target_directory, proposed_name, proposed_target_path,
            decision, edited_name, validity, confidence, risk_level,
            requires_confirmation, blocking_code, blocking_detail,
            authoritative_preview_id, operation_log_id, execution_id, revision,
            created_at, updated_at
         FROM organization_plan_items WHERE plan_id = ?1
           AND decision IN ('accepted', 'edited')"
        .to_string();
    let requested = request.item_ids.iter().cloned().collect::<HashSet<_>>();
    let mut items = {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![request.plan_id], item_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    if !request.all_accepted {
        items.retain(|item| requested.contains(&item.id));
        if items.len() != requested.len() {
            return Err(DbError::Validation(
                "organization_selection_item_invalid".to_string(),
            ));
        }
    }
    items.sort_by_key(|item| (item.ordinal, item.id.clone()));
    Ok(items)
}

fn paths_cross_volume(left: &str, right: &str) -> bool {
    fn volume(path: &str) -> String {
        let normalized = path.replace('\\', "/");
        normalized
            .split('/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
    }
    volume(left) != volume(right)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn test_database() -> (Database, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "zen-canvas-organization-test-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        (
            Database::open(&path).expect("organization test database"),
            path,
        )
    }

    pub(crate) fn seed_current_managed_semantics_for_file(db: &Database, file_id: &str) {
        let row = {
            let conn = db.conn().expect("semantic fixture source connection");
            load_indexed_file_by_id(&conn, file_id)
                .expect("load semantic fixture source")
                .expect("semantic fixture source exists")
        };
        let settings = crate::ai::settings::AISettings {
            enabled: true,
            ..Default::default()
        };
        let settings_json =
            serde_json::to_string(&settings).expect("serialize semantic fixture AI settings");
        let entry_id = format!("organization-test-global-{file_id}");
        let platform_file_id = format!("organization-test-platform-{file_id}");
        let managed_entry_id = format!("organization-test-managed-{file_id}");
        let job_id = format!("organization-test-ai-job-{file_id}");
        let job_item_id = format!("organization-test-ai-item-{file_id}");
        let input_fingerprint = crate::global_index::managed_worker::metadata_fingerprint(
            "organization-test-semantic-volume",
            &platform_file_id,
            &row.name,
            row.size,
            Some(row.mtime),
            row.is_dir,
        );
        let binding = crate::ai::semantic::SemanticSourceBinding {
            global_entry_id: entry_id.clone(),
            managed_scope_id: "organization-test-semantic-scope".to_string(),
            input_fingerprint: input_fingerprint.clone(),
            provider: "cloud".to_string(),
        };
        let confidence = if row.confidence > 0.0 {
            row.confidence
        } else {
            0.95
        };
        let requires_confirmation = row.requires_confirmation || confidence < 0.8;
        let suggested_name = if row.suggested_name.trim().is_empty() {
            row.name.clone()
        } else {
            row.suggested_name.clone()
        };
        let provider_response = serde_json::json!({
            "version": 1,
            "refId": format!("managed:{entry_id}"),
            "fileType": "Document",
            "purpose": "Work",
            "lifecycle": "Active",
            "context": "organization test fixture",
            "riskLevel": "Normal",
            "suggestedAction": "Rename",
            "suggestedName": suggested_name,
            "confidence": confidence,
            "reason": "current semantic organization test fixture",
            "keywords": [],
            "requiresConfirmation": requires_confirmation
        })
        .to_string();
        let assessment = crate::ai::semantic::SemanticAssessmentV1::parse_provider_response(
            &provider_response,
            binding,
            &row.name,
            &row.extension,
            row.is_dir,
        )
        .expect("build semantic organization test assessment");
        let canonical = assessment
            .canonical_json()
            .expect("encode semantic organization test assessment");

        let mut conn = db.conn().expect("semantic fixture publish connection");
        let tx = conn
            .transaction()
            .expect("semantic fixture publish transaction");
        tx.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![crate::ai::settings::AI_SETTINGS_KEY, settings_json],
        )
        .expect("enable semantic fixture AI settings");
        tx.execute(
            "INSERT OR IGNORE INTO global_volumes (
                id, platform, stable_volume_id, display_name, mount_path,
                filesystem_type, drive_kind, enabled, provider, index_status,
                entry_count, created_at, updated_at
             ) VALUES (
                'organization-test-semantic-volume', 'test',
                'organization-test-semantic-stable-volume', 'Organization test semantics',
                '/', 'test', 'fixed', 1, 'test', 'ready', 0, 1, 1
             )",
            [],
        )
        .expect("seed semantic fixture global volume");
        tx.execute(
            "INSERT OR IGNORE INTO managed_scopes (
                id, path, global_entry_id, enabled,
                allow_local_ai, allow_cloud_ai, created_at, updated_at
             ) VALUES (
                'organization-test-semantic-scope', '/', NULL, 1, 0, 1, 1, 1
             )",
            [],
        )
        .expect("seed semantic fixture managed scope");
        tx.execute(
            "INSERT INTO global_entries (
                id, volume_id, platform_file_id, parent_platform_file_id,
                name, name_normalized, path, path_normalized, extension,
                is_directory, size, created_at_fs, modified_at_fs,
                file_attributes, is_hidden, is_system, is_stale,
                source_provider, last_seen_at
             ) VALUES (
                ?1, 'organization-test-semantic-volume', ?2, '',
                ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10,
                0, 0, 0, 0, 'test', 1
             )",
            params![
                entry_id,
                platform_file_id,
                row.name,
                row.name.to_ascii_lowercase(),
                row.path,
                crate::global_index::models::normalize_path(&row.path),
                row.extension,
                i64::from(row.is_dir),
                row.size,
                row.mtime,
            ],
        )
        .expect("seed semantic fixture global entry");
        tx.execute(
            "INSERT INTO managed_entries (
                id, global_entry_id, managed_scope_id, enabled, created_at, updated_at
             ) VALUES (?1, ?2, 'organization-test-semantic-scope', 1, 1, 1)",
            params![managed_entry_id, entry_id],
        )
        .expect("seed semantic fixture managed entry");
        tx.execute(
            "INSERT INTO ai_analysis_state (
                global_entry_id, status, input_fingerprint, provider, model,
                content_summary, classification_json, user_corrected, updated_at
             ) VALUES (?1, 'completed', ?2, 'cloud', ?3, 'metadata_only', ?4, 0, 1)",
            params![entry_id, input_fingerprint, settings.model, canonical],
        )
        .expect("seed semantic fixture AI state");
        tx.execute(
            "INSERT INTO ai_jobs (
                id, global_entry_id, managed_scope_id, input_fingerprint,
                provider, model, processing_mode, status, attempt_count,
                created_at, started_at, completed_at
             ) VALUES (
                ?1, ?2, 'organization-test-semantic-scope', ?3,
                'cloud', ?4, 'metadata', 'completed', 1, 1, 1, 1
             )",
            params![job_id, entry_id, input_fingerprint, settings.model],
        )
        .expect("seed semantic fixture completed AI job");
        tx.execute(
            "INSERT INTO ai_job_items (
                id, job_id, global_entry_id, status, created_at, updated_at
             ) VALUES (?1, ?2, ?3, 'completed', 1, 1)",
            params![job_item_id, job_id, entry_id],
        )
        .expect("seed semantic fixture completed AI job item");
        tx.commit()
            .expect("publish semantic organization test fixture");
    }

    fn replace_current_managed_semantic_rename(db: &Database, file_id: &str, suggested_name: &str) {
        let row = {
            let conn = db.conn().expect("semantic replacement source connection");
            load_indexed_file_by_id(&conn, file_id)
                .expect("load semantic replacement source")
                .expect("semantic replacement source exists")
        };
        let settings = {
            let conn = db.conn().expect("semantic replacement settings connection");
            let settings_json: String = conn
                .query_row(
                    "SELECT value FROM app_settings WHERE key = ?1",
                    params![crate::ai::settings::AI_SETTINGS_KEY],
                    |db_row| db_row.get(0),
                )
                .expect("semantic replacement AI settings exist");
            serde_json::from_str::<crate::ai::settings::AISettings>(&settings_json)
                .expect("decode semantic replacement AI settings")
        };
        let entry_id = format!("organization-test-global-{file_id}");
        let platform_file_id = format!("organization-test-platform-{file_id}");
        let input_fingerprint = crate::global_index::managed_worker::metadata_fingerprint(
            "organization-test-semantic-volume",
            &platform_file_id,
            &row.name,
            row.size,
            Some(row.mtime),
            row.is_dir,
        );
        let binding = crate::ai::semantic::SemanticSourceBinding {
            global_entry_id: entry_id.clone(),
            managed_scope_id: "organization-test-semantic-scope".to_string(),
            input_fingerprint,
            provider: "cloud".to_string(),
        };
        let provider_response = serde_json::json!({
            "version": 1,
            "refId": format!("managed:{entry_id}"),
            "fileType": "Document",
            "purpose": "Work",
            "lifecycle": "Active",
            "context": "organization test fixture",
            "riskLevel": "Normal",
            "suggestedAction": "Rename",
            "suggestedName": suggested_name,
            "confidence": 0.95,
            "reason": "current semantic organization test fixture",
            "keywords": [],
            "requiresConfirmation": false
        })
        .to_string();
        let assessment = crate::ai::semantic::SemanticAssessmentV1::parse_provider_response(
            &provider_response,
            binding,
            &row.name,
            &row.extension,
            row.is_dir,
        )
        .expect("build replacement semantic assessment");
        let canonical = assessment
            .canonical_json()
            .expect("encode replacement semantic assessment");
        let conn = db.conn().expect("semantic replacement publish connection");
        conn.execute(
            "UPDATE ai_analysis_state
             SET classification_json = ?2, model = ?3, status = 'completed'
             WHERE global_entry_id = ?1",
            params![entry_id, canonical, settings.model],
        )
        .expect("replace current semantic assessment");
    }

    fn reset_full_projection_count() {
        ORGANIZATION_FULL_PROJECTION_COUNT.with(|count| count.set(0));
    }

    fn full_projection_count() -> usize {
        ORGANIZATION_FULL_PROJECTION_COUNT.with(Cell::get)
    }

    fn seed_plan(db: &Database, status: &str) {
        let conn = db.conn().expect("seed connection");
        conn.execute(
            "INSERT OR IGNORE INTO scan_roots (
                id, normalized_path, display_name, source_kind, enabled,
                health_status, current_generation, needs_reconciliation,
                created_at, updated_at
             ) VALUES ('organization-test-root', '/tmp', 'Organization test',
                       'file_library', 1, 'healthy', 1, 0, 1, 1)",
            [],
        )
        .expect("seed organization managed root");
        conn.execute(
            "INSERT INTO organization_plans (
                id, title, status, source_kind, source_snapshot_revision,
                requested_count, materialized_count, planner_version, revision,
                created_at, updated_at, ready_at
             ) VALUES ('plan-test', 'Test plan', ?1, 'explicit', 1, 1, 1, 1, 1, 1, 1, 1)",
            params![status],
        )
        .expect("seed plan");
        conn.execute(
            "INSERT INTO organization_plan_items (
                id, plan_id, ordinal, file_id_snapshot, source_path_snapshot,
                source_name_snapshot, source_size_snapshot, source_mtime_snapshot,
                source_is_dir_snapshot, proposal_fingerprint, proposal_kind,
                proposed_target_directory, proposed_name, proposed_target_path,
                decision, validity, confidence, risk_level, requires_confirmation,
                authoritative_preview_id, revision, created_at, updated_at
             ) VALUES (
                'item-test', 'plan-test', 0, 'file-test', '/tmp/source.txt',
                'source.txt', 1, 1, 0, 'proposal-fingerprint', 'rename',
                '/tmp', 'renamed.txt', '/tmp/renamed.txt', 'undecided', 'ready',
                0.95, 'Normal', 0, 'preview-test', 1, 1, 1
             )",
            [],
        )
        .expect("seed plan item");
        conn.execute(
            "INSERT INTO files (
                id, path, name, extension, size, mtime, is_dir, state_code,
                suggested_action, suggested_target_path, suggested_name,
                confidence, risk_level, requires_confirmation
             ) VALUES (
                'file-test', '/tmp/source.txt', 'source.txt', 'txt', 1, 1, 0, 0,
                'Rename', '/tmp', 'renamed.txt', 0.95, 'Normal', 0
             )",
            [],
        )
        .expect("seed indexed organization file");
        let preview_id = operation_preview_from_indexed(
            load_indexed_file_by_id(&conn, "file-test")
                .expect("load seeded organization file")
                .expect("seeded organization file exists"),
        )
        .expect("seed organization preview")
        .id;
        conn.execute(
            "UPDATE organization_plan_items SET authoritative_preview_id = ?1
             WHERE id = 'item-test'",
            params![preview_id],
        )
        .expect("bind seeded organization preview");
        let file = load_indexed_file_by_id(&conn, "file-test")
            .expect("reload seeded organization file")
            .expect("seeded organization file remains available");
        let proposal = proposal_from_preview(
            &file.path,
            &file.name,
            &file.classification_status,
            &file.suggested_action,
            operation_preview_from_indexed(file.clone()),
        );
        conn.execute(
            "UPDATE organization_plan_items SET proposal_fingerprint = ?1
             WHERE id = 'item-test'",
            params![proposal.fingerprint],
        )
        .expect("bind seeded organization fingerprint");
    }

    #[allow(clippy::too_many_arguments)]
    fn seed_group_item(
        db: &Database,
        id: &str,
        ordinal: i64,
        target_directory: &std::path::Path,
        validity: &str,
        decision: &str,
        confidence: f64,
        requires_confirmation: bool,
        blocking_code: Option<&str>,
    ) {
        let source_path = target_directory.join(format!("source-{id}.txt"));
        let target_path = target_directory.join(format!("target-{id}.txt"));
        let source_path = source_path.to_string_lossy().replace('\\', "/");
        let target_path = target_path.to_string_lossy().replace('\\', "/");
        let target_directory = target_directory.to_string_lossy().replace('\\', "/");
        let conn = db.conn().expect("group item connection");
        conn.execute(
            "INSERT OR IGNORE INTO scan_roots (
                id, normalized_path, display_name, source_kind, enabled,
                health_status, current_generation, needs_reconciliation,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, 'file_library', 1, 'healthy', 1, 0, 1, 1)",
            params![
                format!("organization-test-root-{id}"),
                target_directory,
                format!("Organization test {id}")
            ],
        )
        .expect("seed group managed root");
        conn.execute(
            "INSERT INTO organization_plan_items (
                id, plan_id, ordinal, file_id_snapshot, source_path_snapshot,
                source_name_snapshot, source_size_snapshot, source_mtime_snapshot,
                source_is_dir_snapshot, proposal_fingerprint, proposal_kind,
                proposed_target_directory, proposed_name, proposed_target_path,
                decision, validity, confidence, risk_level, requires_confirmation,
                blocking_code, authoritative_preview_id, revision, created_at, updated_at
             ) VALUES (?1, 'plan-test', ?2, ?3, ?4, ?5, 10, 1, 0, ?6,
                       'rename', ?7, ?8, ?9, ?10, ?11, ?12, 'Normal', ?13,
                       ?14, ?15, 1, 1, 1)",
            params![
                id,
                ordinal,
                format!("file-{id}"),
                source_path,
                format!("source-{id}.txt"),
                format!("fingerprint-{id}"),
                target_directory,
                format!("target-{id}.txt"),
                target_path,
                decision,
                validity,
                confidence,
                bool_to_i64(requires_confirmation),
                blocking_code,
                if validity == "ready" {
                    Some(format!("preview-{id}"))
                } else {
                    None
                }
            ],
        )
        .expect("seed group item");
    }

    fn sync_item_proposal_fingerprint(db: &Database, item_id: &str, file_id: &str) {
        seed_current_managed_semantics_for_file(db, file_id);
        let conn = db.conn().expect("sync proposal connection");
        let file = load_indexed_file_by_id(&conn, file_id)
            .expect("load sync proposal file")
            .expect("sync proposal file exists");
        let projection = current_organization_projection(&conn, &file)
            .expect("resolve current semantic sync proposal");
        let preview_id = projection
            .preview
            .as_ref()
            .map(|current| current.id.clone());
        conn.execute(
            "UPDATE organization_plan_items SET proposal_fingerprint = ?1,
                    authoritative_preview_id = ?2 WHERE id = ?3",
            params![projection.proposal.fingerprint, preview_id, item_id],
        )
        .expect("sync semantic proposal fingerprint");
    }

    fn seed_live_group_item(
        db: &Database,
        id: &str,
        ordinal: i64,
        target_directory: &std::path::Path,
    ) {
        seed_group_item(
            db,
            id,
            ordinal,
            target_directory,
            "ready",
            "undecided",
            0.95,
            false,
            None,
        );
        let source_path = target_directory.join(format!("source-{id}.txt"));
        let target_directory = target_directory.to_string_lossy().replace('\\', "/");
        let conn = db.conn().expect("live group file connection");
        conn.execute(
            "INSERT INTO files (
                id, path, name, extension, size, mtime, is_dir, state_code,
                suggested_action, suggested_target_path, suggested_name,
                confidence, risk_level, requires_confirmation,
                classification_status, last_classified_mtime, last_classified_size
             ) VALUES (?1, ?2, ?3, 'txt', 10, 1, 0, 0, 'Rename', ?4, ?5,
                       0.95, 'Normal', 0, 'classified', 1, 10)",
            params![
                format!("file-{id}"),
                source_path.to_string_lossy().replace('\\', "/"),
                format!("source-{id}.txt"),
                target_directory,
                format!("target-{id}.txt"),
            ],
        )
        .expect("seed live group file");
        sync_item_proposal_fingerprint(db, id, &format!("file-{id}"));
    }

    #[test]
    fn item_cursor_round_trip_is_bounded() {
        let cursor = OrganizationItemCursor {
            ordinal: 42,
            id: "organization-item-test".to_string(),
        };
        assert_eq!(
            decode_item_cursor(&encode_item_cursor(&cursor))
                .unwrap()
                .ordinal,
            42
        );
        assert!(decode_item_cursor("not-hex").is_err());
    }

    #[test]
    fn organization_group_projection_is_complete_deterministic_and_cursored() {
        let (db, path) = test_database();
        seed_plan(&db, "ready");
        let fixture = path.with_extension("groups");
        let first_target = fixture.join("first");
        let second_target = fixture.join("second");
        std::fs::create_dir_all(&first_target).expect("create first group target");
        std::fs::create_dir_all(&second_target).expect("create second group target");
        {
            let conn = db.conn().expect("group fixture connection");
            conn.execute(
                "UPDATE organization_plan_items SET source_path_snapshot = ?1,
                    proposed_target_directory = ?2, proposed_target_path = ?3
                 WHERE id = 'item-test'",
                params![
                    first_target
                        .join("source-item-test.txt")
                        .to_string_lossy()
                        .replace('\\', "/"),
                    first_target.to_string_lossy().replace('\\', "/"),
                    first_target
                        .join("target-item-test.txt")
                        .to_string_lossy()
                        .replace('\\', "/"),
                ],
            )
            .expect("bind first group item");
            conn.execute(
                "UPDATE files SET path = ?1, name = 'source-item-test.txt',
                        suggested_action = 'Rename', suggested_target_path = ?2,
                        suggested_name = 'target-item-test.txt', classification_status = 'classified',
                        confidence = 0.9, risk_level = 'Normal', requires_confirmation = 0,
                        last_classified_mtime = 1, last_classified_size = 1
                 WHERE id = 'file-test'",
                params![
                    first_target
                        .join("source-item-test.txt")
                        .to_string_lossy()
                        .replace('\\', "/"),
                    first_target.to_string_lossy().replace('\\', "/"),
                ],
            )
            .expect("bind first group indexed file");
        }
        seed_group_item(
            &db,
            "item-group-1",
            1,
            &first_target,
            "ready",
            "undecided",
            0.9,
            false,
            None,
        );
        seed_group_item(
            &db,
            "item-group-2",
            2,
            &first_target,
            "ready",
            "undecided",
            0.7,
            false,
            None,
        );
        seed_group_item(
            &db,
            "item-group-3",
            3,
            &second_target,
            "ready",
            "undecided",
            0.9,
            false,
            None,
        );
        {
            let conn = db.conn().expect("group indexed files connection");
            for (id, target, confidence) in [
                ("1", &first_target, 0.9_f64),
                ("2", &first_target, 0.7_f64),
                ("3", &second_target, 0.9_f64),
            ] {
                let source = target.join(format!("source-item-group-{id}.txt"));
                let target_text = target.to_string_lossy().replace('\\', "/");
                conn.execute(
                    "INSERT INTO files (
                        id, path, name, extension, size, mtime,
                        suggested_action, suggested_target_path, suggested_name,
                        confidence, risk_level, requires_confirmation,
                        classification_status, last_classified_mtime, last_classified_size
                     ) VALUES (?1, ?2, ?3, 'txt', 10, 1, 'Rename', ?4, ?5,
                               ?6, 'Normal', 0, 'classified', 1, 10)",
                    params![
                        format!("file-item-group-{id}"),
                        source.to_string_lossy().replace('\\', "/"),
                        format!("source-item-group-{id}.txt"),
                        target_text,
                        format!("target-item-group-{id}.txt"),
                        confidence,
                    ],
                )
                .expect("seed group indexed file");
            }
        }
        sync_item_proposal_fingerprint(&db, "item-test", "file-test");
        sync_item_proposal_fingerprint(&db, "item-group-1", "file-item-group-1");
        sync_item_proposal_fingerprint(&db, "item-group-2", "file-item-group-2");
        sync_item_proposal_fingerprint(&db, "item-group-3", "file-item-group-3");

        let first_page = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 1,
            })
            .expect("first group page");
        assert!(first_page.has_more);
        let second_page = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: first_page.next_cursor.clone(),
                page_size: 1,
            })
            .expect("second group page");
        assert_eq!(
            first_page.projection_fingerprint,
            second_page.projection_fingerprint
        );
        assert_ne!(
            first_page.groups[0].group_id,
            second_page.groups[0].group_id
        );

        let all_groups = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("complete group projection");
        let first_group = all_groups
            .groups
            .iter()
            .find(|group| group.item_count == 3)
            .expect("three-item group");
        assert_eq!(first_group.total_bytes, 21);
        assert_eq!(
            first_group.sample_items.len(),
            ORGANIZATION_GROUP_SAMPLE_MAX
        );
        assert_eq!(first_group.confidence_band, "mixed");
        assert_eq!(first_group.revision, 1);
        assert!(!first_group.projection_fingerprint.is_empty());
        assert!(first_group.group_actions.can_accept_all);
        assert!(first_group.group_actions.can_keep_all);
        assert!(!first_group.group_actions.can_clear_all);
        let repeated = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("repeat group projection");
        assert_eq!(
            all_groups
                .groups
                .iter()
                .map(|group| group.group_id.clone())
                .collect::<Vec<_>>(),
            repeated
                .groups
                .iter()
                .map(|group| group.group_id.clone())
                .collect::<Vec<_>>()
        );

        let group_page = db
            .query_organization_plan_group_items(QueryOrganizationPlanGroupItemsRequest {
                plan_id: "plan-test".into(),
                group_id: first_group.group_id.clone(),
                cursor: None,
                expected_projection_fingerprint: first_group.projection_fingerprint.clone(),
                page_size: 1,
            })
            .expect("group item page");
        assert_eq!(
            group_page.projection_fingerprint,
            first_group.projection_fingerprint
        );
        assert!(group_page.has_more);
        let next_group_page = db
            .query_organization_plan_group_items(QueryOrganizationPlanGroupItemsRequest {
                plan_id: "plan-test".into(),
                group_id: first_group.group_id.clone(),
                cursor: group_page.next_cursor,
                expected_projection_fingerprint: first_group.projection_fingerprint.clone(),
                page_size: 10,
            })
            .expect("next group item page");
        assert_eq!(next_group_page.items.len(), 2);
        assert_eq!(
            next_group_page.projection_fingerprint,
            first_group.projection_fingerprint
        );
        drop(db);
        let _ = std::fs::remove_dir_all(fixture);
        let _ = std::fs::remove_file(path);
    }

    fn seed_group_pagination_fixture(db: &Database, path: &std::path::Path) -> std::path::PathBuf {
        seed_plan(db, "ready");
        let fixture = path.with_extension("group-pagination");
        let second_target = fixture.join("second");
        std::fs::create_dir_all(&second_target).expect("create group pagination target");
        seed_live_group_item(db, "item-pagination-1", 1, &second_target);
        seed_live_group_item(db, "item-pagination-2", 2, &second_target);
        fixture
    }

    #[test]
    fn organization_group_cursor_rejects_changed_complete_projection_and_group_item_projection() {
        let (db, path) = test_database();
        let fixture = seed_group_pagination_fixture(&db, &path);
        let first_page = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 1,
            })
            .expect("stable group cursor first page");
        let old_cursor = first_page
            .next_cursor
            .clone()
            .expect("group pagination cursor");
        let second_page = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: Some(old_cursor.clone()),
                page_size: 1,
            })
            .expect("stable group cursor second page");
        assert_eq!(
            first_page.projection_fingerprint,
            second_page.projection_fingerprint
        );
        assert_ne!(
            first_page.groups[0].group_id,
            second_page.groups[0].group_id
        );

        let complete_before = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("complete group projection before mutation");
        let item_group = complete_before
            .groups
            .iter()
            .find(|group| group.item_count == 2)
            .expect("two-item group");
        let old_group_fingerprint = item_group.projection_fingerprint.clone();
        let item_page = db
            .query_organization_plan_group_items(QueryOrganizationPlanGroupItemsRequest {
                plan_id: "plan-test".into(),
                group_id: item_group.group_id.clone(),
                cursor: None,
                expected_projection_fingerprint: old_group_fingerprint.clone(),
                page_size: 1,
            })
            .expect("stable group item first page");
        let old_item_cursor = item_page.next_cursor.clone().expect("group item cursor");

        {
            let conn = db.conn().expect("reorder groups without plan revision");
            conn.execute(
                "UPDATE organization_plan_items SET proposed_target_directory = '/tmp/changed'
                 WHERE id = 'item-test'",
                [],
            )
            .expect("reorder and replace group without plan revision");
        }

        let top_error = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: Some(old_cursor),
                page_size: 1,
            })
            .expect_err("old top-level cursor must reject changed projection");
        assert!(top_error
            .to_string()
            .contains("organization_group_projection_changed"));

        let complete_after = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("new first page after projection change");
        assert_eq!(complete_before.groups.len(), complete_after.groups.len());
        assert_ne!(
            complete_before.projection_fingerprint,
            complete_after.projection_fingerprint
        );
        assert_ne!(
            complete_before
                .groups
                .iter()
                .map(|group| group.group_id.clone())
                .collect::<Vec<_>>(),
            complete_after
                .groups
                .iter()
                .map(|group| group.group_id.clone())
                .collect::<Vec<_>>()
        );

        {
            let conn = db
                .conn()
                .expect("change indexed metadata without plan revision");
            conn.execute(
                "UPDATE files SET size = size + 1, mtime = mtime + 1
                 WHERE id = 'file-item-pagination-1'",
                [],
            )
            .expect("change indexed metadata");
        }

        let item_error = db
            .query_organization_plan_group_items(QueryOrganizationPlanGroupItemsRequest {
                plan_id: "plan-test".into(),
                group_id: item_group.group_id.clone(),
                cursor: Some(old_item_cursor),
                expected_projection_fingerprint: old_group_fingerprint,
                page_size: 1,
            })
            .expect_err("old group-item cursor must reject changed projection");
        assert!(item_error
            .to_string()
            .contains("organization_group_projection_changed"));

        let malformed = encode_group_cursor(&OrganizationGroupCursor {
            version: ORGANIZATION_GROUP_CURSOR_VERSION + 1,
            projection_fingerprint: complete_after.projection_fingerprint,
            label: "label".into(),
            group_id: "group-id".into(),
        });
        assert!(decode_group_cursor(&malformed)
            .expect_err("unsupported cursor version must fail closed")
            .to_string()
            .contains("organization_group_cursor_invalid"));

        drop(db);
        let _ = std::fs::remove_dir_all(fixture);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn group_projection_fingerprint_rejects_changed_item_without_partial_update() {
        let (db, path) = test_database();
        seed_plan(&db, "ready");
        let group = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("fingerprint group projection")
            .groups
            .into_iter()
            .next()
            .expect("fingerprint group");
        {
            let conn = db.conn().expect("change fingerprint item");
            conn.execute(
                "UPDATE organization_plan_items SET revision = revision + 1
                 WHERE id = 'item-test'",
                [],
            )
            .expect("change item revision without plan revision");
        }
        let error = db
            .update_organization_plan_group_decision(UpdateOrganizationPlanGroupDecisionRequest {
                plan_id: "plan-test".into(),
                group_id: group.group_id,
                expected_plan_revision: 1,
                expected_projection_fingerprint: group.projection_fingerprint,
                expected_item_count: group.item_count,
                decision: "kept".into(),
            })
            .expect_err("stale projection must be rejected");
        assert!(error.to_string().contains("organization_group_changed"));
        assert_eq!(
            db.get_organization_plan("plan-test")
                .expect("unchanged plan")
                .revision,
            1
        );
        assert_eq!(
            db.query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("unchanged item after stale projection")
            .items
            .into_iter()
            .filter(|item| item.decision != "undecided")
            .count(),
            0
        );
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn group_projection_fingerprint_rejects_live_source_change_without_partial_update() {
        let (db, path) = test_database();
        seed_plan(&db, "ready");
        let group = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("live source fingerprint group")
            .groups
            .into_iter()
            .next()
            .expect("live source fingerprint group exists");
        {
            let conn = db.conn().expect("change live source metadata connection");
            conn.execute(
                "UPDATE files SET size = size + 1, mtime = mtime + 1 WHERE id = 'file-test'",
                [],
            )
            .expect("change live source metadata without plan revision");
        }
        let changed_group = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("changed live source group")
            .groups
            .into_iter()
            .next()
            .expect("changed live source group exists");
        assert_ne!(
            group.projection_fingerprint,
            changed_group.projection_fingerprint
        );
        let error = db
            .update_organization_plan_group_decision(UpdateOrganizationPlanGroupDecisionRequest {
                plan_id: "plan-test".into(),
                group_id: group.group_id,
                expected_plan_revision: 1,
                expected_projection_fingerprint: group.projection_fingerprint,
                expected_item_count: group.item_count,
                decision: "kept".into(),
            })
            .expect_err("live source change must reject stale projection");
        assert!(error.to_string().contains("organization_group_changed"));
        assert_eq!(
            db.get_organization_plan("plan-test")
                .expect("unchanged live source plan")
                .revision,
            1
        );
        assert_eq!(
            db.query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("unchanged items after live source change")
            .items
            .into_iter()
            .filter(|item| item.decision != "undecided")
            .count(),
            0
        );
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn group_projection_fingerprint_rejects_member_join_without_partial_update() {
        let (db, path) = test_database();
        seed_plan(&db, "ready");
        sync_item_proposal_fingerprint(&db, "item-test", "file-test");
        let group = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("member join baseline group")
            .groups
            .into_iter()
            .next()
            .expect("member join baseline group exists");
        seed_live_group_item(&db, "item-joined", 1, std::path::Path::new("/tmp"));
        let joined_group = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("member join projection")
            .groups
            .into_iter()
            .find(|candidate| candidate.item_count == 2)
            .expect("joined member group");
        assert_eq!(joined_group.item_count, group.item_count + 1);
        assert_ne!(
            joined_group.projection_fingerprint,
            group.projection_fingerprint
        );
        let error = db
            .update_organization_plan_group_decision(UpdateOrganizationPlanGroupDecisionRequest {
                plan_id: "plan-test".into(),
                group_id: group.group_id,
                expected_plan_revision: 1,
                expected_projection_fingerprint: group.projection_fingerprint,
                expected_item_count: group.item_count,
                decision: "kept".into(),
            })
            .expect_err("member join must reject stale projection");
        assert!(error.to_string().contains("organization_group_changed"));
        assert_eq!(
            db.query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("unchanged items after member join")
            .items
            .into_iter()
            .filter(|item| item.decision != "undecided")
            .count(),
            0
        );
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn group_projection_fingerprint_rejects_member_migration_without_partial_update() {
        let (db, path) = test_database();
        seed_plan(&db, "ready");
        sync_item_proposal_fingerprint(&db, "item-test", "file-test");
        seed_live_group_item(&db, "item-migrated", 1, std::path::Path::new("/tmp"));
        let group = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("member migration baseline projection")
            .groups
            .into_iter()
            .find(|candidate| candidate.item_count == 2)
            .expect("member migration baseline group");
        {
            let conn = db.conn().expect("remove migrated member file");
            conn.execute("DELETE FROM files WHERE id = 'file-item-migrated'", [])
                .expect("migrate member out of ready group");
        }
        let error = db
            .update_organization_plan_group_decision(UpdateOrganizationPlanGroupDecisionRequest {
                plan_id: "plan-test".into(),
                group_id: group.group_id,
                expected_plan_revision: 1,
                expected_projection_fingerprint: group.projection_fingerprint,
                expected_item_count: group.item_count,
                decision: "kept".into(),
            })
            .expect_err("member migration must reject stale projection");
        assert!(error.to_string().contains("organization_group_changed"));
        assert_eq!(
            db.get_organization_plan("plan-test")
                .expect("unchanged member migration plan")
                .revision,
            1
        );
        assert_eq!(
            db.query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("unchanged items after member migration")
            .items
            .into_iter()
            .filter(|item| item.decision != "undecided")
            .count(),
            0
        );
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn plan_list_and_open_plan_do_not_duplicate_full_group_projection() {
        let (db, path) = test_database();
        seed_plan(&db, "ready");
        {
            let conn = db.conn().expect("projection count seed connection");
            for index in 0..200_i64 {
                conn.execute(
                    "INSERT INTO organization_plans (
                        id, title, status, source_kind, source_snapshot_revision,
                        requested_count, materialized_count, planner_version, revision,
                        created_at, updated_at, ready_at
                     ) VALUES (?1, ?2, 'ready', 'explicit', 1, 0, 0, 1, 1, ?3, ?3, ?3)",
                    params![
                        format!("projection-plan-{index}"),
                        format!("Projection {index}"),
                        index
                    ],
                )
                .expect("seed lightweight projection plan");
            }
        }
        reset_full_projection_count();
        let plans = db.list_organization_plans().expect("lightweight plan list");
        assert_eq!(full_projection_count(), 0);
        assert!(plans.iter().all(|plan| plan.effective_summary.is_none()));
        let plan = db
            .get_organization_plan("plan-test")
            .expect("lightweight open plan");
        assert!(plan.effective_summary.is_none());
        assert_eq!(full_projection_count(), 0);
        let page = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("single full group projection");
        assert_eq!(full_projection_count(), 1);
        assert!(!page.groups.is_empty());
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn group_mutation_handles_one_thousand_members_in_one_transaction() {
        use std::time::Instant;

        let (db, path) = test_database();
        seed_plan(&db, "ready");
        let fixture = path.with_extension("group-1000");
        let target_directory = fixture.to_string_lossy().replace('\\', "/");
        {
            let mut conn = db.conn().expect("large group seed connection");
            let tx = conn.transaction().expect("large group seed transaction");
            tx.execute(
                "UPDATE organization_plan_items SET source_path_snapshot = ?1,
                        proposed_target_directory = ?2, proposed_target_path = ?3
                 WHERE id = 'item-test'",
                params![
                    format!("{target_directory}/missing-seed.txt"),
                    target_directory,
                    format!("{target_directory}/target-item-test.txt")
                ],
            )
            .expect("bind large group seed item");
            for ordinal in 1..1_000_i64 {
                let id = format!("large-group-item-{ordinal:04}");
                let source_path = format!("{target_directory}/missing-source-{ordinal:04}.txt");
                let target_path = format!("{target_directory}/missing-target-{ordinal:04}.txt");
                tx.execute(
                    "INSERT INTO organization_plan_items (
                        id, plan_id, ordinal, file_id_snapshot, source_path_snapshot,
                        source_name_snapshot, source_size_snapshot, source_mtime_snapshot,
                        source_is_dir_snapshot, proposal_fingerprint, proposal_kind,
                        proposed_target_directory, proposed_name, proposed_target_path,
                        decision, validity, confidence, risk_level, requires_confirmation,
                        authoritative_preview_id, revision, created_at, updated_at
                     ) VALUES (?1, 'plan-test', ?2, ?3, ?4, ?5, 1, 1, 0, ?6,
                               'rename', ?7, ?8, ?9, 'undecided', 'ready', 0.95,
                               'Normal', 0, ?10, 1, 1, 1)",
                    params![
                        id,
                        ordinal,
                        format!("large-group-file-{ordinal:04}"),
                        source_path,
                        format!("missing-source-{ordinal:04}.txt"),
                        format!("large-group-fingerprint-{ordinal:04}"),
                        target_directory,
                        format!("missing-target-{ordinal:04}.txt"),
                        target_path,
                        format!("large-group-preview-{ordinal:04}"),
                    ],
                )
                .expect("insert large group member");
            }
            tx.commit().expect("publish large group seed");
        }
        let projected_groups = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("large group projection");
        let group = projected_groups
            .groups
            .into_iter()
            .find(|candidate| candidate.item_count == 1_000)
            .expect("one thousand member group");
        let expected_projection_fingerprint = group.projection_fingerprint.clone();
        let expected_item_count = group.item_count;
        let start = Instant::now();
        let updated = db
            .update_organization_plan_group_decision(UpdateOrganizationPlanGroupDecisionRequest {
                plan_id: "plan-test".into(),
                group_id: group.group_id,
                expected_plan_revision: 1,
                expected_projection_fingerprint,
                expected_item_count,
                decision: "kept".into(),
            })
            .expect("large group keep mutation");
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert!(elapsed_ms <= 5_000.0, "1k group mutation {elapsed_ms:.3}ms");
        assert_eq!(updated.plan.summary.kept, 1_000);
        let kept: i64 = db
            .conn()
            .expect("large group result connection")
            .query_row(
                "SELECT COUNT(*) FROM organization_plan_items
                 WHERE plan_id = 'plan-test' AND decision = 'kept'",
                [],
                |row| row.get(0),
            )
            .expect("count large group result");
        assert_eq!(kept, 1_000);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn review_metadata_is_projected_and_requires_decision_uses_ordinary_mutation() {
        let (db, path) = test_database();
        seed_plan(&db, "ready");
        let fixture = path.with_extension("review-metadata");
        std::fs::create_dir_all(&fixture).expect("create review metadata target");
        std::fs::write(
            fixture.join("target-item-review-metadata.txt"),
            b"collision",
        )
        .expect("seed review metadata collision");
        seed_group_item(
            &db,
            "item-review-metadata",
            1,
            &fixture,
            "needs_review",
            "undecided",
            0.7,
            true,
            Some("target_collision"),
        );
        {
            let conn = db.conn().expect("review metadata connection");
            conn.execute(
                "INSERT INTO files (id, path, name, extension, size, mtime)
                 VALUES ('file-item-review-metadata', ?1, 'source-item-review-metadata.txt', 'txt', 10, 1)",
                params![fixture
                    .join("source-item-review-metadata.txt")
                    .to_string_lossy()
                    .replace('\\', "/")],
            )
            .expect("seed review metadata indexed file");
            conn.execute(
                "UPDATE files SET classification_status = 'classified',
                        suggested_action = 'Rename', suggested_target_path = ?1,
                        suggested_name = 'target-item-review-metadata.txt',
                        confidence = 0.7, risk_level = 'Normal',
                        last_classified_mtime = 1, last_classified_size = 10
                 WHERE id = 'file-item-review-metadata'",
                params![fixture.to_string_lossy().replace('\\', "/")],
            )
            .expect("seed review metadata proposal");
            let preview_id = operation_preview_from_indexed(
                load_indexed_file_by_id(&conn, "file-item-review-metadata")
                    .expect("load review metadata file")
                    .expect("review metadata file exists"),
            )
            .expect("review metadata preview")
            .id;
            conn.execute(
                "UPDATE organization_plan_items
                 SET authoritative_preview_id = ?1
                 WHERE id = 'item-review-metadata'",
                params![preview_id],
            )
            .expect("bind review metadata preview");
        }
        sync_item_proposal_fingerprint(&db, "item-review-metadata", "file-item-review-metadata");

        let groups = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("review metadata groups");
        let group = groups
            .groups
            .iter()
            .find(|group| group.readiness == "requires-decision")
            .expect("review metadata group");
        assert!(group
            .review_reason_counts
            .iter()
            .any(|reason| reason.reason == "low_confidence" && reason.count == 1));
        assert!(group
            .review_reason_counts
            .iter()
            .any(|reason| reason.reason == "requires_confirmation" && reason.count == 1));
        assert!(group
            .review_reason_counts
            .iter()
            .any(|reason| reason.reason == "target_collision" && reason.count == 1));
        assert!(!group
            .available_actions
            .iter()
            .any(|action| action == "accept_suggestion"));
        assert!(group
            .available_actions
            .iter()
            .any(|action| action == "edit_name"));
        let reason_counts = group.review_reason_counts.clone();
        {
            let conn = db.conn().expect("review detail connection");
            conn.execute(
                "UPDATE organization_plan_items
                 SET blocking_detail = 'directory preview collision confirmation'
                 WHERE id = 'item-review-metadata'",
                [],
            )
            .expect("change human review detail");
        }
        let detail_changed_group = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("review reason detail-independent group page")
            .groups
            .into_iter()
            .find(|candidate| candidate.readiness == "requires-decision")
            .expect("review reason detail-independent group");
        assert_eq!(detail_changed_group.review_reason_counts, reason_counts);

        let item = db
            .query_organization_plan_group_items(QueryOrganizationPlanGroupItemsRequest {
                plan_id: "plan-test".into(),
                group_id: group.group_id.clone(),
                cursor: None,
                expected_projection_fingerprint: group.projection_fingerprint.clone(),
                page_size: 20,
            })
            .expect("review metadata items")
            .items
            .into_iter()
            .next()
            .expect("review metadata item");
        assert_eq!(item.review_reasons[0], "low_confidence");
        assert!(!item
            .available_actions
            .iter()
            .any(|action| action == "accept_suggestion"));
        assert!(item
            .available_actions
            .iter()
            .any(|action| action == "edit_name"));

        let error = db
            .update_organization_plan_decisions(UpdateOrganizationPlanDecisionsRequest {
                plan_id: "plan-test".into(),
                expected_plan_revision: 1,
                safe_batch: false,
                mutations: vec![OrganizationDecisionMutation {
                    item_id: item.id.clone(),
                    expected_item_revision: item.revision,
                    decision: "accepted".into(),
                    edited_filename: None,
                }],
            })
            .expect_err("collision review cannot be accepted");
        assert!(error
            .to_string()
            .contains("organization_item_accept_not_available"));

        assert!(db
            .update_organization_plan_decisions(UpdateOrganizationPlanDecisionsRequest {
                plan_id: "plan-test".into(),
                expected_plan_revision: 1,
                safe_batch: true,
                mutations: vec![OrganizationDecisionMutation {
                    item_id: item.id,
                    expected_item_revision: item.revision,
                    decision: "accepted".into(),
                    edited_filename: None,
                }],
            })
            .expect_err("review item is not eligible for safe batch")
            .to_string()
            .contains("organization_safe_batch_item_blocked"));
        drop(db);
        let _ = std::fs::remove_dir_all(fixture);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn group_accept_uses_action_intersection_and_current_plan_revision() {
        let (db, path) = test_database();
        seed_plan(&db, "ready");
        let fixture = path.with_extension("group-action");
        std::fs::create_dir_all(&fixture).expect("create group-action target");
        {
            let conn = db.conn().expect("group action fixture connection");
            conn.execute(
                "UPDATE organization_plan_items SET source_path_snapshot = ?1,
                    proposed_target_directory = ?2, proposed_target_path = ?3
                 WHERE id = 'item-test'",
                params![
                    fixture
                        .join("source-item-test.txt")
                        .to_string_lossy()
                        .replace('\\', "/"),
                    fixture.to_string_lossy().replace('\\', "/"),
                    fixture
                        .join("target-item-test.txt")
                        .to_string_lossy()
                        .replace('\\', "/"),
                ],
            )
            .expect("bind group action item");
            conn.execute(
                "UPDATE files SET path = ?1, suggested_target_path = ?2,
                        suggested_name = ?3
                 WHERE id = 'file-test'",
                params![
                    fixture
                        .join("source-item-test.txt")
                        .to_string_lossy()
                        .replace('\\', "/"),
                    fixture.to_string_lossy().replace('\\', "/"),
                    "target-item-test.txt"
                ],
            )
            .expect("bind group action indexed file");
        }
        sync_item_proposal_fingerprint(&db, "item-test", "file-test");
        seed_group_item(
            &db,
            "item-group-blocked",
            1,
            &fixture,
            "ready",
            "undecided",
            0.95,
            true,
            None,
        );
        {
            let conn = db.conn().expect("group action member connection");
            let source = fixture.join("source-item-group-blocked.txt");
            conn.execute(
                "INSERT INTO files (id, path, name, extension, size, mtime,
                        suggested_action, suggested_target_path, suggested_name,
                        confidence, risk_level, requires_confirmation,
                        classification_status, last_classified_mtime, last_classified_size)
                 VALUES ('file-item-group-blocked', ?1, 'source-item-group-blocked.txt', 'txt',
                         10, 1, 'Rename', ?2, 'target-item-group-blocked.txt',
                         0.95, 'Normal', 1, 'classified', 1, 10)",
                params![
                    source.to_string_lossy().replace('\\', "/"),
                    fixture.to_string_lossy().replace('\\', "/")
                ],
            )
            .expect("seed group action member file");
        }
        sync_item_proposal_fingerprint(&db, "item-group-blocked", "file-item-group-blocked");
        let group = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("group action projection")
            .groups
            .into_iter()
            .find(|group| group.item_count == 2)
            .expect("group action group");
        assert!(group.group_actions.can_accept_all);
        assert!(group.group_actions.can_keep_all);

        {
            let conn = db.conn().expect("remove group accept action");
            conn.execute(
                "UPDATE organization_plan_items SET decision = 'kept'
                 WHERE id = 'item-group-blocked'",
                [],
            )
            .expect("remove group accept action");
        }
        let unavailable_group = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("group action intersection projection")
            .groups
            .into_iter()
            .find(|candidate| candidate.item_count == 2)
            .expect("group action intersection group");
        assert!(!unavailable_group.group_actions.can_accept_all);
        assert!(unavailable_group.group_actions.can_keep_all);
        let error = db
            .update_organization_plan_group_decision(UpdateOrganizationPlanGroupDecisionRequest {
                plan_id: "plan-test".into(),
                group_id: unavailable_group.group_id.clone(),
                expected_plan_revision: 1,
                expected_projection_fingerprint: unavailable_group.projection_fingerprint.clone(),
                expected_item_count: unavailable_group.item_count,
                decision: "accepted".into(),
            })
            .expect_err("unavailable group action must fail atomically");
        assert!(error
            .to_string()
            .contains("organization_group_action_not_available"));
        assert_eq!(
            db.get_organization_plan("plan-test")
                .expect("unchanged group plan")
                .revision,
            1
        );
        let unchanged_items = db
            .query_organization_plan_group_items(QueryOrganizationPlanGroupItemsRequest {
                plan_id: "plan-test".into(),
                group_id: unavailable_group.group_id.clone(),
                cursor: None,
                expected_projection_fingerprint: unavailable_group.projection_fingerprint.clone(),
                page_size: 20,
            })
            .expect("group members after rejected accept")
            .items;
        assert_eq!(
            unchanged_items
                .iter()
                .filter(|item| item.decision == "accepted")
                .count(),
            0
        );
        {
            let conn = db.conn().expect("restore group accept action");
            conn.execute(
                "UPDATE organization_plan_items SET decision = 'undecided'
                 WHERE id = 'item-group-blocked'",
                [],
            )
            .expect("restore group accept action");
        }
        let refreshed_group = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("group action projection after refresh")
            .groups
            .into_iter()
            .find(|candidate| candidate.item_count == 2)
            .expect("complete group action group");
        assert!(refreshed_group.group_actions.can_accept_all);
        let updated = db
            .update_organization_plan_group_decision(UpdateOrganizationPlanGroupDecisionRequest {
                plan_id: "plan-test".into(),
                group_id: refreshed_group.group_id.clone(),
                expected_plan_revision: 1,
                expected_projection_fingerprint: refreshed_group.projection_fingerprint.clone(),
                expected_item_count: refreshed_group.item_count,
                decision: "accepted".into(),
            })
            .expect("accept complete group action");
        assert_eq!(updated.plan.revision, 2);
        assert_eq!(updated.plan.summary.accepted, 2);
        assert!(updated.group.is_none());
        let accepted_group = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("accepted group projection")
            .groups
            .into_iter()
            .find(|candidate| candidate.item_count == 2)
            .expect("accepted group");
        assert!(!accepted_group.group_actions.can_accept_all);
        assert!(accepted_group.group_actions.can_clear_all);
        assert!(db
            .update_organization_plan_group_decision(UpdateOrganizationPlanGroupDecisionRequest {
                plan_id: "plan-test".into(),
                group_id: refreshed_group.group_id,
                expected_plan_revision: 1,
                expected_projection_fingerprint: refreshed_group.projection_fingerprint,
                expected_item_count: refreshed_group.item_count,
                decision: "kept".into(),
            })
            .expect_err("stale group decision")
            .to_string()
            .contains("organization_plan_revision_conflict"));
        drop(db);
        let _ = std::fs::remove_dir_all(fixture);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn group_mutation_rejects_edit_and_unknown_decisions_without_panic() {
        let (db, path) = test_database();
        seed_plan(&db, "ready");
        sync_item_proposal_fingerprint(&db, "item-test", "file-test");
        let group = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("group decision validation projection")
            .groups
            .into_iter()
            .next()
            .expect("group decision validation group");

        for decision in ["edited", "unexpected"] {
            let error = db
                .update_organization_plan_group_decision(
                    UpdateOrganizationPlanGroupDecisionRequest {
                        plan_id: "plan-test".into(),
                        group_id: group.group_id.clone(),
                        expected_plan_revision: 1,
                        expected_projection_fingerprint: group.projection_fingerprint.clone(),
                        expected_item_count: group.item_count,
                        decision: decision.into(),
                    },
                )
                .expect_err("invalid group decision must return validation error");
            assert!(error
                .to_string()
                .contains("organization_group_decision_invalid"));
        }

        let unchanged = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("unchanged items after invalid group decisions");
        assert!(unchanged
            .items
            .iter()
            .all(|item| item.decision == "undecided"));
        assert_eq!(
            db.get_organization_plan("plan-test")
                .expect("unchanged plan after invalid group decisions")
                .revision,
            1
        );

        let accepted = db
            .update_organization_plan_group_decision(UpdateOrganizationPlanGroupDecisionRequest {
                plan_id: "plan-test".into(),
                group_id: group.group_id.clone(),
                expected_plan_revision: 1,
                expected_projection_fingerprint: group.projection_fingerprint,
                expected_item_count: group.item_count,
                decision: "accept".into(),
            })
            .expect("accept alias remains supported");
        assert_eq!(accepted.plan.summary.accepted, 1);

        let accepted_group = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("accepted group projection")
            .groups
            .into_iter()
            .find(|candidate| candidate.group_id == group.group_id)
            .expect("accepted group remains addressable");
        assert!(accepted_group.group_actions.can_clear_all);
        let cleared = db
            .update_organization_plan_group_decision(UpdateOrganizationPlanGroupDecisionRequest {
                plan_id: "plan-test".into(),
                group_id: accepted_group.group_id.clone(),
                expected_plan_revision: 2,
                expected_projection_fingerprint: accepted_group.projection_fingerprint,
                expected_item_count: accepted_group.item_count,
                decision: "clear".into(),
            })
            .expect("clear alias remains supported");
        assert_eq!(cleared.plan.summary.undecided, 1);

        let cleared_group = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("cleared group projection")
            .groups
            .into_iter()
            .find(|candidate| candidate.group_id == group.group_id)
            .expect("cleared group remains addressable");
        let kept = db
            .update_organization_plan_group_decision(UpdateOrganizationPlanGroupDecisionRequest {
                plan_id: "plan-test".into(),
                group_id: cleared_group.group_id,
                expected_plan_revision: 3,
                expected_projection_fingerprint: cleared_group.projection_fingerprint,
                expected_item_count: cleared_group.item_count,
                decision: "keep".into(),
            })
            .expect("keep alias remains supported");
        assert_eq!(kept.plan.summary.kept, 1);

        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn blocked_preview_actions_are_projected_and_rejected_by_mutation() {
        let (db, path) = test_database();
        seed_plan(&db, "ready");
        let fixture = path.with_extension("blocked-actions");
        std::fs::create_dir_all(&fixture).expect("create blocked action fixture");
        seed_group_item(
            &db,
            "item-missing-preview",
            1,
            &fixture,
            "blocked",
            "undecided",
            0.95,
            false,
            Some("missing_preview"),
        );
        seed_group_item(
            &db,
            "item-extension-blocked",
            2,
            &fixture,
            "blocked",
            "undecided",
            0.95,
            false,
            Some("extension_change_blocked"),
        );
        {
            let conn = db.conn().expect("blocked action connection");
            conn.execute(
                "INSERT INTO files (id, path, name, extension, size, mtime)
                 VALUES ('file-item-extension-blocked', ?1, 'source-item-extension-blocked.txt', 'txt', 10, 1)",
                params![fixture
                    .join("source-item-extension-blocked.txt")
                    .to_string_lossy()
                    .replace('\\', "/")],
            )
            .expect("seed extension blocked file");
            conn.execute(
                "UPDATE files SET classification_status = 'classified',
                        suggested_action = 'Rename', suggested_target_path = ?1,
                        suggested_name = 'target-item-extension-blocked.pdf',
                        confidence = 0.95, risk_level = 'Normal',
                        last_classified_mtime = 1, last_classified_size = 10
                 WHERE id = 'file-item-extension-blocked'",
                params![fixture.to_string_lossy().replace('\\', "/")],
            )
            .expect("seed extension blocked proposal");
            let preview_id = operation_preview_from_indexed(
                load_indexed_file_by_id(&conn, "file-item-extension-blocked")
                    .expect("load extension blocked file")
                    .expect("extension blocked file exists"),
            )
            .expect("extension blocked preview")
            .id;
            conn.execute(
                "UPDATE organization_plan_items SET authoritative_preview_id = ?1
                 WHERE id = 'item-extension-blocked'",
                params![preview_id],
            )
            .expect("bind extension blocked preview");
        }

        let group_page = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("blocked action groups");
        let missing_group = group_page
            .groups
            .iter()
            .find(|group| {
                group
                    .sample_items
                    .iter()
                    .any(|sample| sample.item_id == "item-missing-preview")
            })
            .expect("missing preview group");
        let missing_item = db
            .query_organization_plan_group_items(QueryOrganizationPlanGroupItemsRequest {
                plan_id: "plan-test".into(),
                group_id: missing_group.group_id.clone(),
                cursor: None,
                expected_projection_fingerprint: missing_group.projection_fingerprint.clone(),
                page_size: 20,
            })
            .expect("missing preview items")
            .items
            .into_iter()
            .find(|item| item.id == "item-missing-preview")
            .expect("missing preview item");
        assert!(missing_item
            .review_reasons
            .iter()
            .any(|reason| reason == "missing_preview"));
        assert!(!missing_item
            .available_actions
            .iter()
            .any(|action| action == "accept_suggestion" || action == "edit_name"));
        let missing_error = db
            .update_organization_plan_decisions(UpdateOrganizationPlanDecisionsRequest {
                plan_id: "plan-test".into(),
                expected_plan_revision: 1,
                safe_batch: false,
                mutations: vec![OrganizationDecisionMutation {
                    item_id: missing_item.id,
                    expected_item_revision: missing_item.revision,
                    decision: "accepted".into(),
                    edited_filename: None,
                }],
            })
            .expect_err("missing preview accept must be rejected");
        assert!(missing_error
            .to_string()
            .contains("organization_item_accept_not_available"));

        let extension_group = group_page
            .groups
            .iter()
            .find(|group| {
                group
                    .sample_items
                    .iter()
                    .any(|sample| sample.item_id == "item-extension-blocked")
            })
            .expect("extension blocked group");
        let extension_item = db
            .query_organization_plan_group_items(QueryOrganizationPlanGroupItemsRequest {
                plan_id: "plan-test".into(),
                group_id: extension_group.group_id.clone(),
                cursor: None,
                expected_projection_fingerprint: extension_group.projection_fingerprint.clone(),
                page_size: 20,
            })
            .expect("extension blocked items")
            .items
            .into_iter()
            .find(|item| item.id == "item-extension-blocked")
            .expect("extension blocked item");
        assert!(extension_item
            .review_reasons
            .iter()
            .any(|reason| reason == "extension_change_blocked"));
        assert!(!extension_item
            .available_actions
            .iter()
            .any(|action| action == "accept_suggestion" || action == "edit_name"));
        let extension_error = db
            .update_organization_plan_decisions(UpdateOrganizationPlanDecisionsRequest {
                plan_id: "plan-test".into(),
                expected_plan_revision: 1,
                safe_batch: false,
                mutations: vec![OrganizationDecisionMutation {
                    item_id: extension_item.id,
                    expected_item_revision: extension_item.revision,
                    decision: "accepted".into(),
                    edited_filename: None,
                }],
            })
            .expect_err("extension blocked accept must be rejected");
        assert!(extension_error
            .to_string()
            .contains("organization_item_accept_not_available"));
        drop(db);
        let _ = std::fs::remove_dir_all(fixture);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn collision_can_be_resolved_by_edit_before_dry_run() {
        let (db, path) = test_database();
        seed_plan(&db, "ready");
        let fixture = path.with_extension("collision-edit");
        std::fs::create_dir_all(&fixture).expect("create collision edit fixture");
        let source = fixture.join("source.txt");
        let target = fixture.join("target.txt");
        std::fs::write(&target, b"collision").expect("seed collision target");
        let fixture_text = fixture.to_string_lossy().replace('\\', "/");
        let source_text = source.to_string_lossy().replace('\\', "/");
        {
            let conn = db.conn().expect("collision edit connection");
            conn.execute(
                "INSERT INTO scan_roots (
                    id, normalized_path, display_name, source_kind, enabled,
                    health_status, current_generation, needs_reconciliation,
                    created_at, updated_at
                 ) VALUES ('root-collision-edit', ?1, 'Collision edit', 'file_library', 1,
                           'healthy', 1, 0, 1, 1)",
                params![fixture_text],
            )
            .expect("seed collision edit root");
            conn.execute(
                "UPDATE files SET path = ?1, suggested_target_path = ?2,
                        suggested_name = 'target.txt', confidence = 0.95,
                        risk_level = 'Normal', requires_confirmation = 0,
                        classification_status = 'classified',
                        last_classified_mtime = 1, last_classified_size = 1
                 WHERE id = 'file-test'",
                params![source_text.clone(), fixture_text],
            )
            .expect("bind collision edit file");
            let indexed = load_indexed_file_by_id(&conn, "file-test")
                .expect("load collision edit file")
                .expect("collision edit file exists");
            let preview =
                operation_preview_from_indexed(indexed.clone()).expect("collision edit preview");
            let proposal = proposal_from_preview(
                &indexed.path,
                &indexed.name,
                &indexed.classification_status,
                &indexed.suggested_action,
                Some(preview.clone()),
            );
            conn.execute(
                "UPDATE organization_plan_items SET source_path_snapshot = ?1,
                        source_size_snapshot = 1, source_mtime_snapshot = 1,
                        proposal_fingerprint = ?2, proposal_kind = ?3,
                        proposed_target_directory = ?4, proposed_name = ?5,
                        proposed_target_path = ?6, validity = 'blocked',
                        confidence = ?7, risk_level = ?8,
                        requires_confirmation = ?9, blocking_code = 'target_collision',
                        blocking_detail = 'The target path is already occupied.',
                        authoritative_preview_id = ?10
                 WHERE id = 'item-test'",
                params![
                    source_text,
                    proposal.fingerprint,
                    proposal.kind,
                    proposal.target_directory,
                    proposal.name,
                    proposal.target_path,
                    proposal.confidence,
                    proposal.risk,
                    bool_to_i64(proposal.requires_confirmation),
                    preview.id,
                ],
            )
            .expect("bind collision edit plan item");
        }
        sync_item_proposal_fingerprint(&db, "item-test", "file-test");

        let item = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 10,
            })
            .expect("collision edit item page")
            .items
            .into_iter()
            .next()
            .expect("collision edit item");
        assert!(!item
            .available_actions
            .iter()
            .any(|action| action == "accept_suggestion"));
        assert!(item
            .available_actions
            .iter()
            .any(|action| action == "edit_name"));
        let updated = db
            .update_organization_plan_decisions(UpdateOrganizationPlanDecisionsRequest {
                plan_id: "plan-test".into(),
                expected_plan_revision: 1,
                safe_batch: false,
                mutations: vec![OrganizationDecisionMutation {
                    item_id: item.id,
                    expected_item_revision: item.revision,
                    decision: "edited".into(),
                    edited_filename: Some("resolved.txt".into()),
                }],
            })
            .expect("collision edit decision");
        assert_eq!(updated.summary.reviewed, 1);
        let dry_run = db
            .get_organization_plan_dry_run(OrganizationPlanSelectionRequest {
                plan_id: "plan-test".into(),
                expected_plan_revision: updated.revision,
                item_ids: Vec::new(),
                all_accepted: true,
            })
            .expect("collision edit dry run");
        assert_eq!(dry_run.executable_count, 1);
        assert_eq!(dry_run.blocked_count, 0);
        assert_eq!(
            dry_run.items[0].to.replace('\\', "/"),
            fixture
                .join("resolved.txt")
                .to_string_lossy()
                .replace('\\', "/")
        );
        drop(db);
        let _ = std::fs::remove_file(target);
        let _ = std::fs::remove_dir(fixture);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn decision_batch_uses_plan_and_item_revision_cas() {
        let (db, path) = test_database();
        seed_plan(&db, "ready");
        sync_item_proposal_fingerprint(&db, "item-test", "file-test");
        let updated = db
            .update_organization_plan_decisions(UpdateOrganizationPlanDecisionsRequest {
                plan_id: "plan-test".into(),
                expected_plan_revision: 1,
                safe_batch: false,
                mutations: vec![OrganizationDecisionMutation {
                    item_id: "item-test".into(),
                    expected_item_revision: 1,
                    decision: "accepted".into(),
                    edited_filename: None,
                }],
            })
            .expect("first decision");
        assert_eq!(updated.revision, 2);
        let page = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 10,
            })
            .expect("decision page");
        assert_eq!(page.items[0].decision, "accepted");
        assert_eq!(page.items[0].revision, 2);
        assert!(db
            .update_organization_plan_decisions(UpdateOrganizationPlanDecisionsRequest {
                plan_id: "plan-test".into(),
                expected_plan_revision: 1,
                safe_batch: false,
                mutations: vec![OrganizationDecisionMutation {
                    item_id: "item-test".into(),
                    expected_item_revision: 1,
                    decision: "kept".into(),
                    edited_filename: None,
                }],
            })
            .is_err());
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn effective_readiness_revalidates_live_facts_without_mutating_validity() {
        let (db, path) = test_database();
        seed_plan(&db, "ready");
        sync_item_proposal_fingerprint(&db, "item-test", "file-test");
        let initial = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 10,
            })
            .expect("initial effective item page")
            .items
            .remove(0);
        assert_eq!(initial.effective_readiness, "ready");
        let initial_explanation = initial
            .semantic_explanation
            .as_ref()
            .expect("current semantic assessment explanation");
        assert_eq!(
            initial_explanation.reason,
            "current semantic organization test fixture"
        );
        assert!(!initial_explanation.assessment_fingerprint.is_empty());
        assert_eq!(
            db.query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("initial effective summary")
            .effective_summary
            .ready,
            1
        );

        {
            let conn = db.conn().expect("live identity connection");
            conn.execute("UPDATE files SET size = 2 WHERE id = 'file-test'", [])
                .expect("change source size");
        }
        let size_changed = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 10,
            })
            .expect("size changed projection")
            .items
            .remove(0);
        assert_eq!(size_changed.effective_readiness, "blocked");
        assert!(size_changed
            .review_reasons
            .iter()
            .any(|reason| reason == "source_changed"));

        {
            let conn = db.conn().expect("restore identity connection");
            conn.execute(
                "UPDATE files SET size = 1, path = '/tmp/moved-source.txt' WHERE id = 'file-test'",
                [],
            )
            .expect("move source");
        }
        let moved = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 10,
            })
            .expect("moved source projection")
            .items
            .remove(0);
        assert_eq!(moved.effective_readiness, "blocked");

        {
            let conn = db.conn().expect("restore source connection");
            conn.execute(
                "UPDATE files SET path = '/tmp/source.txt', mtime = 2 WHERE id = 'file-test'",
                [],
            )
            .expect("change source mtime");
        }
        let mtime_changed = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 10,
            })
            .expect("mtime changed projection")
            .items
            .remove(0);
        assert_eq!(mtime_changed.effective_readiness, "blocked");

        {
            let conn = db.conn().expect("restore mtime connection");
            conn.execute(
                "UPDATE files SET mtime = 1, is_stale = 1 WHERE id = 'file-test'",
                [],
            )
            .expect("mark source unavailable");
        }
        let missing = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 10,
            })
            .expect("missing source projection")
            .items
            .remove(0);
        assert_eq!(missing.effective_readiness, "blocked");

        {
            let conn = db.conn().expect("restore source availability connection");
            conn.execute("UPDATE files SET is_stale = 0 WHERE id = 'file-test'", [])
                .expect("restore source availability");
            conn.execute(
                "UPDATE files SET suggested_name = 'legacy-ignored.txt',
                        suggested_target_path = '/tmp' WHERE id = 'file-test'",
                [],
            )
            .expect("change legacy proposal fields");
        }
        let legacy_changed = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 10,
            })
            .expect("legacy proposal field projection")
            .items
            .remove(0);
        assert_eq!(legacy_changed.effective_readiness, "ready");
        assert!(!legacy_changed
            .review_reasons
            .iter()
            .any(|reason| reason == "proposal_changed"));
        assert_eq!(
            legacy_changed
                .semantic_explanation
                .as_ref()
                .map(|explanation| explanation.assessment_fingerprint.as_str()),
            Some(initial_explanation.assessment_fingerprint.as_str())
        );

        replace_current_managed_semantic_rename(&db, "file-test", "live-changed.txt");
        let proposal_changed = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 10,
            })
            .expect("current semantic proposal projection")
            .items
            .remove(0);
        assert_eq!(proposal_changed.effective_readiness, "blocked");
        assert!(proposal_changed
            .review_reasons
            .iter()
            .any(|reason| reason == "proposal_changed"));
        assert!(proposal_changed.semantic_explanation.is_none());

        replace_current_managed_semantic_rename(&db, "file-test", "renamed.txt");
        {
            let conn = db.conn().expect("preview mismatch connection");
            conn.execute(
                "UPDATE files SET suggested_name = 'renamed.txt',
                        suggested_target_path = '/tmp'
                 WHERE id = 'file-test'",
                [],
            )
            .expect("restore legacy proposal fields");
            conn.execute(
                "UPDATE organization_plan_items SET authoritative_preview_id = 'stale-preview'
                 WHERE id = 'item-test'",
                [],
            )
            .expect("change authoritative preview");
        }
        let preview_changed = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 10,
            })
            .expect("preview mismatch projection")
            .items
            .remove(0);
        assert_eq!(preview_changed.effective_readiness, "blocked");
        assert!(preview_changed
            .review_reasons
            .iter()
            .any(|reason| reason == "proposal_changed"));

        {
            let conn = db.conn().expect("restore proposal connection");
            conn.execute(
                "UPDATE files SET content_hash = 'content-only' WHERE id = 'file-test'",
                [],
            )
            .expect("change content only");
            conn.execute(
                "UPDATE organization_plan_items SET authoritative_preview_id = ?1
                 WHERE id = 'item-test'",
                params![initial.authoritative_preview_id],
            )
            .expect("restore authoritative preview");
        }
        let content_only = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 10,
            })
            .expect("content-only projection")
            .items
            .remove(0);
        assert_eq!(content_only.effective_readiness, "ready");
        let persisted_validity: String = db
            .conn()
            .expect("persisted validity connection")
            .query_row(
                "SELECT validity FROM organization_plan_items WHERE id = 'item-test'",
                [],
                |row| row.get(0),
            )
            .expect("read persisted validity");
        assert_eq!(persisted_validity, "ready");

        {
            let conn = db.conn().expect("scope provenance connection");
            conn.execute(
                "UPDATE organization_plans SET source_kind = 'all_matching',
                        source_query_spec_json = NULL, source_query_fingerprint = NULL
                 WHERE id = 'plan-test'",
                [],
            )
            .expect("invalidate managed scope provenance");
        }
        let scope_unavailable = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 10,
            })
            .expect("scope unavailable projection")
            .items
            .remove(0);
        assert_eq!(scope_unavailable.effective_readiness, "blocked");
        assert!(scope_unavailable
            .review_reasons
            .iter()
            .any(|reason| reason == "managed_scope_changed"));
        let refreshed = db
            .refresh_organization_plan(OrganizationPlanRevisionRequest {
                plan_id: "plan-test".into(),
                expected_plan_revision: 1,
            })
            .expect("refresh unavailable scope");
        assert_eq!(refreshed.status, "stale");
        assert!(refreshed.effective_summary.is_none());
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn plan_summary_is_authoritative_for_the_whole_ledger() {
        let (db, path) = test_database();
        seed_plan(&db, "ready");
        {
            let conn = db.conn().expect("summary fixture");
            conn.execute(
                "UPDATE organization_plan_items
                 SET decision = 'accepted', validity = 'needs_review'
                 WHERE id = 'item-test'",
                [],
            )
            .expect("mark reviewed item");
        }
        let plan = db.get_organization_plan("plan-test").expect("summary plan");
        assert_eq!(plan.summary.accepted, 1);
        assert_eq!(plan.summary.needs_review, 1);
        assert_eq!(plan.summary.remaining_executable, 1);
        assert_eq!(plan.summary.undecided, 0);
        let page = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 1,
            })
            .expect("review-state page");
        assert_eq!(page.items[0].review_state, "reviewed");
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn managed_scope_membership_fails_closed_for_watcher_recovery() {
        let (db, path) = test_database();
        {
            let conn = db.conn().expect("managed scope fixture");
            conn.execute(
                "INSERT INTO scan_roots (
                    id, normalized_path, display_name, source_kind, enabled,
                    health_status, current_generation, needs_reconciliation,
                    watcher_revision, watcher_applied_revision,
                    watcher_rule_recovery_required, created_at, updated_at
                 ) VALUES (
                    'root-scope-test', '/managed', 'Managed', 'file_library', 1,
                    'healthy', 1, 0, 4, 4, 0, 1, 1
                 )",
                [],
            )
            .expect("insert managed root");
        }
        db.insert_file(InsertFileRequest {
            id: "file-scope-test".into(),
            path: "/managed/source.txt".into(),
            name: "source.txt".into(),
            extension: "txt".into(),
            size: 1,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("insert managed file");
        let query = FileQuerySpecV2 {
            scope: FileLibraryScopeV2::AllEnabledRoots,
            text: None,
            filters: FileQueryFiltersV2::default(),
            sort: FileLibrarySortV2::default(),
        };
        {
            let conn = db.conn().expect("healthy scope");
            assert!(
                file_matches_authoritative_query_scope(&conn, &query, "file-scope-test")
                    .expect("healthy scope membership")
            );
            conn.execute(
                "UPDATE scan_roots SET watcher_rule_recovery_required = 1
                 WHERE id = 'root-scope-test'",
                [],
            )
            .expect("require watcher recovery");
            let error = file_matches_authoritative_query_scope(&conn, &query, "file-scope-test")
                .expect_err("watcher recovery must fail closed");
            assert!(error.to_string().contains("library_scope_unavailable"));
        }
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn needs_review_requires_explicit_accept_before_becoming_reviewed() {
        let (db, path) = test_database();
        seed_plan(&db, "stale");
        sync_item_proposal_fingerprint(&db, "item-test", "file-test");
        {
            let conn = db.conn().expect("review fixture");
            conn.execute(
                "UPDATE organization_plan_items SET validity = 'needs_review',
                        confidence = 0.7, risk_level = 'Normal', requires_confirmation = 1
                 WHERE id = 'item-test'",
                [],
            )
            .expect("mark needs review");
        }
        let before = db
            .get_organization_plan("plan-test")
            .expect("pending review plan");
        assert_eq!(before.summary.needs_review, 1);
        assert_eq!(before.summary.pending_review, 1);
        assert_eq!(before.summary.reviewed, 0);
        let before_group = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("pending review group page");
        assert_eq!(before_group.groups.len(), 1);
        assert_eq!(before_group.groups[0].readiness, "requires-decision");
        let updated = db
            .update_organization_plan_decisions(UpdateOrganizationPlanDecisionsRequest {
                plan_id: "plan-test".into(),
                expected_plan_revision: 1,
                safe_batch: false,
                mutations: vec![OrganizationDecisionMutation {
                    item_id: "item-test".into(),
                    expected_item_revision: 1,
                    decision: "accepted".into(),
                    edited_filename: None,
                }],
            })
            .expect("explicit review");
        assert_eq!(updated.status, "ready");
        assert_eq!(updated.summary.remaining_executable, 1);
        assert_eq!(updated.summary.pending_review, 0);
        assert_eq!(updated.summary.reviewed, 1);
        let page = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 10,
            })
            .expect("reviewed item");
        assert_eq!(page.items[0].review_state, "reviewed");
        assert_eq!(page.items[0].risk_level, "Normal");
        assert!(page.items[0].requires_confirmation);
        let after_groups = db
            .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 20,
            })
            .expect("reviewed group page");
        assert_eq!(after_groups.groups.len(), 1);
        assert_eq!(after_groups.groups[0].readiness, "reviewed");
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn reviewed_edited_target_is_the_actual_executed_target() {
        let (db, path) = test_database();
        let fixture = std::env::temp_dir().join(format!(
            "zen-canvas-organization-live-target-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&fixture).expect("create live-target fixture");
        let source = fixture.join("source.txt");
        let reviewed_target = fixture.join("reviewed.txt");
        let fixture_text = fixture.to_string_lossy().replace('\\', "/");
        let source_text = source.to_string_lossy().replace('\\', "/");
        std::fs::write(&source, b"reviewed target").expect("write source");
        let metadata = std::fs::metadata(&source).expect("source metadata");
        let mtime = metadata
            .modified()
            .expect("source modified")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("source epoch")
            .as_secs() as i64;
        {
            let conn = db.conn().expect("live target fixture");
            conn.execute(
                "INSERT INTO scan_roots (
                    id, normalized_path, display_name, source_kind, enabled,
                    health_status, current_generation, needs_reconciliation,
                    created_at, updated_at
                 ) VALUES (
                    'root-live-target', ?1, 'Live target', 'file_library', 1,
                    'healthy', 1, 0, 1, 1
                 )",
                params![fixture_text],
            )
            .expect("insert healthy managed root");
        }
        db.insert_file(InsertFileRequest {
            id: "file-live-target".into(),
            path: source_text,
            name: "source.txt".into(),
            extension: "txt".into(),
            size: metadata.len() as i64,
            mtime,
            ctime: mtime,
            is_dir: false,
            state_code: 0,
        })
        .expect("index source");
        {
            let conn = db.conn().expect("classification fixture");
            conn.execute(
                "UPDATE files SET classification_status = 'classified',
                        suggested_action = 'Rename',
                        suggested_target_path = ?2,
                        suggested_name = 'model-name.txt',
                        confidence = 0.95, risk_level = 'Normal',
                        classification_reason = 'test proposal',
                        last_classified_mtime = mtime,
                        last_classified_size = size
                 WHERE id = ?1",
                params![
                    "file-live-target",
                    fixture.to_string_lossy().replace('\\', "/")
                ],
            )
            .expect("classify source");
        }
        seed_current_managed_semantics_for_file(&db, "file-live-target");
        let plan = db
            .create_organization_plan(CreateOrganizationPlanRequestV1 {
                version: 1,
                request_id: "live-target-request".into(),
                title: Some("Live target".into()),
                source: LibrarySelectionV1::Explicit {
                    file_ids: vec!["file-live-target".into()],
                },
                expected_count: Some(1),
            })
            .expect("create plan");
        let item = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: plan.id.clone(),
                cursor: None,
                page_size: 10,
            })
            .expect("load live item")
            .items
            .remove(0);
        let reviewed = db
            .update_organization_plan_decisions(UpdateOrganizationPlanDecisionsRequest {
                plan_id: plan.id.clone(),
                expected_plan_revision: plan.revision,
                safe_batch: false,
                mutations: vec![OrganizationDecisionMutation {
                    item_id: item.id,
                    expected_item_revision: item.revision,
                    decision: "edited".into(),
                    edited_filename: Some("reviewed.txt".into()),
                }],
            })
            .expect("review edited target");
        let selection = OrganizationPlanSelectionRequest {
            plan_id: plan.id.clone(),
            expected_plan_revision: reviewed.revision,
            item_ids: Vec::new(),
            all_accepted: true,
        };
        let dry_run = db
            .get_organization_plan_dry_run(selection)
            .expect("canonical live dry run");
        assert_eq!(
            std::path::Path::new(&dry_run.items[0].to),
            reviewed_target.as_path()
        );
        assert!(
            dry_run.items[0].executable,
            "live item must be executable: {:?}",
            dry_run.items[0]
        );
        let stale_dispatch =
            db.begin_organization_plan_execution(&ExecuteOrganizationPlanRequest {
                plan_id: plan.id.clone(),
                expected_plan_revision: reviewed.revision,
                dry_run_fingerprint: "stale-fingerprint".into(),
                item_ids: Vec::new(),
                all_accepted: true,
                confirmed: true,
            });
        assert!(matches!(
            stale_dispatch,
            Err(DbError::Validation(code)) if code == "organization_dry_run_expired"
        ));
        let dispatch = db
            .begin_organization_plan_execution(&ExecuteOrganizationPlanRequest {
                plan_id: plan.id.clone(),
                expected_plan_revision: reviewed.revision,
                dry_run_fingerprint: dry_run.dry_run_fingerprint,
                item_ids: Vec::new(),
                all_accepted: true,
                confirmed: true,
            })
            .expect("claim canonical execution");
        assert_eq!(
            std::path::Path::new(&dispatch.selections[0].target_path),
            reviewed_target.as_path()
        );
        let result = crate::file_ops::execute_moves_with_persistence(
            &db,
            crate::file_ops::ExecuteMovesRequest {
                operations: dispatch.selections.clone(),
            },
        )
        .expect("execute canonical target");
        assert!(reviewed_target.exists());
        assert!(!source.exists());
        assert_eq!(
            std::fs::canonicalize(&result.logs[0].path_after).expect("canonical journal target"),
            std::fs::canonicalize(&reviewed_target).expect("canonical reviewed target")
        );
        db.finalize_organization_plan_execution(&plan.id, &dispatch, &result.logs)
            .expect("finalize live target plan");
        drop(db);
        let _ = std::fs::remove_file(reviewed_target);
        let _ = std::fs::remove_dir(fixture);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn safe_batch_is_revalidated_by_the_repository() {
        let (db, path) = test_database();
        seed_plan(&db, "ready");
        let fixture = path.with_extension("safe-batch");
        std::fs::create_dir_all(&fixture).expect("create safe batch fixture");
        let source = fixture.join("source.txt");
        let target = fixture.join("renamed.txt");
        std::fs::write(&target, b"collision").expect("seed target collision");
        {
            let conn = db.conn().expect("safe batch fixture connection");
            conn.execute(
                "UPDATE organization_plan_items SET source_path_snapshot = ?1,
                        proposed_target_directory = ?2, proposed_target_path = ?3
                 WHERE id = 'item-test'",
                params![
                    source.to_string_lossy(),
                    fixture.to_string_lossy(),
                    target.to_string_lossy(),
                ],
            )
            .expect("bind safe batch fixture");
            conn.execute(
                "UPDATE files SET path = ?1, suggested_target_path = ?2,
                        suggested_name = ?3
                 WHERE id = 'file-test'",
                params![
                    source.to_string_lossy(),
                    fixture.to_string_lossy(),
                    "renamed.txt"
                ],
            )
            .expect("bind safe batch indexed file");
        }
        let error = db
            .update_organization_plan_decisions(UpdateOrganizationPlanDecisionsRequest {
                plan_id: "plan-test".into(),
                expected_plan_revision: 1,
                safe_batch: true,
                mutations: vec![OrganizationDecisionMutation {
                    item_id: "item-test".into(),
                    expected_item_revision: 1,
                    decision: "accepted".into(),
                    edited_filename: None,
                }],
            })
            .expect_err("live collision must block safe batch");
        assert!(error
            .to_string()
            .contains("organization_safe_batch_item_blocked"));
        drop(db);
        let _ = std::fs::remove_file(target);
        let _ = std::fs::remove_dir(fixture);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn terminal_plan_cannot_regress_and_delete_requires_confirmation() {
        let (db, path) = test_database();
        seed_plan(&db, "ready");
        let cancelled = db
            .cancel_organization_plan(OrganizationPlanRevisionRequest {
                plan_id: "plan-test".into(),
                expected_plan_revision: 1,
            })
            .expect("cancel plan");
        assert_eq!(cancelled.status, "cancelled");
        assert!(db
            .cancel_organization_plan(OrganizationPlanRevisionRequest {
                plan_id: "plan-test".into(),
                expected_plan_revision: cancelled.revision,
            })
            .is_err());
        assert!(db
            .delete_organization_plan(DeleteOrganizationPlanRequest {
                plan_id: "plan-test".into(),
                expected_plan_revision: cancelled.revision,
                confirmed: false,
            })
            .is_err());
        assert!(db
            .delete_organization_plan(DeleteOrganizationPlanRequest {
                plan_id: "plan-test".into(),
                expected_plan_revision: cancelled.revision,
                confirmed: true,
            })
            .expect("confirmed terminal delete"));
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn startup_recovery_never_replays_unjournaled_execution() {
        let (db, path) = test_database();
        seed_plan(&db, "executing");
        {
            let conn = db.conn().expect("execution fixture");
            conn.execute(
                "UPDATE organization_plans SET active_execution_id = 'execution-test',
                    active_operation_batch_id = 'batch-test' WHERE id = 'plan-test'",
                [],
            )
            .expect("mark execution owner");
            conn.execute(
                "UPDATE organization_plan_items SET validity = 'executing',
                    execution_id = 'execution-test' WHERE id = 'item-test'",
                [],
            )
            .expect("mark item executing");
        }
        assert_eq!(db.recover_organization_plans().expect("recover plan"), 1);
        let plan = db
            .get_organization_plan("plan-test")
            .expect("recovered plan");
        assert_eq!(plan.status, "ready");
        assert!(plan.active_execution_id.is_none());
        let page = db
            .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                plan_id: "plan-test".into(),
                cursor: None,
                page_size: 10,
            })
            .expect("recovered items");
        assert_eq!(page.items[0].validity, "ready");
        assert!(page.items[0].execution_id.is_none());
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn startup_recovery_projects_completed_after_journal_commit_crash() {
        let (db, path) = test_database();
        seed_plan(&db, "executing");
        {
            let conn = db.conn().expect("completed recovery fixture");
            conn.execute(
                "UPDATE organization_plans SET active_execution_id = 'execution-test',
                    active_operation_batch_id = 'batch-test' WHERE id = 'plan-test'",
                [],
            )
            .expect("mark owner");
            conn.execute(
                "UPDATE organization_plan_items SET decision = 'accepted',
                    validity = 'executed', execution_id = 'execution-test',
                    operation_log_id = 'log-preview-test' WHERE id = 'item-test'",
                [],
            )
            .expect("persist item journal projection");
            conn.execute(
                "INSERT INTO operation_batches (id, created_at, status)
                 VALUES ('batch-test', 1, 'completed')",
                [],
            )
            .expect("persist operation batch");
            conn.execute(
                "INSERT INTO operation_logs (
                    id, batch_id, operation_type, source_path, target_path,
                    old_name, new_name, status, created_at, path_before,
                    path_after, name_before, name_after
                 ) VALUES (
                    'log-preview-test', 'batch-test', 'rename', '/tmp/source.txt',
                    '/tmp/renamed.txt', 'source.txt', 'renamed.txt', 'success', 1,
                    '/tmp/source.txt', '/tmp/renamed.txt', 'source.txt', 'renamed.txt'
                 )",
                [],
            )
            .expect("persist successful journal before simulated crash");
        }
        assert_eq!(
            db.recover_organization_plans().expect("restart recovery"),
            1
        );
        let plan = db
            .get_organization_plan("plan-test")
            .expect("recovered plan");
        assert_eq!(plan.status, "completed");
        assert!(plan.completed_at.is_some());
        assert!(plan.active_execution_id.is_none());
        assert_eq!(plan.summary.executed, 1);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn retention_uses_age_union_count_with_dedup_and_batch_cap() {
        let (db, path) = test_database();
        let now = current_unix_seconds();
        let old = now.saturating_sub(31 * 24 * 60 * 60);
        {
            let mut conn = db.conn().expect("retention fixture");
            let tx = conn.transaction().expect("retention transaction");
            for index in 0..101 {
                tx.execute(
                    "INSERT INTO organization_plans (
                        id, title, status, source_kind, source_snapshot_revision,
                        requested_count, materialized_count, planner_version, revision,
                        created_at, updated_at, ready_at, completed_at
                     ) VALUES (?1, 'Recent terminal', 'completed', 'explicit', 1,
                               0, 0, 1, 1, ?2, ?2, ?2, ?2)",
                    params![format!("recent-{index:03}"), now - index],
                )
                .expect("insert recent terminal");
            }
            tx.execute(
                "INSERT INTO organization_plans (
                    id, title, status, source_kind, source_snapshot_revision,
                    requested_count, materialized_count, planner_version, revision,
                    created_at, updated_at, ready_at, completed_at
                 ) VALUES ('age-only', 'Age terminal', 'completed', 'explicit', 1,
                           0, 0, 1, 1, ?1, ?1, ?1, ?1)",
                params![old],
            )
            .expect("insert age terminal");
            tx.execute(
                "INSERT INTO organization_plans (
                    id, title, status, source_kind, source_snapshot_revision,
                    requested_count, materialized_count, planner_version, revision,
                    active_execution_id, active_operation_batch_id,
                    created_at, updated_at, ready_at
                 ) VALUES ('active-execution', 'Active', 'executing', 'explicit', 1,
                           0, 0, 1, 1, 'execution', 'batch',
                           ?1, ?1, ?1)",
                params![old],
            )
            .expect("insert active plan");
            tx.commit().expect("publish retention fixture");
        }
        let pruned = db.prune_organization_plans().expect("prune union");
        assert_eq!(pruned, 2);
        let conn = db.conn().expect("retention assertions");
        let age_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM organization_plans WHERE id = 'age-only'",
                [],
                |row| row.get(0),
            )
            .expect("age count");
        let recent_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM organization_plans WHERE id LIKE 'recent-%'",
                [],
                |row| row.get(0),
            )
            .expect("recent count");
        let active_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM organization_plans WHERE id = 'active-execution'",
                [],
                |row| row.get(0),
            )
            .expect("active count");
        assert_eq!(age_exists, 0);
        assert_eq!(recent_exists, 100);
        assert_eq!(active_exists, 1);
        drop(conn);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    fn seed_performance_current_managed_semantics(db: &Database, count: usize) {
        let settings = crate::ai::settings::AISettings {
            enabled: true,
            ..Default::default()
        };
        let settings_json =
            serde_json::to_string(&settings).expect("serialize benchmark AI settings");
        let mut conn = db.conn().expect("benchmark semantic seed connection");
        let tx = conn
            .transaction()
            .expect("benchmark semantic seed transaction");
        tx.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![crate::ai::settings::AI_SETTINGS_KEY, settings_json],
        )
        .expect("enable benchmark AI settings");
        tx.execute(
            "INSERT INTO global_volumes (
                id, platform, stable_volume_id, display_name, mount_path,
                filesystem_type, drive_kind, enabled, provider, index_status,
                entry_count, created_at, updated_at
             ) VALUES (
                'organization-benchmark-volume', 'test',
                'organization-benchmark-stable-volume', 'Organization benchmark',
                '/missing', 'test', 'fixed', 1, 'test', 'ready', 0, 1, 1
             )",
            [],
        )
        .expect("seed benchmark global volume");
        tx.execute(
            "INSERT INTO managed_scopes (
                id, path, global_entry_id, enabled,
                allow_local_ai, allow_cloud_ai, created_at, updated_at
             ) VALUES (
                'organization-benchmark-scope', '/missing', NULL, 1, 0, 1, 1, 1
             )",
            [],
        )
        .expect("seed benchmark managed scope");

        for ordinal in 0..count {
            let file_id = format!("bench-file-{ordinal:05}");
            let entry_id = format!("bench-global-{ordinal:05}");
            let managed_entry_id = format!("bench-managed-{ordinal:05}");
            let job_id = format!("bench-ai-job-{ordinal:05}");
            let job_item_id = format!("bench-ai-item-{ordinal:05}");
            let platform_file_id = format!("bench-platform-{ordinal:05}");
            let source_name = format!("source-{ordinal:05}.txt");
            let target_name = format!("renamed-{ordinal:05}.txt");
            let source_path = format!("/missing/{source_name}");
            let input_fingerprint = crate::global_index::managed_worker::metadata_fingerprint(
                "organization-benchmark-volume",
                &platform_file_id,
                &source_name,
                1,
                Some(1),
                false,
            );
            let binding = crate::ai::semantic::SemanticSourceBinding {
                global_entry_id: entry_id.clone(),
                managed_scope_id: "organization-benchmark-scope".to_string(),
                input_fingerprint: input_fingerprint.clone(),
                provider: "cloud".to_string(),
            };
            let provider_response = serde_json::json!({
                "version": 1,
                "refId": format!("managed:{entry_id}"),
                "fileType": "Document",
                "purpose": "Work",
                "lifecycle": "Active",
                "context": "organization benchmark",
                "riskLevel": "Normal",
                "suggestedAction": "Rename",
                "suggestedName": target_name,
                "confidence": 0.95,
                "reason": "current semantic benchmark fixture",
                "keywords": [],
                "requiresConfirmation": false
            })
            .to_string();
            let assessment = crate::ai::semantic::SemanticAssessmentV1::parse_provider_response(
                &provider_response,
                binding,
                &source_name,
                "txt",
                false,
            )
            .expect("build benchmark semantic assessment");
            let canonical = assessment
                .canonical_json()
                .expect("encode benchmark semantic assessment");

            tx.execute(
                "INSERT INTO global_entries (
                    id, volume_id, platform_file_id, parent_platform_file_id,
                    name, name_normalized, path, path_normalized, extension,
                    is_directory, size, created_at_fs, modified_at_fs,
                    file_attributes, is_hidden, is_system, is_stale,
                    source_provider, last_seen_at
                 ) VALUES (
                    ?1, 'organization-benchmark-volume', ?2, '',
                    ?3, ?4, ?5, ?5, 'txt',
                    0, 1, 1, 1, 0, 0, 0, 0, 'test', 1
                 )",
                params![
                    entry_id,
                    platform_file_id,
                    source_name,
                    source_name.to_ascii_lowercase(),
                    source_path
                ],
            )
            .expect("seed benchmark global entry");
            tx.execute(
                "INSERT INTO managed_entries (
                    id, global_entry_id, managed_scope_id, enabled, created_at, updated_at
                 ) VALUES (?1, ?2, 'organization-benchmark-scope', 1, 1, 1)",
                params![managed_entry_id, entry_id],
            )
            .expect("seed benchmark managed entry");
            tx.execute(
                "INSERT INTO ai_analysis_state (
                    global_entry_id, status, input_fingerprint, provider, model,
                    content_summary, classification_json, user_corrected, updated_at
                 ) VALUES (?1, 'completed', ?2, 'cloud', ?3, 'metadata_only', ?4, 0, 1)",
                params![entry_id, input_fingerprint, settings.model, canonical],
            )
            .expect("seed benchmark current AI state");
            tx.execute(
                "INSERT INTO ai_jobs (
                    id, global_entry_id, managed_scope_id, input_fingerprint,
                    provider, model, processing_mode, status, attempt_count,
                    created_at, started_at, completed_at
                 ) VALUES (
                    ?1, ?2, 'organization-benchmark-scope', ?3,
                    'cloud', ?4, 'metadata', 'completed', 1, 1, 1, 1
                 )",
                params![job_id, entry_id, input_fingerprint, settings.model],
            )
            .expect("seed benchmark completed AI job");
            tx.execute(
                "INSERT INTO ai_job_items (
                    id, job_id, global_entry_id, status, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, 'completed', 1, 1)",
                params![job_item_id, job_id, entry_id],
            )
            .expect("seed benchmark completed AI job item");

            // Keep the legacy File Library row in place because Task 06 still
            // benchmarks the durable Plan ledger. PM-01 semantic authority is
            // supplied by the current Managed AI binding above.
            let _ = file_id;
        }
        tx.commit().expect("publish benchmark semantic fixtures");
    }

    fn organization_profile_quantile(samples: &[f64], quantile: f64) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        let mut sorted = samples.to_vec();
        sorted.sort_by(f64::total_cmp);
        let position = (sorted.len() - 1) as f64 * quantile;
        let lower = position.floor() as usize;
        let upper = position.ceil() as usize;
        let fraction = position - lower as f64;
        sorted[lower] + (sorted[upper] - sorted[lower]) * fraction
    }

    fn print_organization_execution_profile(
        scope: &str,
        samples: HashMap<&'static str, Vec<f64>>,
        total_ms: f64,
        cpu_ms: Option<f64>,
        before: Option<OrganizationProcessSample>,
        after: Option<OrganizationProcessSample>,
    ) {
        let mut phases = samples.into_iter().collect::<Vec<_>>();
        phases.sort_by_key(|(phase, _)| *phase);
        for (phase, values) in phases {
            println!(
                "[organization-plan-profile] phase={phase} samples={} p50_ms={:.3} p95_ms={:.3} p99_ms={:.3}",
                values.len(),
                organization_profile_quantile(&values, 0.50),
                organization_profile_quantile(&values, 0.95),
                organization_profile_quantile(&values, 0.99),
            );
        }
        let cpu_percent = cpu_ms.map(|cpu| cpu / total_ms * 100.0);
        let memory = |sample: Option<OrganizationProcessSample>| {
            sample.map(|value| {
                format!(
                    "ws_mib={:.1} private_mib={:.1}",
                    value.working_set_bytes as f64 / (1024.0 * 1024.0),
                    value.private_bytes as f64 / (1024.0 * 1024.0),
                )
            })
        };
        println!(
            "[organization-plan-profile] phase={scope}.resources wall_ms={total_ms:.3} process_cpu_ms={cpu_ms:?} process_cpu_pct_one_core={cpu_percent:?} before=({}) after=({})",
            memory(before).unwrap_or_else(|| "unavailable".to_string()),
            memory(after).unwrap_or_else(|| "unavailable".to_string()),
        );
    }

    #[derive(Clone, Copy)]
    struct OrganizationProcessSample {
        cpu_time_100ns: u64,
        working_set_bytes: u64,
        private_bytes: u64,
    }

    #[cfg(target_os = "windows")]
    fn organization_process_sample() -> Option<OrganizationProcessSample> {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

        let process = unsafe { GetCurrentProcess() };
        let mut counters = PROCESS_MEMORY_COUNTERS_EX {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        let memory_ok = unsafe {
            GetProcessMemoryInfo(
                process,
                (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX)
                    .cast::<PROCESS_MEMORY_COUNTERS>(),
                counters.cb,
            )
        };
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let cpu_ok =
            unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) };
        if memory_ok == 0 || cpu_ok == 0 {
            return None;
        }
        let filetime = |value: FILETIME| {
            (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
        };
        Some(OrganizationProcessSample {
            cpu_time_100ns: filetime(kernel).saturating_add(filetime(user)),
            working_set_bytes: counters.WorkingSetSize as u64,
            private_bytes: counters.PrivateUsage as u64,
        })
    }

    #[cfg(not(target_os = "windows"))]
    fn organization_process_sample() -> Option<OrganizationProcessSample> {
        None
    }

    fn print_organization_execution_database_profile(
        conn: &rusqlite::Connection,
        path: &std::path::Path,
    ) {
        let sqlite_version: String = conn
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .expect("read SQLite version");
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("read journal mode");
        let synchronous: i64 = conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .expect("read synchronous mode");
        let temp_store: i64 = conn
            .query_row("PRAGMA temp_store", [], |row| row.get(0))
            .expect("read temp store");
        let mmap_size: i64 = conn
            .query_row("PRAGMA mmap_size", [], |row| row.get(0))
            .expect("read mmap size");
        let page_count: i64 = conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .expect("read page count");
        let page_size: i64 = conn
            .query_row("PRAGMA page_size", [], |row| row.get(0))
            .expect("read page size");
        let file_bytes = |suffix: &str| {
            let candidate = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
            std::fs::metadata(candidate)
                .map(|metadata| metadata.len())
                .unwrap_or(0)
        };
        println!(
            "[organization-plan-profile] database sqlite={sqlite_version} journal_mode={journal_mode} synchronous={synchronous} temp_store={temp_store} mmap_size={mmap_size} page_size={page_size} page_count={page_count} main_bytes={} wal_bytes={} shm_bytes={}",
            file_bytes(""),
            file_bytes("-wal"),
            file_bytes("-shm"),
        );
    }

    fn print_organization_execution_query_plans(
        conn: &rusqlite::Connection,
        execution_fixture: &std::path::Path,
    ) {
        use rusqlite::ToSql;

        fn print_plan(conn: &rusqlite::Connection, label: &str, sql: &str, params: &[&dyn ToSql]) {
            let explain_sql = format!("EXPLAIN QUERY PLAN {sql}");
            let mut statement = conn.prepare(&explain_sql).expect("prepare query plan");
            let details = statement
                .query_map(params, |row| row.get::<_, String>(3))
                .expect("read query plan")
                .collect::<Result<Vec<_>, _>>()
                .expect("collect query plan");
            println!("[organization-plan-profile] query_plan={label} details={details:?}");
        }

        let root = execution_fixture.to_string_lossy().replace('\\', "/");
        let root_prefix = format!("{root}/%");
        let windows_root_prefix = format!("{root}\\%");
        let source_path = format!("{root}/source-00000.txt");
        let scope_file_ids = (0..500)
            .map(|index| format!("bench-file-{index:05}"))
            .collect::<Vec<_>>();
        let scope_placeholders = (1..=scope_file_ids.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let mut scope_params = scope_file_ids
            .iter()
            .map(|id| id as &dyn ToSql)
            .collect::<Vec<_>>();
        scope_params.extend([
            &root as &dyn ToSql,
            &root_prefix as &dyn ToSql,
            &windows_root_prefix as &dyn ToSql,
        ]);
        let scope_sql = format!(
            "SELECT f.id FROM files AS f WHERE f.id IN ({scope_placeholders}) AND f.is_stale = 0
               AND (f.path = ?501 OR f.path LIKE ?502 ESCAPE '~' OR f.path LIKE ?503 ESCAPE '~')"
        );
        print_plan(
            conn,
            "selected_plan_items",
            "SELECT id, plan_id, ordinal, file_id_snapshot, source_path_snapshot,
                    source_name_snapshot, source_size_snapshot, source_mtime_snapshot,
                    source_is_dir_snapshot, proposal_fingerprint, proposal_kind,
                    proposed_target_directory, proposed_name, proposed_target_path,
                    decision, edited_name, validity, confidence, risk_level,
                    requires_confirmation, blocking_code, blocking_detail,
                    authoritative_preview_id, operation_log_id, execution_id, revision,
                    created_at, updated_at
             FROM organization_plan_items WHERE plan_id = ?1
               AND decision IN ('accepted', 'edited')",
            &[&"plan-bench"],
        );
        print_plan(
            conn,
            "file_scope_membership_500_ids",
            &scope_sql,
            &scope_params,
        );
        print_plan(
            conn,
            "global_identity_for_semantic_proposal",
            "SELECT entry.id, entry.volume_id, entry.platform_file_id, entry.name,
                    entry.path, entry.extension, entry.is_directory, entry.size,
                    entry.modified_at_fs, entry.is_stale, volume.enabled
             FROM global_entries entry JOIN global_volumes volume ON volume.id = entry.volume_id
             WHERE entry.path_normalized = ?1 ORDER BY entry.id LIMIT 2",
            &[&source_path],
        );
        print_plan(
            conn,
            "execution_item_snapshot",
            "SELECT file_id_snapshot, source_name_snapshot, validity, decision
             FROM organization_plan_items WHERE id = ?1 AND plan_id = ?2",
            &[&"bench-item-00000", &"plan-bench"],
        );
        print_plan(
            conn,
            "execution_item_claim",
            "UPDATE organization_plan_items SET validity = 'executing',
                    execution_id = 'profile', revision = revision + 1, updated_at = 1
             WHERE id = 'bench-item-00000' AND validity IN ('ready', 'needs_review')",
            &[],
        );
    }

    #[test]
    #[ignore = "Task 06 100/1k/10k plan ledger, review, dry-run, refresh, WAL and prune benchmark"]
    fn performance_task06_plan_100_1k_10k_repository() {
        use std::time::Instant;

        for count in [100_usize, 1_000, 10_000] {
            let (db, path) = test_database();
            let create_start = Instant::now();
            {
                let mut conn = db.conn().expect("benchmark seed connection");
                let tx = conn.transaction().expect("benchmark seed transaction");
                tx.execute(
                    "INSERT INTO organization_plans (
                        id, title, status, source_kind, source_snapshot_revision,
                        requested_count, materialized_count, planner_version, revision,
                        created_at, updated_at, ready_at
                     ) VALUES ('plan-bench', 'Benchmark', 'ready', 'explicit', 1, ?1, ?1, 1, 1, 1, 1, 1)",
                    params![count as i64],
                )
                .expect("seed benchmark plan");
                tx.execute(
                    "INSERT INTO scan_roots (
                        id, normalized_path, display_name, source_kind, enabled,
                        health_status, current_generation, needs_reconciliation,
                        created_at, updated_at
                     ) VALUES ('organization-benchmark-root', '/missing',
                               'Organization benchmark', 'file_library', 1,
                               'healthy', 1, 0, 1, 1)",
                    [],
                )
                .expect("seed benchmark managed root");
                {
                    let mut insert = tx
                        .prepare(
                            "INSERT INTO organization_plan_items (
                                id, plan_id, ordinal, file_id_snapshot, source_path_snapshot,
                                source_name_snapshot, source_size_snapshot, source_mtime_snapshot,
                                source_is_dir_snapshot, proposal_fingerprint, proposal_kind,
                                proposed_target_directory, proposed_name, proposed_target_path,
                                decision, validity, confidence, risk_level, requires_confirmation,
                                authoritative_preview_id, revision, created_at, updated_at
                             ) VALUES (?1, 'plan-bench', ?2, ?3, ?4, ?5, 1, 1, 0, ?6,
                                       'rename', '/missing', ?7, ?8, 'undecided', 'ready',
                                       0.95, 'Normal', 0, ?9, 1, 1, 1)",
                        )
                        .expect("prepare item insert");
                    for ordinal in 0..count {
                        let file_id = format!("bench-file-{ordinal:05}");
                        let preview_id = if count <= 1_000 {
                            let digest = blake3::hash(file_id.as_bytes()).to_hex().to_string();
                            format!("op-{}", &digest[..16])
                        } else {
                            format!("preview-{ordinal:05}")
                        };
                        insert
                            .execute(params![
                                format!("bench-item-{ordinal:05}"),
                                ordinal as i64,
                                &file_id,
                                format!("/missing/source-{ordinal:05}.txt"),
                                format!("source-{ordinal:05}.txt"),
                                format!("fingerprint-{ordinal:05}"),
                                format!("renamed-{ordinal:05}.txt"),
                                format!("/missing/renamed-{ordinal:05}.txt"),
                                preview_id,
                            ])
                            .expect("insert benchmark item");
                    }
                }
                if count <= 1_000 {
                    let mut insert = tx
                        .prepare(
                            "INSERT INTO files (
                                id, path, name, extension, size, mtime,
                                suggested_action, suggested_target_path, suggested_name,
                                confidence, classification_status, last_seen_at
                             ) VALUES (?1, ?2, ?3, 'txt', 1, 1, 'Rename', '/missing', ?4,
                                       0.95, 'classified', 1)",
                        )
                        .expect("prepare benchmark indexed file insert");
                    for ordinal in 0..count {
                        insert
                            .execute(params![
                                format!("bench-file-{ordinal:05}"),
                                format!("/missing/source-{ordinal:05}.txt"),
                                format!("source-{ordinal:05}.txt"),
                                format!("renamed-{ordinal:05}.txt"),
                            ])
                            .expect("insert benchmark indexed file");
                    }
                }
                tx.commit().expect("publish benchmark plan");
            }
            let create_ms = create_start.elapsed().as_secs_f64() * 1000.0;
            if count == 1_000 {
                assert!(create_ms <= 500.0, "1k plan ledger create {create_ms:.3}ms");
            }
            if count == 10_000 {
                assert!(
                    create_ms <= 3_000.0,
                    "10k plan ledger create {create_ms:.3}ms"
                );
            }

            if count <= 1_000 {
                seed_performance_current_managed_semantics(&db, count);
            }

            if count <= 1_000 {
                let mut conn = db.conn().expect("sync benchmark proposals connection");
                let tx = conn
                    .transaction()
                    .expect("sync benchmark proposals transaction");
                for ordinal in 0..count {
                    let file_id = format!("bench-file-{ordinal:05}");
                    let item_id = format!("bench-item-{ordinal:05}");
                    let file = load_indexed_file_by_id(&tx, &file_id)
                        .expect("load benchmark proposal file")
                        .expect("benchmark proposal file exists");
                    let projection = current_organization_projection(&tx, &file)
                        .expect("resolve benchmark current semantic proposal");
                    let preview = projection
                        .preview
                        .expect("benchmark semantic proposal has authoritative preview");
                    tx.execute(
                        "UPDATE organization_plan_items SET proposal_fingerprint = ?1,
                                authoritative_preview_id = ?2 WHERE id = ?3",
                        params![projection.proposal.fingerprint, preview.id, item_id],
                    )
                    .expect("bind benchmark proposal facts");
                }
                tx.commit().expect("publish benchmark proposal facts");
            }

            let first_start = Instant::now();
            let first = db
                .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                    plan_id: "plan-bench".into(),
                    cursor: None,
                    page_size: 100,
                })
                .expect("benchmark first page");
            let first_ms = first_start.elapsed().as_secs_f64() * 1000.0;
            assert!(first_ms <= 100.0, "first page {first_ms:.3}ms");
            let group_start = Instant::now();
            let groups = db
                .query_organization_plan_groups(QueryOrganizationPlanGroupsRequest {
                    plan_id: "plan-bench".into(),
                    cursor: None,
                    page_size: 200,
                })
                .expect("benchmark group projection");
            let group_ms = group_start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(
                groups
                    .groups
                    .iter()
                    .map(|group| group.item_count)
                    .sum::<i64>(),
                count as i64
            );
            if count == 10_000 {
                assert!(group_ms <= 1_000.0, "10k group projection {group_ms:.3}ms");
            }
            if count == 100 {
                let conn = db.conn().expect("organization query plan connection");
                for (sql, expected_index) in [
                    (
                        "EXPLAIN QUERY PLAN SELECT id FROM organization_plans \
                         WHERE status = 'completed' ORDER BY updated_at DESC, id LIMIT 20",
                        "idx_organization_plans_status_updated",
                    ),
                    (
                        "EXPLAIN QUERY PLAN SELECT id FROM organization_plan_items \
                         WHERE plan_id = 'plan-bench' AND validity = 'ready' \
                           AND decision = 'accepted' ORDER BY ordinal, id LIMIT 1000",
                        "idx_organization_plan_items_plan_state",
                    ),
                    (
                        "EXPLAIN QUERY PLAN SELECT id FROM organization_plan_items \
                         WHERE file_id_snapshot = 'bench-file-00000'",
                        "idx_organization_plan_items_file",
                    ),
                    (
                        "EXPLAIN QUERY PLAN SELECT id FROM organization_plan_items \
                         WHERE execution_id = 'execution-bench' AND validity = 'executing'",
                        "idx_organization_plan_items_execution",
                    ),
                ] {
                    let plan = conn
                        .prepare(sql)
                        .expect("prepare organization query plan")
                        .query_map([], |row| row.get::<_, String>(3))
                        .expect("read organization query plan")
                        .collect::<Result<Vec<_>, _>>()
                        .expect("collect organization query plan");
                    assert!(
                        plan.iter().any(|detail| detail.contains(expected_index)),
                        "{expected_index} must serve its Task 06 read path: {plan:?}"
                    );
                }
            }

            let deep_start = Instant::now();
            let mut cursor = first.next_cursor;
            let mut last_page = first.items;
            while let Some(next) = cursor {
                let page = db
                    .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                        plan_id: "plan-bench".into(),
                        cursor: Some(next),
                        page_size: 200,
                    })
                    .expect("benchmark keyset page");
                cursor = page.next_cursor;
                last_page = page.items;
            }
            assert!(!last_page.is_empty());
            let deep_ms = deep_start.elapsed().as_secs_f64() * 1000.0;

            let page = db
                .query_organization_plan_items(QueryOrganizationPlanItemsRequest {
                    plan_id: "plan-bench".into(),
                    cursor: None,
                    page_size: 200,
                })
                .expect("mutation source page");
            let all_items = {
                let conn = db.conn().expect("all mutation rows");
                let mut stmt = conn
                    .prepare("SELECT id, revision FROM organization_plan_items WHERE plan_id = 'plan-bench' ORDER BY ordinal")
                    .expect("prepare all mutation rows");
                stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .expect("query mutation rows")
                .collect::<Result<Vec<_>, _>>()
                .expect("collect mutation rows")
            };
            assert_eq!(page.plan_revision, 1);
            let decision_start = Instant::now();
            // The 10k case keeps the original missing-source benchmark shape so
            // its ledger/group/refresh thresholds remain comparable. Acceptance
            // is exercised with authoritative files and previews at 100/1k.
            let benchmark_decision = if count == 10_000 { "kept" } else { "accepted" };
            let changed = db
                .update_organization_plan_decisions(UpdateOrganizationPlanDecisionsRequest {
                    plan_id: "plan-bench".into(),
                    expected_plan_revision: 1,
                    safe_batch: false,
                    mutations: all_items
                        .into_iter()
                        .map(|(item_id, revision)| OrganizationDecisionMutation {
                            item_id,
                            expected_item_revision: revision,
                            decision: benchmark_decision.into(),
                            edited_filename: None,
                        })
                        .collect(),
                })
                .expect("benchmark batch decision");
            let decision_ms = decision_start.elapsed().as_secs_f64() * 1000.0;
            if count == 10_000 {
                assert!(decision_ms <= 500.0, "10k decision {decision_ms:.3}ms");
            }

            let execution_fixture = path.with_extension("task06-execution-files");
            if count == 1_000 {
                std::fs::create_dir_all(&execution_fixture)
                    .expect("create execution benchmark fixture");
                let mut conn = db.conn().expect("execution benchmark seed");
                let tx = conn.transaction().expect("execution benchmark transaction");
                tx.execute(
                    "INSERT INTO scan_roots (
                        id, normalized_path, display_name, source_kind, enabled,
                        health_status, current_generation, needs_reconciliation,
                        created_at, updated_at
                    ) VALUES ('organization-benchmark-execution-root', ?1,
                               'Organization benchmark', 'file_library', 1,
                               'healthy', 1, 0, 1, 1)",
                    params![execution_fixture.to_string_lossy().replace('\\', "/")],
                )
                .expect("seed authoritative execution root");
                for ordinal in 0..count {
                    let file_id = format!("bench-file-{ordinal:05}");
                    let item_id = format!("bench-item-{ordinal:05}");
                    let source_name = format!("source-{ordinal:05}.txt");
                    let target_name = format!("renamed-{ordinal:05}.txt");
                    let source_path = execution_fixture.join(&source_name);
                    let target_path = execution_fixture.join(&target_name);
                    std::fs::write(&source_path, b"x").expect("write execution benchmark source");
                    tx.execute(
                        "UPDATE files SET path = ?2, name = ?3, extension = 'txt',
                                size = 1, mtime = 1, is_stale = 0
                         WHERE id = ?1",
                        params![
                            file_id,
                            source_path.to_string_lossy().replace('\\', "/"),
                            source_name,
                        ],
                    )
                    .expect("seed indexed execution source");
                    tx.execute(
                        "UPDATE files SET classification_status = 'classified',
                                suggested_action = 'Rename', suggested_target_path = ?2,
                                suggested_name = ?3, confidence = 0.95,
                                risk_level = 'Normal', last_classified_mtime = 1,
                                last_classified_size = 1
                         WHERE id = ?1",
                        params![
                            file_id,
                            execution_fixture.to_string_lossy().replace('\\', "/"),
                            target_name,
                        ],
                    )
                    .expect("seed authoritative execution proposal");
                    let source_path_text = source_path.to_string_lossy().replace('\\', "/");
                    let source_path_normalized =
                        crate::global_index::models::normalize_path(&source_path_text);
                    tx.execute(
                        "UPDATE global_entries
                         SET path = ?2, path_normalized = ?3, last_seen_at = 2
                         WHERE id = ?1",
                        params![
                            format!("bench-global-{ordinal:05}"),
                            source_path_text,
                            source_path_normalized,
                        ],
                    )
                    .expect("move benchmark global identity with execution fixture");
                    let indexed = load_indexed_file_by_id(&tx, &file_id)
                        .expect("load authoritative execution file")
                        .expect("authoritative execution file exists");
                    let projection = current_organization_projection(&tx, &indexed)
                        .expect("resolve authoritative execution semantic proposal");
                    let preview = projection
                        .preview
                        .clone()
                        .expect("authoritative semantic execution preview");
                    let proposal = projection.proposal;
                    tx.execute(
                        "UPDATE organization_plan_items SET
                            source_path_snapshot = ?2, source_name_snapshot = ?3,
                            proposed_target_directory = ?4, proposed_name = ?5,
                            proposed_target_path = ?6, proposal_fingerprint = ?7,
                            proposal_kind = ?8, validity = ?9,
                            confidence = ?10, risk_level = ?11,
                            requires_confirmation = ?12,
                            authoritative_preview_id = ?13
                         WHERE id = ?1",
                        params![
                            item_id,
                            source_path.to_string_lossy().replace('\\', "/"),
                            source_name,
                            execution_fixture.to_string_lossy().replace('\\', "/"),
                            target_name,
                            target_path.to_string_lossy().replace('\\', "/"),
                            proposal.fingerprint,
                            proposal.kind,
                            proposal.validity,
                            proposal.confidence,
                            proposal.risk,
                            i64::from(proposal.requires_confirmation),
                            preview.id,
                        ],
                    )
                    .expect("bind execution benchmark item");
                }
                tx.commit().expect("publish execution benchmark fixture");
            }

            let mut dry_ms = None;
            let mut dry_run_for_execution = None;
            if count <= 1_000 {
                start_organization_execution_profile();
                let dry_process_before = organization_process_sample();
                let dry_start = Instant::now();
                let dry = db
                    .get_organization_plan_dry_run(OrganizationPlanSelectionRequest {
                        plan_id: "plan-bench".into(),
                        expected_plan_revision: changed.revision,
                        item_ids: Vec::new(),
                        all_accepted: true,
                    })
                    .expect("benchmark dry run");
                dry_ms = Some(dry_start.elapsed().as_secs_f64() * 1000.0);
                let dry_process_after = organization_process_sample();
                let dry_process_cpu_ms =
                    dry_process_before
                        .zip(dry_process_after)
                        .map(|(before, after)| {
                            after.cpu_time_100ns.saturating_sub(before.cpu_time_100ns) as f64
                                / 10_000.0
                        });
                print_organization_execution_profile(
                    &format!("standalone_dry_run_{count}"),
                    take_organization_execution_profile(),
                    dry_ms.unwrap(),
                    dry_process_cpu_ms,
                    dry_process_before,
                    dry_process_after,
                );
                assert_eq!(dry.selected_count, count as i64);
                if count == 1_000 {
                    assert!(dry_ms.unwrap() <= 1_000.0, "1k dry run {dry_ms:?}ms");
                    assert_eq!(dry.executable_count, 1_000);
                    dry_run_for_execution = Some(dry);
                }
            }

            let wal_reader = Connection::open(&path).expect("benchmark WAL reader");
            assert_eq!(
                wal_reader
                    .query_row(
                        "SELECT COUNT(*) FROM organization_plan_items WHERE plan_id = 'plan-bench'",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .expect("WAL read"),
                count as i64
            );

            let mut execution_ms = None;
            let refresh_ms = if let Some(dry) = dry_run_for_execution {
                if count == 1_000 {
                    start_organization_execution_profile();
                }
                let process_before = if count == 1_000 {
                    organization_process_sample()
                } else {
                    None
                };
                let execution_start = Instant::now();
                let dispatch = db
                    .begin_organization_plan_execution(&ExecuteOrganizationPlanRequest {
                        plan_id: "plan-bench".into(),
                        expected_plan_revision: changed.revision,
                        dry_run_fingerprint: dry.dry_run_fingerprint,
                        item_ids: Vec::new(),
                        all_accepted: true,
                        confirmed: true,
                    })
                    .expect("benchmark execution preparation");
                execution_ms = Some(execution_start.elapsed().as_secs_f64() * 1000.0);
                if count == 1_000 {
                    let process_after = organization_process_sample();
                    let execution_wall_ms = execution_ms.unwrap();
                    let process_cpu_ms =
                        process_before.zip(process_after).map(|(before, after)| {
                            after.cpu_time_100ns.saturating_sub(before.cpu_time_100ns) as f64
                                / 10_000.0
                        });
                    print_organization_execution_profile(
                        "execution_1k",
                        take_organization_execution_profile(),
                        execution_wall_ms,
                        process_cpu_ms,
                        process_before,
                        process_after,
                    );
                    let selected_count = dispatch.item_ids.len();
                    let file_id_chunks = selected_count.div_ceil(500);
                    let scope_sql_statements = file_id_chunks * 3;
                    let sql_statements_per_dry_run =
                        3 + file_id_chunks + scope_sql_statements + selected_count * 6;
                    let execution_sql_statements =
                        sql_statements_per_dry_run + 1 + 2 * selected_count + 1;
                    println!(
                        "[organization-plan-profile] query_counts source_derived=true internal_dry_run_builds=1 selected_items={selected_count} file_id_chunks_per_build={file_id_chunks} scope_validation_batches_per_build={file_id_chunks} scope_sql_statements_per_batch=3 semantic_proposal_calls_per_build={selected_count} semantic_sql_per_proposal=6 execution_item_snapshot_selects={selected_count} execution_item_claim_updates={selected_count} plan_claim_updates=1 estimated_execution_sql_statements={execution_sql_statements} filesystem_exists_checks_per_build={}",
                        selected_count * 2,
                    );
                    let conn = db.conn().expect("profile organization database");
                    print_organization_execution_database_profile(&conn, &path);
                    print_organization_execution_query_plans(&conn, &execution_fixture);
                }
                assert_eq!(dispatch.item_ids.len(), 1_000);
                assert!(
                    execution_ms.unwrap() <= 1_000.0,
                    "1k execution preparation {execution_ms:?}ms"
                );
                db.fail_unjournaled_organization_execution(
                    "plan-bench",
                    &dispatch,
                    "benchmark rollback",
                )
                .expect("close benchmark execution lease");
                0.0
            } else {
                let refresh_start = Instant::now();
                let refreshed = db
                    .refresh_organization_plan(OrganizationPlanRevisionRequest {
                        plan_id: "plan-bench".into(),
                        expected_plan_revision: changed.revision,
                    })
                    .expect("benchmark refresh");
                let elapsed = refresh_start.elapsed().as_secs_f64() * 1000.0;
                if count == 10_000 {
                    assert!(elapsed <= 3_000.0, "10k refresh {elapsed:.3}ms");
                }
                let expected_status = if count == 10_000 { "stale" } else { "ready" };
                assert_eq!(refreshed.status, expected_status);
                elapsed
            };

            {
                let conn = db.conn().expect("terminal benchmark plan");
                conn.execute(
                    "UPDATE organization_plans SET status = 'completed', updated_at = 0,
                        completed_at = 0 WHERE id = 'plan-bench'",
                    [],
                )
                .expect("terminalize benchmark plan");
                for ordinal in 0..100 {
                    conn.execute(
                        "INSERT INTO organization_plans (
                            id, title, status, source_kind, source_snapshot_revision,
                            requested_count, materialized_count, planner_version, revision,
                            created_at, updated_at, completed_at
                         ) VALUES (?1, 'Retention fixture', 'completed', 'explicit', 1,
                                   0, 0, 1, 1, 1, 1, 1)",
                        params![format!("retained-plan-{ordinal:03}")],
                    )
                    .expect("seed retained terminal plan");
                }
            }
            // Task 07 keeps the Task 06 retention contract fail-closed while
            // pruning the age/count candidate union in bounded batches.
            assert_eq!(db.prune_organization_plans().expect("benchmark prune"), 20);
            println!(
                "Task 06 plan items={count} create_ms={create_ms:.3} first_page_ms={first_ms:.3} deep_keyset_ms={deep_ms:.3} decision_ms={decision_ms:.3} dry_ms={dry_ms:?} execution_ms={execution_ms:?} refresh_ms={refresh_ms:.3}"
            );
            drop(wal_reader);
            drop(db);
            let _ = std::fs::remove_file(path);
            let _ = std::fs::remove_dir_all(execution_fixture);
        }
    }
}
