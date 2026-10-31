use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use fylura_tauri::ai::{
    debug::debug_preview,
    ollama::OllamaProvider,
    openai_compatible::{
        debug_extract_openai_response, join_base_url_and_chat_path, OpenAICompatibleProvider,
    },
    presets::{all_provider_presets, provider_preset, AIExtraBodyStrategy},
    provider::AIProvider,
    schema::{
        AIChatMessage, AIChatRequest, AICustomProviderProfile, AIProviderKind, AIProviderOptions,
        AIProviderPresetId,
    },
    settings::test_ai_provider_connection_for_settings,
    settings::{
        get_ai_settings_with_store, list_ai_models_for_settings, normalize_ai_settings,
        save_ai_settings_with_store, AISettings, ApiKeyAction, CredentialStore,
        InMemoryCredentialStore, AI_SETTINGS_KEY,
    },
};
use fylura_tauri::{db::Database, settings::get_app_settings};
use serde_json::Value;

#[derive(Default)]
struct ReadbackMismatchStore {
    value: Mutex<Option<String>>,
    reads: AtomicUsize,
}

impl ReadbackMismatchStore {
    fn stored(&self) -> Option<String> {
        self.value.lock().expect("credential mutex").clone()
    }
}

impl CredentialStore for ReadbackMismatchStore {
    fn set(&self, value: &str) -> Result<(), String> {
        *self
            .value
            .lock()
            .map_err(|_| "credential mutex".to_string())? = Some(value.to_string());
        Ok(())
    }

    fn get(&self) -> Result<Option<String>, String> {
        if self.reads.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(self.stored())
        } else {
            Ok(Some("mismatched-readback".to_string()))
        }
    }

    fn delete(&self) -> Result<(), String> {
        *self
            .value
            .lock()
            .map_err(|_| "credential mutex".to_string())? = None;
        Ok(())
    }
}

struct FailingReadStore;

impl CredentialStore for FailingReadStore {
    fn set(&self, _value: &str) -> Result<(), String> {
        Ok(())
    }

    fn get(&self) -> Result<Option<String>, String> {
        Err("credential backend unavailable".to_string())
    }

