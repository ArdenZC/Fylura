//! Session-scoped, non-durable Browse enumeration.
//!
//! This module deliberately accepts only a backend-resolved directory and
//! publishes only the W1-01 opaque workspace references. It does not admit
//! scan roots, write managed state, or authorize reads or mutations.

#![allow(dead_code)]

use super::{
    BrowseEntryRef, BrowseEnumerationRef, BrowsePathRef, LocationRef, MaterializationState,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs::{self, DirEntry, ReadDir};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use thiserror::Error;
use uuid::Uuid;

const MAX_ID_LENGTH: usize = 256;
const MAX_QUERY_TEXT_LENGTH: usize = 4096;
/// Maximum number of raw directory entries inspected by one backend page call.
///
/// This is deliberately independent from the number of matching entries
/// requested by the caller. A query may therefore publish an empty partial
/// page and resume from its live cursor on a later call.
pub(crate) const RAW_DIRECTORY_SCAN_BUDGET: usize = 1024;

// W1-11 measured the legacy 4,096-entry/1,024-path working set against real
// 100k local fixtures before changing these values: the entry-heavy shape
// stopped at 4,096 entries, while the path-heavy shape stopped at 769 live
// paths because the next 256-entry page could not be reserved atomically.
// W1-10 keeps every published page valid until supersede, target teardown or
// session disposal, so eviction would invalidate frontend-owned refs and
// history pins. These are deliberately fixed per-session bounds for the
// representative 90k-file/10k-directory 100k workload, not an unbounded cache.
// The process-wide limits intentionally provide only a second bounded working
// set. They prevent max_sessions from multiplying the measured single-session
// RSS cost into an unbounded process-level reservation without evicting any
// frontend-owned EntryRef or W1-10 history PathRef.
pub(crate) const DEFAULT_MAX_BROWSE_PATH_REFS: usize = 16_384;
pub(crate) const DEFAULT_MAX_BROWSE_ENTRY_REFS: usize = 100_000;
pub(crate) const DEFAULT_MAX_BROWSE_PROCESS_PATH_REFS: usize = 32_768;
pub(crate) const DEFAULT_MAX_BROWSE_PROCESS_ENTRY_REFS: usize = 200_000;

#[derive(Debug, Clone)]
pub(crate) struct BackendResolvedDirectory {
    path: PathBuf,
}

impl BackendResolvedDirectory {
    pub(crate) fn from_backend_path(path: PathBuf) -> Result<Self, BrowseError> {
        validate_directory_path(&path)?;
        Ok(Self { path })
    }

    pub(crate) fn as_path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BrowseLimits {
    pub(crate) max_sessions: usize,
    pub(crate) max_page_size: usize,
    pub(crate) max_path_refs: usize,
    pub(crate) max_entry_refs: usize,
    pub(crate) max_process_path_refs: usize,
    pub(crate) max_process_entry_refs: usize,
}

impl Default for BrowseLimits {
    fn default() -> Self {
        Self {
            max_sessions: 32,
            max_page_size: 256,
            max_path_refs: DEFAULT_MAX_BROWSE_PATH_REFS,
            max_entry_refs: DEFAULT_MAX_BROWSE_ENTRY_REFS,
            max_process_path_refs: DEFAULT_MAX_BROWSE_PROCESS_PATH_REFS,
            max_process_entry_refs: DEFAULT_MAX_BROWSE_PROCESS_ENTRY_REFS,
        }
    }
}

impl BrowseLimits {
    fn validate(self) -> Result<Self, BrowseError> {
        if self.max_sessions == 0
            || self.max_page_size == 0
            || self.max_path_refs == 0
            || self.max_entry_refs < self.max_page_size
            || self.max_process_path_refs < self.max_path_refs
            || self.max_process_entry_refs < self.max_entry_refs
        {
            return Err(BrowseError::InvalidLimits);
        }
        Ok(self)
    }

    fn page_size(self, requested: usize) -> Result<usize, BrowseError> {
        if requested == 0 {
            return Err(BrowseError::InvalidPageSize);
        }
        Ok(requested.min(self.max_page_size))
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowseError {
    #[error("browse_session_not_found")]
    SessionNotFound,
    #[error("browse_session_capacity_exceeded")]
    SessionCapacityExceeded,
    #[error("browse_location_ref_invalid")]
    InvalidLocationRef,
    #[error("browse_path_ref_invalid")]
    InvalidPathRef,
    #[error("browse_entry_ref_invalid")]
    InvalidEntryRef,
    #[error("browse_target_not_directory")]
    TargetNotDirectory,
    #[error("browse_directory_permission_denied")]
    DirectoryPermissionDenied,
    #[error("browse_directory_not_found")]
    DirectoryNotFound,
    #[error("browse_directory_unavailable")]
    DirectoryUnavailable,
    #[error("browse_entry_permission_denied")]
    EntryPermissionDenied,
    #[error("browse_entry_not_found")]
    EntryNotFound,
    #[error("browse_entry_unavailable")]
    EntryUnavailable,
    #[error("browse_unsupported_entry")]
    UnsupportedEntry,
    #[error("browse_cursor_invalid")]
    InvalidCursor,
    #[error("browse_enumeration_stale")]
    StaleEnumeration,
    #[error("browse_publication_stale")]
    StalePublication,
    #[error("browse_cancelled")]
    Cancelled,
    #[error("browse_request_invalid")]
    InvalidRequest,
    #[error("browse_sort_capacity_exceeded")]
    SortCapacityExceeded,
    #[error("browse_page_size_invalid")]
    InvalidPageSize,
    #[error("browse_limits_invalid")]
    InvalidLimits,
    #[error("browse_temporary_state_unavailable")]
    StateUnavailable,
    #[error("browse_temporary_state_capacity_exceeded")]
    TemporaryStateCapacityExceeded,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BrowseCompletion {
    Partial,
    Complete,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BrowseEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowseQueryEntryKind {
    #[default]
    All,
    File,
    Directory,
}

/// Sorting is an opt-in bounded metadata snapshot, never a page-local reorder.
pub(crate) const MAX_SORT_ENTRIES: usize = 20_000;
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowseSortField {
    Name,
    Type,
    Size,
    Modified,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowseSortDirection {
    Asc,
    Desc,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowseSortSpec {
    pub(crate) field: BrowseSortField,
    pub(crate) direction: BrowseSortDirection,
    pub(crate) folders_first: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowseQuerySpecV1 {
    #[serde(default)]
    pub(crate) text: Option<String>,
    #[serde(default)]
    pub(crate) entry_kind: BrowseQueryEntryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sort: Option<BrowseSortSpec>,
}

impl Default for BrowseQuerySpecV1 {
    fn default() -> Self {
        Self {
            text: None,
            entry_kind: BrowseQueryEntryKind::All,
            sort: None,
        }
    }
}

impl BrowseQuerySpecV1 {
    fn normalized(self) -> Result<Self, BrowseError> {
        let text = self
            .text
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());
        if text
            .as_ref()
            .is_some_and(|value| value.chars().count() > MAX_QUERY_TEXT_LENGTH)
        {
            return Err(BrowseError::InvalidRequest);
        }
        Ok(Self {
            text,
            entry_kind: self.entry_kind,
            sort: self.sort,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EphemeralBrowseEntry {
    #[serde(rename = "ref")]
    pub(crate) entry_ref: BrowseEntryRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path_ref: Option<BrowsePathRef>,
    pub(crate) name: String,
    /// Presentation only. This value is never accepted as a resolver input.
    pub(crate) display_path: String,
    pub(crate) kind: BrowseEntryKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) extension: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) modified_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) created_at: Option<i64>,
    pub(crate) materialization: MaterializationState,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowsePage {
    pub(crate) session_id: String,
    pub(crate) request_id: String,
    pub(crate) enumeration_id: String,
    pub(crate) entries: Vec<EphemeralBrowseEntry>,
    pub(crate) next_cursor: Option<String>,
    pub(crate) completion: BrowseCompletion,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) known_count: Option<usize>,
    /// Cumulative omitted children: never interpret completion as full coverage.
    #[serde(default)]
    pub(crate) skipped_entries: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowseSessionInfo {
    pub(crate) session_id: String,
    pub(crate) location: LocationRef,
    pub(crate) root_path_ref: BrowsePathRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedBrowseEntry {
    pub(crate) path: PathBuf,
    pub(crate) kind: BrowseEntryKind,
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct TestPublishGate {
    state: Mutex<TestPublishGateState>,
    wake: std::sync::Condvar,
}

#[cfg(test)]
#[derive(Debug, Default)]
struct TestPublishGateState {
    reached: bool,
    release: bool,
}

#[cfg(test)]
impl TestPublishGate {
    pub(crate) fn pause(&self) {
        let mut state = self.state.lock().expect("test publish gate lock");
        state.reached = true;
        self.wake.notify_all();
        while !state.release {
            state = self.wake.wait(state).expect("test publish gate wait");
        }
    }

    pub(crate) fn wait_until_reached(&self) {
        let mut state = self.state.lock().expect("test publish gate lock");
        while !state.reached {
            state = self.wake.wait(state).expect("test publish gate wait");
        }
    }

    pub(crate) fn release(&self) {
        let mut state = self.state.lock().expect("test publish gate lock");
        state.release = true;
        self.wake.notify_all();
    }
}

#[derive(Debug)]
pub(crate) struct BrowseService {
    sessions: Mutex<HashMap<String, BrowseSessionState>>,
    limits: BrowseLimits,
    #[cfg(test)]
    test_publish_gate: Mutex<Option<Arc<TestPublishGate>>>,
}

impl Default for BrowseService {
    fn default() -> Self {
        Self::new(BrowseLimits::default()).expect("default BrowseLimits are valid")
    }
}

impl BrowseService {
    pub(crate) fn new(limits: BrowseLimits) -> Result<Self, BrowseError> {
        Ok(Self {
            sessions: Mutex::new(HashMap::new()),
            limits: limits.validate()?,
            #[cfg(test)]
            test_publish_gate: Mutex::new(None),
        })
    }

    pub(crate) fn start_session(
        &self,
        directory: BackendResolvedDirectory,
    ) -> Result<BrowseSessionInfo, BrowseError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        self.start_session_locked(&mut sessions, directory)
    }

    /// Re-admit the backend-owned root of a live ephemeral Location into a
    /// completely new Browse session. The source LocationRef is checked
    /// against BrowseService state; no renderer presentation field or path is
    /// accepted here.
    pub(crate) fn re_admit_ephemeral_location(
        &self,
        location: &LocationRef,
    ) -> Result<BrowseSessionInfo, BrowseError> {
        let LocationRef::Ephemeral {
            browse_session_id,
            location_id,
        } = location
        else {
            return Err(BrowseError::InvalidLocationRef);
        };
        if browse_session_id.is_empty() || location_id.is_empty() {
            return Err(BrowseError::InvalidLocationRef);
        }

        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        let source = sessions
            .get(browse_session_id)
            .ok_or(BrowseError::SessionNotFound)?;
        if source.location_id.as_str() != location_id.as_str() {
            return Err(BrowseError::InvalidLocationRef);
        }
        let path = source
            .paths
            .get(&source.root_path_ref.id)
            .ok_or(BrowseError::InvalidPathRef)?
            .path
            .clone();
        validate_directory_path(&path)?;
        self.start_session_locked(&mut sessions, BackendResolvedDirectory { path })
    }

    fn start_session_locked(
        &self,
        sessions: &mut HashMap<String, BrowseSessionState>,
        directory: BackendResolvedDirectory,
    ) -> Result<BrowseSessionInfo, BrowseError> {
        let session_id = opaque_id();
        let location_id = opaque_id();
        let root_path_ref = BrowsePathRef { id: opaque_id() };
        let location = LocationRef::Ephemeral {
            browse_session_id: session_id.clone(),
            location_id: location_id.clone(),
        };

        if sessions.len() >= self.limits.max_sessions {
            return Err(BrowseError::SessionCapacityExceeded);
        }
        let live_path_refs = sessions
            .values()
            .map(|session| session.paths.len())
            .sum::<usize>();
        if live_path_refs.saturating_add(1) > self.limits.max_process_path_refs {
            return Err(BrowseError::TemporaryStateCapacityExceeded);
        }

        let mut paths = HashMap::new();
        paths.insert(
            root_path_ref.id.clone(),
            StoredPath {
                path: directory.path,
                pinned: true,
            },
        );
        sessions.insert(
            session_id.clone(),
            BrowseSessionState {
                location_id,
                root_path_ref: root_path_ref.clone(),
                paths,
                entries: HashMap::new(),
                active: None,
            },
        );

        Ok(BrowseSessionInfo {
            session_id,
            location,
            root_path_ref,
        })
    }

    /// Resolve an existing Browse path reference for a disposable ephemeral
    /// watcher. This deliberately does not change the path reference pin so
    /// the existing Browse owner remains responsible for its lifecycle.
    pub(crate) fn resolve_watch_target(
        &self,
        session_id: &str,
        path_ref: &BrowsePathRef,
    ) -> Result<BackendResolvedDirectory, BrowseError> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        let session = sessions
            .get(session_id)
            .ok_or(BrowseError::SessionNotFound)?;
        let path = session
            .paths
            .get(&path_ref.id)
            .ok_or(BrowseError::InvalidPathRef)?
            .path
            .clone();
        validate_directory_path(&path)?;
        Ok(BackendResolvedDirectory { path })
    }

    pub(crate) fn start_enumeration(
        &self,
        session_id: &str,
        request_id: impl Into<String>,
        path_ref: &BrowsePathRef,
        page_size: usize,
    ) -> Result<BrowsePage, BrowseError> {
        self.start_enumeration_with_query(
            session_id,
            request_id,
            path_ref,
            page_size,
            BrowseQuerySpecV1::default(),
        )
    }

    pub(crate) fn start_enumeration_with_query(
        &self,
        session_id: &str,
        request_id: impl Into<String>,
        path_ref: &BrowsePathRef,
        page_size: usize,
        query: BrowseQuerySpecV1,
    ) -> Result<BrowsePage, BrowseError> {
        #[cfg(all(test, feature = "performance-test-tauri"))]
        let observation = stage_trace::Call::begin();
        let request_id = request_id.into();
        self.validate_request_id(&request_id)?;
        let page_size = self.limits.page_size(page_size)?;
        let query = query.normalized()?;

        #[cfg(all(test, feature = "performance-test-tauri"))]
        stage_trace::mark(1);
        let enumeration = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| BrowseError::StateUnavailable)?;
            #[cfg(all(test, feature = "performance-test-tauri"))]
            stage_trace::mark(2);
            let session = sessions
                .get_mut(session_id)
                .ok_or(BrowseError::SessionNotFound)?;
            let stored_path = session
                .paths
                .get_mut(&path_ref.id)
                .ok_or(BrowseError::InvalidPathRef)?;
            stored_path.pinned = true;
            let path = stored_path.path.clone();
            validate_directory_path(&path)?;
            let source = fs::read_dir(&path).map_err(map_directory_error)?;

            #[cfg(all(test, feature = "performance-test-tauri"))]
            stage_trace::mark(3);
            if let Some(previous) = session.active.take() {
                invalidate_entries_for_enumeration(session, &previous.identity.enumeration_id);
                previous.cancel(CancelReason::Superseded);
            }

            #[cfg(all(test, feature = "performance-test-tauri"))]
            stage_trace::mark(4);
            let enumeration = Arc::new(EnumerationState::new_with_query(
                BrowseEnumerationRef {
                    session_id: session_id.to_string(),
                    request_id,
                    enumeration_id: opaque_id(),
                },
                source,
                query,
            ));
            session.active = Some(Arc::clone(&enumeration));
            enumeration
        };

        #[cfg(all(test, feature = "performance-test-tauri"))]
        stage_trace::mark(5);
        let result = self.read_page(session_id, enumeration, None, page_size);
        #[cfg(all(test, feature = "performance-test-tauri"))]
        if let Ok(page) = &result {
            observation.complete(page.entries.len());
        }
        result
    }

    pub(crate) fn next_page(
        &self,
        session_id: &str,
        cursor: &str,
        page_size: usize,
    ) -> Result<BrowsePage, BrowseError> {
        if cursor.is_empty() || cursor.len() > MAX_ID_LENGTH {
            return Err(BrowseError::InvalidCursor);
        }
        let page_size = self.limits.page_size(page_size)?;
        let enumeration = {
            let sessions = self
                .sessions
                .lock()
                .map_err(|_| BrowseError::StateUnavailable)?;
            let session = sessions
                .get(session_id)
                .ok_or(BrowseError::SessionNotFound)?;
            session
                .active
                .clone()
                .ok_or(BrowseError::StaleEnumeration)?
        };
        self.read_page(session_id, enumeration, Some(cursor), page_size)
    }

    pub(crate) fn cancel(
        &self,
        session_id: &str,
        identity: &BrowseEnumerationRef,
    ) -> Result<(), BrowseError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        let session = sessions
            .get_mut(session_id)
            .ok_or(BrowseError::SessionNotFound)?;
        let enumeration = session
            .active
            .clone()
            .ok_or(BrowseError::StaleEnumeration)?;
        if enumeration.identity != *identity {
            return Err(BrowseError::StaleEnumeration);
        }
        enumeration.cancel(CancelReason::Explicit);
        invalidate_entries_for_enumeration(session, &enumeration.identity.enumeration_id);
        session.active = None;
        Ok(())
    }

    /// Cancel the active enumeration by its caller-owned request id. This is
    /// the cancellation seam for a start request whose opaque enumeration ref
    /// has not been published yet; BrowseService still validates the active
    /// identity and owns the cancellation token.
    pub(crate) fn cancel_request(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Result<(), BrowseError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        let session = sessions
            .get_mut(session_id)
            .ok_or(BrowseError::SessionNotFound)?;
        let enumeration = session
            .active
            .clone()
            .filter(|enumeration| enumeration.identity.request_id == request_id)
            .ok_or(BrowseError::StaleEnumeration)?;
        enumeration.cancel(CancelReason::Explicit);
        invalidate_entries_for_enumeration(session, &enumeration.identity.enumeration_id);
        session.active = None;
        Ok(())
    }

    pub(crate) fn invalidate(&self, session_id: &str) -> Result<(), BrowseError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        let session = sessions
            .get_mut(session_id)
            .ok_or(BrowseError::SessionNotFound)?;
        if let Some(enumeration) = session.active.take() {
            invalidate_entries_for_enumeration(session, &enumeration.identity.enumeration_id);
            enumeration.cancel(CancelReason::Invalidated);
        }
        Ok(())
    }

    /// Releases refs owned by one published page. A directory path ref that
    /// has been promoted by navigation remains pinned until `release_path_ref`.
    pub(crate) fn release_page(&self, page: &BrowsePage) -> Result<(), BrowseError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        let session = sessions
            .get_mut(&page.session_id)
            .ok_or(BrowseError::SessionNotFound)?;

        for entry in &page.entries {
            let BrowseEntryRef::Ephemeral {
                browse_session_id,
                entry_id,
            } = &entry.entry_ref;
            if browse_session_id != &page.session_id {
                continue;
            }
            let stored = session.entries.get(entry_id);
            let owned = stored.is_some_and(|stored| stored.enumeration_id == page.enumeration_id);
            if !owned {
                continue;
            }
            let path_ref_id = stored.and_then(|stored| stored.path_ref_id.clone());
            session.entries.remove(entry_id);
            if let Some(path_ref_id) = path_ref_id {
                remove_path_if_unpinned(session, &path_ref_id);
            }
        }
        Ok(())
    }

    /// Releases a navigation/history pin. Page ownership, if still live, keeps
    /// the path resolvable until that page is released.
    pub(crate) fn release_path_ref(
        &self,
        session_id: &str,
        path_ref: &BrowsePathRef,
    ) -> Result<(), BrowseError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        let session = sessions
            .get_mut(session_id)
            .ok_or(BrowseError::SessionNotFound)?;
        if path_ref.id == session.root_path_ref.id {
            return Ok(());
        }
        {
            let path = session
                .paths
                .get_mut(&path_ref.id)
                .ok_or(BrowseError::InvalidPathRef)?;
            path.pinned = false;
        }
        remove_path_if_unpinned(session, &path_ref.id);
        Ok(())
    }

    /// Promotes a page-owned directory ref to a navigation/history pin before
    /// its publishing page or enumeration is torn down. The caller still
    /// owns the eventual release; this only keeps the opaque ref resolvable
    /// across the page-to-history ownership transition.
    pub(crate) fn retain_path_ref(
        &self,
        session_id: &str,
        path_ref: &BrowsePathRef,
    ) -> Result<(), BrowseError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        let session = sessions
            .get_mut(session_id)
            .ok_or(BrowseError::SessionNotFound)?;
        let path = session
            .paths
            .get_mut(&path_ref.id)
            .ok_or(BrowseError::InvalidPathRef)?;
        path.pinned = true;
        Ok(())
    }

    pub(crate) fn validate_page(&self, page: &BrowsePage) -> Result<(), BrowseError> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        let session = sessions
            .get(&page.session_id)
            .ok_or(BrowseError::SessionNotFound)?;
        let current = session
            .active
            .as_ref()
            .ok_or(BrowseError::StaleEnumeration)?;
        if current.identity.session_id == page.session_id
            && current.identity.request_id == page.request_id
            && current.identity.enumeration_id == page.enumeration_id
        {
            Ok(())
        } else {
            Err(BrowseError::StaleEnumeration)
        }
    }

    pub(crate) fn resolve_entry(
        &self,
        entry_ref: &BrowseEntryRef,
    ) -> Result<ResolvedBrowseEntry, BrowseError> {
        let BrowseEntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        } = entry_ref;
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        let session = sessions
            .get(browse_session_id)
            .ok_or(BrowseError::SessionNotFound)?;
        let entry = session
            .entries
            .get(entry_id)
            .ok_or(BrowseError::InvalidEntryRef)?;
        Ok(ResolvedBrowseEntry {
            path: entry.path.clone(),
            kind: entry.kind,
        })
    }

    /// Resolve only the owning Browse enumeration for cache/deduplication
    /// namespacing. The entry registry remains the lifetime authority; this
    /// method does not expose the stored path or session state.
    pub(crate) fn resolve_entry_generation(
        &self,
        entry_ref: &BrowseEntryRef,
    ) -> Result<String, BrowseError> {
        let BrowseEntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        } = entry_ref;
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        let session = sessions
            .get(browse_session_id)
            .ok_or(BrowseError::SessionNotFound)?;
        session
            .entries
            .get(entry_id)
            .map(|entry| entry.enumeration_id.clone())
            .ok_or(BrowseError::InvalidEntryRef)
    }

    pub(crate) fn dispose_session(&self, session_id: &str) -> Result<(), BrowseError> {
        let session = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| BrowseError::StateUnavailable)?;
            sessions
                .remove(session_id)
                .ok_or(BrowseError::SessionNotFound)?
        };
        if let Some(enumeration) = &session.active {
            enumeration.cancel(CancelReason::Disposed);
        }
        Ok(())
    }

    #[cfg(test)]
    fn state_counts(&self, session_id: &str) -> Result<(usize, usize), BrowseError> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        let session = sessions
            .get(session_id)
            .ok_or(BrowseError::SessionNotFound)?;
        Ok((session.paths.len(), session.entries.len()))
    }

    #[cfg(test)]
    pub(crate) fn session_count(&self) -> usize {
        self.sessions
            .lock()
            .map(|sessions| sessions.len())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn resource_counts(&self) -> BrowseResourceCounts {
        self.sessions
            .lock()
            .map(|sessions| BrowseResourceCounts {
                sessions: sessions.len(),
                entry_refs: sessions.values().map(|session| session.entries.len()).sum(),
                path_refs: sessions.values().map(|session| session.paths.len()).sum(),
                active_enumerations: sessions
                    .values()
                    .map(|session| usize::from(session.active.is_some()))
                    .sum(),
            })
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn active_enumeration_debug(
        &self,
        session_id: &str,
    ) -> Result<Option<BrowseEnumerationDebugSnapshot>, BrowseError> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        let session = sessions
            .get(session_id)
            .ok_or(BrowseError::SessionNotFound)?;
        let Some(enumeration) = &session.active else {
            return Ok(None);
        };
        let current_cursor = match &*enumeration
            .cursor
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?
        {
            CursorState::Ready(cursor) => Some(cursor.clone()),
            CursorState::Initial
            | CursorState::InFlight
            | CursorState::Complete
            | CursorState::Failed
            | CursorState::Cancelled => None,
        };
        Ok(Some(BrowseEnumerationDebugSnapshot {
            session_id: enumeration.identity.session_id.clone(),
            request_id: enumeration.identity.request_id.clone(),
            enumeration_id: enumeration.identity.enumeration_id.clone(),
            current_cursor,
        }))
    }

    #[cfg(test)]
    pub(crate) fn set_test_publish_gate(&self, gate: Arc<TestPublishGate>) {
        *self
            .test_publish_gate
            .lock()
            .expect("test publish gate lock") = Some(gate);
    }

    #[cfg(test)]
    pub(crate) fn pause_before_publish(&self) {
        let gate = self
            .test_publish_gate
            .lock()
            .expect("test publish gate lock")
            .take();
        if let Some(gate) = gate {
            gate.pause();
        }
    }

    fn read_page(
        &self,
        session_id: &str,
        enumeration: Arc<EnumerationState>,
        cursor: Option<&str>,
        page_size: usize,
    ) -> Result<BrowsePage, BrowseError> {
        self.ensure_current_or_cancelled(session_id, &enumeration)?;
        enumeration.claim_cursor(cursor)?;

        #[cfg(all(test, feature = "performance-test-tauri"))]
        stage_trace::mark(6);
        let (pending_entries, complete) = match read_entries(&enumeration, page_size) {
            Ok(result) => result,
            Err(error) => {
                enumeration.fail();
                self.revoke_if_current(session_id, &enumeration)?;
                return Err(error);
            }
        };

        #[cfg(all(test, feature = "performance-test-tauri"))]
        stage_trace::mark(5);
        #[cfg(test)]
        self.pause_before_publish();

        #[cfg(all(test, feature = "performance-test-tauri"))]
        stage_trace::mark(7);

        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        #[cfg(all(test, feature = "performance-test-tauri"))]
        stage_trace::mark(8);
        let live_entry_refs = sessions
            .values()
            .map(|value| value.entries.len())
            .sum::<usize>();
        let live_path_refs = sessions
            .values()
            .map(|value| value.paths.len())
            .sum::<usize>();
        let session = sessions
            .get_mut(session_id)
            .ok_or(BrowseError::StalePublication)?;
        let is_current = session
            .active
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &enumeration));
        if !is_current {
            return Err(enumeration.publication_error());
        }
        if let Some(reason) = enumeration.cancel_reason() {
            return Err(reason.error());
        }

        let required_entries = pending_entries.len();
        let required_paths = pending_entries
            .iter()
            .filter(|entry| entry.kind == BrowseEntryKind::Directory)
            .count();
        if session.entries.len().saturating_add(required_entries) > self.limits.max_entry_refs
            || session.paths.len().saturating_add(required_paths) > self.limits.max_path_refs
            || live_entry_refs.saturating_add(required_entries) > self.limits.max_process_entry_refs
            || live_path_refs.saturating_add(required_paths) > self.limits.max_process_path_refs
        {
            drop(sessions);
            enumeration.restore_page(cursor, pending_entries)?;
            return Err(BrowseError::TemporaryStateCapacityExceeded);
        }

        let mut entries = Vec::with_capacity(pending_entries.len());
        for pending in pending_entries {
            let path_ref = if pending.kind == BrowseEntryKind::Directory {
                Some(register_path(
                    session,
                    pending.path.clone(),
                    self.limits.max_path_refs,
                )?)
            } else {
                None
            };
            let entry_id = opaque_id();
            let entry_ref = BrowseEntryRef::Ephemeral {
                browse_session_id: session_id.to_string(),
                entry_id: entry_id.clone(),
            };
            register_entry(
                session,
                entry_id,
                pending.path.clone(),
                pending.kind,
                &enumeration.identity.enumeration_id,
                path_ref.as_ref().map(|value| value.id.clone()),
            );
            entries.push(EphemeralBrowseEntry {
                entry_ref,
                path_ref,
                name: pending.name,
                display_path: pending.display_path,
                kind: pending.kind,
                extension: pending.extension,
                size: pending.size,
                modified_at: pending.modified_at,
                created_at: pending.created_at,
                materialization: MaterializationState::Unknown,
            });
        }

        let emitted_count = enumeration.add_emitted_count(entries.len());
        let next_cursor = if complete {
            enumeration.complete();
            None
        } else {
            let next_cursor = opaque_id();
            enumeration.publish_cursor(next_cursor.clone())?;
            Some(next_cursor)
        };

        Ok(BrowsePage {
            session_id: enumeration.identity.session_id.clone(),
            request_id: enumeration.identity.request_id.clone(),
            enumeration_id: enumeration.identity.enumeration_id.clone(),
            entries,
            next_cursor,
            completion: if complete {
                BrowseCompletion::Complete
            } else {
                BrowseCompletion::Partial
            },
            known_count: complete.then_some(emitted_count),
            skipped_entries: enumeration.skipped_entries.load(Ordering::Acquire),
        })
    }

    fn revoke_if_current(
        &self,
        session_id: &str,
        enumeration: &EnumerationState,
    ) -> Result<(), BrowseError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        if let Some(session) = sessions.get_mut(session_id) {
            let is_current = session
                .active
                .as_ref()
                .is_some_and(|current| current.identity == enumeration.identity);
            if is_current {
                invalidate_entries_for_enumeration(session, &enumeration.identity.enumeration_id);
                session.active = None;
            }
        }
        Ok(())
    }

    fn ensure_current_or_cancelled(
        &self,
        session_id: &str,
        enumeration: &EnumerationState,
    ) -> Result<(), BrowseError> {
        if let Some(reason) = enumeration.cancel_reason() {
            return Err(reason.error());
        }
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        let current = sessions
            .get(session_id)
            .and_then(|session| session.active.as_ref());
        if current.is_some_and(|current| current.identity == enumeration.identity) {
            Ok(())
        } else {
            Err(BrowseError::StaleEnumeration)
        }
    }

    fn validate_request_id(&self, request_id: &str) -> Result<(), BrowseError> {
        if request_id.is_empty() || request_id.len() > MAX_ID_LENGTH {
            Err(BrowseError::InvalidRequest)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BrowseResourceCounts {
    pub(crate) sessions: usize,
    pub(crate) entry_refs: usize,
    pub(crate) path_refs: usize,
    pub(crate) active_enumerations: usize,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowseEnumerationDebugSnapshot {
    pub(crate) session_id: String,
    pub(crate) request_id: String,
    pub(crate) enumeration_id: String,
    pub(crate) current_cursor: Option<String>,
}

#[derive(Debug)]
struct BrowseSessionState {
    location_id: String,
    root_path_ref: BrowsePathRef,
    paths: HashMap<String, StoredPath>,
    entries: HashMap<String, StoredEntry>,
    active: Option<Arc<EnumerationState>>,
}

#[derive(Debug)]
struct StoredPath {
    path: PathBuf,
    pinned: bool,
}

#[derive(Debug)]
struct StoredEntry {
    path: PathBuf,
    kind: BrowseEntryKind,
    enumeration_id: String,
    path_ref_id: Option<String>,
}

#[derive(Debug)]
struct EnumerationState {
    identity: BrowseEnumerationRef,
    query: BrowseQuerySpecV1,
    source: Mutex<EnumerationSource>,
    cursor: Mutex<CursorState>,
    cancel_reason: AtomicU8,
    emitted_count: AtomicUsize,
    skipped_entries: AtomicUsize,
}

impl EnumerationState {
    fn new(identity: BrowseEnumerationRef, source: ReadDir) -> Self {
        Self::new_with_query(identity, source, BrowseQuerySpecV1::default())
    }

    fn new_with_query(
        identity: BrowseEnumerationRef,
        source: ReadDir,
        query: BrowseQuerySpecV1,
    ) -> Self {
        Self {
            identity,
            query,
            source: Mutex::new(EnumerationSource {
                read_dir: source,
                sort_ready: false,
                sort_scanned: 0,
                buffered: VecDeque::new(),
            }),
            cursor: Mutex::new(CursorState::Initial),
            cancel_reason: AtomicU8::new(CancelReason::None as u8),
            emitted_count: AtomicUsize::new(0),
            skipped_entries: AtomicUsize::new(0),
        }
    }

    fn claim_cursor(&self, requested: Option<&str>) -> Result<(), BrowseError> {
        let mut cursor = self
            .cursor
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        let valid = match (&*cursor, requested) {
            (CursorState::Initial, None) => true,
            (CursorState::Ready(expected), Some(actual)) => expected == actual,
            _ => false,
        };
        if !valid {
            return Err(BrowseError::InvalidCursor);
        }
        *cursor = CursorState::InFlight;
        Ok(())
    }

    fn restore_page(
        &self,
        requested_cursor: Option<&str>,
        entries: Vec<PendingEntry>,
    ) -> Result<(), BrowseError> {
        {
            let mut source = self
                .source
                .lock()
                .map_err(|_| BrowseError::StateUnavailable)?;
            for entry in entries.into_iter().rev() {
                source.buffered.push_front(entry);
            }
        }
        let mut cursor = self
            .cursor
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        if !matches!(*cursor, CursorState::InFlight) {
            return Err(BrowseError::StalePublication);
        }
        *cursor = match requested_cursor {
            Some(value) => CursorState::Ready(value.to_string()),
            None => CursorState::Initial,
        };
        Ok(())
    }

    fn publish_cursor(&self, cursor_value: String) -> Result<(), BrowseError> {
        let mut cursor = self
            .cursor
            .lock()
            .map_err(|_| BrowseError::StateUnavailable)?;
        if !matches!(*cursor, CursorState::InFlight) {
            return Err(BrowseError::StalePublication);
        }
        *cursor = CursorState::Ready(cursor_value);
        Ok(())
    }

    fn complete(&self) {
        if let Ok(mut cursor) = self.cursor.lock() {
            *cursor = CursorState::Complete;
        }
    }

    fn fail(&self) {
        if let Ok(mut cursor) = self.cursor.lock() {
            *cursor = CursorState::Failed;
        }
    }

    fn cancel(&self, reason: CancelReason) {
        self.cancel_reason.store(reason as u8, Ordering::Release);
        if let Ok(mut cursor) = self.cursor.lock() {
            *cursor = CursorState::Cancelled;
        }
    }

    fn cancel_reason(&self) -> Option<CancelReason> {
        CancelReason::from_u8(self.cancel_reason.load(Ordering::Acquire))
            .filter(|reason| *reason != CancelReason::None)
    }

    fn publication_error(&self) -> BrowseError {
        self.cancel_reason()
            .map(CancelReason::error)
            .unwrap_or(BrowseError::StalePublication)
    }

    fn add_emitted_count(&self, count: usize) -> usize {
        self.emitted_count.fetch_add(count, Ordering::AcqRel) + count
    }
}

#[derive(Debug)]
struct EnumerationSource {
    read_dir: ReadDir,
    sort_ready: bool,
    sort_scanned: usize,
    buffered: VecDeque<PendingEntry>,
}

#[derive(Debug)]
enum CursorState {
    Initial,
    Ready(String),
    InFlight,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum CancelReason {
    None = 0,
    Explicit = 1,
    Superseded = 2,
    Invalidated = 3,
    Disposed = 4,
}

impl CancelReason {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Explicit),
            2 => Some(Self::Superseded),
            3 => Some(Self::Invalidated),
            4 => Some(Self::Disposed),
            _ => None,
        }
    }

    fn error(self) -> BrowseError {
        match self {
            Self::Explicit | Self::Disposed => BrowseError::Cancelled,
            Self::Superseded | Self::Invalidated => BrowseError::StalePublication,
            Self::None => BrowseError::StateUnavailable,
        }
    }
}

#[derive(Debug)]
struct PendingEntry {
    path: PathBuf,
    name: String,
    display_path: String,
    kind: BrowseEntryKind,
    extension: Option<String>,
    size: Option<u64>,
    modified_at: Option<i64>,
    created_at: Option<i64>,
}

fn read_entries(
    enumeration: &EnumerationState,
    page_size: usize,
) -> Result<(Vec<PendingEntry>, bool), BrowseError> {
    let mut source = enumeration
        .source
        .lock()
        .map_err(|_| BrowseError::StateUnavailable)?;
    if let Some(sort) = &enumeration.query.sort {
        if !source.sort_ready {
            let mut eof = false;
            for _ in 0..RAW_DIRECTORY_SCAN_BUDGET {
                ensure_not_cancelled(enumeration)?;
                let Some(entry) = next_dir_entry(&mut source)? else {
                    eof = true;
                    break;
                };
                source.sort_scanned += 1;
                if source.sort_scanned > MAX_SORT_ENTRIES {
                    return Err(BrowseError::SortCapacityExceeded);
                }
                match read_entry(entry) {
                    Ok(entry) if entry_matches_query(&entry, &enumeration.query) => {
                        source.buffered.push_back(entry)
                    }
                    Ok(_) => {}
                    Err(error) if is_skippable_child_error(error) => {
                        enumeration.skipped_entries.fetch_add(1, Ordering::AcqRel);
                    }
                    Err(error) => return Err(error),
                }
            }
            if !eof {
                return Ok((Vec::new(), false));
            }
            source
                .buffered
                .make_contiguous()
                .sort_by(|a, b| compare_pending(a, b, sort));
            source.sort_ready = true;
        }
        let count = page_size.min(source.buffered.len());
        let entries = source.buffered.drain(..count).collect();
        return Ok((entries, source.buffered.is_empty()));
    }
    let mut entries = Vec::with_capacity(page_size);
    let mut raw_scanned = 0;

    while entries.len() < page_size && raw_scanned < RAW_DIRECTORY_SCAN_BUDGET {
        ensure_not_cancelled(enumeration)?;
        if let Some(entry) = source.buffered.pop_front() {
            if entry_matches_query(&entry, &enumeration.query) {
                entries.push(entry);
            }
            continue;
        }
        raw_scanned += 1;
        let Some(entry) = next_dir_entry(&mut source)? else {
            return Ok((entries, true));
        };
        match read_entry(entry) {
            Ok(entry) if entry_matches_query(&entry, &enumeration.query) => entries.push(entry),
            Ok(_) => continue,
            Err(error) if is_skippable_child_error(error) => {
                enumeration.skipped_entries.fetch_add(1, Ordering::AcqRel);
                continue;
            }
            Err(error) => return Err(error),
        }
    }

    ensure_not_cancelled(enumeration)?;
    Ok((entries, false))
}

fn compare_pending(
    a: &PendingEntry,
    b: &PendingEntry,
    sort: &BrowseSortSpec,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    if sort.folders_first && a.kind != b.kind {
        return if a.kind == BrowseEntryKind::Directory {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    let value = match sort.field {
        BrowseSortField::Name => a.name.cmp(&b.name),
        BrowseSortField::Type => a.extension.cmp(&b.extension),
        BrowseSortField::Size => a.size.cmp(&b.size),
        BrowseSortField::Modified => a.modified_at.cmp(&b.modified_at),
    }
    .then_with(|| a.name.cmp(&b.name))
    .then_with(|| a.path.cmp(&b.path));
    if sort.direction == BrowseSortDirection::Desc {
        value.reverse()
    } else {
        value
    }
}

fn entry_matches_query(entry: &PendingEntry, query: &BrowseQuerySpecV1) -> bool {
    let kind_matches = match query.entry_kind {
        BrowseQueryEntryKind::All => true,
        BrowseQueryEntryKind::File => entry.kind == BrowseEntryKind::File,
        BrowseQueryEntryKind::Directory => entry.kind == BrowseEntryKind::Directory,
    };
    let text_matches = query
        .text
        .as_ref()
        .is_none_or(|text| entry.name.to_lowercase().contains(text));
    kind_matches && text_matches
}

fn next_dir_entry(source: &mut EnumerationSource) -> Result<Option<DirEntry>, BrowseError> {
    match source.read_dir.next() {
        None => Ok(None),
        Some(Ok(entry)) => Ok(Some(entry)),
        Some(Err(error)) => Err(map_directory_error(error)),
    }
}

fn read_entry(entry: DirEntry) -> Result<PendingEntry, BrowseError> {
    let path = entry.path();
    let metadata = fs::symlink_metadata(&path).map_err(map_entry_error)?;
    if matches!(
        classify_link_like(&path, &metadata),
        LinkLike::Unsafe | LinkLike::Unknown
    ) {
        return Err(BrowseError::UnsupportedEntry);
    }

    let kind = if metadata.is_dir() {
        BrowseEntryKind::Directory
    } else if metadata.is_file() {
        BrowseEntryKind::File
    } else {
        return Err(BrowseError::UnsupportedEntry);
    };

    Ok(PendingEntry {
        name: entry.file_name().to_string_lossy().into_owned(),
        display_path: path.to_string_lossy().into_owned(),
        extension: path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_owned),
        size: (kind == BrowseEntryKind::File).then_some(metadata.len()),
        modified_at: filesystem_time_seconds(&metadata, false),
        created_at: filesystem_time_seconds(&metadata, true),
        path,
        kind,
    })
}

fn is_skippable_child_error(error: BrowseError) -> bool {
    matches!(
        error,
        BrowseError::EntryPermissionDenied
            | BrowseError::EntryNotFound
            | BrowseError::EntryUnavailable
            | BrowseError::UnsupportedEntry
    )
}

fn ensure_not_cancelled(enumeration: &EnumerationState) -> Result<(), BrowseError> {
    if let Some(reason) = enumeration.cancel_reason() {
        Err(reason.error())
    } else {
        Ok(())
    }
}

fn register_path(
    session: &mut BrowseSessionState,
    path: PathBuf,
    max_path_refs: usize,
) -> Result<BrowsePathRef, BrowseError> {
    if session.paths.len() >= max_path_refs {
        return Err(BrowseError::TemporaryStateCapacityExceeded);
    }
    let path_ref = BrowsePathRef { id: opaque_id() };
    session.paths.insert(
        path_ref.id.clone(),
        StoredPath {
            path,
            pinned: false,
        },
    );
    Ok(path_ref)
}

fn register_entry(
    session: &mut BrowseSessionState,
    entry_id: String,
    path: PathBuf,
    kind: BrowseEntryKind,
    enumeration_id: &str,
    path_ref_id: Option<String>,
) {
    session.entries.insert(
        entry_id,
        StoredEntry {
            path,
            kind,
            enumeration_id: enumeration_id.to_string(),
            path_ref_id,
        },
    );
}

fn remove_path_if_unpinned(session: &mut BrowseSessionState, path_ref_id: &str) {
    if path_ref_id == session.root_path_ref.id {
        return;
    }
    let pinned = session
        .paths
        .get(path_ref_id)
        .is_some_and(|stored| stored.pinned);
    let still_page_owned = session
        .entries
        .values()
        .any(|entry| entry.path_ref_id.as_deref() == Some(path_ref_id));
    if !pinned && !still_page_owned {
        session.paths.remove(path_ref_id);
    }
}

fn invalidate_entries_for_enumeration(session: &mut BrowseSessionState, enumeration_id: &str) {
    let path_ref_ids = session
        .entries
        .values()
        .filter(|entry| entry.enumeration_id == enumeration_id)
        .filter_map(|entry| entry.path_ref_id.clone())
        .collect::<Vec<_>>();
    session
        .entries
        .retain(|_, entry| entry.enumeration_id != enumeration_id);
    for path_ref_id in path_ref_ids {
        remove_path_if_unpinned(session, &path_ref_id);
    }
}

fn validate_directory_path(path: &Path) -> Result<(), BrowseError> {
    if path.as_os_str().is_empty() {
        return Err(BrowseError::DirectoryNotFound);
    }
    let metadata = fs::symlink_metadata(path).map_err(map_directory_error)?;
    if matches!(
        classify_link_like(path, &metadata),
        LinkLike::Unsafe | LinkLike::Unknown
    ) {
        return Err(BrowseError::UnsupportedEntry);
    }
    if !metadata.is_dir() {
        return Err(BrowseError::TargetNotDirectory);
    }
    Ok(())
}

fn map_directory_error(error: io::Error) -> BrowseError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => BrowseError::DirectoryPermissionDenied,
        io::ErrorKind::NotFound => BrowseError::DirectoryNotFound,
        _ => BrowseError::DirectoryUnavailable,
    }
}

