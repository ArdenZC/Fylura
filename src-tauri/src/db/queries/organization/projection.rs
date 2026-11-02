use super::cursor::OrganizationGroupCursor;
use super::*;
use rusqlite::{params, params_from_iter};
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub(super) struct OrganizationPlanGroupKey {
    target_directory: String,
    proposal_kind: String,
    readiness: String,
    risk_level: String,
}

#[derive(Debug, Default)]
struct OrganizationPlanGroupAccumulator {
    key: Option<OrganizationPlanGroupKey>,
    group_id: String,
    item_count: i64,
    total_bytes: i64,
    accepted_count: i64,
    excluded_count: i64,
    stale_count: i64,
    conflict_count: i64,
    all_high_confidence: bool,
    all_medium_confidence: bool,
    all_low_confidence: bool,
    review_reason_counts: BTreeMap<String, i64>,
    available_actions: HashSet<String>,
    can_accept_all: bool,
    can_keep_all: bool,
    can_clear_all: bool,
    fingerprint_members: Vec<OrganizationGroupFingerprintMember>,
    sample_items: Vec<OrganizationPlanGroupSampleDto>,
}

#[derive(Debug, Clone)]
pub(super) struct OrganizationItemProjection {
    pub(super) item: OrganizationPlanItemDto,
    current_file: Option<IndexedFileRow>,
    managed_scope_membership: Option<bool>,
}

#[derive(Debug, Clone)]
pub(super) struct OrganizationPlanGroupProjection {
    pub(super) items: Vec<OrganizationItemProjection>,
    pub(super) groups: Vec<OrganizationPlanGroupSummaryDto>,
    pub(super) projection_fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OrganizationGroupFingerprintMember {
    item_id: String,
    item_revision: i64,
    effective_readiness: String,
    decision: String,
    authoritative_preview_id: Option<String>,
    current_source_path: Option<String>,
    current_size: Option<i64>,
    current_mtime: Option<i64>,
    current_is_dir: Option<bool>,
    available_actions: Vec<String>,
    proposal_fingerprint: String,
    blocking_code: Option<String>,
    managed_scope_membership: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OrganizationGroupFingerprintPayload<'a> {
    version: &'static str,
    plan_id: &'a str,
    plan_revision: i64,
    group_id: &'a str,
    target_directory: &'a str,
    proposal_kind: &'a str,
    readiness: &'a str,
    risk_level: &'a str,
    members: Vec<OrganizationGroupFingerprintMember>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OrganizationPlanGroupProjectionFingerprintPayload<'a> {
    version: &'static str,
    plan_id: &'a str,
    plan_revision: i64,
    groups: Vec<OrganizationPlanGroupProjectionFingerprintEntry<'a>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OrganizationPlanGroupProjectionFingerprintEntry<'a> {
    group_id: &'a str,
    projection_fingerprint: &'a str,
}

pub(super) fn load_organization_item_projections(
    conn: &rusqlite::Connection,
    plan_id: &str,
) -> Result<Vec<OrganizationItemProjection>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, plan_id, ordinal, file_id_snapshot, source_path_snapshot,
                source_name_snapshot, source_size_snapshot, source_mtime_snapshot,
                source_is_dir_snapshot, proposal_fingerprint, proposal_kind,
                proposed_target_directory, proposed_name, proposed_target_path,
                decision, edited_name, validity, confidence, risk_level,
                requires_confirmation, blocking_code, blocking_detail,
                authoritative_preview_id, operation_log_id, execution_id, revision,
                created_at, updated_at
         FROM organization_plan_items WHERE plan_id = ?1 ORDER BY ordinal, id",
    )?;
    let items = stmt
        .query_map(params![plan_id], item_from_row)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(DbError::from)?;
    let file_ids = items
        .iter()
        .map(|item| item.file_id_snapshot.clone())
        .collect::<Vec<_>>();
    let current_files = load_indexed_files_for_projection(conn, &file_ids)?;
    let scope = load_organization_scope_projection(conn, plan_id);
    let scope_memberships = organization_scope_memberships(conn, &scope, &file_ids);
    let mut projections = Vec::with_capacity(items.len());
    for mut item in items {
        let scope_membership = scope_memberships
            .get(&item.file_id_snapshot)
            .copied()
            .unwrap_or(Some(false));
        let current_file = current_files.get(&item.file_id_snapshot).cloned();
        decorate_organization_item_metadata_with_file(
            conn,
            &mut item,
            current_file.as_ref(),
            scope_membership,
        )?;
        projections.push(OrganizationItemProjection {
            item,
            current_file,
            managed_scope_membership: scope_membership,
        });
    }
    Ok(projections)
}