    fn delete(&self) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn deepseek_preset_uses_openai_compatible_json_mode_defaults() {
    let preset = provider_preset(AIProviderPresetId::DeepSeek).expect("deepseek preset");

    assert_eq!(preset.id, AIProviderPresetId::DeepSeek);
    assert_eq!(preset.provider_kind, AIProviderKind::OpenAICompatible);
    assert_eq!(preset.default_base_url, "https://api.deepseek.com");
    assert_eq!(preset.default_chat_path, "/chat/completions");
    assert_eq!(preset.default_model, "deepseek-v4-flash");
    assert!(preset.supports_thinking);
    assert!(preset.supports_reasoning_effort);
    assert!(preset.supports_response_format);
    assert_eq!(
        preset.extra_body_strategy,
        AIExtraBodyStrategy::DeepSeekThinking
    );
}

#[test]
fn ai_settings_default_starts_disabled_on_deepseek() {
    let settings = AISettings::default();

    assert!(!settings.enabled);
    assert_eq!(settings.provider, AIProviderKind::OpenAICompatible);
    assert_eq!(settings.preset, AIProviderPresetId::DeepSeek);
    assert_eq!(settings.base_url, "https://api.deepseek.com");
    assert_eq!(settings.chat_path, "/chat/completions");
    assert_eq!(settings.model, "deepseek-v4-flash");
    assert_eq!(settings.batch_size, 10);
    assert_eq!(settings.classification_concurrency, 2);
    assert_eq!(settings.timeout_seconds, 120);
    assert!(!settings.send_full_path);
    assert!(settings.send_parent_path);
    assert!(settings.cleanup_ai_enabled);
    assert!(settings.force_json_output);
    assert!(!settings.enable_thinking);
}

#[test]
fn ai_settings_roundtrip_uses_separate_settings_row_and_normalizes_paths() {
    let db = Database::open(test_db_path()).expect("open test database");
    let credentials = InMemoryCredentialStore::default();
    let app_settings_before = get_app_settings(&db).expect("app settings before");
    let mut settings = AISettings {
        enabled: true,
        preset: AIProviderPresetId::Kimi,
        provider: AIProviderKind::OpenAICompatible,
        base_url: " https://api.moonshot.cn/v1/ ".to_string(),
        chat_path: " chat/completions ".to_string(),
        api_key: " moonshot-secret ".to_string(),
        api_key_action: fylura_tauri::ai::settings::ApiKeyAction::Replace,
        model: " kimi-k2.6 ".to_string(),
        timeout_seconds: 1,
        batch_size: 1,
        ..AISettings::default()
    };

    let saved =
        save_ai_settings_with_store(&db, &settings, &credentials).expect("save ai settings");
    settings.base_url = "https://api.moonshot.cn/v1".to_string();
    settings.chat_path = "/chat/completions".to_string();
    settings.api_key = "moonshot-secret".to_string();
    settings.model = "kimi-k2.6".to_string();
    settings.timeout_seconds = 1;
    settings.batch_size = 1;
    let loaded = get_ai_settings_with_store(&db, &credentials).expect("load ai settings");
    let app_settings_after = get_app_settings(&db).expect("app settings after");

    assert_eq!(saved.base_url, settings.base_url);
    assert_eq!(saved.chat_path, settings.chat_path);
    assert_eq!(loaded.model, settings.model);
    assert_eq!(loaded.api_key, "moonshot-secret");
    assert_eq!(
        app_settings_before.close_behavior,
        app_settings_after.close_behavior
    );
    assert_eq!(
        app_settings_before.folder_naming_language,
        app_settings_after.folder_naming_language
    );
}

#[test]
fn missing_ai_settings_loads_deepseek_defaults_and_presets_list_has_all_ids() {
    let db = Database::open(test_db_path()).expect("open test database");
    let credentials = InMemoryCredentialStore::default();

    let settings = get_ai_settings_with_store(&db, &credentials).expect("default ai settings");
    let ids = all_provider_presets()
        .into_iter()
        .map(|preset| preset.id)
        .collect::<Vec<_>>();

    assert_eq!(settings.preset, AIProviderPresetId::DeepSeek);
    assert_eq!(settings.provider, AIProviderKind::OpenAICompatible);
    assert_eq!(settings.base_url, "https://api.deepseek.com");
    assert_eq!(ids.len(), 14);
    assert!(ids.contains(&AIProviderPresetId::DeepSeek));
    assert!(ids.contains(&AIProviderPresetId::Ollama));
    assert!(ids.contains(&AIProviderPresetId::CustomOpenAICompatible));
}

#[test]
fn ai_api_key_actions_are_explicit_and_sqlite_never_stores_plaintext() {
    let db = Database::open(test_db_path()).expect("open test database");
    let credentials = InMemoryCredentialStore::default();
    credentials.set("existing-secret").expect("seed key");

    let preserved = save_ai_settings_with_store(
        &db,
        &AISettings {
            api_key: "ignored-value".to_string(),
            api_key_action: ApiKeyAction::Preserve,
            ..AISettings::default()
        },
        &credentials,
    )
    .expect("preserve key");
    assert_eq!(preserved.api_key, "existing-secret");

    let replaced = save_ai_settings_with_store(
        &db,
        &AISettings {
            api_key: "replacement-secret".to_string(),
            api_key_action: ApiKeyAction::Replace,
            ..AISettings::default()
        },
        &credentials,
    )
    .expect("replace key");
    assert_eq!(replaced.api_key, "replacement-secret");
    assert_eq!(
        credentials.get().unwrap().as_deref(),
        Some("replacement-secret")
    );

    let cleared = save_ai_settings_with_store(
        &db,
        &AISettings {
            api_key_action: ApiKeyAction::Clear,
            ..AISettings::default()
        },
        &credentials,
    )
    .expect("clear key");
    assert!(cleared.api_key.is_empty());
    assert_eq!(credentials.get().unwrap(), None);

    let conn = rusqlite::Connection::open(db.path()).expect("open sqlite");
    let persisted: String = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            rusqlite::params![AI_SETTINGS_KEY],
            |row| row.get(0),
        )
        .expect("persisted AI settings");
    assert!(!persisted.contains("existing-secret"));
    assert!(!persisted.contains("replacement-secret"));
}

