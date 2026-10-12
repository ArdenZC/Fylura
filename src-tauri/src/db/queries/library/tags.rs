use super::*;

impl Database {
    pub fn list_user_tags(&self) -> Result<Vec<UserTagDto>, DbError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT t.id, t.display_name, t.color_token, (SELECT COUNT(*) FROM file_user_tags AS fut WHERE fut.tag_id = t.id), t.created_at, t.updated_at, t.revision FROM user_tags AS t ORDER BY t.normalized_name COLLATE NOCASE, t.id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(UserTagDto {
                id: row.get(0)?,
                display_name: row.get(1)?,
                color_token: row.get(2)?,
                usage_count: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
                revision: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
    }

    pub fn create_user_tag(&self, request: CreateUserTagRequest) -> Result<UserTagDto, DbError> {
        let display_name = validate_tag_name(&request.display_name)?;
        let color = validate_color_token(&request.color_token)?;
        let normalized_name = normalize_tag_name(&display_name);
        let now = current_unix_seconds();
        let id = format!("user-tag-{}", uuid::Uuid::new_v4());
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO user_tags(id, display_name, normalized_name, color_token, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, display_name, normalized_name, color, now],
        )
        .map_err(map_tag_write_error)?;
        bump_library_query_revision_in_transaction(&tx)?;
        tx.commit()?;
        Ok(UserTagDto {
            id,
            display_name,
            color_token: color,
            usage_count: 0,
            created_at: now,
            updated_at: now,
            revision: 1,
        })
    }

    pub fn update_user_tag(&self, request: UpdateUserTagRequest) -> Result<UserTagDto, DbError> {
        let id = validate_file_id(&request.id)?;
        let display_name = validate_tag_name(&request.display_name)?;
        let color = validate_color_token(&request.color_token)?;
        let normalized_name = normalize_tag_name(&display_name);
        let now = current_unix_seconds();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let updated = tx.execute(
            "UPDATE user_tags SET display_name = ?1, normalized_name = ?2, color_token = ?3, updated_at = ?4, revision = revision + 1 WHERE id = ?5 AND revision = ?6",
            params![display_name, normalized_name, color, now, id, request.expected_revision],
        ).map_err(map_tag_write_error)?;
        if updated != 1 {
            return Err(DbError::Validation(
                "library_tag_stale_or_missing".to_string(),
            ));
        }
        let (usage_count, created_at): (i64, i64) = tx.query_row(
            "SELECT (SELECT COUNT(*) FROM file_user_tags WHERE tag_id = t.id), t.created_at FROM user_tags AS t WHERE t.id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        bump_library_query_revision_in_transaction(&tx)?;
        tx.commit()?;
        Ok(UserTagDto {
            id,
            display_name,
            color_token: color,
            usage_count,
            created_at,
            updated_at: now,
            revision: request.expected_revision + 1,
        })
    }

    pub fn delete_user_tag(&self, request: DeleteUserTagRequest) -> Result<bool, DbError> {
        let id = validate_file_id(&request.id)?;
        if !request.confirm {
            return Err(DbError::Validation(
                "library_tag_delete_confirmation_required".to_string(),
            ));
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let (usage, revision): (i64, i64) = tx.query_row(
            "SELECT (SELECT COUNT(*) FROM file_user_tags WHERE tag_id = t.id), t.revision FROM user_tags AS t WHERE t.id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?.ok_or_else(|| DbError::Validation("library_tag_not_found".to_string()))?;
        if usage != request.expected_usage_count || revision != request.expected_revision {
            return Err(DbError::Validation("library_tag_stale_usage".to_string()));
        }
        tx.execute("DELETE FROM user_tags WHERE id = ?1", params![id])?;
        bump_library_query_revision_in_transaction(&tx)?;
        tx.commit()?;
        Ok(true)
    }

    pub fn mutate_file_user_tags(
        &self,
        request: MutateFileUserTagsRequest,
    ) -> Result<MutateFileUserTagsResultDto, DbError> {
        if request.tag_ids.is_empty() || request.tag_ids.len() > 32 {
            return Err(DbError::Validation(
                "library_tag_mutation_invalid_tags".to_string(),
            ));
        }
        let mut tag_ids = request.tag_ids;
        normalize_id_vec(&mut tag_ids, "tag")?;
        let mut conn = self.conn()?;
        // Reserve the writer before the selection reads so a competing WAL
        // commit cannot turn this read snapshot into SQLITE_BUSY_SNAPSHOT.
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for tag_id in &tag_ids {
            if !tag_exists(&tx, tag_id)? {
                return Err(DbError::Validation("library_tag_not_found".to_string()));
            }
        }
        let current_revision = current_library_revision(&tx)?;
        let (where_sql, where_params, missing_count, excluded_count, fingerprint) =
            selection_where(&tx, &request.selection, current_revision)?;
        if let Some(expected) = request.expected_count {
            let count: i64 = tx.query_row(
                &format!("SELECT COUNT(*) FROM files AS f WHERE {where_sql}"),
                params_from_iter(where_params.iter()),
                |row| row.get(0),
            )?;
            if count != expected {
                return Err(DbError::Validation(
                    "library_selection_expected_count_mismatch".to_string(),
                ));
            }
        }
        let target_count: i64 = tx.query_row(
            &format!("SELECT COUNT(*) FROM files AS f WHERE {where_sql}"),
            params_from_iter(where_params.iter()),
            |row| row.get(0),
        )?;
        if target_count > LIBRARY_SELECTION_MAX as i64 {
            return Err(DbError::Validation(
                "library_selection_too_large".to_string(),
            ));
        }
        let timestamp = current_unix_seconds();
        let mut applied = 0_i64;
        let total_pairs = target_count.saturating_mul(i64::try_from(tag_ids.len()).unwrap_or(0));
        for tag_id in &tag_ids {
            let sql = match request.operation {
                UserTagMutationOperation::Add => format!(
                    "INSERT OR IGNORE INTO file_user_tags(file_id, tag_id, created_at) SELECT f.id, ?1, ?2 FROM files AS f WHERE {where_sql} AND NOT EXISTS (SELECT 1 FROM file_user_tags AS existing WHERE existing.file_id = f.id AND existing.tag_id = ?1)"
                ),
                UserTagMutationOperation::Remove => format!(
                    "DELETE FROM file_user_tags WHERE tag_id = ?1 AND file_id IN (SELECT f.id FROM files AS f WHERE {where_sql})"
                ),
            };
            let mut params = vec![SqlValue::Text(tag_id.clone())];
            if matches!(request.operation, UserTagMutationOperation::Add) {
                params.push(SqlValue::Integer(timestamp));
            }
            params.extend(where_params.clone());
            applied += tx.execute(&sql, params_from_iter(params.iter()))? as i64;
        }
        let revision = if applied > 0 {
            bump_library_query_revision_in_transaction(&tx)?
        } else {
            current_revision
        };
        clear_temp_selection_ids(&tx)?;
        tx.commit()?;
        let _ = fingerprint;
        Ok(MutateFileUserTagsResultDto {
            applied_count: applied,
            already_present_count: total_pairs.saturating_sub(applied),
            missing_count,
            excluded_count,
            revision,
        })
    }
}
