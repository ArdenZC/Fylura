use serde::Serialize;
use tauri::{Runtime, State, WebviewWindow};

use rusqlite::{params, OptionalExtension};

use super::{
    classification::{
        build_ai_classification_prompt, collect_selected_ai_classification_targets,
        parse_ai_classification_response, sanitize_ai_classification_result, AIClassificationIdMap,
    },
    openai_compatible::{
        debug_extract_openai_response, AIRawProviderResponse, OpenAICompatibleProvider,
    },
    prompts::clean_ai_json_text,
    schema::{AIChatRequest, AIProviderKind, AIProviderOptions, AIProviderPresetId},
    settings::{
        get_ai_settings_for_db, get_ai_settings_with_store, normalize_ai_settings, AISettings,
        CredentialStore, SystemCredentialStore,
    },
};
use crate::{
    db::{normalize_path_text, trim_trailing_path_separators, Database},
    window_auth::require_main_window,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AIDebugClassificationResult {
    pub provider: AIProviderKind,
    pub preset: AIProviderPresetId,
    pub model: String,
    pub base_url: String,
    pub chat_path: String,
    pub force_json_output: bool,
    pub enable_thinking: bool,
    pub max_tokens: u32,
    pub batch_size: usize,
    pub request_used_response_format: bool,
    pub request_used_thinking_field: Option<String>,
    pub http_status: u16,
    pub provider_response_summary: String,
    pub raw_response_preview: String,
    pub message_content_preview: String,
    pub reasoning_content_preview: String,
    pub extracted_content_preview: String,
    pub cleaned_content_preview: String,
    pub parse_stage: String,
    pub parse_error: Option<String>,
    pub success: bool,
    pub ref_id: String,
    pub real_file_id: String,
    pub path: String,
    pub model_returned_ref_id: Option<String>,
    pub model_returned_id: Option<String>,
    pub id_mapping_matched: bool,
    pub missing_optional_fields: Vec<String>,
    pub fallback_applied: bool,
    pub item_parse_warnings: Vec<String>,
}

pub trait AIDebugRawProvider {
    fn send_raw(&self, request: AIChatRequest) -> Result<AIRawProviderResponse, String>;
}

impl AIDebugRawProvider for OpenAICompatibleProvider {
    fn send_raw(&self, request: AIChatRequest) -> Result<AIRawProviderResponse, String> {
        self.send_chat_request_raw(request)
            .map_err(|error| error.to_string())
    }
}

#[tauri::command]
pub async fn debug_ai_classification_once<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    target: Option<String>,
    file_id: Option<String>,
) -> Result<AIDebugClassificationResult, String> {
    require_main_window(&window)?;
    if !cfg!(any(debug_assertions, feature = "ai-debug")) {
        return Err("AI debug commands are disabled in production builds.".to_string());
    }
    let target = target
        .or(file_id)
        .ok_or_else(|| "file id or path is required for AI classification debug.".to_string())?;
    let db = db.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let settings =
            normalize_ai_settings(get_ai_settings_for_db(&db).map_err(|error| error.to_string())?);
        if settings.provider != AIProviderKind::OpenAICompatible {
            return Err(
                "AI classification debug currently supports OpenAI-compatible providers only."
                    .to_string(),
            );
        }
        let provider = OpenAICompatibleProvider::new(settings.clone());
        debug_ai_classification_once_for_db(&db, &target, &provider)
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) fn debug_ai_classification_once_for_db(
    db: &Database,
    target: &str,
    raw_provider: &dyn AIDebugRawProvider,
) -> Result<AIDebugClassificationResult, String> {
    debug_ai_classification_once_with_store(db, target, raw_provider, &SystemCredentialStore)
}

