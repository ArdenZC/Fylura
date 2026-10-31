//! Bounded W3-04 Text, source-code and Markdown Preview providers.
//!
//! These providers are deliberately read-only presentation adapters. They
//! receive only an opaque source reference, source version, Preview operation
//! context and the narrow Preview read adapter. The existing read gate remains
//! the only authority that resolves and opens bytes.

use super::preview_asset::MAX_PREVIEW_RANGE_SOURCE_BYTES;
use super::{
    contracts::{ContentReadEligibility, PreviewHostKind, PreviewSourceRef},
    preview::{
        BoundedContentRead, BoundedContentReadRequest, PreparedPreview, PreviewCapabilities,
        PreviewCompleteness, PreviewContentReadAccess, PreviewEntryKind, PreviewMetadata,
        PreviewOperationContext, PreviewProvider, PreviewProviderDescriptor,
        PreviewProviderEnvironment, PreviewProviderError, PreviewProviderResult,
        PreviewReadAccessError, PreviewRepresentation, PreviewSourceSnapshot, ProviderProbe,
    },
};
use fylura_preview_representation::{
    RepresentationCompleteness, RepresentationError, RepresentationHint,
};

/// Shared W3-04 source prefix. It remains below the existing one-megabyte
/// read-gate ceiling and is used by Text, Code and Markdown alike.
pub(crate) const PREVIEW_TEXT_READ_BYTES: u32 = 512 * 1024;
pub(crate) const PDF_PREVIEW_PROVIDER_PRIORITY: i32 = 310;
const ZEN_HOSTS: &[PreviewHostKind] = &[PreviewHostKind::ZenFloating, PreviewHostKind::ZenPinned];

fn text_capabilities() -> PreviewCapabilities {
    PreviewCapabilities {
        can_select_text: true,
        ..PreviewCapabilities::default()
    }
}

pub(crate) fn production_preview_providers() -> Vec<std::sync::Arc<dyn PreviewProvider>> {
    let mut providers: Vec<std::sync::Arc<dyn PreviewProvider>> = vec![
        std::sync::Arc::new(PdfPreviewProvider::new()),
        std::sync::Arc::new(MarkdownPreviewProvider::new()),
        std::sync::Arc::new(crate::file_workspace::preview_folder::FolderPreviewProvider::new()),
        std::sync::Arc::new(crate::file_workspace::preview_image::ImagePreviewProvider::new()),
        std::sync::Arc::new(
            crate::file_workspace::preview_archive::ArchiveZipPreviewProvider::new(),
        ),
        std::sync::Arc::new(SourceCodePreviewProvider::new()),
        std::sync::Arc::new(PlainTextPreviewProvider::new()),
    ];
    providers.extend(crate::file_workspace::preview_structured::production_preview_providers());
    providers
}

/// Local Zen renderer input for PDFs. The provider reuses the existing
/// PreviewReadAccess and ephemeral PreviewAssetRegistry seams; it never
/// exposes a path or delegates rendering to Explorer/another host.
pub(crate) struct PdfPreviewProvider {
    descriptor: PreviewProviderDescriptor,
}

impl PdfPreviewProvider {
    pub(crate) fn new() -> Self {
        Self {
            descriptor: PreviewProviderDescriptor::new(
                "builtin.pdf",
                PDF_PREVIEW_PROVIDER_PRIORITY,
                PreviewCapabilities {
                    can_zoom: true,
                    ..PreviewCapabilities::default()
                },
                ZEN_HOSTS.to_vec(),
                true,
            ),
        }
    }
}

impl PreviewProvider for PdfPreviewProvider {
    fn descriptor(&self) -> &PreviewProviderDescriptor {
        &self.descriptor
    }

    fn probe(
        &self,
        snapshot: &PreviewSourceSnapshot,
        _context: &PreviewOperationContext,
    ) -> ProviderProbe {
        if source_can_render_pdf(snapshot) {
            ProviderProbe::Compatible
        } else {
            ProviderProbe::Unsupported
        }
    }

