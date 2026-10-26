use super::super::{
    browse::{BrowseCompletion, BrowsePage, BrowseQuerySpecV1, EphemeralBrowseEntry},
    change::{EphemeralChangeHint, EphemeralChangeKind, EphemeralRefreshRequest},
    contracts::{
        BrowseEntryRef, BrowseEnumerationRef, BrowsePathRef, ContentReadEligibility, EntryRef,
        LocationRef, MaterializationState, PreviewHostKind, PreviewSourceRef, WorkClass,
        WorkspacePlatform, WorkspaceRestoreLocator,
    },
    location::LocationDescriptor,
    preview::{
        PreviewCapabilities, PreviewRepresentationEnvelope, PreviewSessionSnapshot,
        PreviewSessionState,
    },
    thumbnail::{ThumbnailArtifact, ThumbnailVariant},
};
use serde::{de, Deserialize, Deserializer, Serialize};

const MAX_REQUEST_TEXT_LENGTH: usize = 4096;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowseOpenRequest {
    pub platform: WorkspacePlatform,
    pub routing_hint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_entry_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowseOpenResponse {
    pub session_id: String,
    pub location: LocationDescriptor,
    pub root_path_ref: BrowsePathRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_page: Option<BrowsePageDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_entry_ref: Option<BrowseEntryRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowseRestoreRequest {
    pub locator: WorkspaceRestoreLocator,
}

/// Request one backend-owned Location to be re-admitted into a fresh
/// process-local Browse session. The request intentionally contains no
/// routing, display, provider or path-shaped fields.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocationBrowseRequest {
    pub location: LocationRef,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowseStartEnumerationRequest {
    pub session_id: String,
    pub request_id: String,
    pub path_ref: BrowsePathRef,
    pub page_size: usize,
    #[serde(default)]
    pub query: BrowseQuerySpecV1,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowseNextPageRequest {
    pub session_id: String,
    pub cursor: String,
    pub page_size: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowseCancelRequest {
    pub session_id: String,
    /// Present when the caller already received a published page.  A pending
    /// start request does not know the opaque enumeration id yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enumeration: Option<BrowseEnumerationRef>,
    /// Request id fallback used only to cancel the current pending enumeration
    /// owned by this Browse session.  BrowseService remains the identity
    /// authority; the integration layer never manufactures an enum ref.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl<'de> Deserialize<'de> for BrowseCancelRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            session_id: String,
            #[serde(default)]
            enumeration: Option<BrowseEnumerationRef>,
            #[serde(default)]
            request_id: Option<String>,
        }

        let wire = Wire::deserialize(deserializer)?;
        match (wire.enumeration, wire.request_id) {
            (Some(enumeration), None) => Ok(Self {
                session_id: wire.session_id,
                enumeration: Some(enumeration),
                request_id: None,
            }),
            (None, Some(request_id)) if !request_id.is_empty() => Ok(Self {
                session_id: wire.session_id,
                enumeration: None,
                request_id: Some(request_id),
            }),
            _ => Err(de::Error::custom(
                "browse_cancel_requires_exactly_one_identity",
            )),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowseReleasePageRequest {
    pub page: BrowsePageDto,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowseReleasePathRequest {
    pub session_id: String,
    pub path_ref: BrowsePathRef,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowseRetainPathRequest {
    pub session_id: String,
    pub path_ref: BrowsePathRef,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowseSessionRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowseCompletionDto {
    Partial,
    Complete,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowseEntryDto {
    #[serde(rename = "ref")]
    pub entry_ref: BrowseEntryRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_ref: Option<BrowsePathRef>,
    pub name: String,
    /// Presentation only. This field is never accepted by a resolver.
    pub display_path: String,
    pub kind: BrowseEntryKindDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    pub materialization: MaterializationState,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowseEntryKindDto {
    File,
    Directory,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowsePageDto {
    pub session_id: String,
    pub request_id: String,
    pub enumeration_id: String,
    pub entries: Vec<BrowseEntryDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub completion: BrowseCompletionDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_count: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocationListRequest {}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeStartRequest {
    pub session_id: String,
    pub path_ref: BrowsePathRef,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeStartResponse {
    pub monitor_id: String,
    pub session_id: String,
    pub path_ref: BrowsePathRef,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangePendingRequest {
    pub monitor_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangePendingResponse {
    pub monitor_id: String,
    pub sequence: u64,
    pub hint: ChangeHintDto,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKindDto {
    ContentChanged,
    Renamed,
    TargetUnavailable,
    Uncertain,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeHintDto {
    pub kind: ChangeKindDto,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeRefreshRequest {
    pub monitor_id: String,
    pub request_id: String,
    pub page_size: usize,
    #[serde(default)]
    pub query: BrowseQuerySpecV1,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadEligibilityRequest {
    pub source: PreviewSourceRef,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadEligibilityResponse {
    pub source: PreviewSourceRef,
    pub eligibility: ContentReadEligibility,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThumbnailVariantDto {
    Small,
    Medium,
    Large,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThumbnailRequestDto {
    pub request_id: String,
    pub source: EntryRef,
    pub variant: ThumbnailVariantDto,
    pub work_class: WorkClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThumbnailCancelRequest {
    pub request_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThumbnailArtifactDto {
    pub cache_key: String,
    pub bytes: Vec<u8>,
}

pub(crate) const THUMBNAIL_IPC_MAX_BYTES: usize = 16 * 1024 * 1024;
const THUMBNAIL_IPC_MAGIC: &[u8; 4] = b"ZCTH";
const THUMBNAIL_IPC_VERSION: u8 = 1;
const THUMBNAIL_IPC_HEADER_BYTES: usize = 13;

/// Encode thumbnail metadata and bytes into one bounded raw IPC response.
///
/// The cache key is a logical identity only.  It is deliberately carried as a
/// length-delimited UTF-8 field so the command can return Tauri's binary
/// `Response` without exposing a cache/staging path or serializing bytes as a
/// JSON number array.
pub(crate) fn encode_thumbnail_ipc_response(
    artifact: &ThumbnailArtifactDto,
) -> Result<Vec<u8>, String> {
    if artifact.cache_key.is_empty() || artifact.cache_key.len() > MAX_REQUEST_TEXT_LENGTH {
        return Err("thumbnail_ipc_metadata_invalid".to_string());
    }
    if artifact.bytes.len() > THUMBNAIL_IPC_MAX_BYTES {
        return Err("thumbnail_ipc_output_too_large".to_string());
    }
    let cache_key_len = u32::try_from(artifact.cache_key.len())
        .map_err(|_| "thumbnail_ipc_metadata_invalid".to_string())?;
    let bytes_len = u32::try_from(artifact.bytes.len())
        .map_err(|_| "thumbnail_ipc_output_too_large".to_string())?;
    let mut encoded = Vec::with_capacity(
        THUMBNAIL_IPC_HEADER_BYTES + artifact.cache_key.len() + artifact.bytes.len(),
    );
    encoded.extend_from_slice(THUMBNAIL_IPC_MAGIC);
    encoded.push(THUMBNAIL_IPC_VERSION);
    encoded.extend_from_slice(&cache_key_len.to_le_bytes());
    encoded.extend_from_slice(&bytes_len.to_le_bytes());
    encoded.extend_from_slice(artifact.cache_key.as_bytes());
    encoded.extend_from_slice(&artifact.bytes);
    Ok(encoded)
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewCreateRequest {
    pub request_id: String,
    pub source: PreviewSourceRef,
    pub host_kind: PreviewHostKind,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewSessionRequest {
    pub preview_id: String,
    /// Optional renderer geometry for a backend/native host presentation.
    /// The token and source version remain opaque renderer inputs; the native
    /// bridge binds them to the current backend snapshot before resolving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_presentation: Option<PreviewNativePresentation>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewNativePresentation {
    pub host: PreviewHostKind,
    pub token: String,
    pub source_version: String,
    pub bounds: PreviewNativeBounds,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewNativeBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewSwitchSourceRequest {
    pub preview_id: String,
    pub request_id: String,
    pub source: PreviewSourceRef,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewAssetRequestDto {
    pub preview_id: String,
    pub request_id: String,
    pub source_version: String,
    pub asset_token: String,
    /// Optional range coordinates for a range-backed PDF asset. Both fields
    /// must be present together; omission keeps the existing full owned-asset
    /// request contract for images and other small artifacts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewAssetArtifactDto {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

pub(crate) const PREVIEW_ASSET_IPC_MAX_BYTES: usize =
    crate::file_workspace::preview_asset::MAX_PREVIEW_ASSET_BYTES;
const PREVIEW_ASSET_IPC_MAGIC: &[u8; 4] = b"ZCAS";
const PREVIEW_ASSET_IPC_VERSION: u8 = 1;
const PREVIEW_ASSET_IPC_HEADER_BYTES: usize = 13;

/// Encode one bounded Preview asset without exposing a staging/cache path or
/// turning the asset into a JSON byte array.
pub(crate) fn encode_preview_asset_ipc_response(
    artifact: &PreviewAssetArtifactDto,
) -> Result<Vec<u8>, String> {
    if artifact.media_type.is_empty()
        || artifact.media_type.len() > MAX_REQUEST_TEXT_LENGTH
        || artifact
            .media_type
            .bytes()
            .any(|byte| byte.is_ascii_control())
    {
        return Err("preview_asset_media_type_invalid".to_string());
    }
    if artifact.bytes.len() > PREVIEW_ASSET_IPC_MAX_BYTES {
        return Err("preview_asset_output_too_large".to_string());
    }
    let media_type_len = u32::try_from(artifact.media_type.len())
        .map_err(|_| "preview_asset_media_type_invalid".to_string())?;
    let bytes_len = u32::try_from(artifact.bytes.len())
        .map_err(|_| "preview_asset_output_too_large".to_string())?;
    let mut encoded = Vec::with_capacity(
        PREVIEW_ASSET_IPC_HEADER_BYTES + artifact.media_type.len() + artifact.bytes.len(),
    );
    encoded.extend_from_slice(PREVIEW_ASSET_IPC_MAGIC);
    encoded.push(PREVIEW_ASSET_IPC_VERSION);
    encoded.extend_from_slice(&media_type_len.to_le_bytes());
    encoded.extend_from_slice(&bytes_len.to_le_bytes());
    encoded.extend_from_slice(artifact.media_type.as_bytes());
    encoded.extend_from_slice(&artifact.bytes);
    Ok(encoded)
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewSnapshotDto {
    pub preview_id: String,
    pub session_id: String,
    pub request_id: String,
    pub source: PreviewSourceRef,
    pub host_kind: PreviewHostKind,
    pub state: PreviewSessionStateDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representation: Option<PreviewRepresentationEnvelope>,
    pub effective_capabilities: PreviewCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_provider_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewSessionStateDto {
    Idle,
    Resolving,
    Preparing,
    Loading,
    Ready,
    Failed,
    Cancelled,
    Disposed,
}

impl BrowsePageDto {
    pub(crate) fn from_internal(page: BrowsePage) -> Self {
        Self {
            session_id: page.session_id,
            request_id: page.request_id,
            enumeration_id: page.enumeration_id,
            entries: page
                .entries
                .into_iter()
                .map(BrowseEntryDto::from_internal)
                .collect(),
            next_cursor: page.next_cursor,
            completion: match page.completion {
                BrowseCompletion::Partial => BrowseCompletionDto::Partial,
                BrowseCompletion::Complete => BrowseCompletionDto::Complete,
            },
            known_count: page.known_count,
        }
    }

    pub(crate) fn into_internal(self) -> BrowsePage {
        BrowsePage {
            session_id: self.session_id,
            request_id: self.request_id,
            enumeration_id: self.enumeration_id,
            entries: self
                .entries
                .into_iter()
                .map(BrowseEntryDto::into_internal)
                .collect(),
            next_cursor: self.next_cursor,
            completion: match self.completion {
                BrowseCompletionDto::Partial => BrowseCompletion::Partial,
                BrowseCompletionDto::Complete => BrowseCompletion::Complete,
            },
            known_count: self.known_count,
        }
    }
}

impl BrowseEntryDto {
    fn from_internal(entry: EphemeralBrowseEntry) -> Self {
        Self {
            entry_ref: entry.entry_ref,
            path_ref: entry.path_ref,
            name: entry.name,
            display_path: entry.display_path,
            kind: match entry.kind {
                super::super::browse::BrowseEntryKind::File => BrowseEntryKindDto::File,
                super::super::browse::BrowseEntryKind::Directory => BrowseEntryKindDto::Directory,
            },
            extension: entry.extension,
            size: entry.size,
            modified_at: entry.modified_at,
            created_at: entry.created_at,
            materialization: entry.materialization,
        }
    }

    fn into_internal(self) -> EphemeralBrowseEntry {
        EphemeralBrowseEntry {
            entry_ref: self.entry_ref,
            path_ref: self.path_ref,
            name: self.name,
            display_path: self.display_path,
            kind: match self.kind {
                BrowseEntryKindDto::File => super::super::browse::BrowseEntryKind::File,
                BrowseEntryKindDto::Directory => super::super::browse::BrowseEntryKind::Directory,
            },
            extension: self.extension,
            size: self.size,
            modified_at: self.modified_at,
            created_at: self.created_at,
            materialization: self.materialization,
        }
    }
}

impl From<EphemeralChangeKind> for ChangeKindDto {
    fn from(value: EphemeralChangeKind) -> Self {
        match value {
            EphemeralChangeKind::ContentChanged => Self::ContentChanged,
            EphemeralChangeKind::Renamed => Self::Renamed,
            EphemeralChangeKind::TargetUnavailable => Self::TargetUnavailable,
            EphemeralChangeKind::Uncertain => Self::Uncertain,
        }
    }
}

impl From<EphemeralChangeHint> for ChangeHintDto {
    fn from(value: EphemeralChangeHint) -> Self {
        Self {
            kind: value.kind.into(),
        }
    }
}

impl From<EphemeralRefreshRequest> for ChangePendingResponse {
    fn from(value: EphemeralRefreshRequest) -> Self {
        Self {
            monitor_id: String::new(),
            sequence: value.sequence,
            hint: value.hint.into(),
        }
    }
}

impl From<ThumbnailVariantDto> for ThumbnailVariant {
    fn from(value: ThumbnailVariantDto) -> Self {
        match value {
            ThumbnailVariantDto::Small => Self::Small,
            ThumbnailVariantDto::Medium => Self::Medium,
            ThumbnailVariantDto::Large => Self::Large,
        }
    }
}

impl From<ThumbnailArtifact> for ThumbnailArtifactDto {
    fn from(value: ThumbnailArtifact) -> Self {
        Self {
            cache_key: value.cache_key,
            bytes: value.bytes,
        }
    }
}

impl PreviewSnapshotDto {
    pub(crate) fn from_internal(preview_id: String, snapshot: PreviewSessionSnapshot) -> Self {
        Self {
            preview_id,
            session_id: snapshot.session_id,
            request_id: snapshot.request_id,
            source: snapshot.source,
            host_kind: snapshot.host_kind,
            state: snapshot.state.into(),
            source_version: snapshot.source_version,
            representation: snapshot.representation,
            effective_capabilities: snapshot.effective_capabilities,
            active_provider_id: snapshot.active_provider_id,
        }
    }
}

impl From<PreviewSessionState> for PreviewSessionStateDto {
    fn from(value: PreviewSessionState) -> Self {
        match value {
            PreviewSessionState::Idle => Self::Idle,
            PreviewSessionState::Resolving => Self::Resolving,
            PreviewSessionState::Preparing => Self::Preparing,
            PreviewSessionState::Loading => Self::Loading,
            PreviewSessionState::Ready => Self::Ready,
            PreviewSessionState::Failed => Self::Failed,
            PreviewSessionState::Cancelled => Self::Cancelled,
            PreviewSessionState::Disposed => Self::Disposed,
        }
    }
}

pub(crate) fn valid_bounded_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_REQUEST_TEXT_LENGTH && !value.contains('\0')
}
