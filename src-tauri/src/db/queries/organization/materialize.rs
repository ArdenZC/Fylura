//! The existing Organization Plan builder, shared by ordinary Organize and
//! atomic manual Automation receipt creation. No execution is exposed here.
use super::*;

pub(crate) fn materialize_organization_plan(
    tx: &Connection,
    request: CreateOrganizationPlanRequestV1,
) -> Result<OrganizationPlanDto, DbError> {
    validate_create_request(&request)?;
    let library_revision = current_library_revision(tx)?;
    let (where_sql, where_params, missing_count, _, source_fingerprint) =
        selection_where(tx, &request.source, library_revision)?;
    if missing_count != 0 {
        return Err(DbError::Validation(
            "organization_plan_source_missing".to_string(),
        ));
    }
    let sql = format!(
            "SELECT f.id, f.path, f.name, f.extension, f.size, f.mtime, f.ctime, f.is_dir, f.state_code, \
                    f.file_type, f.purpose, f.lifecycle, f.context, f.risk_level, f.suggested_action, \
                    f.suggested_target_path, f.suggested_name, f.confidence, f.classification_reason, \
                    f.classification_status, f.matched_rules, f.requires_confirmation, f.content_hash, \
                    EXISTS (SELECT 1 FROM active_duplicate_membership AS membership WHERE membership.file_id = f.id), \
                    f.is_stale, f.last_seen_at, f.last_classified_at, f.classified_rule_version, \
                    f.last_classified_mtime, f.last_classified_size \
             FROM files AS f WHERE {where_sql} ORDER BY f.id LIMIT ?"
        );
    let mut query_params = where_params;
    query_params.push(rusqlite::types::Value::Integer(
        ORGANIZATION_PLAN_MAX_ITEMS as i64 + 1,
    ));
    let source_rows = {
        let mut stmt = tx.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(query_params.iter()), indexed_file_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    materialize_rows(
        tx,
        request,
        source_rows,
        library_revision,
        source_fingerprint,
        None,
    )
}

pub(super) fn materialize_rows(
    tx: &Connection,
    request: CreateOrganizationPlanRequestV1,
    source_rows: Vec<IndexedFileRow>,
    library_revision: i64,
    source_fingerprint: Option<String>,
    one_off: Option<&super::one_off::CapturedFolder>,
) -> Result<OrganizationPlanDto, DbError> {
    if source_rows.len() > ORGANIZATION_PLAN_MAX_ITEMS {
        return Err(DbError::Validation(
            "organization_plan_too_large".to_string(),
        ));
    }
    let materialized_count = source_rows.len() as i64;
    if request
        .expected_count
        .is_some_and(|expected| expected != materialized_count)
    {
        return Err(DbError::Validation(
            "organization_plan_expected_count_mismatch".to_string(),
        ));
    }

    let now = current_unix_seconds();
    let plan_id = format!("organization-plan-{}", uuid::Uuid::new_v4());
    let title = request
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("Organization plan")
        .to_string();
    let (source_kind, source_query_json, source_snapshot_revision) = match &request.source {
        LibrarySelectionV1::Explicit { .. } => ("explicit", None, library_revision),
        LibrarySelectionV1::AllMatching {
            query,
            snapshot_revision,
            ..
        } => (
            "all_matching",
            Some(serde_json::to_string(query.as_ref())?),
            *snapshot_revision,
        ),
    };
    tx.execute(
        "INSERT INTO organization_plans (
                id, title, status, source_kind, source_query_spec_json,
                source_query_fingerprint, source_snapshot_revision, requested_count,
                materialized_count, planner_version, revision, created_at, updated_at
             ) VALUES (?1, ?2, 'building', ?3, ?4, ?5, ?6, ?7, 0, 1, 1, ?8, ?8)",
        params![
            plan_id,
            title,
            source_kind,
            source_query_json,
            source_fingerprint,
            source_snapshot_revision,
            request.expected_count.unwrap_or(materialized_count),
            now
        ],
    )?;

    if let Some(source) = one_off {
        super::one_off::persist_sources(tx, &plan_id, source)?;
    }
    for (ordinal, row) in source_rows.into_iter().enumerate() {
        let source_id = row.id.clone();
        let source_path = row.path.clone();
        let source_name = row.name.clone();
        let source_size = row.size;
        let source_mtime = row.mtime;
        let source_is_dir = row.is_dir;
        let proposal = current_organization_proposal(tx, &row)?;
        tx.execute(
            "INSERT INTO organization_plan_items (
                    id, plan_id, ordinal, file_id_snapshot, source_path_snapshot,
                    source_name_snapshot, source_size_snapshot, source_mtime_snapshot,
                    source_is_dir_snapshot, proposal_fingerprint, proposal_kind,
                    proposed_target_directory, proposed_name, proposed_target_path,
                    decision, edited_name, validity, confidence, risk_level,
                    requires_confirmation, blocking_code, blocking_detail,
                    authoritative_preview_id, revision, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                           ?13, ?14, 'undecided', NULL, ?15, ?16, ?17, ?18, ?19,
                           ?20, ?21, 1, ?22, ?22)",
            params![
                format!("organization-item-{}", uuid::Uuid::new_v4()),
                plan_id,
                ordinal as i64,
                source_id,
                source_path,
                source_name,
                source_size,
                source_mtime,
                i64::from(source_is_dir),
                proposal.fingerprint,
                proposal.kind,
                proposal.target_directory,
                proposal.name,
                proposal.target_path,
                proposal.validity,
                proposal.confidence,
                proposal.risk,
                i64::from(proposal.requires_confirmation),
                proposal.blocking_code,
                proposal.blocking_detail,
                proposal.preview_id,
                now
            ],
        )?;
    }
    clear_temp_selection_ids(tx)?;
    tx.execute(
        "UPDATE organization_plans SET status = 'ready', materialized_count = ?2,
                    ready_at = ?3, updated_at = ?3 WHERE id = ?1 AND status = 'building'",
        params![plan_id, materialized_count, now],
    )?;
    let plan = load_plan(tx, &plan_id)?;
    Ok(plan)
}