pub(super) fn load_organization_plan_items_for_projection(
    conn: &rusqlite::Connection,
    plan_id: &str,
) -> Result<Vec<OrganizationPlanItemDto>, DbError> {
    Ok(load_organization_item_projections(conn, plan_id)?
        .into_iter()
        .map(|projection| projection.item)
        .collect())
}

pub(super) fn load_indexed_files_for_projection(
    conn: &rusqlite::Connection,
    file_ids: &[String],
) -> Result<HashMap<String, IndexedFileRow>, DbError> {
    const FILE_ID_CHUNK: usize = 500;
    let mut files = HashMap::with_capacity(file_ids.len());
    for chunk in file_ids.chunks(FILE_ID_CHUNK) {
        let placeholders = std::iter::repeat_n("?", chunk.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT f.id, f.path, f.name, f.extension, f.size, f.mtime, f.ctime, f.is_dir, f.state_code,
                    f.file_type, f.purpose, f.lifecycle, f.context, f.risk_level, f.suggested_action,
                    f.suggested_target_path, f.suggested_name, f.confidence, f.classification_reason,
                    f.classification_status, f.matched_rules, f.requires_confirmation, f.content_hash,
                    EXISTS (SELECT 1 FROM active_duplicate_membership AS membership
                            WHERE membership.file_id = f.id),
                    f.is_stale, f.last_seen_at, f.last_classified_at, f.classified_rule_version,
                    f.last_classified_mtime, f.last_classified_size
             FROM files AS f WHERE f.id IN ({placeholders}) AND f.is_stale = 0"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(chunk.iter()), indexed_file_from_row)?;
        for row in rows {
            let file = row?;
            files.insert(file.id.clone(), file);
        }
    }
    for id in file_ids
        .iter()
        .filter(|id| super::one_off::is_source_id(id))
    {
        if let Some(row) = super::one_off::load_source(conn, id)? {
            files.insert(id.clone(), row);
        }
    }
    Ok(files)
}

pub(super) fn organization_group_key(item: &OrganizationPlanItemDto) -> OrganizationPlanGroupKey {
    let readiness = organization_group_readiness(item);
    OrganizationPlanGroupKey {
        target_directory: item.proposed_target_directory.clone(),
        proposal_kind: item.proposal_kind.clone(),
        readiness: readiness.to_string(),
        risk_level: if item.risk_level.trim().is_empty() {
            "Unknown".to_string()
        } else {
            item.risk_level.clone()
        },
    }
}

fn organization_group_readiness(item: &OrganizationPlanItemDto) -> String {
    item.effective_readiness.clone()
}

pub(super) fn organization_group_id(plan_id: &str, key: &OrganizationPlanGroupKey) -> String {
    let digest = blake3::hash(
        [
            "organization-plan-group-v1",
            plan_id,
            &key.target_directory,
            &key.proposal_kind,
            &key.readiness,
            &key.risk_level,
        ]
        .join("\0")
        .as_bytes(),
    );
    format!("organization-group-{}", digest.to_hex())
}

fn organization_group_label(key: &OrganizationPlanGroupKey) -> String {
    if key.target_directory.trim().is_empty() {
        key.proposal_kind.clone()
    } else {
        format!("{} · {}", key.target_directory, key.proposal_kind)
    }
}

fn organization_group_is_conflict(item: &OrganizationPlanItemDto) -> bool {
    item.blocking_code
        .as_deref()
        .is_some_and(|code| matches!(code, "organization_target_collision" | "target_collision"))
}

pub(super) fn load_organization_plan_group_projection(
    conn: &rusqlite::Connection,
    plan_id: &str,
    revision: i64,
) -> Result<OrganizationPlanGroupProjection, DbError> {
    #[cfg(test)]
    ORGANIZATION_FULL_PROJECTION_COUNT.with(|count| count.set(count.get() + 1));
    let items = load_organization_item_projections(conn, plan_id)?;
    let groups = build_organization_plan_group_summaries(plan_id, revision, &items);
    let projection_fingerprint =
        organization_plan_group_projection_fingerprint(plan_id, revision, &groups);
    Ok(OrganizationPlanGroupProjection {
        items,
        groups,
        projection_fingerprint,
    })
}

fn build_organization_plan_group_summaries(
    plan_id: &str,
    revision: i64,
    items: &[OrganizationItemProjection],
) -> Vec<OrganizationPlanGroupSummaryDto> {
    let mut groups = HashMap::<OrganizationPlanGroupKey, OrganizationPlanGroupAccumulator>::new();
    for projection in items {
        let item = &projection.item;
        let key = organization_group_key(item);
        let group_id = organization_group_id(plan_id, &key);
        let entry = groups
            .entry(key.clone())
            .or_insert_with(|| OrganizationPlanGroupAccumulator {
                key: Some(key.clone()),
                group_id,
                all_high_confidence: true,
                all_medium_confidence: true,
                all_low_confidence: true,
                can_accept_all: true,
                can_keep_all: true,
                can_clear_all: true,
                ..OrganizationPlanGroupAccumulator::default()
            });
        entry.item_count += 1;
        entry.total_bytes = entry
            .total_bytes
            .saturating_add(item.source_size_snapshot.max(0));
        if matches!(item.decision.as_str(), "accepted" | "edited") {
            entry.accepted_count += 1;
        }
        if item.decision == "kept" {
            entry.excluded_count += 1;
        }
        if item.validity == "stale" {
            entry.stale_count += 1;
        }
        if organization_group_is_conflict(item) {
            entry.conflict_count += 1;
        }
        for reason in &item.review_reasons {
            *entry
                .review_reason_counts
                .entry(reason.clone())
                .or_insert(0) += 1;
        }
        for action in &item.available_actions {
            entry.available_actions.insert(action.clone());
        }
        entry.can_accept_all &= item
            .available_actions
            .iter()
            .any(|action| action == "accept_suggestion");
        entry.can_keep_all &= item.available_actions.iter().any(|action| action == "keep");
        entry.can_clear_all &= item
            .available_actions
            .iter()
            .any(|action| action == "clear_decision");
        let mut available_actions = item.available_actions.clone();
        available_actions.sort();
        entry
            .fingerprint_members
            .push(OrganizationGroupFingerprintMember {
                item_id: item.id.clone(),
                item_revision: item.revision,
                effective_readiness: item.effective_readiness.clone(),
                decision: item.decision.clone(),
                authoritative_preview_id: item.authoritative_preview_id.clone(),
                current_source_path: projection
                    .current_file
                    .as_ref()
                    .map(|file| file.path.clone()),
                current_size: projection.current_file.as_ref().map(|file| file.size),
                current_mtime: projection.current_file.as_ref().map(|file| file.mtime),
                current_is_dir: projection.current_file.as_ref().map(|file| file.is_dir),
                available_actions,
                proposal_fingerprint: item.proposal_fingerprint.clone(),
                blocking_code: item.blocking_code.clone(),
                managed_scope_membership: projection.managed_scope_membership,
            });
        entry.all_high_confidence &= item.confidence >= 0.8;
        entry.all_medium_confidence &= (0.5..0.8).contains(&item.confidence);
        entry.all_low_confidence &= item.confidence < 0.5;
        if entry.sample_items.len() < ORGANIZATION_GROUP_SAMPLE_MAX {
            entry.sample_items.push(OrganizationPlanGroupSampleDto {
                item_id: item.id.clone(),
                source_name: item.source_name_snapshot.clone(),
                reason: item.semantic_explanation.as_ref().map(|s| s.reason.clone()),
                source_path: item.source_path_snapshot.clone(),
                proposed_name: item.proposed_name.clone(),
                decision: item.decision.clone(),
                validity: item.validity.clone(),
            });
        }
    }

    let mut summaries = groups
        .into_values()
        .map(|accumulator| {
            let key = accumulator
                .key
                .expect("organization group accumulator always has a key");
            let confidence_band = if accumulator.all_high_confidence {
                "high"
            } else if accumulator.all_medium_confidence {
                "medium"
            } else if accumulator.all_low_confidence {
                "low"
            } else {
                "mixed"
            };
            let projection_fingerprint = organization_group_projection_fingerprint(
                plan_id,
                revision,
                &accumulator.group_id,
                &key,
                &accumulator.fingerprint_members,
            );
            Ok(OrganizationPlanGroupSummaryDto {
                group_id: accumulator.group_id,
                plan_id: plan_id.to_string(),
                label: organization_group_label(&key),
                target_directory: (!key.target_directory.trim().is_empty())
                    .then_some(key.target_directory),
                proposal_kind: key.proposal_kind,
                readiness: key.readiness,
                risk_level: key.risk_level,
                item_count: accumulator.item_count,
                total_bytes: accumulator.total_bytes,
                accepted_count: accumulator.accepted_count,
                excluded_count: accumulator.excluded_count,
                stale_count: accumulator.stale_count,
                conflict_count: accumulator.conflict_count,
                confidence_band: confidence_band.to_string(),
                review_reason_counts: accumulator
                    .review_reason_counts
                    .into_iter()
                    .map(|(reason, count)| OrganizationReviewReasonCountDto { reason, count })
                    .collect(),
                available_actions: sorted_organization_actions(accumulator.available_actions),
                group_actions: OrganizationPlanGroupActionsDto {
                    can_accept_all: accumulator.item_count > 0 && accumulator.can_accept_all,
                    can_keep_all: accumulator.item_count > 0 && accumulator.can_keep_all,
                    can_clear_all: accumulator.item_count > 0 && accumulator.can_clear_all,
                },
                projection_fingerprint,
                sample_items: accumulator.sample_items,
                revision,
            })
        })
        .collect::<Result<Vec<_>, DbError>>()
        .expect("organization group summary projection is infallible");
    summaries.sort_by(|left, right| {
        left.label
            .to_ascii_lowercase()
            .cmp(&right.label.to_ascii_lowercase())
            .then_with(|| left.group_id.cmp(&right.group_id))
    });
    summaries
}

fn organization_group_projection_fingerprint(
    plan_id: &str,
    revision: i64,
    group_id: &str,
    key: &OrganizationPlanGroupKey,
    members: &[OrganizationGroupFingerprintMember],
) -> String {
    let mut members = members.to_vec();
    members.sort_by(|left, right| left.item_id.cmp(&right.item_id));
    let payload = OrganizationGroupFingerprintPayload {
        version: "organization-group-projection-v1",
        plan_id,
        plan_revision: revision,
        group_id,
        target_directory: &key.target_directory,
        proposal_kind: &key.proposal_kind,
        readiness: &key.readiness,
        risk_level: &key.risk_level,
        members,
    };
    let bytes = serde_json::to_vec(&payload).expect("organization group fingerprint serializes");
    format!(
        "organization-group-projection-v1-{}",
        blake3::hash(&bytes).to_hex()
    )
}

fn organization_plan_group_projection_fingerprint(
    plan_id: &str,
    revision: i64,
    groups: &[OrganizationPlanGroupSummaryDto],
) -> String {
    let payload = OrganizationPlanGroupProjectionFingerprintPayload {
        version: ORGANIZATION_GROUP_PROJECTION_VERSION,
        plan_id,
        plan_revision: revision,
        groups: groups
            .iter()
            .map(|group| OrganizationPlanGroupProjectionFingerprintEntry {
                group_id: &group.group_id,
                projection_fingerprint: &group.projection_fingerprint,
            })
            .collect(),
    };
    let bytes = serde_json::to_vec(&payload)
        .expect("organization plan group projection fingerprint serializes");
    format!(
        "{}-{}",
        ORGANIZATION_GROUP_PROJECTION_VERSION,
        blake3::hash(&bytes).to_hex()
    )
}

pub(super) fn effective_summary_from_groups(
    groups: &[OrganizationPlanGroupSummaryDto],
) -> OrganizationPlanEffectiveSummaryDto {
    let mut summary = OrganizationPlanEffectiveSummaryDto::default();
    for group in groups {
        match group.readiness.as_str() {
            "ready" => summary.ready += group.item_count,
            "reviewed" => summary.reviewed += group.item_count,
            "requires-decision" => summary.pending_review += group.item_count,
            _ => summary.blocked += group.item_count,
        }
    }
    summary
}

fn sorted_organization_actions(actions: HashSet<String>) -> Vec<String> {
    let mut actions = actions.into_iter().collect::<Vec<_>>();
    actions.sort_by_key(|action| match action.as_str() {
        "accept_suggestion" => 0,
        "edit_name" => 1,
        "view_preview" => 2,
        "keep" => 3,
        "defer" => 4,
        "clear_decision" => 5,
        _ => 99,
    });
    actions
}

pub(super) fn organization_group_is_after(
    group: &OrganizationPlanGroupSummaryDto,
    cursor: &OrganizationGroupCursor,
) -> bool {
    group
        .label
        .to_ascii_lowercase()
        .cmp(&cursor.label.to_ascii_lowercase())
        .then_with(|| group.group_id.cmp(&cursor.group_id))
        == Ordering::Greater
}
