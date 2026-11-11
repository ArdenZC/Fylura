//! Read-only projection of the user's well-known folders and mounted volumes.
//!
//! These are navigation entry points only. Nothing in this module persists a
//! preference, admits a Browse session, creates a scan root, authorizes a
//! Global Index source or reads directory contents beyond a single
//! `is_dir` existence check. Opening one of these paths still goes through the
//! existing `file_workspace_browse_open` user-intent admission, which
//! re-validates the directory on the backend.
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Stable identifier of a well-known location. The renderer owns localized
/// labels; `display_name` is only a fallback for volumes and unknown folders.
pub(crate) const ID_HOME: &str = "home";
pub(crate) const ID_DESKTOP: &str = "desktop";
pub(crate) const ID_DOWNLOADS: &str = "downloads";
pub(crate) const ID_DOCUMENTS: &str = "documents";
const MAX_WINDOWS_MOUNTED_LOCATIONS: usize = 256;
const MAX_OTHER_MOUNTED_LOCATIONS: usize = 28;
const MAX_KNOWN_LOCATIONS: usize = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KnownLocationKind {
    Folder,
    Volume,
    CloudProvider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownLocation {
    pub id: String,
    pub kind: KnownLocationKind,
    pub display_name: String,
    pub path: String,
    pub available: bool,
    pub volume: Option<super::storage::VolumeState>,
}

/// Bounded, deterministic ordering: home folders first, then volumes.
pub(crate) fn list_known_locations() -> Vec<KnownLocation> {
    let mut locations = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    for (id, path) in user_folders() {
        push_unique(
            &mut locations,
            &mut seen,
            id,
            KnownLocationKind::Folder,
            path,
        );
    }
    let provider_roots = super::connected_locations::cloud_roots();
    let volume_limit = mounted_volume_limit(locations.len(), provider_roots.len());
    for path in mounted_volumes().into_iter().take(volume_limit) {
        let label = volume_label(&path);
        push_unique(
            &mut locations,
            &mut seen,
            &format!("volume:{}", path.to_string_lossy()),
            KnownLocationKind::Volume,
            Some((path, label)),
        );
    }
    for (path, label) in provider_roots {
        push_unique(
            &mut locations,
            &mut seen,
            &format!("provider:{}", path.to_string_lossy()),
            KnownLocationKind::CloudProvider,
            Some((path, label)),
        );
    }
    locations.truncate(MAX_KNOWN_LOCATIONS);
    locations
}

fn push_unique(
    locations: &mut Vec<KnownLocation>,
    seen: &mut Vec<String>,
    id: &str,
    kind: KnownLocationKind,
    path: Option<(PathBuf, String)>,
) {
    let Some((path, display_name)) = path else {
        return;
    };
    let key = path_key(&path);
    if key.is_empty() || seen.iter().any(|existing| existing == &key) {
        return;
    }
    seen.push(key);
    #[cfg(windows)]
    let connected_unc =
        kind == KnownLocationKind::Volume && connected_unc_share(&path.to_string_lossy()).is_some();
    #[cfg(windows)]
    let network_availability = if kind == KnownLocationKind::Volume {
        if connected_unc {
            Some("connected_unverified")
        } else {
            windows_network_connection_state(&path)
        }
    } else {
        None
    };
    let volume = (kind != KnownLocationKind::Folder).then(|| {
        #[cfg(windows)]
        if let Some(availability) = network_availability {
            return super::storage::VolumeState {
                kind: "network",
                availability,
                ..super::storage::VolumeState::default()
            };
        }
        let mut state = super::storage::inspect(&path, kind == KnownLocationKind::Volume);
        if kind == KnownLocationKind::CloudProvider {
            state.kind = "cloud_provider";
            state.can_eject = false;
        }
        state
    });
    locations.push(KnownLocation {
        id: id.to_string(),
        kind,
        display_name,
        // WNet RESOURCE_CONNECTED is evidence of a system connection record,
        // not evidence that the SMB server can currently be reached. Never
        // probe a network path during navigation refresh; the explicit Browse
        // open is the access check.
        available: {
            #[cfg(windows)]
            if let Some(availability) = network_availability {
                network_status_is_verified_available(availability)
            } else {
                path.is_dir()
            }
            #[cfg(not(windows))]
            {
                path.is_dir()
            }
        },
        volume,
        path: path.to_string_lossy().into_owned(),
    });
}

#[cfg(any(windows, test))]
fn network_status_is_verified_available(availability: &str) -> bool {
    availability == "available"
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy()
        .trim_end_matches(['/', '\\'])
        .to_lowercase()
}

fn user_folders() -> Vec<(&'static str, Option<(PathBuf, String)>)> {
    vec![
        (ID_HOME, named(dirs::home_dir(), "Home")),
        (ID_DESKTOP, named(dirs::desktop_dir(), "Desktop")),
        (ID_DOWNLOADS, named(dirs::download_dir(), "Downloads")),
        (ID_DOCUMENTS, named(dirs::document_dir(), "Documents")),
    ]
}

/// Resolve only a currently mounted backend-owned root. Safe removal must not
/// wait on metadata probes for unrelated offline network shares.
pub(super) fn mounted_volume_path(id: &str) -> Option<PathBuf> {
    let folder_count = user_folders()
        .iter()
        .filter(|(_, path)| path.is_some())
        .count();
    let provider_count = super::connected_locations::cloud_roots().len();
    mounted_volumes()
        .into_iter()
        .take(mounted_volume_limit(folder_count, provider_count))
        .find(|path| format!("volume:{}", path.to_string_lossy()) == id)
}

fn mounted_volume_limit(folder_count: usize, provider_count: usize) -> usize {
    let platform_limit = if cfg!(target_os = "windows") {
        MAX_WINDOWS_MOUNTED_LOCATIONS
    } else {
        MAX_OTHER_MOUNTED_LOCATIONS
    };
    MAX_KNOWN_LOCATIONS
        .saturating_sub(folder_count.saturating_add(provider_count))
        .min(platform_limit)
}

fn named(path: Option<PathBuf>, label: &str) -> Option<(PathBuf, String)> {
    path.map(|path| (path, label.to_string()))
}

#[cfg(target_os = "windows")]
fn mounted_volumes() -> Vec<PathBuf> {
    // Logical drive letters are a cheap, non-recursive existence probe. No
    // directory contents are enumerated and no volume is scanned or indexed.
    let drives = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
    let mut volumes = (0..26)
        .filter(|index| drives & (1 << index) != 0)
        .map(|index| PathBuf::from(format!("{}:\\", (b'A' + index) as char)))
        .collect::<Vec<_>>();
    volumes.extend(connected_unc_shares());
    volumes
}

#[cfg(target_os = "windows")]
fn connected_unc_shares() -> Vec<PathBuf> {
    use std::{mem::size_of, ptr};
    use windows_sys::Win32::{
        Foundation::HANDLE,
        NetworkManagement::WNet::{
            WNetCloseEnum, WNetEnumResourceW, WNetOpenEnumW, NETRESOURCEW, RESOURCETYPE_DISK,
            RESOURCE_CONNECTED,
        },
    };

    const ERROR_MORE_DATA_CODE: u32 = 234;
    const ERROR_NO_MORE_ITEMS_CODE: u32 = 259;
    const MAX_BUFFER_BYTES: usize = 256 * 1024;
    const MAX_RESOURCES: usize = 256;

    struct Enumeration(HANDLE);
    impl Drop for Enumeration {
        fn drop(&mut self) {
            unsafe {
                WNetCloseEnum(self.0);
            }
        }
    }

    let mut handle = ptr::null_mut();
    if unsafe {
        WNetOpenEnumW(
            RESOURCE_CONNECTED,
            RESOURCETYPE_DISK,
            0,
            ptr::null(),
            &mut handle,
        )
    } != 0
        || handle.is_null()
    {
        return Vec::new();
    }
    let _enumeration = Enumeration(handle);
    let mut shares: Vec<PathBuf> = Vec::new();
    let mut buffer = vec![0_u64; 4096];
    let mut buffer_size = (buffer.len() * size_of::<u64>()) as u32;

    let mut enumerated = 0usize;
    'pages: while enumerated < MAX_RESOURCES {
        let mut count = (MAX_RESOURCES - enumerated) as u32;
        let status = unsafe {
            WNetEnumResourceW(
                handle,
                &mut count,
                buffer.as_mut_ptr().cast(),
                &mut buffer_size,
            )
        };
        if status == ERROR_MORE_DATA_CODE {
            let allocated_bytes = buffer.len() * size_of::<u64>();
            if buffer_size as usize > MAX_BUFFER_BYTES || buffer_size as usize <= allocated_bytes {
                break;
            }
            buffer = vec![0_u64; (buffer_size as usize).div_ceil(size_of::<u64>())];
            buffer_size = (buffer.len() * size_of::<u64>()) as u32;
            continue;
        }
        if status == ERROR_NO_MORE_ITEMS_CODE || count == 0 {
            break;
        }
        if status != 0
            || count as usize > buffer.len() * size_of::<u64>() / size_of::<NETRESOURCEW>()
        {
            break;
        }
        enumerated = enumerated.saturating_add(count as usize);
        let resources = unsafe {
            std::slice::from_raw_parts(buffer.as_ptr().cast::<NETRESOURCEW>(), count as usize)
        };
        for resource in resources {
            // Mapped drives remain in GetLogicalDrives. Only include active
            // network connections that have no local drive letter.
            if resource.dwType != RESOURCETYPE_DISK {
                continue;
            }
            let buffer_ptr = buffer.as_ptr().cast::<u8>();
            let buffer_bytes = buffer.len() * size_of::<u64>();
            let has_no_local_name = resource.lpLocalName.is_null()
                || unsafe { bounded_wide_string(resource.lpLocalName, buffer_ptr, buffer_bytes) }
                    .is_some_and(|name| name.is_empty());
            if !has_no_local_name {
                continue;
            }
            let Some(remote) =
                (unsafe { bounded_wide_string(resource.lpRemoteName, buffer_ptr, buffer_bytes) })
            else {
                continue;
            };
            if let Some(path) = connected_unc_share(&remote) {
                if !shares
                    .iter()
                    .any(|existing| path_key(existing) == path_key(&path))
                {
                    shares.push(path);
                }
            }
            if shares.len() >= MAX_RESOURCES {
                break 'pages;
            }
        }
        buffer_size = (buffer.len() * size_of::<u64>()) as u32;
    }
    shares.sort_by_key(|path| path_key(path));
    shares
}

