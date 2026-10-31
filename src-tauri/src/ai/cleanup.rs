use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, State, WebviewWindow};

use super::{
    ollama::OllamaProvider,
    openai_compatible::OpenAICompatibleProvider,
    prompts::{
        ai_cleanup_analysis_system_prompt,
        build_ai_cleanup_analysis_prompt as build_ai_cleanup_analysis_prompt_body,
        clean_ai_json_text, extract_first_json_value,
    },
    provider::AIProvider,
    schema::{AIChatMessage, AIChatRequest, AIProviderKind, AIProviderOptions},
    settings::{
        get_ai_settings_for_db_with_revision, normalize_ai_settings, AISettings, AI_SETTINGS_KEY,
    },
    trace::{AITraceContext, AITraceOperation},
};
use crate::{
    db::{
        AnalysisAiAssessmentPublication, AnalysisAiFindingPrecondition, AnalysisAiPublicationBatch,
        AnalysisDetectorDto, AnalysisFindingDto, AnalysisRunDto, Database,
    },
    ids::new_job_id,
    storage_analyzer::{
        resolve_analysis_candidates_for_cleanup, storage_candidate_from_analysis_finding,
        CleanupActionKind, CleanupTier, StorageCandidate,
    },
    window_auth::require_main_window,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AICleanupInputCandidate {
    pub candidate_id: String,
    pub name: String,
    pub parent_name: Option<String>,
    pub path: Option<String>,
    pub size: u64,
    pub tier: String,
    pub category: String,
    pub reason: String,
    pub suggested_action: String,
    pub risk_note: Option<String>,
    pub trash_allowed: bool,
    pub selected_by_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AICleanupAnalysisOutput {
    pub candidate_id: String,
    pub tier: Option<String>,
    pub category: Option<String>,
    pub suggested_action: Option<String>,
    pub confidence: Option<f64>,
    pub reason: Option<String>,
    pub risk_note: Option<String>,
    pub trash_allowed: Option<bool>,
    pub selected_by_default: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AICleanupAnalysisResponse {
    analyses: Vec<AICleanupAnalysisOutput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AICleanupAnalysisEnvelope {
    result: AICleanupAnalysisResponse,
}

#[tauri::command]
pub async fn analyze_cleanup_candidates_with_ai<R: Runtime>(
    window: WebviewWindow<R>,
    job_id: String,
    ids: Vec<String>,
    app: AppHandle<R>,
    db: State<'_, Database>,
) -> Result<Vec<StorageCandidate>, String> {
    require_main_window(&window)?;
    let app_data_dir = app.path().app_data_dir().ok();
    let db = db.inner().clone();
    let app_for_events = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (settings, settings_revision) =
            get_ai_settings_for_db_with_revision(&db).map_err(|error| error.to_string())?;
        let settings = normalize_ai_settings(settings);
        let selections = ids
            .iter()
            .map(|id| {
                let finding = db
                    .get_analysis_finding(id)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("AI cleanup finding not found: {id}"))?;
                Ok(crate::storage_analyzer::CleanupFindingSelection {
                    finding_id: id.clone(),
                    expected_revision: finding.revision,
                    review_confirmation: None,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let expected_revisions = selections
            .iter()
            .map(|selection| (selection.finding_id.clone(), selection.expected_revision))
            .collect::<HashMap<_, _>>();
        let candidates = resolve_analysis_candidates_for_cleanup(&db, &job_id, &selections, false)?;
        let updated = analyze_cleanup_candidates_with_configured_provider(
            &db,
            &job_id,
            candidates,
            &expected_revisions,
            &settings,
            &settings_revision,
            app_data_dir,
        )?;
        let mut persisted = Vec::with_capacity(updated.len());
        for finding in updated {
            if let Ok(run) = db.get_analysis_run(&finding.run_id) {
                let _ = app_for_events.emit(crate::analysis::ANALYSIS_RUN_UPDATED_EVENT, run);
            }
            persisted.push(storage_candidate_from_analysis_finding(&finding)?);
        }
        Ok(persisted)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[path = "cleanup/publication.rs"]
mod publication;
use publication::analyze_cleanup_candidates_with_configured_provider;
pub(crate) use publication::has_current_ai_assessment;
#[cfg(test)]
use publication::{
    analyze_cleanup_candidates_with_provider, cleanup_ai_coverage, CleanupAiProviderContext,
};

fn call_ai_cleanup_provider(
    provider: &dyn AIProvider,
    settings: &AISettings,
    candidates: &[StorageCandidate],
    retry_json_only: bool,
    trace_context: AITraceContext,
) -> Result<String, String> {
    let mut messages = build_ai_cleanup_analysis_prompt(candidates, settings)?;
    if retry_json_only {
        messages.push(AIChatMessage {
            role: "user".to_string(),
            content: "上一次输出不是有效 JSON。请只返回一个 JSON 对象，不要 Markdown，不要解释，不要 thinking，不要代码块。".to_string(),
        });
    }
    provider
        .chat_json(AIChatRequest {
            messages,
            model: settings.model.clone(),
            temperature: if retry_json_only {
                0.0
            } else {
                settings.temperature
            },
            max_tokens: settings.max_tokens,
            force_json: settings.force_json_output || retry_json_only,
            provider_options: AIProviderOptions {
                enable_thinking: Some(if retry_json_only {
                    false
                } else {
                    settings.enable_thinking
                }),
                reasoning_effort: settings.reasoning_effort.clone(),
                extra_body_json: None,
                use_response_format: retry_json_only.then_some(true),
                trace_context: Some(trace_context),
            },
        })
        .map_err(|error| sanitize_ai_cleanup_error(error.to_string(), &settings.api_key))
}

pub(crate) fn build_ai_cleanup_analysis_prompt(
    candidates: &[StorageCandidate],
    settings: &AISettings,
) -> Result<Vec<AIChatMessage>, String> {
    let input = candidates
        .iter()
        .map(|candidate| ai_cleanup_input_candidate(candidate, settings))
        .collect::<Vec<_>>();
    Ok(vec![
        AIChatMessage {
            role: "system".to_string(),
            content: ai_cleanup_analysis_system_prompt(settings.enable_thinking),
        },
        AIChatMessage {
            role: "user".to_string(),
            content: build_ai_cleanup_analysis_prompt_body(&input)?,
        },
    ])
}

fn ai_cleanup_input_candidate(
    candidate: &StorageCandidate,
    settings: &AISettings,
) -> AICleanupInputCandidate {
    AICleanupInputCandidate {
        candidate_id: candidate.id.clone(),
        name: candidate.name.clone(),
        parent_name: settings
            .send_parent_path
            .then(|| parent_name(&candidate.path))
            .filter(|value| !value.is_empty()),
        path: settings.send_full_path.then(|| candidate.path.clone()),
        size: candidate.size,
        tier: tier_to_string(&candidate.tier).to_string(),
        category: candidate.category.clone(),
        reason: candidate.reason.clone(),
        suggested_action: action_to_string(&candidate.suggested_action).to_string(),
        risk_note: candidate.risk_note.clone(),
        trash_allowed: candidate.trash_allowed,
        selected_by_default: candidate.selected_by_default,
    }
}

pub(crate) fn parse_ai_cleanup_analysis_response(
    content: &str,
) -> Result<Vec<AICleanupAnalysisOutput>, String> {
    let cleaned = clean_ai_json_text(content);
    let value = serde_json::from_str::<serde_json::Value>(&cleaned)
        .or_else(|_| {
            extract_first_json_value(content)
                .ok_or_else(|| {
                    serde_json::Error::io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "no JSON value found",
                    ))
                })
                .and_then(|value| serde_json::from_str::<serde_json::Value>(&value))
        })
        .map_err(|error| ai_cleanup_json_error(content, &error.to_string()))?;
    cleanup_outputs_from_value(value).map_err(|error| ai_cleanup_json_error(content, &error))
}

pub(crate) fn merge_ai_cleanup_analysis(
    original: &StorageCandidate,
    ai_result: &AICleanupAnalysisOutput,
    app_data_dir: Option<&PathBuf>,
) -> StorageCandidate {
    if ai_result.candidate_id != original.id {
        return original.clone();
    }

    let safety = CleanupPathSafety::from_candidate(original, app_data_dir);
    let mut merged = original.clone();
    if let Some(category) = sanitized_text(ai_result.category.as_deref(), 120) {
        merged.category = category;
    }
    if let Some(reason) = sanitized_text(ai_result.reason.as_deref(), 600) {
        merged.reason = reason;
    }
    if let Some(risk_note) = sanitized_text(ai_result.risk_note.as_deref(), 600) {
        merged.risk_note = Some(risk_note);
    }
    let _confidence = ai_result.confidence.unwrap_or(0.0).clamp(0.0, 1.0);

    let proposed_tier = ai_result
        .tier
        .as_deref()
        .and_then(parse_cleanup_tier)
        .unwrap_or_else(|| original.tier.clone());
    merged.tier = more_conservative_tier(&original.tier, &proposed_tier);
    if safety.force_caution {
        merged.tier = CleanupTier::Caution;
    } else if safety.force_review && merged.tier == CleanupTier::Safe {
        merged.tier = CleanupTier::Review;
    }

    let proposed_action = ai_result
        .suggested_action
        .as_deref()
        .and_then(parse_cleanup_action)
        .unwrap_or_else(|| original.suggested_action.clone());
    merged.suggested_action = conservative_action(original, proposed_action, &merged.tier, &safety);

    let proposed_trash_allowed = ai_result.trash_allowed.unwrap_or(original.trash_allowed);
    merged.trash_allowed = original.trash_allowed
        && proposed_trash_allowed
        && merged.tier == CleanupTier::Safe
        && merged.suggested_action == CleanupActionKind::MoveToTrash
        && !safety.prevent_trash;

    if !merged.trash_allowed && merged.suggested_action == CleanupActionKind::MoveToTrash {
        merged.suggested_action = fallback_non_trash_action(original, &safety);
    }

    let proposed_selected = ai_result
        .selected_by_default
        .unwrap_or(original.selected_by_default);
    merged.selected_by_default = original.selected_by_default
        && proposed_selected
        && merged.tier == CleanupTier::Safe
        && merged.trash_allowed
        && !safety.prevent_selected;
    if merged.tier == CleanupTier::Caution {
        merged.selected_by_default = false;
    }

    merged
}

fn conservative_action(
    original: &StorageCandidate,
    proposed: CleanupActionKind,
    final_tier: &CleanupTier,
    safety: &CleanupPathSafety,
) -> CleanupActionKind {
    if proposed == CleanupActionKind::MoveToTrash {
        if safety.prevent_trash || *final_tier != CleanupTier::Safe {
            return fallback_non_trash_action(original, safety);
        }
        if !original.trash_allowed {
            return original.suggested_action.clone();
        }
        if matches!(
            original.suggested_action,
            CleanupActionKind::None
                | CleanupActionKind::AppInternalCleanup
                | CleanupActionKind::UninstallAdvice
        ) {
            return original.suggested_action.clone();
        }
        if original.suggested_action == CleanupActionKind::Reveal && !original.trash_allowed {
            return CleanupActionKind::Reveal;
        }
    }
    proposed
}

fn fallback_non_trash_action(
    original: &StorageCandidate,
    safety: &CleanupPathSafety,
) -> CleanupActionKind {
    if matches!(
        original.suggested_action,
        CleanupActionKind::None
            | CleanupActionKind::Reveal
            | CleanupActionKind::UninstallAdvice
            | CleanupActionKind::AppInternalCleanup
    ) {
        return original.suggested_action.clone();
    }
    if safety.system_path {
        CleanupActionKind::None
    } else {
        CleanupActionKind::Reveal
    }
}

#[derive(Debug, Clone, Default)]
struct CleanupPathSafety {
    force_caution: bool,
    force_review: bool,
    prevent_trash: bool,
    prevent_selected: bool,
    system_path: bool,
}

impl CleanupPathSafety {
    fn from_candidate(candidate: &StorageCandidate, app_data_dir: Option<&PathBuf>) -> Self {
        let lower = normalize_path_text(&candidate.path).to_ascii_lowercase();
        let extension = lower
            .rsplit_once('.')
            .map(|(_, extension)| extension)
            .unwrap_or_default();
        let system_path = is_system_path_text(&lower);
        let program_files = is_program_files_path_text(&lower);
        let program_data = lower.contains("/programdata/");
        let app_data = is_appdata_path_text(&lower);
        let browser_profile = is_browser_profile_path_text(&lower);
        let chat_database = is_chat_database_path_text(&lower, extension);
        let database = is_database_extension(extension);
        let vm_image = is_virtual_machine_image(extension);
        let app_internal = app_data_dir
            .map(|dir| is_same_or_child_text(&lower, &normalize_path_text(&dir.to_string_lossy())))
            .unwrap_or(false);
        let force_caution = system_path
            || program_files
            || program_data
            || browser_profile
            || chat_database
            || database
            || vm_image
            || app_internal;
        let force_review = app_data;
        let prevent_trash = force_caution || app_data;
        let prevent_selected = force_caution || app_data;
        Self {
            force_caution,
            force_review,
            prevent_trash,
            prevent_selected,
            system_path,
        }
    }
}

fn parse_cleanup_tier(value: &str) -> Option<CleanupTier> {
    match value.trim() {
        "Safe" => Some(CleanupTier::Safe),
        "Review" => Some(CleanupTier::Review),
        "Caution" => Some(CleanupTier::Caution),
        _ => None,
    }
}

fn parse_cleanup_action(value: &str) -> Option<CleanupActionKind> {
    match value.trim() {
        "MoveToTrash" => Some(CleanupActionKind::MoveToTrash),
        "Reveal" => Some(CleanupActionKind::Reveal),
        "UninstallAdvice" => Some(CleanupActionKind::UninstallAdvice),
        "AppInternalCleanup" => Some(CleanupActionKind::AppInternalCleanup),
        "None" => Some(CleanupActionKind::None),
        _ => None,
    }
}

fn more_conservative_tier(original: &CleanupTier, proposed: &CleanupTier) -> CleanupTier {
    if tier_rank(proposed) > tier_rank(original) {
        proposed.clone()
    } else {
        original.clone()
    }
}

fn tier_rank(tier: &CleanupTier) -> u8 {
    match tier {
        CleanupTier::Safe => 0,
        CleanupTier::Review => 1,
        CleanupTier::Caution => 2,
    }
}

fn tier_to_string(tier: &CleanupTier) -> &'static str {
    match tier {
        CleanupTier::Safe => "Safe",
        CleanupTier::Review => "Review",
        CleanupTier::Caution => "Caution",
    }
}

fn action_to_string(action: &CleanupActionKind) -> &'static str {
    match action {
        CleanupActionKind::MoveToTrash => "MoveToTrash",
        CleanupActionKind::Reveal => "Reveal",
        CleanupActionKind::UninstallAdvice => "UninstallAdvice",
        CleanupActionKind::AppInternalCleanup => "AppInternalCleanup",
        CleanupActionKind::None => "None",
    }
}

fn sanitized_text(value: Option<&str>, limit: usize) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(limit).collect())
}

fn parent_name(path: &str) -> String {
    let normalized = normalize_path_text(path);
    normalized
        .rsplit_once('/')
        .and_then(|(parent, _)| parent.rsplit('/').next())
        .unwrap_or_default()
        .to_string()
}

fn normalize_path_text(path: &str) -> String {
    path.replace('\\', "/")
}

fn is_same_or_child_text(path: &str, parent: &str) -> bool {
    let path = path.trim_end_matches('/');
    let parent = parent.trim_end_matches('/').to_ascii_lowercase();
    path == parent || path.starts_with(&format!("{parent}/"))
}

fn is_system_path_text(lower: &str) -> bool {
    lower.contains("/windows/")
        || lower.ends_with("/windows")
        || lower.contains("/windows/system32/")
        || lower.contains("/system volume information/")
        || lower.contains("/$recycle.bin/")
}

fn is_program_files_path_text(lower: &str) -> bool {
    lower.contains("/program files/") || lower.contains("/program files (x86)/")
}

fn is_appdata_path_text(lower: &str) -> bool {
    lower.contains("/appdata/local/")
        || lower.contains("/appdata/roaming/")
        || lower.contains("/appdata/locallow/")
}

fn is_browser_profile_path_text(lower: &str) -> bool {
    (lower.contains("/google/chrome/user data")
        || lower.contains("/microsoft/edge/user data")
        || lower.contains("/mozilla/firefox/profiles"))
        && !lower.contains("/cache/")
}

fn is_chat_database_path_text(lower: &str, extension: &str) -> bool {
    (lower.contains("wechat") || lower.contains("/qq/") || lower.contains("tencent"))
        && is_database_extension(extension)
}

fn is_database_extension(extension: &str) -> bool {
    matches!(extension, "db" | "sqlite" | "sqlite3" | "mdb" | "accdb")
}

fn is_virtual_machine_image(extension: &str) -> bool {
    matches!(
        extension,
        "vmdk" | "vhd" | "vhdx" | "qcow2" | "ova" | "avhdx"
    )
}

fn cleanup_outputs_from_value(
    value: serde_json::Value,
) -> Result<Vec<AICleanupAnalysisOutput>, String> {
    if value.is_array() {
        return serde_json::from_value::<Vec<AICleanupAnalysisOutput>>(value)
            .map_err(|error| format!("cleanup array schema mismatch: {error}"));
    }

    if value.get("analyses").is_some() {
        return serde_json::from_value::<AICleanupAnalysisResponse>(value)
            .map(|response| response.analyses)
            .map_err(|error| format!("cleanup object schema mismatch: {error}"));
    }

    if let Some(result) = value.get("result") {
        if result.get("analyses").is_some() {
            return serde_json::from_value::<AICleanupAnalysisEnvelope>(value)
                .map(|envelope| envelope.result.analyses)
                .map_err(|error| format!("result.analyses schema mismatch: {error}"));
        }
    }

    Err("missing analyses array".to_string())
}

fn ai_cleanup_json_error(content: &str, detail: &str) -> String {
    let lower = content.to_ascii_lowercase();
    if lower.contains("<think>") {
        return "模型返回了 thinking 内容，导致 JSON 解析失败。请关闭 Thinking，或换用非思考模型。"
            .to_string();
    }
    if lower.contains("```") {
        return "模型返回了 Markdown 代码块，Fylura 已尝试提取 JSON，但结构仍不符合要求。"
            .to_string();
    }
    format!("模型返回的内容不是 Fylura 需要的 JSON 格式。已尝试清洗，但仍失败：{detail}")
}

fn sanitize_ai_cleanup_error(message: String, api_key: &str) -> String {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        message
    } else {
        message.replace(api_key, "[redacted]")
    }
}

#[cfg(test)]
#[path = "cleanup/tests.rs"]
mod tests;
