use super::{
    runtime::{BrowseRecord, FileWorkspaceRuntime, MAX_BROWSE_SESSIONS},
    types::{
        BrowseCancelRequest, BrowseNextPageRequest, BrowseOpenRequest, BrowseOpenResponse,
        BrowsePageDto, BrowseReleasePageRequest, BrowseReleasePathRequest, BrowseRestoreRequest,
        BrowseRetainPathRequest, BrowseSessionRequest, BrowseStartEnumerationRequest,
    },
};
use crate::file_workspace::{
    browse::{BackendResolvedDirectory, BrowseError, BrowseSessionInfo},
    contracts::WorkspacePlatform,
    location::{
        project_ephemeral_location, project_managed_scan_root, EphemeralLocationLifecycle,
        EphemeralLocationProjectionInput, LocationRuntimeEvidence,
    },
};
use std::path::PathBuf;

impl FileWorkspaceRuntime {
    /// Read-only OS Reveal: the owning session resolves the path, never the renderer.
    pub(crate) fn resolve_browse_reveal_path(
        &self,
        request: super::types::BrowseRevealRequest,
    ) -> Result<PathBuf, String> {
        self.ensure_live()?;
        let entry = self
            .inner
            .browse
            .resolve_entry(&request.entry_ref)
            .map_err(map_browse_error)?;
        std::fs::symlink_metadata(&entry.path)
            .map_err(|_| "workspace_reveal_entry_unavailable".to_string())?;
        Ok(entry.path)
    }

    pub(crate) fn open_browse(
        &self,
        request: BrowseOpenRequest,
    ) -> Result<BrowseOpenResponse, String> {
        self.ensure_live()?;
        validate_platform(request.platform)?;
        if let Some(entry_id) = &request.global_entry_id {
            return self.open_global_search_parent(entry_id, request.platform);
        }
        let routing_hint = validate_routing_hint(&request.routing_hint)?;
        let directory =
            BackendResolvedDirectory::from_backend_path(routing_hint).map_err(map_browse_error)?;
        let display_name = request
            .display_hint
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Browse".to_string());

        let info = self
            .inner
            .browse
            .start_session(directory)
            .map_err(map_browse_error)?;
        self.publish_browse_admission(
            info,
            display_name,
            LocationRuntimeEvidence::browse_admitted(),
        )
    }

