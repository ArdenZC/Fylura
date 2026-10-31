//! Preview Contract Core for W1-06.
//!
//! This module owns only the disposable, read-only Preview contract. It does
//! not resolve paths, open bytes, materialize provider content, mutate the
//! filesystem, or persist state. A resolver supplies a backend-owned source
//! snapshot and providers consume that snapshot through an opaque operation
//! context. The existing authoritative read/open boundary remains outside
//! this module and will be adapted by W1-07.

use super::contracts::{
    ContentReadEligibility, ContentReadLeaseRef, MaterializationState, PreviewHostKind,
    PreviewSourceRef,
};
use super::preview_publication::PublicationSequence;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex, MutexGuard, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};
use thiserror::Error;

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Capabilities exposed by the host, provider and resolved source.
///
/// The effective value is always the intersection of all three layers. The
/// renderer must consume this result rather than infer controls from names,
/// extensions, paths or platform labels.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct PreviewCapabilities {
    pub can_search: bool,
    pub can_zoom: bool,
    pub can_playback: bool,
    pub can_select_text: bool,
    pub can_navigate_internal: bool,
    pub can_navigate_siblings: bool,
    pub can_open_external: bool,
    pub can_reveal: bool,
    pub can_request_materialization: bool,
}

impl PreviewCapabilities {
    pub const fn all() -> Self {
        Self {
            can_search: true,
            can_zoom: true,
            can_playback: true,
            can_select_text: true,
            can_navigate_internal: true,
            can_navigate_siblings: true,
            can_open_external: true,
            can_reveal: true,
            can_request_materialization: true,
        }
    }

    pub const fn metadata_fallback() -> Self {
        Self {
            can_search: false,
            can_zoom: false,
            can_playback: false,
            can_select_text: false,
            can_navigate_internal: false,
            can_navigate_siblings: false,
            can_open_external: true,
            can_reveal: true,
            // W3-01 has no renderer-callable authoritative materialization
            // action. Do not advertise a control that the host cannot safely
            // execute.
            can_request_materialization: false,
        }
    }

    pub const fn intersect(self, other: Self) -> Self {
        Self {
            can_search: self.can_search && other.can_search,
            can_zoom: self.can_zoom && other.can_zoom,
            can_playback: self.can_playback && other.can_playback,
            can_select_text: self.can_select_text && other.can_select_text,
            can_navigate_internal: self.can_navigate_internal && other.can_navigate_internal,
            can_navigate_siblings: self.can_navigate_siblings && other.can_navigate_siblings,
            can_open_external: self.can_open_external && other.can_open_external,
            can_reveal: self.can_reveal && other.can_reveal,
            can_request_materialization: self.can_request_materialization
                && other.can_request_materialization,
        }
    }
}

/// The host shell is intentionally separate from Preview Core/provider work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewHost {
    pub kind: PreviewHostKind,
    pub capabilities: PreviewCapabilities,
}

impl PreviewHost {
    pub const fn new(kind: PreviewHostKind, capabilities: PreviewCapabilities) -> Self {
        Self { kind, capabilities }
    }
}

/// Bounded metadata supplied by a SourceResolver.
///
/// This contains presentation metadata only. It deliberately has no path,
/// handle, provider URL, security-scoped URL or native object.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct PreviewMetadata {
    pub display_name: String,
    pub media_type: Option<String>,
    pub extension: Option<String>,
    pub size_bytes: Option<u64>,
    pub modified_at_epoch_ms: Option<i64>,
    pub materialization: MaterializationState,
    pub read_eligibility: ContentReadEligibility,
}

/// A backend-owned source snapshot. A source version is required for stale
/// publication protection; the snapshot never grants byte-read authority.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSourceSnapshot {
    pub source: PreviewSourceRef,
    pub source_version: String,
    pub metadata: PreviewMetadata,
    pub capabilities: PreviewCapabilities,
    /// Backend-only source shape used for provider routing. This is not part
    /// of the IPC snapshot wire; the resolver remains the authority for it.
    #[serde(skip)]
    pub entry_kind: PreviewEntryKind,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewEntryKind {
    #[default]
    File,
    Directory,
}

impl PreviewSourceSnapshot {
    pub fn new(
        source: PreviewSourceRef,
        source_version: impl Into<String>,
        metadata: PreviewMetadata,
        capabilities: PreviewCapabilities,
    ) -> Self {
        Self {
            source,
            source_version: source_version.into(),
            metadata,
            capabilities,
            entry_kind: PreviewEntryKind::File,
        }
    }

    pub fn with_entry_kind(mut self, entry_kind: PreviewEntryKind) -> Self {
        self.entry_kind = entry_kind;
        self
    }
}

pub type PreviewSourceCapabilities = PreviewCapabilities;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewRepresentationFamily {
    Metadata,
    Text,
    SafeHtml,
    StructuredTree,
    Table,
    Image,
    Pdf,
    Media,
    FolderSummary,
    ArchiveTree,
    NativeOpaque,
}

/// Host-neutral representations. NativeOpaque is the only host-bound family
/// and carries an opaque token that is meaningful only to the declared host.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "family", rename_all = "snake_case", deny_unknown_fields)]
pub enum PreviewRepresentation {
    Metadata {
        metadata: PreviewMetadata,
    },
    Text {
        text: String,
        language: Option<String>,
    },
    SafeHtml {
        html: String,
    },
    StructuredTree {
        #[serde(rename = "encodedTree")]
        encoded_tree: String,
    },
    Table {
        #[serde(rename = "encodedTable")]
        encoded_table: String,
    },
    Image {
        #[serde(rename = "assetToken")]
        asset_token: String,
        #[serde(rename = "mediaType")]
        media_type: String,
    },
    Pdf {
        #[serde(rename = "assetToken")]
        asset_token: String,
        #[serde(rename = "mediaType")]
        media_type: String,
        #[serde(rename = "lengthBytes")]
        length_bytes: u64,
    },
    Media {
        #[serde(rename = "assetToken")]
        asset_token: String,
        #[serde(rename = "mediaType")]
        media_type: String,
    },
    FolderSummary {
        #[serde(rename = "encodedSummary")]
        encoded_summary: String,
    },
    ArchiveTree {
        #[serde(rename = "encodedTree")]
        encoded_tree: String,
    },
    NativeOpaque {
        host: PreviewHostKind,
        token: String,
    },
}

impl PreviewRepresentation {
    pub const fn family(&self) -> PreviewRepresentationFamily {
        match self {
            Self::Metadata { .. } => PreviewRepresentationFamily::Metadata,
            Self::Text { .. } => PreviewRepresentationFamily::Text,
            Self::SafeHtml { .. } => PreviewRepresentationFamily::SafeHtml,
            Self::StructuredTree { .. } => PreviewRepresentationFamily::StructuredTree,
            Self::Table { .. } => PreviewRepresentationFamily::Table,
            Self::Image { .. } => PreviewRepresentationFamily::Image,
            Self::Pdf { .. } => PreviewRepresentationFamily::Pdf,
            Self::Media { .. } => PreviewRepresentationFamily::Media,
            Self::FolderSummary { .. } => PreviewRepresentationFamily::FolderSummary,
            Self::ArchiveTree { .. } => PreviewRepresentationFamily::ArchiveTree,
            Self::NativeOpaque { .. } => PreviewRepresentationFamily::NativeOpaque,
        }
    }