#[cfg(target_os = "windows")]
unsafe fn bounded_wide_string(
    value: windows_sys::core::PWSTR,
    buffer: *const u8,
    buffer_bytes: usize,
) -> Option<String> {
    if value.is_null() || buffer.is_null() {
        return None;
    }
    let buffer_start = buffer as usize;
    let buffer_end = buffer_start.checked_add(buffer_bytes)?;
    let address = value as usize;
    if address < buffer_start
        || address >= buffer_end
        || !(address - buffer_start).is_multiple_of(2)
    {
        return None;
    }
    let max_units = ((buffer_end - address) / std::mem::size_of::<u16>()).min(32768);
    let units = unsafe { std::slice::from_raw_parts(value, max_units) };
    let end = units.iter().position(|unit| *unit == 0)?;
    String::from_utf16(&units[..end]).ok()
}

#[cfg(target_os = "windows")]
fn windows_network_connection_state(path: &Path) -> Option<&'static str> {
    use windows_sys::Win32::{
        NetworkManagement::WNet::WNetGetConnectionW, Storage::FileSystem::GetDriveTypeW,
    };

    let encoded = path.to_string_lossy();
    let root = encoded.as_ref();
    let root_wide = root.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let local = root.trim_end_matches(['\\', '/']);
    let local = local.get(..2)?;
    let local_wide = local.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut remote = vec![0u16; 32768];
    let mut length = remote.len() as u32;
    let result =
        unsafe { WNetGetConnectionW(local_wide.as_ptr(), remote.as_mut_ptr(), &mut length) };
    let drive_type = unsafe { GetDriveTypeW(root_wide.as_ptr()) };
    windows_network_connection_state_from_status(result, drive_type)
}

