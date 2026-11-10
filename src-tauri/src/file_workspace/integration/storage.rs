//! Mounted-volume metadata and explicit, identity-bound OS safe removal.
//! This is not a transfer executor or a new persistent device registry.
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeState {
    pub identity: Option<String>,
    /// Native device identity used only while grouping mounted volumes for
    /// safe removal. This is deliberately not part of the renderer contract.
    #[cfg(windows)]
    #[serde(skip)]
    pub(crate) physical_device_identity: Option<String>,
    pub kind: &'static str,
    pub read_only: Option<bool>,
    pub available_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub can_eject: bool,
    pub availability: &'static str,
}

impl Default for VolumeState {
    fn default() -> Self {
        Self {
            identity: None,
            #[cfg(windows)]
            physical_device_identity: None,
            kind: "unknown",
            read_only: None,
            available_bytes: None,
            total_bytes: None,
            can_eject: false,
            availability: "unknown",
        }
    }
}

pub(crate) fn root_identity(path: &Path) -> Option<String> {
    let physical_key = crate::fs_safety::capture_source_object_identity(path)
        .ok()?
        .physical_key?;
    #[cfg(windows)]
    if let Some(device_key) = native::physical_device_identity(path) {
        return Some(format!("{physical_key}:pnp:{device_key}"));
    }
    Some(physical_key)
}

pub(crate) fn inspect(path: &Path, removal: bool) -> VolumeState {
    let mut state = native::inspect(path, removal);
    let physical_key = crate::fs_safety::capture_source_object_identity(path)
        .ok()
        .and_then(|identity| identity.physical_key);
    #[cfg(windows)]
    let device_key = state.physical_device_identity.clone();
    state.identity = physical_key.map(|physical_key| {
        #[cfg(windows)]
        if let Some(device_key) = device_key.as_deref() {
            return format!("{physical_key}:pnp:{device_key}");
        }
        physical_key
    });
    state.can_eject &= state.identity.is_some();
    state.availability = match std::fs::symlink_metadata(path) {
        Ok(_) => "available",
        Err(error) => unavailable_reason(&error),
    };
    state
}

fn unavailable_reason(error: &std::io::Error) -> &'static str {
    use std::io::ErrorKind;
    #[cfg(windows)]
    match error.raw_os_error() {
        Some(5 | 65 | 86 | 1219 | 1326) => return "permission_required",
        Some(53 | 64 | 67 | 121 | 1201 | 1231 | 1232 | 1236 | 2250) => return "offline",
        _ => {}
    }
    match error.kind() {
        ErrorKind::PermissionDenied => "permission_required",
        ErrorKind::NotFound
        | ErrorKind::NotConnected
        | ErrorKind::ConnectionAborted
        | ErrorKind::ConnectionReset
        | ErrorKind::TimedOut => "offline",
        _ => "unknown",
    }
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
        self.release_volume_readers(&path, &request.identity)?;
        native::eject(&path, &request.identity)
    }

    /// Release our own disposable directory/preview handles before asking the OS
    /// to remove a volume. Keep session roots and history intent for reconnect or
    /// a veto; do not close other processes' handles or force a dismount.
    pub(crate) fn release_volume_readers(
        &self,
        root: &Path,
        expected_identity: &str,
    ) -> Result<(), String> {
        if root_identity(root).as_deref() != Some(expected_identity) {
            return Err("storage_device_changed".into());
        }
        #[cfg(windows)]
        let selected_device = expected_identity
            .split_once(":pnp:")
            .map(|(_, device)| device.to_string())
            .filter(|device| !device.is_empty())
            .ok_or("storage_device_changed")?;
        #[cfg(not(windows))]
        let selected_device = crate::fs_safety::capture_source_object_identity(root)
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
        let session_roots = sessions
            .into_iter()
            .map(|(id, reference)| {
                self.inner
                    .browse
                    .paths_for_storage_removal(&id, &reference)
                    .map(|(directories, identity)| {
                        (
                            id,
                            directories
                                .into_iter()
                                .map(|directory| directory.as_path().to_path_buf())
                                .collect(),
                            identity,
                        )
                    })
                    .map_err(|_| "storage_eject_busy")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let ids =
            sessions_on_eject_device(&session_roots, |session_id, paths, admission_identity| {
                #[cfg(windows)]
                {
                    let _ = (session_id, admission_identity);
                    // This is resource cleanup only: use the live PnP device
                    // identity to find every partition, even if a retained
                    // browse root has gone stale. The selected volume identity
                    // is still revalidated before the OS eject request.
                    paths
                        .iter()
                        .any(|path| same_eject_device(&selected_device, path))
                }
                #[cfg(not(windows))]
                {
                    let _ = (session_id, paths);
                    admission_identity
                        .as_ref()
                        .and_then(|identity| identity.platform_volume_id.as_deref())
                        .is_some_and(|volume| volume == selected_device)
                }
            });
        if root_identity(root).as_deref() != Some(expected_identity) {
            return Err("storage_device_changed".into());
        }
        for id in ids {
            self.dispose_monitors_for_session(&id)
                .map_err(|_| "storage_eject_busy")?;
            if self.dispose_previews_for_session(&id).is_some() {
                return Err("storage_eject_busy".into());
            }
            self.inner
                .browse
                .invalidate(&id)
                .map_err(|_| "storage_eject_busy")?;
        }
        Ok(())
    }
}

fn sessions_on_eject_device<T>(
    sessions: &[(String, Vec<std::path::PathBuf>, T)],
    mut belongs_to_device: impl FnMut(&String, &[std::path::PathBuf], &T) -> bool,
) -> Vec<String> {
    sessions
        .iter()
        .filter(|(id, paths, identity)| belongs_to_device(id, paths, identity))
        .map(|(id, _, _)| id.clone())
        .collect()
}

#[cfg(windows)]
fn same_eject_device(selected: &str, candidate: &Path) -> bool {
    native::same_physical_device_identity(selected, candidate)
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

    #[test]
    fn eject_releases_all_panes_and_nested_mounts_but_not_other_devices() {
        let sessions = vec![
            ("tab-a-pane-1".to_string(), vec!["E:\\".into()], ()),
            ("tab-a-pane-2".to_string(), vec!["F:\\".into()], ()),
            (
                "tab-b-pane-1".to_string(),
                vec!["C:\\".into(), "E:\\mounted-child".into()],
                (),
            ),
            ("other-usb".to_string(), vec!["G:\\".into()], ()),
            ("system".to_string(), vec!["C:\\".into()], ()),
        ];
        let device = |path: &Path| match path.to_string_lossy().chars().next() {
            Some('E' | 'F') => Some("pnp:usb-device-a"),
            Some('G') => Some("pnp:usb-device-b"),
            Some('C') => Some("pnp:system-disk"),
            _ => None,
        };
        let selected = device(Path::new("E:\\")).unwrap();
        let released = sessions_on_eject_device(&sessions, |_, paths, _| {
            paths
                .iter()
                .any(|path| device(path).is_some_and(|candidate| candidate == selected))
        });
        assert_eq!(released, ["tab-a-pane-1", "tab-a-pane-2", "tab-b-pane-1"]);
        assert!(!released.contains(&"other-usb".to_string()));
        assert!(!released.contains(&"system".to_string()));
    }
}
