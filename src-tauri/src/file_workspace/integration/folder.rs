use super::preview::WorkspacePreviewResolver;
use crate::{
    file_workspace::{
        browse::{BrowseCompletion, BrowseError, BrowsePage, BrowseService},
        contracts::PreviewSourceRef,
        preview::{
            PreviewContextError, PreviewFolderEntryFact, PreviewFolderEntryKind,
            PreviewFolderEnumerationAccess, PreviewFolderEnumerationError, PreviewFolderPage,
            PreviewFolderPageAction, PreviewOperationContext,
        },
    },
    scheduler::{adapters::FolderPreviewResourceLeaseAdapter, WorkScheduler},
};
use std::sync::Arc;

pub(crate) const FOLDER_BROWSE_PAGE_SIZE: usize = 256;

struct FolderPageRequest<'a> {
    source: &'a PreviewSourceRef,
    source_version: &'a str,
    context: &'a PreviewOperationContext,
    session_id: &'a str,
    root_path_ref: &'a crate::file_workspace::BrowsePathRef,
    request_id: &'a str,
}

pub(crate) struct FolderPreviewEnumerationAdapter {
    resolver: Arc<WorkspacePreviewResolver>,
    browse: Arc<BrowseService>,
    admission: FolderPreviewResourceLeaseAdapter,
    #[cfg(test)]
    test_gate: std::sync::Mutex<Option<Arc<FolderPreviewTestGate>>>,
}