fn map_entry_error(error: io::Error) -> BrowseError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => BrowseError::EntryPermissionDenied,
        io::ErrorKind::NotFound => BrowseError::EntryNotFound,
        _ => BrowseError::EntryUnavailable,
    }
}

fn filesystem_time_seconds(metadata: &fs::Metadata, created: bool) -> Option<i64> {
    let time = if created {
        metadata.created().ok()
    } else {
        metadata.modified().ok()
    }?;
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
}

fn opaque_id() -> String {
    Uuid::new_v4().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkLike {
    Ordinary,
    Unsafe,
    ProviderOrOther,
    Unknown,
}

#[cfg(windows)]
fn classify_link_like(path: &Path, metadata: &fs::Metadata) -> LinkLike {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    let has_reparse_attribute = metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    if !has_reparse_attribute && !metadata.file_type().is_symlink() {
        return LinkLike::Ordinary;
    }
    windows_reparse_tag(path)
        .map(classify_windows_reparse_tag)
        .unwrap_or(LinkLike::Unknown)
}

#[cfg(windows)]
fn classify_windows_reparse_tag(tag: u32) -> LinkLike {
    use windows_sys::Win32::System::SystemServices::{
        IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK,
    };
    const IO_REPARSE_TAG_NAME_SURROGATE: u32 = 0x2000_0000;
    if matches!(tag, IO_REPARSE_TAG_MOUNT_POINT | IO_REPARSE_TAG_SYMLINK)
        || tag & IO_REPARSE_TAG_NAME_SURROGATE != 0
    {
        LinkLike::Unsafe
    } else {
        LinkLike::ProviderOrOther
    }
}

#[cfg(windows)]
fn windows_reparse_tag(path: &Path) -> Option<u32> {
    use std::fs::OpenOptions;
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FileAttributeTagInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_TAG_INFO,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    };

    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    let success = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    (success != 0).then_some(info.ReparseTag)
}

