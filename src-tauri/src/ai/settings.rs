use std::{collections::HashMap, sync::Mutex, time::Instant};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{Runtime, State, WebviewWindow};

use super::{
    ollama::OllamaProvider,
    openai_compatible::OpenAICompatibleProvider,
    presets::{all_provider_presets, provider_preset, AIProviderPreset},
    provider::AIProvider,
    schema::{
        AIConnectionTestResult, AICustomProviderProfile, AIModelInfo, AIProviderKind,
        AIProviderPresetId,
    },
    trace::AITraceMode,
};
use crate::{
    db::{app_setting_value_fingerprint, Database, DbError},
    window_auth::require_main_window,
};

pub const AI_SETTINGS_KEY: &str = "ai_settings_v1";
#[cfg(any(target_os = "windows", target_os = "macos"))]
// Keep this credential-store namespace stable across the product rename so
// existing API keys remain available to the same local user account.
const AI_CREDENTIAL_SERVICE: &str = "com.startlan.zencanvas";
#[cfg(any(target_os = "windows", target_os = "macos"))]
const AI_CREDENTIAL_USER: &str = "ai-api-key";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const SYSTEM_CREDENTIAL_STORE_UNSUPPORTED_PLATFORM: &str =
    "system_credential_store_unsupported_platform";
static AI_SETTINGS_SAVE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyAction {
    #[default]
    Preserve,
    Replace,
    Clear,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AISettings {
    pub enabled: bool,
    pub provider: AIProviderKind,
    pub preset: AIProviderPresetId,
    pub base_url: String,
    pub chat_path: String,
    #[serde(default)]
    pub models_path: Option<String>,
    pub api_key: String,
    #[serde(default, skip_serializing)]
    pub api_key_action: ApiKeyAction,
    #[serde(default)]
    pub api_key_configured: bool,
    pub model: String,
    pub temperature: f32,
    pub max_tokens: u32,
    pub batch_size: usize,
    pub classification_concurrency: usize,
    pub timeout_seconds: u64,
    pub send_full_path: bool,
    pub send_parent_path: bool,
    pub classification_mode: String,
    pub cleanup_ai_enabled: bool,
    pub cleanup_local_ai_allowed: bool,
    pub cleanup_cloud_ai_allowed: bool,
    pub force_json_output: bool,
    pub enable_thinking: bool,
    pub reasoning_effort: Option<String>,
    pub extra_body_json: Option<String>,
    #[serde(default)]
    pub diagnostics_mode: AITraceMode,
    #[serde(default)]
    pub include_sensitive_document_content_in_diagnostics: bool,
    #[serde(default)]
    pub custom_profiles: Vec<AICustomProviderProfile>,
    #[serde(default)]
    pub active_custom_profile_id: Option<String>,
}

impl Default for AISettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: AIProviderKind::OpenAICompatible,
            preset: AIProviderPresetId::DeepSeek,
            base_url: "https://api.deepseek.com".to_string(),
            chat_path: "/chat/completions".to_string(),
            models_path: Some("/models".to_string()),
            api_key: String::new(),
            api_key_action: ApiKeyAction::Preserve,
            api_key_configured: false,
            model: "deepseek-v4-flash".to_string(),
            temperature: 0.0,
            max_tokens: 8192,
            batch_size: 10,
            classification_concurrency: 2,
            timeout_seconds: 120,
            send_full_path: false,
            send_parent_path: true,
            classification_mode: "ai_first".to_string(),
            cleanup_ai_enabled: true,
            cleanup_local_ai_allowed: false,
            cleanup_cloud_ai_allowed: false,
            force_json_output: true,
            enable_thinking: false,
            reasoning_effort: None,
            extra_body_json: None,
            diagnostics_mode: AITraceMode::Off,
            include_sensitive_document_content_in_diagnostics: false,
            custom_profiles: Vec::new(),
            active_custom_profile_id: None,
        }
    }
}

pub fn get_ai_settings_for_db(db: &Database) -> Result<AISettings, DbError> {
    get_ai_settings_with_store(db, &SystemCredentialStore)
}

pub(crate) fn get_ai_settings_for_db_with_revision(
    db: &Database,
) -> Result<(AISettings, String), DbError> {
    get_ai_settings_with_store_and_revision(db, &SystemCredentialStore)
}

pub fn get_ai_settings_with_store(
    db: &Database,
    credentials: &impl CredentialStore,
) -> Result<AISettings, DbError> {
    get_ai_settings_with_store_and_revision(db, credentials).map(|(settings, _)| settings)
}