pub(crate) fn debug_ai_classification_once_with_store(
    db: &Database,
    target: &str,
    raw_provider: &dyn AIDebugRawProvider,
    credentials: &impl CredentialStore,
) -> Result<AIDebugClassificationResult, String> {
    let settings = normalize_ai_settings(
        get_ai_settings_with_store(db, credentials).map_err(|error| error.to_string())?,
    );
    if !settings.enabled {
        return Err("请先在设置中启用 AI。".to_string());
    }
    let resolved_file_id = resolve_debug_target_file_id(db, target)?;
    let targets =
        collect_selected_ai_classification_targets(db, std::slice::from_ref(&resolved_file_id))
            .map_err(|error| error.to_string())?;
    if targets.len() != 1 {
        return Err("No indexed non-stale file matched this id or path. Please scan the folder first or choose a file from the library.".to_string());
    }
    let learned_rules = db
        .learned_rule_hints(10)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|hint| hint.summary.trim().chars().take(80).collect::<String>())
        .collect::<Vec<_>>();
    let messages = build_ai_classification_prompt(&targets, &settings, &learned_rules)?;
    let id_map = AIClassificationIdMap::from_targets(&targets);
    let raw = raw_provider
        .send_raw(AIChatRequest {
            messages,
            model: settings.model.clone(),
            temperature: settings.temperature,
            max_tokens: settings.max_tokens,
            force_json: settings.force_json_output,
            provider_options: AIProviderOptions::default(),
        })
        .map_err(|error| sanitize_debug_text(&error, &settings.api_key))?;

    Ok(build_debug_result(&settings, raw, &id_map))
}

fn resolve_debug_target_file_id(db: &Database, target: &str) -> Result<String, String> {
    let cleaned = clean_debug_target(target);
    if cleaned.is_empty() {
        return Err("file id or path is required for AI classification debug.".to_string());
    }

    let conn = db.conn().map_err(|error| error.to_string())?;
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM files WHERE id = ?1 AND is_stale = 0 LIMIT 1",
            params![cleaned],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
    {
        return Ok(id);
    }

    if let Some(id) = conn
        .query_row(
            "SELECT id FROM files WHERE path = ?1 AND is_stale = 0 LIMIT 1",
            params![cleaned],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
    {
        return Ok(id);
    }

    let normalized = normalize_path_text(&cleaned);
    let normalized = trim_trailing_path_separators(&normalized).to_string();
    let target_key = if is_windows_path_like(&cleaned) {
        normalized.to_lowercase()
    } else {
        normalized
    };
    let mut matches = Vec::new();
    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE is_stale = 0")
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (id, path) = row.map_err(|error| error.to_string())?;
        let normalized_path = normalize_path_text(trim_trailing_path_separators(path.trim()));
        let candidate_key = if is_windows_path_like(&cleaned) || is_windows_path_like(&path) {
            normalized_path.to_lowercase()
        } else {
            normalized_path
        };
        if candidate_key == target_key {
            matches.push(id);
        }
    }

    matches.sort();
    matches.dedup();
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err("No indexed non-stale file matched this id or path. Please scan the folder first or choose a file from the library.".to_string()),
        _ => Err("Multiple files matched this debug target. Please select one from the file library.".to_string()),
    }
}

fn clean_debug_target(target: &str) -> String {
    let mut value = target.trim();
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let quoted = (bytes.first() == Some(&b'"') && bytes.last() == Some(&b'"'))
            || (bytes.first() == Some(&b'\'') && bytes.last() == Some(&b'\''));
        if quoted {
            value = &value[1..value.len() - 1];
        }
    }
    trim_trailing_path_separators(value.trim()).to_string()
}

fn is_windows_path_like(path: &str) -> bool {
    path.contains('\\')
        || path.as_bytes().get(1) == Some(&b':')
        || path.starts_with("//")
        || path.starts_with("\\\\")
}