    fn open_global_search_parent(
        &self,
        entry_id: &str,
        platform: WorkspacePlatform,
    ) -> Result<BrowseOpenResponse, String> {
        use crate::file_workspace::browse::BrowseQuerySpecV1;
        use crate::fs_safety::capture_source_object_identity;
        use crate::global_index::commands::revalidated_global_entry;
        let entry = revalidated_global_entry(&self.inner.database, entry_id)?;
        let path = PathBuf::from(&entry.path);
        let evidence = capture_source_object_identity(&path)
            .map_err(|_| "global_search_result_identity_changed".to_string())?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| "global_search_parent_unavailable".to_string())?;
        let mut response = self.open_browse(BrowseOpenRequest {
            platform,
            routing_hint: parent.to_string_lossy().into_owned(),
            display_hint: Some(
                parent
                    .file_name()
                    .unwrap_or(parent.as_os_str())
                    .to_string_lossy()
                    .into_owned(),
            ),
            global_entry_id: None,
        })?;
        let admitted = (|| {
            // Probe directory readability before publishing any frontend state.
            let page = self
                .inner
                .browse
                .start_enumeration_with_query(
                    &response.session_id,
                    format!("search-handoff-{}", uuid::Uuid::new_v4()),
                    &response.root_path_ref,
                    32,
                    BrowseQuerySpecV1::default(),
                )
                .map_err(map_browse_error)?;
            let current = revalidated_global_entry(&self.inner.database, entry_id)?;
            let actual = capture_source_object_identity(&path)
                .map_err(|_| "global_search_result_identity_changed".to_string())?;
            if current.path != entry.path
                || current.platform_file_id != entry.platform_file_id
                || actual.physical_key.is_none()
                || actual != evidence
            {
                return Err("global_search_result_identity_changed".to_string());
            }
            // A path-only index identity cannot prove the indexed object is the
            // same object. Parent-only navigation is intentional in that case.
            if !entry.platform_file_id.starts_with("path:") {
                response.focus_entry_ref = page.entries.iter().find_map(|candidate| {
                    let resolved = self.inner.browse.resolve_entry(&candidate.entry_ref).ok()?;
                    if resolved.path != path {
                        return None;
                    }
                    let fresh = capture_source_object_identity(&resolved.path).ok()?;
                    (fresh == evidence).then(|| candidate.entry_ref.clone())
                });
            }
            response.initial_page = Some(BrowsePageDto::from_internal(page));
            Ok(())
        })();
        if let Err(error) = admitted {
            let _ = self.dispose_browse(BrowseSessionRequest {
                session_id: response.session_id,
            });
            return Err(error);
        }
        Ok(response)
    }

    pub(crate) fn restore_browse(
        &self,
        request: BrowseRestoreRequest,
    ) -> Result<BrowseOpenResponse, String> {
        let locator = request.locator;
        match locator {
            crate::file_workspace::WorkspaceRestoreLocator::Browse {
                platform,
                routing_hint,
                display_hint,
            } => self.open_browse(BrowseOpenRequest {
                global_entry_id: None,
                platform,
                routing_hint,
                display_hint,
            }),
            crate::file_workspace::WorkspaceRestoreLocator::Library { .. } => {
                Err("workspace_restore_requires_browse_locator".to_string())
            }
        }
    }

    pub(crate) fn start_enumeration(
        &self,
        request: BrowseStartEnumerationRequest,
    ) -> Result<BrowsePageDto, String> {
        self.ensure_live()?;
        let page = self
            .inner
            .browse
            .start_enumeration_with_query(
                &request.session_id,
                request.request_id,
                &request.path_ref,
                request.page_size,
                request.query,
            )
            .map_err(map_browse_error)?;
        Ok(BrowsePageDto::from_internal(page))
    }

    pub(crate) fn next_page(
        &self,
        request: BrowseNextPageRequest,
    ) -> Result<BrowsePageDto, String> {
        self.ensure_live()?;
        let page = self
            .inner
            .browse
            .next_page(&request.session_id, &request.cursor, request.page_size)
            .map_err(map_browse_error)?;
        Ok(BrowsePageDto::from_internal(page))
    }

    pub(crate) fn cancel_enumeration(&self, request: BrowseCancelRequest) -> Result<(), String> {
        self.ensure_live()?;
        match (request.enumeration, request.request_id) {
            (Some(enumeration), None) => self
                .inner
                .browse
                .cancel(&request.session_id, &enumeration)
                .map_err(map_browse_error),
            (None, Some(request_id)) if !request_id.is_empty() => self
                .inner
                .browse
                .cancel_request(&request.session_id, &request_id)
                .map_err(map_browse_error),
            _ => Err("browse_cancel_requires_exactly_one_identity".to_string()),
        }
    }

    pub(crate) fn release_page(&self, request: BrowseReleasePageRequest) -> Result<(), String> {
        self.ensure_live()?;
        self.inner
            .browse
            .release_page(&request.page.into_internal())
            .map_err(map_browse_error)
    }

    pub(crate) fn release_path(&self, request: BrowseReleasePathRequest) -> Result<(), String> {
        self.ensure_live()?;
        self.inner
            .browse
            .release_path_ref(&request.session_id, &request.path_ref)
            .map_err(map_browse_error)
    }

    pub(crate) fn retain_path(&self, request: BrowseRetainPathRequest) -> Result<(), String> {
        self.ensure_live()?;
        self.inner
            .browse
            .retain_path_ref(&request.session_id, &request.path_ref)
            .map_err(map_browse_error)
    }

    pub(crate) fn dispose_browse(&self, request: BrowseSessionRequest) -> Result<(), String> {
        self.ensure_live()?;
        self.dispose_monitors_for_session(&request.session_id);
        let mut first_error = self.dispose_previews_for_session(&request.session_id);
        if let Err(error) = self
            .inner
            .sessions
            .lock()
            .map(|mut sessions| {
                sessions
                    .remove(&request.session_id)
                    .ok_or_else(|| "browse_session_not_found".to_string())
            })
            .map_err(|_| "workspace_session_state_unavailable".to_string())
            .and_then(|result| result)
        {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
        if let Err(error) = self
            .inner
            .browse
            .dispose_session(&request.session_id)
            .map_err(map_browse_error)
        {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub(crate) fn list_locations(
        &self,
    ) -> Result<Vec<crate::file_workspace::LocationDescriptor>, String> {
        self.ensure_live()?;
        let roots = self
            .inner
            .database
            .list_scan_roots()
            .map_err(|_| "workspace_location_list_unavailable".to_string())?;
        let mut descriptors = roots
            .iter()
            .filter_map(|root| {
                project_managed_scan_root(root, &LocationRuntimeEvidence::unknown()).ok()
            })
            .collect::<Vec<_>>();
        let ephemeral = self
            .inner
            .sessions
            .lock()
            .map_err(|_| "workspace_session_state_unavailable".to_string())?
            .values()
            .map(|record| {
                project_ephemeral_location(EphemeralLocationProjectionInput {
                    location_ref: record.info.location.clone(),
                    display_name: record.display_name.clone(),
                    runtime: LocationRuntimeEvidence::browse_admitted(),
                    lifecycle: EphemeralLocationLifecycle::Active,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "workspace_location_projection_failed".to_string())?;
        descriptors.extend(ephemeral);
        Ok(descriptors)
    }

    pub(crate) fn read_eligibility(
        &self,
        request: super::types::ReadEligibilityRequest,
    ) -> Result<super::types::ReadEligibilityResponse, String> {
        self.ensure_live()?;
        let eligibility = self
            .inner
            .read_gate
            .content_read_eligibility(&request.source);
        Ok(super::types::ReadEligibilityResponse {
            source: request.source,
            eligibility,
        })
    }

    /// Publish one already-admitted Browse session into the integration
    /// lifecycle registry. The registry owns only command-addressable handles;
    /// BrowseService remains the session/path authority.
    pub(crate) fn publish_browse_admission(
        &self,
        info: BrowseSessionInfo,
        display_name: String,
        runtime: LocationRuntimeEvidence,
    ) -> Result<BrowseOpenResponse, String> {
        let descriptor = match project_ephemeral_location(EphemeralLocationProjectionInput {
            location_ref: info.location.clone(),
            display_name: display_name.clone(),
            runtime,
            lifecycle: EphemeralLocationLifecycle::Active,
        }) {
            Ok(descriptor) => descriptor,
            Err(_) => {
                let _ = self.inner.browse.dispose_session(&info.session_id);
                return Err("workspace_location_projection_failed".to_string());
            }
        };

        let mut sessions = match self.inner.sessions.lock() {
            Ok(sessions) => sessions,
            Err(_) => {
                let _ = self.inner.browse.dispose_session(&info.session_id);
                return Err("workspace_session_state_unavailable".to_string());
            }
        };
        if sessions.len() >= MAX_BROWSE_SESSIONS {
            drop(sessions);
            let _ = self.inner.browse.dispose_session(&info.session_id);
            return Err("browse_session_capacity_exceeded".to_string());
        }
        let session_id = info.session_id.clone();
        let response = BrowseOpenResponse {
            focus_entry_ref: None,
            initial_page: None,
            session_id: session_id.clone(),
            location: descriptor,
            root_path_ref: info.root_path_ref.clone(),
        };
        sessions.insert(session_id, BrowseRecord { info, display_name });
        Ok(response)
    }
}

impl FileWorkspaceRuntime {
    pub(crate) fn has_browse_session(&self, session_id: &str) -> bool {
        self.inner
            .sessions
            .lock()
            .map(|sessions| sessions.contains_key(session_id))
            .unwrap_or(false)
    }

    pub(crate) fn dispose_monitors_for_session(&self, session_id: &str) {
        let monitors = {
            let Ok(mut records) = self.inner.monitors.lock() else {
                return;
            };
            let ids = records
                .iter()
                .filter(|(_, record)| record.session_id == session_id)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| records.remove(&id))
                .map(|record| record.monitor)
                .collect::<Vec<_>>()
        };
        for monitor in monitors {
            monitor.dispose();
        }
    }

    pub(crate) fn dispose_previews_for_session(&self, session_id: &str) -> Option<String> {
        let previews = {
            let Ok(mut records) = self.inner.preview_sessions.lock() else {
                return Some("workspace_preview_state_unavailable".to_string());
            };
            let ids = records
                .iter()
                .filter(|(_, session)| {
                    matches!(
                        session.request().source,
                        crate::file_workspace::PreviewSourceRef::Ephemeral {
                            ref browse_session_id,
                            ..
                        } if browse_session_id == session_id
                    )
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| records.remove(&id).map(|session| (id, session)))
                .collect::<Vec<_>>()
        };
        #[cfg(target_os = "macos")]
        let mut first_error = None;
        #[cfg(not(target_os = "macos"))]
        let first_error: Option<String> = None;
        for (preview_id, session) in previews {
            #[cfg(target_os = "macos")]
            let superseded = super::types::PreviewSnapshotDto::from_internal(
                preview_id.clone(),
                session.snapshot(),
            );
            session.dispose();
            #[cfg(target_os = "macos")]
            let detach_error = self
                .inner
                .native_preview_host
                .detach(&preview_id, Some(&superseded))
                .err();
            self.inner.native_preview_access.revoke_session(&preview_id);
            self.inner.preview_assets.revoke_session(&preview_id);
            #[cfg(target_os = "macos")]
            if let Some(error) = detach_error {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        first_error
    }
}

fn validate_platform(platform: WorkspacePlatform) -> Result<(), String> {
    let supported = if cfg!(target_os = "windows") {
        platform == WorkspacePlatform::Windows
    } else if cfg!(target_os = "macos") {
        platform == WorkspacePlatform::Macos
    } else {
        false
    };
    supported
        .then_some(())
        .ok_or_else(|| "workspace_platform_not_supported".to_string())
}

fn validate_routing_hint(value: &str) -> Result<PathBuf, String> {
    if !super::types::valid_bounded_text(value) {
        return Err("browse_routing_hint_invalid".to_string());
    }
    Ok(PathBuf::from(value))
}

pub(super) fn map_browse_error(error: BrowseError) -> String {
    error.to_string()
}