    fn is_host_compatible(&self, host: PreviewHostKind) -> bool {
        match self {
            Self::NativeOpaque {
                host: representation_host,
                ..
            } => *representation_host == host,
            _ => true,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewCompleteness {
    Complete,
    Partial,
    Unknown,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewProviderErrorCode {
    Unsupported,
    Failed,
    Timeout,
    CorruptSource,
    SourceUnavailable,
    MaterializationRequired,
    PermissionDenied,
    IdentityChanged,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewTerminalCondition {
    SourceUnavailable,
    MaterializationRequired,
    PermissionDenied,
    IdentityChanged,
    Cancelled,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum PreviewWarning {
    ProviderFallback {
        #[serde(rename = "providerId")]
        provider_id: String,
        reason: PreviewProviderErrorCode,
    },
    MetadataFallback,
    TerminalCondition {
        condition: PreviewTerminalCondition,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRepresentationEnvelope {
    pub source_version: String,
    pub representation: PreviewRepresentation,
    pub completeness: PreviewCompleteness,
    pub warnings: Vec<PreviewWarning>,
    pub capabilities: PreviewCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewProviderResult {
    pub representation: PreviewRepresentation,
    pub completeness: PreviewCompleteness,
    pub warnings: Vec<PreviewWarning>,
}

pub use super::preview_publication::{
    PreviewPublicationError, PreviewPublicationSink, PreviewPublicationUpdate,
};

pub use fylura_native_host::BoundedContentRead;

/// A bounded byte request for a previously issued opaque lease. It contains
/// no path, provider URL or filesystem handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundedContentReadRequest {
    pub offset_bytes: u64,
    pub max_bytes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ContentReadAccessError {
    #[error("content read lease is invalid")]
    LeaseInvalid,
    #[error("content read lease source version does not match")]
    SourceVersionMismatch,
    #[error("content read permission was denied")]
    PermissionDenied,
    #[error("content source is unavailable")]
    SourceUnavailable,
    #[error("content read was cancelled")]
    Cancelled,
    #[error("content read timed out")]
    TimedOut,
    #[error("content read failed")]
    Failed,
}

/// Preview-specific read failures preserve source/session terminal semantics
/// that are intentionally narrower than the shared thumbnail/content-read
/// error taxonomy. The adapter maps these into PreviewProviderError without
/// turning authoritative materialization or availability states into generic
/// provider failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PreviewReadAccessError {
    #[error("content read lease is invalid")]
    LeaseInvalid,
    #[error("content read lease source version does not match")]
    SourceVersionMismatch,
    #[error("content read permission was denied")]
    PermissionDenied,
    #[error("content source is unavailable")]
    SourceUnavailable,
    #[error("content materialization is required")]
    MaterializationRequired,
    #[error("content source is metadata-only")]
    MetadataOnly,
    #[error("content read was cancelled")]
    Cancelled,
    #[error("content read timed out")]
    TimedOut,
    #[error("content read failed")]
    Failed,
}

/// W1-07 supplies the authoritative implementation. Preview providers only
/// receive this bounded, opaque consumer; they never receive a raw path.
pub trait ContentReadLeaseConsumer: Send + Sync {
    fn read_bounded(
        &self,
        lease: &ContentReadLeaseRef,
        request: BoundedContentReadRequest,
        context: &PreviewOperationContext,
    ) -> Result<BoundedContentRead, ContentReadAccessError>;
}

/// Narrow backend-only source read access for providers that need to consume
/// the existing MaterializationReadGate. Implementations issue a short-lived
/// Preview-intent lease internally and must release it before returning. The
/// source and source version are carried explicitly so a provider cannot read
/// through a different source than the one it prepared for.
pub trait PreviewContentReadAccess: Send + Sync {
    fn read_source_bounded(
        &self,
        source: &PreviewSourceRef,
        source_version: &str,
        request: BoundedContentReadRequest,
        context: &PreviewOperationContext,
    ) -> Result<BoundedContentRead, PreviewReadAccessError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewFolderEntryKind {
    File,
    Directory,
    Other,
}

/// Bounded facts for one direct child. The adapter intentionally does not
/// expose the Browse entry ref, path ref, path, or a filesystem handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFolderEntryFact {
    pub name: String,
    pub kind: PreviewFolderEntryKind,
    pub extension: Option<String>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFolderPage {
    pub entries: Vec<PreviewFolderEntryFact>,
    pub complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PreviewFolderEnumerationError {
    #[error("folder enumeration is unsupported for this source")]
    Unsupported,
    #[error("folder source is unavailable")]
    SourceUnavailable,
    #[error("folder source identity changed")]
    IdentityChanged,
    #[error("folder permission was denied")]
    PermissionDenied,
    #[error("folder enumeration was cancelled")]
    Cancelled,
    #[error("folder enumeration deadline reached")]
    Deadline,
    #[error("folder enumeration failed")]
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewFolderPageAction {
    Continue,
    Stop,
}

/// Backend-only Folder Preview access. Implementations reuse the existing
/// BrowseService and must release page/session resources before returning.
pub trait PreviewFolderEnumerationAccess: Send + Sync {
    fn enumerate_direct_children(
        &self,
        source: &PreviewSourceRef,
        source_version: &str,
        context: &PreviewOperationContext,
        visit_page: &mut dyn FnMut(
            PreviewFolderPage,
        ) -> Result<
            PreviewFolderPageAction,
            PreviewFolderEnumerationError,
        >,
    ) -> Result<(), PreviewFolderEnumerationError>;
}

/// Optional provider environment. Production injects the narrow Preview read
/// adapter owned by the existing MaterializationReadGate boundary. The
/// legacy lease-consumer field remains for compatibility with older focused
/// tests/providers and never exposes a filesystem path.
#[derive(Clone, Copy)]
pub struct PreviewProviderEnvironment<'a> {
    pub content_read: Option<&'a dyn ContentReadLeaseConsumer>,
    pub preview_read: Option<&'a dyn PreviewContentReadAccess>,
    pub folder_enumeration: Option<&'a dyn PreviewFolderEnumerationAccess>,
    pub publication: Option<&'a dyn PreviewPublicationSink>,
    pub asset_publisher: Option<&'a dyn PreviewAssetPublisher>,
    pub(crate) decoder_admission:
        Option<&'a crate::scheduler::adapters::PreviewDecoderResourceLeaseAdapter>,
    pub(crate) archive_admission:
        Option<&'a crate::scheduler::adapters::PreviewArchiveResourceLeaseAdapter>,
}

/// Owned injection point for the existing authoritative content-read path.
///
/// The handle keeps provider byte access at the session/coordinator boundary;
/// providers receive only the bounded, request/sourceVersion-bound adapter.
#[derive(Clone, Default)]
pub struct PreviewProviderEnvironmentHandle {
    pub content_read: Option<Arc<dyn ContentReadLeaseConsumer>>,
    pub preview_read: Option<Arc<dyn PreviewContentReadAccess>>,
    pub(crate) folder_enumeration: Option<Arc<dyn PreviewFolderEnumerationAccess>>,
    pub asset_publisher: Option<Arc<dyn PreviewAssetPublisher>>,
    pub(crate) decoder_admission:
        Option<Arc<crate::scheduler::adapters::PreviewDecoderResourceLeaseAdapter>>,
    pub(crate) archive_admission:
        Option<Arc<crate::scheduler::adapters::PreviewArchiveResourceLeaseAdapter>>,
}

impl PreviewProviderEnvironmentHandle {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn with_content_read(content_read: Arc<dyn ContentReadLeaseConsumer>) -> Self {
        Self {
            content_read: Some(content_read),
            preview_read: None,
            folder_enumeration: None,
            asset_publisher: None,
            decoder_admission: None,
            archive_admission: None,
        }
    }

    pub fn with_content_read_and_asset_publisher(
        content_read: Arc<dyn ContentReadLeaseConsumer>,
        asset_publisher: Arc<dyn PreviewAssetPublisher>,
    ) -> Self {
        Self {
            content_read: Some(content_read),
            preview_read: None,
            folder_enumeration: None,
            asset_publisher: Some(asset_publisher),
            decoder_admission: None,
            archive_admission: None,
        }
    }

    pub fn with_preview_read(preview_read: Arc<dyn PreviewContentReadAccess>) -> Self {
        Self {
            content_read: None,
            preview_read: Some(preview_read),
            folder_enumeration: None,
            asset_publisher: None,
            decoder_admission: None,
            archive_admission: None,
        }
    }

    pub fn with_asset_publisher(asset_publisher: Arc<dyn PreviewAssetPublisher>) -> Self {
        Self {
            content_read: None,
            preview_read: None,
            folder_enumeration: None,
            asset_publisher: Some(asset_publisher),
            decoder_admission: None,
            archive_admission: None,
        }
    }

    pub fn with_preview_read_and_asset_publisher(
        preview_read: Arc<dyn PreviewContentReadAccess>,
        asset_publisher: Arc<dyn PreviewAssetPublisher>,
    ) -> Self {
        Self {
            content_read: None,
            preview_read: Some(preview_read),
            folder_enumeration: None,
            asset_publisher: Some(asset_publisher),
            decoder_admission: None,
            archive_admission: None,
        }
    }

    pub fn with_preview_read_and_asset_publisher_and_decoder(
        preview_read: Arc<dyn PreviewContentReadAccess>,
        asset_publisher: Arc<dyn PreviewAssetPublisher>,
        decoder_admission: Arc<crate::scheduler::adapters::PreviewDecoderResourceLeaseAdapter>,
    ) -> Self {
        Self {
            content_read: None,
            preview_read: Some(preview_read),
            folder_enumeration: None,
            asset_publisher: Some(asset_publisher),
            decoder_admission: Some(decoder_admission),
            archive_admission: None,
        }
    }

    pub fn with_preview_read_and_folder_enumeration_and_asset_publisher_and_decoder(
        preview_read: Arc<dyn PreviewContentReadAccess>,
        folder_enumeration: Arc<dyn PreviewFolderEnumerationAccess>,
        asset_publisher: Arc<dyn PreviewAssetPublisher>,
        decoder_admission: Arc<crate::scheduler::adapters::PreviewDecoderResourceLeaseAdapter>,
    ) -> Self {
        Self {
            content_read: None,
            preview_read: Some(preview_read),
            folder_enumeration: Some(folder_enumeration),
            asset_publisher: Some(asset_publisher),
            decoder_admission: Some(decoder_admission),
            archive_admission: None,
        }
    }

    pub fn with_preview_read_and_asset_publisher_and_decoder_and_archive(
        preview_read: Arc<dyn PreviewContentReadAccess>,
        asset_publisher: Arc<dyn PreviewAssetPublisher>,
        decoder_admission: Arc<crate::scheduler::adapters::PreviewDecoderResourceLeaseAdapter>,
        archive_admission: Arc<crate::scheduler::adapters::PreviewArchiveResourceLeaseAdapter>,
    ) -> Self {
        Self {
            content_read: None,
            preview_read: Some(preview_read),
            folder_enumeration: None,
            asset_publisher: Some(asset_publisher),
            decoder_admission: Some(decoder_admission),
            archive_admission: Some(archive_admission),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PreviewAssetError {
    #[error("preview asset publication is stale")]
    StalePublication,
    #[error("preview asset publication was cancelled")]
    Cancelled,
    #[error("preview asset media type is invalid")]
    InvalidMediaType,
    #[error("preview asset output is too large")]
    OutputTooLarge,
    #[error("preview asset capacity is exceeded")]
    CapacityExceeded,
    #[error("preview asset registry is disposed")]
    Disposed,
    #[error("preview asset range backing is unavailable")]
    RangeUnsupported,
}

/// Preview-only asset publication seam. Implementations own bounded storage
/// and retrieval; providers receive only an opaque token and never a path.
pub trait PreviewAssetPublisher: Send + Sync {
    fn publish_asset(
        &self,
        context: &PreviewOperationContext,
        media_type: &str,
        bytes: Vec<u8>,
    ) -> Result<String, PreviewAssetError>;

    /// Publish a request-scoped, opaque range-backed asset. The provider gives
    /// the registry the already-authorized source reference and source
    /// version, but never a path or an independent reader. Implementations
    /// must revalidate the tuple and use the existing bounded read authority
    /// for every subsequent range request.
    fn publish_range_asset(
        &self,
        _context: &PreviewOperationContext,
        _media_type: &str,
        _source: &PreviewSourceRef,
        _source_version: &str,
        _length_bytes: u64,
    ) -> Result<String, PreviewAssetError> {
        Err(PreviewAssetError::RangeUnsupported)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewProviderDescriptor {
    pub id: String,
    pub priority: i32,
    pub capabilities: PreviewCapabilities,
    pub supported_hosts: Vec<PreviewHostKind>,
    /// True when the provider may require the W1-07 opaque read gate. This is
    /// descriptive routing metadata, not a second eligibility decision.
    pub reads_content: bool,
}

impl PreviewProviderDescriptor {
    pub fn new(
        id: impl Into<String>,
        priority: i32,
        capabilities: PreviewCapabilities,
        supported_hosts: Vec<PreviewHostKind>,
        reads_content: bool,
    ) -> Self {
        Self {
            id: id.into(),
            priority,
            capabilities,
            supported_hosts,
            reads_content,
        }
    }

    fn supports_host(&self, host: PreviewHostKind) -> bool {
        self.supported_hosts.is_empty() || self.supported_hosts.contains(&host)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderProbe {
    Compatible,
    Unsupported,
}

/// Providers receive only a snapshot and an opaque operation context.
pub trait PreviewProvider: Send + Sync {
    fn descriptor(&self) -> &PreviewProviderDescriptor;

    /// Probe must be cheap and bounded; it must not perform unbounded reads.
    fn probe(
        &self,
        snapshot: &PreviewSourceSnapshot,
        context: &PreviewOperationContext,
    ) -> ProviderProbe;

    fn prepare(
        &self,
        snapshot: &PreviewSourceSnapshot,
        context: &PreviewOperationContext,
    ) -> Result<Box<dyn PreparedPreview>, PreviewProviderError>;
}

/// Prepared provider state is disposable and never owns a durable authority.
pub trait PreparedPreview: Send {
    fn load(
        &mut self,
        context: &PreviewOperationContext,
        environment: PreviewProviderEnvironment<'_>,
    ) -> Result<PreviewProviderResult, PreviewProviderError>;

    fn cleanup(&mut self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PreviewProviderError {
    #[error("provider does not support this source")]
    Unsupported,
    #[error("provider failed")]
    Failed,
    #[error("provider timed out")]
    Timeout,
    #[error("provider reported a corrupt source")]
    CorruptSource,
    #[error("source is unavailable")]
    SourceUnavailable,
    #[error("materialization is required")]
    MaterializationRequired,
    #[error("permission was denied")]
    PermissionDenied,
    #[error("source identity changed")]
    IdentityChanged,
    #[error("preview was cancelled")]
    Cancelled,
}

impl PreviewProviderError {
    pub const fn code(self) -> PreviewProviderErrorCode {
        match self {
            Self::Unsupported => PreviewProviderErrorCode::Unsupported,
            Self::Failed => PreviewProviderErrorCode::Failed,
            Self::Timeout => PreviewProviderErrorCode::Timeout,
            Self::CorruptSource => PreviewProviderErrorCode::CorruptSource,
            Self::SourceUnavailable => PreviewProviderErrorCode::SourceUnavailable,
            Self::MaterializationRequired => PreviewProviderErrorCode::MaterializationRequired,
            Self::PermissionDenied => PreviewProviderErrorCode::PermissionDenied,
            Self::IdentityChanged => PreviewProviderErrorCode::IdentityChanged,
            Self::Cancelled => PreviewProviderErrorCode::Cancelled,
        }
    }

    pub const fn terminal_condition(self) -> Option<PreviewTerminalCondition> {
        match self {
            Self::SourceUnavailable => Some(PreviewTerminalCondition::SourceUnavailable),
            Self::MaterializationRequired => {
                Some(PreviewTerminalCondition::MaterializationRequired)
            }
            Self::PermissionDenied => Some(PreviewTerminalCondition::PermissionDenied),
            Self::IdentityChanged => Some(PreviewTerminalCondition::IdentityChanged),
            Self::Cancelled => Some(PreviewTerminalCondition::Cancelled),
            Self::Unsupported | Self::Failed | Self::Timeout | Self::CorruptSource => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SourceResolveError {
    #[error("source is unavailable")]
    SourceUnavailable,
    #[error("materialization is required before source resolution")]
    MaterializationRequired,
    #[error("permission was denied while resolving the source")]
    PermissionDenied,
    #[error("source identity changed while resolving")]
    IdentityChanged,
    #[error("source resolution timed out")]
    Timeout,
    #[error("source resolution was cancelled")]
    Cancelled,
    #[error("source resolution returned a mismatched source")]
    SourceMismatch,
    #[error("source resolution failed")]
    Failed,
}

pub trait SourceResolver: Send + Sync {
    fn resolve(
        &self,
        request: &PreviewResolveRequest,
        context: &PreviewOperationContext,
    ) -> Result<PreviewSourceSnapshot, SourceResolveError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewResolveRequest {
    pub request_id: String,
    pub source: PreviewSourceRef,
    pub host_kind: PreviewHostKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewContextError {
    Cancelled,
    TimedOut,
    StalePublication,
}

#[derive(Clone, Debug, Default)]
pub struct PreviewCancellation(Arc<AtomicBool>);

impl PreviewCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn scheduler_token(&self) -> crate::scheduler::CancellationToken {
        crate::scheduler::CancellationToken::from_flag(Arc::clone(&self.0))
    }
}

#[derive(Debug)]
struct PublicationAuthority {
    generation: AtomicU64,
    disposed: AtomicBool,
}

/// Opaque publication capability used to reject late source/provider results.
#[derive(Clone, Debug)]
pub struct PreviewPublicationToken {
    session_id: String,
    request_id: String,
    source_version: Option<String>,
    generation: u64,
    authority: Arc<PublicationAuthority>,
    cancellation: PreviewCancellation,
}

impl PreviewPublicationToken {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn source_version(&self) -> Option<&str> {
        self.source_version.as_deref()
    }

    pub fn is_current(&self) -> bool {
        !self.authority.disposed.load(Ordering::Acquire)
            && self.authority.generation.load(Ordering::Acquire) == self.generation
            && !self.cancellation.is_cancelled()
    }

    fn with_source_version(&self, source_version: impl Into<String>) -> Self {
        Self {
            session_id: self.session_id.clone(),
            request_id: self.request_id.clone(),
            source_version: Some(source_version.into()),
            generation: self.generation,
            authority: Arc::clone(&self.authority),
            cancellation: self.cancellation.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PreviewOperationContext {
    session_id: String,
    request_id: String,
    host: PreviewHostKind,
    source_version: Option<String>,
    publication: PreviewPublicationToken,
    cancellation: PreviewCancellation,
    deadline: Instant,
}

impl PreviewOperationContext {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn host(&self) -> PreviewHostKind {
        self.host
    }

    pub fn source_version(&self) -> Option<&str> {
        self.source_version.as_deref()
    }

    pub fn publication(&self) -> &PreviewPublicationToken {
        &self.publication
    }

    pub fn cancellation(&self) -> PreviewCancellation {
        self.cancellation.clone()
    }

    pub(crate) fn scheduler_cancellation(&self) -> crate::scheduler::CancellationToken {
        self.cancellation.scheduler_token()
    }

    pub(crate) fn scheduler_cancellation_with(
        &self,
        additional_flag: Arc<AtomicBool>,
    ) -> crate::scheduler::CancellationToken {
        crate::scheduler::CancellationToken::from_flags(vec![
            Arc::clone(&self.cancellation.0),
            additional_flag,
        ])
    }

    pub fn is_publication_current(&self) -> bool {
        self.publication.is_current()
    }

    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    pub(crate) fn deadline(&self) -> Instant {
        self.deadline
    }

    pub fn ensure_active(&self) -> Result<(), PreviewContextError> {
        if self.cancellation.is_cancelled() {
            return Err(PreviewContextError::Cancelled);
        }
        if !self.publication.is_current() {
            return Err(PreviewContextError::StalePublication);
        }
        if Instant::now() >= self.deadline {
            return Err(PreviewContextError::TimedOut);
        }
        Ok(())
    }

    pub(crate) fn with_deadline(&self, deadline: Instant) -> Self {
        Self {
            deadline: if deadline < self.deadline {
                deadline
            } else {
                self.deadline
            },
            ..self.clone()
        }
    }

    /// Construct a short-lived operation context for a backend infrastructure
    /// consumer that uses the existing opaque content-read boundary without
    /// starting a full PreviewSession.  The context deliberately creates a
    /// fresh publication authority and carries no filesystem path or native
    /// handle.  Thumbnail uses this narrow seam so W1-07 can keep its
    /// `ContentReadLeaseConsumer` contract unchanged.
    pub(crate) fn for_backend_content_read(
        session_id: impl Into<String>,
        request_id: impl Into<String>,
        source_version: impl Into<String>,
        cancellation: PreviewCancellation,
        deadline: Instant,
    ) -> Self {
        let session_id = session_id.into();
        let request_id = request_id.into();
        let source_version = source_version.into();
        let authority = Arc::new(PublicationAuthority {
            generation: AtomicU64::new(1),
            disposed: AtomicBool::new(false),
        });
        let publication = PreviewPublicationToken {
            session_id: session_id.clone(),
            request_id: request_id.clone(),
            source_version: Some(source_version.clone()),
            generation: 1,
            authority,
            cancellation: cancellation.clone(),
        };
        Self {
            session_id,
            request_id,
            host: PreviewHostKind::ZenFloating,
            source_version: Some(source_version),
            publication,
            cancellation,
            deadline,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewWorkBudget {
    pub resolve_timeout: Duration,
    pub probe_timeout: Duration,
    pub prepare_timeout: Duration,
    pub load_timeout: Duration,
}

impl Default for PreviewWorkBudget {
    fn default() -> Self {
        Self {
            resolve_timeout: Duration::from_secs(2),
            probe_timeout: Duration::from_millis(250),
            prepare_timeout: Duration::from_secs(2),
            load_timeout: Duration::from_secs(5),
        }
    }
}

/// The bounded execution lanes used by Preview Core.
///
/// Coordinators and potentially blocking provider calls use separate bounded
/// lanes so a coordinator waiting on one provider cannot consume the only
/// worker that could execute another provider call. W1-10 may inject a shared
/// WorkScheduler-backed implementation through `PreviewExecution`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewExecutionLane {
    Coordinator,
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PreviewExecutionError {
    #[error("preview {lane:?} execution queue is full")]
    QueueFull { lane: PreviewExecutionLane },
    #[error("preview {lane:?} execution queue is unavailable")]
    Unavailable { lane: PreviewExecutionLane },
}

/// Shared execution boundary for Preview lifecycle and provider work.
///
/// Implementations must bound active work and queueing. The contract is
/// intentionally fire-and-observe: callers observe completion through their
/// own result channel and may stop waiting at a deadline while the late work
/// finishes without publication rights.
pub trait PreviewExecution: Send + Sync {
    fn submit(
        &self,
        lane: PreviewExecutionLane,
        name: &str,
        work: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<(), PreviewExecutionError>;
}

type PreviewWorkItem = Box<dyn FnOnce() + Send + 'static>;

struct BoundedPreviewJobPool {
    sender: SyncSender<PreviewWorkItem>,
}

impl BoundedPreviewJobPool {
    fn new(name: &str, worker_count: usize, queue_capacity: usize) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<PreviewWorkItem>(queue_capacity.max(1));
        let receiver = Arc::new(Mutex::new(receiver));
        for worker_index in 0..worker_count.max(1) {
            let receiver = Arc::clone(&receiver);
            let worker_name = format!("{name}-{worker_index}");
            thread::Builder::new()
                .name(worker_name)
                .spawn(move || loop {
                    let work = {
                        let receiver = lock(&receiver);
                        receiver.recv()
                    };
                    match work {
                        Ok(work) => work(),
                        Err(_) => break,
                    }
                })
                .expect("bounded preview executor worker must start");
        }
        Self { sender }
    }

    fn submit(
        &self,
        lane: PreviewExecutionLane,
        work: PreviewWorkItem,
    ) -> Result<(), PreviewExecutionError> {
        self.sender.try_send(work).map_err(|error| match error {
            TrySendError::Full(_) => PreviewExecutionError::QueueFull { lane },
            TrySendError::Disconnected(_) => PreviewExecutionError::Unavailable { lane },
        })
    }
}

/// Default bounded Preview execution boundary.
///
/// The pool is created once and shared by all default sessions. It is not a
/// per-session executor and therefore does not create an unbounded OS thread
/// for every `PreviewSession::start()` call.
pub struct BoundedPreviewExecution {
    coordinators: BoundedPreviewJobPool,
    providers: BoundedPreviewJobPool,
}

impl BoundedPreviewExecution {
    pub fn new(coordinator_workers: usize, provider_workers: usize, queue_capacity: usize) -> Self {
        Self {
            coordinators: BoundedPreviewJobPool::new(
                "preview-coordinator",
                coordinator_workers,
                queue_capacity,
            ),
            providers: BoundedPreviewJobPool::new(
                "preview-provider",
                provider_workers,
                queue_capacity,
            ),
        }
    }
}

impl Default for BoundedPreviewExecution {
    fn default() -> Self {
        Self::new(2, 4, 32)
    }
}

impl PreviewExecution for BoundedPreviewExecution {
    fn submit(
        &self,
        lane: PreviewExecutionLane,
        _name: &str,
        work: Box<dyn FnOnce() + Send + 'static>,
    ) -> Result<(), PreviewExecutionError> {
        match lane {
            PreviewExecutionLane::Coordinator => self.coordinators.submit(lane, work),
            PreviewExecutionLane::Provider => self.providers.submit(lane, work),
        }
    }
}

fn default_preview_execution() -> Arc<dyn PreviewExecution> {
    static DEFAULT: OnceLock<Arc<dyn PreviewExecution>> = OnceLock::new();
    DEFAULT
        .get_or_init(|| Arc::new(BoundedPreviewExecution::default()))
        .clone()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewRequest {
    pub request_id: String,
    pub source: PreviewSourceRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewSessionConfig {
    pub session_id: String,
    pub request: PreviewRequest,
    pub host: PreviewHost,
    pub budget: PreviewWorkBudget,
}

impl PreviewSessionConfig {
    pub fn new(
        session_id: impl Into<String>,
        request_id: impl Into<String>,
        source: PreviewSourceRef,
        host: PreviewHost,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            request: PreviewRequest {
                request_id: request_id.into(),
                source,
            },
            host,
            budget: PreviewWorkBudget::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewSessionState {
    Idle,
    Resolving,
    Preparing,
    Loading,
    Ready,
    Failed,
    Cancelled,
    Disposed,
}

#[derive(Debug, Clone)]
pub struct PreviewSessionSnapshot {
    pub session_id: String,
    pub request_id: String,
    pub source: PreviewSourceRef,
    pub host_kind: PreviewHostKind,
    pub state: PreviewSessionState,
    pub source_version: Option<String>,
    pub representation: Option<PreviewRepresentationEnvelope>,
    pub effective_capabilities: PreviewCapabilities,
    pub active_provider_id: Option<String>,
}

/// Exact identity captured by the native host when a bind fails. Fallback is
/// a compare-and-swap against this tuple, not a family-level replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativePreviewFailureIdentity {
    pub(crate) request_id: String,
    pub(crate) source_version: String,
    pub(crate) host: PreviewHostKind,
    pub(crate) token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PreviewSessionError {
    #[error("preview session is disposed")]
    Disposed,
    #[error("preview request is invalid")]
    InvalidRequest,
    #[error("preview session already has an active operation")]
    AlreadyRunning,
    #[error("preview session cannot start from state {0:?}")]
    InvalidState(PreviewSessionState),
    #[error("preview execution unavailable: {0}")]
    ExecutionUnavailable(#[from] PreviewExecutionError),
}

#[derive(Debug, Error)]
pub enum PreviewRunError {
    #[error("{0}")]
    Session(#[from] PreviewSessionError),
    #[error("source resolver failed: {0}")]
    SourceResolver(#[from] SourceResolveError),
    #[error("provider {provider_id} reached terminal condition {condition:?}")]
    ProviderTerminal {
        provider_id: String,
        condition: PreviewTerminalCondition,
    },
    #[error("preview was cancelled")]
    Cancelled,
    #[error("preview result was stale")]
    StalePublication,
    #[error("preview execution unavailable: {0}")]
    ExecutionUnavailable(#[from] PreviewExecutionError),
    #[error("preview worker panicked")]
    WorkerPanicked,
}

#[derive(Debug, Clone)]
pub struct PreviewRunOutcome {
    pub provider_id: Option<String>,
    pub envelope: PreviewRepresentationEnvelope,
    pub attempted_provider_ids: Vec<String>,
}

pub struct PreviewTask {
    receiver: Receiver<Result<PreviewRunOutcome, PreviewRunError>>,
}

impl PreviewTask {
    pub fn join(self) -> Result<PreviewRunOutcome, PreviewRunError> {
        self.receiver
            .recv()
            .unwrap_or(Err(PreviewRunError::WorkerPanicked))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PreviewRegistryError {
    #[error("preview provider id must not be empty")]
    EmptyProviderId,
    #[error("duplicate preview provider id: {0}")]
    DuplicateProviderId(String),
}

pub struct PreviewProviderRegistry {
    providers: Vec<Arc<dyn PreviewProvider>>,
}

impl PreviewProviderRegistry {
    pub fn new(providers: Vec<Arc<dyn PreviewProvider>>) -> Result<Self, PreviewRegistryError> {
        let mut ids = HashSet::new();
        for provider in &providers {
            let id = provider.descriptor().id.as_str();
            if id.is_empty() {
                return Err(PreviewRegistryError::EmptyProviderId);
            }
            if !ids.insert(id.to_string()) {
                return Err(PreviewRegistryError::DuplicateProviderId(id.to_string()));
            }
        }

        let mut providers = providers;
        providers.sort_by(|left, right| {
            right
                .descriptor()
                .priority
                .cmp(&left.descriptor().priority)
                .then_with(|| left.descriptor().id.cmp(&right.descriptor().id))
        });
        Ok(Self { providers })
    }

    pub fn provider_ids(&self) -> Vec<String> {
        self.providers
            .iter()
            .map(|provider| provider.descriptor().id.clone())
            .collect()
    }
}

struct PreparedPreviewGuard {
    inner: Option<Box<dyn PreparedPreview>>,
}

impl PreparedPreviewGuard {
    fn new(inner: Box<dyn PreparedPreview>) -> Self {
        Self { inner: Some(inner) }
    }

    fn load(
        &mut self,
        context: &PreviewOperationContext,
        environment: PreviewProviderEnvironment<'_>,
    ) -> Result<PreviewProviderResult, PreviewProviderError> {
        self.inner
            .as_mut()
            .ok_or(PreviewProviderError::Failed)?
            .load(context, environment)
    }

    fn cleanup_once(&mut self) {
        if let Some(mut inner) = self.inner.take() {
            inner.cleanup();
        }
    }
}

impl Drop for PreparedPreviewGuard {
    fn drop(&mut self) {
        self.cleanup_once();
    }
}

struct ActiveProvider {
    id: String,
}

struct SessionInner {
    session_id: String,
    request: PreviewRequest,
    host: PreviewHost,
    budget: PreviewWorkBudget,
    state: PreviewSessionState,
    cancellation: PreviewCancellation,
    running: bool,
    source_snapshot: Option<PreviewSourceSnapshot>,
    representation: Option<PreviewRepresentationEnvelope>,
    publication_sequence: PublicationSequence,
    effective_capabilities: PreviewCapabilities,
    active_provider: Option<ActiveProvider>,
}

#[derive(Clone)]
pub struct PreviewSession {
    inner: Arc<Mutex<SessionInner>>,
    authority: Arc<PublicationAuthority>,
    execution: Arc<dyn PreviewExecution>,
}

struct SessionPublicationSink {
    session: PreviewSession,
    token: PreviewPublicationToken,
    provider_id: String,
    host: PreviewHostKind,
    provider_capabilities: PreviewCapabilities,
    context: PreviewOperationContext,
    enabled: Arc<AtomicBool>,
}

struct OperationSeed {
    request: PreviewRequest,
    host: PreviewHost,
    budget: PreviewWorkBudget,
    token: PreviewPublicationToken,
}

impl OperationSeed {
    fn context(&self, source_version: Option<&str>, timeout: Duration) -> PreviewOperationContext {
        PreviewOperationContext {
            session_id: self.token.session_id.clone(),
            request_id: self.token.request_id.clone(),
            host: self.host.kind,
            source_version: source_version.map(str::to_owned),
            publication: match source_version {
                Some(version) => self.token.with_source_version(version),
                None => self.token.clone(),
            },
            cancellation: self.token.cancellation.clone(),
            deadline: Instant::now() + timeout,
        }
    }
}

#[derive(Debug)]
enum PreviewExecutionWaitError {
    TimedOut,
    Panicked,
    Execution(PreviewExecutionError),
}

fn execute_bounded<T, F>(
    execution: &dyn PreviewExecution,
    name: &str,
    timeout: Duration,
    work: F,
) -> Result<T, PreviewExecutionWaitError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (sender, receiver) = mpsc::sync_channel(1);
    let work = Box::new(move || {
        let result = catch_unwind(AssertUnwindSafe(work));
        let _ = sender.send(result);
    });
    execution
        .submit(PreviewExecutionLane::Provider, name, work)
        .map_err(PreviewExecutionWaitError::Execution)?;
    match receiver.recv_timeout(timeout) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err(PreviewExecutionWaitError::Panicked)
        }
        Err(mpsc::RecvTimeoutError::Timeout) => Err(PreviewExecutionWaitError::TimedOut),
    }
}

impl PreviewSession {
    pub fn new(config: PreviewSessionConfig) -> Self {
        Self::with_execution(config, default_preview_execution())
    }

    pub fn with_execution(
        config: PreviewSessionConfig,
        execution: Arc<dyn PreviewExecution>,
    ) -> Self {
        let cancellation = PreviewCancellation::default();
        Self {
            inner: Arc::new(Mutex::new(SessionInner {
                session_id: config.session_id,
                request: config.request,
                host: config.host,
                budget: config.budget,
                state: PreviewSessionState::Idle,
                cancellation,
                running: false,
                source_snapshot: None,
                representation: None,
                publication_sequence: PublicationSequence::default(),
                effective_capabilities: PreviewCapabilities::default(),
                active_provider: None,
            })),
            authority: Arc::new(PublicationAuthority {
                generation: AtomicU64::new(1),
                disposed: AtomicBool::new(false),
            }),
            execution,
        }
    }

    pub fn state(&self) -> PreviewSessionState {
        lock(&self.inner).state
    }

    pub fn request(&self) -> PreviewRequest {
        lock(&self.inner).request.clone()
    }

    pub fn host(&self) -> PreviewHost {
        lock(&self.inner).host
    }

    pub fn source_snapshot(&self) -> Option<PreviewSourceSnapshot> {
        lock(&self.inner).source_snapshot.clone()
    }

    pub fn representation(&self) -> Option<PreviewRepresentationEnvelope> {
        lock(&self.inner).representation.clone()
    }

    pub fn effective_capabilities(&self) -> PreviewCapabilities {
        lock(&self.inner).effective_capabilities
    }

    pub fn snapshot(&self) -> PreviewSessionSnapshot {
        let inner = lock(&self.inner);
        PreviewSessionSnapshot {
            session_id: inner.session_id.clone(),
            request_id: inner.request.request_id.clone(),
            source: inner.request.source.clone(),
            host_kind: inner.host.kind,
            state: inner.state,
            source_version: inner
                .source_snapshot
                .as_ref()
                .map(|snapshot| snapshot.source_version.clone()),
            representation: inner.representation.clone(),
            effective_capabilities: inner.effective_capabilities,
            active_provider_id: inner
                .active_provider
                .as_ref()
                .map(|provider| provider.id.clone()),
        }
    }

    /// Switches the source and immediately revokes all old publication rights.
    pub fn switch_source(&self, request: PreviewRequest) -> Result<(), PreviewSessionError> {
        let mut inner = lock(&self.inner);
        if inner.state == PreviewSessionState::Disposed
            || self.authority.disposed.load(Ordering::Acquire)
        {
            return Err(PreviewSessionError::Disposed);
        }
        if request.request_id.trim().is_empty() {
            return Err(PreviewSessionError::InvalidRequest);
        }
        inner.cancellation.cancel();
        self.authority.generation.fetch_add(1, Ordering::AcqRel);
        inner.cancellation = PreviewCancellation::default();
        inner.request = request;
        inner.state = PreviewSessionState::Resolving;
        inner.running = false;
        inner.source_snapshot = None;
        inner.representation = None;
        inner.effective_capabilities = PreviewCapabilities::default();
        inner.active_provider.take();
        Ok(())
    }

    /// Cancels the current operation, revokes publication rights and cleans up
    /// the currently prepared provider through its owning worker. It is
    /// idempotent and never waits for a provider-held load operation.
    pub fn cancel(&self) -> bool {
        let mut inner = lock(&self.inner);
        if matches!(
            inner.state,
            PreviewSessionState::Disposed | PreviewSessionState::Cancelled
        ) {
            return false;
        }
        inner.cancellation.cancel();
        self.authority.generation.fetch_add(1, Ordering::AcqRel);
        inner.state = PreviewSessionState::Cancelled;
        inner.running = false;
        if let Some(snapshot) = inner.source_snapshot.as_ref() {
            let envelope = metadata_fallback(snapshot, inner.host, Vec::new());
            inner.effective_capabilities = envelope.capabilities;
            inner.representation = Some(envelope);
        }
        inner.active_provider.take();
        true
    }

    /// Disposes the session permanently. The host may use this for close,
    /// source teardown or app shutdown; a disposed session cannot be reused.
    pub fn dispose(&self) -> bool {
        let mut inner = lock(&self.inner);
        if inner.state == PreviewSessionState::Disposed {
            return false;
        }
        inner.cancellation.cancel();
        self.authority.disposed.store(true, Ordering::Release);
        self.authority.generation.fetch_add(1, Ordering::AcqRel);
        inner.state = PreviewSessionState::Disposed;
        inner.running = false;
        inner.active_provider.take();
        true
    }

    /// Replaces a published native representation with the existing metadata
    /// fallback when the platform host cannot bind or retain the native view.
    /// The publication generation is advanced first so a late native/provider
    /// result cannot restore the failed NativeOpaque representation.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn fallback_from_native_failure(
        &self,
        provider_id: &str,
        expected: &NativePreviewFailureIdentity,
    ) -> bool {
        let mut inner = lock(&self.inner);
        if matches!(
            inner.state,
            PreviewSessionState::Disposed | PreviewSessionState::Cancelled
        ) {
            return false;
        }
        let Some(snapshot) = inner.source_snapshot.clone() else {
            return false;
        };
        if inner.request.request_id != expected.request_id
            || inner.host.kind != expected.host
            || snapshot.source_version != expected.source_version
        {
            return false;
        }
        let Some(envelope) = inner.representation.as_ref() else {
            return false;
        };
        if envelope.source_version != expected.source_version
            || !matches!(
                &envelope.representation,
                PreviewRepresentation::NativeOpaque { host, token }
                    if *host == expected.host && token == &expected.token
            )
        {
            return false;
        }
        inner.cancellation.cancel();
        self.authority.generation.fetch_add(1, Ordering::AcqRel);
        let envelope = metadata_fallback(
            &snapshot,
            inner.host,
            vec![PreviewWarning::ProviderFallback {
                provider_id: provider_id.to_string(),
                reason: PreviewProviderErrorCode::Failed,
            }],
        );
        inner.effective_capabilities = envelope.capabilities;
        inner.representation = Some(envelope);
        inner.state = PreviewSessionState::Ready;
        inner.running = false;
        inner.active_provider.take();
        true
    }

    pub fn current_publication(&self) -> Option<PreviewPublicationToken> {
        let inner = lock(&self.inner);
        if matches!(
            inner.state,
            PreviewSessionState::Disposed | PreviewSessionState::Cancelled
        ) {
            return None;
        }
        let source_version = inner
            .source_snapshot
            .as_ref()
            .map(|snapshot| snapshot.source_version.clone());
        Some(self.token_for(&inner, source_version))
    }

    pub fn can_publish(&self, token: &PreviewPublicationToken) -> bool {
        let inner = lock(&self.inner);
        self.identity_current_locked(&inner, token)
            && token.is_current()
            && !matches!(
                inner.state,
                PreviewSessionState::Disposed | PreviewSessionState::Cancelled
            )
    }

    /// Runs synchronously for embedders that own their executor/thread.
    pub fn run(
        &self,
        resolver: Arc<dyn SourceResolver>,
        registry: Arc<PreviewProviderRegistry>,
    ) -> Result<PreviewRunOutcome, PreviewRunError> {
        self.run_with_environment(resolver, registry, PreviewProviderEnvironmentHandle::none())
    }

    /// Runs synchronously with an optional injected authoritative read-access
    /// dependency. Provider work still crosses the bounded provider lane, so
    /// a blocking resolver/provider cannot hold the caller past its deadline.
    pub fn run_with_environment(
        &self,
        resolver: Arc<dyn SourceResolver>,
        registry: Arc<PreviewProviderRegistry>,
        environment: PreviewProviderEnvironmentHandle,
    ) -> Result<PreviewRunOutcome, PreviewRunError> {
        let operation = self.begin_operation().map_err(PreviewRunError::Session)?;
        self.run_operation(operation, resolver, registry, environment)
    }

    /// Creates the shell synchronously, then performs resolver/provider work on
    /// a disposable worker. The shell is therefore observable before any slow
    /// provider result is available.
    pub fn start(
        &self,
        resolver: Arc<dyn SourceResolver>,
        registry: Arc<PreviewProviderRegistry>,
    ) -> Result<PreviewTask, PreviewSessionError> {
        self.start_with_environment(resolver, registry, PreviewProviderEnvironmentHandle::none())
    }

    /// Starts a bounded coordinator with an optional injected provider
    /// environment. The coordinator is submitted to the shared execution
    /// boundary; it does not create an OS thread for this session.
    pub fn start_with_environment(
        &self,
        resolver: Arc<dyn SourceResolver>,
        registry: Arc<PreviewProviderRegistry>,
        environment: PreviewProviderEnvironmentHandle,
    ) -> Result<PreviewTask, PreviewSessionError> {
        let operation = self.begin_operation()?;
        let session = self.clone();
        let request_id = operation.token.request_id().to_string();
        let (sender, receiver) = mpsc::sync_channel(1);
        let execution = Arc::clone(&self.execution);
        let work = Box::new(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                session.run_operation(operation, resolver, registry, environment)
            }))
            .unwrap_or(Err(PreviewRunError::WorkerPanicked));
            let _ = sender.send(result);
        });
        if let Err(error) = execution.submit(
            PreviewExecutionLane::Coordinator,
            &format!("preview-{request_id}"),
            work,
        ) {
            self.fail_spawned_operation();
            return Err(error.into());
        }
        Ok(PreviewTask { receiver })
    }

    fn begin_operation(&self) -> Result<OperationSeed, PreviewSessionError> {
        let mut inner = lock(&self.inner);
        if inner.state == PreviewSessionState::Disposed
            || self.authority.disposed.load(Ordering::Acquire)
        {
            return Err(PreviewSessionError::Disposed);
        }
        if inner.running {
            return Err(PreviewSessionError::AlreadyRunning);
        }
        if !matches!(
            inner.state,
            PreviewSessionState::Idle | PreviewSessionState::Resolving
        ) {
            return Err(PreviewSessionError::InvalidState(inner.state));
        }
        inner.state = PreviewSessionState::Resolving;
        inner.running = true;
        let token = self.token_for(&inner, None);
        Ok(OperationSeed {
            request: inner.request.clone(),
            host: inner.host,
            budget: inner.budget,
            token,
        })
    }

    fn token_for(
        &self,
        inner: &SessionInner,
        source_version: Option<String>,
    ) -> PreviewPublicationToken {
        PreviewPublicationToken {
            session_id: inner.session_id.clone(),
            request_id: inner.request.request_id.clone(),
            source_version,
            generation: self.authority.generation.load(Ordering::Acquire),
            authority: Arc::clone(&self.authority),
            cancellation: inner.cancellation.clone(),
        }
    }

    fn fail_spawned_operation(&self) {
        let mut inner = lock(&self.inner);
        if inner.state != PreviewSessionState::Disposed {
            inner.running = false;
            inner.state = PreviewSessionState::Failed;
        }
    }

    fn run_operation(
        &self,
        operation: OperationSeed,
        resolver: Arc<dyn SourceResolver>,
        registry: Arc<PreviewProviderRegistry>,
        environment: PreviewProviderEnvironmentHandle,
    ) -> Result<PreviewRunOutcome, PreviewRunError> {
        let resolve_context = operation.context(None, operation.budget.resolve_timeout);
        let resolve_request = PreviewResolveRequest {
            request_id: operation.request.request_id.clone(),
            source: operation.request.source.clone(),
            host_kind: operation.host.kind,
        };
        let resolve_context_for_worker = resolve_context.clone();
        let resolve_request_for_worker = resolve_request.clone();
        let resolved = match execute_bounded(
            self.execution.as_ref(),
            "preview-source-resolve",
            operation.budget.resolve_timeout,
            move || resolver.resolve(&resolve_request_for_worker, &resolve_context_for_worker),
        ) {
            Ok(resolved) => resolved,
            Err(PreviewExecutionWaitError::TimedOut) => {
                self.mark_failed_if_current(&operation.token);
                return Err(PreviewRunError::SourceResolver(SourceResolveError::Timeout));
            }
            Err(PreviewExecutionWaitError::Panicked) => {
                self.mark_failed_if_current(&operation.token);
                return Err(PreviewRunError::SourceResolver(SourceResolveError::Failed));
            }
            Err(PreviewExecutionWaitError::Execution(error)) => {
                self.mark_failed_if_current(&operation.token);
                return Err(PreviewRunError::ExecutionUnavailable(error));
            }
        };

        let snapshot = match resolved {
            Ok(snapshot) => {
                if let Err(error) = resolve_context.ensure_active() {
                    return match error {
                        PreviewContextError::Cancelled => {
                            self.cancel_if_current(&operation.token, None, Vec::new())
                        }
                        PreviewContextError::StalePublication => {
                            self.stale_or_cancelled(&operation.token)
                        }
                        PreviewContextError::TimedOut => {
                            self.mark_failed_if_current(&operation.token);
                            Err(PreviewRunError::SourceResolver(SourceResolveError::Timeout))
                        }
                    };
                }
                if snapshot.source != operation.request.source || snapshot.source_version.is_empty()
                {
                    self.mark_failed_if_current(&operation.token);
                    return Err(PreviewRunError::SourceResolver(
                        SourceResolveError::SourceMismatch,
                    ));
                }
                snapshot
            }
            Err(error) => {
                if !self.identity_current(&operation.token) {
                    return self.stale_or_cancelled(&operation.token);
                }
                if matches!(error, SourceResolveError::Cancelled)
                    || resolve_context.cancellation.is_cancelled()
                {
                    return self.cancel_if_current(&operation.token, None, Vec::new());
                }
                self.mark_failed_if_current(&operation.token);
                return Err(PreviewRunError::SourceResolver(error));
            }
        };

        let source_token = match self.publish_snapshot(&operation.token, snapshot.clone()) {
            Some(token) => token,
            None => return self.stale_or_cancelled(&operation.token),
        };

        let mut attempted = Vec::new();
        let mut warnings = Vec::new();
        for provider in &registry.providers {
            let (provider_id, provider_capabilities, supports_host, reads_content) = {
                let descriptor = provider.descriptor();
                (
                    descriptor.id.clone(),
                    descriptor.capabilities,
                    descriptor.supports_host(operation.host.kind),
                    descriptor.reads_content,
                )
            };
            if !supports_host {
                continue;
            }
            attempted.push(provider_id.clone());

            if reads_content {
                if let Some(condition) =
                    terminal_condition_for_read_eligibility(snapshot.metadata.read_eligibility)
                {
                    return self.terminal_if_current(
                        &source_token,
                        &snapshot,
                        provider_id,
                        condition,
                        warnings,
                    );
                }
                if snapshot.metadata.read_eligibility != ContentReadEligibility::Eligible {
                    warnings.push(PreviewWarning::ProviderFallback {
                        provider_id,
                        reason: PreviewProviderErrorCode::Unsupported,
                    });
                    continue;
                }
            }

            if !self.can_publish(&source_token) {
                return self.stale_or_cancelled(&source_token);
            }
            self.set_phase(&source_token, PreviewSessionState::Preparing);

            let probe_context = operation_context(
                &source_token,
                operation.host.kind,
                Some(&snapshot.source_version),
                operation.budget.probe_timeout,
            );
            if let Err(error) = probe_context.ensure_active() {
                if error == PreviewContextError::Cancelled {
                    return self.cancel_if_current(&source_token, Some(&snapshot), warnings);
                }
                if error == PreviewContextError::StalePublication {
                    return self.stale_or_cancelled(&source_token);
                }
                warnings.push(PreviewWarning::ProviderFallback {
                    provider_id,
                    reason: PreviewProviderErrorCode::Timeout,
                });
                continue;
            }
            let probe_context_for_worker = probe_context.clone();
            let snapshot_for_probe = snapshot.clone();
            let provider_for_probe = Arc::clone(provider);
            let probe = match execute_bounded(
                self.execution.as_ref(),
                "preview-provider-probe",
                operation.budget.probe_timeout,
                move || provider_for_probe.probe(&snapshot_for_probe, &probe_context_for_worker),
            ) {
                Ok(probe) => probe,
                Err(PreviewExecutionWaitError::TimedOut) => {
                    warnings.push(PreviewWarning::ProviderFallback {
                        provider_id: provider_id.clone(),
                        reason: PreviewProviderErrorCode::Timeout,
                    });
                    continue;
                }
                Err(PreviewExecutionWaitError::Panicked) => {
                    warnings.push(PreviewWarning::ProviderFallback {
                        provider_id: provider_id.clone(),
                        reason: PreviewProviderErrorCode::Failed,
                    });
                    continue;
                }
                Err(PreviewExecutionWaitError::Execution(error)) => {
                    self.mark_failed_if_current(&source_token);
                    return Err(PreviewRunError::ExecutionUnavailable(error));
                }
            };
            match probe_context.ensure_active() {
                Ok(()) => {}
                Err(PreviewContextError::Cancelled) => {
                    return self.cancel_if_current(&source_token, Some(&snapshot), warnings);
                }
                Err(PreviewContextError::StalePublication) => {
                    return self.stale_or_cancelled(&source_token);
                }
                Err(PreviewContextError::TimedOut) => {
                    warnings.push(PreviewWarning::ProviderFallback {
                        provider_id: provider_id.clone(),
                        reason: PreviewProviderErrorCode::Timeout,
                    });
                    continue;
                }
            }
            if probe == ProviderProbe::Unsupported {
                warnings.push(PreviewWarning::ProviderFallback {
                    provider_id: provider_id.clone(),
                    reason: PreviewProviderErrorCode::Unsupported,
                });
                continue;
            }

            let prepare_context = operation_context(
                &source_token,
                operation.host.kind,
                Some(&snapshot.source_version),
                operation.budget.prepare_timeout,
            );
            match prepare_context.ensure_active() {
                Ok(()) => {}
                Err(PreviewContextError::Cancelled) => {
                    return self.cancel_if_current(&source_token, Some(&snapshot), warnings);
                }
                Err(PreviewContextError::StalePublication) => {
                    return self.stale_or_cancelled(&source_token);
                }
                Err(PreviewContextError::TimedOut) => {
                    warnings.push(PreviewWarning::ProviderFallback {
                        provider_id: provider_id.clone(),
                        reason: PreviewProviderErrorCode::Timeout,
                    });
                    continue;
                }
            }
            let prepare_context_for_worker = prepare_context.clone();
            let snapshot_for_prepare = snapshot.clone();
            let provider_for_prepare = Arc::clone(provider);
            let prepared = match execute_bounded(
                self.execution.as_ref(),
                "preview-provider-prepare",
                operation.budget.prepare_timeout,
                move || {
                    provider_for_prepare
                        .prepare(&snapshot_for_prepare, &prepare_context_for_worker)
                        .map(PreparedPreviewGuard::new)
                },
            ) {
                Ok(Ok(prepared)) => prepared,
                Ok(Err(mut error)) => {
                    error = match prepare_context.ensure_active() {
                        Ok(()) => error,
                        Err(PreviewContextError::Cancelled) => {
                            return self.cancel_if_current(
                                &source_token,
                                Some(&snapshot),
                                warnings,
                            );
                        }
                        Err(PreviewContextError::StalePublication) => {
                            return self.stale_or_cancelled(&source_token);
                        }
                        Err(PreviewContextError::TimedOut) => PreviewProviderError::Timeout,
                    };
                    if error == PreviewProviderError::Cancelled {
                        return self.cancel_if_current(&source_token, Some(&snapshot), warnings);
                    }
                    if let Some(condition) = error.terminal_condition() {
                        return self.terminal_if_current(
                            &source_token,
                            &snapshot,
                            provider_id,
                            condition,
                            warnings,
                        );
                    }
                    warnings.push(PreviewWarning::ProviderFallback {
                        provider_id,
                        reason: error.code(),
                    });
                    continue;
                }
                Err(PreviewExecutionWaitError::TimedOut) => {
                    warnings.push(PreviewWarning::ProviderFallback {
                        provider_id: provider_id.clone(),
                        reason: PreviewProviderErrorCode::Timeout,
                    });
                    continue;
                }
                Err(PreviewExecutionWaitError::Panicked) => {
                    warnings.push(PreviewWarning::ProviderFallback {
                        provider_id: provider_id.clone(),
                        reason: PreviewProviderErrorCode::Failed,
                    });
                    continue;
                }
                Err(PreviewExecutionWaitError::Execution(error)) => {
                    self.mark_failed_if_current(&source_token);
                    return Err(PreviewRunError::ExecutionUnavailable(error));
                }
            };
            if let Err(error) = prepare_context.ensure_active() {
                drop(prepared);
                if error == PreviewContextError::Cancelled {
                    return self.cancel_if_current(&source_token, Some(&snapshot), warnings);
                }
                if error == PreviewContextError::StalePublication {
                    return self.stale_or_cancelled(&source_token);
                }
                warnings.push(PreviewWarning::ProviderFallback {
                    provider_id,
                    reason: PreviewProviderErrorCode::Timeout,
                });
                continue;
            }

            if !self.install_active_provider(&source_token, provider_id.clone()) {
                drop(prepared);
                return self.stale_or_cancelled(&source_token);
            }
            self.set_phase(&source_token, PreviewSessionState::Loading);
            let load_context = operation_context(
                &source_token,
                operation.host.kind,
                Some(&snapshot.source_version),
                operation.budget.load_timeout,
            );
            let load_context_for_worker = load_context.clone();
            let environment_for_worker = environment.clone();
            let publication_enabled = Arc::new(AtomicBool::new(true));
            let publication_sink = Arc::new(SessionPublicationSink {
                session: self.clone(),
                token: source_token.clone(),
                provider_id: provider_id.clone(),
                host: operation.host.kind,
                provider_capabilities,
                context: load_context.clone(),
                enabled: Arc::clone(&publication_enabled),
            });
            let loaded = execute_bounded(
                self.execution.as_ref(),
                "preview-provider-load",
                operation.budget.load_timeout,
                move || {
                    let mut prepared = prepared;
                    let provider_environment = PreviewProviderEnvironment {
                        content_read: environment_for_worker.content_read.as_deref(),
                        preview_read: environment_for_worker.preview_read.as_deref(),
                        folder_enumeration: environment_for_worker.folder_enumeration.as_deref(),
                        publication: Some(publication_sink.as_ref()),
                        asset_publisher: environment_for_worker.asset_publisher.as_deref(),
                        decoder_admission: environment_for_worker.decoder_admission.as_deref(),
                        archive_admission: environment_for_worker.archive_admission.as_deref(),
                    };
                    let loaded = prepared.load(&load_context_for_worker, provider_environment);
                    prepared.cleanup_once();
                    loaded
                },
            );
            publication_enabled.store(false, Ordering::Release);

            let loaded = match loaded {
                Ok(loaded) => loaded,
                Err(PreviewExecutionWaitError::TimedOut) => {
                    self.clear_active_provider(&source_token, &provider_id);
                    if !self.can_publish(&source_token) {
                        return self.stale_or_cancelled(&source_token);
                    }
                    warnings.push(PreviewWarning::ProviderFallback {
                        provider_id,
                        reason: PreviewProviderErrorCode::Timeout,
                    });
                    continue;
                }
                Err(PreviewExecutionWaitError::Panicked) => {
                    self.clear_active_provider(&source_token, &provider_id);
                    if !self.can_publish(&source_token) {
                        return self.stale_or_cancelled(&source_token);
                    }
                    warnings.push(PreviewWarning::ProviderFallback {
                        provider_id,
                        reason: PreviewProviderErrorCode::Failed,
                    });
                    continue;
                }
                Err(PreviewExecutionWaitError::Execution(error)) => {
                    self.clear_active_provider(&source_token, &provider_id);
                    self.mark_failed_if_current(&source_token);
                    return Err(PreviewRunError::ExecutionUnavailable(error));
                }
            };

            if !self.can_publish(&source_token) {
                self.clear_active_provider(&source_token, &provider_id);
                return self.stale_or_cancelled(&source_token);
            }
            if let Err(error) = load_context.ensure_active() {
                self.clear_active_provider(&source_token, &provider_id);
                if error == PreviewContextError::Cancelled {
                    return self.cancel_if_current(&source_token, Some(&snapshot), warnings);
                }
                if error == PreviewContextError::StalePublication {
                    return self.stale_or_cancelled(&source_token);
                }
                warnings.push(PreviewWarning::ProviderFallback {
                    provider_id,
                    reason: PreviewProviderErrorCode::Timeout,
                });
                continue;
            }

            match loaded {
                Ok(mut result) => {
                    // Preserve the coordinator-owned fallback history when a
                    // later provider succeeds. Provider-local failures are
                    // recoverable, but hiding them would make the final wire
                    // indistinguishable from a first-provider success.
                    if !warnings.is_empty() {
                        let mut ordered_warnings = std::mem::take(&mut warnings);
                        ordered_warnings.append(&mut result.warnings);
                        result.warnings = ordered_warnings;
                    }
                    let sequence = match self.next_publication_sequence(&source_token) {
                        Ok(sequence) => sequence,
                        Err(PreviewPublicationError::StalePublication) => {
                            self.clear_active_provider(&source_token, &provider_id);
                            return self.stale_or_cancelled(&source_token);
                        }
                        Err(_) => {
                            self.clear_active_provider(&source_token, &provider_id);
                            warnings.push(PreviewWarning::ProviderFallback {
                                provider_id,
                                reason: PreviewProviderErrorCode::Failed,
                            });
                            continue;
                        }
                    };
                    let update = PreviewPublicationUpdate { sequence, result };
                    match self.publish_progressive_update(
                        &source_token,
                        &provider_id,
                        operation.host.kind,
                        provider_capabilities,
                        update,
                        true,
                    ) {
                        Ok(envelope) => {
                            return Ok(PreviewRunOutcome {
                                provider_id: Some(provider_id),
                                envelope,
                                attempted_provider_ids: attempted,
                            });
                        }
                        Err(PreviewPublicationError::StalePublication) => {
                            self.clear_active_provider(&source_token, &provider_id);
                            return self.stale_or_cancelled(&source_token);
                        }
                        Err(PreviewPublicationError::HostIncompatible) => {
                            self.clear_active_provider(&source_token, &provider_id);
                            warnings.push(PreviewWarning::ProviderFallback {
                                provider_id,
                                reason: PreviewProviderErrorCode::Unsupported,
                            });
                        }
                        Err(
                            PreviewPublicationError::OutOfOrder
                            | PreviewPublicationError::InvalidSequence,
                        ) => {
                            self.clear_active_provider(&source_token, &provider_id);
                            warnings.push(PreviewWarning::ProviderFallback {
                                provider_id,
                                reason: PreviewProviderErrorCode::Failed,
                            });
                        }
                    }
                }
                Err(error) => {
                    self.clear_active_provider(&source_token, &provider_id);
                    if error == PreviewProviderError::Cancelled {
                        return self.cancel_if_current(&source_token, Some(&snapshot), warnings);
                    }
                    if let Some(condition) = error.terminal_condition() {
                        return self.terminal_if_current(
                            &source_token,
                            &snapshot,
                            provider_id,
                            condition,
                            warnings,
                        );
                    }
                    warnings.push(PreviewWarning::ProviderFallback {
                        provider_id,
                        reason: error.code(),
                    });
                }
            }
        }

        let envelope = metadata_fallback(&snapshot, operation.host, warnings);
        if !self.publish_fallback(&source_token, envelope.clone(), PreviewSessionState::Ready) {
            return self.stale_or_cancelled(&source_token);
        }
        Ok(PreviewRunOutcome {
            provider_id: None,
            envelope,
            attempted_provider_ids: attempted,
        })
    }

    fn publish_snapshot(
        &self,
        token: &PreviewPublicationToken,
        snapshot: PreviewSourceSnapshot,
    ) -> Option<PreviewPublicationToken> {
        let mut inner = lock(&self.inner);
        if !self.identity_current_locked(&inner, token) || !token.is_current() {
            return None;
        }
        let source_token = token.with_source_version(snapshot.source_version.clone());
        inner.source_snapshot = Some(snapshot);
        inner.representation = None;
        inner.publication_sequence.reset();
        inner.effective_capabilities = PreviewCapabilities::default();
        inner.state = PreviewSessionState::Preparing;
        Some(source_token)
    }

    fn next_publication_sequence(
        &self,
        token: &PreviewPublicationToken,
    ) -> Result<u64, PreviewPublicationError> {
        let inner = lock(&self.inner);
        if !self.identity_current_locked(&inner, token) || !token.is_current() {
            return Err(PreviewPublicationError::StalePublication);
        }
        inner.publication_sequence.next()
    }

    fn publish_progressive_update(
        &self,
        token: &PreviewPublicationToken,
        provider_id: &str,
        host: PreviewHostKind,
        provider_capabilities: PreviewCapabilities,
        update: PreviewPublicationUpdate,
        final_result: bool,
    ) -> Result<PreviewRepresentationEnvelope, PreviewPublicationError> {
        let mut inner = lock(&self.inner);
        if !self.identity_current_locked(&inner, token) || !token.is_current() {
            return Err(PreviewPublicationError::StalePublication);
        }
        if !update.result.representation.is_host_compatible(host) {
            return Err(PreviewPublicationError::HostIncompatible);
        }
        let source_version = inner
            .source_snapshot
            .as_ref()
            .map(|snapshot| snapshot.source_version.clone())
            .ok_or(PreviewPublicationError::StalePublication)?;
        if token.source_version() != Some(source_version.as_str()) {
            return Err(PreviewPublicationError::StalePublication);
        }
        inner.publication_sequence.accept(update.sequence)?;
        let envelope = PreviewRepresentationEnvelope {
            source_version,
            representation: update.result.representation,
            completeness: update.result.completeness,
            warnings: update.result.warnings,
            capabilities: inner
                .host
                .capabilities
                .intersect(
                    inner
                        .source_snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.capabilities)
                        .unwrap_or_default(),
                )
                .intersect(provider_capabilities),
        };
        inner.representation = Some(envelope.clone());
        inner.effective_capabilities = envelope.capabilities;
        if let Some(active) = inner.active_provider.as_mut() {
            active.id = provider_id.to_string();
        }
        inner.state = if final_result || envelope.completeness == PreviewCompleteness::Complete {
            PreviewSessionState::Ready
        } else {
            PreviewSessionState::Loading
        };
        if final_result {
            inner.running = false;
        }
        Ok(envelope)
    }

    fn publish_fallback(
        &self,
        token: &PreviewPublicationToken,
        envelope: PreviewRepresentationEnvelope,
        state: PreviewSessionState,
    ) -> bool {
        let mut inner = lock(&self.inner);
        if !self.identity_current_locked(&inner, token) || !token.is_current() {
            return false;
        }
        inner.effective_capabilities = envelope.capabilities;
        inner.representation = Some(envelope);
        inner.state = state;
        inner.running = false;
        true
    }

    fn set_phase(&self, token: &PreviewPublicationToken, state: PreviewSessionState) {
        let mut inner = lock(&self.inner);
        if self.identity_current_locked(&inner, token) && token.is_current() {
            inner.state = state;
        }
    }

    fn install_active_provider(&self, token: &PreviewPublicationToken, id: String) -> bool {
        let mut inner = lock(&self.inner);
        if !self.identity_current_locked(&inner, token) || !token.is_current() {
            return false;
        }
        inner.active_provider = Some(ActiveProvider { id });
        true
    }

    fn clear_active_provider(&self, token: &PreviewPublicationToken, provider_id: &str) {
        let mut inner = lock(&self.inner);
        if self.identity_current_locked(&inner, token)
            && inner
                .active_provider
                .as_ref()
                .is_some_and(|active| active.id == provider_id)
        {
            inner.active_provider.take();
        }
    }

    fn identity_current(&self, token: &PreviewPublicationToken) -> bool {
        let inner = lock(&self.inner);
        self.identity_current_locked(&inner, token)
    }

    fn identity_current_locked(
        &self,
        inner: &SessionInner,
        token: &PreviewPublicationToken,
    ) -> bool {
        if self.authority.disposed.load(Ordering::Acquire)
            || inner.state == PreviewSessionState::Disposed
            || self.authority.generation.load(Ordering::Acquire) != token.generation
            || inner.request.request_id != token.request_id
        {
            return false;
        }
        match token.source_version.as_deref() {
            Some(version) => inner
                .source_snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.source_version == version),
            None => true,
        }
    }

    fn mark_failed_if_current(&self, token: &PreviewPublicationToken) {
        let mut inner = lock(&self.inner);
        if self.identity_current_locked(&inner, token) {
            inner.cancellation.cancel();
            self.authority.generation.fetch_add(1, Ordering::AcqRel);
            inner.running = false;
            inner.state = PreviewSessionState::Failed;
            inner.active_provider.take();
        }
    }

    fn stale_or_cancelled(
        &self,
        token: &PreviewPublicationToken,
    ) -> Result<PreviewRunOutcome, PreviewRunError> {
        let inner = lock(&self.inner);
        if inner.request.request_id == token.request_id
            && inner.state == PreviewSessionState::Cancelled
        {
            Err(PreviewRunError::Cancelled)
        } else {
            Err(PreviewRunError::StalePublication)
        }
    }

    fn cancel_if_current(
        &self,
        token: &PreviewPublicationToken,
        snapshot: Option<&PreviewSourceSnapshot>,
        warnings: Vec<PreviewWarning>,
    ) -> Result<PreviewRunOutcome, PreviewRunError> {
        let cancelled = {
            let mut inner = lock(&self.inner);
            if !self.identity_current_locked(&inner, token) {
                false
            } else {
                inner.cancellation.cancel();
                self.authority.generation.fetch_add(1, Ordering::AcqRel);
                inner.state = PreviewSessionState::Cancelled;
                inner.running = false;
                if let Some(snapshot) = snapshot.or(inner.source_snapshot.as_ref()) {
                    let envelope = metadata_fallback(snapshot, inner.host, warnings);
                    inner.effective_capabilities = envelope.capabilities;
                    inner.representation = Some(envelope);
                }
                let _ = inner.active_provider.take();
                true
            }
        };
        if !cancelled {
            return self.stale_or_cancelled(token);
        }
        Err(PreviewRunError::Cancelled)
    }

    fn terminal_if_current(
        &self,
        token: &PreviewPublicationToken,
        snapshot: &PreviewSourceSnapshot,
        provider_id: String,
        condition: PreviewTerminalCondition,
        mut warnings: Vec<PreviewWarning>,
    ) -> Result<PreviewRunOutcome, PreviewRunError> {
        warnings.push(PreviewWarning::TerminalCondition { condition });
        let envelope = metadata_fallback(snapshot, self.host(), warnings);
        if !self.publish_fallback(token, envelope, PreviewSessionState::Failed) {
            return self.stale_or_cancelled(token);
        }
        Err(PreviewRunError::ProviderTerminal {
            provider_id,
            condition,
        })
    }
}

impl PreviewPublicationSink for SessionPublicationSink {
    fn publish(&self, update: PreviewPublicationUpdate) -> Result<(), PreviewPublicationError> {
        if !self.enabled.load(Ordering::Acquire) {
            return Err(PreviewPublicationError::StalePublication);
        }
        self.context
            .ensure_active()
            .map_err(|_| PreviewPublicationError::StalePublication)?;
        self.session
            .publish_progressive_update(
                &self.token,
                &self.provider_id,
                self.host,
                self.provider_capabilities,
                update,
                false,
            )
            .map(|_| ())
    }

    fn publish_next(&self, result: PreviewProviderResult) -> Result<(), PreviewPublicationError> {
        let sequence = self.session.next_publication_sequence(&self.token)?;
        self.publish(PreviewPublicationUpdate { sequence, result })
    }
}

fn operation_context(
    token: &PreviewPublicationToken,
    host: PreviewHostKind,
    source_version: Option<&str>,
    timeout: Duration,
) -> PreviewOperationContext {
    PreviewOperationContext {
        session_id: token.session_id.clone(),
        request_id: token.request_id.clone(),
        host,
        source_version: source_version.map(str::to_owned),
        publication: match source_version {
            Some(version) => token.with_source_version(version),
            None => token.clone(),
        },
        cancellation: token.cancellation.clone(),
        deadline: Instant::now() + timeout,
    }
}

fn metadata_fallback(
    snapshot: &PreviewSourceSnapshot,
    host: PreviewHost,
    mut warnings: Vec<PreviewWarning>,
) -> PreviewRepresentationEnvelope {
    warnings.push(PreviewWarning::MetadataFallback);
    PreviewRepresentationEnvelope {
        source_version: snapshot.source_version.clone(),
        representation: PreviewRepresentation::Metadata {
            metadata: snapshot.metadata.clone(),
        },
        completeness: PreviewCompleteness::Complete,
        warnings,
        capabilities: host
            .capabilities
            .intersect(snapshot.capabilities)
            .intersect(PreviewCapabilities::metadata_fallback()),
    }
}

fn terminal_condition_for_read_eligibility(
    eligibility: ContentReadEligibility,
) -> Option<PreviewTerminalCondition> {
    match eligibility {
        ContentReadEligibility::MaterializationRequired | ContentReadEligibility::Downloading => {
            Some(PreviewTerminalCondition::MaterializationRequired)
        }
        ContentReadEligibility::PermissionRequired => {
            Some(PreviewTerminalCondition::PermissionDenied)
        }
        ContentReadEligibility::SourceUnavailable | ContentReadEligibility::AvailabilityUnknown => {
            Some(PreviewTerminalCondition::SourceUnavailable)
        }
        ContentReadEligibility::IdentityChanged => Some(PreviewTerminalCondition::IdentityChanged),
        ContentReadEligibility::Eligible
        | ContentReadEligibility::MetadataOnly
        | ContentReadEligibility::SourceNotSupported
        | ContentReadEligibility::PackageUnsupported
        | ContentReadEligibility::Symlink => None,
    }
}

#[cfg(test)]
#[path = "preview_publication_tests.rs"]
mod publication_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    pub(super) fn source(id: &str) -> PreviewSourceRef {
        PreviewSourceRef::Ephemeral {
            browse_session_id: "browse-1".to_string(),
            entry_id: id.to_string(),
        }
    }

    fn metadata(name: &str) -> PreviewMetadata {
        PreviewMetadata {
            display_name: name.to_string(),
            media_type: Some("text/plain".to_string()),
            extension: Some("txt".to_string()),
            size_bytes: Some(12),
            modified_at_epoch_ms: Some(1),
            materialization: MaterializationState::Local,
            read_eligibility: ContentReadEligibility::Eligible,
        }
    }

    fn snapshot(source: PreviewSourceRef, version: &str) -> PreviewSourceSnapshot {
        PreviewSourceSnapshot::new(
            source,
            version,
            metadata("sample.txt"),
            PreviewCapabilities::all(),
        )
    }

    pub(super) fn text_result(text: &str) -> PreviewProviderResult {
        PreviewProviderResult {
            representation: PreviewRepresentation::Text {
                text: text.to_string(),
                language: Some("text".to_string()),
            },
            completeness: PreviewCompleteness::Complete,
            warnings: Vec::new(),
        }
    }

    pub(super) fn partial_text_result(text: &str) -> PreviewProviderResult {
        PreviewProviderResult {
            representation: PreviewRepresentation::Text {
                text: text.to_string(),
                language: Some("text".to_string()),
            },
            completeness: PreviewCompleteness::Partial,
            warnings: Vec::new(),
        }
    }

    fn host() -> PreviewHost {
        PreviewHost::new(PreviewHostKind::ZenFloating, PreviewCapabilities::all())
    }

    pub(super) fn session(entry_id: &str) -> PreviewSession {
        PreviewSession::new(PreviewSessionConfig::new(
            "session-1",
            "request-1",
            source(entry_id),
            host(),
        ))
    }

    pub(super) struct FakeResolver {
        snapshot: PreviewSourceSnapshot,
    }

    impl SourceResolver for FakeResolver {
        fn resolve(
            &self,
            request: &PreviewResolveRequest,
            context: &PreviewOperationContext,
        ) -> Result<PreviewSourceSnapshot, SourceResolveError> {
            context.ensure_active().map_err(|error| match error {
                PreviewContextError::Cancelled => SourceResolveError::Cancelled,
                PreviewContextError::TimedOut => SourceResolveError::Timeout,
                PreviewContextError::StalePublication => SourceResolveError::Cancelled,
            })?;
            if request.source != self.snapshot.source {
                return Err(SourceResolveError::SourceMismatch);
            }
            Ok(self.snapshot.clone())
        }
    }

    struct FakeContentRead;

    impl ContentReadLeaseConsumer for FakeContentRead {
        fn read_bounded(
            &self,
            lease: &ContentReadLeaseRef,
            request: BoundedContentReadRequest,
            context: &PreviewOperationContext,
        ) -> Result<BoundedContentRead, ContentReadAccessError> {
            context.ensure_active().map_err(|error| match error {
                PreviewContextError::Cancelled => ContentReadAccessError::Cancelled,
                PreviewContextError::TimedOut => ContentReadAccessError::TimedOut,
                PreviewContextError::StalePublication => {
                    ContentReadAccessError::SourceVersionMismatch
                }
            })?;
            if lease.source_version != "version-lease" || request.max_bytes != 4 {
                return Err(ContentReadAccessError::LeaseInvalid);
            }
            Ok(BoundedContentRead {
                bytes: b"data".to_vec(),
                complete: true,
            })
        }
    }

    struct FakePrepared {
        load_result: Result<PreviewProviderResult, PreviewProviderError>,
        cleanup_count: Arc<AtomicUsize>,
        started: Option<Arc<AtomicBool>>,
        wait_for_cancel: bool,
    }

    impl PreparedPreview for FakePrepared {
        fn load(
            &mut self,
            context: &PreviewOperationContext,
            _environment: PreviewProviderEnvironment<'_>,
        ) -> Result<PreviewProviderResult, PreviewProviderError> {
            if let Some(started) = &self.started {
                started.store(true, Ordering::Release);
            }
            if self.wait_for_cancel {
                while !context.cancellation().is_cancelled() && context.is_publication_current() {
                    thread::yield_now();
                }
                return Err(PreviewProviderError::Cancelled);
            }
            self.load_result.clone()
        }

        fn cleanup(&mut self) {
            self.cleanup_count.fetch_add(1, Ordering::AcqRel);
        }
    }

    struct FakeProvider {
        descriptor: PreviewProviderDescriptor,
        probe: ProviderProbe,
        prepare_error: Option<PreviewProviderError>,
        load_result: Result<PreviewProviderResult, PreviewProviderError>,
        prepare_calls: AtomicUsize,
        cleanup_count: Arc<AtomicUsize>,
        started: Option<Arc<AtomicBool>>,
        wait_for_cancel: bool,
    }

    impl PreviewProvider for FakeProvider {
        fn descriptor(&self) -> &PreviewProviderDescriptor {
            &self.descriptor
        }

        fn probe(
            &self,
            _snapshot: &PreviewSourceSnapshot,
            _context: &PreviewOperationContext,
        ) -> ProviderProbe {
            self.probe
        }

        fn prepare(
            &self,
            _snapshot: &PreviewSourceSnapshot,
            _context: &PreviewOperationContext,
        ) -> Result<Box<dyn PreparedPreview>, PreviewProviderError> {
            self.prepare_calls.fetch_add(1, Ordering::AcqRel);
            if let Some(error) = self.prepare_error {
                return Err(error);
            }
            Ok(Box::new(FakePrepared {
                load_result: self.load_result.clone(),
                cleanup_count: Arc::clone(&self.cleanup_count),
                started: self.started.clone(),
                wait_for_cancel: self.wait_for_cancel,
            }))
        }
    }

    struct GatedResolver {
        snapshot: PreviewSourceSnapshot,
        started: Arc<AtomicBool>,
        release: Arc<AtomicBool>,
    }

    impl SourceResolver for GatedResolver {
        fn resolve(
            &self,
            _request: &PreviewResolveRequest,
            _context: &PreviewOperationContext,
        ) -> Result<PreviewSourceSnapshot, SourceResolveError> {
            self.started.store(true, Ordering::Release);
            while !self.release.load(Ordering::Acquire) {
                thread::yield_now();
            }
            Ok(self.snapshot.clone())
        }
    }

    struct GatedPrepared {
        started: Arc<AtomicBool>,
        release: Arc<AtomicBool>,
        cleanup_count: Arc<AtomicUsize>,
    }

    impl PreparedPreview for GatedPrepared {
        fn load(
            &mut self,
            _context: &PreviewOperationContext,
            _environment: PreviewProviderEnvironment<'_>,
        ) -> Result<PreviewProviderResult, PreviewProviderError> {
            self.started.store(true, Ordering::Release);
            while !self.release.load(Ordering::Acquire) {
                thread::yield_now();
            }
            Ok(text_result("late-provider-result"))
        }

        fn cleanup(&mut self) {
            self.cleanup_count.fetch_add(1, Ordering::AcqRel);
        }
    }

    struct GatedProvider {
        descriptor: PreviewProviderDescriptor,
        started: Arc<AtomicBool>,
        release: Arc<AtomicBool>,
        cleanup_count: Arc<AtomicUsize>,
    }

    impl PreviewProvider for GatedProvider {
        fn descriptor(&self) -> &PreviewProviderDescriptor {
            &self.descriptor
        }

        fn probe(
            &self,
            _snapshot: &PreviewSourceSnapshot,
            _context: &PreviewOperationContext,
        ) -> ProviderProbe {
            ProviderProbe::Compatible
        }

        fn prepare(
            &self,
            _snapshot: &PreviewSourceSnapshot,
            _context: &PreviewOperationContext,
        ) -> Result<Box<dyn PreparedPreview>, PreviewProviderError> {
            Ok(Box::new(GatedPrepared {
                started: Arc::clone(&self.started),
                release: Arc::clone(&self.release),
                cleanup_count: Arc::clone(&self.cleanup_count),
            }))
        }
    }

    struct EnvironmentAwarePrepared {
        saw_content_read: Arc<AtomicBool>,
    }

    impl PreparedPreview for EnvironmentAwarePrepared {
        fn load(
            &mut self,
            _context: &PreviewOperationContext,
            environment: PreviewProviderEnvironment<'_>,
        ) -> Result<PreviewProviderResult, PreviewProviderError> {
            self.saw_content_read
                .store(environment.content_read.is_some(), Ordering::Release);
            Ok(text_result("environment-aware"))
        }

        fn cleanup(&mut self) {}
    }

    struct EnvironmentAwareProvider {
        descriptor: PreviewProviderDescriptor,
        saw_content_read: Arc<AtomicBool>,
    }

    impl PreviewProvider for EnvironmentAwareProvider {
        fn descriptor(&self) -> &PreviewProviderDescriptor {
            &self.descriptor
        }

        fn probe(
            &self,
            _snapshot: &PreviewSourceSnapshot,
            _context: &PreviewOperationContext,
        ) -> ProviderProbe {
            ProviderProbe::Compatible
        }

        fn prepare(
            &self,
            _snapshot: &PreviewSourceSnapshot,
            _context: &PreviewOperationContext,
        ) -> Result<Box<dyn PreparedPreview>, PreviewProviderError> {
            Ok(Box::new(EnvironmentAwarePrepared {
                saw_content_read: Arc::clone(&self.saw_content_read),
            }))
        }
    }

    fn fake_provider(
        id: &str,
        priority: i32,
        probe: ProviderProbe,
        prepare_error: Option<PreviewProviderError>,
        load_result: Result<PreviewProviderResult, PreviewProviderError>,
    ) -> Arc<FakeProvider> {
        Arc::new(FakeProvider {
            descriptor: PreviewProviderDescriptor::new(
                id,
                priority,
                PreviewCapabilities::all(),
                vec![PreviewHostKind::ZenFloating],
                true,
            ),
            probe,
            prepare_error,
            load_result,
            prepare_calls: AtomicUsize::new(0),
            cleanup_count: Arc::new(AtomicUsize::new(0)),
            started: None,
            wait_for_cancel: false,
        })
    }

    pub(super) fn registry<P>(providers: Vec<Arc<P>>) -> Arc<PreviewProviderRegistry>
    where
        P: PreviewProvider + 'static,
    {
        let providers: Vec<Arc<dyn PreviewProvider>> = providers
            .into_iter()
            .map(|provider| provider as Arc<dyn PreviewProvider>)
            .collect();
        Arc::new(PreviewProviderRegistry::new(providers).expect("fake provider registry"))
    }

    fn empty_registry() -> Arc<PreviewProviderRegistry> {
        Arc::new(
            PreviewProviderRegistry::new(Vec::<Arc<dyn PreviewProvider>>::new())
                .expect("empty provider registry"),
        )
    }

    pub(super) fn resolver(entry_id: &str, version: &str) -> Arc<FakeResolver> {
        Arc::new(FakeResolver {
            snapshot: snapshot(source(entry_id), version),
        })
    }

    fn session_with_budget(entry_id: &str, budget: PreviewWorkBudget) -> PreviewSession {
        PreviewSession::new(PreviewSessionConfig {
            session_id: format!("session-{entry_id}"),
            request: PreviewRequest {
                request_id: format!("request-{entry_id}"),
                source: source(entry_id),
            },
            host: host(),
            budget,
        })
    }

    pub(super) fn wait_until(flag: &AtomicBool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !flag.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(flag.load(Ordering::Acquire), "fake provider did not start");
    }

    fn wait_until_state(session: &PreviewSession, expected: PreviewSessionState) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while session.state() != expected && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        session.state() == expected
    }

    fn wait_until_cleanup_count(counter: &AtomicUsize, expected: usize) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while counter.load(Ordering::Acquire) < expected && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        counter.load(Ordering::Acquire) == expected
    }

    pub(super) fn wait_until_representation(session: &PreviewSession) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while session.representation().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(
            session.representation().is_some(),
            "progressive representation missing"
        );
    }

    fn gated_provider(
        id: &str,
    ) -> (
        Arc<GatedProvider>,
        Arc<AtomicBool>,
        Arc<AtomicBool>,
        Arc<AtomicUsize>,
    ) {
        let started = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let cleanup_count = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(GatedProvider {
            descriptor: PreviewProviderDescriptor::new(
                id,
                100,
                PreviewCapabilities::all(),
                vec![PreviewHostKind::ZenFloating],
                true,
            ),
            started: Arc::clone(&started),
            release: Arc::clone(&release),
            cleanup_count: Arc::clone(&cleanup_count),
        });
        (provider, started, release, cleanup_count)
    }

    #[test]
    fn shell_and_session_exist_before_slow_provider_result() {
        let started = Arc::new(AtomicBool::new(false));
        let cleanup_count = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(FakeProvider {
            descriptor: PreviewProviderDescriptor::new(
                "slow",
                10,
                PreviewCapabilities::all(),
                vec![PreviewHostKind::ZenFloating],
                true,
            ),
            probe: ProviderProbe::Compatible,
            prepare_error: None,
            load_result: Ok(text_result("slow")),
            prepare_calls: AtomicUsize::new(0),
            cleanup_count: Arc::clone(&cleanup_count),
            started: Some(Arc::clone(&started)),
            wait_for_cancel: true,
        });
        let session = session("entry-a");
        let task = session
            .start(resolver("entry-a", "version-a"), registry(vec![provider]))
            .expect("preview worker starts");

        assert!(matches!(
            session.state(),
            PreviewSessionState::Resolving
                | PreviewSessionState::Preparing
                | PreviewSessionState::Loading
        ));
        wait_until(&started);
        assert!(session.representation().is_none());
        assert!(session.cancel());
        assert!(matches!(task.join(), Err(PreviewRunError::Cancelled)));
        assert_eq!(session.state(), PreviewSessionState::Cancelled);
        assert_eq!(cleanup_count.load(Ordering::Acquire), 1);
    }

    #[test]
    fn resolver_timeout_is_observed_while_blocked_worker_is_unreleased() {
        let started = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let resolver = Arc::new(GatedResolver {
            snapshot: snapshot(source("entry-resolver-timeout"), "version-resolver-timeout"),
            started: Arc::clone(&started),
            release: Arc::clone(&release),
        });
        let session = session_with_budget(
            "entry-resolver-timeout",
            PreviewWorkBudget {
                resolve_timeout: Duration::from_millis(25),
                ..PreviewWorkBudget::default()
            },
        );
        let task = session
            .start(resolver, empty_registry())
            .expect("bounded coordinator starts");
        wait_until(&started);

        let timed_out_while_blocked = wait_until_state(&session, PreviewSessionState::Failed);
        assert!(session.representation().is_none());
        release.store(true, Ordering::Release);
        let result = task.join();

        assert!(timed_out_while_blocked);
        assert!(matches!(
            result,
            Err(PreviewRunError::SourceResolver(SourceResolveError::Timeout))
        ));
        assert!(session.representation().is_none());
    }

    #[test]
    fn provider_timeout_falls_back_while_blocked_load_later_cleans_once() {
        let (provider, started, release, cleanup_count) = gated_provider("gated-timeout");
        let session = session_with_budget(
            "entry-provider-timeout",
            PreviewWorkBudget {
                load_timeout: Duration::from_millis(25),
                ..PreviewWorkBudget::default()
            },
        );
        let task = session
            .start(
                resolver("entry-provider-timeout", "version-provider-timeout"),
                registry(vec![provider]),
            )
            .expect("bounded coordinator starts");
        wait_until(&started);

        let fallback_while_blocked = wait_until_state(&session, PreviewSessionState::Ready);
        assert_eq!(cleanup_count.load(Ordering::Acquire), 0);
        assert!(matches!(
            session
                .representation()
                .map(|envelope| envelope.representation),
            Some(PreviewRepresentation::Metadata { .. })
        ));
        release.store(true, Ordering::Release);
        let outcome = task.join().expect("metadata fallback completes");

        assert!(fallback_while_blocked);
        assert!(outcome.provider_id.is_none());
        assert!(matches!(
            outcome.envelope.representation,
            PreviewRepresentation::Metadata { .. }
        ));
        assert!(wait_until_cleanup_count(&cleanup_count, 1));
        assert_eq!(cleanup_count.load(Ordering::Acquire), 1);
        assert!(matches!(
            session
                .representation()
                .map(|envelope| envelope.representation),
            Some(PreviewRepresentation::Metadata { .. })
        ));
    }

    #[test]
    fn controls_revoke_immediately_without_waiting_for_gated_load() {
        let cases = ["cancel", "switch", "dispose"];
        for case in cases {
            let (provider, started, release, cleanup_count) = gated_provider(case);
            let entry_id = format!("entry-control-{case}");
            let session = session_with_budget(
                &entry_id,
                PreviewWorkBudget {
                    load_timeout: Duration::from_secs(1),
                    ..PreviewWorkBudget::default()
                },
            );
            let task = session
                .start(
                    resolver(&entry_id, &format!("version-{case}")),
                    registry(vec![provider]),
                )
                .expect("bounded coordinator starts");
            wait_until(&started);

            let control_started = Instant::now();
            let expected_result = match case {
                "cancel" => {
                    assert!(session.cancel());
                    PreviewSessionState::Cancelled
                }
                "switch" => {
                    session
                        .switch_source(PreviewRequest {
                            request_id: "request-switched".to_string(),
                            source: source("entry-switched"),
                        })
                        .expect("source switch succeeds");
                    PreviewSessionState::Resolving
                }
                "dispose" => {
                    assert!(session.dispose());
                    PreviewSessionState::Disposed
                }
                _ => unreachable!(),
            };
            assert_eq!(session.state(), expected_result);
            assert!(control_started.elapsed() < Duration::from_millis(250));
            assert_eq!(cleanup_count.load(Ordering::Acquire), 0);

            release.store(true, Ordering::Release);
            let result = task.join();
            assert!(matches!(
                result,
                Err(PreviewRunError::StalePublication) | Err(PreviewRunError::Cancelled)
            ));
            assert_eq!(cleanup_count.load(Ordering::Acquire), 1);
            if case == "switch" {
                assert!(session.representation().is_none());
            }
        }
    }

    #[test]
    fn injected_provider_environment_reaches_load_without_changing_default_none() {
        let saw_content_read = Arc::new(AtomicBool::new(false));
        let provider = Arc::new(EnvironmentAwareProvider {
            descriptor: PreviewProviderDescriptor::new(
                "environment-aware",
                100,
                PreviewCapabilities::all(),
                vec![PreviewHostKind::ZenFloating],
                true,
            ),
            saw_content_read: Arc::clone(&saw_content_read),
        });
        let consumer: Arc<dyn ContentReadLeaseConsumer> = Arc::new(FakeContentRead);
        let outcome = session("entry-environment")
            .run_with_environment(
                resolver("entry-environment", "version-environment"),
                registry(vec![provider]),
                PreviewProviderEnvironmentHandle::with_content_read(consumer),
            )
            .expect("injected provider environment is accepted");

        assert_eq!(outcome.provider_id.as_deref(), Some("environment-aware"));
        assert!(saw_content_read.load(Ordering::Acquire));
    }

    #[test]
    fn higher_priority_compatible_provider_wins_over_generic_provider() {
        let specific = fake_provider(
            "specific",
            100,
            ProviderProbe::Compatible,
            None,
            Ok(text_result("specific")),
        );
        let generic = fake_provider(
            "generic",
            10,
            ProviderProbe::Compatible,
            None,
            Ok(text_result("generic")),
        );
        let session = session("entry-a");
        let outcome = session
            .run(
                resolver("entry-a", "version-a"),
                registry(vec![generic.clone(), specific.clone()]),
            )
            .expect("specific provider succeeds");

        assert_eq!(outcome.provider_id.as_deref(), Some("specific"));
        assert_eq!(outcome.attempted_provider_ids, vec!["specific"]);
        assert_eq!(specific.prepare_calls.load(Ordering::Acquire), 1);
        assert_eq!(generic.prepare_calls.load(Ordering::Acquire), 0);
    }

    #[test]
    fn macos_native_priority_wins_and_recoverable_failure_falls_back_to_builtin_pdf() {
        let native_result = Ok(PreviewProviderResult {
            representation: PreviewRepresentation::NativeOpaque {
                host: PreviewHostKind::ZenFloating,
                token: "native-pdf-token".to_string(),
            },
            completeness: PreviewCompleteness::Complete,
            warnings: Vec::new(),
        });
        let pdf_result = Ok(PreviewProviderResult {
            representation: PreviewRepresentation::Pdf {
                asset_token: "pdf-token".to_string(),
                media_type: "application/pdf".to_string(),
                length_bytes: 20 * 1024 * 1024,
            },
            completeness: PreviewCompleteness::Complete,
            warnings: Vec::new(),
        });

        let native = fake_provider(
            "native.macos.quick-look",
            400,
            ProviderProbe::Compatible,
            None,
            native_result.clone(),
        );
        let pdf = fake_provider(
            "builtin.pdf",
            310,
            ProviderProbe::Compatible,
            None,
            pdf_result.clone(),
        );
        let native_wins = session("entry-macos-pdf");
        let outcome = native_wins
            .run(
                resolver("entry-macos-pdf", "version-macos-pdf"),
                registry(vec![pdf, native]),
            )
            .expect("macOS native provider wins by priority");
        assert_eq!(
            outcome.provider_id.as_deref(),
            Some("native.macos.quick-look")
        );
        assert!(matches!(
            outcome.envelope.representation,
            PreviewRepresentation::NativeOpaque { .. }
        ));

        let native_failed = fake_provider(
            "native.macos.quick-look",
            400,
            ProviderProbe::Compatible,
            None,
            Err(PreviewProviderError::Failed),
        );
        let pdf_fallback = fake_provider(
            "builtin.pdf",
            310,
            ProviderProbe::Compatible,
            None,
            pdf_result,
        );
        let fallback = session("entry-macos-pdf-fallback");
        let outcome = fallback
            .run(
                resolver("entry-macos-pdf-fallback", "version-macos-pdf-fallback"),
                registry(vec![pdf_fallback, native_failed]),
            )
            .expect("builtin PDF fallback succeeds after native provider failure");
        assert_eq!(outcome.provider_id.as_deref(), Some("builtin.pdf"));
        assert_eq!(
            outcome.attempted_provider_ids,
            vec![
                "native.macos.quick-look".to_string(),
                "builtin.pdf".to_string()
            ]
        );
        assert!(matches!(
            outcome.envelope.representation,
            PreviewRepresentation::Pdf { length_bytes, .. } if length_bytes == 20 * 1024 * 1024
        ));
    }

    #[test]
    fn provider_local_unsupported_failure_timeout_and_corruption_fall_back() {
        let cases = [
            (
                ProviderProbe::Unsupported,
                None,
                Ok(text_result("generic")),
                PreviewProviderErrorCode::Unsupported,
            ),
            (
                ProviderProbe::Compatible,
                None,
                Err(PreviewProviderError::Failed),
                PreviewProviderErrorCode::Failed,
            ),
            (
                ProviderProbe::Compatible,
                None,
                Err(PreviewProviderError::Timeout),
                PreviewProviderErrorCode::Timeout,
            ),
            (
                ProviderProbe::Compatible,
                None,
                Err(PreviewProviderError::CorruptSource),
                PreviewProviderErrorCode::CorruptSource,
            ),
        ];
        for (index, (probe, prepare_error, load_result, expected_reason)) in
            cases.into_iter().enumerate()
        {
            let local = fake_provider("local", 100, probe, prepare_error, load_result);
            let generic = fake_provider(
                "generic",
                10,
                ProviderProbe::Compatible,
                None,
                Ok(text_result("generic")),
            );
            let session = session(&format!("entry-{index}"));
            let outcome = session
                .run(
                    resolver(&format!("entry-{index}"), "version-a"),
                    registry(vec![local.clone(), generic]),
                )
                .expect("provider-local error falls back");
            assert_eq!(outcome.provider_id.as_deref(), Some("generic"));
            assert_eq!(outcome.attempted_provider_ids, vec!["local", "generic"]);
            assert!(outcome.envelope.warnings.iter().any(|warning| matches!(
                warning,
                PreviewWarning::ProviderFallback { provider_id, reason }
                    if provider_id == "local" && *reason == expected_reason
            )));
            assert!(!outcome
                .envelope
                .warnings
                .iter()
                .any(|warning| matches!(warning, PreviewWarning::TerminalCondition { .. })));
        }
    }

    #[test]
    fn terminal_source_conditions_do_not_fall_through_to_byte_provider() {
        for (error, expected_condition) in [
            (
                PreviewProviderError::SourceUnavailable,
                PreviewTerminalCondition::SourceUnavailable,
            ),
            (
                PreviewProviderError::MaterializationRequired,
                PreviewTerminalCondition::MaterializationRequired,
            ),
            (
                PreviewProviderError::PermissionDenied,
                PreviewTerminalCondition::PermissionDenied,
            ),
            (
                PreviewProviderError::IdentityChanged,
                PreviewTerminalCondition::IdentityChanged,
            ),
        ] {
            let terminal =
                fake_provider("terminal", 100, ProviderProbe::Compatible, None, Err(error));
            let generic = fake_provider(
                "generic",
                10,
                ProviderProbe::Compatible,
                None,
                Ok(text_result("must-not-load")),
            );
            let session = session("entry-terminal");
            let result = session.run(
                resolver("entry-terminal", "version-terminal"),
                registry(vec![terminal, generic.clone()]),
            );
            assert!(matches!(
                result,
                Err(PreviewRunError::ProviderTerminal { .. })
            ));
            assert_eq!(generic.prepare_calls.load(Ordering::Acquire), 0);
            assert_eq!(session.state(), PreviewSessionState::Failed);
            assert!(matches!(
                session
                    .representation()
                    .map(|envelope| envelope.representation),
                Some(PreviewRepresentation::Metadata { .. })
            ));
            let envelope = session
                .representation()
                .expect("terminal envelope published");
            assert!(envelope.warnings.iter().any(|warning| matches!(
                warning,
                PreviewWarning::TerminalCondition { condition } if *condition == expected_condition
            )));
            assert!(envelope
                .warnings
                .iter()
                .any(|warning| matches!(warning, PreviewWarning::MetadataFallback)));
            assert!(!envelope
                .warnings
                .iter()
                .any(|warning| matches!(warning, PreviewWarning::ProviderFallback { .. })));
        }
    }

    #[test]
    fn provider_error_matrix_is_exactly_recoverable_or_terminal() {
        let recoverable = [
            PreviewProviderError::Unsupported,
            PreviewProviderError::Failed,
            PreviewProviderError::Timeout,
            PreviewProviderError::CorruptSource,
        ];
        let terminal = [
            PreviewProviderError::SourceUnavailable,
            PreviewProviderError::MaterializationRequired,
            PreviewProviderError::PermissionDenied,
            PreviewProviderError::IdentityChanged,
            PreviewProviderError::Cancelled,
        ];

        for error in recoverable {
            assert!(error.terminal_condition().is_none());
        }
        for error in terminal {
            assert!(error.terminal_condition().is_some());
        }
        assert_eq!(recoverable.len() + terminal.len(), 9);
    }

    #[test]
    fn materialization_required_source_blocks_content_provider_before_prepare() {
        let mut source_snapshot =
            snapshot(source("entry-materialization"), "version-materialization");
        source_snapshot.metadata.read_eligibility = ContentReadEligibility::MaterializationRequired;
        source_snapshot.metadata.materialization = MaterializationState::RemotePlaceholder;
        let content_provider = fake_provider(
            "content-provider",
            100,
            ProviderProbe::Compatible,
            None,
            Ok(text_result("must-not-load")),
        );
        let session = session("entry-materialization");
        let result = session.run(
            Arc::new(FakeResolver {
                snapshot: source_snapshot,
            }),
            registry(vec![content_provider.clone()]),
        );
        assert!(matches!(
            result,
            Err(PreviewRunError::ProviderTerminal {
                condition: PreviewTerminalCondition::MaterializationRequired,
                ..
            })
        ));
        assert_eq!(content_provider.prepare_calls.load(Ordering::Acquire), 0);
        assert!(matches!(
            session.representation().map(|value| value.representation),
            Some(PreviewRepresentation::Metadata { .. })
        ));
    }

    #[test]
    fn cancellation_is_terminal_and_does_not_fall_through() {
        let terminal = fake_provider(
            "cancelled",
            100,
            ProviderProbe::Compatible,
            None,
            Err(PreviewProviderError::Cancelled),
        );
        let generic = fake_provider(
            "generic",
            10,
            ProviderProbe::Compatible,
            None,
            Ok(text_result("must-not-load")),
        );
        let session = session("entry-cancelled");
        let result = session.run(
            resolver("entry-cancelled", "version-cancelled"),
            registry(vec![terminal, generic.clone()]),
        );
        assert!(matches!(result, Err(PreviewRunError::Cancelled)));
        assert_eq!(generic.prepare_calls.load(Ordering::Acquire), 0);
        assert_eq!(session.state(), PreviewSessionState::Cancelled);
    }

    #[test]
    fn stale_result_cannot_publish_after_switching_source() {
        let started = Arc::new(AtomicBool::new(false));
        let cleanup_count = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(FakeProvider {
            descriptor: PreviewProviderDescriptor::new(
                "slow",
                100,
                PreviewCapabilities::all(),
                vec![PreviewHostKind::ZenFloating],
                true,
            ),
            probe: ProviderProbe::Compatible,
            prepare_error: None,
            load_result: Ok(text_result("source-a")),
            prepare_calls: AtomicUsize::new(0),
            cleanup_count: Arc::clone(&cleanup_count),
            started: Some(Arc::clone(&started)),
            wait_for_cancel: true,
        });
        let session = session("entry-a");
        let task = session
            .start(resolver("entry-a", "version-a"), registry(vec![provider]))
            .expect("preview worker starts");
        wait_until(&started);

        session
            .switch_source(PreviewRequest {
                request_id: "request-b".to_string(),
                source: source("entry-b"),
            })
            .expect("source switch revokes old publication");
        assert_eq!(session.state(), PreviewSessionState::Resolving);
        assert!(matches!(
            task.join(),
            Err(PreviewRunError::StalePublication)
        ));
        assert!(session.representation().is_none());
        assert_eq!(cleanup_count.load(Ordering::Acquire), 1);
    }

    #[test]
    fn dispose_cleans_prepared_provider_and_revokes_publication() {
        let started = Arc::new(AtomicBool::new(false));
        let cleanup_count = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(FakeProvider {
            descriptor: PreviewProviderDescriptor::new(
                "disposable",
                100,
                PreviewCapabilities::all(),
                vec![PreviewHostKind::ZenFloating],
                true,
            ),
            probe: ProviderProbe::Compatible,
            prepare_error: None,
            load_result: Ok(text_result("never-published")),
            prepare_calls: AtomicUsize::new(0),
            cleanup_count: Arc::clone(&cleanup_count),
            started: Some(Arc::clone(&started)),
            wait_for_cancel: true,
        });
        let session = session("entry-dispose");
        let task = session
            .start(
                resolver("entry-dispose", "version-dispose"),
                registry(vec![provider]),
            )
            .expect("preview worker starts");
        wait_until(&started);
        assert!(session.dispose());
        assert_eq!(session.state(), PreviewSessionState::Disposed);
        assert!(matches!(
            task.join(),
            Err(PreviewRunError::StalePublication)
        ));
        assert_eq!(cleanup_count.load(Ordering::Acquire), 1);
    }

    #[test]
    fn metadata_fallback_survives_provider_failure() {
        let provider = fake_provider(
            "failed",
            100,
            ProviderProbe::Compatible,
            None,
            Err(PreviewProviderError::Failed),
        );
        let session = session("entry-fallback");
        let outcome = session
            .run(
                resolver("entry-fallback", "version-fallback"),
                registry(vec![provider]),
            )
            .expect("metadata fallback remains available");
        assert!(outcome.provider_id.is_none());
        assert!(matches!(
            outcome.envelope.representation,
            PreviewRepresentation::Metadata { .. }
        ));
        assert!(outcome
            .envelope
            .warnings
            .iter()
            .any(|warning| matches!(warning, PreviewWarning::ProviderFallback { .. })));
        assert!(outcome
            .envelope
            .warnings
            .iter()
            .any(|warning| matches!(warning, PreviewWarning::MetadataFallback)));
        assert_eq!(session.state(), PreviewSessionState::Ready);
    }

    #[test]
    fn current_a_failure_falls_back_a() {
        let provider = fake_provider(
            "native.macos.quick-look",
            100,
            ProviderProbe::Compatible,
            None,
            Ok(PreviewProviderResult {
                representation: PreviewRepresentation::NativeOpaque {
                    host: PreviewHostKind::ZenFloating,
                    token: "opaque-native-token".to_string(),
                },
                completeness: PreviewCompleteness::Complete,
                warnings: Vec::new(),
            }),
        );
        let session = session("entry-native-failure");
        let task = session
            .start(
                resolver("entry-native-failure", "version-native-failure"),
                registry(vec![provider]),
            )
            .expect("native provider starts");
        task.join().expect("native provider publishes");
        assert!(matches!(
            session
                .representation()
                .map(|envelope| envelope.representation),
            Some(PreviewRepresentation::NativeOpaque { .. })
        ));

        let expected = NativePreviewFailureIdentity {
            request_id: "request-1".to_string(),
            source_version: "version-native-failure".to_string(),
            host: PreviewHostKind::ZenFloating,
            token: "opaque-native-token".to_string(),
        };
        assert!(session.fallback_from_native_failure("native.macos.quick-look", &expected,));
        let envelope = session
            .representation()
            .expect("native failure publishes fallback");
        assert!(matches!(
            envelope.representation,
            PreviewRepresentation::Metadata { .. }
        ));
        assert_eq!(session.state(), PreviewSessionState::Ready);
        assert!(envelope.warnings.iter().any(|warning| matches!(
            warning,
            PreviewWarning::ProviderFallback {
                provider_id,
                reason: PreviewProviderErrorCode::Failed,
            } if provider_id == "native.macos.quick-look"
        )));
        assert!(!session.fallback_from_native_failure("native.macos.quick-look", &expected,));
    }

    #[test]
    fn stale_a_failure_after_b_publish_does_not_fallback_b() {
        let provider_a = fake_provider(
            "native.macos.quick-look",
            100,
            ProviderProbe::Compatible,
            None,
            Ok(PreviewProviderResult {
                representation: PreviewRepresentation::NativeOpaque {
                    host: PreviewHostKind::ZenFloating,
                    token: "native-token-a".to_string(),
                },
                completeness: PreviewCompleteness::Complete,
                warnings: Vec::new(),
            }),
        );
        let provider_b = fake_provider(
            "native.macos.quick-look",
            100,
            ProviderProbe::Compatible,
            None,
            Ok(PreviewProviderResult {
                representation: PreviewRepresentation::NativeOpaque {
                    host: PreviewHostKind::ZenFloating,
                    token: "native-token-b".to_string(),
                },
                completeness: PreviewCompleteness::Complete,
                warnings: Vec::new(),
            }),
        );
        let session = session("entry-native-stale");
        session
            .run(
                resolver("entry-native-stale", "version-native-a"),
                registry(vec![provider_a]),
            )
            .expect("native A publishes");
        let expected_a = NativePreviewFailureIdentity {
            request_id: "request-1".to_string(),
            source_version: "version-native-a".to_string(),
            host: PreviewHostKind::ZenFloating,
            token: "native-token-a".to_string(),
        };

        session
            .switch_source(PreviewRequest {
                request_id: "request-b".to_string(),
                source: source("entry-native-b"),
            })
            .expect("source B switches");
        session
            .run(
                resolver("entry-native-b", "version-native-b"),
                registry(vec![provider_b]),
            )
            .expect("native B publishes");

        assert!(!session.fallback_from_native_failure("native.macos.quick-look", &expected_a,));
        assert!(matches!(
            session
                .representation()
                .map(|envelope| envelope.representation),
            Some(PreviewRepresentation::NativeOpaque { token, .. }) if token == "native-token-b"
        ));
    }

    #[test]
    fn failure_after_cancel_does_not_publish_fallback() {
        let provider = || {
            fake_provider(
                "native.macos.quick-look",
                100,
                ProviderProbe::Compatible,
                None,
                Ok(PreviewProviderResult {
                    representation: PreviewRepresentation::NativeOpaque {
                        host: PreviewHostKind::ZenFloating,
                        token: "terminal-native-token".to_string(),
                    },
                    completeness: PreviewCompleteness::Complete,
                    warnings: Vec::new(),
                }),
            )
        };
        let expected = NativePreviewFailureIdentity {
            request_id: "request-1".to_string(),
            source_version: "version-terminal-native".to_string(),
            host: PreviewHostKind::ZenFloating,
            token: "terminal-native-token".to_string(),
        };

        let cancelled = session("entry-native-cancelled");
        cancelled
            .run(
                resolver("entry-native-cancelled", "version-terminal-native"),
                registry(vec![provider()]),
            )
            .expect("native cancelled fixture publishes");
        assert!(cancelled.cancel());
        assert!(!cancelled.fallback_from_native_failure("native.macos.quick-look", &expected,));
    }

    #[test]
    fn failure_after_dispose_does_not_publish_fallback() {
        let provider = fake_provider(
            "native.macos.quick-look",
            100,
            ProviderProbe::Compatible,
            None,
            Ok(PreviewProviderResult {
                representation: PreviewRepresentation::NativeOpaque {
                    host: PreviewHostKind::ZenFloating,
                    token: "terminal-native-token".to_string(),
                },
                completeness: PreviewCompleteness::Complete,
                warnings: Vec::new(),
            }),
        );
        let session = session("entry-native-disposed");
        session
            .run(
                resolver("entry-native-disposed", "version-terminal-native"),
                registry(vec![provider]),
            )
            .expect("native disposed fixture publishes");
        assert!(session.dispose());
        let expected = NativePreviewFailureIdentity {
            request_id: "request-1".to_string(),
            source_version: "version-terminal-native".to_string(),
            host: PreviewHostKind::ZenFloating,
            token: "terminal-native-token".to_string(),
        };
        assert!(!session.fallback_from_native_failure("native.macos.quick-look", &expected,));
    }

    #[test]
    fn effective_capabilities_are_host_provider_source_intersection() {
        let mut host_capabilities = PreviewCapabilities::all();
        host_capabilities.can_zoom = false;
        let mut source_capabilities = PreviewCapabilities::all();
        source_capabilities.can_playback = false;
        let mut provider_capabilities = PreviewCapabilities::all();
        provider_capabilities.can_search = false;
        let provider = Arc::new(FakeProvider {
            descriptor: PreviewProviderDescriptor::new(
                "capability-provider",
                100,
                provider_capabilities,
                vec![PreviewHostKind::ZenFloating],
                true,
            ),
            probe: ProviderProbe::Compatible,
            prepare_error: None,
            load_result: Ok(text_result("capabilities")),
            prepare_calls: AtomicUsize::new(0),
            cleanup_count: Arc::new(AtomicUsize::new(0)),
            started: None,
            wait_for_cancel: false,
        });
        let session = PreviewSession::new(PreviewSessionConfig {
            session_id: "session-capabilities".to_string(),
            request: PreviewRequest {
                request_id: "request-capabilities".to_string(),
                source: source("entry-capabilities"),
            },
            host: PreviewHost::new(PreviewHostKind::ZenFloating, host_capabilities),
            budget: PreviewWorkBudget::default(),
        });
        let source_resolver = Arc::new(FakeResolver {
            snapshot: PreviewSourceSnapshot::new(
                source("entry-capabilities"),
                "version-capabilities",
                metadata("capabilities.txt"),
                source_capabilities,
            ),
        });
        let outcome = session
            .run(source_resolver, registry(vec![provider]))
            .expect("capability provider succeeds");
        assert!(!outcome.envelope.capabilities.can_search);
        assert!(!outcome.envelope.capabilities.can_zoom);
        assert!(!outcome.envelope.capabilities.can_playback);
        assert!(outcome.envelope.capabilities.can_select_text);
    }

    #[test]
    fn publication_token_is_revoked_by_source_switch() {
        let session = session("entry-token-a");
        let token = session.current_publication().expect("idle token exists");
        assert!(session.can_publish(&token));
        session
            .switch_source(PreviewRequest {
                request_id: "request-token-b".to_string(),
                source: source("entry-token-b"),
            })
            .expect("switch succeeds");
        assert!(!token.is_current());
        assert!(!session.can_publish(&token));
    }

    #[test]
    fn content_read_lease_consumer_is_bounded_and_path_free() {
        let session = session("entry-lease");
        let token = session
            .current_publication()
            .expect("idle publication token");
        let context = operation_context(
            &token,
            PreviewHostKind::ZenFloating,
            None,
            Duration::from_secs(1),
        );
        let lease = ContentReadLeaseRef {
            lease_id: "lease-1".to_string(),
            request_id: "request-1".to_string(),
            source_version: "version-lease".to_string(),
        };
        let consumer = FakeContentRead;
        let environment = PreviewProviderEnvironment {
            content_read: Some(&consumer),
            preview_read: None,
            folder_enumeration: None,
            publication: None,
            asset_publisher: None,
            decoder_admission: None,
            archive_admission: None,
        };
        assert!(environment.content_read.is_some());
        let read = consumer
            .read_bounded(
                &lease,
                BoundedContentReadRequest {
                    offset_bytes: 0,
                    max_bytes: 4,
                },
                &context,
            )
            .expect("fake bounded lease read");
        assert_eq!(read.bytes, b"data");
        assert!(read.complete);
        let wire = serde_json::to_value(lease).expect("opaque lease serializes");
        assert!(wire.get("path").is_none());
        assert!(wire.get("filePath").is_none());
    }

    #[test]
    fn representation_and_warning_wire_is_exhaustive_and_strict() {
        let representations = vec![
            PreviewRepresentation::Metadata {
                metadata: metadata("metadata"),
            },
            PreviewRepresentation::Text {
                text: "text".to_string(),
                language: Some("text".to_string()),
            },
            PreviewRepresentation::SafeHtml {
                html: "<p>safe</p>".to_string(),
            },
            PreviewRepresentation::StructuredTree {
                encoded_tree: "{}".to_string(),
            },
            PreviewRepresentation::Table {
                encoded_table: "[]".to_string(),
            },
            PreviewRepresentation::Image {
                asset_token: "preview-asset-image".to_string(),
                media_type: "image/png".to_string(),
            },
            PreviewRepresentation::Pdf {
                asset_token: "preview-asset-pdf".to_string(),
                media_type: "application/pdf".to_string(),
                length_bytes: 20 * 1024 * 1024,
            },
            PreviewRepresentation::Media {
                asset_token: "preview-asset-media".to_string(),
                media_type: "audio/mpeg".to_string(),
            },
            PreviewRepresentation::FolderSummary {
                encoded_summary: "{}".to_string(),
            },
            PreviewRepresentation::ArchiveTree {
                encoded_tree: "{}".to_string(),
            },
            PreviewRepresentation::NativeOpaque {
                host: PreviewHostKind::ZenFloating,
                token: "native-token".to_string(),
            },
        ];
        for representation in representations {
            let value = serde_json::to_value(&representation).expect("representation wire");
            assert!(value.get("path").is_none());
            assert!(value.get("filePath").is_none());
            assert_eq!(
                serde_json::from_value::<PreviewRepresentation>(value).expect("strict round trip"),
                representation
            );
        }

        assert_eq!(
            serde_json::to_value(PreviewRepresentation::Pdf {
                asset_token: "preview-asset-pdf".to_string(),
                media_type: "application/pdf".to_string(),
                length_bytes: 20 * 1024 * 1024,
            })
            .expect("PDF representation wire"),
            serde_json::json!({
                "family": "pdf",
                "assetToken": "preview-asset-pdf",
                "mediaType": "application/pdf",
                "lengthBytes": 20 * 1024 * 1024,
            })
        );

        let warning = PreviewWarning::ProviderFallback {
            provider_id: "provider-1".to_string(),
            reason: PreviewProviderErrorCode::Timeout,
        };
        assert_eq!(
            serde_json::to_value(warning).expect("warning wire"),
            serde_json::json!({
                "kind": "provider_fallback",
                "providerId": "provider-1",
                "reason": "timeout"
            })
        );
        assert!(
            serde_json::from_value::<PreviewRepresentation>(serde_json::json!({
                "family": "text",
                "text": "x",
                "language": null,
                "path": "C:\\secret"
            }))
            .is_err()
        );
        assert!(serde_json::from_value::<PreviewWarning>(serde_json::json!({
            "kind": "future_warning"
        }))
        .is_err());
    }
}
