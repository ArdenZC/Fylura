//! Versioned IPC protocol and hardened named-pipe transport for the Windows
//! global-index service.
//!
//! The service accepts metadata-index commands only from the same installed
//! Zen Canvas executable running in an interactive session. It never receives
//! arbitrary paths or file-operation commands, and service shutdown remains an
//! SCM-only operation.

use crate::global_index::models::{GlobalEntry, GlobalEntryInput, GlobalVolume};
use serde::{Deserialize, Serialize};

pub const IPC_PROTOCOL_VERSION: u16 = 4;
pub const INDEX_SERVICE_PIPE: &str = r"\\.\pipe\ZenCanvas.GlobalIndex.v4";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IndexServiceCommand {
    DiscoverSources,
    StartInitialIndex {
        source_id: String,
    },
    ResumeIncrementalSync {
        source_id: String,
    },
    Pause,
    Status,
    Rebuild {
        source_id: String,
    },
    /// Retained for wire compatibility with pre-v3 clients. The hardened
    /// validator always rejects it; the SCM is the only shutdown authority.
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IndexServiceRequest {
    pub protocol_version: u16,
    pub request_id: String,
    pub command: IndexServiceCommand,
    #[serde(default)]
    pub source: Option<GlobalVolume>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IndexServiceResponse {
    pub protocol_version: u16,
    pub request_id: String,
    pub ok: bool,
    pub error_code: Option<String>,
    pub message: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "event")]
#[serde(deny_unknown_fields)]
pub enum IndexServiceEvent {
    Sources {
        sources: Vec<GlobalVolume>,
    },
    Entries {
        entries: Vec<GlobalEntryInput>,
    },
    EntryStale {
        entry_id: String,
    },
    VolumeEntriesStale {
        volume_id: String,
    },
    Checkpoint {
        volume_id: String,
        journal_id: Option<String>,
        journal_cursor: Option<String>,
    },
    SourceState {
        volume_id: String,
        status: String,
        error: Option<String>,
    },
    SourceProvider {
        volume_id: String,
        provider: String,
    },
    ResolveParentPath {
        lookup_id: String,
        volume_id: String,
        parent_platform_file_id: String,
    },
    FindEntryByIdentity {
        lookup_id: String,
        volume_id: String,
        platform_file_id: String,
        parent_platform_file_id: String,
        name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "response")]
#[serde(deny_unknown_fields)]
pub enum IndexServiceLookupResponse {
    ParentPath {
        lookup_id: String,
        path: Option<String>,
    },
    Entry {
        lookup_id: String,
        entry: Box<Option<GlobalEntry>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[serde(deny_unknown_fields)]
pub enum IndexServiceFrame {
    Event {
        request_id: String,
        event: IndexServiceEvent,
    },
    LookupResponse {
        response: IndexServiceLookupResponse,
    },
    Response {
        response: IndexServiceResponse,
    },
}

impl IndexServiceRequest {
    pub fn new(command: IndexServiceCommand, source: Option<GlobalVolume>) -> Self {
        Self {
            protocol_version: IPC_PROTOCOL_VERSION,
            request_id: uuid::Uuid::new_v4().to_string(),
            command,
            source,
        }
    }
}

pub fn validate_request(request: &IndexServiceRequest) -> Result<(), String> {
    if request.protocol_version != IPC_PROTOCOL_VERSION {
        return Err("unsupported_index_service_protocol".to_string());
    }
    if request.request_id.trim().is_empty() || request.request_id.len() > 128 {
        return Err("invalid_index_service_request_id".to_string());
    }
    match &request.command {
        IndexServiceCommand::StartInitialIndex { source_id }
        | IndexServiceCommand::ResumeIncrementalSync { source_id }
        | IndexServiceCommand::Rebuild { source_id } => {
            if source_id.trim().is_empty() || source_id.len() > 256 {
                return Err("invalid_index_service_source_id".to_string());
            }
            let source = request
                .source
                .as_ref()
                .ok_or_else(|| "index_service_source_snapshot_required".to_string())?;
            validate_source_snapshot(source)?;
            if source.id != *source_id {
                return Err("index_service_source_identity_mismatch".into());
            }
        }
        IndexServiceCommand::Shutdown => {
            return Err("index_service_shutdown_via_scm_only".to_string());
        }
        IndexServiceCommand::DiscoverSources
        | IndexServiceCommand::Pause
        | IndexServiceCommand::Status => {
            if request.source.is_some() {
                return Err("index_service_unexpected_source_snapshot".to_string());
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_source_snapshot(source: &GlobalVolume) -> Result<(), String> {
    for (field, value, max) in [
        ("id", source.id.as_str(), 256usize),
        ("stable_volume_id", source.stable_volume_id.as_str(), 1024),
        ("mount_path", source.mount_path.as_str(), 4096),
        ("filesystem_type", source.filesystem_type.as_str(), 128),
        ("provider", source.provider.as_str(), 128),
    ] {
        if value.trim().is_empty() || value.len() > max {
            return Err(format!("index_service_invalid_source_{field}"));
        }
    }
    Ok(())
}

pub fn direct_mode_reason() -> &'static str {
    "The Windows index service is unavailable; using the in-process least-privilege fallback."
}

/// Only connect failure before dispatch permits Direct. A write/read failure after
/// dispatch is ambiguous; a protocol rejection is not a remote outcome claim.
#[derive(Debug, thiserror::Error)]
pub(crate) enum IndexServiceCallError {
    #[error("request was not sent: {0}")]
    NotSent(String),
    #[error("invalid service protocol: {0}")]
    Protocol(String),
    #[error("service transport outcome unknown: {0}")]
    UnknownOutcome(String),
    #[error("local index sink failed: {0}")]
    Sink(#[from] crate::global_index::coordinator::GlobalIndexError),
}

impl IndexServiceCallError {
    pub(crate) fn into_global(self) -> crate::global_index::coordinator::GlobalIndexError {
        use crate::global_index::coordinator::GlobalIndexError;
        match self {
            Self::Protocol(message) => GlobalIndexError::ServiceProtocol(message),
            Self::UnknownOutcome(message) => GlobalIndexError::ServiceOutcomeUnknown(message),
            Self::NotSent(message) => GlobalIndexError::Provider(message),
            Self::Sink(error) => error,
        }
    }
}

pub(crate) trait IndexServiceIo {
    fn send_request(&mut self, request: &IndexServiceRequest) -> Result<(), IndexServiceCallError>;
    fn read_frame(&mut self) -> Result<IndexServiceFrame, IndexServiceCallError>;
    fn send_lookup(&mut self, response: IndexServiceLookupResponse) -> Result<(), String>;
}

/// Shared by the actual named-pipe client and deterministic transport tests.
/// Validate the envelope before the callback can touch a database or reply.
pub(crate) fn exchange<F>(
    request: &IndexServiceRequest,
    io: &mut impl IndexServiceIo,
    mut event: F,
) -> Result<IndexServiceResponse, IndexServiceCallError>
where
    F: FnMut(
        IndexServiceEvent,
    ) -> Result<Option<IndexServiceLookupResponse>, IndexServiceCallError>,
{
    validate_request(request).map_err(IndexServiceCallError::Protocol)?;
    io.send_request(request)?;
    loop {
        match io.read_frame()? {
            IndexServiceFrame::Event {
                request_id,
                event: item,
            } => {
                if request_id != request.request_id {
                    return Err(IndexServiceCallError::Protocol(
                        "event request identity mismatch".into(),
                    ));
                }
                if let Some(response) = event(item)? {
                    io.send_lookup(response)
                        .map_err(IndexServiceCallError::UnknownOutcome)?;
                }
            }
            IndexServiceFrame::Response { response } => {
                if response.protocol_version != IPC_PROTOCOL_VERSION
                    || response.request_id != request.request_id
                {
                    return Err(IndexServiceCallError::Protocol(
                        "response version/request identity mismatch".into(),
                    ));
                }
                if (response.ok && response.error_code.is_some())
                    || (!response.ok
                        && !response
                            .error_code
                            .as_deref()
                            .is_some_and(super::service_errors::known_code))
                {
                    return Err(IndexServiceCallError::Protocol(
                        "invalid response error_code/ok structure".into(),
                    ));
                }
                return Ok(response);
            }
            IndexServiceFrame::LookupResponse { .. } => {
                return Err(IndexServiceCallError::Protocol(
                    "unsolicited lookup response".into(),
                ))
            }
        }
    }
}

pub(crate) trait IndexServiceConnection: Send {
    fn send_event(&mut self, event: IndexServiceEvent) -> Result<(), String>;
    fn send_response(&mut self, response: IndexServiceResponse) -> Result<(), String>;
    fn read_lookup_response(
        &mut self,
        expected_lookup_id: &str,
    ) -> Result<IndexServiceLookupResponse, String>;
}

/// In-memory validation of an identity-bound v3 scan stream, not a persistent
/// Source ownership claim. EntryStale retains its original one-field wire shape.
pub(crate) struct IndexServiceAdmission<'a> {
    request: &'a IndexServiceRequest,
    admitted: bool,
    provider: bool,
    stale: bool,
    checkpoint: bool,
    fault: Option<String>,
    lookups: std::collections::HashSet<String>,
    owned_entries: std::collections::HashSet<String>,
}

impl<'a> IndexServiceAdmission<'a> {
    pub(crate) fn new(request: &'a IndexServiceRequest) -> Self {
        Self {
            request,
            admitted: false,
            provider: false,
            stale: false,
            checkpoint: false,
            fault: None,
            lookups: Default::default(),
            owned_entries: Default::default(),
        }
    }

    fn reject(message: &str) -> IndexServiceCallError {
        IndexServiceCallError::Protocol(message.into())
    }

    pub(crate) fn apply(
        &mut self,
        sink: &mut dyn crate::global_index::coordinator::GlobalIndexSink,
        event: IndexServiceEvent,
    ) -> Result<Option<IndexServiceLookupResponse>, IndexServiceCallError> {
        use crate::global_index::models::*;
        let source = self
            .request
            .source
            .as_ref()
            .ok_or_else(|| Self::reject("scan source absent"))?;
        if let IndexServiceEvent::Sources { sources } = &event {
            if self.admitted
                || sources.len() != 1
                || !sources.first().is_some_and(|offered| {
                    offered.id == source.id
                        && offered.stable_volume_id == source.stable_volume_id
                        && offered.mount_path.eq_ignore_ascii_case(&source.mount_path)
                        && offered
                            .filesystem_type
                            .eq_ignore_ascii_case(&source.filesystem_type)
                        && offered.platform == source.platform
                        && offered.provider == PROVIDER_WINDOWS_MFT_USN
                        && offered.enabled == source.enabled
                        && offered.journal_id == source.journal_id
                        && offered.journal_cursor == source.journal_cursor
                })
            {
                return Err(Self::reject("invalid or duplicate admission"));
            }
            let kind = match &self.request.command {
                IndexServiceCommand::StartInitialIndex { .. } => {
                    crate::global_index::repository::SourceOperationKind::Initial
                }
                IndexServiceCommand::ResumeIncrementalSync { .. } => {
                    crate::global_index::repository::SourceOperationKind::Incremental
                }
                IndexServiceCommand::Rebuild { .. } => {
                    crate::global_index::repository::SourceOperationKind::Rebuild
                }
                _ => return Err(Self::reject("admission for a non-index command")),
            };
            sink.admit_source_operation(
                source,
                &self.request.request_id,
                kind,
                crate::global_index::repository::SourceOperationRoute::Service,
            )
            .map_err(IndexServiceCallError::Sink)?;
            self.admitted = true;
            return Ok(None);
        }
        if !self.admitted || self.fault.is_some() {
            return Err(Self::reject("event outside admitted active scan"));
        }
        let volume = match &event {
            IndexServiceEvent::Checkpoint { volume_id, .. }
            | IndexServiceEvent::SourceState { volume_id, .. }
            | IndexServiceEvent::SourceProvider { volume_id, .. }
            | IndexServiceEvent::VolumeEntriesStale { volume_id }
            | IndexServiceEvent::ResolveParentPath { volume_id, .. }
            | IndexServiceEvent::FindEntryByIdentity { volume_id, .. } => Some(volume_id),
            _ => None,
        };
        if volume.is_some_and(|id| id != &source.id) {
            return Err(Self::reject("foreign volume event"));
        }
        let full = !matches!(
            self.request.command,
            IndexServiceCommand::ResumeIncrementalSync { .. }
        );
        match event {
            IndexServiceEvent::SourceProvider {
                volume_id,
                provider,
            } => {
                if self.provider
                    || self.checkpoint
                    || provider != crate::global_index::models::PROVIDER_WINDOWS_MFT_USN
                {
                    return Err(Self::reject("invalid scan provider/order"));
                }
                sink.set_source_provider(&volume_id, &provider)?;
                self.provider = true;
            }
            IndexServiceEvent::VolumeEntriesStale { volume_id } => {
                if !self.provider || self.stale || (full && self.checkpoint) {
                    return Err(Self::reject("invalid stale boundary"));
                }
                sink.mark_volume_entries_stale(&volume_id)?;
                self.stale = true;
            }
            IndexServiceEvent::Entries { entries } => {
                if !self.provider
                    || self.checkpoint
                    || (full && !self.stale)
                    || entries.iter().any(|entry| {
                        entry.volume_id != source.id
                            || entry.source_provider != PROVIDER_WINDOWS_MFT_USN
                    })
                {
                    return Err(Self::reject("unauthorized entries/order"));
                }
                sink.write_batch(&entries)?;
            }
            IndexServiceEvent::EntryStale { entry_id } => {
                if !self.provider || self.checkpoint || !self.owned_entries.remove(&entry_id) {
                    return Err(Self::reject("unproven stale entry identity"));
                }
                sink.mark_entry_stale(&entry_id)?;
            }
            IndexServiceEvent::Checkpoint {
                volume_id,
                journal_id,
                journal_cursor,
            } => {
                if !self.provider
                    || self.checkpoint
                    || (full && !self.stale)
                    || journal_id.as_ref().is_none_or(|value| {
                        value
                            .parse::<u64>()
                            .ok()
                            .is_none_or(|parsed| parsed.to_string() != *value)
                    })
                    || journal_cursor.as_ref().is_none_or(|value| {
                        value
                            .parse::<i64>()
                            .ok()
                            .is_none_or(|parsed| parsed < 0 || parsed.to_string() != *value)
                    })
                {
                    return Err(Self::reject("missing scan completion boundary"));
                }
                if !full {
                    if source
                        .journal_id
                        .as_ref()
                        .is_some_and(|old| Some(old) != journal_id.as_ref())
                    {
                        return Err(Self::reject("incremental journal identity changed"));
                    }
                    if let Some(old) = &source.journal_cursor {
                        let old = old
                            .parse::<i64>()
                            .map_err(|_| Self::reject("invalid saved incremental cursor"))?;
                        if journal_cursor
                            .as_ref()
                            .and_then(|value| value.parse::<i64>().ok())
                            .is_none_or(|cursor| cursor < old)
                        {
                            return Err(Self::reject("incremental checkpoint rewind"));
                        }
                    }
                }
                sink.checkpoint(&volume_id, journal_id.as_deref(), journal_cursor.as_deref())?;
                self.checkpoint = true;
            }
            IndexServiceEvent::SourceState {
                volume_id,
                status,
                error,
            } => {
                if !matches!(
                    status.as_str(),
                    INDEX_STATUS_PAUSED
                        | INDEX_STATUS_PERMISSION_REQUIRED
                        | INDEX_STATUS_REBUILD_REQUIRED
                        | INDEX_STATUS_ERROR
                        | INDEX_STATUS_UNAVAILABLE
                ) {
                    return Err(Self::reject("illegal service source state"));
                }
                sink.set_source_state(&volume_id, &status, error.as_deref())?;
                self.fault = Some(status);
            }
            IndexServiceEvent::ResolveParentPath {
                lookup_id,
                volume_id,
                parent_platform_file_id,
            } => {
                if !self.provider
                    || self.checkpoint
                    || (lookup_id.trim().is_empty() || lookup_id.len() > 128)
                    || !self.lookups.insert(lookup_id.clone())
                {
                    return Err(Self::reject("invalid or replayed lookup"));
                }
                let path = sink.resolve_parent_path(&volume_id, &parent_platform_file_id)?;
                return Ok(Some(IndexServiceLookupResponse::ParentPath {
                    lookup_id,
                    path,
                }));
            }
            IndexServiceEvent::FindEntryByIdentity {
                lookup_id,
                volume_id,
                platform_file_id,
                parent_platform_file_id,
                name,
            } => {
                if !self.provider
                    || self.checkpoint
                    || (lookup_id.trim().is_empty() || lookup_id.len() > 128)
                    || !self.lookups.insert(lookup_id.clone())
                {
                    return Err(Self::reject("invalid or replayed lookup"));
                }
                let entry = sink.find_entry_by_identity(
                    &volume_id,
                    &platform_file_id,
                    &parent_platform_file_id,
                    &name,
                )?;
                if let Some(entry) = &entry {
                    if entry.id.is_empty()
                        || entry.volume_id != volume_id
                        || entry.platform_file_id != platform_file_id
                        || entry.parent_platform_file_id != parent_platform_file_id
                        || entry.name != name
                    {
                        return Err(Self::reject("foreign lookup result identity"));
                    }
                    self.owned_entries.insert(entry.id.clone());
                }
                return Ok(Some(IndexServiceLookupResponse::Entry {
                    lookup_id,
                    entry: Box::new(entry),
                }));
            }
            IndexServiceEvent::Sources { .. } => unreachable!(),
        }
        Ok(None)
    }

    pub(crate) fn finish(
        &self,
        response: &IndexServiceResponse,
    ) -> Result<(), IndexServiceCallError> {
        if response.ok {
            let full = !matches!(
                self.request.command,
                IndexServiceCommand::ResumeIncrementalSync { .. }
            );
            if !self.admitted
                || !self.provider
                || !self.checkpoint
                || (full && !self.stale)
                || (!full && self.stale)
                || self.fault.is_some()
            {
                return Err(Self::reject("success without complete scan evidence"));
            }
        } else if response.error_code.as_deref() == Some("index_service_busy") && self.admitted {
            return Err(Self::reject("busy after admission"));
        } else if let Some(fault) = &self.fault {
            let (_, expected) =
                super::service_errors::classify(&super::service_errors::decode(response));
            if fault != expected {
                return Err(Self::reject("source fault/terminal error mismatch"));
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
mod named_pipe {
    use super::{
        validate_request, IndexServiceEvent, IndexServiceFrame, IndexServiceLookupResponse,
        IndexServiceRequest, IndexServiceResponse, INDEX_SERVICE_PIPE,
    };
    use std::path::PathBuf;
    use std::ptr;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    use std::thread;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, LocalFree, ERROR_MORE_DATA, ERROR_PIPE_BUSY,
        ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{RevertToSelf, SECURITY_ATTRIBUTES, TOKEN_QUERY};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FlushFileBuffers, ReadFile, WriteFile, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
        GetNamedPipeClientSessionId, ImpersonateNamedPipeClient, SetNamedPipeHandleState,
        WaitNamedPipeW, PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE,
        PIPE_WAIT,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, OpenProcess, OpenThreadToken, QueryFullProcessImageNameW,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    const MAX_FRAME_BYTES: usize = 1024 * 1024;
    const PIPE_BUFFER_BYTES: u32 = 64 * 1024;
    const PIPE_CONNECT_TIMEOUT_MS: u32 = 2_000;
    const MAX_PIPE_INSTANCES: u32 = 4;

    pub struct IndexServiceClient {
        pipe: HANDLE,
    }

    impl Drop for IndexServiceClient {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.pipe) };
        }
    }

    impl IndexServiceClient {
        pub fn send_lookup_response(
            &mut self,
            response: IndexServiceLookupResponse,
        ) -> Result<(), String> {
            write_frame(self.pipe, &IndexServiceFrame::LookupResponse { response })
        }

        fn send_request(&mut self, request: &IndexServiceRequest) -> Result<(), String> {
            write_frame(self.pipe, request)
        }

        fn read_frame(&mut self) -> Result<IndexServiceFrame, String> {
            let bytes = read_frame(self.pipe)?;
            serde_json::from_slice(&bytes)
                .map_err(|error| format!("index_service_invalid_frame: {error}"))
        }
    }

    impl super::IndexServiceIo for IndexServiceClient {
        fn send_request(
            &mut self,
            request: &IndexServiceRequest,
        ) -> Result<(), super::IndexServiceCallError> {
            write_frame_typed(self.pipe, request)
        }
        fn read_frame(&mut self) -> Result<IndexServiceFrame, super::IndexServiceCallError> {
            let bytes = read_frame_typed(self.pipe)?;
            serde_json::from_slice(&bytes)
                .map_err(|error| super::IndexServiceCallError::Protocol(error.to_string()))
        }
        fn send_lookup(&mut self, response: IndexServiceLookupResponse) -> Result<(), String> {
            self.send_lookup_response(response)
        }
    }

    pub(crate) fn call_admitted<F>(
        request: &IndexServiceRequest,
        event: F,
    ) -> Result<IndexServiceResponse, super::IndexServiceCallError>
    where
        F: FnMut(
            IndexServiceEvent,
        )
            -> Result<Option<IndexServiceLookupResponse>, super::IndexServiceCallError>,
    {
        validate_request(request).map_err(super::IndexServiceCallError::Protocol)?;
        let mut client = connect().map_err(super::IndexServiceCallError::NotSent)?;
        super::exchange(request, &mut client, event)
    }

    pub fn call(request: &IndexServiceRequest) -> Result<IndexServiceResponse, String> {
        call_admitted(request, |_| {
            Err(super::IndexServiceCallError::Protocol(
                "unexpected control event".into(),
            ))
        })
        .map_err(|error| error.to_string())
    }

    pub fn call_stream<F>(
        request: &IndexServiceRequest,
        mut on_event: F,
    ) -> Result<IndexServiceResponse, String>
    where
        F: FnMut(&mut IndexServiceClient, IndexServiceEvent) -> Result<(), String>,
    {
        validate_request(request)?;
        let mut client = connect()?;
        client.send_request(request)?;
        loop {
            match client.read_frame()? {
                IndexServiceFrame::Response { response } => {
                    if response.protocol_version != super::IPC_PROTOCOL_VERSION
                        || response.request_id != request.request_id
                        || (response.ok && response.error_code.is_some())
                        || (!response.ok
                            && !response
                                .error_code
                                .as_deref()
                                .is_some_and(super::super::service_errors::known_code))
                    {
                        return Err("index_service_invalid_response".into());
                    }
                    return Ok(response);
                }
                IndexServiceFrame::Event { request_id, event } => {
                    if request_id != request.request_id {
                        return Err("index_service_event_identity_mismatch".into());
                    }
                    on_event(&mut client, event)?;
                }
                IndexServiceFrame::LookupResponse { .. } => {
                    return Err("index_service_unexpected_lookup_response".to_string());
                }
            }
        }
    }

    pub fn wake_service_pipe() {
        let request = IndexServiceRequest::new(super::IndexServiceCommand::Status, None);
        let _ = call(&request);
    }

    pub fn serve_once(
        handler: impl FnOnce(IndexServiceRequest) -> IndexServiceResponse,
    ) -> Result<(), String> {
        let (pipe, security_descriptor) = create_server_pipe()?;
        accept_and_handle(pipe, security_descriptor, |request, connection| {
            let response = handler(request);
            connection.send_response(response)
        })
    }

    pub type ServiceRequestHandler = dyn Fn(IndexServiceRequest, &mut IndexServiceServerConnection) -> Result<(), String>
        + Send
        + Sync
        + 'static;

    pub struct IndexServiceServerConnection {
        pipe: HANDLE,
        request_id: String,
    }

    unsafe impl Send for IndexServiceServerConnection {}

    impl IndexServiceServerConnection {
        pub fn send_event(&mut self, event: IndexServiceEvent) -> Result<(), String> {
            write_frame(
                self.pipe,
                &IndexServiceFrame::Event {
                    request_id: self.request_id.clone(),
                    event,
                },
            )
        }

        pub fn send_response(&mut self, response: IndexServiceResponse) -> Result<(), String> {
            write_frame(self.pipe, &IndexServiceFrame::Response { response })
        }

        pub fn read_lookup_response(
            &mut self,
            expected_lookup_id: &str,
        ) -> Result<IndexServiceLookupResponse, String> {
            let frame = read_frame(self.pipe)?;
            let frame: IndexServiceFrame = serde_json::from_slice(&frame)
                .map_err(|error| format!("index_service_invalid_lookup_frame: {error}"))?;
            let IndexServiceFrame::LookupResponse { response } = frame else {
                return Err("index_service_expected_lookup_response".to_string());
            };
            let lookup_id = match &response {
                IndexServiceLookupResponse::ParentPath { lookup_id, .. }
                | IndexServiceLookupResponse::Entry { lookup_id, .. } => lookup_id,
            };
            if lookup_id != expected_lookup_id {
                return Err("index_service_lookup_id_mismatch".to_string());
            }
            Ok(response)
        }
    }

    impl super::IndexServiceConnection for IndexServiceServerConnection {
        fn send_event(&mut self, event: IndexServiceEvent) -> Result<(), String> {
            IndexServiceServerConnection::send_event(self, event)
        }
        fn send_response(&mut self, response: IndexServiceResponse) -> Result<(), String> {
            IndexServiceServerConnection::send_response(self, response)
        }
        fn read_lookup_response(&mut self, id: &str) -> Result<IndexServiceLookupResponse, String> {
            IndexServiceServerConnection::read_lookup_response(self, id)
        }
    }

    pub fn serve_loop(
        stop: Arc<AtomicBool>,
        handler: Arc<ServiceRequestHandler>,
    ) -> Result<(), String> {
        let active = Arc::new(AtomicUsize::new(0));
        while !stop.load(Ordering::Acquire) {
            let (pipe, security_descriptor) = create_server_pipe()?;
            let connection_result = unsafe { ConnectNamedPipe(pipe, ptr::null_mut()) };
            if connection_result == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
                cleanup_pipe(pipe, security_descriptor);
                if stop.load(Ordering::Acquire) {
                    break;
                }
                return Err(format!("index_service_pipe_connect_failed: {}", unsafe {
                    GetLastError()
                }));
            }
            let stop_for_connection = stop.clone();
            let handler = handler.clone();
            let active_for_connection = active.clone();
            let pipe_value = pipe as usize;
            let descriptor_value = security_descriptor as usize;
            active.fetch_add(1, Ordering::AcqRel);
            thread::spawn(move || {
                let _ = handle_connected_pipe(
                    pipe_value as HANDLE,
                    descriptor_value as *mut core::ffi::c_void,
                    stop_for_connection,
                    handler,
                );
                active_for_connection.fetch_sub(1, Ordering::AcqRel);
            });
        }
        for _ in 0..250 {
            if active.load(Ordering::Acquire) == 0 {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(20));
        }
        Ok(())
    }

    fn connect() -> Result<IndexServiceClient, String> {
        let pipe_name = super::super::volumes::to_wide(INDEX_SERVICE_PIPE);
        let waited = unsafe { WaitNamedPipeW(pipe_name.as_ptr(), PIPE_CONNECT_TIMEOUT_MS) };
        if waited == 0 {
            let error = unsafe { GetLastError() };
            if error != ERROR_PIPE_BUSY {
                return Err(format!("index_service_pipe_wait_failed: {error}"));
            }
            return Err("index_service_pipe_busy".to_string());
        }
        let pipe = unsafe {
            CreateFileW(
                pipe_name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                ptr::null(),
                OPEN_EXISTING,
                0,
                ptr::null_mut(),
            )
        };
        if pipe == INVALID_HANDLE_VALUE {
            return Err(format!("index_service_pipe_open_failed: {}", unsafe {
                GetLastError()
            }));
        }
        let mode = PIPE_READMODE_MESSAGE;
        if unsafe { SetNamedPipeHandleState(pipe, &mode, ptr::null(), ptr::null()) } == 0 {
            let error = unsafe { GetLastError() };
            unsafe { CloseHandle(pipe) };
            return Err(format!("index_service_pipe_mode_failed: {error}"));
        }
        Ok(IndexServiceClient { pipe })
    }

    fn handle_connected_pipe(
        pipe: HANDLE,
        security_descriptor: *mut core::ffi::c_void,
        stop: Arc<AtomicBool>,
        handler: Arc<ServiceRequestHandler>,
    ) -> Result<(), String> {
        let mut request_id = "unknown".to_string();
        let result = (|| {
            let bytes = read_frame(pipe)?;
            validate_pipe_client(pipe)?;
            let request: IndexServiceRequest = serde_json::from_slice(&bytes)
                .map_err(|error| format!("index_service_invalid_request: {error}"))?;
            validate_request(&request)?;
            request_id = request.request_id.clone();
            let mut connection = IndexServiceServerConnection {
                pipe,
                request_id: request_id.clone(),
            };
            handler(request, &mut connection)
        })();
        if result.is_err() && !stop.load(Ordering::Acquire) {
            let response = IndexServiceResponse {
                protocol_version: super::IPC_PROTOCOL_VERSION,
                request_id,
                ok: false,
                error_code: Some("index_service_request_failed".to_string()),
                message: result.as_ref().err().cloned(),
                status: None,
            };
            let _ = write_frame(pipe, &IndexServiceFrame::Response { response });
        }
        unsafe { FlushFileBuffers(pipe) };
        cleanup_pipe(pipe, security_descriptor);
        result
    }

    fn accept_and_handle(
        pipe: HANDLE,
        security_descriptor: *mut core::ffi::c_void,
        handler: impl FnOnce(
            IndexServiceRequest,
            &mut IndexServiceServerConnection,
        ) -> Result<(), String>,
    ) -> Result<(), String> {
        let connection_result = unsafe { ConnectNamedPipe(pipe, ptr::null_mut()) };
        if connection_result == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
            cleanup_pipe(pipe, security_descriptor);
            return Err(format!("index_service_pipe_connect_failed: {}", unsafe {
                GetLastError()
            }));
        }
        let result = (|| {
            let bytes = read_frame(pipe)?;
            validate_pipe_client(pipe)?;
            let request: IndexServiceRequest = serde_json::from_slice(&bytes)
                .map_err(|error| format!("index_service_invalid_request: {error}"))?;
            validate_request(&request)?;
            let request_id = request.request_id.clone();
            let mut connection = IndexServiceServerConnection { pipe, request_id };
            handler(request, &mut connection)
        })();
        unsafe { FlushFileBuffers(pipe) };
        cleanup_pipe(pipe, security_descriptor);
        result
    }

    fn validate_pipe_client(pipe: HANDLE) -> Result<(), String> {
        if unsafe { ImpersonateNamedPipeClient(pipe) } == 0 {
            return Err(format!(
                "index_service_client_impersonation_failed: {}",
                unsafe { GetLastError() }
            ));
        }
        let mut token = ptr::null_mut();
        let token_opened =
            unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 0, &mut token) } != 0;
        let token_error = (!token_opened).then(|| unsafe { GetLastError() });
        if token_opened && !token.is_null() {
            unsafe { CloseHandle(token) };
        }
        let revert_error = if unsafe { RevertToSelf() } == 0 {
            Some(unsafe { GetLastError() })
        } else {
            None
        };
        if let Some(error) = revert_error {
            return Err(format!("index_service_client_revert_failed: {error}"));
        }
        if let Some(error) = token_error {
            return Err(format!("index_service_client_token_failed: {error}"));
        }

        let mut session_id = 0u32;
        if unsafe { GetNamedPipeClientSessionId(pipe, &mut session_id) } == 0 {
            return Err(format!("index_service_client_session_failed: {}", unsafe {
                GetLastError()
            }));
        }
        if session_id == 0 {
            return Err("index_service_client_not_interactive".to_string());
        }

        let mut process_id = 0u32;
        if unsafe { GetNamedPipeClientProcessId(pipe, &mut process_id) } == 0 || process_id == 0 {
            return Err(format!("index_service_client_process_failed: {}", unsafe {
                GetLastError()
            }));
        }
        let client_path = process_image_path(process_id)?;
        let current_path = std::env::current_exe()
            .map_err(|error| format!("index_service_current_exe_failed: {error}"))?;
        if normalize_windows_path(&client_path) != normalize_windows_path(&current_path) {
            return Err("index_service_client_executable_mismatch".to_string());
        }
        Ok(())
    }

    fn process_image_path(process_id: u32) -> Result<PathBuf, String> {
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
        if process.is_null() {
            return Err(format!(
                "index_service_client_process_open_failed: {}",
                unsafe { GetLastError() }
            ));
        }
        let mut buffer = vec![0u16; 32_768];
        let mut size = buffer.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut size) };
        let error = if ok == 0 {
            Some(unsafe { GetLastError() })
        } else {
            None
        };
        unsafe { CloseHandle(process) };
        if let Some(error) = error {
            return Err(format!("index_service_client_image_failed: {error}"));
        }
        buffer.truncate(size as usize);
        Ok(PathBuf::from(String::from_utf16_lossy(&buffer)))
    }

    fn normalize_windows_path(path: &std::path::Path) -> String {
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_start_matches(r"\\?\")
            .to_ascii_lowercase()
    }

    fn create_server_pipe() -> Result<(HANDLE, *mut core::ffi::c_void), String> {
        let name = super::super::volumes::to_wide(INDEX_SERVICE_PIPE);
        let sddl = super::super::volumes::to_wide(
            // LocalSystem owns the service. Built-in administrators and the
            // interactive user receive only generic read/write; the executable
            // and session checks above remain authoritative.
            "D:P(A;;GA;;;SY)(A;;GRGW;;;BA)(A;;GRGW;;;IU)",
        );
        let mut descriptor = ptr::null_mut();
        let mut descriptor_size = 0u32;
        let descriptor_ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                &mut descriptor_size,
            )
        } != 0;
        if !descriptor_ok {
            return Err(format!(
                "index_service_security_descriptor_failed: {}",
                unsafe { GetLastError() }
            ));
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let pipe = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                MAX_PIPE_INSTANCES,
                PIPE_BUFFER_BYTES,
                PIPE_BUFFER_BYTES,
                1000,
                &attributes,
            )
        };
        if pipe == INVALID_HANDLE_VALUE {
            unsafe { LocalFree(descriptor) };
            return Err(format!("index_service_pipe_create_failed: {}", unsafe {
                GetLastError()
            }));
        }
        Ok((pipe, descriptor))
    }

    fn cleanup_pipe(pipe: HANDLE, security_descriptor: *mut core::ffi::c_void) {
        unsafe {
            DisconnectNamedPipe(pipe);
            CloseHandle(pipe);
            if !security_descriptor.is_null() {
                LocalFree(security_descriptor);
            }
        }
    }

    fn write_frame<T: serde::Serialize>(pipe: HANDLE, value: &T) -> Result<(), String> {
        write_frame_typed(pipe, value).map_err(|error| error.to_string())
    }

    fn write_frame_typed<T: serde::Serialize>(
        pipe: HANDLE,
        value: &T,
    ) -> Result<(), super::IndexServiceCallError> {
        let mut frame = serde_json::to_vec(value).map_err(|error| {
            super::IndexServiceCallError::Protocol(format!("index_service_encode_failed: {error}"))
        })?;
        frame.push(b'\n');
        if frame.is_empty() || frame.len() > MAX_FRAME_BYTES {
            return Err(super::IndexServiceCallError::Protocol(
                "index_service_frame_too_large".into(),
            ));
        }
        let mut written = 0u32;
        let ok = unsafe {
            WriteFile(
                pipe,
                frame.as_ptr(),
                frame.len() as u32,
                &mut written,
                ptr::null_mut(),
            )
        };
        if ok == 0 || written as usize != frame.len() {
            return Err(super::IndexServiceCallError::UnknownOutcome(format!(
                "index_service_pipe_write_failed: {}",
                unsafe { GetLastError() }
            )));
        }
        Ok(())
    }

    fn read_frame(pipe: HANDLE) -> Result<Vec<u8>, String> {
        read_frame_typed(pipe).map_err(|error| error.to_string())
    }

    fn read_frame_typed(pipe: HANDLE) -> Result<Vec<u8>, super::IndexServiceCallError> {
        let mut frame = Vec::with_capacity(PIPE_BUFFER_BYTES as usize);
        loop {
            if frame.len() >= MAX_FRAME_BYTES {
                return Err(super::IndexServiceCallError::Protocol(
                    "index_service_invalid_frame_size".into(),
                ));
            }
            let remaining = MAX_FRAME_BYTES - frame.len();
            let chunk_len = remaining.min(PIPE_BUFFER_BYTES as usize);
            let start = frame.len();
            frame.resize(start + chunk_len, 0);
            let mut read = 0u32;
            let ok = unsafe {
                ReadFile(
                    pipe,
                    frame[start..].as_mut_ptr(),
                    chunk_len as u32,
                    &mut read,
                    ptr::null_mut(),
                )
            };
            frame.truncate(start + read as usize);
            if ok != 0 {
                break;
            }
            let error = unsafe { GetLastError() };
            if error != ERROR_MORE_DATA {
                return Err(super::IndexServiceCallError::UnknownOutcome(format!(
                    "index_service_pipe_read_failed: {error}"
                )));
            }
        }
        if frame.last() == Some(&b'\n') {
            frame.pop();
        }
        if frame.is_empty() || frame.len() > MAX_FRAME_BYTES {
            return Err(super::IndexServiceCallError::Protocol(
                "index_service_invalid_frame_size".into(),
            ));
        }
        Ok(frame)
    }
}

