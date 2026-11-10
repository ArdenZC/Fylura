//! Disposable metadata projection; the existing Read Gate remains authority
//! for content, and mutation still uses its own identity/provider validation.
use super::MaterializationState;
use std::{fs::Metadata, path::Path};

pub(crate) fn from_metadata(path: &Path, metadata: &Metadata) -> MaterializationState {
    #[cfg(windows)]
    {
        use crate::platform::windows::cloud_metadata;
        use std::os::windows::fs::MetadataExt;
        if cloud_metadata::needs_download(metadata.file_attributes()) {
            return MaterializationState::RemotePlaceholder;
        }
        if metadata.file_attributes() & 0x400 != 0 {
            let Ok(tag) = cloud_metadata::reparse_tag(path) else {
                return MaterializationState::Unknown;
            };
            if !cloud_metadata::is_cloud_tag(tag) {
                return MaterializationState::Unknown;
            }
            let Ok(state) = cloud_metadata::placeholder_state_for_path(path, metadata) else {
                return MaterializationState::Unknown;
            };
            return match cloud_metadata::ensure_placeholder_availability(state) {
                Ok(()) => MaterializationState::Local,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    MaterializationState::RemotePlaceholder
                }
                Err(_) => MaterializationState::Unknown,
            };
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        // Probe native provider state only for a provider namespace hint or
        // a dataless object, keeping ordinary 100k-directory metadata cheap.
        let provider_namespace = dirs::home_dir().is_some_and(|home| {
            path.starts_with(home.join("Library/CloudStorage"))
                || path.starts_with(home.join("Library/Mobile Documents"))
        });
        if provider_namespace || metadata.st_flags() & 0x4000_0000 != 0 {
            use crate::platform::macos::{file_semantics, MacContentAvailability as State};
            return match file_semantics::inspect(path).content_availability {
                State::Local => MaterializationState::Local,
                State::BoundaryReadable => MaterializationState::BoundaryReadable,
                State::NotLocal => MaterializationState::RemotePlaceholder,
                State::Downloading => MaterializationState::Hydrating,
                State::MetadataOnly => MaterializationState::MetadataOnly,
                State::Unknown => MaterializationState::Unknown,
            };
        }
    }
    let _ = (path, metadata);
    MaterializationState::Unknown
}
