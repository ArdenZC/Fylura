//! Bounded ZIP central-directory metadata Preview provider for W3-08.
//!
//! This provider deliberately has no filesystem authority.  It receives an
//! opaque Preview source and reads only through the existing Preview read
//! adapter.  The small ZIP preflight below exists to validate attacker-owned
//! count/length fields before handing the bounded reader to the reviewed
//! `zip` crate.  The provider never calls an entry payload `Read` method and
//! never creates an extraction target.

use super::{
    contracts::{ContentReadEligibility, PreviewHostKind, PreviewSourceRef},
    preview::{
        BoundedContentReadRequest, PreparedPreview, PreviewCapabilities, PreviewCompleteness,
        PreviewContentReadAccess, PreviewContextError, PreviewOperationContext, PreviewProvider,
        PreviewProviderDescriptor, PreviewProviderEnvironment, PreviewProviderError,
        PreviewProviderResult, PreviewReadAccessError, PreviewRepresentation,
        PreviewSourceSnapshot, ProviderProbe,
    },
};
use crate::scheduler::{adapters::PreviewArchiveResourceLeaseAdapter, AcquireError, ResourceLease};
use serde::Serialize;
use std::{
    io::{self, Read, Seek, SeekFrom},
    sync::{Arc, Mutex},
    time::Duration,
};
use zip::{
    read::{ArchiveOffset, Config},
    ZipArchive,
};

pub(crate) const ARCHIVE_ZIP_PROVIDER_ID: &str = "builtin.archive-zip";
pub(crate) const ARCHIVE_ZIP_PROVIDER_PRIORITY: i32 = 270;

pub(crate) const MAX_ZIP_ENTRIES_INSPECTED: usize = 20_000;
pub(crate) const MAX_ZIP_TREE_NODES: usize = 2_000;
pub(crate) const MAX_ZIP_TREE_DEPTH: usize = 64;
pub(crate) const MAX_ZIP_ENTRY_NAME_BYTES: usize = 4 * 1024;
pub(crate) const MAX_ZIP_ENTRY_NAME_CHARS: usize = 2_048;
pub(crate) const MAX_ZIP_EXTRA_METADATA_BYTES: usize = 16 * 1024;
pub(crate) const MAX_ZIP_ARCHIVE_COMMENT_BYTES: usize = 16 * 1024;
pub(crate) const MAX_ZIP_CENTRAL_DIRECTORY_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const MAX_ZIP_TOTAL_SOURCE_BYTES_READ: u64 = 12 * 1024 * 1024;
pub(crate) const MAX_ZIP_SINGLE_READ_BYTES: u32 = 1024 * 1024;
// The reader intentionally has no byte cache.  Seeks are cheap opaque read
// requests against the existing gate and an in-memory cache would be a second
// source/read authority.
pub(crate) const MAX_ZIP_READER_CACHE_BYTES: usize = 0;
pub(crate) const MAX_ARCHIVE_ENCODED_TREE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_ARCHIVE_WARNINGS: usize = 32;
pub(crate) const MAX_ARCHIVE_TREE_CHILDREN_PER_NODE: usize = 512;

/// Reserve time for the provider to return a truthful bounded result before
/// Preview Core's outer load deadline wins the race.  This is deliberately a
/// guard over the existing `PreviewOperationContext` deadline, not a second
/// timeout authority.
pub(crate) const ZIP_DEADLINE_RETURN_GUARD: Duration = Duration::from_millis(100);

const MAX_ZIP_TAIL_READ_BYTES: usize = 256 * 1024;
const ZIP_CENTRAL_HEADER_BYTES: u64 = 46;
const ZIP_LOCAL_HEADER_BYTES: u64 = 30;
const ZIP_EOCD_BYTES: u64 = 22;
const ZIP64_LOCATOR_BYTES: u64 = 20;
const ZIP64_FIXED_RECORD_BYTES: usize = 56;
const ZIP_PARSE_FIXED_OVERHEAD: u64 = 512 * 1024;
const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_ZIP_CENTRAL_SCAN_WINDOW_BYTES: usize = MAX_ZIP_SINGLE_READ_BYTES as usize;
#[allow(clippy::absurd_extreme_comparisons)]
const _: () = assert!(MAX_ZIP_READER_CACHE_BYTES <= 1024 * 1024);
const _: () = assert!(MAX_ZIP_CENTRAL_SCAN_WINDOW_BYTES <= 1024 * 1024);

const ZEN_HOSTS: &[PreviewHostKind] = &[PreviewHostKind::ZenFloating, PreviewHostKind::ZenPinned];

fn archive_capabilities() -> PreviewCapabilities {
    // Sibling navigation remains host/source-owned.  The provider does not
    // grant it to a host that did not already advertise it, while every
    // archive-entry action/search/decode capability stays disabled.
    PreviewCapabilities {
        can_navigate_siblings: true,
        ..PreviewCapabilities::default()
    }
}

pub(crate) struct ArchiveZipPreviewProvider {
    descriptor: PreviewProviderDescriptor,
}

impl ArchiveZipPreviewProvider {
    pub(crate) fn new() -> Self {
        Self {
            descriptor: PreviewProviderDescriptor::new(
                ARCHIVE_ZIP_PROVIDER_ID,
                ARCHIVE_ZIP_PROVIDER_PRIORITY,
                archive_capabilities(),
                ZEN_HOSTS.to_vec(),
                true,
            ),
        }
    }
}

impl PreviewProvider for ArchiveZipPreviewProvider {
    fn descriptor(&self) -> &PreviewProviderDescriptor {
        &self.descriptor
    }

    fn probe(
        &self,
        snapshot: &PreviewSourceSnapshot,
        _context: &PreviewOperationContext,
    ) -> ProviderProbe {
        if source_can_render_archive(snapshot) {
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
        if !source_can_render_archive(snapshot) {
            return Err(PreviewProviderError::Unsupported);
        }
        let Some(source_size) = snapshot.metadata.size_bytes else {
            // Range addressing requires an authoritative source size.  Do not
            // guess a size or fall back to an unbounded read-to-end loop.
            return Err(PreviewProviderError::Failed);
        };
        Ok(Box::new(PreparedArchiveZipPreview {
            source: snapshot.source.clone(),
            source_version: snapshot.source_version.clone(),
            source_size,
        }))
    }
}

struct PreparedArchiveZipPreview {
    source: PreviewSourceRef,
    source_version: String,
    source_size: u64,
}

impl PreparedPreview for PreparedArchiveZipPreview {
    fn load(
        &mut self,
        context: &PreviewOperationContext,
        environment: PreviewProviderEnvironment<'_>,
    ) -> Result<PreviewProviderResult, PreviewProviderError> {
        context.ensure_active().map_err(map_context_error)?;
        if deadline_guard_reached(context) {
            return Err(PreviewProviderError::Timeout);
        }
        let read_gate = environment
            .preview_read
            .ok_or(PreviewProviderError::Failed)?;
        let _archive_lease = acquire_archive_lease(context, environment.archive_admission)?;
        let read_state = Arc::new(Mutex::new(ArchiveReadState::default()));
        let mut reader = PreviewArchiveReader::new(
            read_gate,
            self.source.clone(),
            self.source_version.clone(),
            self.source_size,
            context.clone(),
            Arc::clone(&read_state),
        );

        let preflight = match preflight_zip(&mut reader) {
            Ok(preflight) => preflight,
            Err(error) => {
                let state = read_state_snapshot(&read_state);
                if state.budget_exhausted {
                    if state.structure_validated {
                        return partial_empty_result(
                            ArchiveLimitReason::SourceReadLimit,
                            state.inspected_entries,
                        );
                    }
                    return Err(error);
                }
                if matches!(error, PreviewProviderError::Timeout) {
                    if state.structure_validated {
                        return partial_empty_result(
                            ArchiveLimitReason::Deadline,
                            state.inspected_entries,
                        );
                    }
                    return Err(PreviewProviderError::Timeout);
                }
                return Err(error);
            }
        };

        if preflight.limit_reason == Some(ArchiveLimitReason::Deadline)
            || deadline_guard_reached(context)
        {
            if preflight.structure_validated {
                return partial_empty_result(
                    ArchiveLimitReason::Deadline,
                    preflight.inspected_entries,
                );
            }
            return Err(PreviewProviderError::Timeout);
        }
        if preflight.limit_reason == Some(ArchiveLimitReason::EntryLimit) {
            // The bounded central-directory pass has already established a
            // truthful entry-limit result.  Avoid handing the same large
            // directory to ZipArchive and then re-seeking every entry when
            // no entry metadata can be published under the contract.
            if preflight.structure_validated {
                return partial_empty_result(
                    ArchiveLimitReason::EntryLimit,
                    preflight.inspected_entries,
                );
            }
            return Err(PreviewProviderError::CorruptSource);
        }
        context.ensure_active().map_err(map_context_error)?;
        let config = Config {
            archive_offset: ArchiveOffset::Known(preflight.archive_offset),
        };
        let mut archive = match ZipArchive::with_config(config, reader) {
            Ok(archive) => archive,
            Err(_error) => {
                let state = read_state_snapshot(&read_state);
                if state.budget_exhausted {
                    if state.structure_validated {
                        return partial_empty_result(
                            ArchiveLimitReason::SourceReadLimit,
                            state.inspected_entries,
                        );
                    }
                    return Err(PreviewProviderError::CorruptSource);
                }
                if let Some(error) = state.failure {
                    if matches!(error, PreviewProviderError::Timeout) {
                        if state.structure_validated {
                            return partial_empty_result(
                                ArchiveLimitReason::Deadline,
                                state.inspected_entries,
                            );
                        }
                        return Err(PreviewProviderError::Timeout);
                    }
                    return Err(error);
                }
                return Err(PreviewProviderError::CorruptSource);
            }
        };

        let archive_len = archive.len();
        if archive_len > MAX_ZIP_ENTRIES_INSPECTED {
            // This should be impossible after preflight/EOCD patching.  Keep
            // the final guard so a future zip crate change cannot turn an
            // attacker-controlled count into unbounded provider work.
            return partial_empty_result(
                ArchiveLimitReason::EntryLimit,
                preflight.inspected_entries,
            );
        }

        let mut builder = ArchiveTreeBuilder::new();
        if let Some(reason) = preflight.limit_reason {
            builder.set_limit(reason);
        }
        let declared_entries = preflight.declared_entries;
        let mut index = 0usize;
        while index < archive_len {
            if deadline_guard_reached(context) {
                builder.set_limit(ArchiveLimitReason::Deadline);
                break;
            }
            match context.ensure_active() {
                Ok(()) => {}
                Err(PreviewContextError::TimedOut) => {
                    builder.set_limit(ArchiveLimitReason::Deadline);
                    break;
                }
                Err(error) => return Err(map_context_error(error)),
            }

            let metadata = match archive.by_index_raw(index) {
                Ok(file) => ArchiveEntryMetadata {
                    name: file
                        .name()
                        .map_err(|_| PreviewProviderError::CorruptSource)?
                        .into_owned(),
                    is_directory: file.is_dir(),
                    compressed_size: file.compressed_size(),
                    uncompressed_size: file.size(),
                    compression_method: format!("{:?}", file.compression()),
                    encrypted: file.encrypted(),
                },
                Err(_error) => {
                    let state = read_state_snapshot(&read_state);
                    if state.budget_exhausted {
                        builder.set_limit(ArchiveLimitReason::SourceReadLimit);
                        break;
                    }
                    if let Some(error) = state.failure {
                        match error {
                            PreviewProviderError::Timeout => {
                                builder.set_limit(ArchiveLimitReason::Deadline);
                                break;
                            }
                            PreviewProviderError::Cancelled
                            | PreviewProviderError::IdentityChanged
                            | PreviewProviderError::SourceUnavailable
                            | PreviewProviderError::MaterializationRequired
                            | PreviewProviderError::PermissionDenied => return Err(error),
                            _ => return Err(error),
                        }
                    }
                    return Err(PreviewProviderError::CorruptSource);
                }
            };

            if deadline_guard_reached(context) {
                builder.set_limit(ArchiveLimitReason::Deadline);
                break;
            }

            builder.observe_entry(&metadata);
            if !builder.insert_entry(index, metadata) {
                builder.set_limit(ArchiveLimitReason::TreeLimit);
                break;
            }
            index += 1;
        }

        let state = read_state_snapshot(&read_state);
        if state.budget_exhausted {
            builder.set_limit(ArchiveLimitReason::SourceReadLimit);
        }
        if let Some(error) = state.failure {
            match error {
                PreviewProviderError::Timeout => builder.set_limit(ArchiveLimitReason::Deadline),
                PreviewProviderError::Cancelled
                | PreviewProviderError::IdentityChanged
                | PreviewProviderError::SourceUnavailable
                | PreviewProviderError::MaterializationRequired
                | PreviewProviderError::PermissionDenied => return Err(error),
                _ => return Err(error),
            }
        }

        let complete = builder.limit_reason.is_none()
            && builder.inspected_entries as u64 == declared_entries
            && builder.inspected_entries == archive_len;
        if let Err(error) = context.ensure_active() {
            match error {
                PreviewContextError::TimedOut => builder.set_limit(ArchiveLimitReason::Deadline),
                other => return Err(map_context_error(other)),
            }
        }
        if deadline_guard_reached(context) {
            builder.set_limit(ArchiveLimitReason::Deadline);
        }
        let complete = complete && builder.limit_reason.is_none();
        let encoded_tree = builder.encode(complete)?;
        Ok(PreviewProviderResult {
            representation: PreviewRepresentation::ArchiveTree { encoded_tree },
            completeness: if complete {
                PreviewCompleteness::Complete
            } else {
                PreviewCompleteness::Partial
            },
            warnings: Vec::new(),
        })
    }

