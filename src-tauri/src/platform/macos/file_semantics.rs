//! Unified read-only macOS file semantics and the single byte-read gate.

use super::{cloud_item, file_provider, package, volume, MacCloudBacking, MacContentAvailability};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacContentReadEligibility {
    Eligible,
    Symlink,
    NonRegular,
    PackageUnsupported,
    ICloudItemNotLocal,
    ICloudLocalReadDeferred,
    FileProviderItemNotLocal,
    CloudDownloading,
    MetadataOnly,
    PermissionRequired,
    ContentSourceNotSupported,
    ContentAvailabilityUnknown,
}

impl MacContentReadEligibility {
    pub fn is_eligible(self) -> bool {
        matches!(self, Self::Eligible)
    }

    pub fn reason(self) -> &'static str {
        match self {
            Self::Eligible => "content_eligible",
            Self::Symlink => "content_symlink_traversal_blocked",
            Self::NonRegular | Self::ContentSourceNotSupported => "content_source_not_supported",
            Self::PackageUnsupported => "package_not_supported",
            Self::ICloudItemNotLocal => "icloud_item_not_local",
            Self::ICloudLocalReadDeferred => "icloud_local_read_deferred",
            Self::FileProviderItemNotLocal => "file_provider_item_not_local",
            Self::CloudDownloading => "cloud_item_downloading",
            Self::MetadataOnly => "content_metadata_only",
            Self::PermissionRequired => "content_permission_required",
            Self::ContentAvailabilityUnknown => "content_availability_unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacFileSemantics {
    pub is_symlink: bool,
    pub is_regular_file: bool,
    pub is_package: bool,
    pub backing_kind: MacCloudBacking,
    pub content_availability: MacContentAvailability,
    pub logical_size: Option<u64>,
    pub allocated_size: Option<u64>,
    pub volume: super::MacVolumeSemantics,
    pub provider_identity_evidence: file_provider::MacProviderIdentityEvidence,
}

impl MacFileSemantics {
    pub fn unsupported() -> Self {
        Self {
            is_symlink: false,
            is_regular_file: false,
            is_package: false,
            backing_kind: MacCloudBacking::Unknown,
            content_availability: MacContentAvailability::Unknown,
            logical_size: None,
            allocated_size: None,
            volume: super::MacVolumeSemantics {
                stable_id: None,
                mount_path: None,
                filesystem_type: None,
                is_local: None,
                is_removable: None,
                is_read_only: None,
            },
            provider_identity_evidence: file_provider::MacProviderIdentityEvidence::None,
        }
    }
}

/// Classifies whether content bytes may be opened without following a link or
/// implicitly materializing a cloud/File Provider placeholder.
pub fn content_read_eligibility(path: &Path) -> MacContentReadEligibility {
    let Some(metadata) = std::fs::symlink_metadata(path).ok() else {
        return MacContentReadEligibility::PermissionRequired;
    };
    if metadata.file_type().is_symlink() {
        return MacContentReadEligibility::Symlink;
    }
    if !metadata.is_file() {
        if metadata.is_dir() && package::is_package(path) {
            return MacContentReadEligibility::PackageUnsupported;
        }
        return MacContentReadEligibility::NonRegular;
    }

    let semantics = inspect(path);
    if semantics.is_package {
        return MacContentReadEligibility::PackageUnsupported;
    }
    match (semantics.backing_kind, semantics.content_availability) {
        (MacCloudBacking::Local, MacContentAvailability::Local) => {
            MacContentReadEligibility::Eligible
        }
        (MacCloudBacking::ICloud, MacContentAvailability::NotLocal) => {
            MacContentReadEligibility::ICloudItemNotLocal
        }
        (MacCloudBacking::ICloud, MacContentAvailability::Downloading) => {
            MacContentReadEligibility::CloudDownloading
        }
        (MacCloudBacking::ICloud, MacContentAvailability::MetadataOnly) => {
            MacContentReadEligibility::MetadataOnly
        }
        // A native current/downloaded resource value is the provider-native
        // full-consumption state. Placeholder and downloading states remain
        // blocked, so byte reads never start a silent cloud download.
        (MacCloudBacking::ICloud, MacContentAvailability::Local) => {
            MacContentReadEligibility::Eligible
        }
        (MacCloudBacking::ICloud, MacContentAvailability::Unknown) => {
            MacContentReadEligibility::ContentAvailabilityUnknown
        }
        (MacCloudBacking::Local, MacContentAvailability::BoundaryReadable)
        | (MacCloudBacking::ICloud, MacContentAvailability::BoundaryReadable) => {
            MacContentReadEligibility::ContentAvailabilityUnknown
        }
        // A CloudStorage path is only a routing hint. The generic client route
        // uses a coordinated user-visible URL plus physical identity; a
        // provider-internal item/domain ID is neither required nor available
        // to this ordinary desktop app.
        (MacCloudBacking::FileProvider, MacContentAvailability::Local)
            if file_provider::GENERIC_FILE_PROVIDER_COORDINATED_URL_SUPPORTED
                && matches!(
                    semantics.provider_identity_evidence,
                    file_provider::MacProviderIdentityEvidence::NamespaceHint
                        | file_provider::MacProviderIdentityEvidence::CoordinatedUserVisibleUrl { .. }
                ) =>
        {
            MacContentReadEligibility::Eligible
        }
        (MacCloudBacking::FileProvider, MacContentAvailability::BoundaryReadable)
            if file_provider::GENERIC_FILE_PROVIDER_COORDINATED_URL_SUPPORTED
                && matches!(
                    semantics.provider_identity_evidence,
                    file_provider::MacProviderIdentityEvidence::NamespaceHint
                        | file_provider::MacProviderIdentityEvidence::CoordinatedUserVisibleUrl { .. }
                ) =>
        {
            // This is only a recent bounded proof. The actual byte consumer
            // must still reopen and validate the source at its own boundary.
            MacContentReadEligibility::Eligible
        }
        (MacCloudBacking::FileProvider, _) => MacContentReadEligibility::FileProviderItemNotLocal,
        (MacCloudBacking::Unknown, _) => MacContentReadEligibility::ContentAvailabilityUnknown,
        (_, MacContentAvailability::NotLocal) => {
            MacContentReadEligibility::ContentSourceNotSupported
        }
        (_, MacContentAvailability::Downloading) => MacContentReadEligibility::CloudDownloading,
        (_, MacContentAvailability::MetadataOnly) => MacContentReadEligibility::MetadataOnly,
        (_, MacContentAvailability::Unknown) => {
            MacContentReadEligibility::ContentAvailabilityUnknown
        }
    }
}

/// The only production macOS byte-open primitive. It performs the eligibility
/// check again at the open boundary, uses O_NOFOLLOW/O_CLOEXEC, and validates
/// the opened descriptor with fstat-derived metadata before returning it.
#[cfg(target_os = "macos")]
pub fn open_content_read(path: &Path) -> Result<std::fs::File, &'static str> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let provider = file_provider::inspect(path);
    let provider_identity_evidence = if matches!(
        provider.domain_state,
        file_provider::FileProviderDomainState::KnownDomain
    ) {
        Some(file_provider::coordinated_user_visible_url_for_execution(
            path,
        )?)
    } else {
        None
    };
    let eligibility = content_read_eligibility(path);
    if !eligibility.is_eligible() {
        return Err(eligibility.reason());
    }
    let before = std::fs::symlink_metadata(path)
        .map_err(|_| MacContentReadEligibility::PermissionRequired.reason())?;
    if !before.is_file() || before.file_type().is_symlink() {
        return Err(MacContentReadEligibility::ContentSourceNotSupported.reason());
    }

    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::PermissionDenied => {
                MacContentReadEligibility::PermissionRequired.reason()
            }
            _ => MacContentReadEligibility::ContentSourceNotSupported.reason(),
        })?;
    let after = file
        .metadata()
        .map_err(|_| MacContentReadEligibility::ContentSourceNotSupported.reason())?;
    if !after.is_file()
        || after.dev() != before.dev()
        || after.ino() != before.ino()
        || after.len() != before.len()
    {
        return Err("content_source_identity_changed");
    }
    if let Some(expected) = provider_identity_evidence {
        let current = file_provider::coordinated_user_visible_url_for_execution(path)?;
        if current != expected {
            return Err("mac_provider_url_changed");
        }
        let Some(expected_physical) = expected.physical_identity() else {
            return Err("mac_provider_coordinated_url_unavailable");
        };
        let current_physical =
            crate::platform::macos::identity::MacPhysicalIdentity::from_fd(&file)
                .map_err(|_| "content_source_identity_changed")?;
        if !expected_physical.matches_strict(current_physical) {
            return Err("mac_provider_url_changed");
        }
    }
    Ok(file)
}

