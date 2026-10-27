use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub const PROVIDER_WINDOWS_MFT_USN: &str = "windows_mft_usn";
pub const PROVIDER_WINDOWS_RECURSIVE_FALLBACK: &str = "windows_recursive_fallback";
pub const PROVIDER_WINDOWS_UNSUPPORTED: &str = "windows_native_provider_unsupported";
pub const PROVIDER_MACOS_SPOTLIGHT: &str = "macos_spotlight";
pub const PROVIDER_MACOS_FSEVENTS_RECONCILE: &str = "macos_fsevents_reconcile";
pub const PROVIDER_RECURSIVE_FALLBACK: &str = "recursive_fallback";

pub const SCOPE_AUTHORIZATION_NOT_REQUIRED: &str = "not_required";
pub const SCOPE_AUTHORIZATION_NOT_AUTHORIZED: &str = "not_authorized";
pub const SCOPE_AUTHORIZATION_LEGACY_CONFIRMATION_REQUIRED: &str = "legacy_confirmation_required";
pub const SCOPE_AUTHORIZATION_AUTHORIZED: &str = "authorized";

// These bits reuse the existing durable `file_attributes` column so package,
// backing and byte-availability metadata can be projected without a schema
// migration. They are descriptive flags, never mutation authorization.
pub const MACOS_FILE_ATTRIBUTE_ICLOUD: i64 = 1_i64 << 56;
pub const MACOS_FILE_ATTRIBUTE_FILE_PROVIDER: i64 = 1_i64 << 57;
pub const MACOS_FILE_ATTRIBUTE_CONTENT_DOWNLOADING: i64 = 1_i64 << 58;
pub const MACOS_FILE_ATTRIBUTE_CONTENT_UNKNOWN: i64 = 1_i64 << 59;
pub const MACOS_FILE_ATTRIBUTE_PACKAGE: i64 = 1_i64 << 60;
pub const MACOS_FILE_ATTRIBUTE_CLOUD_NOT_LOCAL: i64 = 1_i64 << 61;

pub const INDEX_STATUS_DISCOVERED: &str = "discovered";
pub const INDEX_STATUS_INDEXING: &str = "indexing";
pub const INDEX_STATUS_SYNCING: &str = "syncing";
pub const INDEX_STATUS_READY: &str = "ready";
pub const INDEX_STATUS_PAUSED: &str = "paused";
pub const INDEX_STATUS_REBUILD_REQUIRED: &str = "rebuild_required";
pub const INDEX_STATUS_PERMISSION_REQUIRED: &str = "permission_required";
pub const INDEX_STATUS_DATABASE_BUSY: &str = "database_busy";
pub const INDEX_STATUS_SERVICE_BUSY: &str = "service_busy";
pub const INDEX_STATUS_SPOTLIGHT_UNAVAILABLE: &str = "spotlight_unavailable";
pub const INDEX_STATUS_SPOTLIGHT_NOT_INDEXED: &str = "spotlight_not_indexed";
pub const INDEX_STATUS_SPOTLIGHT_EXTERNAL_NOT_INDEXED: &str = "spotlight_external_not_indexed";
pub const INDEX_STATUS_FSEVENTS_UNAVAILABLE: &str = "fsevents_unavailable";
pub const INDEX_STATUS_UNAVAILABLE: &str = "unavailable";
pub const INDEX_STATUS_ERROR: &str = "error";