    fn prepare(
        &self,
        snapshot: &PreviewSourceSnapshot,
        _context: &PreviewOperationContext,
    ) -> Result<Box<dyn PreparedPreview>, PreviewProviderError> {
        if !source_can_render_pdf(snapshot) {
            return Err(PreviewProviderError::Unsupported);
        }
        let Some(length_bytes) = snapshot.metadata.size_bytes else {
            return Err(PreviewProviderError::Unsupported);
        };
        if length_bytes == 0 || length_bytes > MAX_PREVIEW_RANGE_SOURCE_BYTES {
            return Err(PreviewProviderError::Unsupported);
        }
        Ok(Box::new(PreparedPdfPreview {
            source: snapshot.source.clone(),
            source_version: snapshot.source_version.clone(),
            length_bytes,
        }))
    }
}

struct PreparedPdfPreview {
    source: PreviewSourceRef,
    source_version: String,
    length_bytes: u64,
}

impl PreparedPreview for PreparedPdfPreview {
    fn load(
        &mut self,
        context: &PreviewOperationContext,
        environment: PreviewProviderEnvironment<'_>,
    ) -> Result<PreviewProviderResult, PreviewProviderError> {
        let publisher = environment
            .asset_publisher
            .ok_or(PreviewProviderError::Failed)?;
        context.ensure_active().map_err(map_context_error)?;
        let asset_token = publisher
            .publish_range_asset(
                context,
                "application/pdf",
                &self.source,
                &self.source_version,
                self.length_bytes,
            )
            .map_err(map_asset_error)?;
        Ok(PreviewProviderResult {
            representation: PreviewRepresentation::Pdf {
                asset_token,
                media_type: "application/pdf".to_string(),
                length_bytes: self.length_bytes,
            },
            completeness: PreviewCompleteness::Complete,
            warnings: Vec::new(),
        })
    }

    fn cleanup(&mut self) {}
}

/// Compose the W3 registry plus the optional W4 native host adapter. The
/// Zen-owned PDF provider is part of the ordinary provider registry, while
/// the native adapter is only composed when the platform bridge and its
/// request-scoped access registry are both present.
pub(crate) fn production_preview_providers_with_native_access(
    native_preview_access: Option<
        std::sync::Arc<crate::file_workspace::native_preview::access::NativePreviewAccessRegistry>,
    >,
) -> Vec<std::sync::Arc<dyn PreviewProvider>> {
    let mut providers = production_preview_providers();
    if crate::platform::macos::native_preview::available() {
        if let Some(access) = native_preview_access {
            providers.push(std::sync::Arc::new(
                crate::file_workspace::native_preview::provider::MacNativePreviewProvider::new(
                    access,
                ),
            ));
        }
    }
    providers
}

pub(crate) struct MarkdownPreviewProvider {
    descriptor: PreviewProviderDescriptor,
}

impl MarkdownPreviewProvider {
    pub(crate) fn new() -> Self {
        Self {
            descriptor: PreviewProviderDescriptor::new(
                "builtin.markdown",
                300,
                text_capabilities(),
                ZEN_HOSTS.to_vec(),
                true,
            ),
        }
    }
}

impl PreviewProvider for MarkdownPreviewProvider {
    fn descriptor(&self) -> &PreviewProviderDescriptor {
        &self.descriptor
    }

    fn probe(
        &self,
        snapshot: &PreviewSourceSnapshot,
        _context: &PreviewOperationContext,
    ) -> ProviderProbe {
        if source_can_render_text(snapshot) && is_markdown_hint(&snapshot.metadata) {
            ProviderProbe::Compatible
        } else {
            ProviderProbe::Unsupported
        }
    }

    fn prepare(
        &self,
        snapshot: &PreviewSourceSnapshot,
        _context: &PreviewOperationContext,
    ) -> Result<Box<dyn PreparedPreview>, PreviewProviderError> {
        if !source_can_render_text(snapshot) || !is_markdown_hint(&snapshot.metadata) {
            return Err(PreviewProviderError::Unsupported);
        }
        Ok(Box::new(PreparedMarkdownPreview {
            source: snapshot.source.clone(),
            source_version: snapshot.source_version.clone(),
        }))
    }
}

struct PreparedMarkdownPreview {
    source: PreviewSourceRef,
    source_version: String,
}

impl PreparedPreview for PreparedMarkdownPreview {
    fn load(
        &mut self,
        context: &PreviewOperationContext,
        environment: PreviewProviderEnvironment<'_>,
    ) -> Result<PreviewProviderResult, PreviewProviderError> {
        let read = read_source_prefix(
            &self.source,
            &self.source_version,
            context,
            environment.preview_read,
        )?;
        let (text, completeness) = decode_text(read)?;
        let html = render_safe_markdown(&text)?;
        Ok(PreviewProviderResult {
            representation: PreviewRepresentation::SafeHtml { html },
            completeness,
            warnings: Vec::new(),
        })
    }