pub fn inspect(path: &Path) -> MacFileSemantics {
    let Some(metadata) = std::fs::symlink_metadata(path).ok() else {
        return MacFileSemantics::unsupported();
    };
    inspect_with_metadata(path, &metadata)
}

/// Inspects provider and cloud semantics while reusing metadata already read
/// by a caller that needs the same identity and file-type facts.
pub fn inspect_with_metadata(path: &Path, metadata: &std::fs::Metadata) -> MacFileSemantics {
    inspect_with_metadata_and_volume(path, metadata, None)
}

/// Uses a proven same-volume metadata snapshot to avoid repeating volume
/// resource queries while converting a batch of Spotlight results.
pub fn inspect_with_metadata_and_volume(
    path: &Path,
    metadata: &std::fs::Metadata,
    cached_volume: Option<&super::MacVolumeSemantics>,
) -> MacFileSemantics {
    #[cfg(target_os = "macos")]
    {
        // Preview classification runs on a long-lived worker. Keep every
        // Foundation object created by the complete read-only probe inside a
        // bounded autorelease pool; the returned semantics contain only Rust
        // values and do not retain native objects between epochs.
        objc2::rc::autoreleasepool(|_| inspect_unpooled(path, metadata, cached_volume))
    }

    #[cfg(not(target_os = "macos"))]
    inspect_unpooled(path, metadata, cached_volume)
}