    fn cleanup(&mut self) {}
}

fn source_can_render_archive(snapshot: &PreviewSourceSnapshot) -> bool {
    snapshot.metadata.read_eligibility == ContentReadEligibility::Eligible
        && is_zip_hint(
            snapshot.metadata.extension.as_deref(),
            snapshot.metadata.media_type.as_deref(),
        )
}

fn is_zip_hint(extension: Option<&str>, media_type: Option<&str>) -> bool {
    let extension = extension
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_start_matches('.').to_ascii_lowercase());
    let media_type = media_type
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    let media_is_zip = media_type.as_deref().is_some_and(|value| {
        matches!(
            value,
            "application/zip" | "application/x-zip-compressed" | "multipart/x-zip"
        )
    });
    match extension.as_deref() {
        Some("zip") => media_type.is_none() || media_is_zip,
        Some(_) => false,
        None => media_is_zip,
    }
}

fn acquire_archive_lease(
    context: &PreviewOperationContext,
    configured_adapter: Option<&PreviewArchiveResourceLeaseAdapter>,
) -> Result<ResourceLease, PreviewProviderError> {
    match configured_adapter {
        Some(adapter) => adapter
            .try_acquire(
                context.request_id(),
                context.session_id(),
                context.scheduler_cancellation(),
            )
            .map_err(map_archive_acquire_error),
        None => PreviewArchiveResourceLeaseAdapter::global()
            .try_acquire(
                context.request_id(),
                context.session_id(),
                context.scheduler_cancellation(),
            )
            .map_err(map_archive_acquire_error),
    }
}

fn map_archive_acquire_error(error: AcquireError) -> PreviewProviderError {
    match error {
        AcquireError::Cancelled => PreviewProviderError::Cancelled,
        AcquireError::WouldBlock | AcquireError::QueueFull => PreviewProviderError::Timeout,
        AcquireError::Unavailable
        | AcquireError::PolicyDenied
        | AcquireError::InvalidRequest(_) => PreviewProviderError::Failed,
    }
}

fn map_context_error(error: PreviewContextError) -> PreviewProviderError {
    match error {
        PreviewContextError::Cancelled | PreviewContextError::StalePublication => {
            PreviewProviderError::Cancelled
        }
        PreviewContextError::TimedOut => PreviewProviderError::Timeout,
    }
}

fn deadline_guard_reached(context: &PreviewOperationContext) -> bool {
    context.remaining() <= ZIP_DEADLINE_RETURN_GUARD
}

fn map_read_error(error: PreviewReadAccessError) -> PreviewProviderError {
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

#[derive(Debug, Default, Clone, Copy)]
struct ArchiveReadState {
    charged_bytes: u64,
    budget_exhausted: bool,
    structure_validated: bool,
    inspected_entries: u64,
    failure: Option<PreviewProviderError>,
}

fn read_state_snapshot(state: &Arc<Mutex<ArchiveReadState>>) -> ArchiveReadState {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .to_owned()
}

#[derive(Clone)]
struct BytePatch {
    offset: u64,
    bytes: Vec<u8>,
}

struct PreviewArchiveReader<'a> {
    read_gate: &'a dyn PreviewContentReadAccess,
    source: PreviewSourceRef,
    source_version: String,
    source_size: u64,
    position: u64,
    context: PreviewOperationContext,
    state: Arc<Mutex<ArchiveReadState>>,
    patches: Arc<Mutex<Vec<BytePatch>>>,
}

impl<'a> PreviewArchiveReader<'a> {
    fn new(
        read_gate: &'a dyn PreviewContentReadAccess,
        source: PreviewSourceRef,
        source_version: String,
        source_size: u64,
        context: PreviewOperationContext,
        state: Arc<Mutex<ArchiveReadState>>,
    ) -> Self {
        Self {
            read_gate,
            source,
            source_version,
            source_size,
            position: 0,
            context,
            state,
            patches: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn add_patch(&self, offset: u64, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        let mut patches = self
            .patches
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if patches.len() < 8 {
            patches.push(BytePatch { offset, bytes });
        }
    }

    fn state_failure(&self, fallback: PreviewProviderError) -> PreviewProviderError {
        read_state_snapshot(&self.state).failure.unwrap_or(fallback)
    }

    fn record_failure(&self, error: PreviewProviderError) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.failure.is_none() {
            state.failure = Some(error);
        }
    }

    fn mark_structure_validated(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.structure_validated = true;
    }

    fn structure_validated(&self) -> bool {
        read_state_snapshot(&self.state).structure_validated
    }

    fn mark_entry_inspected(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.inspected_entries = state.inspected_entries.saturating_add(1);
    }

    fn charge_read(&self, requested: u64) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(next) = state.charged_bytes.checked_add(requested) else {
            state.budget_exhausted = true;
            return false;
        };
        if next > MAX_ZIP_TOTAL_SOURCE_BYTES_READ {
            state.budget_exhausted = true;
            return false;
        }
        state.charged_bytes = next;
        true
    }

    fn remaining_budget(&self) -> u64 {
        MAX_ZIP_TOTAL_SOURCE_BYTES_READ
            .saturating_sub(read_state_snapshot(&self.state).charged_bytes)
    }

    fn ensure_work_window(&self) -> Result<(), PreviewProviderError> {
        self.context.ensure_active().map_err(map_context_error)?;
        if deadline_guard_reached(&self.context) {
            return Err(PreviewProviderError::Timeout);
        }
        Ok(())
    }

    fn apply_patches(&self, offset: u64, bytes: &mut [u8]) {
        let end = offset.saturating_add(bytes.len() as u64);
        let patches = self
            .patches
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for patch in patches.iter() {
            let patch_end = patch.offset.saturating_add(patch.bytes.len() as u64);
            let overlap_start = offset.max(patch.offset);
            let overlap_end = end.min(patch_end);
            if overlap_start >= overlap_end {
                continue;
            }
            let destination_start = (overlap_start - offset) as usize;
            let source_start = (overlap_start - patch.offset) as usize;
            let length = (overlap_end - overlap_start) as usize;
            bytes[destination_start..destination_start + length]
                .copy_from_slice(&patch.bytes[source_start..source_start + length]);
        }
    }
}

impl Read for PreviewArchiveReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if let Err(error) = self.ensure_work_window() {
            self.record_failure(error);
            return Err(io::Error::new(io::ErrorKind::Interrupted, error));
        }
        if self.position >= self.source_size {
            return Ok(0);
        }
        let available = self.source_size - self.position;
        let remaining_budget = self.remaining_budget();
        if remaining_budget == 0 {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.budget_exhausted = true;
            return Err(io::Error::other("preview archive source-read limit"));
        }
        let requested = (buffer.len() as u64)
            .min(MAX_ZIP_SINGLE_READ_BYTES as u64)
            .min(available)
            .min(remaining_budget) as u32;
        if requested == 0 {
            return Ok(0);
        }
        if !self.charge_read(requested as u64) {
            return Err(io::Error::other("preview archive source-read limit"));
        }
        let offset = self.position;
        let result = self.read_gate.read_source_bounded(
            &self.source,
            &self.source_version,
            BoundedContentReadRequest {
                offset_bytes: offset,
                max_bytes: requested,
            },
            &self.context,
        );
        let read = match result {
            Ok(read) => read,
            Err(error) => {
                let mapped = map_read_error(error);
                self.record_failure(mapped);
                return Err(io::Error::other(mapped));
            }
        };
        if read.bytes.len() > requested as usize {
            self.record_failure(PreviewProviderError::Failed);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "preview archive read exceeded request",
            ));
        }
        let count = read.bytes.len();
        buffer[..count].copy_from_slice(&read.bytes);
        self.apply_patches(offset, &mut buffer[..count]);
        self.position = self.position.checked_add(count as u64).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "preview archive offset overflow",
            )
        })?;
        Ok(count)
    }
}

impl Seek for PreviewArchiveReader<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        if let Err(error) = self.ensure_work_window() {
            self.record_failure(error);
            return Err(io::Error::new(io::ErrorKind::Interrupted, error));
        }
        let next = match position {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::Current(offset) => add_signed_offset(self.position, offset),
            SeekFrom::End(offset) => add_signed_offset(self.source_size, offset),
        }
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "preview archive seek overflow")
        })?;
        if next > self.source_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "preview archive seek outside source",
            ));
        }
        self.position = next;
        Ok(next)
    }
}