#[test]
fn custom_profiles_keep_independent_credentials_and_never_persist_them() {
    let db = Database::open(test_db_path()).expect("open test database");
    let credentials = InMemoryCredentialStore::default();
    let profiles = vec![
        custom_profile("profile-a", "http://127.0.0.1:1"),
        custom_profile("profile-b", "http://127.0.0.1:2"),
    ];
    let first = AISettings {
        preset: AIProviderPresetId::CustomOpenAICompatible,
        provider: AIProviderKind::OpenAICompatible,
        base_url: "http://127.0.0.1:1".to_string(),
        chat_path: "/chat/completions".to_string(),
        model: "model-a".to_string(),
        custom_profiles: profiles.clone(),
        active_custom_profile_id: Some("profile-a".to_string()),
        api_key: "profile-a-secret".to_string(),
        api_key_action: ApiKeyAction::Replace,
        ..AISettings::default()
    };
    save_ai_settings_with_store(&db, &first, &credentials).expect("save profile A");

    let second = AISettings {
        preset: AIProviderPresetId::CustomOpenAICompatible,
        provider: AIProviderKind::OpenAICompatible,
        base_url: "http://127.0.0.1:2".to_string(),
        chat_path: "/chat/completions".to_string(),
        model: "model-b".to_string(),
        custom_profiles: profiles,
        active_custom_profile_id: Some("profile-b".to_string()),
        api_key: "profile-b-secret".to_string(),
        api_key_action: ApiKeyAction::Replace,
        ..AISettings::default()
    };
    save_ai_settings_with_store(&db, &second, &credentials).expect("save profile B");

    assert_eq!(
        credentials.get_profile("profile-a").unwrap().as_deref(),
        Some("profile-a-secret")
    );
    assert_eq!(
        credentials.get_profile("profile-b").unwrap().as_deref(),
        Some("profile-b-secret")
    );
    let loaded = get_ai_settings_with_store(&db, &credentials).expect("load profile settings");
    assert_eq!(
        loaded.active_custom_profile_id.as_deref(),
        Some("profile-b")
    );
    assert_eq!(loaded.api_key, "profile-b-secret");
    assert!(loaded
        .custom_profiles
        .iter()
        .all(|profile| profile.api_key_configured));

    let conn = rusqlite::Connection::open(db.path()).expect("open sqlite");
    let persisted: String = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            rusqlite::params![AI_SETTINGS_KEY],
            |row| row.get(0),
        )
        .expect("persisted AI settings");
    assert!(!persisted.contains("profile-a-secret"));
    assert!(!persisted.contains("profile-b-secret"));
}

#[test]
fn credential_replace_requires_matching_readback_and_rolls_back() {
    let db = Database::open(test_db_path()).expect("open test database");
    let credentials = ReadbackMismatchStore::default();
    credentials.set("existing-secret").expect("seed key");

    let error = save_ai_settings_with_store(
        &db,
        &AISettings {
            api_key: "replacement-secret".to_string(),
            api_key_action: ApiKeyAction::Replace,
            ..AISettings::default()
        },
        &credentials,
    )
    .expect_err("mismatched read-back must fail");

    assert!(error.to_string().contains("read-back verification failed"));
    assert_eq!(credentials.stored().as_deref(), Some("existing-secret"));
}