fn inspect_unpooled(
    path: &Path,
    metadata: &std::fs::Metadata,
    cached_volume: Option<&super::MacVolumeSemantics>,
) -> MacFileSemantics {
    let volume = cached_volume
        .cloned()
        .unwrap_or_else(|| volume::inspect(path));
    let is_symlink = metadata.file_type().is_symlink();
    if is_symlink {
        return MacFileSemantics {
            is_symlink: true,
            is_regular_file: false,
            is_package: false,
            backing_kind: MacCloudBacking::Unknown,
            content_availability: MacContentAvailability::Unknown,
            logical_size: None,
            allocated_size: None,
            volume,
            provider_identity_evidence: file_provider::MacProviderIdentityEvidence::None,
        };
    }

    let i_cloud = cloud_item::inspect(path);
    let file_provider = file_provider::inspect(path);
    let backing_kind = if i_cloud.state != cloud_item::ICloudItemState::NotICloud {
        MacCloudBacking::ICloud
    } else if matches!(
        file_provider.domain_state,
        file_provider::FileProviderDomainState::KnownDomain
    ) {
        MacCloudBacking::FileProvider
    } else if volume.is_local == Some(true) {
        MacCloudBacking::Local
    } else {
        MacCloudBacking::Unknown
    };
    let content_availability = match backing_kind {
        MacCloudBacking::ICloud => i_cloud.content_availability,
        MacCloudBacking::FileProvider => match file_provider.materialization {
            file_provider::MacProviderMaterialization::FullyConsumable
            | file_provider::MacProviderMaterialization::ProviderNative => {
                MacContentAvailability::Local
            }
            file_provider::MacProviderMaterialization::BoundaryReadable => {
                MacContentAvailability::BoundaryReadable
            }
            _ => file_provider.content_availability,
        },
        MacCloudBacking::Local if metadata.is_file() || metadata.is_dir() => {
            MacContentAvailability::Local
        }
        MacCloudBacking::Local | MacCloudBacking::Unknown => MacContentAvailability::Unknown,
    };
    let is_regular_file = metadata.is_file();
    let is_directory = metadata.is_dir();
    let is_package = is_directory && package::is_package(path);
    let logical_size = is_regular_file.then_some(metadata.len());
    let allocated_size = (content_availability == MacContentAvailability::Local)
        .then(|| allocated_size(path, Some(metadata)))
        .flatten();
    MacFileSemantics {
        is_symlink: false,
        is_regular_file,
        is_package,
        backing_kind,
        content_availability,
        logical_size,
        allocated_size,
        volume,
        provider_identity_evidence: file_provider.identity_evidence,
    }
}