#[cfg(not(windows))]
fn classify_link_like(_path: &Path, metadata: &fs::Metadata) -> LinkLike {
    if metadata.file_type().is_symlink() {
        LinkLike::Unsafe
    } else {
        LinkLike::Ordinary
    }
}

// #404-B: only the synchronous performance test opts in. No observer symbols
// or calls exist in production or in tests without performance-test-tauri.
#[cfg(all(test, feature = "performance-test-tauri"))]
pub(crate) mod stage_trace {
    use std::{cell::RefCell, time::Instant};

    pub const CAPACITY: usize = 40;
    pub const STAGES: usize = 9;
    // Stages partition the inner call; lock-held totals are supplementary,
    // overlapping envelopes (2+3+4 and 8), never added to the partition.
    // They include guard release/observer overhead; they are not exact native
    // mutex ownership intervals. Stage 5 also includes ensure-current locking.
    pub const NAMES: [&str; STAGES] = [
        "request_validation",
        "open_sessions_lock_wait",
        "lookup_directory_validation_open_lock_held",
        "previous_enumeration_ref_invalidation_lock_held",
        "install_enumeration_lock_release_envelope",
        "current_check_cursor_and_envelope_mixed",
        "read_entries_metadata_source_lock_error_cleanup_mixed",
        "publication_sessions_lock_wait",
        "publication_refs_cursor_lock_release_envelope",
    ];