    fn cleanup(&mut self) {}
}

pub(crate) struct SourceCodePreviewProvider {
    descriptor: PreviewProviderDescriptor,
}

impl SourceCodePreviewProvider {
    pub(crate) fn new() -> Self {
        Self {
            descriptor: PreviewProviderDescriptor::new(
                "builtin.source-code",
                200,
                text_capabilities(),
                ZEN_HOSTS.to_vec(),
                true,
            ),
        }
    }
}

impl PreviewProvider for SourceCodePreviewProvider {
    fn descriptor(&self) -> &PreviewProviderDescriptor {
        &self.descriptor
    }

    fn probe(
        &self,
        snapshot: &PreviewSourceSnapshot,
        _context: &PreviewOperationContext,
    ) -> ProviderProbe {
        if source_can_render_text(snapshot)
            && !crate::file_workspace::preview_structured::is_structured_or_table_hint(
                &snapshot.metadata,
            )
            && code_language(&snapshot.metadata).is_some()
        {
            ProviderProbe::Compatible
        } else {
            ProviderProbe::Unsupported
        }
    }

    fn prepare(
        &self,
        snapshot: &PreviewSourceSnapshot,
        _context: &PreviewOperationContext,
    ) -> Result<Box<dyn PreparedPreview>, PreviewProviderError> {
        let Some(language) = code_language(&snapshot.metadata) else {
            return Err(PreviewProviderError::Unsupported);
        };
        if !source_can_render_text(snapshot)
            || crate::file_workspace::preview_structured::is_structured_or_table_hint(
                &snapshot.metadata,
            )
        {
            return Err(PreviewProviderError::Unsupported);
        }
        Ok(Box::new(PreparedTextPreview {
            source: snapshot.source.clone(),
            source_version: snapshot.source_version.clone(),
            language: Some(language),
        }))
    }
}

pub(crate) struct PlainTextPreviewProvider {
    descriptor: PreviewProviderDescriptor,
}

impl PlainTextPreviewProvider {
    pub(crate) fn new() -> Self {
        Self {
            descriptor: PreviewProviderDescriptor::new(
                "builtin.text",
                100,
                text_capabilities(),
                ZEN_HOSTS.to_vec(),
                true,
            ),
        }
    }
}

impl PreviewProvider for PlainTextPreviewProvider {
    fn descriptor(&self) -> &PreviewProviderDescriptor {
        &self.descriptor
    }

    fn probe(
        &self,
        snapshot: &PreviewSourceSnapshot,
        _context: &PreviewOperationContext,
    ) -> ProviderProbe {
        if source_can_render_text(snapshot)
            && !crate::file_workspace::preview_structured::is_structured_or_table_hint(
                &snapshot.metadata,
            )
            && is_plain_text_hint(&snapshot.metadata)
        {
            ProviderProbe::Compatible
        } else {
            ProviderProbe::Unsupported
        }
    }

    fn prepare(
        &self,
        snapshot: &PreviewSourceSnapshot,
        _context: &PreviewOperationContext,
    ) -> Result<Box<dyn PreparedPreview>, PreviewProviderError> {
        if !source_can_render_text(snapshot)
            || crate::file_workspace::preview_structured::is_structured_or_table_hint(
                &snapshot.metadata,
            )
            || !is_plain_text_hint(&snapshot.metadata)
        {
            return Err(PreviewProviderError::Unsupported);
        }
        Ok(Box::new(PreparedTextPreview {
            source: snapshot.source.clone(),
            source_version: snapshot.source_version.clone(),
            language: None,
        }))
    }
}

struct PreparedTextPreview {
    source: PreviewSourceRef,
    source_version: String,
    language: Option<&'static str>,
}

impl PreparedPreview for PreparedTextPreview {
    fn load(
        &mut self,
        context: &PreviewOperationContext,
        environment: PreviewProviderEnvironment<'_>,
    ) -> Result<PreviewProviderResult, PreviewProviderError> {
        let read = read_source_prefix(
            &self.source,
            &self.source_version,
            context,
            environment.preview_read,
        )?;
        let (text, completeness) = decode_text(read)?;
        Ok(PreviewProviderResult {
            representation: PreviewRepresentation::Text {
                text,
                language: self.language.map(str::to_owned),
            },
            completeness,
            warnings: Vec::new(),
        })
    }

