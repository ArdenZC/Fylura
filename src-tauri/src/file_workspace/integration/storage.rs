//! Mounted-volume metadata and explicit, identity-bound OS safe removal.
//! This is not a transfer executor or a new persistent device registry.
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeState {
    pub identity: Option<String>,
    pub kind: &'static str,
    pub read_only: Option<bool>,
    pub available_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub can_eject: bool,
}

impl Default for VolumeState {
    fn default() -> Self {
        Self {
            identity: None,
            kind: "unknown",
            read_only: None,
            available_bytes: None,
            total_bytes: None,
            can_eject: false,
        }
    }
}

pub(crate) fn root_identity(path: &Path) -> Option<String> {
    crate::fs_safety::capture_source_object_identity(path)
        .ok()?
        .physical_key
}

pub(crate) fn inspect(path: &Path, removal: bool) -> VolumeState {
    let mut state = native::inspect(path, removal);
    state.identity = root_identity(path);
    state.can_eject &= state.identity.is_some();
    state
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EjectVolumeRequest {
    pub location_id: String,
    pub identity: String,
}

impl super::FileWorkspaceRuntime {
    pub(crate) fn eject_volume(&self, request: EjectVolumeRequest) -> Result<(), String> {
        self.ensure_live()?;
        if request.location_id.len() > 4096 || request.identity.len() > 512 {
            return Err("storage_device_changed".into());
        }
        let path = super::known_locations::mounted_volume_path(&request.location_id)
            .ok_or("storage_device_disconnected")?;
        let volume = inspect(&path, true);
        validate_eject(&volume, &request.identity)?;
        self.release_volume_readers(&path)?;
        native::eject(&path, &request.identity)
    }

    /// Release our own disposable directory/preview handles before asking the OS
    /// to remove a volume. Keep session roots and history intent for reconnect or
    /// a veto; do not close other processes' handles or force a dismount.
    pub(crate) fn release_volume_readers(&self, root: &Path) -> Result<(), String> {
        let volume = crate::fs_safety::capture_source_object_identity(root)
            .map_err(|_| "storage_device_disconnected")?
            .platform_volume_id
            .ok_or("storage_device_changed")?;
        let sessions = self
            .inner
            .sessions
            .lock()
            .map_err(|_| "storage_eject_busy")?
            .values()
            .map(|record| {
                (
                    record.info.session_id.clone(),
                    record.info.root_path_ref.clone(),
                )
            })
            .collect::<Vec<_>>();
        for (id, reference) in sessions {
            let Ok(directory) = self.inner.browse.resolve_watch_target(&id, &reference) else {
                continue;
            };
            let same_volume = crate::fs_safety::capture_source_object_identity(directory.as_path())
                .ok()
                .is_some_and(|identity| {
                    identity.platform_volume_id.as_deref() == Some(volume.as_str())
                });
            if same_volume {
                self.dispose_monitors_for_session(&id);
                if self.dispose_previews_for_session(&id).is_some() {
                    return Err("storage_eject_busy".into());
                }
                self.inner
                    .browse
                    .invalidate(&id)
                    .map_err(|_| "storage_eject_busy")?;
            }
        }
        Ok(())
    }
}

fn validate_eject(volume: &VolumeState, identity: &str) -> Result<(), String> {
    if volume.identity.as_deref() != Some(identity) || identity.is_empty() {
        return Err("storage_device_changed".into());
    }
    if !volume.can_eject {
        return Err("storage_eject_unsupported".into());
    }
    Ok(())
}

/// Advisory preflight only. OS writes and the existing executor remain the
/// authority when quota, connectivity or free space changes after preview.
pub(crate) fn check_destination(path: &Path, required_bytes: u64) -> Result<(), String> {
    check_capacity(&inspect(path, false), required_bytes)
}

pub(crate) fn check_capacity(volume: &VolumeState, required_bytes: u64) -> Result<(), String> {
    if volume.read_only == Some(true) {
        return Err("storage_volume_read_only".into());
    }
    if volume
        .available_bytes
        .is_some_and(|free| required_bytes > free)
    {
        return Err("storage_insufficient_space".into());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
#[path = "storage_windows.rs"]
mod native;
#[cfg(target_os = "macos")]
#[path = "storage_macos.rs"]
mod native;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod native {
    use super::*;
    pub(super) fn inspect(_: &Path, _: bool) -> VolumeState {
        VolumeState::default()
    }
    pub(super) fn eject(_: &Path, _: &str) -> Result<(), String> {
        Err("storage_eject_unsupported".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removal_rejects_rebound_or_non_ejectable_volumes() {
        let volume = VolumeState {
            identity: Some("original".into()),
            can_eject: true,
            ..VolumeState::default()
        };
        assert!(validate_eject(&volume, "replacement").is_err());
        assert!(validate_eject(&volume, "original").is_ok());
        assert!(validate_eject(
            &VolumeState {
                can_eject: false,
                ..volume
            },
            "original"
        )
        .is_err());
        assert!(serde_json::from_value::<EjectVolumeRequest>(
            serde_json::json!({"locationId":"volume:x", "identity":"x", "path":"/forbidden"})
        )
        .is_err());
    }
    #[test]
    fn capacity_preflight_preserves_unknown_and_rejects_read_only_or_short_space() {
        let mut volume = VolumeState::default();
        assert!(check_capacity(&volume, 100).is_ok());
        volume.available_bytes = Some(99);
        assert_eq!(
            check_capacity(&volume, 100).unwrap_err(),
            "storage_insufficient_space"
        );
        assert!(check_capacity(&volume, 99).is_ok());
        volume.read_only = Some(true);
        assert_eq!(
            check_capacity(&volume, 0).unwrap_err(),
            "storage_volume_read_only"
        );
    }
}