fn add_signed_offset(base: u64, offset: i64) -> Option<u64> {
    if offset >= 0 {
        base.checked_add(offset as u64)
    } else {
        base.checked_sub(offset.unsigned_abs())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ArchiveLimitReason {
    EntryLimit,
    TreeLimit,
    MetadataLimit,
    SourceReadLimit,
    Deadline,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ArchiveProgressState {
    Complete,
    Partial,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum ArchiveNodeKind {
    Directory,
    File,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ArchiveWarning {
    UnsafeName,
    EntryLimit,
    TreeLimit,
    MetadataLimit,
    SourceReadLimit,
    Deadline,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct ArchiveTreePayload {
    version: u8,
    format: &'static str,
    progress: ArchiveProgress,
    totals: ArchiveTotals,
    root: ArchiveNode,
    warnings: Vec<ArchiveWarning>,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct ArchiveProgress {
    inspected_entries: usize,
    state: ArchiveProgressState,
    limit_reason: Option<ArchiveLimitReason>,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct ArchiveTotals {
    entries_observed: usize,
    files_observed: usize,
    directories_observed: usize,
    compressed_bytes_observed: u64,
    uncompressed_bytes_declared_observed: u64,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct ArchiveNode {
    kind: ArchiveNodeKind,
    name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<ArchiveNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compressed_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uncompressed_size_declared: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compression_method: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    encrypted: bool,
    #[serde(skip_serializing_if = "is_false")]
    unsafe_name: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl ArchiveNode {
    fn directory(name: impl Into<String>, unsafe_name: bool) -> Self {
        Self {
            kind: ArchiveNodeKind::Directory,
            name: name.into(),
            children: Vec::new(),
            compressed_size: None,
            uncompressed_size_declared: None,
            compression_method: None,
            encrypted: false,
            unsafe_name,
        }
    }

    fn file(name: impl Into<String>, metadata: &ArchiveEntryMetadata, unsafe_name: bool) -> Self {
        Self {
            kind: ArchiveNodeKind::File,
            name: name.into(),
            children: Vec::new(),
            compressed_size: safe_json_size(metadata.compressed_size),
            uncompressed_size_declared: safe_json_size(metadata.uncompressed_size),
            compression_method: Some(metadata.compression_method.clone()),
            encrypted: metadata.encrypted,
            unsafe_name,
        }
    }
}

#[derive(Debug, Clone)]
struct ArchiveEntryMetadata {
    name: String,
    is_directory: bool,
    compressed_size: u64,
    uncompressed_size: u64,
    compression_method: String,
    encrypted: bool,
}

fn safe_json_size(value: u64) -> Option<u64> {
    (value <= MAX_SAFE_JSON_INTEGER).then_some(value)
}

struct ArchiveTreeBuilder {
    root: ArchiveNode,
    inspected_entries: usize,
    totals: ArchiveTotals,
    warnings: Vec<ArchiveWarning>,
    limit_reason: Option<ArchiveLimitReason>,
    estimated_encoded_bytes: usize,
    node_count: usize,
}

impl ArchiveTreeBuilder {
    fn new() -> Self {
        Self {
            root: ArchiveNode::directory("", false),
            inspected_entries: 0,
            totals: ArchiveTotals {
                entries_observed: 0,
                files_observed: 0,
                directories_observed: 0,
                compressed_bytes_observed: 0,
                uncompressed_bytes_declared_observed: 0,
            },
            warnings: Vec::new(),
            limit_reason: None,
            estimated_encoded_bytes: 128,
            node_count: 1,
        }
    }

    fn observe_entry(&mut self, metadata: &ArchiveEntryMetadata) {
        self.inspected_entries = self.inspected_entries.saturating_add(1);
        self.totals.entries_observed = self.totals.entries_observed.saturating_add(1);
        if metadata.is_directory {
            self.totals.directories_observed = self.totals.directories_observed.saturating_add(1);
        } else {
            self.totals.files_observed = self.totals.files_observed.saturating_add(1);
        }
        if let Some(value) = self
            .totals
            .compressed_bytes_observed
            .checked_add(metadata.compressed_size)
        {
            self.totals.compressed_bytes_observed = value.min(MAX_SAFE_JSON_INTEGER);
            if value > MAX_SAFE_JSON_INTEGER {
                self.set_limit(ArchiveLimitReason::MetadataLimit);
            }
        } else {
            self.totals.compressed_bytes_observed = MAX_SAFE_JSON_INTEGER;
            self.set_limit(ArchiveLimitReason::MetadataLimit);
        }
        if let Some(value) = self
            .totals
            .uncompressed_bytes_declared_observed
            .checked_add(metadata.uncompressed_size)
        {
            self.totals.uncompressed_bytes_declared_observed = value.min(MAX_SAFE_JSON_INTEGER);
            if value > MAX_SAFE_JSON_INTEGER {
                self.set_limit(ArchiveLimitReason::MetadataLimit);
            }
        } else {
            self.totals.uncompressed_bytes_declared_observed = MAX_SAFE_JSON_INTEGER;
            self.set_limit(ArchiveLimitReason::MetadataLimit);
        }
        if metadata.compressed_size > MAX_SAFE_JSON_INTEGER
            || metadata.uncompressed_size > MAX_SAFE_JSON_INTEGER
        {
            self.set_limit(ArchiveLimitReason::MetadataLimit);
        }
    }

    fn insert_entry(&mut self, entry_index: usize, metadata: ArchiveEntryMetadata) -> bool {
        let name_too_large = metadata.name.len() > MAX_ZIP_ENTRY_NAME_BYTES
            || metadata.name.chars().count() > MAX_ZIP_ENTRY_NAME_CHARS;
        let unsafe_name = name_too_large || !is_safe_archive_name(&metadata.name);
        if name_too_large {
            self.set_limit(ArchiveLimitReason::MetadataLimit);
        }
        if unsafe_name {
            self.push_warning(ArchiveWarning::UnsafeName);
        }
        let display_name = bounded_display_name(&metadata.name, entry_index);
        let mut components = if unsafe_name {
            vec!["unsafe entries".to_string(), display_name.clone()]
        } else {
            split_safe_name(&metadata.name)
        };
        if components.is_empty() {
            self.push_warning(ArchiveWarning::UnsafeName);
            components = vec!["unsafe entries".to_string(), display_name];
        } else if components.len() > MAX_ZIP_TREE_DEPTH {
            // Keep the original entry inert and bounded rather than recursing
            // attacker-controlled depth into the virtual tree.
            self.set_limit(ArchiveLimitReason::MetadataLimit);
            components = vec!["unsafe entries".to_string(), display_name];
        }
        let leaf = if metadata.is_directory {
            ArchiveNode::directory(components.last().cloned().unwrap_or_default(), unsafe_name)
        } else {
            ArchiveNode::file(
                components.last().cloned().unwrap_or_default(),
                &metadata,
                unsafe_name,
            )
        };
        if components.len() > 1 {
            let parent_components = &components[..components.len() - 1];
            if !insert_directory_path(
                &mut self.root,
                parent_components,
                &mut self.estimated_encoded_bytes,
                &mut self.node_count,
            ) {
                return false;
            }
        }
        insert_leaf(
            &mut self.root,
            &components,
            leaf,
            &mut self.estimated_encoded_bytes,
            &mut self.node_count,
        )
    }

    fn set_limit(&mut self, reason: ArchiveLimitReason) {
        if self.limit_reason.is_none() {
            self.limit_reason = Some(reason);
        }
        let warning = match reason {
            ArchiveLimitReason::EntryLimit => ArchiveWarning::EntryLimit,
            ArchiveLimitReason::TreeLimit => ArchiveWarning::TreeLimit,
            ArchiveLimitReason::MetadataLimit => ArchiveWarning::MetadataLimit,
            ArchiveLimitReason::SourceReadLimit => ArchiveWarning::SourceReadLimit,
            ArchiveLimitReason::Deadline => ArchiveWarning::Deadline,
        };
        self.push_warning(warning);
    }

    fn push_warning(&mut self, warning: ArchiveWarning) {
        if self.warnings.len() < MAX_ARCHIVE_WARNINGS && !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }

    fn encode(mut self, complete: bool) -> Result<String, PreviewProviderError> {
        if !complete && self.limit_reason.is_none() {
            self.set_limit(ArchiveLimitReason::MetadataLimit);
        }
        let payload = ArchiveTreePayload {
            version: 1,
            format: "zip",
            progress: ArchiveProgress {
                inspected_entries: self.inspected_entries,
                state: if complete {
                    ArchiveProgressState::Complete
                } else {
                    ArchiveProgressState::Partial
                },
                limit_reason: self.limit_reason,
            },
            totals: self.totals,
            root: self.root,
            warnings: self.warnings,
        };
        let encoded = serde_json::to_string(&payload).map_err(|_| PreviewProviderError::Failed)?;
        if encoded.len() > MAX_ARCHIVE_ENCODED_TREE_BYTES {
            return Err(PreviewProviderError::Failed);
        }
        Ok(encoded)
    }
}

impl PartialEq for ArchiveWarning {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

fn insert_directory_path(
    node: &mut ArchiveNode,
    components: &[String],
    estimated_bytes: &mut usize,
    node_count: &mut usize,
) -> bool {
    if components.is_empty() {
        return true;
    }
    let component = &components[0];
    let Some(child_index) = node
        .children
        .iter()
        .position(|child| child.name == *component)
    else {
        if !can_add_child(node, component, estimated_bytes, node_count) {
            return false;
        }
        node.children
            .push(ArchiveNode::directory(component.clone(), false));
        return if components.len() == 1 {
            true
        } else {
            let child = node.children.last_mut().expect("directory child inserted");
            insert_directory_path(child, &components[1..], estimated_bytes, node_count)
        };
    };
    if !matches!(node.children[child_index].kind, ArchiveNodeKind::Directory) {
        return false;
    }
    if components.len() == 1 {
        true
    } else {
        insert_directory_path(
            &mut node.children[child_index],
            &components[1..],
            estimated_bytes,
            node_count,
        )
    }
}

fn insert_leaf(
    node: &mut ArchiveNode,
    components: &[String],
    leaf: ArchiveNode,
    estimated_bytes: &mut usize,
    node_count: &mut usize,
) -> bool {
    if components.is_empty() {
        return false;
    }
    if components.len() == 1 {
        let name = &components[0];
        if let Some(existing) = node.children.iter_mut().find(|child| child.name == *name) {
            if matches!(existing.kind, ArchiveNodeKind::Directory)
                && matches!(leaf.kind, ArchiveNodeKind::Directory)
            {
                return true;
            }
            return false;
        }
        if !can_add_child(node, name, estimated_bytes, node_count) {
            return false;
        }
        node.children.push(leaf);
        return true;
    }
    let Some(child_index) = node
        .children
        .iter()
        .position(|child| child.name == components[0])
    else {
        if !can_add_child(node, &components[0], estimated_bytes, node_count) {
            return false;
        }
        node.children
            .push(ArchiveNode::directory(components[0].clone(), false));
        let child = node.children.last_mut().expect("directory child inserted");
        return insert_leaf(child, &components[1..], leaf, estimated_bytes, node_count);
    };
    if !matches!(node.children[child_index].kind, ArchiveNodeKind::Directory) {
        return false;
    }
    insert_leaf(
        &mut node.children[child_index],
        &components[1..],
        leaf,
        estimated_bytes,
        node_count,
    )
}

fn can_add_child(
    node: &ArchiveNode,
    name: &str,
    estimated_bytes: &mut usize,
    node_count: &mut usize,
) -> bool {
    if node.children.len() >= MAX_ARCHIVE_TREE_CHILDREN_PER_NODE {
        return false;
    }
    if *node_count >= MAX_ZIP_TREE_NODES {
        return false;
    }
    let Some(next) = estimated_bytes.checked_add(name.len().saturating_add(192)) else {
        return false;
    };
    if next > MAX_ARCHIVE_ENCODED_TREE_BYTES.saturating_sub(128) {
        return false;
    }
    *estimated_bytes = next;
    *node_count = (*node_count).saturating_add(1);
    true
}

fn split_safe_name(name: &str) -> Vec<String> {
    name.split(['/', '\\'])
        .filter(|component| !component.is_empty())
        .map(str::to_owned)
        .collect()
}

fn bounded_display_name(name: &str, entry_index: usize) -> String {
    let mut bounded = name
        .chars()
        .take(MAX_ZIP_ENTRY_NAME_CHARS)
        .collect::<String>();
    if bounded.is_empty() {
        bounded = format!("unnamed entry {}", entry_index.saturating_add(1));
    }
    bounded
}

fn is_safe_archive_name(name: &str) -> bool {
    if name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.as_bytes().get(1) == Some(&b':')
        || name.chars().any(char::is_control)
        || name.chars().any(is_normalization_sensitive_path_character)
    {
        return false;
    }
    let components = name.split(['/', '\\']).collect::<Vec<_>>();
    let has_trailing_separator = components.last() == Some(&"");
    let non_empty_components = if has_trailing_separator {
        &components[..components.len().saturating_sub(1)]
    } else {
        components.as_slice()
    };
    if non_empty_components.is_empty()
        || non_empty_components
            .iter()
            .any(|component| component.is_empty() || *component == ".." || *component == ".")
    {
        return false;
    }
    // A single trailing separator is the normal directory spelling.  Any
    // internal or repeated separator remains unsafe; no OS path is built.
    true
}

fn is_normalization_sensitive_path_character(character: char) -> bool {
    // These compatibility characters are common fullwidth/small/leader forms
    // of path punctuation. The provider never normalizes names, but marking
    // them unsafe prevents a future presentation or extraction consumer from
    // treating a normalized spelling as a root escape.
    matches!(
        character,
        '\u{2024}' // ONE DOT LEADER -> .
            | '\u{2025}' // TWO DOT LEADER -> ..
            | '\u{2026}' // HORIZONTAL ELLIPSIS -> ...
            | '\u{FE52}' // SMALL FULL STOP -> .
            | '\u{FF0E}' // FULLWIDTH FULL STOP -> .
            | '\u{FF0F}' // FULLWIDTH SOLIDUS -> /
            | '\u{FF1A}' // FULLWIDTH COLON -> :
            | '\u{FF3C}' // FULLWIDTH REVERSE SOLIDUS -> \
    )
}

fn partial_empty_result(
    reason: ArchiveLimitReason,
    inspected_entries: u64,
) -> Result<PreviewProviderResult, PreviewProviderError> {
    let mut builder = ArchiveTreeBuilder::new();
    // A bounded central-directory preflight may have inspected many records
    // without publishing entry metadata. Preserve that authoritative progress
    // rather than reporting a misleading zero-count partial result.
    builder.inspected_entries = inspected_entries
        .min(MAX_ZIP_ENTRIES_INSPECTED as u64)
        .try_into()
        .expect("bounded ZIP inspected count fits usize");
    builder.set_limit(reason);
    let encoded_tree = builder.encode(false)?;
    Ok(PreviewProviderResult {
        representation: PreviewRepresentation::ArchiveTree { encoded_tree },
        completeness: PreviewCompleteness::Partial,
        warnings: Vec::new(),
    })
}

#[derive(Debug, Clone, Copy)]
struct ArchivePreflight {
    archive_offset: u64,
    declared_entries: u64,
    inspected_entries: u64,
    limit_reason: Option<ArchiveLimitReason>,
    structure_validated: bool,
}

fn preflight_zip(
    reader: &mut PreviewArchiveReader<'_>,
) -> Result<ArchivePreflight, PreviewProviderError> {
    reader.ensure_work_window()?;
    let tail_len = reader.source_size.min(MAX_ZIP_TAIL_READ_BYTES as u64) as usize;
    if tail_len < ZIP_EOCD_BYTES as usize {
        return Err(PreviewProviderError::CorruptSource);
    }
    let tail_start = reader.source_size - tail_len as u64;
    let mut tail = vec![0_u8; tail_len];
    read_exact_at(reader, tail_start, &mut tail)?;
    reader.ensure_work_window()?;
    let eocd_tail_offset = find_eocd(&tail).ok_or(PreviewProviderError::CorruptSource)?;
    let eocd_offset = tail_start
        .checked_add(eocd_tail_offset as u64)
        .ok_or(PreviewProviderError::CorruptSource)?;
    let comment_length = read_u16(&tail, eocd_tail_offset + 20) as usize;
    let eocd_end = eocd_offset
        .checked_add(ZIP_EOCD_BYTES)
        .and_then(|value| value.checked_add(comment_length as u64))
        .ok_or(PreviewProviderError::CorruptSource)?;
    if eocd_end > reader.source_size {
        return Err(PreviewProviderError::CorruptSource);
    }

    let disk_number = read_u16(&tail, eocd_tail_offset + 4);
    let central_disk = read_u16(&tail, eocd_tail_offset + 6);
    let entries_on_disk = read_u16(&tail, eocd_tail_offset + 8) as u64;
    let entries_total = read_u16(&tail, eocd_tail_offset + 10) as u64;
    let central_size_32 = read_u32(&tail, eocd_tail_offset + 12) as u64;
    let central_offset_32 = read_u32(&tail, eocd_tail_offset + 16) as u64;
    if disk_number != 0 || central_disk != 0 || entries_on_disk != entries_total {
        return Err(PreviewProviderError::CorruptSource);
    }
    reader.ensure_work_window()?;

    let needs_zip64 = entries_total == u16::MAX as u64
        || entries_on_disk == u16::MAX as u64
        || central_size_32 == u32::MAX as u64
        || central_offset_32 == u32::MAX as u64;
    let mut patches = Vec::<(u64, Vec<u8>)>::new();
    let mut zip64_count_offsets = None;
    let (declared_entries, central_size, central_offset) = if needs_zip64 {
        if eocd_offset < ZIP64_LOCATOR_BYTES {
            return Err(PreviewProviderError::CorruptSource);
        }
        let locator_offset = eocd_offset - ZIP64_LOCATOR_BYTES;
        let mut locator = [0_u8; ZIP64_LOCATOR_BYTES as usize];
        read_exact_at(reader, locator_offset, &mut locator)?;
        reader.ensure_work_window()?;
        if read_u32(&locator, 0) != 0x0706_4b50 || read_u32(&locator, 16) != 1 {
            return Err(PreviewProviderError::CorruptSource);
        }
        let record_offset = read_u64(&locator, 8);
        let record_end = record_offset
            .checked_add(ZIP64_FIXED_RECORD_BYTES as u64)
            .ok_or(PreviewProviderError::CorruptSource)?;
        if record_end > locator_offset || record_end > reader.source_size {
            return Err(PreviewProviderError::CorruptSource);
        }
        let mut record = [0_u8; ZIP64_FIXED_RECORD_BYTES];
        read_exact_at(reader, record_offset, &mut record)?;
        reader.ensure_work_window()?;
        if read_u32(&record, 0) != 0x0606_4b50 {
            return Err(PreviewProviderError::CorruptSource);
        }
        let record_size = read_u64(&record, 4);
        if record_size < 44
            || record_size > (MAX_ZIP_ARCHIVE_COMMENT_BYTES + 44) as u64
            || record_offset
                .checked_add(12)
                .and_then(|value| value.checked_add(record_size))
                .is_none_or(|value| value > locator_offset)
        {
            return Err(PreviewProviderError::CorruptSource);
        }
        let disk_number_64 = read_u32(&record, 12);
        let central_disk_64 = read_u32(&record, 16);
        let entries_on_disk_64 = read_u64(&record, 24);
        let entries_total_64 = read_u64(&record, 32);
        if disk_number_64 != 0 || central_disk_64 != 0 || entries_on_disk_64 != entries_total_64 {
            return Err(PreviewProviderError::CorruptSource);
        }
        let central_size = read_u64(&record, 40);
        let central_offset = read_u64(&record, 48);
        zip64_count_offsets = Some((record_offset + 24, record_offset + 32));
        if entries_total_64 > MAX_ZIP_ENTRIES_INSPECTED as u64 {
            patches.push((
                record_offset + 24,
                (MAX_ZIP_ENTRIES_INSPECTED as u64).to_le_bytes().to_vec(),
            ));
            patches.push((
                record_offset + 32,
                (MAX_ZIP_ENTRIES_INSPECTED as u64).to_le_bytes().to_vec(),
            ));
        }
        (entries_total_64, central_size, central_offset)
    } else {
        if entries_total > MAX_ZIP_ENTRIES_INSPECTED as u64 {
            patches.push((
                eocd_offset + 8,
                (MAX_ZIP_ENTRIES_INSPECTED as u16).to_le_bytes().to_vec(),
            ));
            patches.push((
                eocd_offset + 10,
                (MAX_ZIP_ENTRIES_INSPECTED as u16).to_le_bytes().to_vec(),
            ));
        }
        (entries_total, central_size_32, central_offset_32)
    };

    let mut limit_reason = if declared_entries > MAX_ZIP_ENTRIES_INSPECTED as u64 {
        Some(ArchiveLimitReason::EntryLimit)
    } else {
        None
    };
    if comment_length > MAX_ZIP_ARCHIVE_COMMENT_BYTES {
        reader.add_patch(
            eocd_offset + 20,
            (MAX_ZIP_ARCHIVE_COMMENT_BYTES as u16)
                .to_le_bytes()
                .to_vec(),
        );
        if limit_reason.is_none() {
            limit_reason = Some(ArchiveLimitReason::MetadataLimit);
        }
    }
    for (offset, bytes) in patches {
        reader.add_patch(offset, bytes);
    }

    reader.ensure_work_window()?;

    if central_offset > reader.source_size {
        return Err(PreviewProviderError::CorruptSource);
    }
    // The EOCD is immediately after the central directory.  Deriving the
    // physical start from that boundary avoids scanning from a claimed
    // relative offset through local headers or entry payload bytes when an
    // archive has a prepended stub.  The offset is used only to derive the
    // ZIP reader's logical archive offset; it is never a read target itself.
    let central_start = eocd_offset
        .checked_sub(central_size)
        .ok_or(PreviewProviderError::CorruptSource)?;
    if central_start < central_offset {
        return Err(PreviewProviderError::CorruptSource);
    }
    let central_end = central_start
        .checked_add(central_size)
        .ok_or(PreviewProviderError::CorruptSource)?;
    if central_end > eocd_offset || central_end > reader.source_size {
        return Err(PreviewProviderError::CorruptSource);
    }
    if central_size > MAX_ZIP_CENTRAL_DIRECTORY_BYTES {
        limit_reason.get_or_insert(ArchiveLimitReason::MetadataLimit);
    }

    let archive_offset = central_start.saturating_sub(central_offset);
    if declared_entries == 0 && central_start == central_end {
        // EOCD plus an empty, bounded central directory is sufficient to
        // establish a truthful empty ZIP result without reading an entry.
        reader.mark_structure_validated();
    }
    let safe_entry_count = scan_central_directory(
        reader,
        central_start,
        central_end,
        declared_entries,
        &mut limit_reason,
    )?;
    if limit_reason == Some(ArchiveLimitReason::Deadline) {
        return Ok(ArchivePreflight {
            archive_offset,
            declared_entries,
            inspected_entries: safe_entry_count,
            limit_reason,
            structure_validated: reader.structure_validated(),
        });
    }
    if safe_entry_count < declared_entries.min(MAX_ZIP_ENTRIES_INSPECTED as u64) {
        patch_entry_count(
            reader,
            eocd_offset,
            safe_entry_count as u64,
            zip64_count_offsets,
        )?;
    }
    Ok(ArchivePreflight {
        archive_offset,
        declared_entries,
        inspected_entries: safe_entry_count,
        limit_reason,
        structure_validated: reader.structure_validated(),
    })
}

fn find_eocd(tail: &[u8]) -> Option<usize> {
    if tail.len() < ZIP_EOCD_BYTES as usize {
        return None;
    }
    (0..=tail.len() - ZIP_EOCD_BYTES as usize)
        .rev()
        .find(|offset| {
            read_u32(tail, *offset) == 0x0605_4b50
                && offset
                    .checked_add(ZIP_EOCD_BYTES as usize)
                    .and_then(|value| value.checked_add(read_u16(tail, *offset + 20) as usize))
                    .is_some_and(|value| value <= tail.len())
        })
}

fn scan_central_directory(
    reader: &mut PreviewArchiveReader<'_>,
    central_start: u64,
    central_end: u64,
    declared_entries: u64,
    limit_reason: &mut Option<ArchiveLimitReason>,
) -> Result<u64, PreviewProviderError> {
    let max_entries = declared_entries.min(MAX_ZIP_ENTRIES_INSPECTED as u64);
    let bounded_end = central_start
        .saturating_add(MAX_ZIP_CENTRAL_DIRECTORY_BYTES)
        .min(central_end);
    let mut position = central_start;
    let mut inspected = 0_u64;
    let mut scan_window = CentralDirectoryScanWindow::default();
    if declared_entries == 0 {
        if central_start != central_end {
            return Err(PreviewProviderError::CorruptSource);
        }
        return Ok(0);
    }
    while inspected < max_entries {
        match reader.context.ensure_active() {
            Ok(()) => {}
            Err(PreviewContextError::TimedOut) => {
                *limit_reason = Some(ArchiveLimitReason::Deadline);
                break;
            }
            Err(error) => return Err(map_context_error(error)),
        }
        if deadline_guard_reached(&reader.context) {
            *limit_reason = Some(ArchiveLimitReason::Deadline);
            break;
        }
        let remaining = reader.remaining_budget();
        let projected = ZIP_CENTRAL_HEADER_BYTES
            .saturating_mul(2)
            .saturating_add(ZIP_LOCAL_HEADER_BYTES)
            .saturating_add(ZIP_PARSE_FIXED_OVERHEAD);
        if remaining <= projected {
            *limit_reason = Some(ArchiveLimitReason::SourceReadLimit);
            break;
        }
        let Some(header_end) = position.checked_add(ZIP_CENTRAL_HEADER_BYTES) else {
            return Err(PreviewProviderError::CorruptSource);
        };
        if header_end > central_end {
            return Err(PreviewProviderError::CorruptSource);
        }
        if header_end > bounded_end {
            *limit_reason = Some(ArchiveLimitReason::MetadataLimit);
            break;
        }
        let header = scan_window.read_header(reader, position, bounded_end)?;
        if read_u32(&header, 0) != 0x0201_4b50 {
            return Err(PreviewProviderError::CorruptSource);
        }
        let name_length = read_u16(&header, 28) as u64;
        let extra_length = read_u16(&header, 30) as u64;
        let comment_length = read_u16(&header, 32) as u64;
        if name_length > MAX_ZIP_ENTRY_NAME_BYTES as u64
            || extra_length > MAX_ZIP_EXTRA_METADATA_BYTES as u64
            || comment_length > MAX_ZIP_EXTRA_METADATA_BYTES as u64
        {
            *limit_reason = Some(ArchiveLimitReason::MetadataLimit);
            break;
        }
        let entry_bytes = ZIP_CENTRAL_HEADER_BYTES
            .checked_add(name_length)
            .and_then(|value| value.checked_add(extra_length))
            .and_then(|value| value.checked_add(comment_length))
            .ok_or(PreviewProviderError::CorruptSource)?;
        let next_position = position
            .checked_add(entry_bytes)
            .ok_or(PreviewProviderError::CorruptSource)?;
        if next_position > central_end {
            return Err(PreviewProviderError::CorruptSource);
        }
        if next_position > bounded_end {
            *limit_reason = Some(ArchiveLimitReason::MetadataLimit);
            break;
        }
        if inspected == 0 {
            // Do not publish an ArchiveTree merely because an EOCD-shaped
            // byte sequence was found. The first bounded central record must
            // be structurally valid before timeout partials are truthful.
            reader.mark_structure_validated();
        }
        position = next_position;
        inspected = inspected.saturating_add(1);
        reader.mark_entry_inspected();
        let future_cost = ZIP_CENTRAL_HEADER_BYTES
            .saturating_add(entry_bytes)
            .saturating_add(ZIP_LOCAL_HEADER_BYTES)
            .saturating_add(ZIP_PARSE_FIXED_OVERHEAD);
        if reader.remaining_budget() < future_cost && inspected < max_entries {
            *limit_reason = Some(ArchiveLimitReason::SourceReadLimit);
            break;
        }
    }
    if declared_entries > MAX_ZIP_ENTRIES_INSPECTED as u64 && inspected == max_entries {
        *limit_reason = Some(ArchiveLimitReason::EntryLimit);
    }
    Ok(inspected)
}

#[derive(Default)]
struct CentralDirectoryScanWindow {
    start: u64,
    end: u64,
    bytes: Vec<u8>,
}

impl CentralDirectoryScanWindow {
    /// Batch the bounded central-directory preflight without creating a
    /// persistent source cache or changing the opaque read authority.  The
    /// window is local to one provider call and never exceeds one read bound.
    fn read_header(
        &mut self,
        reader: &mut PreviewArchiveReader<'_>,
        position: u64,
        bounded_end: u64,
    ) -> Result<[u8; ZIP_CENTRAL_HEADER_BYTES as usize], PreviewProviderError> {
        let header_end = position
            .checked_add(ZIP_CENTRAL_HEADER_BYTES)
            .ok_or(PreviewProviderError::CorruptSource)?;
        if position < self.start || header_end > self.end {
            let window_end = position
                .saturating_add(MAX_ZIP_CENTRAL_SCAN_WINDOW_BYTES as u64)
                .min(bounded_end);
            let window_len = window_end
                .checked_sub(position)
                .filter(|length| *length >= ZIP_CENTRAL_HEADER_BYTES)
                .ok_or(PreviewProviderError::CorruptSource)? as usize;
            self.bytes.resize(window_len, 0);
            read_exact_at(reader, position, &mut self.bytes)?;
            self.start = position;
            self.end = window_end;
        }

        let offset = position
            .checked_sub(self.start)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or(PreviewProviderError::CorruptSource)?;
        let end = offset
            .checked_add(ZIP_CENTRAL_HEADER_BYTES as usize)
            .ok_or(PreviewProviderError::CorruptSource)?;
        let mut header = [0_u8; ZIP_CENTRAL_HEADER_BYTES as usize];
        header.copy_from_slice(
            self.bytes
                .get(offset..end)
                .ok_or(PreviewProviderError::CorruptSource)?,
        );
        Ok(header)
    }
}

fn patch_entry_count(
    reader: &PreviewArchiveReader<'_>,
    eocd_offset: u64,
    count: u64,
    zip64_count_offsets: Option<(u64, u64)>,
) -> Result<(), PreviewProviderError> {
    if let Some((on_disk_offset, total_offset)) = zip64_count_offsets {
        reader.add_patch(on_disk_offset, count.to_le_bytes().to_vec());
        reader.add_patch(total_offset, count.to_le_bytes().to_vec());
    } else if count <= u16::MAX as u64 {
        reader.add_patch(eocd_offset + 8, (count as u16).to_le_bytes().to_vec());
        reader.add_patch(eocd_offset + 10, (count as u16).to_le_bytes().to_vec());
    } else {
        // ZIP64 count patches are installed during the initial ZIP64 footer
        // pass.  A count above the provider inspection cap can never reach
        // this branch, but keeping the guard explicit prevents silent wrap.
        return Err(PreviewProviderError::Failed);
    }
    Ok(())
}

fn read_exact_at(
    reader: &mut PreviewArchiveReader<'_>,
    offset: u64,
    buffer: &mut [u8],
) -> Result<(), PreviewProviderError> {
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|_| reader.state_failure(PreviewProviderError::Failed))?;
    if let Err(_error) = reader.read_exact(buffer) {
        return Err(reader.state_failure(PreviewProviderError::CorruptSource));
    }
    reader.ensure_work_window()?;
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_workspace::contracts::{MaterializationState, PreviewSourceRef};
    use crate::file_workspace::preview::{
        BoundedContentRead, PreviewExecution, PreviewExecutionError, PreviewExecutionLane,
        PreviewHost, PreviewMetadata, PreviewProviderEnvironmentHandle, PreviewProviderErrorCode,
        PreviewProviderRegistry, PreviewResolveRequest, PreviewSession, PreviewSessionConfig,
        PreviewSourceSnapshot, PreviewWarning, SourceResolveError, SourceResolver,
    };
    use std::{
        io::Cursor,
        sync::{Arc, Mutex},
        time::Instant,
    };

    #[derive(Clone)]
    struct StaticArchiveResolver {
        snapshot: PreviewSourceSnapshot,
    }

    impl SourceResolver for StaticArchiveResolver {
        fn resolve(
            &self,
            request: &PreviewResolveRequest,
            context: &PreviewOperationContext,
        ) -> Result<PreviewSourceSnapshot, SourceResolveError> {
            context
                .ensure_active()
                .map_err(|_| SourceResolveError::Timeout)?;
            if request.source != self.snapshot.source {
                return Err(SourceResolveError::SourceMismatch);
            }
            Ok(self.snapshot.clone())
        }
    }

    struct InlinePreviewExecution;

    impl PreviewExecution for InlinePreviewExecution {
        fn submit(
            &self,
            _lane: PreviewExecutionLane,
            _name: &str,
            work: Box<dyn FnOnce() + Send + 'static>,
        ) -> Result<(), PreviewExecutionError> {
            work();
            Ok(())
        }
    }

    #[test]
    fn provider_identity_and_bounds_are_stable() {
        let provider = ArchiveZipPreviewProvider::new();
        let descriptor = provider.descriptor();
        assert_eq!(descriptor.id, ARCHIVE_ZIP_PROVIDER_ID);
        assert_eq!(descriptor.priority, ARCHIVE_ZIP_PROVIDER_PRIORITY);
        assert_eq!(
            descriptor.supported_hosts,
            vec![PreviewHostKind::ZenFloating, PreviewHostKind::ZenPinned]
        );
        assert!(descriptor.reads_content);
        assert!(descriptor.capabilities.can_navigate_siblings);
        assert!(!descriptor.capabilities.can_search);
        assert!(!descriptor.capabilities.can_navigate_internal);
        let context = PreviewOperationContext::for_backend_content_read(
            "archive-probe-session",
            "archive-probe-request",
            "probe-v1",
            Default::default(),
            std::time::Instant::now() + std::time::Duration::from_secs(5),
        );
        let snapshot = PreviewSourceSnapshot::new(
            PreviewSourceRef::HostProvided {
                host_token: "probe".into(),
            },
            "probe-v1",
            PreviewMetadata {
                display_name: "probe.zip".into(),
                media_type: Some("application/zip".into()),
                extension: Some("zip".into()),
                size_bytes: Some(22),
                modified_at_epoch_ms: None,
                materialization: MaterializationState::Local,
                read_eligibility: ContentReadEligibility::Eligible,
            },
            PreviewCapabilities::default(),
        );
        assert_eq!(
            provider.probe(&snapshot, &context),
            ProviderProbe::Compatible
        );
        let mut unsupported = snapshot.clone();
        unsupported.metadata.extension = Some("txt".into());
        assert_eq!(
            provider.probe(&unsupported, &context),
            ProviderProbe::Unsupported
        );
        assert_eq!(ARCHIVE_ZIP_PROVIDER_ID, "builtin.archive-zip");
        assert_eq!(ARCHIVE_ZIP_PROVIDER_PRIORITY, 270);
        assert_eq!(
            ZIP_DEADLINE_RETURN_GUARD,
            std::time::Duration::from_millis(100)
        );
        assert_eq!(MAX_ZIP_SINGLE_READ_BYTES, 1024 * 1024);
    }

    #[test]
    fn hostile_names_remain_inert_and_bounded() {
        for name in ["dir/", "a/b/", "a/b/c/", "folder/file.txt"] {
            assert!(
                is_safe_archive_name(name),
                "expected safe nested archive name: {name:?}"
            );
        }
        for name in [
            "/a/b",
            "../a",
            "a/../b",
            "a/./b",
            "a//b",
            "a///b",
            "C:\\a",
            "\\\\server\\share",
            "control/\0name",
            "a/．．/b",
            "a/／b",
        ] {
            assert!(
                !is_safe_archive_name(name),
                "expected unsafe archive name: {name:?}"
            );
        }
        assert!(!is_safe_archive_name("../escape.txt"));
        assert!(!is_safe_archive_name("/absolute.txt"));
        assert!(!is_safe_archive_name("C:\\absolute.txt"));
        assert!(!is_safe_archive_name("\\\\server\\share\\x"));
        assert!(!is_safe_archive_name("safe/\0name"));
        assert!(!is_safe_archive_name("．．/escape.txt"));
        assert!(!is_safe_archive_name("folder／file.txt"));
        assert!(is_safe_archive_name("folder/file.txt"));
        assert!(
            bounded_display_name(&"x".repeat(5000), 0).chars().count() <= MAX_ZIP_ENTRY_NAME_CHARS
        );
    }

    #[test]
    fn nested_directory_entries_stay_in_the_virtual_tree() {
        let bytes = zip_bytes(&[
            ("dir/", true, false),
            ("a/b/", true, false),
            ("a/b/c/", true, false),
            ("folder/file.txt", false, false),
        ]);
        let gate = TestArchiveReadGate::new(bytes);
        let result = load_archive_for_test(&gate, "nested-directory-v1")
            .expect("nested directory ZIP metadata");
        let PreviewRepresentation::ArchiveTree { encoded_tree } = result.representation else {
            panic!("nested directory ZIP did not publish ArchiveTree");
        };
        let payload: serde_json::Value = serde_json::from_str(&encoded_tree).expect("tree JSON");
        let root = payload["root"].to_string();
        assert!(!root.contains("unsafe entries"));
        assert!(root.contains("a"));
        assert!(root.contains("b"));
        assert!(root.contains("c"));
        assert!(root.contains("folder"));
        assert!(root.contains("file.txt"));
    }

    #[test]
    fn zip_hint_requires_an_archive_hint_without_cross_type_mismatch() {
        assert!(is_zip_hint(Some(".ZIP"), None));
        assert!(is_zip_hint(None, Some("application/zip")));
        assert!(!is_zip_hint(Some("txt"), Some("application/zip")));
        assert!(!is_zip_hint(Some("zip"), Some("text/plain")));
    }

    #[test]
    fn archive_reader_never_requests_more_than_one_megabyte() {
        let source_bytes = vec![0_u8; (MAX_ZIP_SINGLE_READ_BYTES as usize) * 2];
        let reader = TestArchiveReadGate::new(source_bytes);
        let source = PreviewSourceRef::HostProvided {
            host_token: "test".into(),
        };
        let context = PreviewOperationContext::for_backend_content_read(
            "archive-reader-session",
            "archive-reader-request",
            "v1",
            Default::default(),
            std::time::Instant::now() + std::time::Duration::from_secs(5),
        );
        let state = Arc::new(Mutex::new(ArchiveReadState::default()));
        let mut bounded_reader = PreviewArchiveReader::new(
            &reader,
            source,
            "v1".into(),
            (MAX_ZIP_SINGLE_READ_BYTES as u64) * 2,
            context,
            state,
        );
        let mut bytes = vec![0_u8; (MAX_ZIP_SINGLE_READ_BYTES as usize) * 2];
        assert_eq!(
            bounded_reader.read(&mut bytes).expect("first bounded read"),
            MAX_ZIP_SINGLE_READ_BYTES as usize
        );
        assert_eq!(
            bounded_reader
                .read(&mut bytes)
                .expect("second bounded read"),
            MAX_ZIP_SINGLE_READ_BYTES as usize
        );
        assert!(reader
            .requests()
            .iter()
            .all(|request| request.max_bytes <= MAX_ZIP_SINGLE_READ_BYTES));
        assert_eq!(
            reader.total_requested(),
            MAX_ZIP_TOTAL_SOURCE_BYTES_READ.min((MAX_ZIP_SINGLE_READ_BYTES as u64) * 2)
        );
    }

    #[test]
    fn many_small_seeks_cannot_bypass_total_source_read_budget() {
        let reader = TestArchiveReadGate::new(vec![0_u8; 1024]);
        let source = PreviewSourceRef::HostProvided {
            host_token: "small-seek-test".into(),
        };
        let context = PreviewOperationContext::for_backend_content_read(
            "archive-small-seek-session",
            "archive-small-seek-request",
            "v1",
            Default::default(),
            std::time::Instant::now() + std::time::Duration::from_secs(30),
        );
        let state = Arc::new(Mutex::new(ArchiveReadState::default()));
        let mut bounded_reader = PreviewArchiveReader::new(
            &reader,
            source,
            "v1".into(),
            MAX_ZIP_TOTAL_SOURCE_BYTES_READ + 1024,
            context,
            Arc::clone(&state),
        );
        let mut buffer = [0_u8; 1024];
        for _ in 0..(MAX_ZIP_TOTAL_SOURCE_BYTES_READ / 1024) {
            bounded_reader
                .seek(SeekFrom::Start(0))
                .expect("bounded seek");
            assert_eq!(
                bounded_reader
                    .read(&mut buffer)
                    .expect("bounded small read"),
                1024
            );
        }
        bounded_reader
            .seek(SeekFrom::Start(0))
            .expect("final bounded seek");
        assert!(bounded_reader.read(&mut buffer).is_err());
        assert_eq!(
            read_state_snapshot(&state).charged_bytes,
            MAX_ZIP_TOTAL_SOURCE_BYTES_READ
        );
        assert_eq!(reader.total_requested(), MAX_ZIP_TOTAL_SOURCE_BYTES_READ);
    }

    #[test]
    fn empty_zip_is_complete_and_unicode_nested_names_are_metadata_only() {
        let empty = zip_bytes(&[]);
        let empty_gate = TestArchiveReadGate::new(empty.clone());
        let empty_result = load_archive_for_test(&empty_gate, "empty-v1").expect("empty ZIP");
        assert_eq!(empty_result.completeness, PreviewCompleteness::Complete);
        let PreviewRepresentation::ArchiveTree {
            encoded_tree: empty_tree,
        } = empty_result.representation
        else {
            panic!("empty ZIP did not publish ArchiveTree");
        };
        let empty_payload: serde_json::Value =
            serde_json::from_str(&empty_tree).expect("empty ZIP payload");
        assert_eq!(empty_payload["progress"]["inspectedEntries"], 0);
        assert_eq!(empty_payload["root"]["children"], serde_json::Value::Null);

        let entries = [
            ("数据/文件.txt", false, false),
            ("数据/nested/second.txt", false, false),
        ];
        let nested = zip_bytes(&entries);
        let nested_gate = TestArchiveReadGate::new(nested);
        let nested_result = load_archive_for_test(&nested_gate, "unicode-v1").expect("nested ZIP");
        assert_eq!(nested_result.completeness, PreviewCompleteness::Complete);
        let PreviewRepresentation::ArchiveTree { encoded_tree } = nested_result.representation
        else {
            panic!("nested ZIP did not publish ArchiveTree");
        };
        assert!(encoded_tree.contains("数据"));
        assert!(encoded_tree.contains("文件.txt"));
        assert!(!nested_gate.reads_payload_data());
    }

    #[test]
    fn rustsec_zip_symlink_and_privileged_modes_stay_inert_metadata() {
        for mode in [0o120777_u32, 0o106755_u32] {
            let mut bytes = zip_bytes(&[("../escape.txt", false, false)]);
            let central = central_offset(&bytes);
            bytes[central + 5] = 3; // Unix creator, including symlink/SUID/SGID mode bits.
            bytes[central + 38..central + 42].copy_from_slice(&(mode << 16).to_le_bytes());
            let gate = TestArchiveReadGate::new(bytes);
            let result = load_archive_for_test(&gate, "hostile-mode-v1").unwrap();
            let PreviewRepresentation::ArchiveTree { encoded_tree } = result.representation else {
                panic!("expected metadata tree");
            };
            assert!(encoded_tree.contains("unsafe_name"));
            assert!(gate.total_requested() <= MAX_ZIP_TOTAL_SOURCE_BYTES_READ);
            // Source access is only through this read adapter: no extraction
            // destination, symlink creation or Unix permission application exists.
            assert!(!gate.reads_payload_data());
        }
    }

    #[test]
    fn zip9_legacy_encoded_member_name_is_preserved_as_inert_metadata() {
        let mut bytes = zip_bytes(&[("bad.txt", false, false)]);
        let central = central_offset(&bytes);
        bytes[6..8].copy_from_slice(&0x0800_u16.to_le_bytes());
        bytes[central + 8..central + 10].copy_from_slice(&0x0800_u16.to_le_bytes());
        bytes[30] = 0x82; // zip 9 decodes non-UTF8 names through CP437.
        bytes[central + 46] = 0x82;
        let gate = TestArchiveReadGate::new(bytes);
        let result = load_archive_for_test(&gate, "legacy-name-v1").unwrap();
        let PreviewRepresentation::ArchiveTree { encoded_tree } = result.representation else {
            panic!("expected metadata tree");
        };
        assert!(encoded_tree.contains("éad.txt"));
        assert!(gate.total_requested() <= MAX_ZIP_TOTAL_SOURCE_BYTES_READ);
    }

    #[test]
    fn source_version_drift_is_terminal_and_never_publishes_archive_tree() {
        let bytes = zip_bytes(&[("file.txt", false, false)]);
        let gate = TestArchiveReadGate::new(bytes).expect_source_version("new-version");
        let result = load_archive_for_test(&gate, "old-version");
        assert!(matches!(result, Err(PreviewProviderError::IdentityChanged)));
    }

    #[test]
    fn corrupt_eocd_and_malformed_offsets_fall_back_without_index_publication() {
        let bytes = zip_bytes(&[("file.txt", false, false)]);
        let truncated = bytes[..bytes.len().saturating_sub(5)].to_vec();
        let truncated_gate = TestArchiveReadGate::new(truncated);
        assert!(matches!(
            load_archive_for_test(&truncated_gate, "truncated-v1"),
            Err(PreviewProviderError::CorruptSource)
        ));

        let mut malformed = bytes;
        let eocd = eocd_offset(&malformed);
        malformed[eocd + 16..eocd + 20].copy_from_slice(&u32::MAX.to_le_bytes());
        let malformed_gate = TestArchiveReadGate::new(malformed);
        assert!(matches!(
            load_archive_for_test(&malformed_gate, "offset-v1"),
            Err(PreviewProviderError::CorruptSource)
        ));
    }

    #[test]
    fn read_timeout_before_zip_validation_remains_provider_timeout() {
        let gate = TestArchiveReadGate::new(zip_bytes(&[("file.txt", false, false)])).timed_out();
        let result = load_archive_for_test(&gate, "prevalidation-timeout-v1");
        assert!(matches!(result, Err(PreviewProviderError::Timeout)));
        assert!(gate.requests().is_empty());
    }

    #[test]
    fn deadline_guard_before_zip_validation_remains_provider_timeout() {
        let gate = TestArchiveReadGate::new(zip_bytes(&[("file.txt", false, false)]));
        let deadline = Instant::now() + ZIP_DEADLINE_RETURN_GUARD / 2;
        let result = load_archive_for_test_with_deadline(&gate, "guard-v1", deadline);
        assert!(matches!(result, Err(PreviewProviderError::Timeout)));
        assert!(
            gate.requests().is_empty(),
            "guard should stop before an underlying read"
        );
    }

    #[test]
    fn corrupt_zip_near_deadline_never_publishes_archive_tree() {
        let gate = TestArchiveReadGate::new(vec![0xa5; ZIP_EOCD_BYTES as usize]);
        let deadline = Instant::now() + ZIP_DEADLINE_RETURN_GUARD / 2;
        let result =
            load_archive_for_test_with_deadline(&gate, "corrupt-near-deadline-v1", deadline);
        assert!(matches!(result, Err(PreviewProviderError::Timeout)));
        assert!(gate.requests().is_empty());
    }

    #[test]
    fn validated_zip_timeout_during_bounded_index_returns_partial_archive_tree() {
        let gate = TestArchiveReadGate::new(zip_bytes(&[
            ("first.txt", false, false),
            ("second.txt", false, false),
        ]))
        .timed_out_after(2);
        let result = load_archive_for_test(&gate, "validated-partial-v1")
            .expect("validated ZIP timeout should publish a partial archive");
        assert_eq!(result.completeness, PreviewCompleteness::Partial);
        let PreviewRepresentation::ArchiveTree { encoded_tree } = result.representation else {
            panic!("validated ZIP timeout did not publish partial ArchiveTree");
        };
        let payload: serde_json::Value = serde_json::from_str(&encoded_tree).expect("tree JSON");
        assert_eq!(payload["progress"]["state"], "partial");
        assert_eq!(payload["progress"]["limitReason"], "deadline");
        assert_eq!(payload["progress"]["inspectedEntries"], 2);
        assert_eq!(gate.requests().len(), 2);
    }

    #[test]
    fn deadline_guard_before_validation_falls_back_to_metadata_at_session_level() {
        let bytes = zip_bytes(&[("file.txt", false, false)]);
        let gate = Arc::new(TestArchiveReadGate::new(bytes.clone()));
        let source = PreviewSourceRef::HostProvided {
            host_token: "session-guard".into(),
        };
        let snapshot = PreviewSourceSnapshot::new(
            source.clone(),
            "session-guard-v1",
            PreviewMetadata {
                display_name: "fixture.zip".into(),
                media_type: Some("application/zip".into()),
                extension: Some("zip".into()),
                size_bytes: Some(bytes.len() as u64),
                modified_at_epoch_ms: None,
                materialization: MaterializationState::Local,
                read_eligibility: ContentReadEligibility::Eligible,
            },
            PreviewCapabilities::default(),
        );
        let mut config = PreviewSessionConfig::new(
            "session-guard",
            "session-guard-request",
            source,
            PreviewHost::new(PreviewHostKind::ZenFloating, PreviewCapabilities::all()),
        );
        config.budget.load_timeout = ZIP_DEADLINE_RETURN_GUARD / 2;
        let session = PreviewSession::with_execution(config, Arc::new(InlinePreviewExecution));
        let archive_admission = Arc::new(test_archive_adapter());
        let environment = PreviewProviderEnvironmentHandle {
            content_read: None,
            preview_read: Some(gate.clone()),
            folder_enumeration: None,
            asset_publisher: None,
            decoder_admission: None,
            archive_admission: Some(archive_admission),
        };
        let registry = Arc::new(
            PreviewProviderRegistry::new(vec![
                Arc::new(ArchiveZipPreviewProvider::new()) as Arc<dyn PreviewProvider>
            ])
            .expect("archive provider registry"),
        );

        let outcome = session
            .run_with_environment(
                Arc::new(StaticArchiveResolver { snapshot }),
                registry,
                environment,
            )
            .expect("pre-validation timeout should reach the outer session fallback");
        assert!(outcome.provider_id.is_none());
        assert_eq!(outcome.envelope.completeness, PreviewCompleteness::Complete);
        assert!(matches!(
            &outcome.envelope.representation,
            PreviewRepresentation::Metadata { .. }
        ));
        assert!(outcome.envelope.warnings.iter().any(|warning| matches!(
            warning,
            PreviewWarning::ProviderFallback {
                provider_id,
                reason: PreviewProviderErrorCode::Timeout,
            } if provider_id == ARCHIVE_ZIP_PROVIDER_ID
        )));
        assert!(outcome
            .envelope
            .warnings
            .iter()
            .any(|warning| matches!(warning, PreviewWarning::MetadataFallback)));
        assert!(gate.requests().is_empty());
    }

    #[test]
    fn cancellation_and_materialization_drift_remain_terminal() {
        let bytes = zip_bytes(&[("file.txt", false, false)]);
        let cancelled = TestArchiveReadGate::new(bytes.clone())
            .with_read_error(PreviewReadAccessError::Cancelled);
        assert!(matches!(
            load_archive_for_test(&cancelled, "cancelled-v1"),
            Err(PreviewProviderError::Cancelled)
        ));
        let materialization = TestArchiveReadGate::new(bytes)
            .with_read_error(PreviewReadAccessError::MaterializationRequired);
        assert!(matches!(
            load_archive_for_test(&materialization, "materialization-v1"),
            Err(PreviewProviderError::MaterializationRequired)
        ));
    }

    #[test]
    fn metadata_and_tree_limits_remain_partial_and_bounded() {
        let mut builder = ArchiveTreeBuilder::new();
        for index in 0..(MAX_ZIP_TREE_NODES + 64) {
            let metadata = ArchiveEntryMetadata {
                name: format!("entry-{index}.txt"),
                is_directory: false,
                compressed_size: 1,
                uncompressed_size: 1,
                compression_method: "Stored".into(),
                encrypted: false,
            };
            builder.observe_entry(&metadata);
            if !builder.insert_entry(index, metadata) {
                builder.set_limit(ArchiveLimitReason::TreeLimit);
                break;
            }
        }
        assert_eq!(builder.limit_reason, Some(ArchiveLimitReason::TreeLimit));
        assert!(builder.node_count <= MAX_ZIP_TREE_NODES);
        let encoded = builder.encode(false).expect("bounded tree payload");
        assert!(encoded.len() <= MAX_ARCHIVE_ENCODED_TREE_BYTES);

        let mut deep = ArchiveTreeBuilder::new();
        let deep_metadata = ArchiveEntryMetadata {
            name: format!("{}/file.txt", ["deep"; MAX_ZIP_TREE_DEPTH + 1].join("/")),
            is_directory: false,
            compressed_size: 1,
            uncompressed_size: 1,
            compression_method: "Stored".into(),
            encrypted: false,
        };
        deep.observe_entry(&deep_metadata);
        assert!(deep.insert_entry(0, deep_metadata));
        assert_eq!(deep.limit_reason, Some(ArchiveLimitReason::MetadataLimit));
        assert!(deep.warnings.contains(&ArchiveWarning::MetadataLimit));

        let mut aggregate = ArchiveTreeBuilder::new();
        let huge = ArchiveEntryMetadata {
            name: "huge.bin".into(),
            is_directory: false,
            compressed_size: u64::MAX,
            uncompressed_size: u64::MAX,
            compression_method: "Stored".into(),
            encrypted: false,
        };
        aggregate.observe_entry(&huge);
        assert_eq!(
            aggregate.limit_reason,
            Some(ArchiveLimitReason::MetadataLimit)
        );
        assert_eq!(
            aggregate.totals.compressed_bytes_observed,
            MAX_SAFE_JSON_INTEGER
        );
        assert_eq!(
            aggregate.totals.uncompressed_bytes_declared_observed,
            MAX_SAFE_JSON_INTEGER
        );
        assert!(aggregate.insert_entry(0, huge));
        let aggregate_payload = aggregate.encode(false).expect("aggregate payload");
        assert!(aggregate_payload.len() <= MAX_ARCHIVE_ENCODED_TREE_BYTES);
    }

    #[test]
    fn huge_comment_extra_and_declared_sizes_publish_partial_metadata_only() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .set_comment("c".repeat(MAX_ZIP_ARCHIVE_COMMENT_BYTES + 1))
            .expect("ZIP format accepts the 16 KiB plus one comment fixture");
        writer
            .start_file("huge.bin", zip::write::SimpleFileOptions::default())
            .expect("file entry");
        std::io::Write::write_all(&mut writer, b"payload").expect("payload");
        let mut bytes = writer.finish().expect("zip finish").into_inner();
        let eocd = eocd_offset(&bytes);
        let central = central_offset(&bytes);
        bytes[central + 30..central + 32]
            .copy_from_slice(&(MAX_ZIP_EXTRA_METADATA_BYTES as u16 + 1).to_le_bytes());
        bytes[central + 24..central + 28].copy_from_slice(&u32::MAX.to_le_bytes());
        let gate = TestArchiveReadGate::new(bytes);
        let result = load_archive_for_test(&gate, "metadata-limit-v1").expect("partial metadata");
        assert_eq!(result.completeness, PreviewCompleteness::Partial);
        let PreviewRepresentation::ArchiveTree { encoded_tree } = result.representation else {
            panic!("metadata limit did not publish ArchiveTree");
        };
        let payload: serde_json::Value = serde_json::from_str(&encoded_tree).expect("payload JSON");
        assert_eq!(payload["progress"]["state"], "partial");
        assert_eq!(payload["progress"]["limitReason"], "metadata_limit");
        assert_eq!(payload["totals"]["entriesObserved"], 0);
        assert!(eocd + ZIP_EOCD_BYTES as usize <= gate.bytes.len());
    }

    #[test]
    fn encrypted_and_nested_zip_entries_stay_inert_metadata() {
        let mut bytes = zip_bytes(&[("nested.zip", false, true)]);
        let eocd = eocd_offset(&bytes);
        let central = central_offset(&bytes);
        bytes[8] |= 1;
        bytes[central + 8] |= 1;
        let gate = TestArchiveReadGate::new(bytes);
        let result = load_archive_for_test(&gate, "encrypted-v1").expect("encrypted metadata");
        assert_eq!(result.completeness, PreviewCompleteness::Complete);
        let PreviewRepresentation::ArchiveTree { encoded_tree } = result.representation else {
            panic!("encrypted entry did not publish ArchiveTree");
        };
        let payload: serde_json::Value = serde_json::from_str(&encoded_tree).expect("payload JSON");
        assert!(encoded_tree.contains("nested.zip"));
        assert_eq!(payload["root"]["children"][0]["encrypted"], true);
        assert!(!gate.reads_payload_data());
        assert!(eocd < gate.bytes.len());
    }

    #[test]
    fn representative_scale_profiles_report_bounded_backend_evidence() {
        for count in [
            1_000_usize,
            10_000,
            MAX_ZIP_ENTRIES_INSPECTED,
            MAX_ZIP_ENTRIES_INSPECTED + 1,
        ] {
            let bytes = zip_many(count);
            let gate = TestArchiveReadGate::new(bytes);
            let started = std::time::Instant::now();
            let result = load_archive_for_test(&gate, &format!("scale-{count}"))
                .expect("scale archive metadata");
            let elapsed = started.elapsed();
            let PreviewRepresentation::ArchiveTree { encoded_tree } = result.representation else {
                panic!("scale archive did not publish ArchiveTree");
            };
            let payload: serde_json::Value =
                serde_json::from_str(&encoded_tree).expect("scale JSON");
            let rendered_nodes = json_node_count(&payload["root"]);
            println!(
                "[w3-08-scale] entries={} elapsed_ms={} source_bytes={} read_count={} source_read_bytes={} rendered_nodes={} encoded_bytes={} completeness={:?} limit_reason={}",
                count,
                elapsed.as_millis(),
                gate.bytes.len(),
                gate.requests().len(),
                gate.total_requested(),
                rendered_nodes,
                encoded_tree.len(),
                result.completeness,
                payload["progress"]["limitReason"]
            );
            assert!(gate.total_requested() <= MAX_ZIP_TOTAL_SOURCE_BYTES_READ);
            assert!(rendered_nodes <= MAX_ZIP_TREE_NODES);
            assert!(encoded_tree.len() <= MAX_ARCHIVE_ENCODED_TREE_BYTES);
            if count > MAX_ZIP_ENTRIES_INSPECTED {
                assert_eq!(result.completeness, PreviewCompleteness::Partial);
                assert_eq!(payload["progress"]["limitReason"], "entry_limit");
                assert_eq!(
                    payload["progress"]["inspectedEntries"],
                    MAX_ZIP_ENTRIES_INSPECTED
                );
                assert!(
                    gate.requests().len() <= 64,
                    "bounded central-directory preflight should batch reads, got {}",
                    gate.requests().len()
                );
            }
        }
    }

    fn zip_many(count: usize) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for index in 0..count {
            writer
                .start_file(
                    format!("entry-{index:05}.txt"),
                    zip::write::SimpleFileOptions::default(),
                )
                .expect("scale file entry");
            std::io::Write::write_all(&mut writer, b"x").expect("scale payload");
        }
        writer.finish().expect("scale zip finish").into_inner()
    }

    fn eocd_offset(bytes: &[u8]) -> usize {
        bytes
            .windows(4)
            .rposition(|window| window == [0x50, 0x4b, 0x05, 0x06])
            .expect("EOCD signature")
    }

    fn central_offset(bytes: &[u8]) -> usize {
        let eocd = eocd_offset(bytes);
        read_u32(bytes, eocd + 16) as usize
    }

    fn json_node_count(value: &serde_json::Value) -> usize {
        let children = value
            .get("children")
            .and_then(serde_json::Value::as_array)
            .map(|nodes| nodes.iter().map(json_node_count).sum())
            .unwrap_or(0);
        1 + children
    }

    #[test]
    fn stored_and_deflated_entries_publish_metadata_without_payload_reads() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .add_directory("folder/", zip::write::SimpleFileOptions::default())
            .expect("directory");
        writer
            .start_file(
                "folder/stored.txt",
                zip::write::SimpleFileOptions::default(),
            )
            .expect("stored file");
        std::io::Write::write_all(&mut writer, b"stored payload").expect("stored bytes");
        writer
            .start_file(
                "folder/deflated.txt",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .expect("deflated file");
        std::io::Write::write_all(&mut writer, b"deflated payload that is never read")
            .expect("deflated bytes");
        let bytes = writer.finish().expect("zip finish").into_inner();
        let gate = TestArchiveReadGate::new(bytes.clone());
        let source = PreviewSourceRef::HostProvided {
            host_token: "archive-test".into(),
        };
        let snapshot = PreviewSourceSnapshot::new(
            source,
            "archive-v1",
            PreviewMetadata {
                display_name: "fixture.zip".into(),
                media_type: Some("application/zip".into()),
                extension: Some("zip".into()),
                size_bytes: Some(bytes.len() as u64),
                modified_at_epoch_ms: None,
                materialization: MaterializationState::Local,
                read_eligibility: ContentReadEligibility::Eligible,
            },
            PreviewCapabilities::default(),
        );
        let provider = ArchiveZipPreviewProvider::new();
        let context = PreviewOperationContext::for_backend_content_read(
            "archive-test-session",
            "archive-test-request",
            "archive-v1",
            Default::default(),
            std::time::Instant::now() + std::time::Duration::from_secs(5),
        );
        let mut prepared = provider.prepare(&snapshot, &context).expect("prepare");
        let archive_admission = test_archive_adapter();
        let environment = PreviewProviderEnvironment {
            content_read: None,
            preview_read: Some(&gate),
            folder_enumeration: None,
            publication: None,
            asset_publisher: None,
            decoder_admission: None,
            archive_admission: Some(&archive_admission),
        };
        let result = prepared
            .load(&context, environment)
            .expect("archive metadata");
        assert_eq!(result.completeness, PreviewCompleteness::Complete);
        let PreviewRepresentation::ArchiveTree { encoded_tree } = result.representation else {
            panic!("archive provider returned a non-tree representation");
        };
        let payload: serde_json::Value = serde_json::from_str(&encoded_tree).expect("payload json");
        assert_eq!(payload["format"], "zip");
        assert_eq!(payload["progress"]["state"], "complete");
        assert_eq!(payload["totals"]["filesObserved"], 2);
        assert!(encoded_tree.contains("stored.txt"));
        assert!(encoded_tree.contains("deflated.txt"));
        assert!(gate
            .requests()
            .iter()
            .all(|request| request.max_bytes <= MAX_ZIP_SINGLE_READ_BYTES));
        assert!(gate.total_requested() <= MAX_ZIP_TOTAL_SOURCE_BYTES_READ);
        assert!(!gate.reads_payload_data());
    }

    fn zip_bytes(entries: &[(&str, bool, bool)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, is_directory, deflated) in entries {
            if *is_directory {
                writer
                    .add_directory(*name, zip::write::SimpleFileOptions::default())
                    .expect("directory entry");
                continue;
            }
            let options = if *deflated {
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
            } else {
                zip::write::SimpleFileOptions::default()
            };
            writer.start_file(*name, options).expect("file entry");
            std::io::Write::write_all(&mut writer, b"archive payload that is never read")
                .expect("file payload");
        }
        writer.finish().expect("zip finish").into_inner()
    }

    fn load_archive_for_test(
        gate: &TestArchiveReadGate,
        source_version: &str,
    ) -> Result<PreviewProviderResult, PreviewProviderError> {
        load_archive_for_test_with_deadline(
            gate,
            source_version,
            std::time::Instant::now() + std::time::Duration::from_secs(30),
        )
    }

    fn load_archive_for_test_with_deadline(
        gate: &TestArchiveReadGate,
        source_version: &str,
        deadline: std::time::Instant,
    ) -> Result<PreviewProviderResult, PreviewProviderError> {
        let source = PreviewSourceRef::HostProvided {
            host_token: "archive-helper".into(),
        };
        let snapshot = PreviewSourceSnapshot::new(
            source,
            source_version,
            PreviewMetadata {
                display_name: "fixture.zip".into(),
                media_type: Some("application/zip".into()),
                extension: Some("zip".into()),
                size_bytes: Some(gate.bytes.len() as u64),
                modified_at_epoch_ms: None,
                materialization: MaterializationState::Local,
                read_eligibility: ContentReadEligibility::Eligible,
            },
            PreviewCapabilities::default(),
        );
        let context = PreviewOperationContext::for_backend_content_read(
            "archive-helper-session",
            "archive-helper-request",
            source_version,
            Default::default(),
            deadline,
        );
        let provider = ArchiveZipPreviewProvider::new();
        let mut prepared = provider.prepare(&snapshot, &context)?;
        let archive_admission = test_archive_adapter();
        let environment = PreviewProviderEnvironment {
            content_read: None,
            preview_read: Some(gate),
            folder_enumeration: None,
            publication: None,
            asset_publisher: None,
            decoder_admission: None,
            archive_admission: Some(&archive_admission),
        };
        prepared.load(&context, environment)
    }

    fn test_archive_adapter() -> PreviewArchiveResourceLeaseAdapter {
        PreviewArchiveResourceLeaseAdapter::new(Arc::new(crate::scheduler::WorkScheduler::new(
            crate::scheduler::SchedulerConfig::default()
                .with_capacities(crate::scheduler::ResourceCapacities::new(1, 1, 8, 1, 1, 1))
                .with_policy(Arc::new(crate::scheduler::PermissiveResourcePolicy)),
        )))
    }

    struct TestArchiveReadGate {
        bytes: Vec<u8>,
        requests: Mutex<Vec<BoundedContentReadRequest>>,
        expected_source_version: Option<String>,
        read_error: Option<PreviewReadAccessError>,
        timeout_after_requests: Option<usize>,
    }

    impl TestArchiveReadGate {
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                bytes,
                requests: Mutex::new(Vec::new()),
                expected_source_version: None,
                read_error: None,
                timeout_after_requests: None,
            }
        }

        fn expect_source_version(mut self, source_version: &str) -> Self {
            self.expected_source_version = Some(source_version.to_string());
            self
        }

        fn with_read_error(mut self, error: PreviewReadAccessError) -> Self {
            self.read_error = Some(error);
            self
        }

        fn timed_out(self) -> Self {
            self.with_read_error(PreviewReadAccessError::TimedOut)
        }

        fn timed_out_after(mut self, successful_requests: usize) -> Self {
            self.timeout_after_requests = Some(successful_requests);
            self
        }

        fn requests(&self) -> Vec<BoundedContentReadRequest> {
            self.requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }

        fn total_requested(&self) -> u64 {
            self.requests()
                .iter()
                .map(|request| request.max_bytes as u64)
                .sum()
        }

        fn reads_payload_data(&self) -> bool {
            let requests = self.requests();
            // The fixture's first local header is 0..30 and its payload starts
            // after the first local variable fields.  The provider may seek
            // to local headers and central metadata, but it must not request
            // the known payload bytes themselves.
            requests
                .iter()
                .any(|request| request.offset_bytes >= 45 && request.offset_bytes < 60)
        }
    }

    impl PreviewContentReadAccess for TestArchiveReadGate {
        fn read_source_bounded(
            &self,
            _source: &PreviewSourceRef,
            source_version: &str,
            request: BoundedContentReadRequest,
            _context: &PreviewOperationContext,
        ) -> Result<BoundedContentRead, PreviewReadAccessError> {
            if let Some(error) = self.read_error {
                return Err(error);
            }
            if self
                .expected_source_version
                .as_deref()
                .is_some_and(|expected| expected != source_version)
            {
                return Err(PreviewReadAccessError::SourceVersionMismatch);
            }
            assert!(request.max_bytes <= MAX_ZIP_SINGLE_READ_BYTES);
            let mut requests = self
                .requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if self
                .timeout_after_requests
                .is_some_and(|limit| requests.len() >= limit)
            {
                return Err(PreviewReadAccessError::TimedOut);
            }
            requests.push(request);
            let start = request.offset_bytes as usize;
            if start >= self.bytes.len() {
                return Ok(BoundedContentRead {
                    bytes: Vec::new(),
                    complete: true,
                });
            }
            let end = start
                .saturating_add(request.max_bytes as usize)
                .min(self.bytes.len());
            Ok(BoundedContentRead {
                bytes: self.bytes[start..end].to_vec(),
                complete: end == self.bytes.len(),
            })
        }
    }
}