impl FolderPreviewEnumerationAdapter {
    pub(crate) fn new(
        resolver: Arc<WorkspacePreviewResolver>,
        browse: Arc<BrowseService>,
        scheduler: Arc<WorkScheduler>,
    ) -> Self {
        Self {
            resolver,
            browse,
            admission: FolderPreviewResourceLeaseAdapter::new(scheduler),
            #[cfg(test)]
            test_gate: std::sync::Mutex::new(None),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_test_gate(&self, gate: Option<Arc<FolderPreviewTestGate>>) {
        *self.test_gate.lock().expect("folder preview test gate") = gate;
    }

    #[cfg(test)]
    fn pause_after_lease(&self) {
        if let Some(gate) = self
            .test_gate
            .lock()
            .expect("folder preview test gate")
            .clone()
        {
            gate.pause_after_lease();
        }
    }

    #[cfg(test)]
    fn pause_after_page(&self) {
        if let Some(gate) = self
            .test_gate
            .lock()
            .expect("folder preview test gate")
            .clone()
        {
            gate.pause_after_page();
        }
    }

    fn enumerate_pages(
        &self,
        request: FolderPageRequest<'_>,
        visit_page: &mut dyn FnMut(
            PreviewFolderPage,
        ) -> Result<
            PreviewFolderPageAction,
            PreviewFolderEnumerationError,
        >,
    ) -> Result<(), PreviewFolderEnumerationError> {
        ensure_page_budget(request.context)?;
        self.resolver.resolve_folder_directory(
            request.source,
            request.source_version,
            request.context,
        )?;
        let mut page = self
            .browse
            .start_enumeration(
                request.session_id,
                request.request_id,
                request.root_path_ref,
                FOLDER_BROWSE_PAGE_SIZE,
            )
            .map_err(map_browse_error)?;

        loop {
            if let Err(error) = ensure_page_budget(request.context) {
                let _ = self.browse.release_page(&page);
                return Err(error);
            }
            if let Err(error) = self.resolver.resolve_folder_directory(
                request.source,
                request.source_version,
                request.context,
            ) {
                let _ = self.browse.release_page(&page);
                return Err(error);
            }
            let complete = page.completion == BrowseCompletion::Complete;
            let facts = page_facts(&page);
            let action = visit_page(PreviewFolderPage {
                entries: facts,
                complete,
            });

            #[cfg(test)]
            self.pause_after_page();

            let release_result = self.browse.release_page(&page).map_err(map_browse_error);
            let action = match action {
                Ok(action) => action,
                Err(error) => {
                    let _ = release_result;
                    return Err(error);
                }
            };
            release_result?;
            if action == PreviewFolderPageAction::Stop || complete {
                return Ok(());
            }
            let cursor = page
                .next_cursor
                .clone()
                .ok_or(PreviewFolderEnumerationError::Failed)?;
            ensure_page_budget(request.context)?;
            self.resolver.resolve_folder_directory(
                request.source,
                request.source_version,
                request.context,
            )?;
            page = self
                .browse
                .next_page(request.session_id, &cursor, FOLDER_BROWSE_PAGE_SIZE)
                .map_err(map_browse_error)?;
        }
    }
}

impl PreviewFolderEnumerationAccess for FolderPreviewEnumerationAdapter {
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
    ) -> Result<(), PreviewFolderEnumerationError> {
        let directory = self
            .resolver
            .resolve_folder_directory(source, source_version, context)?;
        let temporary = self
            .browse
            .start_session(directory)
            .map_err(map_browse_error)?;
        let temporary_session = TemporaryBrowseSession {
            browse: Arc::clone(&self.browse),
            session_id: temporary.session_id.clone(),
        };
        let request_id = bounded_request_id(context.request_id());
        let lease = match self.admission.try_acquire(
            &request_id,
            context.session_id(),
            context.scheduler_cancellation(),
        ) {
            Ok(lease) => lease,
            Err(error) => {
                drop(temporary_session);
                return Err(map_admission_error(error, context));
            }
        };

        #[cfg(test)]
        self.pause_after_lease();

        let result = self.enumerate_pages(
            FolderPageRequest {
                source,
                source_version,
                context,
                session_id: &temporary.session_id,
                root_path_ref: &temporary.root_path_ref,
                request_id: &request_id,
            },
            visit_page,
        );
        drop(lease);
        drop(temporary_session);
        result
    }
}

struct TemporaryBrowseSession {
    browse: Arc<BrowseService>,
    session_id: String,
}

impl Drop for TemporaryBrowseSession {
    fn drop(&mut self) {
        let _ = self.browse.dispose_session(&self.session_id);
    }
}

fn page_facts(page: &BrowsePage) -> Vec<PreviewFolderEntryFact> {
    page.entries
        .iter()
        .map(|entry| PreviewFolderEntryFact {
            name: entry.name.clone(),
            kind: match entry.kind {
                crate::file_workspace::browse::BrowseEntryKind::File => {
                    PreviewFolderEntryKind::File
                }
                crate::file_workspace::browse::BrowseEntryKind::Directory => {
                    PreviewFolderEntryKind::Directory
                }
            },
            extension: entry.extension.clone(),
            size_bytes: entry.size,
        })
        .collect()
}

fn ensure_page_budget(
    context: &PreviewOperationContext,
) -> Result<(), PreviewFolderEnumerationError> {
    context.ensure_active().map_err(map_context_error)?;
    if context.remaining() <= crate::file_workspace::preview_folder::FOLDER_DEADLINE_RETURN_GUARD {
        return Err(PreviewFolderEnumerationError::Deadline);
    }
    Ok(())
}

fn bounded_request_id(value: &str) -> String {
    let prefix = "folder-preview-";
    let suffix = value
        .chars()
        .filter(|character| !character.is_control())
        .take(96)
        .collect::<String>();
    format!("{prefix}{suffix}")
}

fn map_context_error(error: PreviewContextError) -> PreviewFolderEnumerationError {
    match error {
        PreviewContextError::Cancelled | PreviewContextError::StalePublication => {
            PreviewFolderEnumerationError::Cancelled
        }
        PreviewContextError::TimedOut => PreviewFolderEnumerationError::Deadline,
    }
}

fn map_admission_error(
    error: crate::scheduler::AcquireError,
    context: &PreviewOperationContext,
) -> PreviewFolderEnumerationError {
    if context.cancellation().is_cancelled() {
        PreviewFolderEnumerationError::Cancelled
    } else {
        let _ = error;
        PreviewFolderEnumerationError::Failed
    }
}

fn map_browse_error(error: BrowseError) -> PreviewFolderEnumerationError {
    match error {
        BrowseError::DirectoryPermissionDenied | BrowseError::EntryPermissionDenied => {
            PreviewFolderEnumerationError::PermissionDenied
        }
        BrowseError::DirectoryNotFound
        | BrowseError::DirectoryUnavailable
        | BrowseError::SessionNotFound
        | BrowseError::InvalidLocationRef
        | BrowseError::InvalidPathRef
        | BrowseError::InvalidEntryRef
        | BrowseError::TargetNotDirectory => PreviewFolderEnumerationError::SourceUnavailable,
        BrowseError::Cancelled => PreviewFolderEnumerationError::Cancelled,
        BrowseError::StaleEnumeration | BrowseError::StalePublication => {
            PreviewFolderEnumerationError::IdentityChanged
        }
        BrowseError::UnsupportedEntry
        | BrowseError::EntryNotFound
        | BrowseError::EntryUnavailable
        | BrowseError::InvalidCursor
        | BrowseError::InvalidRequest
        | BrowseError::SortCapacityExceeded
        | BrowseError::InvalidPageSize
        | BrowseError::InvalidLimits
        | BrowseError::StateUnavailable
        | BrowseError::SessionCapacityExceeded
        | BrowseError::TemporaryStateCapacityExceeded => PreviewFolderEnumerationError::Failed,
    }
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct FolderPreviewTestGate {
    state: std::sync::Mutex<FolderPreviewTestGateState>,
    wake: std::sync::Condvar,
}

#[cfg(test)]
#[derive(Default)]
struct FolderPreviewTestGateState {
    lease_reached: bool,
    lease_release: bool,
    page_reached: bool,
    page_release: bool,
}

#[cfg(test)]
impl FolderPreviewTestGate {
    pub(crate) fn wait_for_lease(&self) {
        let mut state = self.state.lock().expect("folder gate lock");
        while !state.lease_reached {
            state = self.wake.wait(state).expect("folder gate wait");
        }
    }

    pub(crate) fn release_lease(&self) {
        let mut state = self.state.lock().expect("folder gate lock");
        state.lease_release = true;
        self.wake.notify_all();
    }

    pub(crate) fn wait_for_page(&self) {
        let mut state = self.state.lock().expect("folder gate lock");
        while !state.page_reached {
            state = self.wake.wait(state).expect("folder gate wait");
        }
    }

    pub(crate) fn release_page(&self) {
        let mut state = self.state.lock().expect("folder gate lock");
        state.page_release = true;
        self.wake.notify_all();
    }

    fn pause_after_lease(&self) {
        let mut state = self.state.lock().expect("folder gate lock");
        state.lease_reached = true;
        self.wake.notify_all();
        while !state.lease_release {
            state = self.wake.wait(state).expect("folder gate wait");
        }
    }

    fn pause_after_page(&self) {
        let mut state = self.state.lock().expect("folder gate lock");
        state.page_reached = true;
        self.wake.notify_all();
        while !state.page_release {
            state = self.wake.wait(state).expect("folder gate wait");
        }
    }
}