#[test]
fn credential_read_errors_are_not_reported_as_not_configured() {
    let db = Database::open(test_db_path()).expect("open test database");
    let error = get_ai_settings_with_store(&db, &FailingReadStore)
        .expect_err("credential backend error must propagate");
    assert!(error.to_string().contains("credential backend unavailable"));
}

#[test]
fn credential_replace_rolls_back_when_database_save_fails() {
    let db = Database::open(test_db_path()).expect("open test database");
    let credentials = InMemoryCredentialStore::default();
    credentials.set("existing-secret").expect("seed key");
    let conn = rusqlite::Connection::open(db.path()).expect("open sqlite");
    conn.execute("DROP TABLE app_settings", [])
        .expect("force settings persistence failure");

    let error = save_ai_settings_with_store(
        &db,
        &AISettings {
            api_key: "replacement-secret".to_string(),
            api_key_action: ApiKeyAction::Replace,
            ..AISettings::default()
        },
        &credentials,
    )
    .expect_err("database failure must abort credential replacement");

    assert!(error.to_string().contains("sqlite"));
    assert_eq!(
        credentials.get().unwrap().as_deref(),
        Some("existing-secret")
    );
}

#[test]
fn normalize_ai_settings_reconciles_preset_provider_and_chat_path() {
    let normalized = normalize_ai_settings(AISettings {
        preset: AIProviderPresetId::Ollama,
        provider: AIProviderKind::OpenAICompatible,
        base_url: " http://localhost:11434/ ".to_string(),
        chat_path: "api/chat".to_string(),
        model: " qwen3:8b ".to_string(),
        api_key: " local-key ".to_string(),
        ..AISettings::default()
    });

    assert_eq!(normalized.provider, AIProviderKind::Ollama);
    assert_eq!(normalized.base_url, "http://localhost:11434");
    assert_eq!(normalized.chat_path, "/api/chat");
    assert_eq!(normalized.model, "qwen3:8b");
    assert_eq!(normalized.api_key, "local-key");
}

