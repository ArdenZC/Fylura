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
    for path in mounted_volumes().into_iter().take(28) {
        let label = volume_label(&path);
        push_unique(
            &mut locations,
            &mut seen,
            &format!("volume:{}", path.to_string_lossy()),
            KnownLocationKind::Volume,
            Some((path, label)),
        );
    }
    for (path, label) in super::connected_locations::cloud_roots() {
        push_unique(
            &mut locations,
            &mut seen,
            &format!("provider:{}", path.to_string_lossy()),
            KnownLocationKind::CloudProvider,
            Some((path, label)),
        );
    }
    locations.truncate(48);
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
    let volume = (kind != KnownLocationKind::Folder).then(|| {
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
        available: path.is_dir(),
        volume,
        path: path.to_string_lossy().into_owned(),
    });
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
    mounted_volumes()
        .into_iter()
        .take(28)
        .find(|path| format!("volume:{}", path.to_string_lossy()) == id)
}

fn named(path: Option<PathBuf>, label: &str) -> Option<(PathBuf, String)> {
    path.map(|path| (path, label.to_string()))
}

#[cfg(target_os = "windows")]
fn mounted_volumes() -> Vec<PathBuf> {
    // Logical drive letters are a cheap, non-recursive existence probe. No
    // directory contents are enumerated and no volume is scanned or indexed.
    let drives = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
    (0..26)
        .filter(|index| drives & (1 << index) != 0)
        .map(|index| PathBuf::from(format!("{}:\\", (b'A' + index) as char)))
        .collect()
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
}