#[cfg(target_os = "macos")]
fn allocated_size(path: &Path, metadata: Option<&std::fs::Metadata>) -> Option<u64> {
    use objc2_foundation::{NSArray, NSNumber, NSString, NSURLTotalFileAllocatedSizeKey, NSURL};

    let foundation_size = path.to_str().and_then(|path| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let key = unsafe { NSURLTotalFileAllocatedSizeKey };
        let keys = NSArray::from_slice(&[key]);
        let values = url.resourceValuesForKeys_error(&keys).ok()?;
        values
            .objectForKey(key)
            .and_then(|value| value.downcast::<NSNumber>().ok())
            .and_then(|value| u64::try_from(value.as_i64()).ok())
    });
    foundation_size.or_else(|| {
        use std::os::unix::fs::MetadataExt;
        metadata?.blocks().checked_mul(512)
    })
}

#[cfg(not(target_os = "macos"))]
fn allocated_size(_path: &Path, _metadata: Option<&std::fs::Metadata>) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::{
        content_read_eligibility, inspect, inspect_with_metadata, MacContentReadEligibility,
    };
    use std::path::Path;

    #[test]
    fn missing_paths_fail_closed_without_local_byte_claims() {
        assert_eq!(
            content_read_eligibility(Path::new("/path/that/does/not/exist")),
            MacContentReadEligibility::PermissionRequired
        );
        let semantics = inspect(Path::new("/path/that/does/not/exist"));
        assert!(!semantics.is_package);
        assert!(!semantics.is_regular_file);
    }

    #[test]
    fn i_cloud_local_availability_is_readable_after_materialization_proof() {
        assert_eq!(
            MacContentReadEligibility::Eligible.reason(),
            "content_eligible"
        );
        assert!(MacContentReadEligibility::Eligible.is_eligible());
    }

    #[test]
    fn caller_supplied_metadata_preserves_file_semantics() {
        let root = std::env::temp_dir().join(format!(
            "fylura-file-semantics-metadata-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create metadata reuse fixture");
        let path = root.join("ordinary.txt");
        std::fs::write(&path, b"metadata").expect("write metadata reuse fixture");
        let metadata = std::fs::symlink_metadata(&path).expect("read fixture metadata once");
        let reused = inspect_with_metadata(&path, &metadata);
        let volume = super::volume::inspect(&root);
        let reused_with_volume =
            super::inspect_with_metadata_and_volume(&path, &metadata, Some(&volume));
        let direct = inspect(&path);
        assert_eq!(reused.is_symlink, direct.is_symlink);
        assert_eq!(reused.is_regular_file, direct.is_regular_file);
        assert_eq!(reused.is_package, direct.is_package);
        assert_eq!(reused.logical_size, direct.logical_size);
        assert_eq!(reused_with_volume.is_symlink, direct.is_symlink);
        assert_eq!(reused_with_volume.is_regular_file, direct.is_regular_file);
        assert_eq!(reused_with_volume.is_package, direct.is_package);
        assert_eq!(reused_with_volume.logical_size, direct.logical_size);
        std::fs::remove_dir_all(root).expect("remove metadata reuse fixture");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn local_byte_gate_rejects_symlinks_and_package_contents() {
        use std::fs;
        use std::io::Read;

        let root =
            std::env::temp_dir().join(format!("zen-canvas-content-gate-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create content gate fixture");
        let file = root.join("ordinary.txt");
        fs::write(&file, b"gate").expect("write content gate fixture");
        let mut opened = super::open_content_read(&file).expect("open ordinary fixture");
        let mut content = String::new();
        opened
            .read_to_string(&mut content)
            .expect("read ordinary fixture");
        assert_eq!(content, "gate");

        let link = root.join("ordinary-link.txt");
        std::os::unix::fs::symlink(&file, &link).expect("create symlink fixture");
        assert_eq!(
            content_read_eligibility(&link),
            MacContentReadEligibility::Symlink
        );
        assert_eq!(
            super::open_content_read(&link).err(),
            Some("content_symlink_traversal_blocked")
        );

        let package = root.join("Atomic.app");
        fs::create_dir_all(package.join("Contents/Resources")).expect("create package fixture");
        assert_eq!(
            content_read_eligibility(&package),
            MacContentReadEligibility::PackageUnsupported
        );

        fs::remove_dir_all(root).expect("remove content gate fixture");
    }
}