#[test]
fn test_ai_provider_connection_for_settings_uses_short_json_probe_and_reports_elapsed() {
    let server = TestServer::start(
        200,
        r#"{"choices":[{"message":{"content":"{\"ok\":true}"}}]}"#,
    );
    let mut settings = settings_for_server(
        &server.base_url,
        AIProviderPresetId::DeepSeek,
        "secret-openai-key",
    );
    settings.max_tokens = 8192;

    let result = test_ai_provider_connection_for_settings(settings).expect("connection test");
    let request = server.request();
    let body: Value = serde_json::from_str(&request.body).expect("json body");

    assert!(result.ok);
    assert_eq!(result.provider, Some(AIProviderKind::OpenAICompatible));
    assert_eq!(result.preset, Some(AIProviderPresetId::DeepSeek));
    assert_eq!(result.model.as_deref(), Some("deepseek-v4-flash"));
    assert!(result.elapsed_ms <= 10_000);
    let serialized = serde_json::to_value(&result).expect("serialize connection result");
    assert!(serialized.get("elapsedMs").is_some());
    assert!(serialized.get("elapsed_ms").is_none());
    assert!(body["messages"].to_string().contains(r#"{\"ok\":true}"#));
    assert!(body["messages"]
        .to_string()
        .contains("Return exactly this JSON and nothing else"));
    assert!(body["messages"]
        .to_string()
        .contains("Use non-thinking mode and only return final content"));
    assert_eq!(body["max_tokens"], 4096);
    assert_eq!(body["thinking"]["type"], "disabled");
}

#[test]
fn model_discovery_reads_openai_data_and_merges_registry_suggestions() {
    let server = TestServer::start(
        200,
        r#"{"data":[{"id":"deepseek-v4-flash","owned_by":"deepseek"},{"id":"custom-model"}]}"#,
    );
    let models = list_ai_models_for_settings(settings_for_server(
        &server.base_url,
        AIProviderPresetId::DeepSeek,
        "secret-openai-key",
    ))
    .expect("model discovery");

    assert!(models.iter().any(|model| {
        model.id == "custom-model" && model.discovered && model.owned_by.is_none()
    }));
    assert!(models
        .iter()
        .any(|model| model.id == "deepseek-v4-flash" && model.discovered));
    assert!(models
        .iter()
        .any(|model| model.id == "deepseek-v4-pro" && !model.discovered));
    assert_eq!(server.request().path, "/models");
}

#[test]
fn model_discovery_redacts_authentication_failures() {
    let server = TestServer::start(429, r#"{"error":"secret-openai-key"}"#);
    let error = list_ai_models_for_settings(settings_for_server(
        &server.base_url,
        AIProviderPresetId::DeepSeek,
        "secret-openai-key",
    ))
    .expect_err("model discovery should fail");

    assert!(error.contains("HTTP 429"));
    assert!(!error.contains("secret-openai-key"));
    assert_eq!(server.request().path, "/models");
}

#[test]
fn openai_compatible_test_connection_clamps_small_max_tokens_to_512() {
    let server = TestServer::start(
        200,
        r#"{"choices":[{"message":{"content":"{\"ok\":true}"}}]}"#,
    );
    let mut settings = settings_for_server(
        &server.base_url,
        AIProviderPresetId::DeepSeek,
        "secret-openai-key",
    );
    settings.max_tokens = 128;
    let provider = OpenAICompatibleProvider::new(settings);

    provider.test_connection().expect("connection test");

    let body: Value = serde_json::from_str(&server.request().body).expect("json body");
    assert_eq!(body["max_tokens"], 512);
}

#[test]
fn openai_raw_response_extractor_reads_message_content() {
    let extracted = debug_extract_openai_response(
        r#"{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"{\"classifications\":[]}"}}]}"#,
    );

    assert_eq!(extracted.finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        extracted.message_content.as_deref(),
        Some(r#"{"classifications":[]}"#)
    );
    assert_eq!(
        extracted.extracted_content.as_deref(),
        Some(r#"{"classifications":[]}"#)
    );
    assert!(extracted.message_keys.contains(&"content".to_string()));
}

#[test]
fn openai_raw_response_extractor_reads_reasoning_content() {
    let extracted = debug_extract_openai_response(
        r#"{"choices":[{"finish_reason":"length","message":{"role":"assistant","content":"","reasoning_content":"thinking first"}}]}"#,
    );

    assert_eq!(extracted.finish_reason.as_deref(), Some("length"));
    assert_eq!(extracted.message_content.as_deref(), Some(""));
    assert_eq!(
        extracted.reasoning_content.as_deref(),
        Some("thinking first")
    );
    assert_eq!(extracted.extracted_content.as_deref(), Some(""));
}

#[test]
fn debug_preview_truncates_raw_response() {
    let preview = debug_preview(&"x".repeat(3_500), 3_000);

    assert_eq!(preview.chars().count(), 3_000);
    assert!(preview.ends_with("..."));
}

#[test]
fn ai_schema_serializes_expected_provider_and_preset_ids() {
    assert_eq!(
        serde_json::to_value(AIProviderKind::OpenAICompatible).unwrap(),
        "openai_compatible"
    );
    assert_eq!(
        serde_json::to_value(AIProviderPresetId::QwenDashScope).unwrap(),
        "qwen_dashscope"
    );
    assert_eq!(
        serde_json::to_value(AIProviderPresetId::CustomOpenAICompatible).unwrap(),
        "custom_openai_compatible"
    );
}

#[test]
fn openai_compatible_url_join_accepts_optional_slashes() {
    assert_eq!(
        join_base_url_and_chat_path("https://api.example.com/", "/chat/completions").unwrap(),
        "https://api.example.com/chat/completions"
    );
    assert_eq!(
        join_base_url_and_chat_path("https://api.example.com/v1", "chat/completions").unwrap(),
        "https://api.example.com/v1/chat/completions"
    );
}

#[test]
fn openai_compatible_chat_returns_choice_content_and_requests_json_mode() {
    let server = TestServer::start(
        200,
        r#"{"choices":[{"message":{"content":"{\"ok\":true}"}}]}"#,
    );
    let provider = OpenAICompatibleProvider::new(settings_for_server(
        &server.base_url,
        AIProviderPresetId::Kimi,
        "secret-openai-key",
    ));

    let content = provider
        .chat_json(chat_request("kimi-k2.7-code-highspeed", true))
        .expect("chat response");

    assert_eq!(content, r#"{"ok":true}"#);
    let request = server.request();
    assert_eq!(request.path, "/chat/completions");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer secret-openai-key")
    );
    assert_eq!(
        request.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    let body: Value = serde_json::from_str(&request.body).expect("json body");
    assert_eq!(body["model"], "kimi-k2.7-code-highspeed");
    assert_eq!(body["response_format"]["type"], "json_object");
    let messages = body["messages"].as_array().expect("messages");
    assert!(messages.iter().any(|message| message["content"]
        .as_str()
        .unwrap_or_default()
        .contains("Return JSON only")));
}

#[test]
fn openai_compatible_errors_redact_api_key() {
    let server = TestServer::start(401, r#"{"error":"bad secret-openai-key"}"#);
    let provider = OpenAICompatibleProvider::new(settings_for_server(
        &server.base_url,
        AIProviderPresetId::DeepSeek,
        "secret-openai-key",
    ));

    let error = provider
        .chat_json(chat_request("deepseek-v4-flash", true))
        .expect_err("request should fail");
    let message = error.to_string();

    assert!(message.contains("[redacted]"));
    assert!(!message.contains("secret-openai-key"));
}

#[test]
fn deepseek_chat_writes_thinking_enabled_when_requested() {
    let server = TestServer::start(
        200,
        r#"{"choices":[{"message":{"content":"{\"ok\":true}"}}]}"#,
    );
    let mut settings = settings_for_server(
        &server.base_url,
        AIProviderPresetId::DeepSeek,
        "secret-openai-key",
    );
    settings.enable_thinking = true;
    let provider = OpenAICompatibleProvider::new(settings);

    provider
        .chat_json(chat_request("deepseek-v4-flash", true))
        .expect("chat response");

    let body: Value = serde_json::from_str(&server.request().body).expect("json body");
    assert_eq!(body["thinking"]["type"], "enabled");
}

#[test]
fn deepseek_chat_writes_thinking_disabled_when_not_requested() {
    let server = TestServer::start(
        200,
        r#"{"choices":[{"message":{"content":"{\"ok\":true}"}}]}"#,
    );
    let mut settings = settings_for_server(
        &server.base_url,
        AIProviderPresetId::DeepSeek,
        "secret-openai-key",
    );
    settings.enable_thinking = false;
    let provider = OpenAICompatibleProvider::new(settings);

    provider
        .chat_json(chat_request("deepseek-v4-flash", true))
        .expect("chat response");

    let body: Value = serde_json::from_str(&server.request().body).expect("json body");
    assert_eq!(body["thinking"]["type"], "disabled");
}

#[test]
fn openai_raw_chat_returns_request_diagnostics_without_api_key() {
    let server = TestServer::start(
        200,
        r#"{"choices":[{"finish_reason":"stop","message":{"content":"{\"ok\":true}"}}]}"#,
    );
    let provider = OpenAICompatibleProvider::new(settings_for_server(
        &server.base_url,
        AIProviderPresetId::DeepSeek,
        "secret-openai-key",
    ));

    let raw = provider
        .send_chat_request_raw(chat_request("deepseek-v4-flash", true))
        .expect("raw response");

    assert_eq!(raw.status, 200);
    assert!(!raw.response_text.contains("secret-openai-key"));
    assert!(raw.request_used_response_format);
    assert_eq!(raw.request_used_thinking_field.as_deref(), Some("disabled"));
    assert!(raw.response_summary.contains("has_choices=true"));
}

#[test]
fn deepseek_chat_rejects_user_override_of_provider_thinking_body() {
    let server = TestServer::start(
        200,
        r#"{"choices":[{"message":{"content":"{\"ok\":true}"}}]}"#,
    );
    let mut settings = settings_for_server(
        &server.base_url,
        AIProviderPresetId::DeepSeek,
        "secret-openai-key",
    );
    settings.enable_thinking = false;
    settings.extra_body_json = Some(r#"{"thinking":{"type":"custom"}}"#.to_string());
    let provider = OpenAICompatibleProvider::new(settings);

    provider
        .chat_json(chat_request("deepseek-v4-flash", true))
        .expect("chat response");

    let body: Value = serde_json::from_str(&server.request().body).expect("json body");
    assert_eq!(body["thinking"]["type"], "disabled");
}

#[test]
fn reasoning_content_length_error_explains_truncated_final_content_and_redacts_key() {
    let server = TestServer::start(
        200,
        r#"{"choices":[{"finish_reason":"length","message":{"role":"assistant","content":"","reasoning_content":"thinking with secret-openai-key"}}]}"#,
    );
    let provider = OpenAICompatibleProvider::new(settings_for_server(
        &server.base_url,
        AIProviderPresetId::DeepSeek,
        "secret-openai-key",
    ));

    let error = provider
        .chat_json(chat_request("deepseek-v4-flash", true))
        .expect_err("reasoning-only response should fail")
        .to_string();

    assert!(error.contains("finish_reason=length"));
    assert!(error.contains("reasoning_content"));
    assert!(error.contains("JSON 被截断"));
    assert!(!error.contains("secret-openai-key"));
}

#[test]
fn openai_compatible_respects_qwen_json_mode_capability() {
    let server = TestServer::start(
        200,
        r#"{"choices":[{"message":{"content":"{\"ok\":true}"}}]}"#,
    );
    let provider = OpenAICompatibleProvider::new(settings_for_server(
        &server.base_url,
        AIProviderPresetId::QwenDashScope,
        "dashscope-key",
    ));

    provider
        .chat_json(chat_request("qwen-plus", true))
        .expect("chat response");

    let body: Value = serde_json::from_str(&server.request().body).expect("json body");
    assert_eq!(body["response_format"]["type"], "json_object");
    let messages = body["messages"].as_array().expect("messages");
    assert!(messages.iter().any(|message| message["content"]
        .as_str()
        .unwrap_or_default()
        .contains("Return JSON only")));
}

#[test]
fn ollama_chat_uses_api_chat_stream_false_and_json_format() {
    let server = TestServer::start(200, r#"{"message":{"content":"{\"ok\":true}"}}"#);
    let settings = AISettings {
        provider: AIProviderKind::Ollama,
        preset: AIProviderPresetId::Ollama,
        base_url: server.base_url.clone(),
        chat_path: "/api/chat".to_string(),
        model: "qwen3:8b".to_string(),
        ..AISettings::default()
    };
    let provider = OllamaProvider::new(settings);

    let content = provider
        .chat_json(chat_request("qwen3:8b", true))
        .expect("ollama response");

    assert_eq!(content, r#"{"ok":true}"#);
    let request = server.request();
    assert_eq!(request.path, "/api/chat");
    let body: Value = serde_json::from_str(&request.body).expect("json body");
    assert_eq!(body["model"], "qwen3:8b");
    assert_eq!(body["stream"], false);
    assert_eq!(body["format"], "json");
    let messages = body["messages"].as_array().expect("messages");
    assert!(messages.iter().any(|message| message["content"]
        .as_str()
        .unwrap_or_default()
        .contains("no thinking")));
}

#[test]
fn ollama_test_connection_clamps_small_max_tokens_to_512() {
    let server = TestServer::start(200, r#"{"message":{"content":"{\"ok\":true}"}}"#);
    let settings = AISettings {
        provider: AIProviderKind::Ollama,
        preset: AIProviderPresetId::Ollama,
        base_url: server.base_url.clone(),
        chat_path: "/api/chat".to_string(),
        model: "qwen3:8b".to_string(),
        max_tokens: 128,
        ..AISettings::default()
    };
    let provider = OllamaProvider::new(settings);

    provider.test_connection().expect("ollama connection");

    let body: Value = serde_json::from_str(&server.request().body).expect("json body");
    assert_eq!(body["options"]["num_predict"], 512);
}

fn settings_for_server(base_url: &str, preset_id: AIProviderPresetId, api_key: &str) -> AISettings {
    let preset = provider_preset(preset_id).expect("preset");
    AISettings {
        provider: preset.provider_kind,
        preset: preset.id,
        base_url: base_url.to_string(),
        chat_path: preset.default_chat_path.to_string(),
        api_key: api_key.to_string(),
        model: preset.default_model.to_string(),
        timeout_seconds: 5,
        ..AISettings::default()
    }
}

fn custom_profile(id: &str, base_url: &str) -> AICustomProviderProfile {
    AICustomProviderProfile {
        id: id.to_string(),
        name: id.to_string(),
        base_url: base_url.to_string(),
        chat_path: "/chat/completions".to_string(),
        models_path: Some("/models".to_string()),
        model: format!("{id}-model"),
        supports_response_format: true,
        supports_thinking: false,
        thinking_parameter: "none".to_string(),
        token_parameter: "max_tokens".to_string(),
        content_path: "choices[0].message.content".to_string(),
        reasoning_path: "choices[0].message.reasoning_content".to_string(),
        temperature_min: 0.0,
        temperature_max: 2.0,
        max_output_tokens: 8192,
        extra_body_json: None,
        api_key_configured: false,
    }
}

fn chat_request(model: &str, force_json: bool) -> AIChatRequest {
    AIChatRequest {
        messages: vec![AIChatMessage {
            role: "user".to_string(),
            content: "Return {\"ok\":true}".to_string(),
        }],
        model: model.to_string(),
        temperature: 0.1,
        max_tokens: 128,
        force_json,
        provider_options: AIProviderOptions::default(),
    }
}

struct TestServer {
    base_url: String,
    handle: thread::JoinHandle<CapturedRequest>,
}

impl TestServer {
    fn start(status: u16, response_body: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let port = listener.local_addr().expect("server addr").port();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let request = read_http_request(&mut stream);
            let status_text = if status == 200 { "OK" } else { "ERROR" };
            let response = format!(
                "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
            request
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            handle,
        }
    }

    fn request(self) -> CapturedRequest {
        self.handle
            .join()
            .expect("test server thread should finish")
    }
}

struct CapturedRequest {
    path: String,
    headers: std::collections::HashMap<String, String>,
    body: String,
}

fn read_http_request(stream: &mut TcpStream) -> CapturedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");
    let mut buffer = Vec::new();
    let mut temp = [0_u8; 4096];
    let mut header_end = None;
    while header_end.is_none() {
        let read = stream.read(&mut temp).expect("read headers");
        assert!(read > 0, "client closed before headers");
        buffer.extend_from_slice(&temp[..read]);
        header_end = find_header_end(&buffer);
    }
    let header_end = header_end.expect("headers complete");
    let headers_text = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = headers_text.split("\r\n");
    let request_line = lines.next().expect("request line");
    let path = request_line
        .split_whitespace()
        .nth(1)
        .expect("path")
        .to_string();
    let mut headers = std::collections::HashMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    while buffer.len() < header_end + 4 + content_length {
        let read = stream.read(&mut temp).expect("read body");
        assert!(read > 0, "client closed before body");
        buffer.extend_from_slice(&temp[..read]);
    }
    let body_start = header_end + 4;
    let body =
        String::from_utf8_lossy(&buffer[body_start..body_start + content_length]).to_string();

    CapturedRequest {
        path,
        headers,
        body,
    }
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn test_db_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("zen-canvas-ai-provider-test-{nonce}.sqlite3"))
}