fn build_debug_result(
    settings: &AISettings,
    raw: AIRawProviderResponse,
    id_map: &AIClassificationIdMap,
) -> AIDebugClassificationResult {
    let response_summary = sanitize_debug_text(&raw.response_summary, &settings.api_key);
    let raw_response = sanitize_debug_text(&raw.response_text, &settings.api_key);
    let extracted = debug_extract_openai_response(&raw_response);
    let message_content = extracted.message_content.unwrap_or_default();
    let reasoning_content = extracted.reasoning_content.unwrap_or_default();
    let extracted_content = extracted.extracted_content.unwrap_or_default();
    let cleaned_content = clean_ai_json_text(&extracted_content);
    let parsed_output = parse_ai_classification_response(&extracted_content)
        .ok()
        .and_then(|outputs| outputs.into_iter().next());
    let parsed_returned_ref_id = parsed_output
        .as_ref()
        .and_then(|output| output.ref_id.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let parsed_returned_id = parsed_output
        .as_ref()
        .map(|output| output.id.trim())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let id_mapping = parsed_output
        .as_ref()
        .and_then(|output| id_map.resolve_output(output).ok());
    let model_returned_ref_id = id_mapping
        .as_ref()
        .and_then(|resolution| resolution.returned_ref_id.clone())
        .or(parsed_returned_ref_id);
    let model_returned_id = id_mapping
        .as_ref()
        .and_then(|resolution| resolution.returned_id.clone())
        .or(parsed_returned_id);
    let id_mapping_matched = id_mapping
        .as_ref()
        .map(|resolution| resolution.matched)
        .unwrap_or(false);
    let missing_optional_fields = parsed_output
        .as_ref()
        .map(|output| output.missing_optional_fields.clone())
        .unwrap_or_default();
    let fallback_applied = parsed_output
        .as_ref()
        .map(|output| output.fallback_applied)
        .unwrap_or(false);
    let item_parse_warnings = parsed_output
        .as_ref()
        .map(|output| output.item_parse_warnings.clone())
        .unwrap_or_default();
    let mapping_error = parsed_output
        .as_ref()
        .and_then(|output| id_map.resolve_output(output).err());

    let (success, parse_stage, parse_error) = if let Some(error) = extracted.parse_error {
        (false, "raw_response_extract".to_string(), Some(error))
    } else if extracted_content.trim().is_empty() {
        (
            false,
            "message_content_extract".to_string(),
            Some("No extracted message content found.".to_string()),
        )
    } else if let Some(output) = parsed_output.clone() {
        match sanitize_ai_classification_result(output, id_map) {
            Ok(_) => (true, "parse_ai_classification_response".to_string(), None),
            Err(error) => (
                false,
                "parse_ai_classification_response".to_string(),
                Some(sanitize_debug_text(&error, &settings.api_key)),
            ),
        }
    } else if let Some(error) = mapping_error {
        (
            false,
            "id_mapping".to_string(),
            Some(sanitize_debug_text(&error, &settings.api_key)),
        )
    } else {
        match parse_ai_classification_response(&extracted_content) {
            Ok(_) => (
                false,
                "parse_ai_classification_response".to_string(),
                Some("No classification item found.".to_string()),
            ),
            Err(error) => (
                false,
                "parse_ai_classification_response".to_string(),
                Some(sanitize_debug_text(&error, &settings.api_key)),
            ),
        }
    };
    let entry = id_map.entries.first();

    AIDebugClassificationResult {
        provider: settings.provider,
        preset: settings.preset,
        model: settings.model.clone(),
        base_url: settings.base_url.clone(),
        chat_path: settings.chat_path.clone(),
        force_json_output: settings.force_json_output,
        enable_thinking: settings.enable_thinking,
        max_tokens: settings.max_tokens,
        batch_size: settings.batch_size,
        request_used_response_format: raw.request_used_response_format,
        request_used_thinking_field: raw.request_used_thinking_field,
        http_status: raw.status,
        provider_response_summary: response_summary,
        raw_response_preview: debug_preview(&raw_response, 3000),
        message_content_preview: debug_preview(&message_content, 3000),
        reasoning_content_preview: debug_preview(&reasoning_content, 1000),
        extracted_content_preview: debug_preview(&extracted_content, 3000),
        cleaned_content_preview: debug_preview(&cleaned_content, 3000),
        parse_stage,
        parse_error,
        success,
        ref_id: entry.map(|entry| entry.ref_id.clone()).unwrap_or_default(),
        real_file_id: entry
            .map(|entry| entry.real_file_id.clone())
            .unwrap_or_default(),
        path: entry.map(|entry| entry.path.clone()).unwrap_or_default(),
        model_returned_ref_id,
        model_returned_id,
        id_mapping_matched,
        missing_optional_fields,
        fallback_applied,
        item_parse_warnings,
    }
}

pub fn debug_preview(text: &str, limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }
    let char_count = text.chars().count();
    if char_count <= limit {
        return text.to_string();
    }
    if limit <= 3 {
        return ".".repeat(limit);
    }
    let mut preview = text.chars().take(limit - 3).collect::<String>();
    preview.push_str("...");
    preview
}