    #[derive(Clone, Copy, Debug, Default)]
    pub struct Observation {
        pub group: u8,
        pub sample: usize,
        pub outer_ns: Option<u64>,
        pub inner_ns: Option<u64>,
        pub stages_ns: [u64; STAGES],
        pub entered: bool,
        pub completed: bool,
        pub entries: usize,
        pub cpu_delta_100ns: Option<u64>,
    }

    #[derive(Clone, Copy, Debug, Default)]
    pub struct Calibration {
        pub timer_min_positive_ns: Option<u64>,
        pub timer_pair_max_ns: u64,
        pub cpu_read_max_ns: u64,
        pub cpu_min_positive_delta_100ns: Option<u64>,
        pub cpu_available: bool,
    }

    pub struct Report {
        pub observations: [Observation; CAPACITY],
        pub len: usize,
        pub calibration: Calibration,
    }

    struct Current {
        index: usize,
        call_start: Option<Instant>,
        last: Option<Instant>,
        stage: usize,
    }

    struct State {
        report: Report,
        current: Option<Current>,
    }

    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    fn ns(start: Instant) -> u64 {
        start.elapsed().as_nanos().min(u64::MAX as u128) as u64
    }

    // Uses only the already enabled windows-sys Foundation/Threading APIs.
    // FILETIME is a unit, NOT a promise of effective sub-millisecond precision.
    // Raw deltas are never subtracted from wall time or converted from cycles.
    fn thread_cpu_100ns() -> Option<u64> {
        #[cfg(target_os = "windows")]
        {
            use windows_sys::Win32::{
                Foundation::FILETIME,
                System::Threading::{GetCurrentThread, GetThreadTimes},
            };
            let mut created = FILETIME::default();
            let mut exited = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            // SAFETY: the pseudo-handle denotes this live thread; all four
            // output pointers are distinct, initialized, writable FILETIMEs.
            // No handle is retained or closed and no scheduling state changes.
            let ok = unsafe {
                GetThreadTimes(
                    GetCurrentThread(),
                    &mut created,
                    &mut exited,
                    &mut kernel,
                    &mut user,
                )
            };
            if ok == 0 {
                return None;
            }
            let ticks =
                |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
            ticks(kernel).checked_add(ticks(user))
        }
        #[cfg(not(target_os = "windows"))]
        {
            None
        }
    }

