//! Task 05 File Library query, metadata tag and Saved View repository.
//!
//! This module is deliberately separate from the compatibility `get_paged_files`
//! implementation.  The compatibility API keeps its old path/OFFSET contract;
//! Vault uses only the versioned, root-ID and revision-bound API below.

use super::*;
use rusqlite::{
    params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension, Row,
    Transaction,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

mod personal;
mod saved_views;
mod tags;
pub use personal::*;

const LIBRARY_QUERY_VERSION: i32 = 2;
const LIBRARY_CURSOR_VERSION: i32 = 2;
const LIBRARY_PAGE_MAX: u32 = 200;
const LIBRARY_TAG_NAME_MAX: usize = 64;
const LIBRARY_SAVED_VIEW_NAME_MAX: usize = 128;
const LIBRARY_SELECTION_MAX: usize = 100_000;
const LIBRARY_TAG_PREVIEW_LIMIT: i64 = 3;
const LIBRARY_CURSOR_MAX_CHARS: usize = 12_000;

const COLOR_TOKENS: &[&str] = &[
    "neutral", "blue", "green", "yellow", "red", "purple", "teal", "orange",
];
const FILE_TYPES: &[&str] = &[
    "Document",
    "Image",
    "Video",
    "Audio",
    "Code",
    "ArchivePackage",
    "Installer",
    "Spreadsheet",
    "Presentation",
    "Other",
];
const PURPOSES: &[&str] = &[
    "Project",
    "Teaching",
    "Study",
    "Work",
    "Personal",
    "Career",
    "Finance",
    "Identity",
    "Media",
    "Installer",
    "Temporary",
    "Archive",
    "Document",
    "Duplicate Review",
    "Unknown",
];
const LIFECYCLES: &[&str] = &[
    "Inbox",
    "Active",
    "Reference",
    "Archive",
    "Disposable",
    "Duplicate",
    "Sensitive",
    "TrashReview",
    "Unknown",
];
const RISKS: &[&str] = &["Normal", "Sensitive", "System", "Caution", "Unknown"];

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct FileQueryRequestV2 {
    pub version: i32,
    pub request_id: String,
    pub query: FileQuerySpecV2,
    pub page_size: u32,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct FileQuerySpecV2 {
    pub scope: FileLibraryScopeV2,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub filters: FileQueryFiltersV2,
    #[serde(default)]
    pub sort: FileLibrarySortV2,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileLibraryScopeV2 {
    AllEnabledRoots,
    Roots {
        #[serde(rename = "scanRootIds")]
        scan_root_ids: Vec<String>,
    },
    CurrentScan {
        #[serde(rename = "scanSessionId")]
        scan_session_id: String,
    },
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryMatchMode {
    #[default]
    Any,
    Only,
    Exclude,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct FileQueryFiltersV2 {
    #[serde(default)]
    pub file_types: Vec<String>,
    #[serde(default)]
    pub purposes: Vec<String>,
    #[serde(default)]
    pub lifecycles: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub size_min: Option<i64>,
    #[serde(default)]
    pub size_max: Option<i64>,
    #[serde(default)]
    pub modified_from: Option<i64>,
    #[serde(default)]
    pub modified_to: Option<i64>,
    #[serde(default)]
    pub created_from: Option<i64>,
    #[serde(default)]
    pub created_to: Option<i64>,
    #[serde(default)]
    pub duplicate: LibraryMatchMode,
    #[serde(default)]
    pub review: LibraryMatchMode,
    #[serde(default)]
    pub tags_all_of: Vec<String>,
    #[serde(default)]
    pub tags_any_of: Vec<String>,
    #[serde(default)]
    pub tags_none_of: Vec<String>,
}

impl Default for FileQueryFiltersV2 {
    fn default() -> Self {
        Self {
            file_types: Vec::new(),
            purposes: Vec::new(),
            lifecycles: Vec::new(),
            risks: Vec::new(),
            size_min: None,
            size_max: None,
            modified_from: None,
            modified_to: None,
            created_from: None,
            created_to: None,
            duplicate: LibraryMatchMode::Any,
            review: LibraryMatchMode::Any,
            tags_all_of: Vec::new(),
            tags_any_of: Vec::new(),
            tags_none_of: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySortKind {
    Relevance,
    Modified,
    Created,
    Name,
    Size,
    Confidence,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct FileLibrarySortV2 {
    pub kind: LibrarySortKind,
    pub direction: LibrarySortDirection,
}

impl Default for FileLibrarySortV2 {
    fn default() -> Self {
        Self {
            kind: LibrarySortKind::Modified,
            direction: LibrarySortDirection::Desc,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileQueryResponseV2 {
    pub version: i32,
    pub request_id: String,
    pub query_fingerprint: String,
    pub snapshot_revision: i64,
    pub files: Vec<FileLibrarySummaryDto>,
    pub total_count: Option<i64>,
    pub count_state: String,
    pub count_token: Option<String>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub result_state: String,
    pub scope_health: LibraryScopeHealthDto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryScopeHealthDto {
    pub state: String,
    pub roots: Vec<LibraryScopeRootHealthDto>,
    pub invalid_references: Vec<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryScopeRootHealthDto {
    pub id: String,
    pub display_name: String,
    pub health_status: String,
    pub enabled: bool,
    pub available: bool,
    pub generation: i64,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserTagPreviewDto {
    pub id: String,
    pub display_name: String,
    pub color_token: String,
}

/// Read-only native semantics projected onto the managed File Library row.
/// The durable `files` table remains the query authority; this projection does
/// not authorize mutation, cleanup, restore, or content reads.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileLibraryNativeSemanticsDto {
    pub is_package: bool,
    pub cloud_backing: String,
    pub content_availability: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileLibrarySummaryDto {
    pub id: String,
    pub name: String,
    pub extension: String,
    pub display_directory: String,
    pub size: i64,
    pub modified_at: i64,
    pub created_at: i64,
    pub is_directory: bool,
    pub file_type: String,
    pub purpose: String,
    pub lifecycle: String,
    pub risk: String,
    pub confidence: f64,
    pub is_duplicate: bool,
    pub requires_review: bool,
    pub is_stale: bool,
    pub tags: Vec<UserTagPreviewDto>,
    pub tag_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_semantics: Option<FileLibraryNativeSemanticsDto>,
    #[serde(skip)]
    pub(crate) rank: Option<f64>,
    #[serde(skip)]
    pub(crate) native_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileLibraryDetailDto {
    pub id: String,
    pub name: String,
    pub path: String,
    pub directory: String,
    pub extension: String,
    pub size: i64,
    pub modified_at: i64,
    pub created_at: i64,
    pub is_directory: bool,
    pub file_type: String,
    pub purpose: String,
    pub lifecycle: String,
    pub context: String,
    pub risk: String,
    pub confidence: f64,
    pub classification_status: String,
    pub classification_reason: String,
    pub matched_rules: Vec<String>,
    pub suggested_action: String,
    pub suggested_target_path: String,
    pub suggested_name: String,
    pub is_duplicate: bool,
    pub requires_review: bool,
    pub is_stale: bool,
    pub last_seen_at: i64,
    pub scan_root_id: Option<String>,
    pub scan_root_name: Option<String>,
    pub scope_health: Option<String>,
    pub duplicate_group_id: Option<String>,
    pub duplicate_group_size: i64,
    pub tags: Vec<UserTagPreviewDto>,
    pub active_findings: Vec<FileLibraryFindingSummaryDto>,
    pub safe_actions: Vec<String>,
    pub revision: i64,
    pub content_status: Option<String>,
    pub content_policy: Option<String>,
    pub content_summary: Option<String>,
    pub content_keywords: Vec<String>,
    pub content_language: Option<String>,
    pub content_provenance: Option<String>,
    pub content_truncated: Option<bool>,
    pub content_text_retained: Option<bool>,
    pub content_revision: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_semantics: Option<FileLibraryNativeSemanticsDto>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct ResolveFileLibraryExactCountRequestV2 {
    pub version: i32,
    pub request_id: String,
    pub count_token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveFileLibraryExactCountResponseV2 {
    pub version: i32,
    pub request_id: String,
    pub query_fingerprint: String,
    pub snapshot_revision: i64,
    pub total_count: i64,
    pub count_state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileLibraryFindingSummaryDto {
    pub id: String,
    pub finding_type: String,
    pub severity: String,
    pub detector: String,
    pub state: String,
    pub decision: String,
    pub evidence_summary: serde_json::Value,
    pub analysis_revision: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryTypeCountDto {
    pub file_type: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileLibrarySelectionSummaryDto {
    pub count: i64,
    pub total_size: i64,
    pub type_counts: Vec<LibraryTypeCountDto>,
    pub missing_count: i64,
    pub stale_count: i64,
    pub excluded_count: i64,
    pub common_directory: Option<String>,
    pub common_tags: Vec<UserTagPreviewDto>,
    pub common_tag_ids: Vec<String>,
    pub partial_tag_commonality_count: i64,
    pub snapshot_revision: i64,
    pub query_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LibrarySelectionV1 {
    Explicit {
        #[serde(rename = "fileIds")]
        file_ids: Vec<String>,
    },
    AllMatching {
        query: Box<FileQuerySpecV2>,
        #[serde(rename = "queryFingerprint")]
        query_fingerprint: String,
        #[serde(rename = "snapshotRevision")]
        snapshot_revision: i64,
        #[serde(default, rename = "excludedFileIds")]
        excluded_file_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct MutateFileUserTagsRequest {
    pub selection: LibrarySelectionV1,
    pub tag_ids: Vec<String>,
    pub operation: UserTagMutationOperation,
    pub expected_count: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserTagMutationOperation {
    Add,
    Remove,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutateFileUserTagsResultDto {
    pub applied_count: i64,
    pub already_present_count: i64,
    pub missing_count: i64,
    pub excluded_count: i64,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserTagDto {
    pub id: String,
    pub display_name: String,
    pub color_token: String,
    pub usage_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub revision: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserTagRequest {
    pub display_name: String,
    #[serde(default = "default_color_token")]
    pub color_token: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserTagRequest {
    pub id: String,
    pub display_name: String,
    pub color_token: String,
    pub expected_revision: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct DeleteUserTagRequest {
    pub id: String,
    pub confirm: bool,
    pub expected_usage_count: i64,
    pub expected_revision: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySavedViewDto {
    pub id: String,
    pub display_name: String,
    pub query: FileQuerySpecV2,
    pub query_fingerprint: String,
    pub position: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub revision: i64,
    pub invalid_references: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct CreateLibrarySavedViewRequest {
    pub display_name: String,
    pub query: FileQuerySpecV2,
    #[serde(default)]
    pub position: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct UpdateLibrarySavedViewRequest {
    pub id: String,
    pub display_name: String,
    pub query: FileQuerySpecV2,
    pub position: i64,
    pub expected_revision: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct DeleteLibrarySavedViewRequest {
    pub id: String,
    pub expected_revision: i64,
}

#[derive(Debug, Clone)]
struct CanonicalQuery {
    spec: FileQuerySpecV2,
    fingerprint: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedScope {
    pub(crate) clause: String,
    pub(crate) params: Vec<SqlValue>,
    pub(crate) health: LibraryScopeHealthDto,
}

#[derive(Debug, Clone)]
struct QueryParts {
    ctes: String,
    from: String,
    where_clause: String,
    params: Vec<SqlValue>,
    order: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LibraryCursor {
    pub(super) version: i32,
    pub(super) fingerprint: String,
    pub(super) revision: i64,
    pub(super) total_count: i64,
    pub(super) sort_kind: LibrarySortKind,
    pub(super) direction: LibrarySortDirection,
    pub(super) file_id: String,
    pub(super) last_i64: Option<i64>,
    pub(super) last_text: Option<String>,
    pub(super) last_f64_bits: Option<u64>,
    pub(super) last_rank_bits: Option<u64>,
    pub(super) last_mtime: Option<i64>,
}

fn default_color_token() -> String {
    "neutral".to_string()
}

pub(crate) fn bump_library_query_revision_in_transaction(
    tx: &Transaction<'_>,
) -> Result<i64, DbError> {
    let now = current_unix_seconds();
    tx.execute(
        "UPDATE library_query_state SET revision = revision + 1, updated_at = ?1 WHERE singleton_id = 1",
        params![now],
    )?;
    tx.query_row(
        "SELECT revision FROM library_query_state WHERE singleton_id = 1",
        [],
        |row| row.get(0),
    )
    .map_err(DbError::from)
}

pub fn canonicalize_file_query_spec(
    spec: FileQuerySpecV2,
) -> Result<(FileQuerySpecV2, String, String), DbError> {
    let mut spec = spec;
    let text = spec.text.take().map(|value| value.trim().to_string());
    spec.text = text.filter(|value| !value.is_empty());
    if spec
        .text
        .as_ref()
        .is_some_and(|value| value.chars().count() > 512)
    {
        return Err(DbError::Validation(
            "library_query_text_too_long".to_string(),
        ));
    }

    normalize_enum_vec(&mut spec.filters.file_types, FILE_TYPES, "file_type")?;
    normalize_enum_vec(&mut spec.filters.purposes, PURPOSES, "purpose")?;
    normalize_enum_vec(&mut spec.filters.lifecycles, LIFECYCLES, "lifecycle")?;
    normalize_enum_vec(&mut spec.filters.risks, RISKS, "risk")?;
    normalize_id_vec(&mut spec.filters.tags_all_of, "tag")?;
    normalize_id_vec(&mut spec.filters.tags_any_of, "tag")?;
    normalize_id_vec(&mut spec.filters.tags_none_of, "tag")?;
    if spec.filters.tags_all_of.len() > 100
        || spec.filters.tags_any_of.len() > 100
        || spec.filters.tags_none_of.len() > 100
    {
        return Err(DbError::Validation(
            "library_query_tag_filter_too_large".to_string(),
        ));
    }
    if spec.filters.size_min.is_some_and(|value| value < 0)
        || spec.filters.size_max.is_some_and(|value| value < 0)
        || spec
            .filters
            .size_min
            .zip(spec.filters.size_max)
            .is_some_and(|(min, max)| min > max)
        || spec
            .filters
            .modified_from
            .zip(spec.filters.modified_to)
            .is_some_and(|(min, max)| min > max)
        || spec
            .filters
            .created_from
            .zip(spec.filters.created_to)
            .is_some_and(|(min, max)| min > max)
    {
        return Err(DbError::Validation(
            "library_query_range_invalid".to_string(),
        ));
    }

    match &mut spec.scope {
        FileLibraryScopeV2::Roots { scan_root_ids } => {
            normalize_id_vec(scan_root_ids, "scan_root")?
        }
        FileLibraryScopeV2::CurrentScan { scan_session_id } => {
            *scan_session_id = scan_session_id.trim().to_string();
            if scan_session_id.is_empty()
                || scan_session_id.len() > 256
                || scan_session_id.chars().any(|ch| ch.is_control())
            {
                return Err(DbError::Validation(
                    "library_scope_invalid:session_id".to_string(),
                ));
            }
        }
        FileLibraryScopeV2::AllEnabledRoots => {}
    }
    if matches!(spec.sort.kind, LibrarySortKind::Relevance) && spec.text.is_none() {
        return Err(DbError::Validation(
            "library_sort_relevance_requires_text".to_string(),
        ));
    }
    let json = serde_json::to_string(&spec)?;
    let fingerprint = blake3::hash(json.as_bytes()).to_hex().to_string();
    Ok((spec, json, fingerprint))
}

fn membership_fingerprint(spec: &FileQuerySpecV2) -> Result<String, DbError> {
    let mut membership_spec = spec.clone();
    // Sorting cannot change membership.  The durable revision still binds
    // this bounded cache entry to the same SQLite snapshot.
    membership_spec.sort = FileLibrarySortV2::default();
    let (_, _, fingerprint) = canonicalize_file_query_spec(membership_spec)?;
    Ok(fingerprint)
}

fn normalize_enum_vec(
    values: &mut Vec<String>,
    allowed: &[&str],
    field: &str,
) -> Result<(), DbError> {
    for value in values.iter_mut() {
        *value = value.trim().to_string();
        if !allowed.iter().any(|allowed| *allowed == value) {
            return Err(DbError::Validation(format!(
                "library_query_invalid_{field}"
            )));
        }
    }
    values.sort();
    values.dedup();
    if values.len() > 32 {
        return Err(DbError::Validation(format!(
            "library_query_{field}_filter_too_large"
        )));
    }
    Ok(())
}

fn normalize_id_vec(values: &mut Vec<String>, field: &str) -> Result<(), DbError> {
    for value in values.iter_mut() {
        *value = value.trim().to_string();
        if value.is_empty() || value.len() > 256 || value.chars().any(|ch| ch.is_control()) {
            return Err(DbError::Validation(format!(
                "library_query_invalid_{field}_id"
            )));
        }
    }
    values.sort();
    values.dedup();
    if values.len() > 128 {
        return Err(DbError::Validation(format!(
            "library_query_{field}_filter_too_large"
        )));
    }
    Ok(())
}

impl Database {
    pub fn query_file_library_v2(
        &self,
        request: FileQueryRequestV2,
    ) -> Result<FileQueryResponseV2, DbError> {
        validate_query_request(&request)?;
        let (spec, _json, fingerprint) = canonicalize_file_query_spec(request.query)?;
        let canonical = CanonicalQuery {
            spec,
            fingerprint: fingerprint.clone(),
        };
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let revision = current_library_revision(&tx)?;
        let count_key = membership_fingerprint(&canonical.spec)?;
        let scope = resolve_scope(&tx, &canonical.spec.scope)?;
        let cursor = request.cursor.as_deref().map(decode_cursor).transpose()?;
        if let Some(cursor) = cursor.as_ref() {
            validate_cursor_binding(cursor, &canonical)?;
        }
        if cursor.is_some() && revision != cursor.as_ref().expect("cursor exists").revision {
            let response = FileQueryResponseV2 {
                version: LIBRARY_QUERY_VERSION,
                request_id: request.request_id,
                query_fingerprint: fingerprint,
                snapshot_revision: revision,
                files: Vec::new(),
                total_count: None,
                count_state: "deferred".to_string(),
                count_token: None,
                next_cursor: None,
                has_more: false,
                result_state: "snapshot_expired".to_string(),
                scope_health: scope.health,
            };
            tx.commit()?;
            return Ok(response);
        }
        if cursor.is_some() && scope.health.state == "invalid_reference" {
            return Err(DbError::Validation(
                "library_scope_invalid:reference".to_string(),
            ));
        }

        let tag_invalid = query_has_missing_tags(&tx, &canonical.spec.filters)?;
        let defer_count = cursor.as_ref().is_some_and(|cursor| cursor.total_count < 0)
            || (cursor.is_none()
                && query_supports_deferred_count(&canonical.spec)
                && active_library_rows_exceed_deferred_threshold(&tx)?);
        let total_count = if defer_count {
            None
        } else if let Some(cached) = self.cached_library_count(revision, &count_key) {
            Some(cached)
        } else {
            let (count_sql, count_params) =
                build_library_count_query(&tx, &canonical, &scope, tag_invalid)?;
            let total_count: i64 =
                tx.query_row(&count_sql, params_from_iter(count_params.iter()), |row| {
                    row.get(0)
                })?;
            self.cache_library_count(revision, count_key, total_count);
            Some(total_count)
        };
        if let Some(cursor) = cursor.as_ref() {
            validate_cursor_authority(&tx, &canonical, &scope, cursor, tag_invalid, total_count)?;
        }
        let parts = build_query_parts(
            &tx,
            &canonical,
            &scope,
            cursor.as_ref(),
            tag_invalid,
            true,
            !defer_count,
        )?;
        let row_sql = format!(
            "{} SELECT f.id, f.name, f.path, f.extension, f.size, f.mtime, f.ctime, f.is_dir, f.file_type, f.purpose, f.lifecycle, f.risk_level, f.confidence, (EXISTS (SELECT 1 FROM active_duplicate_membership AS adm WHERE adm.file_id = f.id)) AS is_duplicate, f.requires_confirmation, f.is_stale, {}, (SELECT COALESCE(json_group_array(json_object('id', t.id, 'displayName', t.display_name, 'colorToken', t.color_token)), '[]') FROM (SELECT t.id, t.display_name, t.color_token FROM file_user_tags AS fut JOIN user_tags AS t ON t.id = fut.tag_id WHERE fut.file_id = f.id ORDER BY t.normalized_name COLLATE NOCASE, t.id LIMIT {}) AS t), (SELECT COUNT(*) FROM file_user_tags AS fut WHERE fut.file_id = f.id) FROM {} WHERE {} ORDER BY {} LIMIT ?",
            parts.ctes,
            if canonical.spec.text.is_some() { "fm.rank" } else { "NULL" },
            LIBRARY_TAG_PREVIEW_LIMIT,
            parts.from,
            parts.where_clause,
            parts.order,
        );
        let mut row_params = parts.params.clone();
        row_params.push(SqlValue::Integer(i64::from(request.page_size) + 1));
        let mut summaries = {
            let mut stmt = tx.prepare(&row_sql)?;
            let rows = stmt.query_map(params_from_iter(row_params.iter()), summary_from_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let has_more = summaries.len() > usize::try_from(request.page_size).unwrap_or(0);
        if has_more {
            summaries.truncate(usize::try_from(request.page_size).unwrap_or(0));
        }
        let next_cursor = summaries.last().and_then(|summary| {
            has_more.then(|| {
                encode_cursor(&cursor_for_summary(
                    summary,
                    &canonical,
                    revision,
                    total_count.unwrap_or(-1),
                ))
            })
        });
        let result_state = if tag_invalid || scope.health.state != "healthy" {
            "partial"
        } else if summaries.is_empty() {
            "empty"
        } else {
            "complete"
        };
        let response_scope_health = if tag_invalid {
            let mut health = scope.health;
            health.state = "invalid_reference".to_string();
            health
                .invalid_references
                .extend(missing_tag_ids(&tx, &canonical.spec.filters)?);
            health
        } else {
            scope.health
        };
        tx.commit()?;
        // Native Foundation/statfs calls are intentionally outside both the
        // SQLite transaction and the pooled connection lease. They are
        // bounded, read-only projections and never become query authority.
        drop(conn);
        for summary in &mut summaries {
            // `summary_from_row` retains the durable path in a non-serialized
            // field so enrichment can happen after the connection is
            // released without changing the public DTO contract.
            if let Some(path) = summary.native_path.take() {
                summary.native_semantics = native_semantics_for_path(&path);
            }
        }
        Ok(FileQueryResponseV2 {
            version: LIBRARY_QUERY_VERSION,
            request_id: request.request_id,
            query_fingerprint: fingerprint,
            snapshot_revision: revision,
            files: summaries,
            total_count,
            count_state: if total_count.is_some() {
                "exact".to_string()
            } else {
                "deferred".to_string()
            },
            count_token: total_count.is_none().then(|| {
                encode_count_token(&LibraryCountToken {
                    version: LIBRARY_QUERY_VERSION,
                    query: canonical.spec.clone(),
                    fingerprint: canonical.fingerprint.clone(),
                    membership_fingerprint: membership_fingerprint(&canonical.spec)
                        .unwrap_or_default(),
                    revision,
                })
            }),
            next_cursor,
            has_more,
            result_state: result_state.to_string(),
            scope_health: response_scope_health,
        })
    }

    pub fn resolve_file_library_exact_count_v2(
        &self,
        request: ResolveFileLibraryExactCountRequestV2,
    ) -> Result<ResolveFileLibraryExactCountResponseV2, DbError> {
        if request.version != LIBRARY_QUERY_VERSION
            || request.request_id.trim().is_empty()
            || request.request_id.chars().count() > 128
            || request.count_token.len() > 64_000
        {
            return Err(DbError::Validation(
                "library_count_request_invalid".to_string(),
            ));
        }
        let token = decode_count_token(&request.count_token)?;
        if token.version != LIBRARY_QUERY_VERSION {
            return Err(DbError::Validation(
                "library_count_token_invalid".to_string(),
            ));
        }
        let (spec, _json, fingerprint) = canonicalize_file_query_spec(token.query)?;
        let membership = membership_fingerprint(&spec)?;
        if fingerprint != token.fingerprint || membership != token.membership_fingerprint {
            return Err(DbError::Validation(
                "library_count_token_binding_invalid".to_string(),
            ));
        }
        let canonical = CanonicalQuery { spec, fingerprint };
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let revision = current_library_revision(&tx)?;
        if revision != token.revision {
            return Err(DbError::Validation("library_snapshot_expired".to_string()));
        }
        let scope = resolve_scope(&tx, &canonical.spec.scope)?;
        if scope.health.state != "healthy" || query_has_missing_tags(&tx, &canonical.spec.filters)?
        {
            return Err(DbError::Validation(
                "library_count_scope_unavailable".to_string(),
            ));
        }
        let (sql, sql_params) = build_library_count_query(&tx, &canonical, &scope, false)?;
        let total_count =
            tx.query_row(&sql, params_from_iter(sql_params.iter()), |row| row.get(0))?;
        self.cache_library_count(
            revision,
            membership_fingerprint(&canonical.spec)?,
            total_count,
        );
        tx.commit()?;
        Ok(ResolveFileLibraryExactCountResponseV2 {
            version: LIBRARY_QUERY_VERSION,
            request_id: request.request_id,
            query_fingerprint: canonical.fingerprint,
            snapshot_revision: revision,
            total_count,
            count_state: "exact".to_string(),
        })
    }

    pub fn get_file_library_detail(&self, file_id: &str) -> Result<FileLibraryDetailDto, DbError> {
        let file_id = validate_file_id(file_id)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let revision = current_library_revision(&tx)?;
        let row = {
            let mut stmt = tx.prepare(
                "SELECT id, name, path, extension, size, mtime, ctime, is_dir, file_type, purpose, lifecycle, context, risk_level, confidence, classification_status, classification_reason, matched_rules, suggested_action, suggested_target_path, suggested_name, (EXISTS (SELECT 1 FROM active_duplicate_membership AS adm WHERE adm.file_id = files.id)), requires_confirmation, is_stale, last_seen_at FROM files WHERE id = ?1",
            )?;
            stmt.query_row(params![file_id], detail_from_row)
                .optional()?
        };
        let Some(mut detail) = row else {
            return Err(DbError::Validation("library_file_not_found".to_string()));
        };
        detail.tags = load_file_tags(&tx, &file_id)?;
        detail.active_findings = load_active_finding_summaries(&tx, &file_id)?;
        // Resolve the root with a bounded Rust loop over the small root ledger.
        let root = find_root_for_path(&tx, &detail.path)?;
        detail.scan_root_id = root.as_ref().map(|(id, _, _)| id.clone());
        detail.scan_root_name = root.as_ref().map(|(_, name, _)| name.clone());
        detail.scope_health = root.as_ref().map(|(_, _, health)| health.clone());
        if let Some(content) = tx
            .query_row(
                "SELECT ca.status, csp.enabled, ca.summary, ca.keywords_json,
                        ca.language, ca.provenance_json, ca.truncated,
                        ca.text_retained, ca.revision
                 FROM content_artifacts ca
                 LEFT JOIN content_scope_policies csp ON csp.scan_root_id = ca.scan_root_id
                 WHERE ca.file_id = ?1",
                params![file_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                },
            )
            .optional()?
        {
            detail.content_status = Some(content.0);
            detail.content_policy = Some(if content.1.unwrap_or(0) != 0 {
                "enabled".to_string()
            } else {
                "disabled".to_string()
            });
            detail.content_summary = content.2;
            detail.content_keywords = serde_json::from_str(&content.3).unwrap_or_default();
            detail.content_language = content.4;
            detail.content_provenance = Some(content.5);
            detail.content_truncated = Some(content.6 != 0);
            detail.content_text_retained = Some(content.7 != 0);
            detail.content_revision = Some(content.8);
        }
        let duplicate = tx
            .query_row(
                "SELECT ag.group_id, (SELECT COUNT(*) FROM duplicate_group_members AS agm WHERE agm.group_id = ag.group_id) FROM active_duplicate_membership AS ag JOIN duplicate_group_members AS agm ON agm.group_id = ag.group_id WHERE agm.file_id = ?1 LIMIT 1",
                params![file_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        detail.duplicate_group_id = duplicate.as_ref().map(|(id, _)| id.clone());
        detail.duplicate_group_size = duplicate.map(|(_, count)| count).unwrap_or(0);
        detail.revision = revision;
        tx.commit()?;
        drop(conn);
        detail.native_semantics = native_semantics_for_path(&detail.path);
        Ok(detail)
    }

    pub fn resolve_file_library_path(&self, file_id: &str) -> Result<String, DbError> {
        let file_id = validate_file_id(file_id)?;
        let conn = self.conn()?;
        conn.query_row(
            "SELECT path FROM files WHERE id = ?1 AND is_stale = 0",
            params![file_id],
            |row| row.get(0),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                DbError::Validation("library_file_unavailable".to_string())
            }
            other => DbError::Sqlite(other),
        })
    }

    pub fn get_file_library_selection_summary(
        &self,
        selection: LibrarySelectionV1,
    ) -> Result<FileLibrarySelectionSummaryDto, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let current_revision = current_library_revision(&tx)?;
        let (where_sql, params, missing_count, excluded_count, fingerprint) =
            selection_where(&tx, &selection, current_revision)?;
        let sql = format!(
            "SELECT COUNT(*), COALESCE(SUM(f.size), 0) FROM files AS f WHERE {}",
            where_sql
        );
        let (count, total_size): (i64, i64) =
            tx.query_row(&sql, params_from_iter(params.iter()), |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
        let type_sql = format!(
            "SELECT f.file_type, COUNT(*) FROM files AS f WHERE {} GROUP BY f.file_type ORDER BY f.file_type",
            where_sql
        );
        let type_counts = {
            let mut type_stmt = tx.prepare(&type_sql)?;
            let type_rows = type_stmt.query_map(params_from_iter(params.iter()), |row| {
                Ok(LibraryTypeCountDto {
                    file_type: row.get(0)?,
                    count: row.get(1)?,
                })
            })?;
            type_rows.collect::<Result<Vec<_>, _>>()?
        };
        let (minimum_path, maximum_path): (Option<String>, Option<String>) = tx.query_row(
            &format!("SELECT MIN(f.path), MAX(f.path) FROM files AS f WHERE {where_sql}"),
            params_from_iter(params.iter()),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let common_directory = minimum_path
            .zip(maximum_path)
            .and_then(|(minimum, maximum)| common_directory_for_paths(&minimum, &maximum));
        let tag_sql = format!(
            "SELECT t.id, t.display_name, t.color_token, COUNT(*) \
             FROM files AS f JOIN file_user_tags AS fut ON fut.file_id = f.id \
             JOIN user_tags AS t ON t.id = fut.tag_id WHERE {where_sql} \
             GROUP BY t.id, t.display_name, t.color_token \
             ORDER BY t.normalized_name COLLATE NOCASE, t.id"
        );
        let tag_commonality = {
            let mut stmt = tx.prepare(&tag_sql)?;
            let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
                Ok((
                    UserTagPreviewDto {
                        id: row.get(0)?,
                        display_name: row.get(1)?,
                        color_token: row.get(2)?,
                    },
                    row.get::<_, i64>(3)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let common_tags = tag_commonality
            .iter()
            .filter(|(_, tagged_count)| *tagged_count == count)
            .map(|(tag, _)| tag.clone())
            .collect::<Vec<_>>();
        let common_tag_ids = common_tags.iter().map(|tag| tag.id.clone()).collect();
        let partial_tag_commonality_count = tag_commonality
            .iter()
            .filter(|(_, tagged_count)| *tagged_count > 0 && *tagged_count < count)
            .count() as i64;
        let stale_count = selection_stale_count(&tx, &selection)?;
        clear_temp_selection_ids(&tx)?;
        tx.commit()?;
        Ok(FileLibrarySelectionSummaryDto {
            count,
            total_size,
            type_counts,
            missing_count,
            stale_count,
            excluded_count,
            common_directory,
            common_tags,
            common_tag_ids,
            partial_tag_commonality_count,
            snapshot_revision: current_revision,
            query_fingerprint: fingerprint,
        })
    }
}

fn validate_query_request(request: &FileQueryRequestV2) -> Result<(), DbError> {
    if request.version != LIBRARY_QUERY_VERSION {
        return Err(DbError::Validation(
            "library_query_version_unsupported".to_string(),
        ));
    }
    if request.request_id.trim().is_empty() || request.request_id.chars().count() > 128 {
        return Err(DbError::Validation(
            "library_query_request_id_invalid".to_string(),
        ));
    }
    if request.page_size == 0 || request.page_size > LIBRARY_PAGE_MAX {
        return Err(DbError::Validation(
            "library_query_page_size_invalid".to_string(),
        ));
    }
    if request
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.len() > LIBRARY_CURSOR_MAX_CHARS)
    {
        return Err(DbError::Validation("library_cursor_too_large".to_string()));
    }
    Ok(())
}

pub(crate) fn current_library_revision(conn: &Connection) -> Result<i64, DbError> {
    conn.query_row(
        "SELECT revision FROM library_query_state WHERE singleton_id = 1",
        [],
        |row| row.get(0),
    )
    .map_err(DbError::from)
}

pub(crate) fn resolve_scope(
    conn: &Connection,
    scope: &FileLibraryScopeV2,
) -> Result<ResolvedScope, DbError> {
    let mut roots = Vec::<LibraryScopeRootHealthDto>::new();
    let mut invalid_references = Vec::new();
    let root_ids = match scope {
        FileLibraryScopeV2::AllEnabledRoots => None,
        FileLibraryScopeV2::Roots { scan_root_ids } => Some(scan_root_ids.clone()),
        FileLibraryScopeV2::CurrentScan { scan_session_id } => {
            let exists: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM scan_sessions WHERE id = ?1",
                    params![scan_session_id],
                    |row| row.get(0),
                )
                .optional()?;
            if exists.is_none() {
                return Err(DbError::Validation(
                    "library_scope_invalid:missing_session".to_string(),
                ));
            }
            let mut stmt = conn.prepare("SELECT effective_root_id FROM scan_session_roots WHERE session_id = ?1 AND resolution = 'effective' AND effective_root_id IS NOT NULL ORDER BY effective_index")?;
            let ids = stmt
                .query_map(params![scan_session_id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Some(ids)
        }
    };
    let roots_sql = match root_ids.as_ref() {
        None => "SELECT id, display_name, health_status, enabled, current_generation,
                        needs_reconciliation, watcher_rule_recovery_required,
                        watcher_revision, watcher_applied_revision
                 FROM scan_roots
                 WHERE source_kind = 'file_library' AND enabled = 1
                 ORDER BY id"
            .to_string(),
        Some(ids) if ids.is_empty() => "SELECT id, display_name, health_status, enabled,
                        current_generation, needs_reconciliation,
                        watcher_rule_recovery_required, watcher_revision,
                        watcher_applied_revision
                 FROM scan_roots WHERE 0"
            .to_string(),
        Some(ids) => format!(
            "SELECT id, display_name, health_status, enabled, current_generation,
                    needs_reconciliation, watcher_rule_recovery_required,
                    watcher_revision, watcher_applied_revision
             FROM scan_roots
             WHERE source_kind = 'file_library' AND id IN ({})
             ORDER BY id",
            std::iter::repeat_n("?", ids.len())
                .collect::<Vec<_>>()
                .join(",")
        ),
    };
    let root_params = root_ids
        .as_ref()
        .map(|ids| ids.iter().cloned().map(SqlValue::Text).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut seen = HashSet::new();
    let mut stmt = conn.prepare(&roots_sql)?;
    let rows = stmt.query_map(params_from_iter(root_params.iter()), |row| {
        let id: String = row.get(0)?;
        let display_name: String = row.get(1)?;
        let health_status: String = row.get(2)?;
        let enabled = row.get::<_, i64>(3)? != 0;
        let generation = row.get(4)?;
        let needs_reconciliation = row.get::<_, i64>(5)? != 0;
        let watcher_rule_recovery_required = row.get::<_, i64>(6)? != 0;
        let watcher_revision = row.get::<_, i64>(7)?;
        let watcher_applied_revision = row.get::<_, i64>(8)?;
        Ok((
            id,
            display_name,
            health_status,
            enabled,
            generation,
            needs_reconciliation,
            watcher_rule_recovery_required,
            watcher_revision,
            watcher_applied_revision,
        ))
    })?;
    for row in rows {
        let (
            id,
            display_name,
            health_status,
            enabled,
            generation,
            needs_reconciliation,
            watcher_rule_recovery_required,
            watcher_revision,
            watcher_applied_revision,
        ) = row?;
        seen.insert(id.clone());
        let available = enabled
            && health_status == "healthy"
            && !needs_reconciliation
            && !watcher_rule_recovery_required
            && watcher_revision == watcher_applied_revision;
        roots.push(LibraryScopeRootHealthDto {
            id,
            display_name,
            health_status: health_status.clone(),
            enabled,
            available,
            generation,
            message: (!available).then(|| "library_scope_root_unavailable".to_string()),
        });
    }
    if let Some(ids) = root_ids.as_ref() {
        for id in ids {
            if !seen.contains(id) {
                invalid_references.push(id.clone());
            }
        }
    }
    let state = if !invalid_references.is_empty() {
        "invalid_reference"
    } else if roots.iter().any(|root| !root.available) {
        "partial"
    } else if roots.is_empty() {
        "empty"
    } else {
        "healthy"
    };
    let available_roots = roots
        .iter()
        .filter(|root| root.available)
        .collect::<Vec<_>>();
    let mut clauses = Vec::new();
    let mut params = Vec::new();
    for root in available_roots {
        let path: String = conn.query_row(
            "SELECT normalized_path FROM scan_roots WHERE id = ?1",
            params![root.id],
            |row| row.get(0),
        )?;
        let path = normalize_path_text(&path);
        let escaped = escape_like_pattern(&path);
        clauses.push(
            "(f.path = ? OR f.path LIKE ? ESCAPE '~' OR f.path LIKE ? ESCAPE '~')".to_string(),
        );
        params.push(SqlValue::Text(path));
        params.push(SqlValue::Text(format!("{escaped}/%")));
        params.push(SqlValue::Text(format!("{escaped}\\%")));
    }
    let clause = if clauses.is_empty() {
        "1 = 0".to_string()
    } else {
        clauses.join(" OR ")
    };
    Ok(ResolvedScope {
        clause,
        params,
        health: LibraryScopeHealthDto {
            state: state.to_string(),
            roots,
            invalid_references,
            message: None,
        },
    })
}

/// Revalidates both current managed-root authority and current metadata
/// membership. This intentionally uses a fresh canonical query rather than a
/// historical materialized item list.
pub(crate) fn file_matches_authoritative_query_scope(
    conn: &Connection,
    query: &FileQuerySpecV2,
    file_id: &str,
) -> Result<bool, DbError> {
    Ok(
        files_matching_authoritative_query_scope(conn, query, &[file_id.to_string()])?
            .contains(file_id),
    )
}

/// Revalidates a bounded set of file IDs against one canonical managed-library
/// scope in a single authoritative query. Organization Plan projection uses
/// this to avoid resolving the same root/filter authority once per member.
pub(crate) fn files_matching_authoritative_query_scope(
    conn: &Connection,
    query: &FileQuerySpecV2,
    file_ids: &[String],
) -> Result<HashSet<String>, DbError> {
    if file_ids.is_empty() {
        return Ok(HashSet::new());
    }
    let (spec, _, _) = canonicalize_file_query_spec(query.clone())?;
    let scope = resolve_scope(conn, &spec.scope)?;
    if scope.health.state == "invalid_reference" {
        return Err(DbError::Validation(
            "library_scope_invalid:reference".to_string(),
        ));
    }
    if scope.health.state != "healthy" {
        return Err(DbError::Validation("library_scope_unavailable".to_string()));
    }
    if query_has_missing_tags(conn, &spec.filters)? {
        return Err(DbError::Validation(
            "library_selection_invalid_tag_reference".to_string(),
        ));
    }
    let placeholders = std::iter::repeat_n("?", file_ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let mut conditions = vec![
        format!("f.id IN ({placeholders})"),
        "f.is_stale = 0".to_string(),
    ];
    let mut query_params = file_ids
        .iter()
        .cloned()
        .map(SqlValue::Text)
        .collect::<Vec<_>>();
    if let Some(text) = spec.text.as_deref() {
        let fts_query = build_fts_query(text)
            .ok_or_else(|| DbError::Validation("library_query_text_invalid".to_string()))?;
        conditions.push(
            "f.rowid IN (SELECT files_fts.rowid FROM files_fts WHERE files_fts MATCH ?)"
                .to_string(),
        );
        query_params.push(SqlValue::Text(fts_query));
    }
    conditions.push(format!("({})", scope.clause));
    query_params.extend(scope.params);
    append_filters(conn, &mut conditions, &mut query_params, &spec.filters)?;
    let sql = format!(
        "SELECT f.id FROM files AS f WHERE {}",
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(query_params.iter()), |row| row.get(0))?;
    rows.collect::<Result<HashSet<String>, _>>()
        .map_err(DbError::from)
}

fn build_query_parts(
    conn: &Connection,
    canonical: &CanonicalQuery,
    scope: &ResolvedScope,
    cursor: Option<&LibraryCursor>,
    tag_invalid: bool,
    include_rank: bool,
    materialize_fts: bool,
) -> Result<QueryParts, DbError> {
    let mut ctes = String::new();
    let mut params = Vec::new();
    let from = if let Some(text) = canonical.spec.text.as_deref() {
        let Some(fts_query) = build_fts_query(text) else {
            return Err(DbError::Validation(
                "library_query_text_invalid".to_string(),
            ));
        };
        let rank_projection = if include_rank {
            ", bm25(files_fts, 6.0, 1.5) AS rank"
        } else {
            ""
        };
        let materialization = if materialize_fts {
            "MATERIALIZED"
        } else {
            "NOT MATERIALIZED"
        };
        ctes = format!(
            "WITH fts_matches AS {materialization} (SELECT files_fts.rowid{rank_projection} FROM files_fts WHERE files_fts MATCH ?)"
        );
        params.push(SqlValue::Text(fts_query));
        "files AS f JOIN fts_matches AS fm ON fm.rowid = f.rowid".to_string()
    } else {
        "files AS f".to_string()
    };
    let mut conditions = vec!["f.is_stale = 0".to_string(), format!("({})", scope.clause)];
    params.extend(scope.params.clone());
    append_filters(conn, &mut conditions, &mut params, &canonical.spec.filters)?;
    if tag_invalid {
        conditions.push("1 = 0".to_string());
    }
    if let Some(cursor) = cursor {
        append_cursor_condition(&mut conditions, &mut params, cursor, &canonical.spec.sort)?;
    }
    let order = order_clause(&canonical.spec.sort, canonical.spec.text.is_some());
    Ok(QueryParts {
        ctes,
        from,
        where_clause: conditions.join(" AND "),
        params,
        order,
    })
}

fn build_library_count_query(
    conn: &Connection,
    canonical: &CanonicalQuery,
    scope: &ResolvedScope,
    tag_invalid: bool,
) -> Result<(String, Vec<SqlValue>), DbError> {
    let any_tags = &canonical.spec.filters.tags_any_of;
    let none_tags = &canonical.spec.filters.tags_none_of;
    if !tag_invalid
        && canonical.spec.text.is_none()
        && (!any_tags.is_empty() || !none_tags.is_empty())
    {
        // Positive and negative tag predicates can use the file primary key for
        // duplicate-safe existence checks per candidate. The page query still
        // scans in sort order so keyset pagination keeps its stop-at-page-size
        // behavior; only the exact first-page count uses this plan.
        let mut count_spec = canonical.spec.clone();
        count_spec.filters.tags_any_of.clear();
        count_spec.filters.tags_none_of.clear();
        let count_canonical = CanonicalQuery {
            spec: count_spec,
            fingerprint: String::new(),
        };
        let parts = build_query_parts(conn, &count_canonical, scope, None, false, false, true)?;
        let any_placeholders = std::iter::repeat_n("?", any_tags.len())
            .collect::<Vec<_>>()
            .join(",");
        let none_placeholders = std::iter::repeat_n("?", none_tags.len())
            .collect::<Vec<_>>()
            .join(",");
        let mut tag_conditions = Vec::new();
        if !any_tags.is_empty() {
            tag_conditions.push(format!(
                "EXISTS (SELECT 1 FROM file_user_tags AS tf_any_count WHERE tf_any_count.file_id = f.id AND tf_any_count.tag_id IN ({any_placeholders}))"
            ));
        }
        if !none_tags.is_empty() {
            tag_conditions.push(format!(
                "f.id NOT IN (SELECT tf_none_count.file_id FROM file_user_tags AS tf_none_count INDEXED BY idx_file_user_tags_tag_file WHERE tf_none_count.tag_id IN ({none_placeholders}))"
            ));
        }
        let sql = format!(
            "{} SELECT COUNT(*) FROM {} WHERE {} AND {}",
            parts.ctes,
            parts.from,
            parts.where_clause,
            tag_conditions.join(" AND ")
        );
        let mut params = parts.params;
        params.extend(any_tags.iter().cloned().map(SqlValue::Text));
        params.extend(none_tags.iter().cloned().map(SqlValue::Text));
        return Ok((sql, params));
    }

    let parts = build_query_parts(conn, canonical, scope, None, tag_invalid, false, true)?;
    let sql = format!(
        "{} SELECT COUNT(*) FROM {} WHERE {}",
        parts.ctes, parts.from, parts.where_clause
    );
    Ok((sql, parts.params))
}

pub(super) fn append_filters(
    conn: &Connection,
    conditions: &mut Vec<String>,
    params: &mut Vec<SqlValue>,
    filters: &FileQueryFiltersV2,
) -> Result<(), DbError> {
    append_in_filter(conditions, params, "f.file_type", &filters.file_types);
    append_in_filter(conditions, params, "f.purpose", &filters.purposes);
    append_in_filter(conditions, params, "f.lifecycle", &filters.lifecycles);
    append_in_filter(conditions, params, "f.risk_level", &filters.risks);
    if let Some(min) = filters.size_min {
        conditions.push("f.size >= ?".to_string());
        params.push(SqlValue::Integer(min));
    }
    if let Some(max) = filters.size_max {
        conditions.push("f.size <= ?".to_string());
        params.push(SqlValue::Integer(max));
    }
    if let Some(min) = filters.modified_from {
        conditions.push("f.mtime >= ?".to_string());
        params.push(SqlValue::Integer(min));
    }
    if let Some(max) = filters.modified_to {
        conditions.push("f.mtime <= ?".to_string());
        params.push(SqlValue::Integer(max));
    }
    if let Some(min) = filters.created_from {
        conditions.push("f.ctime >= ?".to_string());
        params.push(SqlValue::Integer(min));
    }
    if let Some(max) = filters.created_to {
        conditions.push("f.ctime <= ?".to_string());
        params.push(SqlValue::Integer(max));
    }
    append_match_filter(
        conditions,
        filters.duplicate.clone(),
        "EXISTS (SELECT 1 FROM active_duplicate_membership AS adm WHERE adm.file_id = f.id)",
    );
    append_match_filter(
        conditions,
        filters.review.clone(),
        "(f.requires_confirmation = 1 OR f.suggested_action IN ('Review', 'DeleteCandidate'))",
    );
    append_tag_filter(conn, conditions, params, "all", &filters.tags_all_of)?;
    append_tag_filter(conn, conditions, params, "any", &filters.tags_any_of)?;
    append_tag_filter(conn, conditions, params, "none", &filters.tags_none_of)?;
    Ok(())
}

fn append_in_filter(
    conditions: &mut Vec<String>,
    params: &mut Vec<SqlValue>,
    column: &str,
    values: &[String],
) {
    if values.is_empty() {
        return;
    }
    conditions.push(format!(
        "{column} IN ({})",
        std::iter::repeat_n("?", values.len())
            .collect::<Vec<_>>()
            .join(",")
    ));
    params.extend(values.iter().cloned().map(SqlValue::Text));
}

fn append_match_filter(conditions: &mut Vec<String>, mode: LibraryMatchMode, predicate: &str) {
    match mode {
        LibraryMatchMode::Any => {}
        LibraryMatchMode::Only => conditions.push(predicate.to_string()),
        LibraryMatchMode::Exclude => conditions.push(format!("NOT ({predicate})")),
    }
}

fn append_tag_filter(
    conn: &Connection,
    conditions: &mut Vec<String>,
    params: &mut Vec<SqlValue>,
    kind: &str,
    ids: &[String],
) -> Result<(), DbError> {
    if ids.is_empty() {
        return Ok(());
    }
    for id in ids {
        if !tag_exists(conn, id)? {
            return Ok(());
        }
    }
    match kind {
        "all" => {
            for id in ids {
                conditions.push(
                    "EXISTS (SELECT 1 FROM file_user_tags AS tf_all \
                     WHERE tf_all.file_id = f.id AND tf_all.tag_id = ?)"
                        .to_string(),
                );
                params.push(SqlValue::Text(id.clone()));
            }
        }
        "any" => {
            conditions.push(format!(
                "EXISTS (SELECT 1 FROM file_user_tags AS tf_any \
                 WHERE tf_any.file_id = f.id AND tf_any.tag_id IN ({}))",
                std::iter::repeat_n("?", ids.len())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
            params.extend(ids.iter().cloned().map(SqlValue::Text));
        }
        "none" => {
            conditions.push(format!(
                "NOT EXISTS (SELECT 1 FROM file_user_tags AS tf_none INDEXED BY idx_file_user_tags_tag_file \
                 WHERE tf_none.file_id = f.id AND tf_none.tag_id IN ({}))",
                std::iter::repeat_n("?", ids.len())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
            params.extend(ids.iter().cloned().map(SqlValue::Text));
        }
        _ => unreachable!(),
    }
    Ok(())
}

pub(super) fn query_has_missing_tags(
    conn: &Connection,
    filters: &FileQueryFiltersV2,
) -> Result<bool, DbError> {
    for id in filters
        .tags_all_of
        .iter()
        .chain(filters.tags_any_of.iter())
        .chain(filters.tags_none_of.iter())
    {
        if !tag_exists(conn, id)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn missing_tag_ids(
    conn: &Connection,
    filters: &FileQueryFiltersV2,
) -> Result<Vec<String>, DbError> {
    let mut ids = filters
        .tags_all_of
        .iter()
        .chain(filters.tags_any_of.iter())
        .chain(filters.tags_none_of.iter())
        .cloned()
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    let mut missing = Vec::new();
    for id in ids {
        if !tag_exists(conn, &id)? {
            missing.push(id);
        }
    }
    Ok(missing)
}

pub(super) fn order_clause(sort: &FileLibrarySortV2, has_text: bool) -> String {
    let direction = match sort.direction {
        LibrarySortDirection::Asc => "ASC",
        LibrarySortDirection::Desc => "DESC",
    };
    match sort.kind {
        LibrarySortKind::Relevance if has_text => {
            format!("fm.rank {direction}, f.mtime DESC, f.name COLLATE NOCASE ASC, f.id ASC")
        }
        LibrarySortKind::Modified => format!("f.mtime {direction}, f.id ASC"),
        LibrarySortKind::Created => format!("f.ctime {direction}, f.id ASC"),
        LibrarySortKind::Name => format!("f.name COLLATE NOCASE {direction}, f.id ASC"),
        LibrarySortKind::Size => format!("f.size {direction}, f.id ASC"),
        LibrarySortKind::Confidence => format!("f.confidence {direction}, f.id ASC"),
        LibrarySortKind::Relevance => "f.mtime DESC, f.id ASC".to_string(),
    }
}

pub(super) fn append_cursor_condition(
    conditions: &mut Vec<String>,
    params: &mut Vec<SqlValue>,
    cursor: &LibraryCursor,
    sort: &FileLibrarySortV2,
) -> Result<(), DbError> {
    let descending = matches!(sort.direction, LibrarySortDirection::Desc);
    let (column, value) = match sort.kind {
        LibrarySortKind::Relevance => {
            let rank_bits = cursor
                .last_rank_bits
                .ok_or_else(|| DbError::Validation("library_cursor_incomplete".to_string()))?;
            let rank = f64::from_bits(rank_bits);
            params.push(SqlValue::Real(rank));
            params.push(SqlValue::Real(rank));
            params.push(SqlValue::Integer(cursor.last_mtime.unwrap_or(0)));
            params.push(SqlValue::Integer(cursor.last_mtime.unwrap_or(0)));
            params.push(SqlValue::Text(cursor.last_text.clone().unwrap_or_default()));
            params.push(SqlValue::Text(cursor.last_text.clone().unwrap_or_default()));
            params.push(SqlValue::Text(cursor.file_id.clone()));
            let rank_op = if descending { "<" } else { ">" };
            conditions.push(format!("(fm.rank {rank_op} ? OR (fm.rank = ? AND (f.mtime < ? OR (f.mtime = ? AND (f.name COLLATE NOCASE > ? OR (f.name COLLATE NOCASE = ? AND f.id > ?))))))"));
            return Ok(());
        }
        LibrarySortKind::Modified => ("f.mtime", SqlValue::Integer(cursor.last_i64.unwrap_or(0))),
        LibrarySortKind::Created => ("f.ctime", SqlValue::Integer(cursor.last_i64.unwrap_or(0))),
        LibrarySortKind::Name => (
            "f.name COLLATE NOCASE",
            SqlValue::Text(cursor.last_text.clone().unwrap_or_default()),
        ),
        LibrarySortKind::Size => ("f.size", SqlValue::Integer(cursor.last_i64.unwrap_or(0))),
        LibrarySortKind::Confidence => (
            "f.confidence",
            SqlValue::Real(f64::from_bits(cursor.last_f64_bits.unwrap_or(0))),
        ),
    };
    let op = if descending { "<" } else { ">" };
    conditions.push(format!(
        "(({column} {op} ? ) OR ({column} = ? AND f.id > ?))"
    ));
    params.push(value.clone());
    params.push(value);
    params.push(SqlValue::Text(cursor.file_id.clone()));
    Ok(())
}

fn native_semantics_for_path(path: &str) -> Option<FileLibraryNativeSemanticsDto> {
    #[cfg(target_os = "macos")]
    {
        let semantics = crate::platform::macos::file_semantics::inspect(std::path::Path::new(path));
        Some(FileLibraryNativeSemanticsDto {
            is_package: semantics.is_package,
            cloud_backing: semantics.backing_kind.as_str().to_string(),
            content_availability: semantics.content_availability.as_str().to_string(),
        })
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        None
    }
}

fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<FileLibrarySummaryDto> {
    let tags_json: String = row.get(17)?;
    let path: String = row.get(2)?;
    Ok(FileLibrarySummaryDto {
        id: row.get(0)?,
        name: row.get(1)?,
        extension: row.get(3)?,
        display_directory: parent_directory(&path),
        size: row.get(4)?,
        modified_at: row.get(5)?,
        created_at: row.get(6)?,
        is_directory: row.get::<_, i64>(7)? != 0,
        file_type: row.get(8)?,
        purpose: row.get(9)?,
        lifecycle: row.get(10)?,
        risk: row.get(11)?,
        confidence: row.get(12)?,
        is_duplicate: row.get::<_, i64>(13)? != 0,
        requires_review: row.get::<_, i64>(14)? != 0,
        is_stale: row.get::<_, i64>(15)? != 0,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        tag_count: row.get(18)?,
        native_semantics: None,
        rank: row.get(16)?,
        native_path: Some(path),
    })
}

fn cursor_for_summary(
    summary: &FileLibrarySummaryDto,
    canonical: &CanonicalQuery,
    revision: i64,
    total_count: i64,
) -> LibraryCursor {
    let mut cursor = LibraryCursor {
        version: LIBRARY_CURSOR_VERSION,
        fingerprint: canonical.fingerprint.clone(),
        revision,
        total_count,
        sort_kind: canonical.spec.sort.kind.clone(),
        direction: canonical.spec.sort.direction.clone(),
        file_id: summary.id.clone(),
        last_i64: None,
        last_text: None,
        last_f64_bits: None,
        last_rank_bits: None,
        last_mtime: None,
    };
    match canonical.spec.sort.kind {
        LibrarySortKind::Modified => cursor.last_i64 = Some(summary.modified_at),
        LibrarySortKind::Created => cursor.last_i64 = Some(summary.created_at),
        LibrarySortKind::Name => cursor.last_text = Some(summary.name.clone()),
        LibrarySortKind::Size => cursor.last_i64 = Some(summary.size),
        LibrarySortKind::Confidence => cursor.last_f64_bits = Some(summary.confidence.to_bits()),
        LibrarySortKind::Relevance => {
            cursor.last_rank_bits = Some(summary.rank.unwrap_or(0.0).to_bits());
            cursor.last_mtime = Some(summary.modified_at);
            cursor.last_text = Some(summary.name.clone());
        }
    }
    cursor
}

fn validate_cursor_binding(
    cursor: &LibraryCursor,
    canonical: &CanonicalQuery,
) -> Result<(), DbError> {
    if cursor
        .last_text
        .as_ref()
        .is_some_and(|value| value.chars().count() > 512 || value.chars().any(|ch| ch.is_control()))
        || cursor
            .last_f64_bits
            .is_some_and(|bits| !f64::from_bits(bits).is_finite())
        || cursor
            .last_rank_bits
            .is_some_and(|bits| !f64::from_bits(bits).is_finite())
        || cursor.total_count < -1
    {
        return Err(DbError::Validation(
            "library_cursor_binding_invalid".to_string(),
        ));
    }
    if cursor.version != LIBRARY_CURSOR_VERSION
        || cursor.fingerprint != canonical.fingerprint
        || cursor.sort_kind != canonical.spec.sort.kind
        || cursor.direction != canonical.spec.sort.direction
        || validate_file_id(&cursor.file_id).is_err()
        || match canonical.spec.sort.kind {
            LibrarySortKind::Modified | LibrarySortKind::Created | LibrarySortKind::Size => {
                cursor.last_i64.is_none()
                    || cursor.last_text.is_some()
                    || cursor.last_f64_bits.is_some()
                    || cursor.last_rank_bits.is_some()
                    || cursor.last_mtime.is_some()
            }
            LibrarySortKind::Name => {
                cursor.last_text.is_none()
                    || cursor.last_i64.is_some()
                    || cursor.last_f64_bits.is_some()
                    || cursor.last_rank_bits.is_some()
                    || cursor.last_mtime.is_some()
            }
            LibrarySortKind::Confidence => {
                cursor.last_f64_bits.is_none()
                    || cursor.last_i64.is_some()
                    || cursor.last_text.is_some()
                    || cursor.last_rank_bits.is_some()
                    || cursor.last_mtime.is_some()
            }
            LibrarySortKind::Relevance => {
                cursor.last_rank_bits.is_none()
                    || cursor.last_mtime.is_none()
                    || cursor.last_text.is_none()
                    || cursor.last_i64.is_some()
                    || cursor.last_f64_bits.is_some()
            }
        }
    {
        return Err(DbError::Validation(
            "library_cursor_binding_invalid".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LibraryCountToken {
    version: i32,
    query: FileQuerySpecV2,
    fingerprint: String,
    membership_fingerprint: String,
    revision: i64,
}

fn validate_cursor_authority(
    conn: &Connection,
    canonical: &CanonicalQuery,
    scope: &ResolvedScope,
    cursor: &LibraryCursor,
    tag_invalid: bool,
    authoritative_total_count: Option<i64>,
) -> Result<(), DbError> {
    if cursor.total_count >= 0 && Some(cursor.total_count) != authoritative_total_count {
        return Err(DbError::Validation(
            "library_cursor_authority_mismatch".to_string(),
        ));
    }
    let parts = build_query_parts(
        conn,
        canonical,
        scope,
        None,
        tag_invalid,
        true,
        cursor.total_count >= 0,
    )?;
    let sql = format!(
        "{} SELECT f.mtime, f.ctime, f.name, f.size, f.confidence, {} \
         FROM {} WHERE {} AND f.id = ? LIMIT 1",
        parts.ctes,
        if canonical.spec.text.is_some() {
            "fm.rank"
        } else {
            "NULL"
        },
        parts.from,
        parts.where_clause,
    );
    let mut query_params = parts.params;
    query_params.push(SqlValue::Text(cursor.file_id.clone()));
    let tuple = conn
        .query_row(&sql, params_from_iter(query_params.iter()), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, Option<f64>>(5)?,
            ))
        })
        .optional()?
        .ok_or_else(|| DbError::Validation("library_cursor_anchor_missing".to_string()))?;
    let valid = match canonical.spec.sort.kind {
        LibrarySortKind::Modified => cursor.last_i64 == Some(tuple.0),
        LibrarySortKind::Created => cursor.last_i64 == Some(tuple.1),
        LibrarySortKind::Name => cursor.last_text.as_deref() == Some(tuple.2.as_str()),
        LibrarySortKind::Size => cursor.last_i64 == Some(tuple.3),
        LibrarySortKind::Confidence => cursor.last_f64_bits == Some(tuple.4.to_bits()),
        LibrarySortKind::Relevance => {
            cursor.last_rank_bits == tuple.5.map(f64::to_bits)
                && cursor.last_mtime == Some(tuple.0)
                && cursor.last_text.as_deref() == Some(tuple.2.as_str())
        }
    };
    if !valid {
        return Err(DbError::Validation(
            "library_cursor_tuple_mismatch".to_string(),
        ));
    }
    Ok(())
}

fn encode_cursor(cursor: &LibraryCursor) -> String {
    let bytes = serde_json::to_vec(cursor).unwrap_or_default();
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push_str(&format!("{byte:02x}"));
    }
    encoded
}

fn query_supports_deferred_count(spec: &FileQuerySpecV2) -> bool {
    let filters = &spec.filters;
    let dimensions = [
        !filters.file_types.is_empty(),
        !filters.purposes.is_empty(),
        !filters.lifecycles.is_empty(),
        !filters.risks.is_empty(),
        filters.size_min.is_some() || filters.size_max.is_some(),
        filters.modified_from.is_some() || filters.modified_to.is_some(),
        filters.created_from.is_some() || filters.created_to.is_some(),
        !matches!(&filters.duplicate, LibraryMatchMode::Any),
        !matches!(&filters.review, LibraryMatchMode::Any),
    ]
    .into_iter()
    .filter(|active| *active)
    .count();
    spec.text.is_some()
        || !filters.tags_all_of.is_empty()
        || !filters.tags_any_of.is_empty()
        || !filters.tags_none_of.is_empty()
        || dimensions >= 2
}

fn active_library_rows_exceed_deferred_threshold(conn: &Connection) -> Result<bool, DbError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM files INDEXED BY idx_library_files_modified \
             WHERE is_stale = 0 LIMIT 1 OFFSET 250000",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

fn encode_count_token(token: &LibraryCountToken) -> String {
    let bytes = serde_json::to_vec(token).unwrap_or_default();
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn decode_count_token(value: &str) -> Result<LibraryCountToken, DbError> {
    if value.is_empty()
        || !value.len().is_multiple_of(2)
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(DbError::Validation(
            "library_count_token_invalid".to_string(),
        ));
    }
    let bytes = (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|_| DbError::Validation("library_count_token_invalid".to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    serde_json::from_slice(&bytes)
        .map_err(|_| DbError::Validation("library_count_token_invalid".to_string()))
}

fn decode_cursor(value: &str) -> Result<LibraryCursor, DbError> {
    if value.is_empty() || value.len() > LIBRARY_CURSOR_MAX_CHARS || !value.len().is_multiple_of(2)
    {
        return Err(DbError::Validation("library_cursor_invalid".to_string()));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks_exact(2) {
        let text = std::str::from_utf8(pair)
            .map_err(|_| DbError::Validation("library_cursor_invalid".to_string()))?;
        bytes.push(
            u8::from_str_radix(text, 16)
                .map_err(|_| DbError::Validation("library_cursor_invalid".to_string()))?,
        );
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| DbError::Validation("library_cursor_invalid".to_string()))
}

fn detail_from_row(row: &Row<'_>) -> rusqlite::Result<FileLibraryDetailDto> {
    let matched_rules: String = row.get(16)?;
    let path: String = row.get(2)?;
    Ok(FileLibraryDetailDto {
        id: row.get(0)?,
        name: row.get(1)?,
        path: path.clone(),
        directory: parent_directory(&path),
        extension: row.get(3)?,
        size: row.get(4)?,
        modified_at: row.get(5)?,
        created_at: row.get(6)?,
        is_directory: row.get::<_, i64>(7)? != 0,
        file_type: row.get(8)?,
        purpose: row.get(9)?,
        lifecycle: row.get(10)?,
        context: row.get(11)?,
        risk: row.get(12)?,
        confidence: row.get(13)?,
        classification_status: row.get(14)?,
        classification_reason: row.get(15)?,
        matched_rules: serde_json::from_str(&matched_rules).unwrap_or_default(),
        suggested_action: row.get(17)?,
        suggested_target_path: row.get(18)?,
        suggested_name: row.get(19)?,
        is_duplicate: row.get::<_, i64>(20)? != 0,
        requires_review: row.get::<_, i64>(21)? != 0,
        is_stale: row.get::<_, i64>(22)? != 0,
        last_seen_at: row.get(23)?,
        scan_root_id: None,
        scan_root_name: None,
        scope_health: None,
        duplicate_group_id: None,
        duplicate_group_size: 0,
        tags: Vec::new(),
        active_findings: Vec::new(),
        safe_actions: if row.get::<_, i64>(22)? == 0 {
            vec!["reveal".to_string()]
        } else {
            Vec::new()
        },
        revision: 0,
        content_status: None,
        content_policy: None,
        content_summary: None,
        content_keywords: Vec::new(),
        content_language: None,
        content_provenance: None,
        content_truncated: None,
        content_text_retained: None,
        content_revision: None,
        native_semantics: None,
    })
}

fn load_file_tags(conn: &Connection, file_id: &str) -> Result<Vec<UserTagPreviewDto>, DbError> {
    let mut stmt = conn.prepare("SELECT t.id, t.display_name, t.color_token FROM file_user_tags AS fut JOIN user_tags AS t ON t.id = fut.tag_id WHERE fut.file_id = ?1 ORDER BY t.normalized_name COLLATE NOCASE, t.id")?;
    let rows = stmt.query_map(params![file_id], |row| {
        Ok(UserTagPreviewDto {
            id: row.get(0)?,
            display_name: row.get(1)?,
            color_token: row.get(2)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
}

fn load_active_finding_summaries(
    conn: &Connection,
    file_id: &str,
) -> Result<Vec<FileLibraryFindingSummaryDto>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT af.id, af.category, af.tier, af.detector_id, af.status, \
                COALESCE(afd.decision, 'open'), af.evidence_summary_json, af.revision \
         FROM analysis_findings AS af \
         LEFT JOIN analysis_finding_decisions AS afd ON afd.finding_key = af.finding_key \
         WHERE af.status = 'active' AND ( \
             (af.primary_subject_kind = 'file' AND af.primary_subject_id = ?1) OR \
             EXISTS (SELECT 1 FROM analysis_finding_evidence AS afe \
                     WHERE afe.finding_id = af.id AND afe.subject_kind = 'file' \
                       AND afe.subject_id = ?1)) \
         ORDER BY CASE af.tier WHEN 'caution' THEN 0 WHEN 'review' THEN 1 ELSE 2 END, \
                  af.updated_at DESC, af.id LIMIT 8",
    )?;
    let rows = stmt.query_map(params![file_id], |row| {
        let evidence_json: String = row.get(6)?;
        Ok(FileLibraryFindingSummaryDto {
            id: row.get(0)?,
            finding_type: row.get(1)?,
            severity: row.get(2)?,
            detector: row.get(3)?,
            state: row.get(4)?,
            decision: row.get(5)?,
            evidence_summary: serde_json::from_str(&evidence_json)
                .unwrap_or(serde_json::Value::Null),
            analysis_revision: row.get(7)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
}

fn find_root_for_path(
    conn: &Connection,
    path: &str,
) -> Result<Option<(String, String, String)>, DbError> {
    let normalized_path = normalize_path_text(path);
    let mut stmt = conn.prepare("SELECT id, display_name, normalized_path, health_status FROM scan_roots WHERE source_kind = 'file_library' AND enabled = 1 ORDER BY length(normalized_path) DESC, id")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (id, name, root, health) = row?;
        let root = normalize_path_text(&root).trim_end_matches('/').to_string();
        if normalized_path == root
            || normalized_path
                .strip_prefix(&root)
                .is_some_and(|suffix| suffix.starts_with('/'))
        {
            return Ok(Some((id, name, health)));
        }
    }
    Ok(None)
}

pub(crate) type SelectionWhere = (String, Vec<SqlValue>, i64, i64, Option<String>);

fn selection_stale_count(
    conn: &Connection,
    selection: &LibrarySelectionV1,
) -> Result<i64, DbError> {
    if !matches!(selection, LibrarySelectionV1::Explicit { .. }) {
        return Ok(0);
    }
    conn.query_row(
        "SELECT COUNT(*) FROM files AS f \
         JOIN temp.library_selection_ids AS selected ON selected.file_id = f.id \
         WHERE selected.kind = 'explicit' AND f.is_stale = 1",
        [],
        |row| row.get(0),
    )
    .map_err(DbError::from)
}

fn common_directory_for_paths(left: &str, right: &str) -> Option<String> {
    let left = normalize_path_text(&parent_directory(left));
    let right = normalize_path_text(&parent_directory(right));
    let left_parts = left.split('/').collect::<Vec<_>>();
    let right_parts = right.split('/').collect::<Vec<_>>();
    let common = left_parts
        .iter()
        .zip(right_parts.iter())
        .take_while(|(left, right)| left.eq_ignore_ascii_case(right))
        .map(|(part, _)| *part)
        .collect::<Vec<_>>();
    if common.is_empty() {
        None
    } else {
        Some(common.join("/"))
    }
}

pub(crate) fn selection_where(
    conn: &Connection,
    selection: &LibrarySelectionV1,
    current_revision: i64,
) -> Result<SelectionWhere, DbError> {
    clear_temp_selection_ids(conn)?;
    match selection {
        LibrarySelectionV1::Explicit { file_ids } => {
            if file_ids.len() > LIBRARY_SELECTION_MAX {
                return Err(DbError::Validation(
                    "library_selection_too_large".to_string(),
                ));
            }
            let mut ids = file_ids
                .iter()
                .map(|id| validate_file_id(id))
                .collect::<Result<Vec<_>, _>>()?;
            ids.sort();
            ids.dedup();
            materialize_temp_selection_ids(conn, "explicit", &ids)?;
            let present: i64 = conn.query_row(
                "SELECT COUNT(*) FROM files AS f \
                 JOIN temp.library_selection_ids AS selected ON selected.file_id = f.id \
                 WHERE selected.kind = 'explicit'",
                [],
                |row| row.get(0),
            )?;
            let missing = i64::try_from(ids.len()).unwrap_or(0) - present;
            let where_sql = if ids.is_empty() {
                "1 = 0".to_string()
            } else {
                "f.is_stale = 0 AND EXISTS (SELECT 1 FROM temp.library_selection_ids AS selected WHERE selected.kind = 'explicit' AND selected.file_id = f.id)".to_string()
            };
            Ok((where_sql, Vec::new(), missing, 0, None))
        }
        LibrarySelectionV1::AllMatching {
            query,
            query_fingerprint,
            snapshot_revision,
            excluded_file_ids,
        } => {
            if *snapshot_revision != current_revision {
                return Err(DbError::Validation("library_snapshot_expired".to_string()));
            }
            let (spec, _json, fingerprint) = canonicalize_file_query_spec(query.as_ref().clone())?;
            if &fingerprint != query_fingerprint {
                return Err(DbError::Validation(
                    "library_selection_query_mismatch".to_string(),
                ));
            }
            let canonical = CanonicalQuery {
                spec,
                fingerprint: fingerprint.clone(),
            };
            let scope = resolve_scope(conn, &canonical.spec.scope)?;
            if scope.health.state == "invalid_reference" {
                return Err(DbError::Validation(
                    "library_scope_invalid:reference".to_string(),
                ));
            }
            if scope.health.state != "healthy" {
                return Err(DbError::Validation("library_scope_unavailable".to_string()));
            }
            if query_has_missing_tags(conn, &canonical.spec.filters)? {
                return Err(DbError::Validation(
                    "library_selection_invalid_tag_reference".to_string(),
                ));
            }
            let mut conditions = vec!["f.is_stale = 0".to_string()];
            let mut params = Vec::new();
            if let Some(text) = canonical.spec.text.as_deref() {
                let fts_query = build_fts_query(text)
                    .ok_or_else(|| DbError::Validation("library_query_text_invalid".to_string()))?;
                conditions.push(
                    "f.rowid IN (SELECT files_fts.rowid FROM files_fts WHERE files_fts MATCH ?)"
                        .to_string(),
                );
                params.push(SqlValue::Text(fts_query));
            }
            conditions.push(format!("({})", scope.clause));
            params.extend(scope.params);
            append_filters(conn, &mut conditions, &mut params, &canonical.spec.filters)?;
            let base_where = conditions.join(" AND ");
            let mut exclusions = excluded_file_ids
                .iter()
                .map(|id| validate_file_id(id))
                .collect::<Result<Vec<_>, _>>()?;
            exclusions.sort();
            exclusions.dedup();
            if exclusions.len() > LIBRARY_SELECTION_MAX {
                return Err(DbError::Validation(
                    "library_selection_too_large".to_string(),
                ));
            }
            let mut excluded_count = 0_i64;
            if !exclusions.is_empty() {
                materialize_temp_selection_ids(conn, "excluded", &exclusions)?;
                let count_sql = format!(
                    "SELECT COUNT(*) FROM files AS f WHERE {base_where} AND EXISTS \
                     (SELECT 1 FROM temp.library_selection_ids AS excluded \
                      WHERE excluded.kind = 'excluded' AND excluded.file_id = f.id)"
                );
                excluded_count =
                    conn.query_row(&count_sql, params_from_iter(params.iter()), |row| {
                        row.get(0)
                    })?;
                conditions.push(
                    "NOT EXISTS (SELECT 1 FROM temp.library_selection_ids AS excluded \
                     WHERE excluded.kind = 'excluded' AND excluded.file_id = f.id)"
                        .to_string(),
                );
            }
            Ok((
                conditions.join(" AND "),
                params,
                0,
                excluded_count,
                Some(fingerprint),
            ))
        }
    }
}

pub(crate) fn clear_temp_selection_ids(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS library_selection_ids (
            kind TEXT NOT NULL,
            file_id TEXT NOT NULL,
            PRIMARY KEY(kind, file_id)
        ) WITHOUT ROWID;
        DELETE FROM temp.library_selection_ids;",
    )?;
    Ok(())
}

fn materialize_temp_selection_ids(
    conn: &Connection,
    kind: &str,
    ids: &[String],
) -> Result<(), DbError> {
    debug_assert!(matches!(kind, "explicit" | "excluded"));
    for chunk in ids.chunks(500) {
        let placeholders = std::iter::repeat_n(format!("('{kind}', ?)"), chunk.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "INSERT OR IGNORE INTO temp.library_selection_ids(kind, file_id) VALUES {placeholders}"
        );
        let mut values = Vec::with_capacity(chunk.len());
        for id in chunk {
            values.push(SqlValue::Text(id.clone()));
        }
        conn.execute(&sql, params_from_iter(values.iter()))?;
    }
    Ok(())
}

fn tag_exists(conn: &Connection, id: &str) -> Result<bool, DbError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM user_tags WHERE id = ?1",
            params![id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

fn validate_file_id(value: &str) -> Result<String, DbError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 512 || value.chars().any(|ch| ch.is_control()) {
        return Err(DbError::Validation("library_file_id_invalid".to_string()));
    }
    Ok(value.to_string())
}

pub(super) fn validate_tag_name(value: &str) -> Result<String, DbError> {
    let value = value.trim();
    let length = value.chars().count();
    if length == 0
        || length > LIBRARY_TAG_NAME_MAX
        || value
            .chars()
            .any(|ch| ch.is_control() || ch == '/' || ch == '\\')
    {
        return Err(DbError::Validation("library_tag_name_invalid".to_string()));
    }
    if value.chars().all(char::is_whitespace)
        || value.to_ascii_lowercase().starts_with("system:")
        || value.to_ascii_lowercase().starts_with("zen:")
    {
        return Err(DbError::Validation("library_tag_name_reserved".to_string()));
    }
    Ok(value.to_string())
}

pub(super) fn validate_saved_view_name(value: &str) -> Result<String, DbError> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > LIBRARY_SAVED_VIEW_NAME_MAX
        || value.chars().any(|ch| ch.is_control())
    {
        return Err(DbError::Validation(
            "library_saved_view_name_invalid".to_string(),
        ));
    }
    Ok(value.to_string())
}

pub(super) fn validate_color_token(value: &str) -> Result<String, DbError> {
    let value = value.trim().to_ascii_lowercase();
    if COLOR_TOKENS.contains(&value.as_str()) {
        Ok(value)
    } else {
        Err(DbError::Validation("library_tag_color_invalid".to_string()))
    }
}

pub(super) fn normalize_tag_name(value: &str) -> String {
    value.trim().to_lowercase()
}

pub(super) fn map_tag_write_error(error: rusqlite::Error) -> DbError {
    if matches!(error, rusqlite::Error::SqliteFailure(_, _)) {
        DbError::Validation("library_tag_conflict".to_string())
    } else {
        DbError::Sqlite(error)
    }
}

pub(super) fn map_saved_view_write_error(error: rusqlite::Error) -> DbError {
    if matches!(error, rusqlite::Error::SqliteFailure(_, _)) {
        DbError::Validation("library_saved_view_conflict".to_string())
    } else {
        DbError::Sqlite(error)
    }
}

fn escape_like_pattern(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '~' | '%' | '_') {
            result.push('~');
        }
        result.push(ch);
    }
    result
}

impl Database {
    pub(crate) fn manual_transfer_selection_ids(
        &self,
        selection: &LibrarySelectionV1,
    ) -> Result<Vec<String>, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let revision = current_library_revision(&tx)?;
        let (predicate, values, missing, _, _) = selection_where(&tx, selection, revision)?;
        if missing > 0 || selection_stale_count(&tx, selection)? > 0 {
            return Err(DbError::Validation("transfer_source_unavailable".into()));
        }
        let mut stmt = tx.prepare(&format!(
            "SELECT f.id FROM files AS f WHERE {predicate} ORDER BY f.id LIMIT 1001"
        ))?;
        let ids = stmt
            .query_map(rusqlite::params_from_iter(values), |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if ids.len() > 1000 {
            return Err(DbError::Validation("transfer_selection_limit_1000".into()));
        }
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_query_is_stable_and_rejects_invalid_values() {
        let spec = FileQuerySpecV2 {
            scope: FileLibraryScopeV2::AllEnabledRoots,
            text: Some("  report  ".to_string()),
            filters: FileQueryFiltersV2 {
                file_types: vec!["Document".into(), "Document".into()],
                ..Default::default()
            },
            sort: FileLibrarySortV2 {
                kind: LibrarySortKind::Relevance,
                direction: LibrarySortDirection::Asc,
            },
        };
        let (_, json, fingerprint) = canonicalize_file_query_spec(spec).expect("canonical query");
        assert!(json.contains("report"));
        assert_eq!(fingerprint.len(), 64);
        let invalid = FileQuerySpecV2 {
            scope: FileLibraryScopeV2::AllEnabledRoots,
            text: None,
            filters: Default::default(),
            sort: FileLibrarySortV2 {
                kind: LibrarySortKind::Relevance,
                direction: LibrarySortDirection::Desc,
            },
        };
        assert_eq!(
            canonicalize_file_query_spec(invalid)
                .unwrap_err()
                .to_string(),
            "library_sort_relevance_requires_text"
        );
    }

    #[test]
    fn membership_fingerprint_ignores_only_sort_changes() {
        let modified = FileQuerySpecV2 {
            scope: FileLibraryScopeV2::AllEnabledRoots,
            text: Some("report".into()),
            filters: FileQueryFiltersV2 {
                file_types: vec!["Document".into()],
                ..Default::default()
            },
            sort: FileLibrarySortV2 {
                kind: LibrarySortKind::Modified,
                direction: LibrarySortDirection::Desc,
            },
        };
        let mut name = modified.clone();
        name.sort = FileLibrarySortV2 {
            kind: LibrarySortKind::Name,
            direction: LibrarySortDirection::Asc,
        };
        let mut different_filter = modified.clone();
        different_filter.filters.file_types = vec!["Image".into()];

        assert_eq!(
            membership_fingerprint(&modified).expect("modified membership fingerprint"),
            membership_fingerprint(&name).expect("name membership fingerprint")
        );
        assert_ne!(
            membership_fingerprint(&modified).expect("modified membership fingerprint"),
            membership_fingerprint(&different_filter)
                .expect("different-filter membership fingerprint")
        );
    }

    #[test]
    fn tag_names_and_colors_are_fail_closed() {
        assert!(validate_tag_name("ok").is_ok());
        assert!(validate_tag_name("system:internal").is_err());
        assert!(validate_tag_name("a/b").is_err());
        assert!(validate_color_token("#fff").is_err());
        assert!(validate_color_token("blue").is_ok());
    }

    #[test]
    fn cursor_round_trip_is_opaque_and_tamper_checked() {
        let cursor = LibraryCursor {
            version: LIBRARY_CURSOR_VERSION,
            fingerprint: "f".into(),
            revision: 1,
            total_count: 1,
            sort_kind: LibrarySortKind::Modified,
            direction: LibrarySortDirection::Desc,
            file_id: "id".into(),
            last_i64: Some(5),
            last_text: None,
            last_f64_bits: None,
            last_rank_bits: None,
            last_mtime: None,
        };
        let encoded = encode_cursor(&cursor);
        let decoded = decode_cursor(&encoded).expect("decode cursor");
        assert_eq!(decoded.file_id, "id");
        assert_eq!(decoded.total_count, 1);
        assert!(decode_cursor(&(encoded + "00")).is_err());
    }

    #[test]
    fn temp_selection_set_chunks_100k_ids_and_cleans_between_requests() {
        let conn = Connection::open_in_memory().expect("temporary selection connection");
        clear_temp_selection_ids(&conn).expect("create temp selection table");
        for count in [0_usize, 1, 128, 129, 999, 32_766, 99_999, 100_000] {
            let ids = (0..count)
                .map(|index| format!("selection-{index:06}"))
                .collect::<Vec<_>>();
            materialize_temp_selection_ids(&conn, "explicit", &ids)
                .expect("chunked selection materialization");
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM temp.library_selection_ids WHERE kind = 'explicit'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("selection count"),
                count as i64
            );
            clear_temp_selection_ids(&conn).expect("request cleanup");
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM temp.library_selection_ids",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("clean selection count"),
                0
            );
        }
    }

    #[test]
    fn cursor_rejects_non_finite_numeric_tuples_and_unknown_saved_view_fields() {
        let cursor = LibraryCursor {
            version: LIBRARY_CURSOR_VERSION,
            fingerprint: "f".into(),
            revision: 1,
            total_count: 1,
            sort_kind: LibrarySortKind::Confidence,
            direction: LibrarySortDirection::Asc,
            file_id: "id".into(),
            last_i64: None,
            last_text: None,
            last_f64_bits: Some(f64::NAN.to_bits()),
            last_rank_bits: None,
            last_mtime: None,
        };
        let canonical = CanonicalQuery {
            spec: FileQuerySpecV2 {
                scope: FileLibraryScopeV2::AllEnabledRoots,
                text: None,
                filters: Default::default(),
                sort: FileLibrarySortV2 {
                    kind: LibrarySortKind::Confidence,
                    direction: LibrarySortDirection::Asc,
                },
            },
            fingerprint: "f".into(),
        };
        assert!(validate_cursor_binding(&cursor, &canonical).is_err());

        let payload = r#"{
            "scope": {"kind": "all_enabled_roots"},
            "filters": {},
            "sort": {"kind": "name", "direction": "asc"},
            "sql": "DELETE FROM files"
        }"#;
        assert!(serde_json::from_str::<FileQuerySpecV2>(payload).is_err());
    }

    #[test]
    fn query_v2_scope_health_never_widens_to_other_sources() {
        let path = std::env::temp_dir().join(format!(
            "zen-canvas-library-scope-test-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&path).expect("open scope test database");
        let conn = Connection::open(&path).expect("open scope fixture connection");
        conn.execute_batch(
            r#"
            INSERT INTO scan_roots (
                id, normalized_path, display_name, source_kind, enabled, health_status,
                current_generation, needs_reconciliation, created_at, updated_at
            ) VALUES
                ('scope-healthy', '/scope/healthy', 'Healthy', 'file_library', 1, 'healthy', 1, 0, 1, 1),
                ('scope-degraded', '/scope/degraded', 'Degraded', 'file_library', 1, 'degraded', 1, 1, 1, 1),
                ('scope-disabled', '/scope/disabled', 'Disabled', 'file_library', 0, 'healthy', 1, 0, 1, 1),
                ('scope-custom', '/scope/custom', 'Custom', 'custom_search', 1, 'healthy', 1, 0, 1, 1);
            "#,
        )
        .expect("insert scope roots");
        drop(conn);
        db.insert_files(&[
            InsertFileRequest {
                id: "scope-healthy-file".into(),
                path: "/scope/healthy/one.txt".into(),
                name: "one.txt".into(),
                extension: "txt".into(),
                size: 1,
                mtime: 1,
                ctime: 1,
                is_dir: false,
                state_code: 0,
            },
            InsertFileRequest {
                id: "scope-custom-file".into(),
                path: "/scope/custom/secret.txt".into(),
                name: "secret.txt".into(),
                extension: "txt".into(),
                size: 1,
                mtime: 1,
                ctime: 1,
                is_dir: false,
                state_code: 0,
            },
        ])
        .expect("insert scoped files");

        let request = |scope| FileQueryRequestV2 {
            version: 2,
            request_id: "scope-test".into(),
            query: FileQuerySpecV2 {
                scope,
                text: None,
                filters: Default::default(),
                sort: FileLibrarySortV2::default(),
            },
            page_size: 50,
            cursor: None,
        };
        let all_enabled = db
            .query_file_library_v2(request(FileLibraryScopeV2::AllEnabledRoots))
            .expect("query all enabled roots");
        assert_eq!(all_enabled.total_count, Some(1));
        assert_eq!(all_enabled.result_state, "partial");
        assert!(all_enabled
            .scope_health
            .roots
            .iter()
            .any(|root| root.id == "scope-degraded" && !root.available));

        let degraded = db
            .query_file_library_v2(request(FileLibraryScopeV2::Roots {
                scan_root_ids: vec!["scope-degraded".into()],
            }))
            .expect("query degraded root");
        assert_eq!(degraded.total_count, Some(0));
        assert_eq!(degraded.result_state, "partial");

        let missing = db
            .query_file_library_v2(request(FileLibraryScopeV2::Roots {
                scan_root_ids: vec!["missing-root".into()],
            }))
            .expect("query missing root");
        assert_eq!(missing.total_count, Some(0));
        assert_eq!(missing.result_state, "partial");
        assert_eq!(missing.scope_health.state, "invalid_reference");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn failed_tag_transaction_and_bulk_limit_do_not_advance_revision() {
        let path = std::env::temp_dir().join(format!(
            "zen-canvas-library-atomic-test-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&path).expect("open atomic test database");
        let tag = db
            .create_user_tag(CreateUserTagRequest {
                display_name: "Atomic tag".into(),
                color_token: "blue".into(),
            })
            .expect("create atomic tag");
        let before_duplicate = current_library_revision(&db.conn().expect("revision connection"))
            .expect("read duplicate revision");
        assert!(db
            .create_user_tag(CreateUserTagRequest {
                display_name: " atomic tag ".into(),
                color_token: "green".into(),
            })
            .is_err());
        assert_eq!(
            current_library_revision(&db.conn().expect("revision connection after duplicate"))
                .expect("read duplicate revision after"),
            before_duplicate
        );

        let too_many_ids = (0..=LIBRARY_SELECTION_MAX)
            .map(|index| format!("missing-{index}"))
            .collect::<Vec<_>>();
        let before_bulk = current_library_revision(&db.conn().expect("bulk revision connection"))
            .expect("read bulk revision");
        assert!(db
            .mutate_file_user_tags(MutateFileUserTagsRequest {
                selection: LibrarySelectionV1::Explicit {
                    file_ids: too_many_ids,
                },
                tag_ids: vec![tag.id],
                operation: UserTagMutationOperation::Add,
                expected_count: None,
            })
            .is_err());
        assert_eq!(
            current_library_revision(&db.conn().expect("bulk revision after connection"))
                .expect("read bulk revision after"),
            before_bulk
        );
        let conn = db.conn().expect("atomic tag rows connection");
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM file_user_tags", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("read atomic tag rows"),
            0
        );
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn tag_and_saved_view_revision_cas_rejects_same_second_stale_writers() {
        let path = std::env::temp_dir().join(format!(
            "zen-canvas-library-cas-test-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&path).expect("open CAS test database");
        let tag = db
            .create_user_tag(CreateUserTagRequest {
                display_name: "CAS tag".into(),
                color_token: "blue".into(),
            })
            .expect("create tag");
        assert_eq!(tag.revision, 1);
        let changed_tag = db
            .update_user_tag(UpdateUserTagRequest {
                id: tag.id.clone(),
                display_name: "CAS tag renamed".into(),
                color_token: "green".into(),
                expected_revision: 1,
            })
            .expect("update tag");
        assert_eq!(changed_tag.revision, 2);
        assert!(db
            .update_user_tag(UpdateUserTagRequest {
                id: tag.id,
                display_name: "stale writer".into(),
                color_token: "red".into(),
                expected_revision: 1,
            })
            .is_err());

        let view = db
            .create_library_saved_view(CreateLibrarySavedViewRequest {
                display_name: "CAS view".into(),
                query: FileQuerySpecV2 {
                    scope: FileLibraryScopeV2::AllEnabledRoots,
                    text: None,
                    filters: Default::default(),
                    sort: FileLibrarySortV2::default(),
                },
                position: Some(0),
            })
            .expect("create saved view");
        assert_eq!(view.revision, 1);
        let changed_view = db
            .update_library_saved_view(UpdateLibrarySavedViewRequest {
                id: view.id.clone(),
                display_name: "CAS view renamed".into(),
                query: view.query.clone(),
                position: 0,
                expected_revision: 1,
            })
            .expect("update saved view");
        assert_eq!(changed_view.revision, 2);
        assert!(db
            .delete_library_saved_view(DeleteLibrarySavedViewRequest {
                id: view.id,
                expected_revision: 1,
            })
            .is_err());
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn relevance_cursor_pages_follow_rank_direction_without_duplicates() {
        let path = std::env::temp_dir().join(format!(
            "zen-canvas-library-relevance-test-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&path).expect("open relevance test database");
        let conn = Connection::open(&path).expect("open relevance fixture connection");
        conn.execute(
            "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, current_generation, needs_reconciliation, created_at, updated_at) VALUES ('relevance-root', '/relevance', 'Relevance', 'file_library', 1, 'healthy', 1, 0, 1, 1)",
            [],
        )
        .expect("insert relevance root");
        drop(conn);
        db.insert_files(&[
            InsertFileRequest {
                id: "relevance-one".into(),
                path: "/relevance/report.txt".into(),
                name: "report.txt".into(),
                extension: "txt".into(),
                size: 1,
                mtime: 1,
                ctime: 1,
                is_dir: false,
                state_code: 0,
            },
            InsertFileRequest {
                id: "relevance-two".into(),
                path: "/relevance/report-report.txt".into(),
                name: "report-report.txt".into(),
                extension: "txt".into(),
                size: 2,
                mtime: 2,
                ctime: 2,
                is_dir: false,
                state_code: 0,
            },
            InsertFileRequest {
                id: "relevance-three".into(),
                path: "/relevance/report-notes.txt".into(),
                name: "report-notes.txt".into(),
                extension: "txt".into(),
                size: 3,
                mtime: 3,
                ctime: 3,
                is_dir: false,
                state_code: 0,
            },
        ])
        .expect("insert relevance files");

        for direction in [LibrarySortDirection::Asc, LibrarySortDirection::Desc] {
            let mut cursor = None;
            let mut ids = Vec::new();
            loop {
                let page = db
                    .query_file_library_v2(FileQueryRequestV2 {
                        version: 2,
                        request_id: "relevance-contract".into(),
                        query: FileQuerySpecV2 {
                            scope: FileLibraryScopeV2::AllEnabledRoots,
                            text: Some("report".into()),
                            filters: Default::default(),
                            sort: FileLibrarySortV2 {
                                kind: LibrarySortKind::Relevance,
                                direction: direction.clone(),
                            },
                        },
                        page_size: 1,
                        cursor,
                    })
                    .expect("query relevance page");
                let has_more = page.has_more;
                cursor = page.next_cursor;
                ids.extend(page.files.into_iter().map(|file| file.id));
                if !has_more {
                    break;
                }
            }
            let count = ids.len();
            ids.sort();
            ids.dedup();
            assert_eq!(count, 3);
            assert_eq!(ids.len(), 3);
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn query_v2_uses_keyset_pages_and_backend_tag_selection_contract() {
        let path = std::env::temp_dir().join(format!(
            "zen-canvas-library-query-test-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&path).expect("open library test database");
        let conn = Connection::open(&path).expect("open library fixture connection");
        conn.execute(
            r#"
            INSERT INTO scan_roots (
                id, normalized_path, display_name, source_kind, enabled, health_status,
                current_generation, needs_reconciliation, created_at, updated_at
            ) VALUES ('library-test-root', '/test/library', 'Library test root', 'file_library', 1, 'healthy', 1, 0, 1, 1)
            "#,
            [],
        )
        .expect("insert library root");
        drop(conn);
        db.insert_files(&[
            InsertFileRequest {
                id: "library-file-a".into(),
                path: "/test/library/alpha.txt".into(),
                name: "alpha.txt".into(),
                extension: "txt".into(),
                size: 10,
                mtime: 10,
                ctime: 10,
                is_dir: false,
                state_code: 0,
            },
            InsertFileRequest {
                id: "library-file-b".into(),
                path: "/test/library/beta.txt".into(),
                name: "beta.txt".into(),
                extension: "txt".into(),
                size: 20,
                mtime: 20,
                ctime: 20,
                is_dir: false,
                state_code: 0,
            },
        ])
        .expect("insert library files");

        let request = |cursor| FileQueryRequestV2 {
            version: 2,
            request_id: "library-test-request".into(),
            query: FileQuerySpecV2 {
                scope: FileLibraryScopeV2::AllEnabledRoots,
                text: None,
                filters: Default::default(),
                sort: FileLibrarySortV2 {
                    kind: LibrarySortKind::Name,
                    direction: LibrarySortDirection::Asc,
                },
            },
            page_size: 1,
            cursor,
        };
        let first = db
            .query_file_library_v2(request(None))
            .expect("query first library page");
        assert_eq!(first.total_count, Some(2));
        assert_eq!(first.files[0].id, "library-file-a");
        let cursor = first.next_cursor.clone().expect("second page cursor");
        let issued = decode_cursor(&cursor).expect("issued cursor JSON");
        let mut tampered = Vec::new();
        let mut total = issued.clone();
        total.total_count += 1;
        tampered.push(total);
        let mut file_id = issued.clone();
        file_id.file_id = "library-file-b".into();
        tampered.push(file_id);
        let mut name = issued.clone();
        name.last_text = Some("forged.txt".into());
        tampered.push(name);
        let mut direction = issued.clone();
        direction.direction = LibrarySortDirection::Desc;
        tampered.push(direction);
        let mut fingerprint = issued.clone();
        fingerprint.fingerprint = "0".repeat(64);
        tampered.push(fingerprint);
        let mut revision = issued.clone();
        revision.revision += 1;
        tampered.push(revision);
        for forged in tampered {
            let result = db.query_file_library_v2(request(Some(encode_cursor(&forged))));
            assert!(
                result.is_err()
                    || result
                        .as_ref()
                        .is_ok_and(|response| response.result_state == "snapshot_expired"),
                "legally encoded cursor tampering must fail closed"
            );
        }
        assert_eq!(
            db.cached_library_count(
                first.snapshot_revision,
                &membership_fingerprint(&request(None).query).expect("membership key")
            ),
            Some(2)
        );
        let second = db
            .query_file_library_v2(request(Some(cursor)))
            .expect("query second library page");
        assert_eq!(
            second
                .files
                .iter()
                .map(|file| file.id.as_str())
                .collect::<Vec<_>>(),
            vec!["library-file-b"]
        );
        assert_eq!(second.total_count, Some(2));
        assert!(!second.has_more);

        db.insert_file(InsertFileRequest {
            id: "library-file-c".into(),
            path: "/test/library/gamma.txt".into(),
            name: "gamma.txt".into(),
            extension: "txt".into(),
            size: 30,
            mtime: 30,
            ctime: 30,
            is_dir: false,
            state_code: 0,
        })
        .expect("insert changed-snapshot file");
        let expired = db
            .query_file_library_v2(request(Some(first.next_cursor.clone().expect("cursor"))))
            .expect("snapshot expiry response");
        assert_eq!(expired.result_state, "snapshot_expired");
        assert!(expired.files.is_empty());

        let tag = db
            .create_user_tag(CreateUserTagRequest {
                display_name: "Pinned".into(),
                color_token: "blue".into(),
            })
            .expect("create user tag");
        let bulk_tag = db
            .create_user_tag(CreateUserTagRequest {
                display_name: "Bulk label".into(),
                color_token: "green".into(),
            })
            .expect("create bulk user tag");
        let revision_before_tag =
            current_library_revision(&Connection::open(&path).expect("revision connection"))
                .expect("read revision before tag mutation");
        let mutation = db
            .mutate_file_user_tags(MutateFileUserTagsRequest {
                selection: LibrarySelectionV1::Explicit {
                    file_ids: vec!["library-file-a".into()],
                },
                tag_ids: vec![tag.id.clone()],
                operation: UserTagMutationOperation::Add,
                expected_count: Some(1),
            })
            .expect("assign user tag");
        assert_eq!(mutation.applied_count, 1);
        assert!(mutation.revision > revision_before_tag);
        let tagged = db
            .query_file_library_v2(FileQueryRequestV2 {
                version: 2,
                request_id: "library-tagged-request".into(),
                query: FileQuerySpecV2 {
                    scope: FileLibraryScopeV2::AllEnabledRoots,
                    text: None,
                    filters: FileQueryFiltersV2 {
                        tags_all_of: vec![tag.id.clone()],
                        ..Default::default()
                    },
                    sort: FileLibrarySortV2::default(),
                },
                page_size: 50,
                cursor: None,
            })
            .expect("query tagged files");
        assert_eq!(
            tagged
                .files
                .iter()
                .map(|file| file.id.as_str())
                .collect::<Vec<_>>(),
            vec!["library-file-a"]
        );

        let base_query = FileQuerySpecV2 {
            scope: FileLibraryScopeV2::AllEnabledRoots,
            text: None,
            filters: Default::default(),
            sort: FileLibrarySortV2::default(),
        };
        let base_response = db
            .query_file_library_v2(FileQueryRequestV2 {
                version: 2,
                request_id: "library-all-matching-request".into(),
                query: base_query.clone(),
                page_size: 50,
                cursor: None,
            })
            .expect("query all matching source");
        let all_matching = LibrarySelectionV1::AllMatching {
            query: Box::new(base_query),
            query_fingerprint: base_response.query_fingerprint.clone(),
            snapshot_revision: base_response.snapshot_revision,
            excluded_file_ids: vec!["library-file-b".into()],
        };
        let selection_summary = db
            .get_file_library_selection_summary(all_matching.clone())
            .expect("summarize all matching selection");
        assert_eq!(selection_summary.count, 2);
        assert_eq!(selection_summary.excluded_count, 1);
        let bulk_result = db
            .mutate_file_user_tags(MutateFileUserTagsRequest {
                selection: all_matching,
                tag_ids: vec![bulk_tag.id.clone()],
                operation: UserTagMutationOperation::Add,
                expected_count: Some(2),
            })
            .expect("mutate all matching selection");
        assert_eq!(bulk_result.applied_count, 2);
        let bulk_query = db
            .query_file_library_v2(FileQueryRequestV2 {
                version: 2,
                request_id: "library-bulk-tagged-request".into(),
                query: tagged_query_spec(&bulk_tag.id),
                page_size: 50,
                cursor: None,
            })
            .expect("query bulk-tagged files");
        assert_eq!(bulk_query.total_count, Some(2));

        let any_tag_query = db
            .query_file_library_v2(FileQueryRequestV2 {
                version: 2,
                request_id: "library-any-tag-request".into(),
                query: FileQuerySpecV2 {
                    scope: FileLibraryScopeV2::AllEnabledRoots,
                    text: None,
                    filters: FileQueryFiltersV2 {
                        tags_any_of: vec![tag.id.clone(), bulk_tag.id.clone()],
                        ..Default::default()
                    },
                    sort: FileLibrarySortV2::default(),
                },
                page_size: 50,
                cursor: None,
            })
            .expect("query any-tag files without duplicate counts");
        assert_eq!(any_tag_query.total_count, Some(2));
        assert_eq!(any_tag_query.files.len(), 2);

        let detail = db
            .get_file_library_detail("library-file-a")
            .expect("load detail by durable id");
        assert!(detail.tags.iter().any(|item| item.id == tag.id));
        assert!(detail.tags.iter().any(|item| item.id == bulk_tag.id));

        let conn = Connection::open(&path).expect("open cascade fixture connection");
        conn.execute(
            "UPDATE files SET id = 'library-file-a-renamed' WHERE id = 'library-file-a'",
            [],
        )
        .expect("update durable file id");
        assert_eq!(
            conn.query_row(
                "SELECT file_id FROM file_user_tags WHERE tag_id = ?1",
                params![tag.id],
                |row| row.get::<_, String>(0),
            )
            .expect("updated tag foreign key"),
            "library-file-a-renamed"
        );
        drop(conn);
        let renamed_detail = db
            .get_file_library_detail("library-file-a-renamed")
            .expect("load renamed durable id");
        assert!(renamed_detail
            .tags
            .iter()
            .any(|item| item.id == bulk_tag.id));

        let revision_before_saved_view_create = current_library_revision(
            &Connection::open(&path).expect("saved view create revision connection"),
        )
        .expect("saved view revision before create");
        let view = db
            .create_library_saved_view(CreateLibrarySavedViewRequest {
                display_name: "Pinned files".into(),
                query: tagged_query_spec(&tag.id),
                position: Some(0),
            })
            .expect("create saved view");
        assert_eq!(view.query_fingerprint.len(), 64);
        assert_eq!(
            current_library_revision(&Connection::open(&path).expect("saved view revision"))
                .expect("saved view revision after create"),
            revision_before_saved_view_create
        );
        assert_eq!(
            db.list_library_saved_views()
                .expect("list saved views")
                .len(),
            1
        );

        let revision_before_saved_view_write = current_library_revision(
            &Connection::open(&path).expect("saved view revision connection"),
        )
        .expect("saved view revision before write");
        let updated_view = db
            .update_library_saved_view(UpdateLibrarySavedViewRequest {
                id: view.id.clone(),
                display_name: "Renamed pinned files".into(),
                query: tagged_query_spec(&tag.id),
                position: 1,
                expected_revision: view.revision,
            })
            .expect("update saved view");
        assert_eq!(
            current_library_revision(&Connection::open(&path).expect("saved view revision"))
                .expect("saved view revision after update"),
            revision_before_saved_view_write
        );
        assert!(db
            .delete_user_tag(DeleteUserTagRequest {
                id: tag.id.clone(),
                confirm: true,
                expected_usage_count: 1,
                expected_revision: tag.revision,
            })
            .expect("delete used tag with confirmation"));
        let invalid_views = db
            .list_library_saved_views()
            .expect("list invalid saved view");
        assert!(invalid_views
            .iter()
            .find(|item| item.id == updated_view.id)
            .is_some_and(|item| item.invalid_references.contains(&format!("tag:{}", tag.id))));
        assert!(db
            .delete_library_saved_view(DeleteLibrarySavedViewRequest {
                id: updated_view.id,
                expected_revision: updated_view.revision,
            })
            .expect("delete saved view"));
        assert!(db
            .list_library_saved_views()
            .expect("list after saved view delete")
            .is_empty());

        for (kind, direction) in [
            (LibrarySortKind::Modified, LibrarySortDirection::Asc),
            (LibrarySortKind::Modified, LibrarySortDirection::Desc),
            (LibrarySortKind::Created, LibrarySortDirection::Asc),
            (LibrarySortKind::Created, LibrarySortDirection::Desc),
            (LibrarySortKind::Name, LibrarySortDirection::Asc),
            (LibrarySortKind::Name, LibrarySortDirection::Desc),
            (LibrarySortKind::Size, LibrarySortDirection::Asc),
            (LibrarySortKind::Size, LibrarySortDirection::Desc),
            (LibrarySortKind::Confidence, LibrarySortDirection::Asc),
            (LibrarySortKind::Confidence, LibrarySortDirection::Desc),
        ] {
            let mut cursor = None;
            let mut ids = Vec::new();
            loop {
                let page = db
                    .query_file_library_v2(FileQueryRequestV2 {
                        version: 2,
                        request_id: "library-sort-contract".into(),
                        query: FileQuerySpecV2 {
                            scope: FileLibraryScopeV2::AllEnabledRoots,
                            text: None,
                            filters: Default::default(),
                            sort: FileLibrarySortV2 {
                                kind: kind.clone(),
                                direction: direction.clone(),
                            },
                        },
                        page_size: 1,
                        cursor,
                    })
                    .expect("query sort contract");
                let has_more = page.has_more;
                let next_cursor = page.next_cursor;
                ids.extend(page.files.into_iter().map(|file| file.id));
                if !has_more {
                    break;
                }
                cursor = next_cursor;
            }
            ids.sort();
            ids.dedup();
            assert_eq!(
                ids.len(),
                3,
                "sort {:?} {:?} lost or duplicated rows",
                kind,
                direction
            );
        }

        let conn = Connection::open(&path).expect("reopen library test database");
        conn.execute("DELETE FROM files WHERE id = 'library-file-a-renamed'", [])
            .expect("delete tagged file");
        let tag_usage: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_user_tags WHERE file_id = 'library-file-a-renamed'",
                [],
                |row| row.get(0),
            )
            .expect("read cascaded tag rows");
        assert_eq!(tag_usage, 0);
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    fn tagged_query_spec(tag_id: &str) -> FileQuerySpecV2 {
        FileQuerySpecV2 {
            scope: FileLibraryScopeV2::AllEnabledRoots,
            text: None,
            filters: FileQueryFiltersV2 {
                tags_all_of: vec![tag_id.into()],
                ..Default::default()
            },
            sort: FileLibrarySortV2::default(),
        }
    }
}