    fn cleanup(&mut self) {}
}

fn source_can_render_text(snapshot: &PreviewSourceSnapshot) -> bool {
    snapshot.metadata.read_eligibility == ContentReadEligibility::Eligible
        && snapshot.capabilities.can_select_text
}

pub(crate) fn read_source_prefix(
    source: &PreviewSourceRef,
    source_version: &str,
    context: &PreviewOperationContext,
    reader: Option<&dyn PreviewContentReadAccess>,
) -> Result<BoundedContentRead, PreviewProviderError> {
    read_source_prefix_with_limit(
        source,
        source_version,
        context,
        reader,
        PREVIEW_TEXT_READ_BYTES,
    )
}

pub(crate) fn read_source_prefix_with_limit(
    source: &PreviewSourceRef,
    source_version: &str,
    context: &PreviewOperationContext,
    reader: Option<&dyn PreviewContentReadAccess>,
    max_bytes: u32,
) -> Result<BoundedContentRead, PreviewProviderError> {
    let reader = reader.ok_or(PreviewProviderError::Failed)?;
    reader
        .read_source_bounded(
            source,
            source_version,
            BoundedContentReadRequest {
                offset_bytes: 0,
                max_bytes,
            },
            context,
        )
        .map_err(map_content_read_error)
}

fn map_content_read_error(error: PreviewReadAccessError) -> PreviewProviderError {
    match error {
        PreviewReadAccessError::LeaseInvalid | PreviewReadAccessError::Failed => {
            PreviewProviderError::Failed
        }
        PreviewReadAccessError::SourceVersionMismatch => PreviewProviderError::IdentityChanged,
        PreviewReadAccessError::PermissionDenied => PreviewProviderError::PermissionDenied,
        PreviewReadAccessError::SourceUnavailable => PreviewProviderError::SourceUnavailable,
        PreviewReadAccessError::MaterializationRequired => {
            PreviewProviderError::MaterializationRequired
        }
        PreviewReadAccessError::MetadataOnly => PreviewProviderError::Unsupported,
        PreviewReadAccessError::Cancelled => PreviewProviderError::Cancelled,
        PreviewReadAccessError::TimedOut => PreviewProviderError::Timeout,
    }
}

fn map_asset_error(error: super::preview::PreviewAssetError) -> PreviewProviderError {
    match error {
        super::preview::PreviewAssetError::Cancelled
        | super::preview::PreviewAssetError::StalePublication => PreviewProviderError::Cancelled,
        super::preview::PreviewAssetError::InvalidMediaType
        | super::preview::PreviewAssetError::OutputTooLarge
        | super::preview::PreviewAssetError::CapacityExceeded
        | super::preview::PreviewAssetError::Disposed
        | super::preview::PreviewAssetError::RangeUnsupported => PreviewProviderError::Failed,
    }
}

fn map_context_error(error: super::preview::PreviewContextError) -> PreviewProviderError {
    match error {
        super::preview::PreviewContextError::Cancelled
        | super::preview::PreviewContextError::StalePublication => PreviewProviderError::Cancelled,
        super::preview::PreviewContextError::TimedOut => PreviewProviderError::Timeout,
    }
}

fn decode_text(
    read: BoundedContentRead,
) -> Result<(String, PreviewCompleteness), PreviewProviderError> {
    let decoded = fylura_preview_representation::decode_text(&read.bytes, read.complete)
        .map_err(map_representation_error)?;
    let completeness = match decoded.completeness {
        RepresentationCompleteness::Complete => PreviewCompleteness::Complete,
        RepresentationCompleteness::Partial => PreviewCompleteness::Partial,
    };
    Ok((decoded.text, completeness))
}

fn render_safe_markdown(text: &str) -> Result<String, PreviewProviderError> {
    let (representation, _) = fylura_preview_representation::render_markdown(text.as_bytes(), true)
        .map_err(map_representation_error)?;
    match representation {
        fylura_preview_representation::SafeRepresentation::SafeHtml { html } => Ok(html),
        fylura_preview_representation::SafeRepresentation::Text { .. } => {
            Err(PreviewProviderError::Failed)
        }
    }
}

