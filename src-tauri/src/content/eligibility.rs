//! Backend eligibility gates for content reads.
//!
//! On macOS this module is a projection of the single native byte-read gate;
//! it does not reimplement iCloud or File Provider checks.

use std::path::Path;

#[allow(
    dead_code,
    reason = "macOS-only native states are compiled on Windows for shared contracts"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentEligibility {
    Eligible,
    Directory,
    ICloudItemNotLocal,
    FileProviderItemNotLocal,
    MetadataOnly,
    CloudDownloading,
    PermissionRequired,
    ContentAvailabilityUnknown,
    PackageUnsupported,
    Unsupported,
    Symlink,
}

impl ContentEligibility {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::Eligible => "content_eligible",
            Self::Directory => "directory_not_supported",
            Self::ICloudItemNotLocal => "icloud_item_not_local",
            Self::FileProviderItemNotLocal => "file_provider_item_not_local",
            Self::MetadataOnly => "content_metadata_only",
            Self::CloudDownloading => "cloud_item_downloading",
            Self::PermissionRequired => "content_permission_required",
            Self::ContentAvailabilityUnknown => "content_availability_unknown",
            Self::PackageUnsupported => "package_not_supported",
            Self::Unsupported => "content_source_not_supported",
            Self::Symlink => "content_symlink_traversal_blocked",
        }
    }

    pub(crate) fn is_eligible(self) -> bool {
        matches!(self, Self::Eligible)
    }
}

pub(crate) fn classify_path(path: &Path, is_directory: bool) -> ContentEligibility {
    #[cfg(target_os = "macos")]
    {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(_) => return ContentEligibility::PermissionRequired,
        };
        if is_directory || metadata.is_dir() {
            if metadata.is_dir() && crate::platform::macos::file_semantics::inspect(path).is_package
            {
                return ContentEligibility::PackageUnsupported;
            }
            return ContentEligibility::Directory;
        }
        map_native_eligibility(
            crate::platform::macos::file_semantics::content_read_eligibility(path),
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            let metadata = match std::fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(_) => return ContentEligibility::ContentAvailabilityUnknown,
            };
            if crate::platform::windows::cloud_metadata::metadata_needs_download(&metadata) {
                return ContentEligibility::FileProviderItemNotLocal;
            }
            if metadata.file_type().is_symlink() {
                return ContentEligibility::Symlink;
            }
            if metadata.file_attributes() & 0x400 != 0 {
                return ContentEligibility::Unsupported;
            }
        }
        let _ = path;
        if is_directory {
            ContentEligibility::Directory
        } else {
            ContentEligibility::Eligible
        }
    }
}

#[cfg(target_os = "macos")]
fn map_native_eligibility(
    eligibility: crate::platform::macos::MacContentReadEligibility,
) -> ContentEligibility {
    use crate::platform::macos::MacContentReadEligibility;

    match eligibility {
        MacContentReadEligibility::Eligible => ContentEligibility::Eligible,
        MacContentReadEligibility::Symlink => ContentEligibility::Symlink,
        MacContentReadEligibility::NonRegular => ContentEligibility::Unsupported,
        MacContentReadEligibility::PackageUnsupported => ContentEligibility::PackageUnsupported,
        MacContentReadEligibility::ICloudItemNotLocal => ContentEligibility::ICloudItemNotLocal,
        MacContentReadEligibility::ICloudLocalReadDeferred => ContentEligibility::MetadataOnly,
        MacContentReadEligibility::FileProviderItemNotLocal => {
            ContentEligibility::FileProviderItemNotLocal
        }
        MacContentReadEligibility::CloudDownloading => ContentEligibility::CloudDownloading,
        MacContentReadEligibility::MetadataOnly => ContentEligibility::MetadataOnly,
        MacContentReadEligibility::PermissionRequired => ContentEligibility::PermissionRequired,
        MacContentReadEligibility::ContentSourceNotSupported => ContentEligibility::Unsupported,
        MacContentReadEligibility::ContentAvailabilityUnknown => {
            ContentEligibility::ContentAvailabilityUnknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ContentEligibility;

    #[test]
    fn user_visible_reasons_distinguish_i_cloud_provider_and_unknown_states() {
        assert_eq!(
            ContentEligibility::ICloudItemNotLocal.reason(),
            "icloud_item_not_local"
        );
        assert_eq!(
            ContentEligibility::FileProviderItemNotLocal.reason(),
            "file_provider_item_not_local"
        );
        assert_eq!(
            ContentEligibility::ContentAvailabilityUnknown.reason(),
            "content_availability_unknown"
        );
        assert_eq!(
            ContentEligibility::PackageUnsupported.reason(),
            "package_not_supported"
        );
        assert!(!ContentEligibility::ICloudItemNotLocal.is_eligible());
        assert!(ContentEligibility::Eligible.is_eligible());
    }
}