pub const AI_JOB_PENDING: &str = "pending";
pub const AI_JOB_RUNNING: &str = "running";
pub const AI_JOB_COMPLETED: &str = "completed";
pub const AI_JOB_FAILED: &str = "failed";
pub const AI_JOB_CANCELED: &str = "canceled";
pub const AI_JOB_STALE: &str = "stale";
pub const AI_JOB_BLOCKED_BY_POLICY: &str = "blocked_by_policy";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalVolume {
    pub id: String,
    pub platform: String,
    pub stable_volume_id: String,
    pub display_name: String,
    pub mount_path: String,
    pub filesystem_type: String,
    pub drive_kind: String,
    pub enabled: bool,
    #[serde(default = "default_scope_authorization_state")]
    pub scope_authorization_state: String,
    pub provider: String,
    pub index_status: String,
    pub last_error: Option<String>,
    pub journal_id: Option<String>,
    pub journal_cursor: Option<String>,
    pub last_full_index_at: Option<i64>,
    pub last_incremental_sync_at: Option<i64>,
    pub entry_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

fn default_scope_authorization_state() -> String {
    SCOPE_AUTHORIZATION_NOT_AUTHORIZED.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalEntry {
    pub id: String,
    pub volume_id: String,
    pub platform_file_id: String,
    pub parent_platform_file_id: String,
    pub name: String,
    pub name_normalized: String,
    pub path: String,
    pub path_normalized: String,
    pub extension: String,
    pub is_directory: bool,
    pub size: i64,
    pub created_at_fs: Option<i64>,
    pub modified_at_fs: Option<i64>,
    pub file_attributes: i64,
    pub is_hidden: bool,
    pub is_system: bool,
    pub is_stale: bool,
    pub source_provider: String,
    pub last_seen_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSearchResult {
    pub id: String,
    pub volume_id: String,
    pub platform_file_id: String,
    pub name: String,
    pub path: String,
    pub extension: String,
    pub is_directory: bool,
    pub size: i64,
    pub created_at_fs: Option<i64>,
    pub modified_at_fs: Option<i64>,
    pub file_attributes: i64,
    pub is_hidden: bool,
    pub is_system: bool,
    pub source_provider: String,
    pub managed: bool,
    pub rank: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalIndexSource {
    pub volume: GlobalVolume,
    pub can_pause: bool,
    pub can_rebuild: bool,
    pub technical_detail: Option<GlobalIndexTechnicalDetail>,
}

#[derive(Debug, Clone)]
pub struct GlobalSourceDescriptor {
    pub volume: GlobalVolume,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalIndexTechnicalDetail {
    pub journal_id: Option<String>,
    pub journal_cursor: Option<String>,
    pub provider: String,
    pub filesystem_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalIndexStatus {
    pub platform: String,
    pub enabled: bool,
    pub status: String,
    pub provider_status: Option<String>,
    pub processed_entries: i64,
    pub collection_complete: bool,
    pub total_entries: i64,
    pub indexed_volumes: i64,
    pub ready_volumes: i64,
    pub pending_volumes: i64,
    pub last_sync_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSearchRequest {
    #[serde(default = "default_search_contract_version")]
    pub version: u32,
    pub request_id: String,
    pub query: String,
    #[serde(default = "default_search_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSearchSourceHealth {
    pub source_id: String,
    pub enabled: bool,
    pub provider: String,
    pub status: String,
    pub last_error: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSearchResponse {
    pub version: u32,
    pub request_id: String,
    pub normalized_query: String,
    pub results: Vec<GlobalSearchResult>,
    pub index_status: GlobalIndexStatus,
    pub collection_complete: bool,
    pub result_state: String,
    pub source_revision: String,
    pub source_health: Vec<GlobalSearchSourceHealth>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedScope {
    pub id: String,
    pub path: String,
    pub global_entry_id: Option<String>,
    pub enabled: bool,
    pub allow_local_ai: bool,
    pub allow_cloud_ai: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedEntry {
    pub id: String,
    pub global_entry_id: String,
    pub managed_scope_id: String,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AiManagementStatus {
    pub enabled_scope_count: i64,
    pub managed_entry_count: i64,
    pub pending_job_count: i64,
    pub running_job_count: i64,
    pub cloud_scope_count: i64,
    pub policy_summary: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddManagedScopeRequest {
    pub path: String,
    pub global_entry_id: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub allow_local_ai: bool,
    #[serde(default)]
    pub allow_cloud_ai: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateManagedScopePolicyRequest {
    pub id: String,
    pub enabled: Option<bool>,
    pub allow_local_ai: Option<bool>,
    pub allow_cloud_ai: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSearchQuery {
    pub query: String,
    #[serde(default = "default_search_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

const fn default_search_contract_version() -> u32 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalEntryInput {
    pub volume_id: String,
    pub platform_file_id: String,
    pub parent_platform_file_id: String,
    pub name: String,
    pub path: String,
    pub extension: String,
    pub is_directory: bool,
    pub size: i64,
    pub created_at_fs: Option<i64>,
    pub modified_at_fs: Option<i64>,
    pub file_attributes: i64,
    pub is_hidden: bool,
    pub is_system: bool,
    pub source_provider: String,
    pub last_seen_at: i64,
}

impl GlobalEntryInput {
    pub fn from_path(
        volume_id: impl Into<String>,
        path: &Path,
        provider: impl Into<String>,
    ) -> Self {
        let volume_id = volume_id.into();
        let path_text = path.to_string_lossy().into_owned();
        let metadata = std::fs::symlink_metadata(path).ok();
        let is_directory = metadata.as_ref().is_some_and(std::fs::Metadata::is_dir);
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| path_text.clone());
        let extension = path
            .extension()
            .map(|value| value.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let modified_at_fs = metadata
            .as_ref()
            .and_then(|value| system_time_seconds(value.modified().ok()));
        let created_at_fs = metadata
            .as_ref()
            .and_then(|value| system_time_seconds(value.created().ok()));
        let size = metadata
            .as_ref()
            .filter(|_| !is_directory)
            .map(|value| value.len().min(i64::MAX as u64) as i64)
            .unwrap_or_default();
        let platform_file_id = format!("path:{}", normalize_path(&path_text));
        Self {
            volume_id,
            platform_file_id,
            parent_platform_file_id: path
                .parent()
                .map(|value| format!("path:{}", normalize_path(&value.to_string_lossy())))
                .unwrap_or_default(),
            name,
            path: path_text,
            extension,
            is_directory,
            size,
            created_at_fs,
            modified_at_fs,
            file_attributes: 0,
            is_hidden: false,
            is_system: false,
            source_provider: provider.into(),
            last_seen_at: unix_now(),
        }
    }

    pub fn entry_id(&self) -> String {
        stable_entry_id(
            &self.volume_id,
            &self.platform_file_id,
            &self.parent_platform_file_id,
            &self.name,
            &self.path,
        )
    }
}

impl GlobalVolume {
    #[cfg(target_os = "macos")]
    pub fn macos_spotlight_local_computer() -> Self {
        let semantics = crate::platform::macos::volume::inspect(Path::new("/"));
        let now = unix_now();
        Self {
            id: "gv_macos_local_computer".to_string(),
            platform: "macos".to_string(),
            // Keep the existing durable local-computer key. Native volume
            // identity is diagnostic metadata here; changing this key would
            // split the existing index without a migration.
            stable_volume_id: "macos:local-computer".to_string(),
            display_name: "Local Mac".to_string(),
            mount_path: "/".to_string(),
            filesystem_type: semantics
                .filesystem_type
                .unwrap_or_else(|| "unknown".to_string()),
            drive_kind: "fixed".to_string(),
            enabled: true,
            scope_authorization_state: SCOPE_AUTHORIZATION_NOT_REQUIRED.to_string(),
            provider: PROVIDER_MACOS_SPOTLIGHT.to_string(),
            index_status: INDEX_STATUS_DISCOVERED.to_string(),
            last_error: None,
            journal_id: None,
            journal_cursor: None,
            last_full_index_at: None,
            last_incremental_sync_at: None,
            entry_count: 0,
            created_at: now,
            updated_at: now,
        }
    }
}

pub fn stable_entry_id(
    volume_id: &str,
    platform_file_id: &str,
    parent_platform_file_id: &str,
    name: &str,
    path_fallback: &str,
) -> String {
    let identity = if platform_file_id.is_empty() || platform_file_id.starts_with("path:") {
        format!("path\0{}", normalize_path(path_fallback))
    } else if platform_file_id.starts_with("mac:") {
        // macOS dev/inode identity represents the physical item. Keeping the
        // entry id independent of its parent/name lets a Spotlight rename or
        // move update the existing row instead of leaving the old path stale.
        platform_file_id.to_string()
    } else {
        format!(
            "{}\0{}\0{}",
            platform_file_id, parent_platform_file_id, name
        )
    };
    format!(
        "ge_{}",
        blake3::hash(format!("{volume_id}\0{identity}").as_bytes()).to_hex()
    )
}

pub fn normalize_path(path: &str) -> String {
    let mut normalized = path.replace('\\', "/");
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    if cfg!(windows) {
        normalized.make_ascii_lowercase();
    }
    normalized
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or_default()
}

fn system_time_seconds(value: Option<SystemTime>) -> Option<i64> {
    value
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
}

fn default_true() -> bool {
    true
}

fn default_search_limit() -> u32 {
    80
}