#[cfg(windows)]
pub(crate) use named_pipe::call_admitted as call_index_service_admitted;

#[cfg(windows)]
pub use named_pipe::{
    call as call_index_service, call_stream as call_index_service_stream,
    serve_loop as serve_index_service_loop, serve_once as serve_index_service_once,
    wake_service_pipe, IndexServiceClient, IndexServiceServerConnection, ServiceRequestHandler,
};

#[cfg(test)]
#[path = "service_p1_tests.rs"]
pub(crate) mod p1_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_protocol_and_shutdown_over_ipc() {
        let request = IndexServiceRequest {
            protocol_version: IPC_PROTOCOL_VERSION + 1,
            request_id: "request".to_string(),
            command: IndexServiceCommand::Status,
            source: None,
        };
        assert_eq!(
            validate_request(&request),
            Err("unsupported_index_service_protocol".to_string())
        );

        let shutdown = IndexServiceRequest::new(IndexServiceCommand::Shutdown, None);
        assert_eq!(
            validate_request(&shutdown),
            Err("index_service_shutdown_via_scm_only".to_string())
        );
    }

    #[test]
    fn index_commands_require_a_bounded_source_snapshot() {
        let request = IndexServiceRequest::new(
            IndexServiceCommand::Rebuild {
                source_id: "volume".to_string(),
            },
            None,
        );
        assert_eq!(
            validate_request(&request),
            Err("index_service_source_snapshot_required".to_string())
        );
    }

    #[test]
    fn v4_service_protocol_and_event_frame_shape_remain_stable() {
        assert_eq!(IPC_PROTOCOL_VERSION, 4);
        assert_eq!(INDEX_SERVICE_PIPE, r"\\.\pipe\ZenCanvas.GlobalIndex.v4");

        let request = IndexServiceRequest {
            protocol_version: IPC_PROTOCOL_VERSION,
            request_id: "request-1".to_string(),
            command: IndexServiceCommand::Status,
            source: None,
        };
        assert_eq!(
            serde_json::to_value(request).expect("serialize v3 request"),
            serde_json::json!({
                "protocol_version": 4,
                "request_id": "request-1",
                "command": "status",
                "source": null,
            })
        );

        let frame = IndexServiceFrame::Event {
            request_id: "request-1".to_string(),
            event: IndexServiceEvent::Checkpoint {
                volume_id: "volume-1".to_string(),
                journal_id: Some("journal-1".to_string()),
                journal_cursor: Some("cursor-1".to_string()),
            },
        };
        assert_eq!(
            serde_json::to_value(frame).expect("serialize v3 event frame"),
            serde_json::json!({
                "kind": "event",
                "request_id": "request-1",
                "event": {
                    "event": "checkpoint",
                    "volume_id": "volume-1",
                    "journal_id": "journal-1",
                    "journal_cursor": "cursor-1",
                },
            })
        );
    }
}
