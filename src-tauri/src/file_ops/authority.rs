use super::*;

const OPERATION_PREVIEW_STALE: &str = "operation_preview_stale";

pub(crate) fn resolve_execute_selections(
    db: &Database,
    request: ExecuteMovesByIdRequest,
) -> Result<ExecuteMovesRequest, String> {
    if request.operations.is_empty() {
        return Err("At least one authoritative preview ID is required.".to_string());
    }
    let file_ids = request
        .operations
        .iter()
        .map(|selection| selection.file_id.clone())
        .collect::<Vec<_>>();
    let previews = db
        .get_operation_previews_by_file_ids(&file_ids)
        .map_err(|error| error.to_string())?;
    let mut previews_by_identity = std::collections::HashMap::new();
    for preview in previews {
        previews_by_identity.insert((preview.file_id.clone(), preview.id.clone()), preview);
    }
    for file_id in &file_ids {
        if let Some(preview) = db
            .get_permanent_delete_operation_preview_if_available(file_id)
            .map_err(|error| error.to_string())?
        {
            previews_by_identity.insert((preview.file_id.clone(), preview.id.clone()), preview);
        }
    }
    let mut operations = Vec::with_capacity(request.operations.len());
    for selection in request.operations {
        let operation_fingerprint = selection.operation_fingerprint.trim();
        let expected_revision = selection.expected_revision.trim();
        if operation_fingerprint.is_empty()
            || expected_revision.is_empty()
            || operation_fingerprint != expected_revision
        {
            return Err(OPERATION_PREVIEW_STALE.to_string());
        }
        if selection.id.starts_with("op-manual-rename-") {
            let name = selection
                .new_name
                .as_deref()
                .ok_or_else(|| OPERATION_PREVIEW_STALE.to_string())?;
            let preview = db
                .get_manual_rename_operation_preview(&selection.file_id, name)
                .map_err(|_| OPERATION_PREVIEW_STALE.to_string())?;
            previews_by_identity.insert((preview.file_id.clone(), preview.id.clone()), preview);
        }
        let preview = previews_by_identity
            .get(&(selection.file_id.clone(), selection.id.clone()))
            .ok_or_else(|| OPERATION_PREVIEW_STALE.to_string())?;
        if preview.operation_fingerprint.as_deref() != Some(operation_fingerprint)
            || preview.is_executable != Some(true)
        {
            return Err(OPERATION_PREVIEW_STALE.to_string());
        }
        if preview.operation_type == "permanent_delete" && selection.new_name.is_some() {
            return Err(OPERATION_PREVIEW_STALE.to_string());
        }
        db.verify_indexed_file_identity(&selection.file_id)
            .map_err(|error| error.to_string())?;
        let (original_name, indexed_extension, is_dir) = db
            .get_indexed_file_naming(&selection.file_id)
            .map_err(|error| error.to_string())?;
        let mut new_name = normalize_proposed_file_name(
            &original_name,
            &indexed_extension,
            &preview.new_name,
            is_dir,
            ExtensionChangePolicy::Preserve,
        )?;
        validate_safe_file_name(&new_name)?;
        let mut target_path = preview.target_path.clone();
        if let Some(override_name) = selection.new_name {
            let normalized_override = normalize_proposed_file_name(
                &original_name,
                &indexed_extension,
                &override_name,
                is_dir,
                ExtensionChangePolicy::Preserve,
            )?;
            validate_safe_file_name(&normalized_override)?;
            let parent = Path::new(&target_path)
                .parent()
                .ok_or_else(|| "Authoritative preview target has no parent.".to_string())?;
            target_path = normalize_path(&parent.join(&normalized_override));
            new_name = normalized_override;
        }
        operations.push(OperationPreviewRequest {
            id: preview.id.clone(),
            file_id: preview.file_id.clone(),
            operation_type: preview.operation_type.clone(),
            source_path: preview.source_path.clone(),
            target_path,
            old_name: preview.old_name.clone(),
            new_name,
            is_executable: preview.is_executable,
        });
    }
    Ok(ExecuteMovesRequest { operations })
}
