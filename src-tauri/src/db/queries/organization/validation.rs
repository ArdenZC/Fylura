use super::*;
use rusqlite::params;

pub(super) fn organization_item_action_error(action: &str) -> &'static str {
    match action {
        "accept_suggestion" => "organization_item_accept_not_available",
        "edit_name" => "organization_item_edit_not_available",
        "keep" => "organization_item_keep_not_available",
        "clear_decision" => "organization_item_clear_not_available",
        _ => "organization_item_action_not_available",
    }
}

pub(super) fn normalize_decision(value: &str) -> Result<&'static str, DbError> {
    match value {
        "accept" | "accepted" => Ok("accepted"),
        "keep" | "kept" => Ok("kept"),
        "edit" | "edited" => Ok("edited"),
        "clear" | "undecided" => Ok("undecided"),
        _ => Err(DbError::Validation(
            "organization_decision_invalid".to_string(),
        )),
    }
}

pub(super) fn normalize_group_decision(value: &str) -> Result<&'static str, DbError> {
    match value {
        "accept" | "accepted" => Ok("accepted"),
        "keep" | "kept" => Ok("kept"),
        "clear" | "undecided" => Ok("undecided"),
        _ => Err(DbError::Validation(
            "organization_group_decision_invalid".to_string(),
        )),
    }
}

pub(super) fn validate_edited_filename(
    conn: &rusqlite::Connection,
    plan_id: &str,
    item_id: &str,
    value: Option<&str>,
) -> Result<String, DbError> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DbError::Validation("organization_edited_name_required".to_string()))?;
    let (file_id, preview_id): (String, Option<String>) = conn.query_row(
        "SELECT file_id_snapshot, authoritative_preview_id
         FROM organization_plan_items WHERE id = ?1 AND plan_id = ?2",
        params![item_id, plan_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let preview_id = preview_id.ok_or_else(|| {
        DbError::Validation("organization_item_has_no_editable_preview".to_string())
    })?;
    let row = load_indexed_file_by_id(conn, &file_id)?
        .ok_or_else(|| DbError::Validation("organization_source_missing".to_string()))?;
    let projection = current_organization_projection(conn, &row)?;
    let preview = projection
        .preview
        .filter(|preview| preview.id == preview_id)
        .ok_or_else(|| DbError::Validation("organization_preview_stale".to_string()))?;
    let normalized = crate::file_naming::normalize_proposed_file_name(
        &row.name,
        &row.extension,
        value,
        row.is_dir,
        crate::file_naming::ExtensionChangePolicy::Preserve,
    )
    .map_err(DbError::Validation)?;
    crate::file_ops::validate_safe_file_name(&normalized).map_err(DbError::Validation)?;
    let target = std::path::Path::new(&preview.target_path)
        .parent()
        .ok_or_else(|| DbError::Validation("organization_target_parent_invalid".to_string()))?
        .join(&normalized);
    if target.exists() {
        return Err(DbError::Validation(
            "organization_target_collision".to_string(),
        ));
    }
    Ok(normalized)
}

pub(super) fn validate_organization_page_size(page_size: u32, code: &str) -> Result<(), DbError> {
    if page_size == 0 || page_size > ORGANIZATION_PAGE_MAX {
        Err(DbError::Validation(code.to_string()))
    } else {
        Ok(())
    }
}

pub(super) fn validate_id(value: &str, code: &str) -> Result<String, DbError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 512 || value.chars().any(|ch| ch.is_control()) {
        Err(DbError::Validation(code.to_string()))
    } else {
        Ok(value.to_string())
    }
}