fn representation_hint(metadata: &PreviewMetadata) -> RepresentationHint {
    RepresentationHint {
        extension: metadata.extension.clone(),
        media_type: metadata.media_type.clone(),
    }
}

fn is_markdown_hint(metadata: &PreviewMetadata) -> bool {
    fylura_preview_representation::is_markdown_hint(&representation_hint(metadata))
}

fn source_can_render_pdf(snapshot: &PreviewSourceSnapshot) -> bool {
    snapshot.entry_kind == PreviewEntryKind::File
        && snapshot.metadata.read_eligibility == ContentReadEligibility::Eligible
        && is_pdf_hint(&snapshot.metadata)
}

fn is_pdf_hint(metadata: &PreviewMetadata) -> bool {
    metadata.extension.as_deref().is_some_and(|extension| {
        extension
            .trim_start_matches('.')
            .eq_ignore_ascii_case("pdf")
    }) || metadata.media_type.as_deref().is_some_and(|media_type| {
        media_type
            .split(';')
            .next()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/pdf"))
    })
}

fn is_plain_text_hint(metadata: &PreviewMetadata) -> bool {
    fylura_preview_representation::is_plain_text_hint(&representation_hint(metadata))
}

fn code_language(metadata: &PreviewMetadata) -> Option<&'static str> {
    fylura_preview_representation::source_code_language(&representation_hint(metadata))
}