pub(crate) fn get_ai_settings_with_store_and_revision(
    db: &Database,
    credentials: &impl CredentialStore,
) -> Result<(AISettings, String), DbError> {
    let conn = db.conn()?;
    let mut settings_json = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![AI_SETTINGS_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()?;

    let mut settings = match settings_json.as_deref() {
        Some(value) => serde_json::from_str(value)
            .map(normalize_ai_settings)
            .map_err(DbError::from)?,
        None => AISettings::default(),
    };
    if !settings.api_key.is_empty() {
        credentials
            .set(&settings.api_key)
            .map_err(DbError::Validation)?;
        settings.api_key.clear();
        persist_ai_settings_without_secret(db, &settings)?;
        let conn = db.conn()?;
        settings_json = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                params![AI_SETTINGS_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
    }
    let settings_revision = app_setting_value_fingerprint(settings_json.as_deref());
    settings.api_key = credentials
        .get()
        .map_err(DbError::Validation)?
        .unwrap_or_default();
    for profile in &mut settings.custom_profiles {
        profile.api_key_configured = credentials
            .get_profile(&profile.id)
            .map_err(DbError::Validation)?
            .is_some();
    }
    if let Some(profile_id) = active_profile_id(&settings) {
        settings.api_key = credentials
            .get_profile(profile_id)
            .map_err(DbError::Validation)?
            .unwrap_or_default();
    }
    settings.api_key_action = ApiKeyAction::Preserve;
    settings.api_key_configured = !settings.api_key.is_empty();
    Ok((settings, settings_revision))
}

pub fn save_ai_settings_for_db(
    db: &Database,
    settings: &AISettings,
) -> Result<AISettings, DbError> {
    save_ai_settings_with_store(db, settings, &SystemCredentialStore)
}

pub fn save_ai_settings_with_store(
    db: &Database,
    settings: &AISettings,
    credentials: &impl CredentialStore,
) -> Result<AISettings, DbError> {
    let _transaction_guard = AI_SETTINGS_SAVE_LOCK
        .lock()
        .map_err(|_| DbError::Validation("credential_transaction_lock_poisoned".to_string()))?;
    let previous_profile_ids = stored_custom_profile_ids(db)?;
    let mut normalized = normalize_ai_settings(settings.clone());
    validate_ai_settings(&normalized, !cfg!(debug_assertions)).map_err(DbError::Validation)?;
    let next_profile_ids = normalized
        .custom_profiles
        .iter()
        .map(|profile| profile.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut removed_profile_ids = previous_profile_ids
        .iter()
        .filter(|profile_id| !next_profile_ids.contains(profile_id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    removed_profile_ids.sort();
    let removed_profile_credentials = removed_profile_ids
        .iter()
        .map(|profile_id| {
            Ok(ProfileCredentialSnapshot {
                id: profile_id.clone(),
                value: credentials
                    .get_profile(profile_id)
                    .map_err(DbError::Validation)?,
            })
        })
        .collect::<Result<Vec<_>, DbError>>()?;
    let profile_id = active_profile_id(&normalized).map(ToString::to_string);
    let previous_key = credential_get(credentials, profile_id.as_deref())?;
    // Acquire the database transaction before touching credentials. If the
    // database cannot provide a transaction, no keyring mutation has occurred
    // that would need compensation.
    let mut conn = db.conn()?;
    let transaction = conn.transaction()?;
    let mut credential_changed = false;
    match normalized.api_key_action {
        ApiKeyAction::Preserve => {
            normalized.api_key = previous_key.clone().unwrap_or_default();
        }
        ApiKeyAction::Replace => {
            if normalized.api_key.is_empty() {
                return Err(DbError::Validation(
                    "Replacing the AI API key requires a non-empty value.".to_string(),
                ));
            }
            credential_set(credentials, profile_id.as_deref(), &normalized.api_key)?;
            credential_changed = true;
            verify_credential_change(
                credentials,
                profile_id.as_deref(),
                Some(normalized.api_key.as_str()),
                previous_key.as_deref(),
            )?;
        }
        ApiKeyAction::Clear => {
            credential_delete(credentials, profile_id.as_deref())?;
            credential_changed = true;
            verify_credential_change(
                credentials,
                profile_id.as_deref(),
                None,
                previous_key.as_deref(),
            )?;
            normalized.api_key.clear();
        }
    }
    if normalized.enabled
        && normalized.provider != AIProviderKind::Ollama
        && normalized.api_key.is_empty()
    {
        if credential_changed {
            rollback_credential_change(credentials, profile_id.as_deref(), previous_key.as_deref())
                .map_err(DbError::Validation)?;
        }
        return Err(DbError::Validation(
            "Enabled cloud AI providers require a configured API key.".to_string(),
        ));
    }
    normalized.api_key_action = ApiKeyAction::Preserve;
    normalized.api_key_configured = !normalized.api_key.is_empty();
    let active_profile_id = active_profile_id(&normalized).map(ToString::to_string);
    for profile in &mut normalized.custom_profiles {
        profile.api_key_configured = active_profile_id
            .as_deref()
            .map(|active| active == profile.id)
            .unwrap_or(false)
            && normalized.api_key_configured;
    }
    if let Err(error) = persist_ai_settings_in_transaction(&transaction, &normalized) {
        drop(transaction);
        if credential_changed {
            rollback_credential_change(credentials, profile_id.as_deref(), previous_key.as_deref())
                .map_err(|rollback| {
                    DbError::Validation(format!(
                        "AI settings save failed: {error}; credential rollback failed: {rollback}"
                    ))
                })?;
        }
        return Err(error);
    }
    for snapshot in &removed_profile_credentials {
        if credential_delete(credentials, Some(&snapshot.id)).is_err() {
            drop(transaction);
            let restore_error =
                restore_profile_credentials(credentials, &removed_profile_credentials).err();
            let active_restore_error = if credential_changed {
                rollback_credential_change(
                    credentials,
                    profile_id.as_deref(),
                    previous_key.as_deref(),
                )
                .err()
            } else {
                None
            };
            if restore_error.is_some() || active_restore_error.is_some() {
                return Err(DbError::Validation(
                    "ai_provider_profile_credential_consistency_error".to_string(),
                ));
            }
            return Err(DbError::Validation(
                "AI provider profile deletion failed; database changes were rolled back."
                    .to_string(),
            ));
        }
    }
    if let Err(error) = transaction.commit() {
        let restore_error =
            restore_profile_credentials(credentials, &removed_profile_credentials).err();
        let active_restore_error = if credential_changed {
            rollback_credential_change(credentials, profile_id.as_deref(), previous_key.as_deref())
                .err()
        } else {
            None
        };
        if restore_error.is_some() || active_restore_error.is_some() {
            return Err(DbError::Validation(
                "ai_provider_profile_credential_consistency_error".to_string(),
            ));
        }
        return Err(DbError::from(error));
    }
    db.notify_managed_ai_control_wake();
    Ok(normalized)
}

struct ProfileCredentialSnapshot {
    id: String,
    value: Option<String>,
}

fn restore_profile_credentials(
    credentials: &impl CredentialStore,
    snapshots: &[ProfileCredentialSnapshot],
) -> Result<(), String> {
    for snapshot in snapshots {
        let result = match snapshot.value.as_deref() {
            Some(value) => credential_set(credentials, Some(&snapshot.id), value),
            None => credential_delete(credentials, Some(&snapshot.id)),
        };
        if result.is_err() {
            return Err("profile credential restore failed".to_string());
        }
    }
    Ok(())
}

fn verify_credential_change(
    credentials: &impl CredentialStore,
    profile_id: Option<&str>,
    expected: Option<&str>,
    previous: Option<&str>,
) -> Result<(), DbError> {
    match credential_get(credentials, profile_id) {
        Ok(actual) if actual.as_deref() == expected => Ok(()),
        Ok(_) => {
            rollback_credential_change(credentials, profile_id, previous).map_err(|rollback| {
                DbError::Validation(format!(
                    "credential read-back verification failed; credential rollback failed: {rollback}"
                ))
            })?;
            Err(DbError::Validation(
                "credential read-back verification failed".to_string(),
            ))
        }
        Err(read_error) => {
            rollback_credential_change(credentials, profile_id, previous).map_err(|rollback| {
                DbError::Validation(format!(
                    "credential read-back failed: {read_error}; credential rollback failed: {rollback}"
                ))
            })?;
            Err(DbError::Validation(format!(
                "credential read-back failed: {read_error}"
            )))
        }
    }
}

fn rollback_credential_change(
    credentials: &impl CredentialStore,
    profile_id: Option<&str>,
    previous: Option<&str>,
) -> Result<(), String> {
    match previous {
        Some(value) => {
            credential_set(credentials, profile_id, value).map_err(|error| error.to_string())
        }
        None => credential_delete(credentials, profile_id).map_err(|error| error.to_string()),
    }
}

fn credential_get(
    credentials: &impl CredentialStore,
    profile_id: Option<&str>,
) -> Result<Option<String>, DbError> {
    let value = match profile_id {
        Some(profile_id) => credentials.get_profile(profile_id),
        None => credentials.get(),
    };
    value.map_err(DbError::Validation)
}

fn credential_set(
    credentials: &impl CredentialStore,
    profile_id: Option<&str>,
    value: &str,
) -> Result<(), DbError> {
    let result = match profile_id {
        Some(profile_id) => credentials.set_profile(profile_id, value),
        None => credentials.set(value),
    };
    result.map_err(DbError::Validation)
}

fn credential_delete(
    credentials: &impl CredentialStore,
    profile_id: Option<&str>,
) -> Result<(), DbError> {
    let result = match profile_id {
        Some(profile_id) => credentials.delete_profile(profile_id),
        None => credentials.delete(),
    };
    result.map_err(DbError::Validation)
}

fn active_profile_id(settings: &AISettings) -> Option<&str> {
    if settings.preset != AIProviderPresetId::CustomOpenAICompatible {
        return None;
    }
    settings
        .active_custom_profile_id
        .as_deref()
        .filter(|profile_id| {
            settings
                .custom_profiles
                .iter()
                .any(|profile| profile.id == *profile_id)
        })
}

fn is_valid_profile_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn persist_ai_settings_without_secret(db: &Database, settings: &AISettings) -> Result<(), DbError> {
    let mut conn = db.conn()?;
    let transaction = conn.transaction()?;
    persist_ai_settings_in_transaction(&transaction, settings)?;
    transaction.commit()?;
    Ok(())
}

fn persist_ai_settings_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    settings: &AISettings,
) -> Result<(), DbError> {
    let mut persisted = settings.clone();
    persisted.api_key.clear();
    persisted.api_key_configured = false;
    for profile in &mut persisted.custom_profiles {
        profile.api_key_configured = false;
    }
    let settings_json = serde_json::to_string(&persisted)?;
    transaction.execute(
        r#"
        INSERT INTO app_settings (key, value)
        VALUES (?1, ?2)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value
        "#,
        params![AI_SETTINGS_KEY, settings_json],
    )?;
    // Provider/model/prompt policy changes invalidate provider-derived content
    // facts without deleting the deterministic local extraction. The next
    // explicit, consented understanding run may publish a new current fact.
    transaction.execute(
        "UPDATE content_artifacts
         SET status='stale', revision=revision+1, updated_at=unixepoch()
         WHERE status='current' AND provider_kind IS NOT NULL",
        [],
    )?;
    Ok(())
}

fn stored_custom_profile_ids(db: &Database) -> Result<std::collections::HashSet<String>, DbError> {
    let conn = db.conn()?;
    let value = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![AI_SETTINGS_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(value) = value else {
        return Ok(std::collections::HashSet::new());
    };
    let settings = serde_json::from_str::<AISettings>(&value)?;
    Ok(settings
        .custom_profiles
        .into_iter()
        .map(|profile| profile.id)
        .collect())
}

pub trait CredentialStore {
    fn set(&self, value: &str) -> Result<(), String>;
    fn get(&self) -> Result<Option<String>, String>;
    fn delete(&self) -> Result<(), String>;

    fn set_profile(&self, _profile_id: &str, value: &str) -> Result<(), String> {
        self.set(value)
    }

    fn get_profile(&self, _profile_id: &str) -> Result<Option<String>, String> {
        self.get()
    }

    fn delete_profile(&self, _profile_id: &str) -> Result<(), String> {
        self.delete()
    }
}

pub struct SystemCredentialStore;

#[derive(Default)]
pub struct InMemoryCredentialStore {
    main: Mutex<Option<String>>,
    profiles: Mutex<HashMap<String, String>>,
}

impl CredentialStore for InMemoryCredentialStore {
    fn set(&self, value: &str) -> Result<(), String> {
        *self
            .main
            .lock()
            .map_err(|_| "in-memory credential store is unavailable".to_string())? =
            Some(value.trim().to_string());
        Ok(())
    }

    fn get(&self) -> Result<Option<String>, String> {
        self.main
            .lock()
            .map(|value| value.clone())
            .map_err(|_| "in-memory credential store is unavailable".to_string())
    }

    fn delete(&self) -> Result<(), String> {
        *self
            .main
            .lock()
            .map_err(|_| "in-memory credential store is unavailable".to_string())? = None;
        Ok(())
    }

    fn set_profile(&self, profile_id: &str, value: &str) -> Result<(), String> {
        self.profiles
            .lock()
            .map_err(|_| "in-memory credential store is unavailable".to_string())?
            .insert(profile_id.to_string(), value.trim().to_string());
        Ok(())
    }

    fn get_profile(&self, profile_id: &str) -> Result<Option<String>, String> {
        self.profiles
            .lock()
            .map(|values| values.get(profile_id).cloned())
            .map_err(|_| "in-memory credential store is unavailable".to_string())
    }

    fn delete_profile(&self, profile_id: &str) -> Result<(), String> {
        self.profiles
            .lock()
            .map_err(|_| "in-memory credential store is unavailable".to_string())?
            .remove(profile_id);
        Ok(())
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn credential_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(AI_CREDENTIAL_SERVICE, AI_CREDENTIAL_USER)
        .map_err(|error| format!("failed to open system credential store: {error}"))
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn profile_credential_entry(profile_id: &str) -> Result<keyring::Entry, String> {
    if !is_valid_profile_id(profile_id) {
        return Err("AI custom provider profile ID is invalid.".to_string());
    }
    keyring::Entry::new(
        // Existing profile credentials use this service name; do not orphan
        // them when the Tauri bundle identifier changes.
        "com.startlan.zencanvas.ai-profile",
        &format!("ai-api-key-{profile_id}"),
    )
    .map_err(|error| format!("failed to open custom provider credential store: {error}"))
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
impl CredentialStore for SystemCredentialStore {
    fn set(&self, value: &str) -> Result<(), String> {
        credential_entry()?
            .set_password(value.trim())
            .map_err(|error| format!("failed to save API key in system credential store: {error}"))
    }
    fn get(&self) -> Result<Option<String>, String> {
        match credential_entry()?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(format!(
                "failed to read API key from system credential store: {error}"
            )),
        }
    }
    fn delete(&self) -> Result<(), String> {
        match credential_entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(format!(
                "failed to delete API key from system credential store: {error}"
            )),
        }
    }

    fn set_profile(&self, profile_id: &str, value: &str) -> Result<(), String> {
        profile_credential_entry(profile_id)?
            .set_password(value.trim())
            .map_err(|error| format!("failed to save custom provider API key: {error}"))
    }

    fn get_profile(&self, profile_id: &str) -> Result<Option<String>, String> {
        match profile_credential_entry(profile_id)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(format!("failed to read custom provider API key: {error}")),
        }
    }

    fn delete_profile(&self, profile_id: &str) -> Result<(), String> {
        match profile_credential_entry(profile_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(format!("failed to delete custom provider API key: {error}")),
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
impl CredentialStore for SystemCredentialStore {
    fn set(&self, _value: &str) -> Result<(), String> {
        Err(SYSTEM_CREDENTIAL_STORE_UNSUPPORTED_PLATFORM.to_string())
    }

    fn get(&self) -> Result<Option<String>, String> {
        Err(SYSTEM_CREDENTIAL_STORE_UNSUPPORTED_PLATFORM.to_string())
    }

    fn delete(&self) -> Result<(), String> {
        Err(SYSTEM_CREDENTIAL_STORE_UNSUPPORTED_PLATFORM.to_string())
    }

    fn set_profile(&self, _profile_id: &str, _value: &str) -> Result<(), String> {
        Err(SYSTEM_CREDENTIAL_STORE_UNSUPPORTED_PLATFORM.to_string())
    }

    fn get_profile(&self, _profile_id: &str) -> Result<Option<String>, String> {
        Err(SYSTEM_CREDENTIAL_STORE_UNSUPPORTED_PLATFORM.to_string())
    }

    fn delete_profile(&self, _profile_id: &str) -> Result<(), String> {
        Err(SYSTEM_CREDENTIAL_STORE_UNSUPPORTED_PLATFORM.to_string())
    }
}

fn public_ai_settings(mut settings: AISettings) -> AISettings {
    settings.api_key_configured = !settings.api_key.is_empty();
    settings.api_key.clear();
    settings.api_key_action = ApiKeyAction::Preserve;
    settings
}

pub fn normalize_ai_settings(mut settings: AISettings) -> AISettings {
    if settings.preset == AIProviderPresetId::CustomOpenAICompatible {
        if let Some(profile) = settings
            .active_custom_profile_id
            .as_deref()
            .and_then(|profile_id| {
                settings
                    .custom_profiles
                    .iter()
                    .find(|profile| profile.id == profile_id)
            })
            .cloned()
        {
            settings.base_url = profile.base_url;
            settings.chat_path = profile.chat_path;
            settings.models_path = profile.models_path;
            settings.model = profile.model;
            settings.force_json_output = profile.supports_response_format;
            settings.enable_thinking = profile.supports_thinking;
            settings.max_tokens = profile.max_output_tokens;
            settings.extra_body_json = profile.extra_body_json;
        }
    }
    if let Some(preset) = provider_preset(settings.preset) {
        settings.provider = preset.provider_kind;
        if settings.base_url.trim().is_empty() && !preset.default_base_url.is_empty() {
            settings.base_url = preset.default_base_url.to_string();
        }
        if settings.chat_path.trim().is_empty() {
            settings.chat_path = preset.default_chat_path.to_string();
        }
        if settings
            .models_path
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            settings.models_path = preset.models_path.map(ToString::to_string);
        }
        if settings.model.trim().is_empty() && !preset.default_model.is_empty() {
            settings.model = preset.default_model.to_string();
        }
        if settings.preset == AIProviderPresetId::Ollama {
            settings.chat_path = "/api/chat".to_string();
        }
    }

    settings.base_url = settings.base_url.trim().trim_end_matches('/').to_string();
    settings.chat_path = normalize_chat_path(&settings.chat_path);
    settings.models_path = settings
        .models_path
        .as_deref()
        .map(normalize_optional_path)
        .filter(|value| !value.is_empty());
    settings.api_key = settings.api_key.trim().to_string();
    settings.model = settings.model.trim().to_string();
    settings.batch_size = settings.batch_size.clamp(1, 100);
    settings.classification_concurrency = settings.classification_concurrency.clamp(1, 4);
    if settings.provider == AIProviderKind::Ollama {
        settings.classification_concurrency = settings.classification_concurrency.min(1);
    }
    settings.timeout_seconds = settings.timeout_seconds.clamp(1, 600);
    settings.max_tokens = settings.max_tokens.clamp(1, 32_768);
    settings.temperature = settings.temperature.clamp(0.0, 2.0);
    settings.classification_mode = match settings.classification_mode.trim() {
        "ai_first" | "rules_first" | "hybrid" => settings.classification_mode.trim().to_string(),
        _ => "rules_first".to_string(),
    };
    settings.reasoning_effort = settings
        .reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    settings.extra_body_json = settings
        .extra_body_json
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let mut seen_profile_ids = std::collections::HashSet::new();
    settings.custom_profiles = settings
        .custom_profiles
        .into_iter()
        .filter_map(|mut profile| {
            profile.id = profile.id.trim().to_string();
            profile.name = profile.name.trim().to_string();
            if !is_valid_profile_id(&profile.id) || !seen_profile_ids.insert(profile.id.clone()) {
                return None;
            }
            profile.base_url = profile.base_url.trim().trim_end_matches('/').to_string();
            profile.chat_path = normalize_chat_path(&profile.chat_path);
            profile.models_path = profile
                .models_path
                .as_deref()
                .map(normalize_optional_path)
                .filter(|value| !value.is_empty());
            profile.model = profile.model.trim().to_string();
            profile.thinking_parameter = profile.thinking_parameter.trim().to_string();
            profile.token_parameter = profile.token_parameter.trim().to_string();
            profile.content_path = profile.content_path.trim().to_string();
            profile.reasoning_path = profile.reasoning_path.trim().to_string();
            profile.extra_body_json = profile
                .extra_body_json
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string);
            Some(profile)
        })
        .take(20)
        .collect();
    if !settings
        .active_custom_profile_id
        .as_deref()
        .map(|profile_id| {
            settings
                .custom_profiles
                .iter()
                .any(|profile| profile.id == profile_id)
        })
        .unwrap_or(false)
    {
        settings.active_custom_profile_id = None;
    }
    settings
}

pub fn validate_ai_settings(settings: &AISettings, release_mode: bool) -> Result<(), String> {
    let preset = provider_preset(settings.preset)
        .ok_or_else(|| "AI provider preset is not supported.".to_string())?;
    if preset.provider_kind != settings.provider {
        return Err("AI provider and preset are incompatible.".to_string());
    }
    if !(1..=100).contains(&settings.batch_size) {
        return Err("AI batch size must be between 1 and 100.".to_string());
    }
    if !(1..=4).contains(&settings.classification_concurrency) {
        return Err("AI classification concurrency must be between 1 and 4.".to_string());
    }
    if !(1..=600).contains(&settings.timeout_seconds) {
        return Err("AI timeout must be between 1 and 600 seconds.".to_string());
    }
    if !(1..=32_768).contains(&settings.max_tokens) {
        return Err("AI max tokens must be between 1 and 32768.".to_string());
    }
    if !settings.temperature.is_finite() || !(0.0..=2.0).contains(&settings.temperature) {
        return Err("AI temperature must be between 0 and 2.".to_string());
    }
    validate_text_limit("model", &settings.model, 200)?;
    if settings.custom_profiles.len() > 20 {
        return Err("AI custom provider profiles cannot exceed 20 entries.".to_string());
    }
    let mut profile_ids = std::collections::HashSet::new();
    for profile in &settings.custom_profiles {
        if !is_valid_profile_id(&profile.id) || !profile_ids.insert(&profile.id) {
            return Err("AI custom provider profile IDs must be unique ASCII names.".to_string());
        }
        validate_text_limit("custom provider profile name", &profile.name, 120)?;
        validate_text_limit("custom provider profile base URL", &profile.base_url, 2_048)?;
        validate_text_limit("custom provider profile chat path", &profile.chat_path, 512)?;
        validate_text_limit("custom provider profile model", &profile.model, 200)?;
        if !profile.temperature_min.is_finite()
            || !profile.temperature_max.is_finite()
            || profile.temperature_min < 0.0
            || profile.temperature_max > 2.0
            || profile.temperature_min > profile.temperature_max
        {
            return Err("Custom provider temperature range must stay within 0 to 2.".to_string());
        }
        if !(1..=32_768).contains(&profile.max_output_tokens) {
            return Err(
                "Custom provider max output tokens must be between 1 and 32768.".to_string(),
            );
        }
        validate_provider_url(
            &AISettings {
                base_url: profile.base_url.clone(),
                chat_path: profile.chat_path.clone(),
                ..settings.clone()
            },
            release_mode,
        )?;
        validate_extra_body_json(profile.extra_body_json.as_deref())?;
    }
    if let Some(active_profile_id) = settings.active_custom_profile_id.as_deref() {
        if settings.preset != AIProviderPresetId::CustomOpenAICompatible
            || !settings
                .custom_profiles
                .iter()
                .any(|profile| profile.id == active_profile_id)
        {
            return Err("Active custom provider profile is not available.".to_string());
        }
    }
    validate_text_limit("base URL", &settings.base_url, 2_048)?;
    validate_text_limit("chat path", &settings.chat_path, 512)?;
    if let Some(models_path) = settings.models_path.as_deref() {
        validate_text_limit("models path", models_path, 512)?;
        if models_path.starts_with("//")
            || models_path.contains("://")
            || models_path.contains('\\')
            || models_path.trim().is_empty()
        {
            return Err("AI models path must be a relative URL path.".to_string());
        }
    }
    if let Some(reasoning_effort) = settings.reasoning_effort.as_deref() {
        validate_text_limit("reasoning effort", reasoning_effort, 64)?;
    }
    validate_provider_url(settings, release_mode)?;
    validate_extra_body_json(settings.extra_body_json.as_deref())?;
    Ok(())
}

fn validate_text_limit(label: &str, value: &str, max_len: usize) -> Result<(), String> {
    if value.len() > max_len {
        return Err(format!("AI {label} exceeds {max_len} characters."));
    }
    Ok(())
}

fn validate_provider_url(settings: &AISettings, release_mode: bool) -> Result<(), String> {
    let parsed = url::Url::parse(settings.base_url.trim())
        .map_err(|_| "AI base URL must be a valid HTTP or HTTPS URL.".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("AI base URL only supports HTTP or HTTPS.".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("AI base URL must not contain user credentials.".to_string());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "AI base URL must include a host.".to_string())?;
    let localhost = host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || matches!(host, "::1" | "[::1]");
    if release_mode && parsed.scheme() == "http" && !localhost {
        return Err("Release builds require HTTPS for non-local AI providers.".to_string());
    }
    let chat_path = settings.chat_path.trim();
    if chat_path.is_empty()
        || chat_path.starts_with("//")
        || chat_path.contains("://")
        || chat_path.contains('\\')
    {
        return Err("AI chat path must be a relative URL path.".to_string());
    }
    let normalized_chat_path = format!("/{}", chat_path.trim_start_matches('/'));
    let joined = parsed
        .join(&normalized_chat_path)
        .map_err(|_| "AI chat path must be a valid relative URL path.".to_string())?;
    if joined.scheme() != parsed.scheme()
        || joined.host_str() != parsed.host_str()
        || joined.port_or_known_default() != parsed.port_or_known_default()
    {
        return Err("AI chat path must not change the provider origin.".to_string());
    }
    Ok(())
}

fn validate_extra_body_json(extra: Option<&str>) -> Result<(), String> {
    let Some(extra) = extra.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    if extra.len() > 16_384 {
        return Err("AI extra body JSON exceeds 16384 characters.".to_string());
    }
    let value: serde_json::Value = serde_json::from_str(extra)
        .map_err(|error| format!("AI extra body must be valid JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "AI extra body must be a JSON object.".to_string())?;
    const RESERVED: &[&str] = &[
        "model",
        "messages",
        "stream",
        "temperature",
        "max_tokens",
        "max_completion_tokens",
        "response_format",
        "thinking",
        "reasoning_effort",
        "tools",
        "tool_choice",
    ];
    if let Some(field) = RESERVED.iter().find(|field| object.contains_key(**field)) {
        return Err(format!(
            "AI extra body cannot override internal field: {field}."
        ));
    }
    if json_depth(&value) > 8 {
        return Err("AI extra body JSON nesting exceeds 8 levels.".to_string());
    }
    Ok(())
}

fn json_depth(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Array(values) => {
            1 + values.iter().map(json_depth).max().unwrap_or_default()
        }
        serde_json::Value::Object(values) => {
            1 + values.values().map(json_depth).max().unwrap_or_default()
        }
        _ => 0,
    }
}

pub fn test_ai_provider_connection_for_settings(
    settings: AISettings,
) -> Result<AIConnectionTestResult, String> {
    let settings = normalize_ai_settings(settings);
    validate_ai_settings(&settings, !cfg!(debug_assertions))?;
    let started = Instant::now();

    let mut result = match settings.provider {
        AIProviderKind::OpenAICompatible => {
            OpenAICompatibleProvider::new(settings.clone()).test_connection()
        }
        AIProviderKind::Ollama => OllamaProvider::new(settings.clone()).test_connection(),
    }
    .map_err(|error| sanitize_ai_error(error.to_string(), &settings.api_key))?;
    result.elapsed_ms = started.elapsed().as_millis();
    Ok(result)
}

pub(crate) fn provider_for_settings(settings: &AISettings) -> Box<dyn AIProvider> {
    match settings.provider {
        AIProviderKind::OpenAICompatible => {
            Box::new(OpenAICompatibleProvider::new(settings.clone()))
        }
        AIProviderKind::Ollama => Box::new(OllamaProvider::new(settings.clone())),
    }
}

pub fn list_ai_models_for_settings(settings: AISettings) -> Result<Vec<AIModelInfo>, String> {
    let settings = normalize_ai_settings(settings);
    validate_ai_settings(&settings, !cfg!(debug_assertions))?;
    let provider = provider_for_settings(&settings);
    let preset = provider_preset(settings.preset);
    let mut models = match provider.discover_models() {
        Ok(models) => models,
        Err(_error)
            if preset
                .as_ref()
                .map(|preset| preset.models_path.is_none())
                .unwrap_or(false) =>
        {
            Vec::new()
        }
        Err(error) => return Err(sanitize_ai_error(error.to_string(), &settings.api_key)),
    };
    if let Some(preset) = preset {
        let mut seen = models
            .iter()
            .map(|model| model.id.clone())
            .collect::<std::collections::HashSet<_>>();
        for model in preset.suggested_models {
            if !model.is_empty() && seen.insert((*model).to_string()) {
                models.push(AIModelInfo {
                    id: (*model).to_string(),
                    owned_by: None,
                    discovered: false,
                });
            }
        }
    }
    Ok(models)
}

#[tauri::command]
pub fn get_ai_settings(db: State<'_, Database>) -> Result<AISettings, String> {
    get_ai_settings_for_db(db.inner())
        .map(public_ai_settings)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_ai_settings<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    settings: AISettings,
) -> Result<AISettings, String> {
    require_main_window(&window)?;
    save_ai_settings_for_db(db.inner(), &settings)
        .map(public_ai_settings)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn list_ai_provider_presets() -> Vec<AIProviderPreset> {
    all_provider_presets()
}

#[tauri::command]
pub async fn test_ai_provider_connection(
    db: State<'_, Database>,
    settings: Option<AISettings>,
) -> Result<AIConnectionTestResult, String> {
    let db = db.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let settings = match settings {
            Some(mut settings) => {
                if settings.api_key.trim().is_empty() {
                    settings.api_key = get_ai_settings_for_db(&db)
                        .map_err(|error| error.to_string())?
                        .api_key;
                }
                normalize_ai_settings(settings)
            }
            None => get_ai_settings_for_db(&db).map_err(|error| error.to_string())?,
        };
        test_ai_provider_connection_for_settings(settings)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn list_ai_models(
    db: State<'_, Database>,
    settings: Option<AISettings>,
) -> Result<Vec<AIModelInfo>, String> {
    let db = db.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let settings = match settings {
            Some(mut settings) => {
                if settings.api_key.trim().is_empty() {
                    settings.api_key = get_ai_settings_for_db(&db)
                        .map_err(|error| error.to_string())?
                        .api_key;
                }
                normalize_ai_settings(settings)
            }
            None => get_ai_settings_for_db(&db).map_err(|error| error.to_string())?,
        };
        list_ai_models_for_settings(settings)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn normalize_chat_path(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "/chat/completions".to_string();
    }
    format!("/{}", trimmed.trim_start_matches('/'))
}

fn normalize_optional_path(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    format!("/{}", trimmed.trim_start_matches('/'))
}

fn sanitize_ai_error(message: String, api_key: &str) -> String {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        message
    } else {
        message.replace(api_key, "[redacted]")
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
        thread,
        time::Duration,
    };

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    #[test]
    fn system_credential_store_fails_closed_on_unsupported_platforms() {
        let store = SystemCredentialStore;

        assert_eq!(
            store.get().expect_err("system credential reads must fail"),
            SYSTEM_CREDENTIAL_STORE_UNSUPPORTED_PLATFORM
        );
        assert_eq!(
            store
                .set("test-api-key")
                .expect_err("system credential writes must fail"),
            SYSTEM_CREDENTIAL_STORE_UNSUPPORTED_PLATFORM
        );
        assert_eq!(
            store
                .delete()
                .expect_err("system credential deletes must fail"),
            SYSTEM_CREDENTIAL_STORE_UNSUPPORTED_PLATFORM
        );
        assert_eq!(
            store
                .get_profile("test-profile")
                .expect_err("custom provider credential reads must fail"),
            SYSTEM_CREDENTIAL_STORE_UNSUPPORTED_PLATFORM
        );
        assert_eq!(
            store
                .set_profile("test-profile", "test-api-key")
                .expect_err("custom provider credential writes must fail"),
            SYSTEM_CREDENTIAL_STORE_UNSUPPORTED_PLATFORM
        );
        assert_eq!(
            store
                .delete_profile("test-profile")
                .expect_err("custom provider credential deletes must fail"),
            SYSTEM_CREDENTIAL_STORE_UNSUPPORTED_PLATFORM
        );
    }

    #[test]
    fn public_ai_settings_reports_configuration_without_exposing_the_key() {
        let public = public_ai_settings(AISettings {
            api_key: "top-secret".to_string(),
            api_key_action: ApiKeyAction::Replace,
            ..AISettings::default()
        });

        assert!(public.api_key.is_empty());
        assert!(public.api_key_configured);
        assert_eq!(public.api_key_action, ApiKeyAction::Preserve);
        let json = serde_json::to_value(public).expect("serialize public settings");
        assert_eq!(json["apiKey"], "");
        assert_eq!(json["apiKeyConfigured"], true);
        assert!(json.get("apiKeyAction").is_none());
        assert!(!json.to_string().contains("top-secret"));
    }

    #[test]
    fn ai_numeric_bounds_accept_limits_and_reject_out_of_range_values() {
        let mut settings = AISettings {
            batch_size: 100,
            classification_concurrency: 4,
            timeout_seconds: 600,
            max_tokens: 32_768,
            temperature: 2.0,
            ..AISettings::default()
        };
        assert!(validate_ai_settings(&settings, true).is_ok());

        settings.batch_size = 101;
        assert!(validate_ai_settings(&settings, true).is_err());
        settings.batch_size = 0;
        assert!(validate_ai_settings(&settings, true).is_err());
        settings.batch_size = 1;
        settings.classification_concurrency = 5;
        assert!(validate_ai_settings(&settings, true).is_err());
        settings.classification_concurrency = 0;
        assert!(validate_ai_settings(&settings, true).is_err());
        settings.classification_concurrency = 1;
        settings.timeout_seconds = 601;
        assert!(validate_ai_settings(&settings, true).is_err());
        settings.timeout_seconds = 0;
        assert!(validate_ai_settings(&settings, true).is_err());
        settings.timeout_seconds = 1;
        settings.max_tokens = 32_769;
        assert!(validate_ai_settings(&settings, true).is_err());
        settings.max_tokens = 0;
        assert!(validate_ai_settings(&settings, true).is_err());
        settings.max_tokens = 1;
        settings.temperature = -0.1;
        assert!(validate_ai_settings(&settings, true).is_err());
        settings.temperature = 2.1;
        assert!(validate_ai_settings(&settings, true).is_err());
        settings.temperature = f32::NAN;
        assert!(validate_ai_settings(&settings, true).is_err());
    }

    #[test]
    fn ai_url_policy_rejects_unsafe_schemes_userinfo_and_release_http() {
        let mut settings = AISettings::default();
        for url in [
            "file:///tmp/model",
            "ftp://example.com",
            "data:text/plain,x",
        ] {
            settings.base_url = url.to_string();
            assert!(validate_ai_settings(&settings, true).is_err(), "{url}");
        }
        settings.base_url = "https://user:secret@example.com".to_string();
        assert!(validate_ai_settings(&settings, true).is_err());
        settings.base_url = "http://example.com".to_string();
        assert!(validate_ai_settings(&settings, true).is_err());

        for url in [
            "http://localhost:11434",
            "http://127.0.0.1:11434",
            "http://[::1]:11434",
        ] {
            settings.base_url = url.to_string();
            assert!(validate_ai_settings(&settings, true).is_ok(), "{url}");
        }
    }

    #[test]
    fn ai_provider_preset_and_chat_path_cannot_cross_security_boundaries() {
        let mut settings = AISettings {
            provider: AIProviderKind::Ollama,
            ..AISettings::default()
        };
        assert!(validate_ai_settings(&settings, true).is_err());

        settings.provider = AIProviderKind::OpenAICompatible;
        for chat_path in [
            "https://evil.example/chat",
            "//evil.example/chat",
            "\\\\evil\\chat",
        ] {
            settings.chat_path = chat_path.to_string();
            assert!(
                validate_ai_settings(&settings, true).is_err(),
                "{chat_path}"
            );
        }
    }

    #[test]
    fn ai_extra_body_requires_bounded_safe_json_object() {
        let mut settings = AISettings::default();
        for json in [
            "not-json",
            "[]",
            r#"{"model":"override"}"#,
            r#"{"messages":[]}"#,
            r#"{"stream":true}"#,
            r#"{"max_completion_tokens":999999}"#,
            r#"{"response_format":{"type":"text"}}"#,
            r#"{"thinking":{"type":"disabled"}}"#,
            r#"{"reasoning_effort":"high"}"#,
        ] {
            settings.extra_body_json = Some(json.to_string());
            assert!(validate_ai_settings(&settings, true).is_err(), "{json}");
        }
        settings.extra_body_json = Some(r#"{"safe_extension":true}"#.to_string());
        assert!(validate_ai_settings(&settings, true).is_ok());
        settings.extra_body_json = Some(format!(r#"{{"value":"{}"}}"#, "x".repeat(16_384)));
        assert!(validate_ai_settings(&settings, true).is_err());
        settings.extra_body_json =
            Some(r#"{"a":{"b":{"c":{"d":{"e":{"f":{"g":{"h":{"i":1}}}}}}}}}}"#.to_string());
        assert!(validate_ai_settings(&settings, true).is_err());
    }

    #[test]
    fn ai_text_fields_enforce_length_limits() {
        let mut settings = AISettings {
            model: "x".repeat(201),
            ..AISettings::default()
        };
        assert!(validate_ai_settings(&settings, true).is_err());
        settings.model = "model".to_string();
        settings.reasoning_effort = Some("x".repeat(65));
        assert!(validate_ai_settings(&settings, true).is_err());
    }

    #[derive(Default)]
    struct ConcurrentCredentialStore {
        value: Mutex<Option<String>>,
        active: AtomicUsize,
        max_active: AtomicUsize,
    }

    impl ConcurrentCredentialStore {
        fn begin(&self) {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(10));
        }

        fn end(&self) {
            self.active.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl CredentialStore for ConcurrentCredentialStore {
        fn set(&self, value: &str) -> Result<(), String> {
            self.begin();
            let result = self
                .value
                .lock()
                .map_err(|_| "credential test lock poisoned".to_string())
                .map(|mut current| *current = Some(value.to_string()));
            self.end();
            result
        }

        fn get(&self) -> Result<Option<String>, String> {
            self.begin();
            let result = self
                .value
                .lock()
                .map(|current| current.clone())
                .map_err(|_| "credential test lock poisoned".to_string());
            self.end();
            result
        }

        fn delete(&self) -> Result<(), String> {
            self.begin();
            let result = self
                .value
                .lock()
                .map_err(|_| "credential test lock poisoned".to_string())
                .map(|mut current| *current = None);
            self.end();
            result
        }
    }

    struct FaultyCredentialStore {
        inner: InMemoryCredentialStore,
        fail_delete_profile: AtomicBool,
    }

    impl Default for FaultyCredentialStore {
        fn default() -> Self {
            Self {
                inner: InMemoryCredentialStore::default(),
                fail_delete_profile: AtomicBool::new(false),
            }
        }
    }

    impl CredentialStore for FaultyCredentialStore {
        fn set(&self, value: &str) -> Result<(), String> {
            self.inner.set(value)
        }

        fn get(&self) -> Result<Option<String>, String> {
            self.inner.get()
        }

        fn delete(&self) -> Result<(), String> {
            self.inner.delete()
        }

        fn set_profile(&self, profile_id: &str, value: &str) -> Result<(), String> {
            self.inner.set_profile(profile_id, value)
        }

        fn get_profile(&self, profile_id: &str) -> Result<Option<String>, String> {
            self.inner.get_profile(profile_id)
        }

        fn delete_profile(&self, profile_id: &str) -> Result<(), String> {
            if self.fail_delete_profile.load(Ordering::SeqCst) {
                return Err("simulated keyring deletion failure".to_string());
            }
            self.inner.delete_profile(profile_id)
        }
    }

    fn custom_profile(id: &str) -> AICustomProviderProfile {
        AICustomProviderProfile {
            id: id.to_string(),
            name: format!("Profile {id}"),
            base_url: "https://provider.example.com".to_string(),
            chat_path: "/chat/completions".to_string(),
            models_path: Some("/models".to_string()),
            model: "model".to_string(),
            supports_response_format: true,
            supports_thinking: false,
            thinking_parameter: "thinking".to_string(),
            token_parameter: "max_tokens".to_string(),
            content_path: "choices.0.message.content".to_string(),
            reasoning_path: "choices.0.message.reasoning_content".to_string(),
            temperature_min: 0.0,
            temperature_max: 2.0,
            max_output_tokens: 1024,
            extra_body_json: None,
            api_key_configured: false,
        }
    }

    fn profile_settings(profile_id: &str, api_key_action: ApiKeyAction) -> AISettings {
        AISettings {
            preset: AIProviderPresetId::CustomOpenAICompatible,
            provider: AIProviderKind::OpenAICompatible,
            custom_profiles: vec![custom_profile(profile_id)],
            active_custom_profile_id: Some(profile_id.to_string()),
            api_key: "profile-secret".to_string(),
            api_key_action,
            ..AISettings::default()
        }
    }

    fn concurrency_test_db_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "zen-canvas-ai-settings-{label}-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn committed_ai_settings_change_wakes_managed_worker() {
        let db_path = concurrency_test_db_path("managed-worker-wake");
        let db = Database::open(&db_path).expect("settings wake database");
        let (wake_tx, wake_rx) = std::sync::mpsc::sync_channel(1);
        db.set_managed_ai_waker(wake_tx);

        save_ai_settings_with_store(
            &db,
            &AISettings::default(),
            &InMemoryCredentialStore::default(),
        )
        .expect("commit AI settings");
        assert_eq!(wake_rx.try_recv(), Ok(()));

        drop(db);
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn concurrent_credential_saves_are_serialized_as_transactions() {
        let credentials = Arc::new(ConcurrentCredentialStore::default());
        let first = AISettings {
            api_key: "first-test-key".to_string(),
            api_key_action: ApiKeyAction::Replace,
            ..AISettings::default()
        };
        let second = AISettings {
            api_key: "second-test-key".to_string(),
            api_key_action: ApiKeyAction::Replace,
            ..AISettings::default()
        };

        let first_path = concurrency_test_db_path("first");
        let second_path = concurrency_test_db_path("second");
        let first_thread_path = first_path.clone();
        let first_credentials = Arc::clone(&credentials);
        let first_thread = thread::spawn(move || {
            let db = Database::open(&first_thread_path).expect("first database");
            save_ai_settings_with_store(&db, &first, first_credentials.as_ref())
        });
        let second_thread_path = second_path.clone();
        let second_credentials = Arc::clone(&credentials);
        let second_thread = thread::spawn(move || {
            let db = Database::open(&second_thread_path).expect("second database");
            save_ai_settings_with_store(&db, &second, second_credentials.as_ref())
        });

        first_thread
            .join()
            .expect("first save thread")
            .expect("first save");
        second_thread
            .join()
            .expect("second save thread")
            .expect("second save");

        assert_eq!(credentials.max_active.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_file(first_path);
        let _ = std::fs::remove_file(second_path);
    }

    #[test]
    fn deleting_provider_profile_compensates_keyring_failure_and_keeps_db_profile() {
        let db_path = concurrency_test_db_path("profile-delete-failure");
        let db = Database::open(&db_path).expect("profile delete database");
        let credentials = FaultyCredentialStore::default();
        let initial = profile_settings("removed-profile", ApiKeyAction::Replace);
        save_ai_settings_with_store(&db, &initial, &credentials).expect("initial profile save");
        credentials
            .fail_delete_profile
            .store(true, Ordering::SeqCst);

        let mut removal = initial.clone();
        removal.custom_profiles.clear();
        removal.active_custom_profile_id = None;
        removal.api_key.clear();
        removal.api_key_action = ApiKeyAction::Preserve;
        let error = save_ai_settings_with_store(&db, &removal, &credentials)
            .expect_err("profile delete should fail closed");

        assert!(error.to_string().contains("profile deletion failed"));
        assert!(stored_custom_profile_ids(&db)
            .expect("read retained profile")
            .contains("removed-profile"));
        assert_eq!(
            credentials
                .get_profile("removed-profile")
                .expect("read restored profile"),
            Some("profile-secret".to_string())
        );
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn deleting_provider_profile_commits_db_and_keyring_together() {
        let db_path = concurrency_test_db_path("profile-delete-success");
        let db = Database::open(&db_path).expect("profile delete database");
        let credentials = InMemoryCredentialStore::default();
        let initial = profile_settings("removed-profile", ApiKeyAction::Replace);
        save_ai_settings_with_store(&db, &initial, &credentials).expect("initial profile save");

        let mut removal = initial.clone();
        removal.custom_profiles.clear();
        removal.active_custom_profile_id = None;
        removal.api_key.clear();
        removal.api_key_action = ApiKeyAction::Preserve;
        save_ai_settings_with_store(&db, &removal, &credentials).expect("profile removal");

        assert!(!stored_custom_profile_ids(&db)
            .expect("read removed profile")
            .contains("removed-profile"));
        assert_eq!(
            credentials
                .get_profile("removed-profile")
                .expect("read removed credential"),
            None
        );
        let _ = std::fs::remove_file(db_path);
    }
}
