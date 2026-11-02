use super::cursor::{
    decode_group_cursor, decode_group_item_cursor, decode_item_cursor, encode_group_cursor,
    encode_group_item_cursor, encode_item_cursor, validate_organization_projection_fingerprint,
    OrganizationGroupCursor, OrganizationGroupItemCursor, OrganizationItemCursor,
};
use super::projection::{
    effective_summary_from_groups, load_organization_plan_group_projection, organization_group_id,
    organization_group_is_after, organization_group_key,
};
use super::*;
use rusqlite::params_from_iter;

impl Database {
    pub fn list_organization_plans(&self) -> Result<Vec<OrganizationPlanDto>, DbError> {
        let conn = self.conn()?;
        let mut plans = {
            let mut stmt = conn.prepare(
                "SELECT id, title, status, source_kind, source_query_fingerprint,
                        source_snapshot_revision, requested_count, materialized_count,
                        planner_version, revision, active_execution_id,
                        active_operation_batch_id, last_error_code, last_error_detail,
                        created_at, updated_at, ready_at, completed_at
                 FROM organization_plans ORDER BY updated_at DESC, id LIMIT 200",
            )?;
            let rows = stmt.query_map([], plan_from_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for plan in &mut plans {
            if one_off::is_plan(&conn, &plan.id)? {
                plan.source_kind = "one_off".into();
            }
            plan.summary = load_plan_summary(&conn, &plan.id)?;
        }
        Ok(plans)
    }

    pub fn get_organization_plan(&self, plan_id: &str) -> Result<OrganizationPlanDto, DbError> {
        let conn = self.conn()?;
        load_plan(
            &conn,
            &validate_id(plan_id, "organization_plan_id_invalid")?,
        )
    }

    pub fn query_organization_plan_items(
        &self,
        request: QueryOrganizationPlanItemsRequest,
    ) -> Result<OrganizationPlanItemPageDto, DbError> {
        if request.page_size == 0 || request.page_size > ORGANIZATION_PAGE_MAX {
            return Err(DbError::Validation(
                "organization_plan_page_size_invalid".to_string(),
            ));
        }
        let plan_id = validate_id(&request.plan_id, "organization_plan_id_invalid")?;
        let cursor = request
            .cursor
            .as_deref()
            .map(decode_item_cursor)
            .transpose()?;
        let conn = self.conn()?;
        let plan = load_plan(&conn, &plan_id)?;
        let mut sql = "SELECT id, plan_id, ordinal, file_id_snapshot, source_path_snapshot,
                source_name_snapshot, source_size_snapshot, source_mtime_snapshot,
                source_is_dir_snapshot, proposal_fingerprint, proposal_kind,
                proposed_target_directory, proposed_name, proposed_target_path,
                decision, edited_name, validity, confidence, risk_level,
                requires_confirmation, blocking_code, blocking_detail,
                authoritative_preview_id, operation_log_id, execution_id, revision,
                created_at, updated_at
             FROM organization_plan_items WHERE plan_id = ?1"
            .to_string();
        let mut values = vec![rusqlite::types::Value::Text(plan_id.clone())];
        if let Some(cursor) = cursor {
            sql.push_str(" AND (ordinal > ?2 OR (ordinal = ?2 AND id > ?3))");
            values.push(rusqlite::types::Value::Integer(cursor.ordinal));
            values.push(rusqlite::types::Value::Text(cursor.id));
        }
        sql.push_str(" ORDER BY ordinal, id LIMIT ?");
        values.push(rusqlite::types::Value::Integer(
            i64::from(request.page_size) + 1,
        ));
        let mut items = {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(values.iter()), item_from_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let scope = load_organization_scope_projection(&conn, &plan_id);
        let file_ids = items
            .iter()
            .map(|item| item.file_id_snapshot.clone())
            .collect::<Vec<_>>();
        let scope_memberships = organization_scope_memberships(&conn, &scope, &file_ids);
        for item in &mut items {
            let scope_membership = scope_memberships
                .get(&item.file_id_snapshot)
                .copied()
                .unwrap_or(Some(false));
            let current_file = load_indexed_file_by_id(&conn, &item.file_id_snapshot)?;
            decorate_organization_item_metadata_with_file(
                &conn,
                item,
                current_file.as_ref(),
                scope_membership,
            )?;
        }
        let has_more = items.len() > request.page_size as usize;
        if has_more {
            items.truncate(request.page_size as usize);
        }
        let next_cursor = items.last().and_then(|item| {
            has_more.then(|| {
                encode_item_cursor(&OrganizationItemCursor {
                    ordinal: item.ordinal,
                    id: item.id.clone(),
                })
            })
        });
        Ok(OrganizationPlanItemPageDto {
            plan_id,
            plan_revision: plan.revision,
            items,
            next_cursor,
            has_more,
        })
    }

    pub fn query_organization_plan_groups(
        &self,
        request: QueryOrganizationPlanGroupsRequest,
    ) -> Result<OrganizationPlanGroupPageDto, DbError> {
        validate_organization_page_size(request.page_size, "organization_group_page_size_invalid")?;
        let plan_id = validate_id(&request.plan_id, "organization_plan_id_invalid")?;
        let conn = self.conn()?;
        let plan = load_plan(&conn, &plan_id)?;
        let projection = load_organization_plan_group_projection(&conn, &plan_id, plan.revision)?;
        let projection_fingerprint = projection.projection_fingerprint.clone();
        let cursor = request
            .cursor
            .as_deref()
            .map(decode_group_cursor)
            .transpose()?;
        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.projection_fingerprint != projection_fingerprint)
        {
            return Err(DbError::Validation(
                "organization_group_projection_changed".to_string(),
            ));
        }
        let mut groups = projection.groups;
        let effective_summary = effective_summary_from_groups(&groups);
        if let Some(cursor) = cursor {
            groups.retain(|group| organization_group_is_after(group, &cursor));
        }
        let has_more = groups.len() > request.page_size as usize;
        if has_more {
            groups.truncate(request.page_size as usize);
        }
        let next_cursor = groups.last().and_then(|group| {
            has_more.then(|| {
                encode_group_cursor(&OrganizationGroupCursor {
                    version: ORGANIZATION_GROUP_CURSOR_VERSION,
                    projection_fingerprint: projection_fingerprint.clone(),
                    label: group.label.clone(),
                    group_id: group.group_id.clone(),
                })
            })
        });
        Ok(OrganizationPlanGroupPageDto {
            plan_id,
            plan_revision: plan.revision,
            groups,
            effective_summary,
            projection_fingerprint,
            next_cursor,
            has_more,
        })
    }

    pub fn query_organization_plan_group_items(
        &self,
        request: QueryOrganizationPlanGroupItemsRequest,
    ) -> Result<OrganizationPlanGroupItemPageDto, DbError> {
        validate_organization_page_size(
            request.page_size,
            "organization_group_item_page_size_invalid",
        )?;
        let plan_id = validate_id(&request.plan_id, "organization_plan_id_invalid")?;
        let group_id = validate_id(&request.group_id, "organization_group_id_invalid")?;
        validate_organization_projection_fingerprint(
            &request.expected_projection_fingerprint,
            "organization_group_projection_fingerprint_invalid",
        )?;
        let cursor = request
            .cursor
            .as_deref()
            .map(decode_group_item_cursor)
            .transpose()?;
        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.group_id != group_id)
        {
            return Err(DbError::Validation(
                "organization_group_cursor_invalid".to_string(),
            ));
        }
        if cursor.as_ref().is_some_and(|cursor| {
            cursor.projection_fingerprint != request.expected_projection_fingerprint
        }) {
            return Err(DbError::Validation(
                "organization_group_projection_changed".to_string(),
            ));
        }
        let conn = self.conn()?;
        let plan = load_plan(&conn, &plan_id)?;
        let projection = load_organization_plan_group_projection(&conn, &plan_id, plan.revision)?;
        let current_group = projection
            .groups
            .iter()
            .find(|group| group.group_id == group_id)
            .ok_or_else(|| DbError::Validation("organization_group_not_found".to_string()))?;
        if current_group.projection_fingerprint != request.expected_projection_fingerprint {
            return Err(DbError::Validation(
                "organization_group_projection_changed".to_string(),
            ));
        }
        let mut items = projection
            .items
            .into_iter()
            .filter(|item| {
                organization_group_id(&plan_id, &organization_group_key(&item.item)) == group_id
            })
            .map(|projection| projection.item)
            .collect::<Vec<_>>();
        if items.is_empty() {
            return Err(DbError::Validation(
                "organization_group_not_found".to_string(),
            ));
        }
        if let Some(cursor) = cursor {
            items.retain(|item| {
                item.ordinal > cursor.ordinal
                    || (item.ordinal == cursor.ordinal && item.id > cursor.id)
            });
        }
        let has_more = items.len() > request.page_size as usize;
        if has_more {
            items.truncate(request.page_size as usize);
        }
        let next_cursor = items.last().and_then(|item| {
            has_more.then(|| {
                encode_group_item_cursor(&OrganizationGroupItemCursor {
                    version: ORGANIZATION_GROUP_CURSOR_VERSION,
                    projection_fingerprint: request.expected_projection_fingerprint.clone(),
                    group_id: group_id.clone(),
                    ordinal: item.ordinal,
                    id: item.id.clone(),
                })
            })
        });
        Ok(OrganizationPlanGroupItemPageDto {
            plan_id,
            group_id,
            plan_revision: plan.revision,
            projection_fingerprint: request.expected_projection_fingerprint,
            items,
            next_cursor,
            has_more,
        })
    }
}