fn map_representation_error(error: RepresentationError) -> PreviewProviderError {
    match error {
        RepresentationError::CorruptSource => PreviewProviderError::CorruptSource,
        RepresentationError::OutputTooLarge => PreviewProviderError::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_workspace::preview::PreviewCancellation;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    struct FakeReader {
        bytes: Mutex<Option<BoundedContentRead>>,
        max_bytes: Mutex<Option<u32>>,
    }

    struct FakePublisher;

    impl crate::file_workspace::preview::PreviewAssetPublisher for FakePublisher {
        fn publish_asset(
            &self,
            _context: &PreviewOperationContext,
            media_type: &str,
            _bytes: Vec<u8>,
        ) -> Result<String, crate::file_workspace::preview::PreviewAssetError> {
            assert_eq!(media_type, "application/pdf");
            Ok("pdf-token".to_string())
        }

        fn publish_range_asset(
            &self,
            _context: &PreviewOperationContext,
            media_type: &str,
            _source: &PreviewSourceRef,
            source_version: &str,
            length_bytes: u64,
        ) -> Result<String, crate::file_workspace::preview::PreviewAssetError> {
            assert_eq!(media_type, "application/pdf");
            assert_eq!(source_version, "version-1");
            assert!(length_bytes > 0);
            Ok("pdf-token".to_string())
        }
    }

    impl PreviewContentReadAccess for FakeReader {
        fn read_source_bounded(
            &self,
            _source: &PreviewSourceRef,
            _source_version: &str,
            request: BoundedContentReadRequest,
            _context: &PreviewOperationContext,
        ) -> Result<BoundedContentRead, PreviewReadAccessError> {
            *self.max_bytes.lock().expect("fake reader request lock") = Some(request.max_bytes);
            self.bytes
                .lock()
                .expect("fake reader lock")
                .take()
                .ok_or(PreviewReadAccessError::Failed)
        }
    }

    fn source() -> PreviewSourceRef {
        PreviewSourceRef::Managed {
            file_id: "file-1".to_string(),
        }
    }

    fn snapshot(extension: Option<&str>, media_type: Option<&str>) -> PreviewSourceSnapshot {
        PreviewSourceSnapshot::new(
            source(),
            "version-1",
            PreviewMetadata {
                display_name: "fixture.txt".to_string(),
                media_type: media_type.map(str::to_owned),
                extension: extension.map(str::to_owned),
                size_bytes: None,
                modified_at_epoch_ms: None,
                materialization: crate::file_workspace::MaterializationState::BoundaryReadable,
                read_eligibility: ContentReadEligibility::Eligible,
            },
            PreviewCapabilities {
                can_select_text: true,
                can_search: true,
                ..PreviewCapabilities::default()
            },
        )
    }

    fn context() -> PreviewOperationContext {
        PreviewOperationContext::for_backend_content_read(
            "session-1",
            "request-1",
            "version-1",
            PreviewCancellation::default(),
            Instant::now() + Duration::from_secs(1),
        )
    }

    fn load(
        provider: &dyn PreviewProvider,
        snapshot: &PreviewSourceSnapshot,
        bytes: &[u8],
        complete: bool,
    ) -> Result<PreviewProviderResult, PreviewProviderError> {
        let prepared = provider.prepare(snapshot, &context())?;
        let reader = Arc::new(FakeReader {
            bytes: Mutex::new(Some(BoundedContentRead {
                bytes: bytes.to_vec(),
                complete,
            })),
            max_bytes: Mutex::new(None),
        });
        let mut prepared = prepared;
        prepared.load(
            &context(),
            PreviewProviderEnvironment {
                content_read: None,
                preview_read: Some(reader.as_ref()),
                folder_enumeration: None,
                publication: None,
                asset_publisher: None,
                decoder_admission: None,
                archive_admission: None,
            },
        )
    }

    #[test]
    fn production_providers_are_static_and_markdown_precedes_text() {
        let providers = production_preview_providers();
        let registry = crate::file_workspace::PreviewProviderRegistry::new(providers)
            .expect("provider registry");
        assert_eq!(
            registry.provider_ids(),
            vec![
                "builtin.pdf".to_string(),
                "builtin.markdown".to_string(),
                "builtin.folder".to_string(),
                "builtin.image".to_string(),
                "builtin.archive-zip".to_string(),
                "builtin.structured-json".to_string(),
                "builtin.structured-yaml".to_string(),
                "builtin.structured-xml".to_string(),
                "builtin.table-csv".to_string(),
                "builtin.table-tsv".to_string(),
                "builtin.source-code".to_string(),
                "builtin.text".to_string()
            ]
        );
    }

    #[test]
    fn pdf_publishes_a_range_backed_opaque_asset_for_a_valid_source() {
        let provider = PdfPreviewProvider::new();
        let mut fixture = snapshot(Some("pdf"), Some("application/pdf"));
        fixture.metadata.size_bytes = Some(10 * 1024 * 1024);
        let reader = Arc::new(FakeReader {
            bytes: Mutex::new(Some(BoundedContentRead {
                bytes: b"%PDF-1.7\n1 0 obj\nendobj\n".to_vec(),
                complete: true,
            })),
            max_bytes: Mutex::new(None),
        });
        let publisher = FakePublisher;
        let mut prepared = provider
            .prepare(&fixture, &context())
            .expect("pdf provider");
        let result = prepared
            .load(
                &context(),
                PreviewProviderEnvironment {
                    content_read: None,
                    preview_read: Some(reader.as_ref()),
                    folder_enumeration: None,
                    publication: None,
                    asset_publisher: Some(&publisher),
                    decoder_admission: None,
                    archive_admission: None,
                },
            )
            .expect("valid PDF");
        assert_eq!(*reader.max_bytes.lock().expect("pdf request lock"), None);
        assert_eq!(
            result.representation,
            PreviewRepresentation::Pdf {
                asset_token: "pdf-token".to_string(),
                media_type: "application/pdf".to_string(),
                length_bytes: 10 * 1024 * 1024
            }
        );
        assert_eq!(result.completeness, PreviewCompleteness::Complete);

        let mut missing_size = snapshot(Some("pdf"), Some("application/pdf"));
        missing_size.metadata.size_bytes = None;
        assert!(matches!(
            provider.prepare(&missing_size, &context()),
            Err(PreviewProviderError::Unsupported)
        ));
    }

    #[test]
    fn text_preserves_bom_unicode_empty_crlf_and_partial_utf8() {
        let provider = PlainTextPreviewProvider::new();
        let fixture = snapshot(Some("txt"), Some("text/plain"));
        let result = load(
            &provider,
            &fixture,
            b"\xef\xbb\xbfhello\r\n\xe4\xb8\x96\xe7\x95\x8c",
            true,
        )
        .expect("valid text");
        assert_eq!(
            result.representation,
            PreviewRepresentation::Text {
                text: "hello\r\n世界".to_string(),
                language: None
            }
        );
        assert_eq!(result.completeness, PreviewCompleteness::Complete);

        let empty = load(&provider, &fixture, b"", true).expect("empty text");
        assert_eq!(empty.completeness, PreviewCompleteness::Complete);
        assert!(matches!(
            empty.representation,
            PreviewRepresentation::Text { ref text, .. } if text.is_empty()
        ));

        let partial = load(&provider, &fixture, &[0xe4, 0xb8], false).expect("partial utf8");
        assert_eq!(partial.completeness, PreviewCompleteness::Partial);
        assert!(matches!(
            partial.representation,
            PreviewRepresentation::Text { ref text, .. } if text.is_empty()
        ));
    }

    #[test]
    fn invalid_utf8_and_binary_bytes_fail_provider_locally() {
        let provider = PlainTextPreviewProvider::new();
        let fixture = snapshot(Some("txt"), Some("text/plain"));
        assert_eq!(
            load(&provider, &fixture, &[0xff, 0xfe], true),
            Err(PreviewProviderError::CorruptSource)
        );
        assert_eq!(
            load(&provider, &fixture, b"text\0binary", true),
            Err(PreviewProviderError::CorruptSource)
        );
    }

    #[test]
    fn source_code_exposes_only_a_presentation_language_hint() {
        let provider = SourceCodePreviewProvider::new();
        let fixture = snapshot(Some("rs"), Some("text/plain"));
        let result = load(&provider, &fixture, b"fn main() {}", true).expect("code text");
        assert_eq!(
            result.representation,
            PreviewRepresentation::Text {
                text: "fn main() {}".to_string(),
                language: Some("rust".to_string())
            }
        );
        assert!(provider.descriptor().reads_content);
        assert!(!provider.descriptor().capabilities.can_search);
    }

    #[test]
    fn huge_line_stays_bounded_and_partial_is_truthful() {
        let provider = PlainTextPreviewProvider::new();
        let fixture = snapshot(Some("log"), Some("text/plain"));
        let bytes = vec![b'x'; PREVIEW_TEXT_READ_BYTES as usize];
        let result = load(&provider, &fixture, &bytes, false).expect("bounded line");
        assert_eq!(result.completeness, PreviewCompleteness::Partial);
        assert_eq!(
            match result.representation {
                PreviewRepresentation::Text { text, .. } => text.len(),
                _ => 0,
            },
            PREVIEW_TEXT_READ_BYTES as usize
        );
    }

    #[test]
    fn markdown_is_sanitized_without_resource_or_navigation_elements() {
        let provider = MarkdownPreviewProvider::new();
        let fixture = snapshot(Some("md"), Some("text/markdown"));
        let hostile = br#"# Safe

<script>alert(1)</script>
<img src="https://attacker.invalid/x" onerror="alert(2)">
[remote](https://attacker.invalid/page)
![file](file:///secret.txt)
<iframe src="//attacker.invalid/frame"></iframe>
<div style="background:url(file:///secret)">text</div>
"#;
        let result = load(&provider, &fixture, hostile, true).expect("sanitized markdown");
        let PreviewRepresentation::SafeHtml { html } = result.representation else {
            panic!("expected safe html");
        };
        for forbidden in [
            "<script",
            "<img",
            "<iframe",
            "<object",
            "<embed",
            "onerror",
            "onclick",
            "href=",
            "src=",
            "javascript:",
            "style=",
        ] {
            assert!(
                !html.to_ascii_lowercase().contains(forbidden),
                "forbidden {forbidden}: {html}"
            );
        }
        assert!(html.contains("Safe"));
    }

    #[test]
    fn preview_read_error_mapping_preserves_terminal_and_metadata_fallback_semantics() {
        assert_eq!(
            map_content_read_error(PreviewReadAccessError::MaterializationRequired),
            PreviewProviderError::MaterializationRequired
        );
        assert_eq!(
            map_content_read_error(PreviewReadAccessError::SourceUnavailable),
            PreviewProviderError::SourceUnavailable
        );
        assert_eq!(
            map_content_read_error(PreviewReadAccessError::MetadataOnly),
            PreviewProviderError::Unsupported
        );
    }

    #[test]
    fn directories_unknown_sources_and_unsupported_hosts_do_not_probe_as_text() {
        let mut directory = snapshot(Some("txt"), Some("text/plain"));
        directory.capabilities.can_select_text = false;
        let provider = PlainTextPreviewProvider::new();
        assert_eq!(
            provider.probe(&directory, &context()),
            ProviderProbe::Unsupported
        );

        let unknown = snapshot(Some("bin"), Some("application/octet-stream"));
        assert_eq!(
            provider.probe(&unknown, &context()),
            ProviderProbe::Unsupported
        );
    }
}
