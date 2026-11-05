//! Live binding and Organization Plan projection for Managed AI semantics.
//!
//! Managed AI describes a file; this adapter verifies that the description is
//! still bound to the current Global Index row, enabled managed scope, current
//! provider policy, and completed queue item before projecting it into the
//! existing deterministic Operation Preview path.

use super::*;
use crate::{
    ai::{
        schema::AIProviderKind,
        semantic::{SemanticAssessmentV1, SemanticSourceBinding},
        settings::{normalize_ai_settings, AI_SETTINGS_KEY},
    },
    db::{build_target_path, OrganizeRootConfig},
    global_index::{managed_worker::metadata_fingerprint, models::normalize_path},
    settings::{AppSettings, APP_SETTINGS_KEY},
};
use rusqlite::OptionalExtension;

enum AssessmentResolution {
    NotManaged,
    Pending,
    Unavailable(&'static str),
    Current(Box<SemanticAssessmentV1>),
}

pub(super) struct OrganizationCurrentProjection {
    pub(super) proposal: Proposal,
    pub(super) preview: Option<OperationPreviewDto>,
    pub(super) semantic_explanation: Option<OrganizationSemanticExplanationDto>,
}

pub(super) fn current_organization_proposal(
    conn: &rusqlite::Connection,
    row: &IndexedFileRow,
) -> Result<Proposal, DbError> {
    Ok(current_organization_projection(conn, row)?.proposal)
}

pub(super) fn current_organization_projection(
    conn: &rusqlite::Connection,
    row: &IndexedFileRow,
) -> Result<OrganizationCurrentProjection, DbError> {
    if super::one_off::is_source_id(&row.id) {
        return super::one_off::projection(conn, row);
    }
    match resolve_current_assessment(conn, row)? {
        AssessmentResolution::NotManaged | AssessmentResolution::Pending => {
            Ok(OrganizationCurrentProjection {
                proposal: pending_proposal(row),
                preview: None,
                semantic_explanation: None,
            })
        }
        AssessmentResolution::Unavailable(code) => Ok(OrganizationCurrentProjection {
            proposal: unavailable_proposal(row, code),
            preview: None,
            semantic_explanation: None,
        }),
        AssessmentResolution::Current(assessment) => {
            let semantic_explanation = OrganizationSemanticExplanationDto {
                assessment_fingerprint: assessment.fingerprint(),
                reason: assessment.reason.chars().take(512).collect(),
                provider_kind: None,
                model: None,
                provider_settings_current: None,
            };
            let mut semantic_row = row.clone();
            semantic_row.file_type = assessment.file_type.clone();
            semantic_row.purpose = assessment.purpose.as_str().to_string();
            semantic_row.lifecycle = assessment.lifecycle.as_str().to_string();
            semantic_row.context = assessment.context.clone();
            semantic_row.risk_level = assessment.risk_level.as_str().to_string();
            semantic_row.suggested_action = assessment.suggested_action.as_str().to_string();
            semantic_row.suggested_name = assessment
                .suggested_name
                .clone()
                .unwrap_or_else(|| row.name.clone());
            semantic_row.confidence = assessment.confidence;
            semantic_row.classification_reason = assessment.reason.clone();
            semantic_row.classification_status = "classified".to_string();
            semantic_row.requires_confirmation = assessment.requires_confirmation;
            semantic_row.suggested_target_path =
                semantic_target_directory(conn, &semantic_row, &assessment)?;
            let preview = operation_preview_from_indexed(semantic_row);
            let mut proposal = proposal_from_preview(
                &row.path,
                &row.name,
                "classified",
                assessment.suggested_action.as_str(),
                preview.clone(),
            );
            proposal.fingerprint = blake3::hash(
                [
                    proposal.fingerprint.as_str(),
                    "semantic-v1",
                    &assessment.fingerprint(),
                ]
                .join("\0")
                .as_bytes(),
            )
            .to_hex()
            .to_string();
            Ok(OrganizationCurrentProjection {
                proposal,
                preview,
                semantic_explanation: Some(semantic_explanation),
            })
        }
    }
}

fn resolve_current_assessment(
    conn: &rusqlite::Connection,
    row: &IndexedFileRow,
) -> Result<AssessmentResolution, DbError> {
    let normalized_path = normalize_path(&row.path);
    let entries = {
        let mut statement = conn.prepare(
            "SELECT entry.id, entry.volume_id, entry.platform_file_id, entry.name,
                    entry.path, entry.extension, entry.is_directory, entry.size,
                    entry.modified_at_fs, entry.is_stale, volume.enabled
             FROM global_entries entry
             JOIN global_volumes volume ON volume.id = entry.volume_id
             WHERE entry.path_normalized = ?1 ORDER BY entry.id LIMIT 2",
        )?;
        let rows = statement.query_map(params![normalized_path], |db_row| {
            Ok(GlobalIdentity {
                id: db_row.get(0)?,
                volume_id: db_row.get(1)?,
                platform_file_id: db_row.get(2)?,
                name: db_row.get(3)?,
                path: db_row.get(4)?,
                extension: db_row.get(5)?,
                is_directory: db_row.get::<_, i64>(6)? != 0,
                size: db_row.get(7)?,
                modified_at_fs: db_row.get(8)?,
                is_stale: db_row.get::<_, i64>(9)? != 0,
                volume_enabled: db_row.get::<_, i64>(10)? != 0,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    if entries.is_empty() {
        return Ok(if has_legacy_ai_classification(row) {
            AssessmentResolution::Unavailable("managed_ai_source_identity_unavailable")
        } else {
            AssessmentResolution::NotManaged
        });
    }
    if entries.len() != 1 {
        return Ok(AssessmentResolution::Unavailable(
            "managed_ai_source_identity_ambiguous",
        ));
    }
    let entry = &entries[0];
    let has_any_managed_link: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM managed_entries WHERE global_entry_id = ?1)",
        params![entry.id],
        |db_row| db_row.get(0),
    )?;
    let state = conn
        .query_row(
            "SELECT status, input_fingerprint, provider, model, classification_json, user_corrected
             FROM ai_analysis_state WHERE global_entry_id = ?1",
            params![entry.id],
            |db_row| {
                Ok(AnalysisState {
                    status: db_row.get(0)?,
                    input_fingerprint: db_row.get(1)?,
                    provider: db_row.get(2)?,
                    model: db_row.get(3)?,
                    classification_json: db_row.get(4)?,
                    user_corrected: db_row.get::<_, i64>(5)? != 0,
                })
            },
        )
        .optional()?;

    if !has_any_managed_link && state.is_none() {
        return Ok(if has_legacy_ai_classification(row) {
            AssessmentResolution::Unavailable("managed_ai_semantic_state_required")
        } else {
            AssessmentResolution::NotManaged
        });
    }
    if entry.is_stale || !entry.volume_enabled || row.is_stale || !identity_matches(entry, row) {
        return Ok(AssessmentResolution::Unavailable(
            "managed_ai_source_identity_stale",
        ));
    }

    let settings_json = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![AI_SETTINGS_KEY],
            |db_row| db_row.get::<_, String>(0),
        )
        .optional()?;
    let settings = match settings_json {
        Some(value) => match serde_json::from_str(&value) {
            Ok(settings) => normalize_ai_settings(settings),
            Err(_) => {
                return Ok(AssessmentResolution::Unavailable(
                    "managed_ai_provider_policy_invalid",
                ));
            }
        },
        None => normalize_ai_settings(Default::default()),
    };
    if !settings.enabled {
        return Ok(AssessmentResolution::Unavailable(
            "managed_ai_provider_disabled",
        ));
    }
    let current_provider = match settings.provider {
        AIProviderKind::Ollama => "local",
        AIProviderKind::OpenAICompatible => "cloud",
    };
    let effective_scope_id = conn
        .query_row(
            "SELECT scope.id
             FROM managed_entries managed
             JOIN managed_scopes scope ON scope.id = managed.managed_scope_id
             WHERE managed.global_entry_id = ?1
               AND managed.enabled = 1 AND scope.enabled = 1
               AND ((?2 = 'local' AND scope.allow_local_ai = 1)
                 OR (?2 = 'cloud' AND scope.allow_cloud_ai = 1))
             ORDER BY length(rtrim(scope.path, '/\\')) DESC, scope.id ASC
             LIMIT 1",
            params![entry.id, current_provider],
            |db_row| db_row.get::<_, String>(0),
        )
        .optional()?;
    let Some(effective_scope_id) = effective_scope_id else {
        return Ok(AssessmentResolution::Unavailable(
            "managed_ai_scope_policy_unavailable",
        ));
    };

    let fingerprint = metadata_fingerprint(
        &entry.volume_id,
        &entry.platform_file_id,
        &entry.name,
        entry.size,
        entry.modified_at_fs,
        entry.is_directory,
    );
    let Some(state) = state else {
        return Ok(AssessmentResolution::Pending);
    };
    if state.user_corrected {
        return Ok(AssessmentResolution::Unavailable(
            "managed_ai_user_correction_requires_review",
        ));
    }
    if state.input_fingerprint != fingerprint || state.provider != current_provider {
        return Ok(AssessmentResolution::Unavailable(
            "managed_ai_assessment_stale",
        ));
    }
    match state.status.as_str() {
        "pending" | "running" => return Ok(AssessmentResolution::Pending),
        "completed" => {}
        _ => {
            return Ok(AssessmentResolution::Unavailable(
                "managed_ai_assessment_unavailable",
            ));
        }
    }
    if state.model != settings.model {
        return Ok(AssessmentResolution::Unavailable(
            "managed_ai_model_changed",
        ));
    }
    let Some(classification_json) = state.classification_json.as_deref() else {
        return Ok(AssessmentResolution::Unavailable(
            "managed_ai_assessment_missing",
        ));
    };

    let completed_job_exists: bool = conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM ai_jobs job
            JOIN ai_job_items item ON item.job_id = job.id
            WHERE job.global_entry_id = ?1 AND job.managed_scope_id = ?2
              AND job.input_fingerprint = ?3 AND job.provider = ?4
              AND job.model = ?5 AND job.status = 'completed'
              AND item.global_entry_id = job.global_entry_id
              AND item.status = 'completed'
        )",
        params![
            entry.id,
            effective_scope_id,
            fingerprint,
            current_provider,
            settings.model
        ],
        |db_row| db_row.get(0),
    )?;
    if !completed_job_exists {
        return Ok(AssessmentResolution::Unavailable(
            "managed_ai_completed_job_binding_missing",
        ));
    }

    let binding = SemanticSourceBinding {
        one_off: None,
        global_entry_id: entry.id.clone(),
        managed_scope_id: effective_scope_id,
        input_fingerprint: fingerprint,
        provider: current_provider.to_string(),
    };
    let assessment = match SemanticAssessmentV1::decode_stored(
        classification_json,
        binding,
        &row.name,
        &row.extension,
        row.is_dir,
    ) {
        Ok(assessment) => assessment,
        Err(_) => {
            return Ok(AssessmentResolution::Unavailable(
                "managed_ai_assessment_schema_invalid",
            ));
        }
    };
    if state.model != settings.model
        || assessment.file_type.trim().is_empty()
        || assessment.reason.trim().is_empty()
        || assessment.context.chars().count() > 512
        || assessment.keywords.len() > 16
        || assessment.risk_level.is_invalid()
        || assessment.purpose.is_invalid()
        || assessment.lifecycle.is_invalid()
        || assessment.suggested_action.is_invalid()
        || (assessment.risk_level.as_str() != "Normal"
            && (assessment.suggested_action.as_str() != "Review"
                || !assessment.requires_confirmation))
        || (assessment.confidence < 0.8 && !assessment.requires_confirmation)
        || (matches!(
            assessment.suggested_action.as_str(),
            "Move" | "MoveAndRename" | "Archive"
        ) && assessment.target_template.is_none())
    {
        return Ok(AssessmentResolution::Unavailable(
            "managed_ai_assessment_schema_invalid",
        ));
    }
    Ok(AssessmentResolution::Current(Box::new(assessment)))
}

fn semantic_target_directory(
    conn: &rusqlite::Connection,
    row: &IndexedFileRow,
    assessment: &SemanticAssessmentV1,
) -> Result<String, DbError> {
    match assessment.suggested_action.as_str() {
        "Rename" if assessment.target_template.is_none() => Ok(parent_directory(&row.path)),
        "Move" | "MoveAndRename" | "Archive" | "Rename" => {
            let settings_json = conn
                .query_row(
                    "SELECT value FROM app_settings WHERE key = ?1",
                    params![APP_SETTINGS_KEY],
                    |db_row| db_row.get::<_, String>(0),
                )
                .optional()?;
            let settings = match settings_json {
                Some(value) => serde_json::from_str::<AppSettings>(&value).map_err(|_| {
                    DbError::Validation("organization_app_settings_invalid".to_string())
                })?,
                None => AppSettings::default(),
            };
            Ok(build_target_path(
                row,
                &assessment.file_type,
                assessment.target_template.as_deref(),
                &settings.folder_naming_language,
                &OrganizeRootConfig::from(&settings),
            ))
        }
        _ => Ok(String::new()),
    }
}

fn identity_matches(entry: &GlobalIdentity, row: &IndexedFileRow) -> bool {
    normalize_path(&entry.path) == normalize_path(&row.path)
        && entry.name == row.name
        && entry.extension.eq_ignore_ascii_case(&row.extension)
        && entry.size == row.size
        && entry.modified_at_fs.unwrap_or_default() == row.mtime
        && entry.is_directory == row.is_dir
}

fn has_legacy_ai_classification(row: &IndexedFileRow) -> bool {
    serde_json::from_str::<Vec<String>>(&row.matched_rules)
        .map(|rules| rules.iter().any(|rule| rule.starts_with("ai:")))
        .unwrap_or_else(|_| row.matched_rules.contains("ai:"))
}

pub(super) fn pending_proposal(row: &IndexedFileRow) -> Proposal {
    Proposal {
        fingerprint: blake3::hash(
            [row.path.as_str(), "managed-ai-pending"]
                .join("\0")
                .as_bytes(),
        )
        .to_hex()
        .to_string(),
        kind: "keep".to_string(),
        target_directory: parent_directory(&row.path),
        name: row.name.clone(),
        target_path: row.path.clone(),
        validity: "needs_analysis".to_string(),
        confidence: 0.0,
        risk: "Unknown".to_string(),
        requires_confirmation: true,
        blocking_code: None,
        blocking_detail: None,
        preview_id: None,
    }
}

pub(super) fn unavailable_proposal(row: &IndexedFileRow, code: &str) -> Proposal {
    Proposal {
        fingerprint: blake3::hash([row.path.as_str(), code].join("\0").as_bytes())
            .to_hex()
            .to_string(),
        kind: "blocked".to_string(),
        target_directory: parent_directory(&row.path),
        name: row.name.clone(),
        target_path: row.path.clone(),
        validity: "blocked".to_string(),
        confidence: 0.0,
        risk: "Unknown".to_string(),
        requires_confirmation: true,
        blocking_code: Some(code.to_string()),
        blocking_detail: Some(
            "The current managed assessment cannot be verified. Review or analyze the source again."
                .to_string(),
        ),
        preview_id: None,
    }
}

struct GlobalIdentity {
    id: String,
    volume_id: String,
    platform_file_id: String,
    name: String,
    path: String,
    extension: String,
    is_directory: bool,
    size: i64,
    modified_at_fs: Option<i64>,
    is_stale: bool,
    volume_enabled: bool,
}

struct AnalysisState {
    status: String,
    input_fingerprint: String,
    provider: String,
    model: String,
    classification_json: Option<String>,
    user_corrected: bool,
}