fn sanitize_debug_text(text: &str, api_key: &str) -> String {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        text.to_string()
    } else {
        text.replace(api_key, "[redacted]")
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ai::{
            debug::{debug_ai_classification_once_with_store, AIDebugRawProvider},
            openai_compatible::AIRawProviderResponse,
            schema::{AIChatRequest, AIProviderPresetId},
            settings::{
                save_ai_settings_with_store, AISettings, ApiKeyAction, InMemoryCredentialStore,
            },
        },
        db::{Database, InsertFileRequest},
    };
    use rusqlite::{params, Connection};
    use std::{
        cell::RefCell,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static TEST_DB_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

    thread_local! {
        static TEST_CREDENTIALS: RefCell<InMemoryCredentialStore> =
            RefCell::new(InMemoryCredentialStore::default());
    }

    fn save_ai_settings_for_db(
        db: &Database,
        settings: &AISettings,
    ) -> Result<AISettings, crate::db::DbError> {
        let mut settings = settings.clone();
        settings.api_key_action = ApiKeyAction::Replace;
        TEST_CREDENTIALS
            .with(|credentials| save_ai_settings_with_store(db, &settings, &*credentials.borrow()))
    }

    fn debug_ai_classification_once_for_db(
        db: &Database,
        target: &str,
        raw_provider: &dyn AIDebugRawProvider,
    ) -> Result<super::AIDebugClassificationResult, String> {
        TEST_CREDENTIALS.with(|credentials| {
            debug_ai_classification_once_with_store(
                db,
                target,
                raw_provider,
                &*credentials.borrow(),
            )
        })
    }

    #[test]
    fn debug_classification_does_not_write_files_table() {
        let db = test_db();
        insert_test_file(&db, "file-1", "/tmp/Scala期末复习题.pdf");
        save_ai_settings_for_db(&db, &enabled_settings()).expect("save ai settings");
        let provider = StaticRawProvider {
            response: Ok(AIRawProviderResponse {
                status: 200,
                response_text:
                    r#"{"choices":[{"finish_reason":"stop","message":{"content":"not json"}}]}"#
                        .to_string(),
                request_used_response_format: false,
                request_used_thinking_field: Some("disabled".to_string()),
                response_summary: "provider response summary: has_choices=true".to_string(),
                trace_id: None,
                pending_trace: None,
            }),
        };

        let result =
            debug_ai_classification_once_for_db(&db, "file-1", &provider).expect("debug result");

        assert!(!result.success);
        assert_eq!(result.message_content_preview, "not json");
        assert_eq!(file_status(&db, "file-1"), "unclassified");
    }

    #[test]
    fn debug_target_finds_file_by_id() {
        let db = test_db();
        insert_test_file(&db, "file-1", "F:/work/demo.docx");
        save_ai_settings_for_db(&db, &enabled_settings()).expect("save ai settings");

        let result =
            debug_ai_classification_once_for_db(&db, "file-1", &ok_provider()).expect("debug");

        assert!(result.message_content_preview.contains("file-1"));
    }

    #[test]
    fn debug_target_finds_file_by_full_path() {
        let db = test_db();
        insert_test_file(&db, "file-1", "F:/work/MySQL教案/教案01.docx");
        save_ai_settings_for_db(&db, &enabled_settings()).expect("save ai settings");

        let result = debug_ai_classification_once_for_db(
            &db,
            "F:/work/MySQL教案/教案01.docx",
            &ok_provider(),
        )
        .expect("debug");

        assert!(result.message_content_preview.contains("file-1"));
    }

    #[test]
    fn debug_target_matches_windows_backslash_path() {
        let db = test_db();
        insert_test_file(&db, "file-1", "F:/work/MySQL教案/教案01.docx");
        save_ai_settings_for_db(&db, &enabled_settings()).expect("save ai settings");

        let result = debug_ai_classification_once_for_db(
            &db,
            r"F:\work\MySQL教案\教案01.docx",
            &ok_provider(),
        )
        .expect("debug");

        assert!(result.message_content_preview.contains("file-1"));
    }

    #[test]
    fn debug_target_matches_quoted_path() {
        let db = test_db();
        insert_test_file(&db, "file-1", "F:/work/MySQL教案/教案01.docx");
        save_ai_settings_for_db(&db, &enabled_settings()).expect("save ai settings");

        let result = debug_ai_classification_once_for_db(
            &db,
            "\"F:\\work\\MySQL教案\\教案01.docx\"",
            &ok_provider(),
        )
        .expect("debug");

        assert!(result.message_content_preview.contains("file-1"));
    }

    #[test]
    fn debug_target_does_not_match_stale_file() {
        let db = test_db();
        insert_test_file(&db, "file-1", "F:/work/stale.docx");
        mark_stale(&db, "file-1");
        save_ai_settings_for_db(&db, &enabled_settings()).expect("save ai settings");

        let error = debug_ai_classification_once_for_db(&db, "file-1", &ok_provider())
            .expect_err("stale file should not match");

        assert!(error.contains("No indexed non-stale file matched"));
    }

    #[test]
    fn debug_target_not_found_mentions_scan_or_library() {
        let db = test_db();
        save_ai_settings_for_db(&db, &enabled_settings()).expect("save ai settings");

        let error = debug_ai_classification_once_for_db(&db, "F:/missing.docx", &ok_provider())
            .expect_err("missing file should not match");

        assert!(error.contains("scan the folder"));
        assert!(error.contains("choose a file from the library"));
    }

    #[test]
    fn debug_target_reports_multiple_normalized_matches() {
        let db = test_db();
        insert_test_file(&db, "file-1", "F:/work/demo.docx");
        insert_test_file(&db, "file-2", "f:/work/demo.docx");
        save_ai_settings_for_db(&db, &enabled_settings()).expect("save ai settings");

        let error = debug_ai_classification_once_for_db(&db, r"F:\work\demo.docx", &ok_provider())
            .expect_err("case-insensitive Windows match should be ambiguous");

        assert!(error.contains("Multiple files matched"));
    }

    #[test]
    fn debug_classification_redacts_api_key_from_result() {
        let db = test_db();
        insert_test_file(&db, "file-1", "/tmp/Scala期末复习题.pdf");
        save_ai_settings_for_db(&db, &enabled_settings()).expect("save ai settings");
        let provider = StaticRawProvider {
            response: Ok(AIRawProviderResponse {
                status: 200,
                response_text: r#"{"choices":[{"finish_reason":"stop","message":{"content":"secret-debug-key"}}]}"#.to_string(),
                request_used_response_format: false,
                request_used_thinking_field: Some("disabled".to_string()),
                response_summary: "provider response summary: has_choices=true".to_string(),
                trace_id: None,
                pending_trace: None,
            }),
        };

        let result =
            debug_ai_classification_once_for_db(&db, "file-1", &provider).expect("debug result");

        let serialized = serde_json::to_string(&result).expect("serialize result");
        assert!(!serialized.contains("secret-debug-key"));
        assert!(serialized.contains("[redacted]"));
    }

    #[test]
    fn debug_classification_reports_id_mapping_match() {
        let db = test_db();
        insert_test_file(
            &db,
            "real-file-1",
            "D:/Install_Package/Scala编程基础期末复习题.docx",
        );
        save_ai_settings_for_db(&db, &enabled_settings()).expect("save ai settings");
        let provider = StaticRawProvider {
            response: Ok(AIRawProviderResponse {
                status: 200,
                response_text: r#"{"choices":[{"finish_reason":"stop","message":{"content":"{\"classifications\":[{\"refId\":\"f1\",\"fileType\":\"Document\",\"purpose\":\"Teaching\",\"lifecycle\":\"Active\",\"context\":\"Scala\",\"riskLevel\":\"Normal\",\"suggestedAction\":\"Move\",\"targetTemplate\":\"Teaching/Scala\",\"suggestedName\":\"\",\"confidence\":0.8,\"reason\":\"debug\",\"keywords\":[\"Scala\"],\"requiresConfirmation\":false}]}"}}]}"#.to_string(),
                request_used_response_format: false,
                request_used_thinking_field: Some("disabled".to_string()),
                response_summary: "provider response summary: has_choices=true".to_string(),
                trace_id: None,
                pending_trace: None,
            }),
        };

        let result = debug_ai_classification_once_for_db(&db, "real-file-1", &provider)
            .expect("debug result");

        assert_eq!(result.ref_id, "f1");
        assert_eq!(result.real_file_id, "real-file-1");
        assert_eq!(
            result.path,
            "D:/Install_Package/Scala编程基础期末复习题.docx"
        );
        assert_eq!(result.model_returned_ref_id.as_deref(), Some("f1"));
        assert_eq!(result.model_returned_id, None);
        assert!(result.id_mapping_matched);
    }

    struct StaticRawProvider {
        response: Result<AIRawProviderResponse, String>,
    }

    impl AIDebugRawProvider for StaticRawProvider {
        fn send_raw(&self, _request: AIChatRequest) -> Result<AIRawProviderResponse, String> {
            self.response.clone()
        }
    }

    fn enabled_settings() -> AISettings {
        AISettings {
            enabled: true,
            api_key: "secret-debug-key".to_string(),
            preset: AIProviderPresetId::DeepSeek,
            batch_size: 20,
            ..AISettings::default()
        }
    }

    fn test_db() -> Database {
        Database::open(test_db_path()).expect("open test database")
    }

    fn insert_test_file(db: &Database, id: &str, path: &str) {
        db.insert_file(InsertFileRequest {
            id: id.to_string(),
            path: path.to_string(),
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            extension: path.rsplit('.').next().unwrap_or_default().to_string(),
            size: 2048,
            mtime: 1_700_000_000,
            ctime: 1_700_000_000,
            is_dir: false,
            state_code: 0,
        })
        .expect("insert file");
    }

    fn ok_provider() -> StaticRawProvider {
        StaticRawProvider {
            response: Ok(AIRawProviderResponse {
                status: 200,
                response_text:
                    r#"{"choices":[{"finish_reason":"stop","message":{"content":"{\"classifications\":[{\"id\":\"file-1\",\"fileType\":\"Document\",\"purpose\":\"Work\",\"lifecycle\":\"Active\",\"riskLevel\":\"Normal\",\"suggestedAction\":\"Review\",\"confidence\":0.8,\"reason\":\"debug\"}]}"}}]}"#
                        .to_string(),
                request_used_response_format: false,
                request_used_thinking_field: Some("disabled".to_string()),
                response_summary: "provider response summary: has_choices=true".to_string(),
                trace_id: None,
                pending_trace: None,
            }),
        }
    }

    fn mark_stale(db: &Database, id: &str) {
        let conn = Connection::open(db.path()).expect("open db");
        conn.execute("UPDATE files SET is_stale = 1 WHERE id = ?1", params![id])
            .expect("mark stale");
    }

    fn file_status(db: &Database, id: &str) -> String {
        let conn = Connection::open(db.path()).expect("open db");
        conn.query_row(
            "SELECT classification_status FROM files WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("file status")
    }

    fn test_db_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        test_db_path_with_nonce(nonce)
    }

    fn test_db_path_with_nonce(nonce: u128) -> PathBuf {
        let counter = TEST_DB_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "zen-canvas-ai-debug-test-{}-{nonce}-{counter}.sqlite3",
            std::process::id()
        ))
    }

    #[test]
    fn test_database_paths_are_unique_for_parallel_same_clock_allocations() {
        let paths = (0..64)
            .map(|_| std::thread::spawn(|| test_db_path_with_nonce(1)))
            .map(|handle| handle.join().expect("join path generator"))
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(paths.len(), 64);
    }
}