    fn min_positive(current: &mut Option<u64>, value: u64) {
        if value > 0 {
            *current = Some(current.map_or(value, |old| old.min(value)));
        }
    }

    fn calibrate() -> Calibration {
        // Fixed 64 adjacent reads, outside the workload/timed samples. No spin,
        // sleep, synthetic pressure, affinity or timer-resolution changes.
        let mut previous = thread_cpu_100ns();
        let mut result = Calibration {
            cpu_available: previous.is_some(),
            ..Calibration::default()
        };
        for _ in 0..64 {
            let start = Instant::now();
            let elapsed = ns(start);
            min_positive(&mut result.timer_min_positive_ns, elapsed);
            result.timer_pair_max_ns = result.timer_pair_max_ns.max(elapsed);
            let start = Instant::now();
            let cpu = thread_cpu_100ns();
            result.cpu_read_max_ns = result.cpu_read_max_ns.max(ns(start));
            if let Some(delta) = previous.zip(cpu).and_then(|(a, b)| b.checked_sub(a)) {
                min_positive(&mut result.cpu_min_positive_delta_100ns, delta);
            }
            result.cpu_available &= cpu.is_some();
            previous = cpu;
        }
        result
    }

    pub struct Scope;
    impl Scope {
        pub fn enable() -> Self {
            let calibration = calibrate();
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                assert!(state.is_none(), "Browse observation scopes cannot nest");
                *state = Some(State {
                    report: Report {
                        observations: [Observation::default(); CAPACITY],
                        len: 0,
                        calibration,
                    },
                    current: None,
                });
            });
            Self
        }
        pub fn finish(self) -> Report {
            STATE.with(|state| {
                state
                    .borrow_mut()
                    .take()
                    .expect("active observation scope")
                    .report
            })
        }
    }
    impl Drop for Scope {
        fn drop(&mut self) {
            STATE.with(|state| {
                *state.borrow_mut() = None;
            });
        }
    }

    pub struct Sample {
        active: bool,
        cpu_before: Option<u64>,
    }
    impl Sample {
        pub fn begin(group: u8, sample: usize) -> Self {
            let active = STATE.with(|state| {
                let mut state = state.borrow_mut();
                let Some(state) = state.as_mut() else {
                    return false;
                };
                if state.report.len == CAPACITY || state.current.is_some() {
                    return false;
                }
                let index = state.report.len;
                state.report.len += 1;
                state.report.observations[index] = Observation {
                    group,
                    sample,
                    ..Observation::default()
                };
                state.current = Some(Current {
                    index,
                    call_start: None,
                    last: None,
                    stage: 0,
                });
                true
            });
            Self {
                active,
                cpu_before: active.then(thread_cpu_100ns).flatten(),
            }
        }
        pub fn finish(mut self, outer_ns: u64) {
            self.end(Some(outer_ns));
        }
        fn end(&mut self, outer_ns: Option<u64>) {
            if !self.active {
                return;
            }
            let cpu_after = thread_cpu_100ns();
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                if let Some(state) = state.as_mut() {
                    if let Some(current) = state.current.take() {
                        let observation = &mut state.report.observations[current.index];
                        observation.outer_ns = outer_ns;
                        observation.cpu_delta_100ns = self
                            .cpu_before
                            .zip(cpu_after)
                            .and_then(|(a, b)| b.checked_sub(a));
                    }
                }
            });
            self.active = false;
        }
    }
    impl Drop for Sample {
        fn drop(&mut self) {
            self.end(None);
        }
    }

    pub struct Call;
    impl Call {
        pub fn begin() -> Self {
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                if let Some(state) = state.as_mut() {
                    if let Some(current) = state.current.as_mut() {
                        let now = Instant::now();
                        current.call_start = Some(now);
                        current.last = Some(now);
                        current.stage = 0;
                        state.report.observations[current.index].entered = true;
                    }
                }
            });
            Self
        }
        pub fn complete(&self, entries: usize) {
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                if let Some(state) = state.as_mut() {
                    if let Some(current) = state.current.as_ref() {
                        let observation = &mut state.report.observations[current.index];
                        observation.completed = true;
                        observation.entries = entries;
                    }
                }
            });
        }
    }
    impl Drop for Call {
        fn drop(&mut self) {
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                if let Some(state) = state.as_mut() {
                    if let Some(current) = state.current.as_mut() {
                        if let (Some(start), Some(last)) =
                            (current.call_start.take(), current.last.take())
                        {
                            let now = Instant::now();
                            let observation = &mut state.report.observations[current.index];
                            observation.stages_ns[current.stage] +=
                                now.duration_since(last).as_nanos() as u64;
                            observation.inner_ns =
                                Some(now.duration_since(start).as_nanos() as u64);
                        }
                    }
                }
            });
        }
    }
    pub fn mark(stage: usize) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            if let Some(state) = state.as_mut() {
                if let Some(current) = state.current.as_mut() {
                    if let Some(last) = current.last {
                        let now = Instant::now();
                        state.report.observations[current.index].stages_ns[current.stage] +=
                            now.duration_since(last).as_nanos() as u64;
                        current.last = Some(now);
                        current.stage = stage;
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_workspace::EntryRef;
    use serde_json::json;
    use std::fs;
    use std::io::{self, ErrorKind};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    static FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("src-tauri has repository parent")
                .to_path_buf();
            Self::new_under(repo_root.join(".tmp-tests").join("browse"))
        }

        fn new_under(parent: PathBuf) -> Self {
            let id = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let root = parent.join(format!(
                "{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos(),
                id
            ));
            fs::create_dir_all(&root).expect("create fixture root");
            Self { root }
        }

        fn directory(&self) -> BackendResolvedDirectory {
            BackendResolvedDirectory::from_backend_path(self.root.clone()).expect("directory")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).expect("remove fixture root");
        }
    }

    fn service(limits: BrowseLimits) -> BrowseService {
        BrowseService::new(limits).expect("valid limits")
    }

    #[test]
    fn sorted_pages_cover_whole_directory_with_stable_ties_and_directions() {
        let fixture = Fixture::new();
        for i in (0..137).rev() {
            let path = fixture.root.join(format!(
                "item-{i:03}.{}",
                if i % 2 == 0 { "md" } else { "txt" }
            ));
            fs::write(path, vec![b'x'; i % 7]).expect("file");
        }
        fs::create_dir(fixture.root.join("folder")).expect("directory");
        let service = service(BrowseLimits::default());
        let session = service.start_session(fixture.directory()).expect("session");
        for field in [
            BrowseSortField::Name,
            BrowseSortField::Type,
            BrowseSortField::Size,
            BrowseSortField::Modified,
        ] {
            for direction in [BrowseSortDirection::Asc, BrowseSortDirection::Desc] {
                let mut page = service
                    .start_enumeration_with_query(
                        &session.session_id,
                        opaque_id(),
                        &session.root_path_ref,
                        17,
                        BrowseQuerySpecV1 {
                            text: None,
                            entry_kind: BrowseQueryEntryKind::All,
                            sort: Some(BrowseSortSpec {
                                field,
                                direction,
                                folders_first: true,
                            }),
                        },
                    )
                    .expect("first");
                let mut entries = page.entries.clone();
                while let Some(cursor) = page.next_cursor.clone() {
                    page = service
                        .next_page(&session.session_id, &cursor, 17)
                        .expect("next");
                    entries.extend(page.entries.clone());
                }
                assert_eq!(entries.len(), 138);
                assert_eq!(page.known_count, Some(138));
                assert_eq!(entries[0].name, "folder");
                for pair in entries[1..].windows(2) {
                    let key = |e: &EphemeralBrowseEntry| match field {
                        BrowseSortField::Name => e.name.clone(),
                        BrowseSortField::Type => {
                            format!("{}:{}", e.extension.as_deref().unwrap_or(""), e.name)
                        }
                        BrowseSortField::Size => format!("{:020}:{}", e.size.unwrap(), e.name),
                        BrowseSortField::Modified => {
                            format!("{:020}:{}", e.modified_at.unwrap(), e.name)
                        }
                    };
                    assert!(if direction == BrowseSortDirection::Asc {
                        key(&pair[0]) < key(&pair[1])
                    } else {
                        key(&pair[0]) > key(&pair[1])
                    });
                }
                let old = &entries[1].entry_ref;
                // A refresh cannot keep old enumeration refs alive.
                service
                    .start_enumeration(&session.session_id, opaque_id(), &session.root_path_ref, 17)
                    .expect("refresh");
                assert_eq!(
                    service.resolve_entry(old),
                    Err(BrowseError::InvalidEntryRef)
                );
            }
        }
    }

    #[test]
    fn sorting_yields_empty_progress_until_eof_and_fails_closed_at_capacity() {
        let fixture = Fixture::new();
        create_fixture_files(&fixture, RAW_DIRECTORY_SCAN_BUDGET + 5);
        let service = service(BrowseLimits::default());
        let session = service.start_session(fixture.directory()).expect("session");
        let query = BrowseQuerySpecV1 {
            sort: Some(BrowseSortSpec {
                field: BrowseSortField::Name,
                direction: BrowseSortDirection::Asc,
                folders_first: false,
            }),
            ..Default::default()
        };
        let first = service
            .start_enumeration_with_query(
                &session.session_id,
                "sort",
                &session.root_path_ref,
                10,
                query,
            )
            .expect("first");
        assert!(first.entries.is_empty());
        assert_eq!(first.completion, BrowseCompletion::Partial);
        assert_eq!(first.known_count, None);
        let active = service
            .sessions
            .lock()
            .unwrap()
            .get(&session.session_id)
            .unwrap()
            .active
            .clone()
            .unwrap();
        active.source.lock().unwrap().sort_scanned = MAX_SORT_ENTRIES;
        assert_eq!(
            service.next_page(
                &session.session_id,
                first.next_cursor.as_deref().unwrap(),
                10
            ),
            Err(BrowseError::SortCapacityExceeded)
        );
    }

    #[cfg(unix)]
    #[test]
    fn sorted_coverage_reports_unsupported_children_instead_of_claiming_full_coverage() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("ok.txt"), b"data").unwrap();
        std::os::unix::fs::symlink(fixture.root.join("missing"), fixture.root.join("link"))
            .unwrap();
        let service = service(BrowseLimits::default());
        let session = service.start_session(fixture.directory()).unwrap();
        let page = service
            .start_enumeration_with_query(
                &session.session_id,
                "sort",
                &session.root_path_ref,
                10,
                BrowseQuerySpecV1 {
                    sort: Some(BrowseSortSpec {
                        field: BrowseSortField::Name,
                        direction: BrowseSortDirection::Asc,
                        folders_first: true,
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.skipped_entries, 1);
        assert_eq!(page.completion, BrowseCompletion::Complete);
    }

    fn create_fixture_files(fixture: &Fixture, count: usize) {
        for index in 0..count {
            fs::File::create(fixture.root.join(format!("entry-{index:06}.txt")))
                .expect("fixture file");
        }
    }

    #[test]
    fn fixture_uses_worktree_local_ignored_temp_and_cleans_it() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repo parent")
            .to_path_buf();
        let temp_root = repo_root.join(".tmp-tests");
        let lifecycle_parent =
            temp_root.join(format!("browse-fixture-lifecycle-{}", uuid::Uuid::new_v4()));
        let root = {
            let fixture = Fixture::new_under(lifecycle_parent.clone());
            assert!(fixture.root.starts_with(&temp_root));
            assert!(fixture.root.exists());
            fixture.root.clone()
        };
        assert!(!root.exists());
        assert!(
            lifecycle_parent.is_dir(),
            "fixture drop must preserve its shared parent"
        );
        assert!(
            temp_root.is_dir(),
            "fixture drop must preserve the worktree temp root"
        );
        fs::remove_dir(&lifecycle_parent)
            .expect("remove isolated fixture parent after its owner exits");
    }

    #[test]
    fn normal_enumeration_is_progressive_and_supports_multiple_pages() {
        let fixture = Fixture::new();
        for name in ["a.txt", "b.txt", "c.txt", "d.txt", "e.txt"] {
            fs::write(fixture.root.join(name), name).expect("file");
        }
        let service = service(BrowseLimits {
            max_sessions: 2,
            max_page_size: 3,
            max_path_refs: 8,
            max_entry_refs: 8,
            max_process_path_refs: 8,
            max_process_entry_refs: 8,
        });
        let session = service.start_session(fixture.directory()).expect("session");
        let first = service
            .start_enumeration(&session.session_id, "request-1", &session.root_path_ref, 2)
            .expect("first page");
        let cursor = first.next_cursor.clone().expect("cursor");
        let second = service
            .next_page(&session.session_id, &cursor, 2)
            .expect("second page");
        let cursor = second.next_cursor.clone().expect("cursor");
        let third = service
            .next_page(&session.session_id, &cursor, 2)
            .expect("third page");
        assert_eq!(first.entries.len(), 2);
        assert_eq!(second.entries.len(), 2);
        assert_eq!(third.entries.len(), 1);
        assert_eq!(third.completion, BrowseCompletion::Complete);
        assert_eq!(third.known_count, Some(5));
    }

    #[test]
    fn query_scans_past_non_matching_entries_and_keeps_exact_completion() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("first.txt"), b"first").expect("first");
        fs::write(fixture.root.join("late-sentinel.md"), b"sentinel").expect("sentinel");
        fs::write(fixture.root.join("other.bin"), b"other").expect("other");
        fs::create_dir(fixture.root.join("sentinel-folder")).expect("sentinel folder");
        let service = service(BrowseLimits::default());
        let session = service.start_session(fixture.directory()).expect("session");
        let query = BrowseQuerySpecV1 {
            text: Some("SENTINEL".to_string()),
            entry_kind: BrowseQueryEntryKind::All,
            sort: None,
        };

        let first = service
            .start_enumeration_with_query(
                &session.session_id,
                "query-first",
                &session.root_path_ref,
                1,
                query,
            )
            .expect("filtered first page");
        assert_eq!(first.entries.len(), 1);
        assert!(first.entries[0].name.to_lowercase().contains("sentinel"));
        assert_eq!(first.completion, BrowseCompletion::Partial);

        let second = service
            .next_page(
                &session.session_id,
                first.next_cursor.as_deref().expect("filtered cursor"),
                1,
            )
            .expect("filtered second page");
        assert_eq!(second.entries.len(), 1);
        assert!(second.entries[0].name.to_lowercase().contains("sentinel"));
        assert_eq!(second.completion, BrowseCompletion::Partial);
        assert!(second.known_count.is_none());

        let third = service
            .next_page(
                &session.session_id,
                second
                    .next_cursor
                    .as_deref()
                    .expect("second filtered cursor"),
                1,
            )
            .expect("filtered completion page");
        assert!(third.entries.is_empty());
        assert_eq!(third.completion, BrowseCompletion::Complete);
        assert_eq!(third.known_count, Some(2));
    }

    #[test]
    fn query_entry_kind_filters_direct_children_without_recursing() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("file.txt"), b"file").expect("file");
        fs::create_dir(fixture.root.join("folder")).expect("folder");
        let service = service(BrowseLimits::default());
        let session = service.start_session(fixture.directory()).expect("session");
        let page = service
            .start_enumeration_with_query(
                &session.session_id,
                "query-directory",
                &session.root_path_ref,
                8,
                BrowseQuerySpecV1 {
                    text: None,
                    entry_kind: BrowseQueryEntryKind::Directory,
                    sort: None,
                },
            )
            .expect("directory page");
        assert_eq!(page.completion, BrowseCompletion::Complete);
        assert_eq!(page.known_count, Some(1));
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].kind, BrowseEntryKind::Directory);
    }

    #[test]
    fn first_page_is_available_without_full_enumeration() {
        let fixture = Fixture::new();
        create_fixture_files(&fixture, 512);
        let service = service(BrowseLimits {
            max_sessions: 2,
            max_page_size: 8,
            max_path_refs: 16,
            max_entry_refs: 16,
            max_process_path_refs: 16,
            max_process_entry_refs: 16,
        });
        let session = service.start_session(fixture.directory()).expect("session");
        let page = service
            .start_enumeration(
                &session.session_id,
                "request-large",
                &session.root_path_ref,
                8,
            )
            .expect("page");
        assert_eq!(page.entries.len(), 8);
        assert_eq!(page.completion, BrowseCompletion::Partial);
        assert!(page.next_cursor.is_some());
        assert!(page.known_count.is_none());
    }

    #[test]
    fn impossible_query_returns_bounded_empty_partial_before_eof() {
        let fixture = Fixture::new();
        create_fixture_files(&fixture, 100_000);
        let service = service(BrowseLimits::default());
        let session = service.start_session(fixture.directory()).expect("session");
        let page = service
            .start_enumeration_with_query(
                &session.session_id,
                "query-impossible",
                &session.root_path_ref,
                32,
                BrowseQuerySpecV1 {
                    text: Some("query-that-cannot-match".to_string()),
                    entry_kind: BrowseQueryEntryKind::All,
                    sort: None,
                },
            )
            .expect("bounded query page");

        assert!(page.entries.is_empty());
        assert_eq!(page.completion, BrowseCompletion::Partial);
        assert!(page.next_cursor.is_some());
        assert!(page.known_count.is_none());
    }

    #[test]
    fn late_sentinel_publishes_across_bounded_turns_and_finishes_exactly() {
        let fixture = Fixture::new();
        create_fixture_files(&fixture, 100_000);
        fs::write(fixture.root.join("late-sentinel.txt"), b"sentinel").expect("sentinel");
        let service = service(BrowseLimits::default());
        let session = service.start_session(fixture.directory()).expect("session");
        let query = BrowseQuerySpecV1 {
            text: Some("late-sentinel".to_string()),
            entry_kind: BrowseQueryEntryKind::All,
            sort: None,
        };

        let mut page = service
            .start_enumeration_with_query(
                &session.session_id,
                "query-late-sentinel",
                &session.root_path_ref,
                8,
                query,
            )
            .expect("first bounded query page");
        assert_eq!(page.completion, BrowseCompletion::Partial);
        assert!(page.next_cursor.is_some());
        assert!(page.known_count.is_none());

        let mut turns = 1;
        let mut found = page
            .entries
            .iter()
            .any(|entry| entry.name == "late-sentinel.txt");
        while page.completion != BrowseCompletion::Complete {
            let cursor = page.next_cursor.as_deref().expect("live cursor");
            page = service
                .next_page(&session.session_id, cursor, 8)
                .expect("bounded continuation");
            turns += 1;
            found |= page
                .entries
                .iter()
                .any(|entry| entry.name == "late-sentinel.txt");
            assert!(turns <= 200, "bounded query took too many turns: {turns}");
        }

        assert!(turns > 1);
        assert!(found);
        assert!(page.entries.len() <= 1);
        assert_eq!(page.known_count, Some(1));
    }

    #[test]
    fn cursor_is_bound_to_session_and_enumeration() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("a.txt"), b"a").expect("a");
        fs::write(fixture.root.join("b.txt"), b"b").expect("b");
        let service = service(BrowseLimits::default());
        let first = service
            .start_session(fixture.directory())
            .expect("first session");
        let second = service
            .start_session(fixture.directory())
            .expect("second session");
        let page = service
            .start_enumeration(&first.session_id, "request-a", &first.root_path_ref, 1)
            .expect("first page");
        let cursor = page.next_cursor.expect("cursor");
        assert_eq!(
            service.next_page(&second.session_id, &cursor, 1),
            Err(BrowseError::StaleEnumeration)
        );
        assert!(service.next_page(&first.session_id, &cursor, 1).is_ok());
    }

    #[test]
    fn reenumeration_invalidates_old_page_and_entry_refs() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("a.txt"), b"a").expect("a");
        fs::write(fixture.root.join("b.txt"), b"b").expect("b");
        let service = service(BrowseLimits::default());
        let session = service.start_session(fixture.directory()).expect("session");
        let old = service
            .start_enumeration(&session.session_id, "old", &session.root_path_ref, 1)
            .expect("old page");
        let old_ref = old.entries[0].entry_ref.clone();
        let fresh = service
            .start_enumeration(&session.session_id, "fresh", &session.root_path_ref, 1)
            .expect("fresh page");
        assert_eq!(
            service.validate_page(&old),
            Err(BrowseError::StaleEnumeration)
        );
        assert_eq!(
            service.resolve_entry(&old_ref),
            Err(BrowseError::InvalidEntryRef)
        );
        assert_eq!(
            service.resolve_entry_generation(&old_ref),
            Err(BrowseError::InvalidEntryRef)
        );
        service.validate_page(&fresh).expect("fresh current");
        assert_eq!(
            service.resolve_entry_generation(&fresh.entries[0].entry_ref),
            Ok(fresh.enumeration_id.clone())
        );
    }

    #[test]
    fn superseded_slow_page_cannot_publish() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("a.txt"), b"a").expect("a");
        fs::write(fixture.root.join("b.txt"), b"b").expect("b");
        let service = Arc::new(service(BrowseLimits::default()));
        let session = service.start_session(fixture.directory()).expect("session");
        let gate = Arc::new(TestPublishGate::default());
        service.set_test_publish_gate(Arc::clone(&gate));
        let worker_service = Arc::clone(&service);
        let session_id = session.session_id.clone();
        let root_ref = session.root_path_ref.clone();
        let worker = thread::spawn(move || {
            worker_service.start_enumeration(&session_id, "old", &root_ref, 1)
        });
        gate.wait_until_reached();
        let fresh = service
            .start_enumeration(&session.session_id, "fresh", &session.root_path_ref, 1)
            .expect("fresh page");
        gate.release();
        assert_eq!(
            worker.join().expect("worker join"),
            Err(BrowseError::StalePublication)
        );
        service.validate_page(&fresh).expect("fresh current");
    }

    #[test]
    fn superseded_query_a_continuation_cannot_publish_after_query_b() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("alpha.txt"), b"alpha").expect("alpha");
        fs::write(fixture.root.join("beta.txt"), b"beta").expect("beta");
        let service = Arc::new(service(BrowseLimits::default()));
        let session = service.start_session(fixture.directory()).expect("session");
        let gate = Arc::new(TestPublishGate::default());
        service.set_test_publish_gate(Arc::clone(&gate));
        let worker_service = Arc::clone(&service);
        let session_id = session.session_id.clone();
        let root_ref = session.root_path_ref.clone();
        let worker = thread::spawn(move || {
            worker_service.start_enumeration_with_query(
                &session_id,
                "query-a",
                &root_ref,
                1,
                BrowseQuerySpecV1 {
                    text: Some("alpha".to_string()),
                    entry_kind: BrowseQueryEntryKind::All,
                    sort: None,
                },
            )
        });
        gate.wait_until_reached();
        let fresh = service
            .start_enumeration_with_query(
                &session.session_id,
                "query-b",
                &session.root_path_ref,
                1,
                BrowseQuerySpecV1 {
                    text: Some("beta".to_string()),
                    entry_kind: BrowseQueryEntryKind::All,
                    sort: None,
                },
            )
            .expect("query B page");
        gate.release();
        assert_eq!(
            worker.join().expect("worker join"),
            Err(BrowseError::StalePublication)
        );
        assert_eq!(fresh.entries.len(), 1);
        assert_eq!(fresh.entries[0].name, "beta.txt");
        service.validate_page(&fresh).expect("query B current");
    }

    #[test]
    fn entry_capacity_backpressures_until_page_release() {
        let fixture = Fixture::new();
        for name in ["a.txt", "b.txt", "c.txt"] {
            fs::write(fixture.root.join(name), name).expect("file");
        }
        let service = service(BrowseLimits {
            max_sessions: 1,
            max_page_size: 1,
            max_path_refs: 4,
            max_entry_refs: 1,
            max_process_path_refs: 4,
            max_process_entry_refs: 1,
        });
        let session = service.start_session(fixture.directory()).expect("session");
        let first = service
            .start_enumeration(&session.session_id, "bounded", &session.root_path_ref, 1)
            .expect("first page");
        let first_ref = first.entries[0].entry_ref.clone();
        let cursor = first.next_cursor.clone().expect("cursor");
        assert_eq!(
            service.next_page(&session.session_id, &cursor, 1),
            Err(BrowseError::TemporaryStateCapacityExceeded)
        );
        assert!(service.resolve_entry(&first_ref).is_ok());
        service.release_page(&first).expect("release first");
        assert_eq!(
            service.resolve_entry(&first_ref),
            Err(BrowseError::InvalidEntryRef)
        );
        let second = service
            .next_page(&session.session_id, &cursor, 1)
            .expect("retry after release");
        assert_eq!(second.entries.len(), 1);
        let second_cursor = second.next_cursor.clone().expect("second cursor");
        service.release_page(&second).expect("release second");
        let third = service
            .next_page(&session.session_id, &second_cursor, 1)
            .expect("buffered page survives retry");
        assert_eq!(third.entries.len(), 1);
        assert_eq!(third.completion, BrowseCompletion::Partial);
        assert!(third.known_count.is_none());
        let fourth = service
            .next_page(
                &session.session_id,
                third.next_cursor.as_deref().expect("third cursor"),
                1,
            )
            .expect("final empty page");
        assert!(fourth.entries.is_empty());
        assert_eq!(fourth.completion, BrowseCompletion::Complete);
        assert_eq!(fourth.known_count, Some(3));
        assert_eq!(service.state_counts(&session.session_id).unwrap().1, 1);
    }

    #[test]
    fn directory_path_refs_backpressure_and_release_with_page() {
        let fixture = Fixture::new();
        fs::create_dir(fixture.root.join("a-dir")).expect("a dir");
        fs::create_dir(fixture.root.join("b-dir")).expect("b dir");
        let service = service(BrowseLimits {
            max_sessions: 1,
            max_page_size: 1,
            max_path_refs: 2,
            max_entry_refs: 4,
            max_process_path_refs: 2,
            max_process_entry_refs: 4,
        });
        let session = service.start_session(fixture.directory()).expect("session");
        let first = service
            .start_enumeration(&session.session_id, "paths", &session.root_path_ref, 1)
            .expect("first page");
        let first_path = first.entries[0]
            .path_ref
            .clone()
            .expect("directory path ref");
        let cursor = first.next_cursor.clone().expect("cursor");
        assert_eq!(service.state_counts(&session.session_id).unwrap().0, 2);
        assert_eq!(
            service.next_page(&session.session_id, &cursor, 1),
            Err(BrowseError::TemporaryStateCapacityExceeded)
        );
        assert_eq!(service.state_counts(&session.session_id).unwrap().0, 2);
        service.release_page(&first).expect("release first");
        assert_eq!(service.state_counts(&session.session_id).unwrap().0, 1);
        assert_eq!(
            service.start_enumeration(&session.session_id, "released", &first_path, 1),
            Err(BrowseError::InvalidPathRef)
        );
        let second = service
            .next_page(&session.session_id, &cursor, 1)
            .expect("second after release");
        assert!(second.entries[0].path_ref.is_some());
    }

    #[test]
    fn process_aggregate_entry_refs_backpressure_and_release() {
        let fixture = Fixture::new();
        for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
            fs::write(fixture.root.join(name), name).expect("file");
        }
        let service = service(BrowseLimits {
            max_sessions: 2,
            max_page_size: 2,
            max_path_refs: 4,
            max_entry_refs: 4,
            max_process_path_refs: 4,
            max_process_entry_refs: 4,
        });
        let first_session = service
            .start_session(fixture.directory())
            .expect("first session");
        let second_session = service
            .start_session(fixture.directory())
            .expect("second session");
        let first_page = service
            .start_enumeration(
                &first_session.session_id,
                "aggregate-first",
                &first_session.root_path_ref,
                2,
            )
            .expect("first aggregate page");
        let first_cursor = first_page.next_cursor.clone().expect("first cursor");
        let second_page = service
            .next_page(&first_session.session_id, &first_cursor, 2)
            .expect("second aggregate page");
        assert!(!second_page.entries.is_empty());
        let retained_entry = first_page.entries[0].entry_ref.clone();
        assert!(service.resolve_entry(&retained_entry).is_ok());
        assert_eq!(
            service.start_enumeration(
                &second_session.session_id,
                "aggregate-overflow",
                &second_session.root_path_ref,
                2,
            ),
            Err(BrowseError::TemporaryStateCapacityExceeded)
        );
        assert_eq!(
            service
                .state_counts(&second_session.session_id)
                .expect("second counts"),
            (1, 0)
        );
        service
            .dispose_session(&first_session.session_id)
            .expect("release first aggregate session");
        let retry = service
            .start_enumeration(
                &second_session.session_id,
                "aggregate-retry",
                &second_session.root_path_ref,
                2,
            )
            .expect("aggregate capacity recovers after dispose");
        assert!(!retry.entries.is_empty());
        assert_eq!(
            service.resolve_entry(&retained_entry),
            Err(BrowseError::SessionNotFound)
        );
        service
            .dispose_session(&second_session.session_id)
            .expect("dispose second aggregate session");
        let counts = service.resource_counts();
        assert_eq!(counts.sessions, 0);
        assert_eq!(counts.entry_refs, 0);
        assert_eq!(counts.path_refs, 0);
    }

    #[test]
    fn process_aggregate_path_refs_backpressure_and_release() {
        let fixture = Fixture::new();
        for name in ["a-dir", "b-dir", "c-dir"] {
            fs::create_dir(fixture.root.join(name)).expect("directory");
        }
        let service = service(BrowseLimits {
            max_sessions: 2,
            max_page_size: 1,
            max_path_refs: 3,
            max_entry_refs: 8,
            max_process_path_refs: 3,
            max_process_entry_refs: 8,
        });
        let first_session = service
            .start_session(fixture.directory())
            .expect("first session");
        let second_session = service
            .start_session(fixture.directory())
            .expect("second session");
        let first_page = service
            .start_enumeration(
                &first_session.session_id,
                "path-aggregate-first",
                &first_session.root_path_ref,
                1,
            )
            .expect("first path aggregate page");
        assert_eq!(
            service.state_counts(&first_session.session_id).unwrap().0,
            2
        );
        assert_eq!(
            service.start_enumeration(
                &second_session.session_id,
                "path-aggregate-overflow",
                &second_session.root_path_ref,
                1,
            ),
            Err(BrowseError::TemporaryStateCapacityExceeded)
        );
        service
            .dispose_session(&first_session.session_id)
            .expect("release first path aggregate session");
        let retry = service
            .start_enumeration(
                &second_session.session_id,
                "path-aggregate-retry",
                &second_session.root_path_ref,
                1,
            )
            .expect("path aggregate capacity recovers after dispose");
        assert!(retry.entries[0].path_ref.is_some());
        service
            .dispose_session(&second_session.session_id)
            .expect("dispose second path aggregate session");
        let counts = service.resource_counts();
        assert_eq!(counts.sessions, 0);
        assert_eq!(counts.entry_refs, 0);
        assert_eq!(counts.path_refs, 0);
        drop(first_page);
    }

    #[test]
    fn process_aggregate_limits_cannot_be_smaller_than_session_limits() {
        let limits = BrowseLimits {
            max_process_path_refs: 1,
            ..BrowseLimits::default()
        };
        assert!(matches!(
            BrowseService::new(limits),
            Err(BrowseError::InvalidLimits)
        ));
    }

    #[test]
    fn promoted_directory_path_survives_page_release_until_unpinned() {
        let fixture = Fixture::new();
        let nested = fixture.root.join("nested");
        fs::create_dir(&nested).expect("nested");
        fs::write(nested.join("child.txt"), b"child").expect("child");
        let service = service(BrowseLimits::default());
        let session = service.start_session(fixture.directory()).expect("session");
        let page = service
            .start_enumeration(&session.session_id, "root", &session.root_path_ref, 8)
            .expect("root page");
        let nested_ref = page.entries[0].path_ref.clone().expect("nested ref");
        let child = service
            .start_enumeration(&session.session_id, "child", &nested_ref, 8)
            .expect("child page");
        service.release_page(&page).expect("release parent page");
        assert_eq!(child.entries[0].name, "child.txt");
        service
            .release_path_ref(&session.session_id, &nested_ref)
            .expect("release history pin");
        assert_eq!(
            service.start_enumeration(&session.session_id, "again", &nested_ref, 8),
            Err(BrowseError::InvalidPathRef)
        );
    }

    #[test]
    fn cancellation_and_dispose_revoke_temporary_state() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("a.txt"), b"a").expect("a");
        fs::write(fixture.root.join("b.txt"), b"b").expect("b");
        let service = service(BrowseLimits::default());
        let session = service.start_session(fixture.directory()).expect("session");
        let page = service
            .start_enumeration(&session.session_id, "cancel", &session.root_path_ref, 1)
            .expect("page");
        let identity = BrowseEnumerationRef {
            session_id: page.session_id.clone(),
            request_id: page.request_id.clone(),
            enumeration_id: page.enumeration_id.clone(),
        };
        let entry_ref = page.entries[0].entry_ref.clone();
        service
            .cancel(&session.session_id, &identity)
            .expect("cancel");
        assert_eq!(
            service.resolve_entry(&entry_ref),
            Err(BrowseError::InvalidEntryRef)
        );
        assert_eq!(
            service.resolve_entry_generation(&entry_ref),
            Err(BrowseError::InvalidEntryRef)
        );
        service
            .dispose_session(&session.session_id)
            .expect("dispose");
        assert_eq!(
            service.state_counts(&session.session_id),
            Err(BrowseError::SessionNotFound)
        );
    }

    #[test]
    fn disappearing_and_unsupported_children_do_not_abort_siblings() {
        let fixture = Fixture::new();
        let disappearing = fixture.root.join("disappearing.txt");
        let visible = fixture.root.join("visible.txt");
        fs::write(&disappearing, b"gone").expect("disappearing");
        fs::write(&visible, b"visible").expect("visible");
        let source = fs::read_dir(&fixture.root).expect("source");
        let disappearing_entry = fs::read_dir(&fixture.root)
            .expect("entries")
            .filter_map(Result::ok)
            .find(|entry| entry.file_name().to_string_lossy() == "disappearing.txt")
            .expect("disappearing entry");
        fs::remove_file(&disappearing).expect("remove disappearing");
        assert_eq!(
            read_entry(disappearing_entry).unwrap_err(),
            BrowseError::EntryNotFound
        );
        let enumeration = EnumerationState::new(
            BrowseEnumerationRef {
                session_id: "session".into(),
                request_id: "request".into(),
                enumeration_id: "enumeration".into(),
            },
            source,
        );
        let (entries, complete) = read_entries(&enumeration, 8).expect("read siblings");
        assert!(complete);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "visible.txt");
        assert_eq!(
            map_entry_error(io::Error::from(ErrorKind::PermissionDenied)),
            BrowseError::EntryPermissionDenied
        );
    }

    #[test]
    fn unsupported_symlink_does_not_abort_sibling_enumeration() {
        let fixture = Fixture::new();
        let target = fixture.root.join("target.txt");
        fs::write(&target, b"target").expect("target");
        fs::write(fixture.root.join("visible.txt"), b"visible").expect("visible");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, fixture.root.join("link.txt")).expect("symlink");
        #[cfg(windows)]
        if std::os::windows::fs::symlink_file(&target, fixture.root.join("link.txt")).is_err() {
            return;
        }
        #[cfg(not(any(unix, windows)))]
        return;

        let service = service(BrowseLimits::default());
        let session = service.start_session(fixture.directory()).expect("session");
        let page = service
            .start_enumeration(&session.session_id, "links", &session.root_path_ref, 8)
            .expect("page");
        assert!(page.entries.iter().any(|entry| entry.name == "target.txt"));
        assert!(page.entries.iter().any(|entry| entry.name == "visible.txt"));
        assert!(page.entries.iter().all(|entry| entry.name != "link.txt"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_reparse_tags_distinguish_links_from_provider_entries() {
        use windows_sys::Win32::System::SystemServices::{
            IO_REPARSE_TAG_CLOUD, IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK,
        };
        assert_eq!(
            classify_windows_reparse_tag(IO_REPARSE_TAG_SYMLINK),
            LinkLike::Unsafe
        );
        assert_eq!(
            classify_windows_reparse_tag(IO_REPARSE_TAG_MOUNT_POINT),
            LinkLike::Unsafe
        );
        assert_eq!(
            classify_windows_reparse_tag(IO_REPARSE_TAG_CLOUD),
            LinkLike::ProviderOrOther
        );
    }

    #[test]
    fn opaque_refs_never_serialize_authoritative_paths() {
        let root = PathBuf::from("C:\\private\\browse-root");
        let entry_ref = EntryRef::Ephemeral {
            browse_session_id: "session-1".to_string(),
            entry_id: "entry-1".to_string(),
        };
        let location = LocationRef::Ephemeral {
            browse_session_id: "session-1".to_string(),
            location_id: "location-1".to_string(),
        };
        let path_ref = BrowsePathRef {
            id: "path-1".to_string(),
        };
        for value in [
            serde_json::to_value(entry_ref).expect("entry json"),
            serde_json::to_value(location).expect("location json"),
            serde_json::to_value(path_ref).expect("path json"),
        ] {
            assert!(value.get("path").is_none());
            assert!(!value.to_string().contains(root.to_string_lossy().as_ref()));
        }
        let page_shape = json!({
            "sessionId": "session-1",
            "requestId": "request-1",
            "enumerationId": "enum-1",
            "nextCursor": "opaque-cursor"
        });
        assert!(page_shape.get("path").is_none());
    }
}