#[cfg(target_os = "windows")]
fn windows_network_connection_state_from_status(
    result: u32,
    drive_type: u32,
) -> Option<&'static str> {
    use windows_sys::Win32::System::WindowsProgramming::{DRIVE_NO_ROOT_DIR, DRIVE_REMOTE};

    let is_remote = drive_type == DRIVE_REMOTE;
    let missing_root = drive_type == DRIVE_NO_ROOT_DIR;
    match result {
        // WNetGetConnection returns the remembered/connected mapping. It does
        // not prove that the remote server is currently reachable.
        0 if is_remote => Some("connected_unverified"),
        0 if missing_root => Some("offline"),
        5 | 65 | 86 | 1326 => Some("permission_required"),
        53 | 54 | 58 | 59 | 64 | 67 | 121 | 1201 | 1203 | 1222 | 1231 | 1232 | 1233 | 1234
        | 1235 | 1236 => Some("offline"),
        2250 if is_remote || missing_root => Some("offline"),
        _ if is_remote || missing_root => Some("unknown"),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn connected_unc_share(remote: &str) -> Option<PathBuf> {
    let value = remote.trim_end_matches('\\');
    let remainder = value.strip_prefix("\\\\")?;
    if remainder.starts_with('?') || remainder.starts_with('.') || remainder.contains('/') {
        return None;
    }
    let mut parts = remainder.split('\\');
    let server = parts.next()?;
    let share = parts.next()?;
    if server.is_empty()
        || share.is_empty()
        || server == "."
        || server == ".."
        || share == "."
        || share == ".."
        || parts.next().is_some()
    {
        return None;
    }
    Some(PathBuf::from(format!("\\\\{server}\\{share}")))
}

#[cfg(target_os = "macos")]
fn mounted_volumes() -> Vec<PathBuf> {
    // `/Volumes` is a synthetic mount list. Reading one level does not hydrate
    // iCloud or File Provider content; entries are not recursed into.
    let Ok(entries) = std::fs::read_dir("/Volumes") else {
        return Vec::new();
    };
    let mut volumes = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    volumes.sort();
    volumes
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn mounted_volumes() -> Vec<PathBuf> {
    Vec::new()
}

fn volume_label(path: &Path) -> String {
    #[cfg(target_os = "windows")]
    if connected_unc_share(&path.to_string_lossy()).is_some() {
        return path.to_string_lossy().into_owned();
    }
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_locations_are_unique_absolute_and_bounded() {
        let locations = list_known_locations();
        let mut ids = std::collections::HashSet::new();
        let mut paths = std::collections::HashSet::new();
        for location in &locations {
            assert!(ids.insert(location.id.clone()), "duplicate id");
            assert!(
                paths.insert(path_key(Path::new(&location.path))),
                "duplicate path"
            );
            assert!(
                Path::new(&location.path).is_absolute(),
                "known location must be absolute: {}",
                location.path
            );
            assert!(!location.display_name.is_empty());
        }
        assert!(locations.len() <= 48, "known locations must stay bounded");
    }

    #[test]
    fn listing_known_locations_never_requires_a_runtime() {
        // Purely a read-only projection: calling it twice must be stable and
        // must not depend on database, scan or index state.
        let first = list_known_locations();
        let second = list_known_locations();
        let entries = |locations: Vec<KnownLocation>| {
            locations
                .into_iter()
                .map(|location| (location.id, location.kind, location.path))
                .collect::<Vec<_>>()
        };
        // Capacity/availability are live observations and may change between
        // calls as the OS writes data. The navigation identity/order is stable.
        assert_eq!(entries(first), entries(second));
    }

    #[test]
    fn missing_folder_is_projected_as_unavailable_not_dropped() {
        let mut locations = Vec::new();
        let mut seen = Vec::new();
        push_unique(
            &mut locations,
            &mut seen,
            "missing",
            KnownLocationKind::Folder,
            Some((
                PathBuf::from("/nonexistent-fylura-known-location"),
                "Missing".into(),
            )),
        );
        assert_eq!(locations.len(), 1);
        assert!(!locations[0].available);
        assert_eq!(locations[0].id, "missing");
    }

    #[test]
    fn system_connection_record_is_not_projected_as_verified_availability() {
        assert!(!network_status_is_verified_available(
            "connected_unverified"
        ));
        assert!(!network_status_is_verified_available("offline"));
        assert!(!network_status_is_verified_available("permission_required"));
        assert!(!network_status_is_verified_available("unknown"));
        assert!(network_status_is_verified_available("available"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn connected_unc_discovery_accepts_only_unc_share_roots() {
        assert_eq!(
            connected_unc_share(r"\\nas.example\documents").unwrap(),
            PathBuf::from(r"\\nas.example\documents")
        );
        assert_eq!(
            connected_unc_share(r"\\nas.example\documents\").unwrap(),
            PathBuf::from(r"\\nas.example\documents")
        );
        for invalid in [
            r"X:\\",
            r"\\nas.example",
            r"\\nas.example\documents\nested",
            r"\\?\Volume{abc}",
            r"\\.\PhysicalDrive0",
            r"\\nas.example\..",
        ] {
            assert!(connected_unc_share(invalid).is_none(), "{invalid}");
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn connected_unc_provider_strings_must_stay_inside_the_enum_buffer() {
        let mut buffer = [0u16; 16];
        buffer[..6].copy_from_slice(&[b'n' as u16, b'a' as u16, b's' as u16, 0, 0, 0]);
        let base = buffer.as_ptr().cast::<u8>();
        let bytes = std::mem::size_of_val(&buffer);
        assert_eq!(
            unsafe { bounded_wide_string(buffer.as_mut_ptr(), base, bytes) }.as_deref(),
            Some("nas")
        );
        assert!(
            unsafe { bounded_wide_string(buffer.as_mut_ptr().add(buffer.len()), base, bytes) }
                .is_none()
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_network_status_preserves_offline_permission_and_available_states() {
        use windows_sys::Win32::System::WindowsProgramming::{
            DRIVE_FIXED, DRIVE_NO_ROOT_DIR, DRIVE_REMOTE,
        };

        assert_eq!(mounted_volume_limit(4, 4), 40);
        assert_eq!(mounted_volume_limit(4, 0), 44);
        assert_eq!(mounted_volume_limit(4, 8), 36);
        assert_eq!(
            windows_network_connection_state_from_status(0, DRIVE_REMOTE),
            Some("connected_unverified")
        );
        assert_eq!(
            windows_network_connection_state_from_status(0, DRIVE_NO_ROOT_DIR),
            Some("offline")
        );
        assert_eq!(
            windows_network_connection_state_from_status(5, DRIVE_REMOTE),
            Some("permission_required")
        );
        assert_eq!(
            windows_network_connection_state_from_status(121, DRIVE_REMOTE),
            Some("offline")
        );
        assert_eq!(
            windows_network_connection_state_from_status(2250, DRIVE_NO_ROOT_DIR),
            Some("offline")
        );
        assert_eq!(
            windows_network_connection_state_from_status(1222, DRIVE_NO_ROOT_DIR),
            Some("offline")
        );
        assert_eq!(
            windows_network_connection_state_from_status(2250, DRIVE_FIXED),
            None
        );
        assert_eq!(
            windows_network_connection_state_from_status(0, DRIVE_FIXED),
            None
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_volume_discovery_remains_limited_to_mounted_volumes() {
        assert_eq!(mounted_volume_limit(4, 8), 28);
        assert!(mounted_volumes()
            .iter()
            .all(|path| path.starts_with("/Volumes")));
    }
}
