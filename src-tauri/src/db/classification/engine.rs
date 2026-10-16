use super::super::*;
use super::{
    builtin_rules::{classify_builtin, days_since_unix, legacy_builtin_classification_rules},
    naming::{build_suggested_name, build_target_path, OrganizeRootConfig},
};
use crate::settings::AppSettings;
use rusqlite::{params, params_from_iter, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

impl Database {
    pub fn execute_rules_for_scope_v2(
        &self,
        request: ExecuteRulesForScopeV2Request,
    ) -> Result<RuleExecutionResultV2, DbError> {
        if !request.confirmed {
            return Err(DbError::Validation(
                "rule_execution_confirmation_required".to_string(),
            ));
        }
        // Freeze the complete backend authority for this execution. Catalog
        // mutations take the same gate, so a concurrent learned/user/settings
        // change cannot land between validation and a classification batch.
        let _catalog_guard = super::super::queries::rules_repo::catalog_execution_guard();
        let catalog = self.get_rule_catalog_state()?;
        if catalog.revision != request.expected_catalog_revision {
            return Err(DbError::Validation(
                "rule_catalog_revision_conflict".to_string(),
            ));
        }
        let settings = crate::settings::get_app_settings(self)?;
        let active_rules = self.authoritative_rules_for_settings(&settings)?;
        let classification_version = classification_version_for_rules(&active_rules, &settings)?;
        let snapshot = self.capture_rule_execution_snapshot(
            &request.scope,
            catalog.revision,
            &settings,
            &active_rules,
            &classification_version,
        )?;
        let scope = self.resolve_authoritative_rule_scope(&request.scope)?;
        let summary = self.execute_active_rules_for_scope_with_settings(
            &scope,
            &active_rules,
            request.mode,
            &settings,
        )?;
        // Revalidate catalog/settings/rule/scope authority after the write
        // batches. The library revision is intentionally allowed to advance
        // here because these batches publish classification metadata; root and
        // catalog revisions must remain identical to the frozen snapshot.
        let after_settings = crate::settings::get_app_settings(self)?;
        let after_rules = self.authoritative_rules_for_settings(&after_settings)?;
        let after_version = classification_version_for_rules(&after_rules, &after_settings)?;
        let after_catalog = self.get_rule_catalog_state()?;
        let after = self.capture_rule_execution_snapshot(
            &request.scope,
            after_catalog.revision,
            &after_settings,
            &after_rules,
            &after_version,
        )?;
        // Each non-empty classification batch advances the File Library
        // revision once.  Account for those owned writes explicitly so a
        // concurrent scan/library mutation cannot be hidden by the normal
        // classification publication revision bump.
        let owned_library_revision_delta = if summary.updated == 0 {
            0
        } else {
            (summary.updated + CLASSIFY_BATCH_SIZE as i64 - 1) / CLASSIFY_BATCH_SIZE as i64
        };
        if after_catalog.revision != catalog.revision
            || after.library_revision
                != snapshot
                    .library_revision
                    .saturating_add(owned_library_revision_delta)
            || snapshot.authority_fingerprint != after.authority_fingerprint
        {
            return Err(DbError::Validation(
                "rule_execution_snapshot_stale".to_string(),
            ));
        }
        Ok(RuleExecutionResultV2 {
            summary,
            catalog_revision: catalog.revision,
            classification_version,
        })
    }

    pub(crate) fn execute_authoritative_rules_for_scope(
        &self,
        scope: &LibraryScope,
        mode: RuleExecutionMode,
    ) -> Result<RuleExecutionSummary, DbError> {
        let settings = crate::settings::get_app_settings(self)?;
        let active_rules = self.authoritative_rules_for_settings(&settings)?;
        self.execute_active_rules_for_scope_with_settings(scope, &active_rules, mode, &settings)
    }

    pub(crate) fn execute_authoritative_rules_for_paths(
        &self,
        paths: &[String],
    ) -> Result<RuleExecutionSummary, DbError> {
        let settings = crate::settings::get_app_settings(self)?;
        let active_rules = self.authoritative_rules_for_settings(&settings)?;
        self.execute_active_rules_for_paths_with_settings(paths, &active_rules, &settings)
    }

    fn authoritative_rules_for_settings(
        &self,
        settings: &AppSettings,
    ) -> Result<Vec<Rule>, DbError> {
        let persisted = self.load_enabled_persisted_rules()?;
        let rules = Self::active_rules_for_preview(&persisted, settings);
        for rule in &rules {
            super::super::queries::rules_repo::validate_user_rule(rule)?;
        }
        Ok(rules)
    }

    pub(crate) fn active_rules_for_preview(
        persisted: &[Rule],
        settings: &AppSettings,
    ) -> Vec<Rule> {
        let selected = persisted
            .iter()
            .filter(|rule| match rule.source {
                RuleSource::User => true,
                RuleSource::Learned => settings.use_learned_rules_as_auto_rules,
                RuleSource::System => settings.use_legacy_builtin_classification_rules,
                _ => false,
            })
            .cloned();
        if settings.use_legacy_builtin_classification_rules {
            legacy_builtin_classification_rules()
                .into_iter()
                .chain(selected)
                .collect::<Vec<_>>()
        } else {
            selected.collect::<Vec<_>>()
        }
    }

    fn capture_rule_execution_snapshot(
        &self,
        scope: &FileLibraryScopeV2,
        catalog_revision: i64,
        settings: &AppSettings,
        rules: &[Rule],
        classification_version: &str,
    ) -> Result<RuleExecutionSnapshot, DbError> {
        let conn = self.conn()?;
        let resolved = super::super::queries::library::resolve_scope(&conn, scope)?;
        if !matches!(resolved.health.state.as_str(), "healthy" | "empty") {
            return Err(DbError::Validation(
                "rule_execution_scope_unavailable".to_string(),
            ));
        }
        let mut root_state = Vec::new();
        for root in &resolved.health.roots {
            let state = conn.query_row(
                "SELECT id, revision, current_generation, needs_reconciliation,
                        watcher_rule_recovery_required, watcher_revision,
                        watcher_applied_revision
                 FROM scan_roots WHERE id = ?1",
                params![root.id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )?;
            root_state.push(state);
        }
        let library_revision = super::super::queries::library::current_library_revision(&conn)?;
        let rules_fingerprint = rule_version_for_rules(rules)?;
        let settings_fingerprint = Sha256::digest(serde_json::to_vec(settings)?);
        let scope_fingerprint =
            Sha256::digest(serde_json::to_vec(&(scope, &resolved.health, &root_state))?);
        let authority_fingerprint = format!(
            "{catalog_revision}:{}:{}:{}:{}",
            hex_digest(&settings_fingerprint),
            rules_fingerprint,
            classification_version,
            hex_digest(&scope_fingerprint)
        );
        Ok(RuleExecutionSnapshot {
            authority_fingerprint,
            library_revision,
        })
    }

    fn resolve_authoritative_rule_scope(
        &self,
        scope: &FileLibraryScopeV2,
    ) -> Result<LibraryScope, DbError> {
        let conn = self.conn()?;
        let resolved = super::super::queries::library::resolve_scope(&conn, scope)?;
        if !matches!(resolved.health.state.as_str(), "healthy" | "empty") {
            return Err(DbError::Validation(
                "rule_execution_scope_unavailable".to_string(),
            ));
        }
        let mut roots = Vec::new();
        for root in resolved.health.roots.iter().filter(|root| root.available) {
            let path = conn.query_row(
                "SELECT normalized_path FROM scan_roots WHERE id = ?1",
                params![root.id],
                |row| row.get::<_, String>(0),
            )?;
            roots.push(path);
        }
        Ok(LibraryScope::Roots { roots })
    }

    pub fn execute_rules_on_inbox(
        &self,
        rules: Vec<Rule>,
    ) -> Result<RuleExecutionSummary, DbError> {
        let settings = crate::settings::get_app_settings(self)?;
        self.execute_rules_on_inbox_with_settings(rules, &settings)
    }

    pub fn execute_rules_for_scope(
        &self,
        scope: &LibraryScope,
        rules: Vec<Rule>,
    ) -> Result<RuleExecutionSummary, DbError> {
        self.execute_rules_for_scope_with_mode(scope, rules, RuleExecutionMode::InboxOnly)
    }

    pub fn execute_rules_for_scope_with_mode(
        &self,
        scope: &LibraryScope,
        rules: Vec<Rule>,
        mode: RuleExecutionMode,
    ) -> Result<RuleExecutionSummary, DbError> {
        let settings = crate::settings::get_app_settings(self)?;
        self.execute_rules_for_scope_with_settings(scope, rules, mode, &settings)
    }

    fn execute_rules_for_scope_with_settings(
        &self,
        scope: &LibraryScope,
        rules: Vec<Rule>,
        mode: RuleExecutionMode,
        settings: &AppSettings,
    ) -> Result<RuleExecutionSummary, DbError> {
        let all_rules = active_rules_for_manual_rules(rules, settings);
        self.execute_active_rules_for_scope_with_settings(scope, &all_rules, mode, settings)
    }

    fn execute_active_rules_for_scope_with_settings(
        &self,
        scope: &LibraryScope,
        all_rules: &[Rule],
        mode: RuleExecutionMode,
        settings: &AppSettings,
    ) -> Result<RuleExecutionSummary, DbError> {
        let rule_version = classification_version_for_rules(all_rules, settings)?;
        let scoped = scoped_files_sql(Some(scope));
        let selection_filter = match mode {
            RuleExecutionMode::InboxOnly => {
                "f.id NOT GLOB 'zen-canvas-source:*:epoch:*' AND f.lifecycle = 'Inbox'"
            }
            RuleExecutionMode::AllChangedOrRuleChanged => {
                "f.id NOT GLOB 'zen-canvas-source:*:epoch:*'"
            }
        };
        let lifecycle_filter = format!("WHERE {selection_filter}");
        let sql = format!(
            r#"
                WITH {},
                dup_groups AS (
                    SELECT file_id, size, content_hash
                    FROM active_duplicate_membership
                )
                SELECT f.id, f.path, f.name, f.extension, f.size, f.mtime, f.ctime, f.is_dir, f.state_code,
                       f.file_type, f.purpose, f.lifecycle, f.context, f.risk_level, f.suggested_action,
                       f.suggested_target_path, f.suggested_name, f.confidence, f.classification_reason,
                       f.classification_status, f.matched_rules, f.requires_confirmation, f.content_hash,
                       (dg.file_id IS NOT NULL) AS is_duplicate,
                       f.is_stale, f.last_seen_at, f.last_classified_at, f.classified_rule_version,
                       f.last_classified_mtime, f.last_classified_size
                FROM scoped_files AS f
                LEFT JOIN dup_groups AS dg
                  ON dg.file_id = f.id
                {}
                ORDER BY f.mtime DESC, f.name COLLATE NOCASE ASC
                "#,
            scoped.cte, lifecycle_filter
        );
        let read_conn = self.conn()?;
        let mut write_conn = self.conn()?;
        let mut stmt = read_conn.prepare(&sql)?;
        let mut rows = stmt.query(params_from_iter(scoped.params.iter()))?;

        let mut scanned = 0_i64;
        let mut updated = 0_i64;
        let mut skipped = 0_i64;
        let mut needs_confirmation = 0_i64;
        let mut batch = Vec::with_capacity(CLASSIFY_BATCH_SIZE);

        while let Some(row) = rows.next()? {
            let row = indexed_file_from_row(row)?;
            scanned += 1;
            if !should_classify_file(&row, &rule_version) {
                skipped += 1;
                continue;
            }

            batch.push(row);

            if batch.len() == CLASSIFY_BATCH_SIZE {
                let batch_summary = execute_classification_batch(
                    &mut write_conn,
                    &batch,
                    all_rules,
                    &rule_version,
                    settings,
                )?;
                updated += batch_summary.updated;
                needs_confirmation += batch_summary.needs_confirmation;
                batch.clear();
            }
        }

        if !batch.is_empty() {
            let batch_summary = execute_classification_batch(
                &mut write_conn,
                &batch,
                all_rules,
                &rule_version,
                settings,
            )?;
            updated += batch_summary.updated;
            needs_confirmation += batch_summary.needs_confirmation;
        }

        Ok(RuleExecutionSummary {
            scanned,
            updated,
            skipped,
            needs_confirmation,
            failed_batches: None,
            failed_files: None,
            warning: None,
        })
    }

    fn execute_rules_on_inbox_with_settings(
        &self,
        rules: Vec<Rule>,
        settings: &AppSettings,
    ) -> Result<RuleExecutionSummary, DbError> {
        let all_rules = active_rules_for_manual_rules(rules, settings);
        let rule_version = classification_version_for_rules(&all_rules, settings)?;
        let read_conn = self.conn()?;
        let mut write_conn = self.conn()?;
        let mut stmt = read_conn.prepare(
            r#"
                WITH dup_groups AS (
                    SELECT file_id, size, content_hash
                    FROM active_duplicate_membership
                )
                SELECT f.id, f.path, f.name, f.extension, f.size, f.mtime, f.ctime, f.is_dir, f.state_code,
                       f.file_type, f.purpose, f.lifecycle, f.context, f.risk_level, f.suggested_action,
                       f.suggested_target_path, f.suggested_name, f.confidence, f.classification_reason,
                       f.classification_status, f.matched_rules, f.requires_confirmation, f.content_hash,
                       (dg.file_id IS NOT NULL) AS is_duplicate,
                       f.is_stale, f.last_seen_at, f.last_classified_at, f.classified_rule_version,
                       f.last_classified_mtime, f.last_classified_size
                FROM files AS f
                LEFT JOIN dup_groups AS dg
                  ON dg.file_id = f.id
                WHERE f.id NOT GLOB 'zen-canvas-source:*:epoch:*'
                  AND f.lifecycle = 'Inbox'
                  AND f.is_stale = 0
                ORDER BY f.mtime DESC, f.name COLLATE NOCASE ASC
                "#,
        )?;
        let mut rows = stmt.query([])?;

        let mut scanned = 0_i64;
        let mut updated = 0_i64;
        let mut skipped = 0_i64;
        let mut needs_confirmation = 0_i64;
        let mut batch = Vec::with_capacity(CLASSIFY_BATCH_SIZE);

        while let Some(row) = rows.next()? {
            let row = indexed_file_from_row(row)?;
            scanned += 1;
            if !should_classify_file(&row, &rule_version) {
                skipped += 1;
                continue;
            }

            batch.push(row);

            if batch.len() == CLASSIFY_BATCH_SIZE {
                let batch_summary = execute_classification_batch(
                    &mut write_conn,
                    &batch,
                    &all_rules,
                    &rule_version,
                    settings,
                )?;
                updated += batch_summary.updated;
                needs_confirmation += batch_summary.needs_confirmation;
                batch.clear();
            }
        }

        if !batch.is_empty() {
            let batch_summary = execute_classification_batch(
                &mut write_conn,
                &batch,
                &all_rules,
                &rule_version,
                settings,
            )?;
            updated += batch_summary.updated;
            needs_confirmation += batch_summary.needs_confirmation;
        }

        Ok(RuleExecutionSummary {
            scanned,
            updated,
            skipped,
            needs_confirmation,
            failed_batches: None,
            failed_files: None,
            warning: None,
        })
    }

    pub fn execute_rules_for_paths(
        &self,
        paths: &[String],
        rules: Vec<Rule>,
    ) -> Result<RuleExecutionSummary, DbError> {
        let settings = crate::settings::get_app_settings(self)?;
        self.execute_rules_for_paths_with_settings(paths, rules, &settings)
    }

    fn execute_rules_for_paths_with_settings(
        &self,
        paths: &[String],
        rules: Vec<Rule>,
        settings: &AppSettings,
    ) -> Result<RuleExecutionSummary, DbError> {
        let all_rules = active_rules_for_manual_rules(rules, settings);
        self.execute_active_rules_for_paths_with_settings(paths, &all_rules, settings)
    }

    fn execute_active_rules_for_paths_with_settings(
        &self,
        paths: &[String],
        all_rules: &[Rule],
        settings: &AppSettings,
    ) -> Result<RuleExecutionSummary, DbError> {
        let target_paths = classification_path_candidates(paths, 500);
        if target_paths.is_empty() {
            return Ok(RuleExecutionSummary {
                scanned: 0,
                updated: 0,
                skipped: 0,
                needs_confirmation: 0,
                failed_batches: None,
                failed_files: None,
                warning: None,
            });
        }
        let rule_version = classification_version_for_rules(all_rules, settings)?;
        let placeholders = std::iter::repeat_n("?", target_paths.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            r#"
            WITH dup_groups AS (
                SELECT file_id, size, content_hash
                FROM active_duplicate_membership
            )
            SELECT f.id, f.path, f.name, f.extension, f.size, f.mtime, f.ctime, f.is_dir, f.state_code,
                   f.file_type, f.purpose, f.lifecycle, f.context, f.risk_level, f.suggested_action,
                   f.suggested_target_path, f.suggested_name, f.confidence, f.classification_reason,
                   f.classification_status, f.matched_rules, f.requires_confirmation, f.content_hash,
                   (dg.file_id IS NOT NULL) AS is_duplicate,
                   f.is_stale, f.last_seen_at, f.last_classified_at, f.classified_rule_version,
                   f.last_classified_mtime, f.last_classified_size
            FROM files AS f
            LEFT JOIN dup_groups AS dg
              ON dg.file_id = f.id
            WHERE f.id NOT GLOB 'zen-canvas-source:*:epoch:*'
              AND f.lifecycle = 'Inbox'
              AND f.is_stale = 0
              AND f.path IN ({placeholders})
            ORDER BY f.mtime DESC, f.name COLLATE NOCASE ASC
            "#
        );
        let read_conn = self.conn()?;
        let mut write_conn = self.conn()?;
        let mut stmt = read_conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(target_paths.iter()), indexed_file_from_row)?;

        let mut scanned = 0_i64;
        let mut updated = 0_i64;
        let mut skipped = 0_i64;
        let mut needs_confirmation = 0_i64;
        let mut batch = Vec::with_capacity(CLASSIFY_BATCH_SIZE);

        for row in rows {
            let row = row?;
            scanned += 1;
            if !should_classify_file(&row, &rule_version) {
                skipped += 1;
                continue;
            }

            batch.push(row);

            if batch.len() == CLASSIFY_BATCH_SIZE {
                let batch_summary = execute_classification_batch(
                    &mut write_conn,
                    &batch,
                    all_rules,
                    &rule_version,
                    settings,
                )?;
                updated += batch_summary.updated;
                needs_confirmation += batch_summary.needs_confirmation;
                batch.clear();
            }
        }

        if !batch.is_empty() {
            let batch_summary = execute_classification_batch(
                &mut write_conn,
                &batch,
                all_rules,
                &rule_version,
                settings,
            )?;
            updated += batch_summary.updated;
            needs_confirmation += batch_summary.needs_confirmation;
        }

        Ok(RuleExecutionSummary {
            scanned,
            updated,
            skipped,
            needs_confirmation,
            failed_batches: None,
            failed_files: None,
            warning: None,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteRulesForScopeV2Request {
    pub scope: FileLibraryScopeV2,
    pub mode: RuleExecutionMode,
    pub expected_catalog_revision: i64,
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleExecutionResultV2 {
    pub summary: RuleExecutionSummary,
    pub catalog_revision: i64,
    pub classification_version: String,
}

fn active_rules_for_manual_rules(rules: Vec<Rule>, settings: &AppSettings) -> Vec<Rule> {
    let user_rules = rules
        .into_iter()
        .filter(|rule| rule.enabled)
        .filter(|rule| {
            rule.source == RuleSource::User
                || (settings.use_learned_rules_as_auto_rules && rule.source == RuleSource::Learned)
                || (settings.use_legacy_builtin_classification_rules
                    && rule.source == RuleSource::System)
        });
    if settings.use_legacy_builtin_classification_rules {
        legacy_builtin_classification_rules()
            .into_iter()
            .chain(user_rules)
            .collect()
    } else {
        user_rules.collect()
    }
}

pub(crate) fn rule_version_for_rules(rules: &[Rule]) -> Result<String, DbError> {
    let mut stable_rules = rules.to_vec();
    stable_rules.sort_by(|left, right| {
        left.priority
            .partial_cmp(&right.priority)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.id.cmp(&right.id))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.source.cmp(&right.source))
    });
    let payload = serde_json::to_string(&stable_rules)?;
    let digest = Sha256::digest(payload.as_bytes());
    Ok(digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>())
}

fn classification_version_for_rules(
    rules: &[Rule],
    settings: &AppSettings,
) -> Result<String, DbError> {
    let rules_version = rule_version_for_rules(rules)?;
    let payload = format!(
        "{rules_version}:folder_naming_language={}:organize_root_mode={:?}:organize_root_path={}",
        settings.folder_naming_language,
        settings.organize_root_mode,
        settings.organize_root_path.as_deref().unwrap_or_default()
    );
    let digest = Sha256::digest(payload.as_bytes());
    Ok(digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Clone)]
struct RuleExecutionSnapshot {
    authority_fingerprint: String,
    #[allow(dead_code)]
    library_revision: i64,
}

fn should_classify_file(row: &IndexedFileRow, rule_version: &str) -> bool {
    if row_has_protected_classification(row) {
        return false;
    }
    row.last_classified_at == 0
        || row.classified_rule_version != rule_version
        || row.last_classified_mtime != row.mtime
        || row.last_classified_size != row.size
}

fn row_has_protected_classification(row: &IndexedFileRow) -> bool {
    serde_json::from_str::<Vec<String>>(&row.matched_rules)
        .map(|rules| {
            rules.iter().any(|rule| {
                rule.starts_with("ai:")
                    || rule == "user_correction"
                    || rule == "user_confirmed"
                    || rule == "manual"
            })
        })
        .unwrap_or(false)
}

fn classification_path_candidates(paths: &[String], limit: usize) -> Vec<String> {
    let mut candidates = Vec::new();
    for path in paths
        .iter()
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
    {
        let path = trim_trailing_path_separators(path);
        if path.is_empty() {
            continue;
        }
        let normalized_path = normalize_path_text(path);
        for candidate in path_lookup_candidates(path, &normalized_path) {
            push_unique(&mut candidates, candidate);
            if candidates.len() >= limit {
                return candidates;
            }
        }
    }
    candidates
}

fn execute_classification_batch(
    conn: &mut Connection,
    batch: &[IndexedFileRow],
    all_rules: &[Rule],
    rule_version: &str,
    settings: &AppSettings,
) -> Result<RuleExecutionSummary, DbError> {
    let mut updated = 0_i64;
    let mut needs_confirmation = 0_i64;
    let classified_at = current_unix_seconds();
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            r#"
                UPDATE files
                SET file_type = ?2,
                    purpose = ?3,
                    lifecycle = ?4,
                    context = ?5,
                    risk_level = ?6,
                    suggested_action = ?7,
                    suggested_target_path = ?8,
                    suggested_name = ?9,
                    confidence = ?10,
                    classification_reason = ?11,
                    classification_status = ?12,
                    matched_rules = ?13,
                    requires_confirmation = ?14,
                    last_classified_at = ?15,
                    classified_rule_version = ?16,
                    last_classified_mtime = ?17,
                    last_classified_size = ?18
                WHERE id = ?1
                "#,
        )?;

        for row in batch {
            let classification = classify_indexed_file(row, all_rules, settings)?;
            if classification.requires_confirmation {
                needs_confirmation += 1;
            }
            stmt.execute(params![
                row.id,
                classification.file_type,
                classification.purpose,
                classification.lifecycle,
                classification.context,
                classification.risk_level,
                classification.suggested_action,
                classification.suggested_target_path,
                classification.suggested_name,
                classification.confidence,
                classification.classification_reason,
                classification.classification_status,
                classification.matched_rules,
                bool_to_i64(classification.requires_confirmation),
                classified_at,
                rule_version,
                row.mtime,
                row.size
            ])?;
            updated += 1;
        }
    }
    if updated > 0 {
        crate::db::bump_library_query_revision_in_transaction(&tx)?;
    }
    tx.commit()?;

    Ok(RuleExecutionSummary {
        scanned: batch.len() as i64,
        updated,
        skipped: 0,
        needs_confirmation,
        failed_batches: None,
        failed_files: None,
        warning: None,
    })
}

pub(crate) fn classify_indexed_file(
    row: &IndexedFileRow,
    all_rules: &[Rule],
    settings: &AppSettings,
) -> Result<ClassificationUpdate, DbError> {
    let file_type = normalized_file_type(row);
    let safety = classify_builtin(row, &file_type);
    let builtin = if settings.use_legacy_builtin_classification_rules {
        safety.clone()
    } else {
        fallback_classification(row)
    };
    // Keep references to the frozen rule snapshot while ranking. Cloning the
    // full rule AST for every match makes bounded impact previews scale with
    // the persisted rule count; only the winning rule's action is cloned below.
    let mut candidates = all_rules
        .iter()
        .filter_map(|rule| {
            evaluate_rule(rule, row, &file_type)
                .then_some((rule.weight + rule.priority * 0.1, rule))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                right
                    .1
                    .priority
                    .partial_cmp(&left.1.priority)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let top = candidates.first();
    let runner_up = candidates.get(1);
    let has_conflict = top
        .zip(runner_up)
        .map(|(top, runner_up)| top.0 - runner_up.0 <= 10.0)
        .unwrap_or(false);
    let action = top
        .map(|candidate| merge_action(&builtin.action, &candidate.1.action))
        .unwrap_or_else(|| builtin.action.clone());
    let mut matched_rule_names = candidates
        .iter()
        .map(|candidate| matched_rule_marker(candidate.1))
        .collect::<Vec<_>>();
    let confidence = top
        .map(|candidate| (candidate.0 / 100.0).clamp(0.35, 0.98))
        .unwrap_or(builtin.confidence);
    let mut risk_level = action
        .risk_level
        .clone()
        .unwrap_or_else(|| default_if_empty(&row.risk_level, "Unknown").into());
    let safety_guard_applied = safety
        .action
        .risk_level
        .as_deref()
        .is_some_and(|value| value == "Sensitive" || value == "System");
    if safety_guard_applied {
        risk_level = safety
            .action
            .risk_level
            .clone()
            .unwrap_or_else(|| "Sensitive".into());
        if !matched_rule_names
            .iter()
            .any(|rule| rule == "safety_guard:sensitive_or_system")
        {
            matched_rule_names.push("safety_guard:sensitive_or_system".to_string());
        }
    }
    let suggested_action = safe_action(
        action
            .suggested_action
            .clone()
            .unwrap_or_else(|| default_if_empty(&row.suggested_action, "Keep").into())
            .as_str(),
        risk_level.as_str(),
    );
    let suggested_target_path = build_target_path(
        row,
        &file_type,
        action.target_template.as_deref(),
        &settings.folder_naming_language,
        &OrganizeRootConfig::from(settings),
    );
    let suggested_name = build_suggested_name(row, action.rename_template.as_deref());
    let requires_confirmation = risk_level.as_str() == "Sensitive"
        || has_conflict
        || confidence < 0.65
        || suggested_action == "Review"
        || suggested_action == "DeleteCandidate";

    Ok(ClassificationUpdate {
        file_type,
        purpose: action
            .purpose
            .clone()
            .unwrap_or_else(|| default_if_empty(&row.purpose, "Unknown").into())
            .to_string(),
        lifecycle: action
            .lifecycle
            .clone()
            .unwrap_or_else(|| default_if_empty(&row.lifecycle, "Inbox").into())
            .to_string(),
        context: action
            .context
            .clone()
            .unwrap_or_else(|| row.context.clone()),
        risk_level: risk_level.to_string(),
        suggested_action,
        suggested_target_path,
        suggested_name,
        confidence,
        classification_reason: build_classification_reason(
            &matched_rule_names,
            has_conflict,
            risk_level.as_str(),
        ),
        classification_status: CLASSIFICATION_STATUS_CLASSIFIED.to_string(),
        matched_rules: serde_json::to_string(&matched_rule_names)?,
        requires_confirmation,
    })
}

fn matched_rule_marker(rule: &Rule) -> String {
    match rule.source.as_str() {
        "system" => format!("legacy_builtin:{}", rule.id),
        "learned" => format!("learned:{}", rule.id),
        _ => format!("rule:{}", rule.id),
    }
}

fn evaluate_rule(rule: &Rule, row: &IndexedFileRow, file_type: &str) -> bool {
    if !rule.enabled || rule.groups.is_empty() {
        return false;
    }
    let results = rule
        .groups
        .iter()
        .map(|group| evaluate_group(group, row, file_type))
        .collect::<Vec<_>>();
    match &rule.root_operator {
        RuleOperator::And => results.iter().all(|value| *value),
        RuleOperator::Or => results.iter().any(|value| *value),
        RuleOperator::Unknown | RuleOperator::Invalid(_) => false,
    }
}

fn evaluate_group(group: &RuleConditionGroup, row: &IndexedFileRow, file_type: &str) -> bool {
    if group.conditions.is_empty() {
        return false;
    }
    let results = group
        .conditions
        .iter()
        .map(|condition| evaluate_condition(condition, row, file_type))
        .collect::<Vec<_>>();
    match &group.operator {
        RuleOperator::Or => results.iter().any(|value| *value),
        RuleOperator::And => results.iter().all(|value| *value),
        RuleOperator::Unknown | RuleOperator::Invalid(_) => false,
    }
}

fn evaluate_condition(condition: &RuleCondition, row: &IndexedFileRow, file_type: &str) -> bool {
    let raw = condition_value(&condition.field, row, file_type).to_lowercase();
    let expected = json_value_to_string(&condition.value).trim().to_lowercase();
    if expected.is_empty() {
        return false;
    }
    match condition.operator.as_str() {
        "contains" => raw.contains(&expected),
        "equals" | "is" => raw == expected,
        "startsWith" => raw.starts_with(&expected),
        "endsWith" => raw.ends_with(&expected),
        "greaterThan" => raw.parse::<f64>().unwrap_or(0.0) > json_value_to_f64(&condition.value),
        "lessThan" => raw.parse::<f64>().unwrap_or(0.0) < json_value_to_f64(&condition.value),
        "olderThanDays" => days_since_unix(row.mtime) > json_value_to_i64(&condition.value),
        "newerThanDays" => days_since_unix(row.mtime) < json_value_to_i64(&condition.value),
        _ => false,
    }
}

fn condition_value(field: &str, row: &IndexedFileRow, file_type: &str) -> String {
    match field {
        "name" => row.name.clone(),
        "extension" => {
            if row.is_dir {
                "folder".to_string()
            } else {
                row.extension.clone()
            }
        }
        "file_type" => file_type.to_string(),
        "path" => row.path.clone(),
        "directory" => parent_directory(&row.path),
        "size" => row.size.to_string(),
        "modified_at" => unix_seconds_to_iso(row.mtime),
        "is_duplicate" => row.is_duplicate.to_string(),
        "risk_level" => row.risk_level.clone(),
        _ => String::new(),
    }
}

fn merge_action(base: &RuleAction, override_action: &RuleAction) -> RuleAction {
    RuleAction {
        purpose: override_action
            .purpose
            .clone()
            .or_else(|| base.purpose.clone()),
        lifecycle: override_action
            .lifecycle
            .clone()
            .or_else(|| base.lifecycle.clone()),
        context: override_action
            .context
            .clone()
            .or_else(|| base.context.clone()),
        risk_level: override_action
            .risk_level
            .clone()
            .or_else(|| base.risk_level.clone()),
        suggested_action: override_action
            .suggested_action
            .clone()
            .or_else(|| base.suggested_action.clone()),
        target_template: override_action
            .target_template
            .clone()
            .or_else(|| base.target_template.clone()),
        rename_template: override_action
            .rename_template
            .clone()
            .or_else(|| base.rename_template.clone()),
    }
}

fn build_classification_reason(
    matched_rules: &[String],
    has_conflict: bool,
    risk_level: &str,
) -> String {
    let mut parts = if matched_rules.is_empty() {
        vec!["No strong rule matched".to_string()]
    } else {
        vec![format!(
            "Matched {}",
            matched_rules
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )]
    };
    if has_conflict {
        parts.push("similar rule scores require review".to_string());
    }
    if risk_level == "Sensitive" {
        parts.push("sensitive files require manual confirmation".to_string());
    }
    parts.join("; ")
}

fn safe_action(action: &str, risk_level: &str) -> String {
    if (risk_level == "Sensitive" && action != "Keep") || action == "DeleteCandidate" {
        "Review".to_string()
    } else {
        action.to_string()
    }
}

fn fallback_classification(row: &IndexedFileRow) -> BuiltinClassification {
    BuiltinClassification {
        action: RuleAction {
            purpose: Some(default_if_empty(&row.purpose, "Unknown").into()),
            lifecycle: Some(default_if_empty(&row.lifecycle, "Inbox").into()),
            risk_level: Some(default_if_empty(&row.risk_level, "Unknown").into()),
            suggested_action: Some(default_if_empty(&row.suggested_action, "Keep").into()),
            target_template: (!row.suggested_target_path.trim().is_empty())
                .then(|| row.suggested_target_path.clone()),
            context: Some(row.context.clone()),
            ..RuleAction::default()
        },
        confidence: row.confidence.clamp(0.0, 1.0),
    }
}

pub(crate) fn normalized_file_type(row: &IndexedFileRow) -> String {
    if row.file_type.is_empty() || row.file_type == "Other" {
        infer_file_type(&row.extension, row.is_dir).to_string()
    } else {
        row.file_type.clone()
    }
}

fn default_if_empty(value: &str, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn json_value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn json_value_to_f64(value: &Value) -> f64 {
    match value {
        Value::Number(value) => value.as_f64().unwrap_or(0.0),
        Value::String(value) => value.parse().unwrap_or(0.0),
        Value::Bool(true) => 1.0,
        Value::Bool(false) => 0.0,
        _ => 0.0,
    }
}

fn json_value_to_i64(value: &Value) -> i64 {
    json_value_to_f64(value) as i64
}
