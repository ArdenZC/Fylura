//! Lightweight, read-only filesystem identity for File Library analysis.
//!
//! This module deliberately does not compute a content or sample hash.  The
//! mutation identity in `identity.rs` remains the authority for preview,
//! operation-journal and restore safety.  The value here is only used to
//! collapse path rows which point at the same physical object and to validate
//! the dedupe fingerprint cache.

use std::{fs, io, path::Path, time::UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalPlatform {
    Windows,
    MacOs,
    Unix,
    Other,
}

impl PhysicalPlatform {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::MacOs => "macos",
            Self::Unix => "unix",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalFileIdentity {
    pub size: u64,
    pub modified_ns: Option<i64>,
    pub platform_kind: PhysicalPlatform,
    pub platform_volume_id: Option<String>,
    pub platform_file_id: Option<String>,
    pub physical_key: Option<String>,
    pub link_count: Option<u64>,
}

#[derive(Debug, Error)]
pub enum PhysicalIdentityError {
    #[error("source_missing")]
    Missing,
    #[error("symlink_or_reparse_point")]
    UnsupportedLink,
    #[error("unsupported_file_type")]
    UnsupportedType,
    #[error("physical identity io: {0}")]
    Io(#[from] io::Error),
}

/// Capture the filesystem identity used to bind a managed scan to the volume
/// on which its selected root lives. This is transient scan context; it is not
/// persisted as a second root authority.
pub fn capture_source_volume_identity(
    path: &Path,
) -> Result<(PhysicalPlatform, String), PhysicalIdentityError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            PhysicalIdentityError::Missing
        } else {
            PhysicalIdentityError::Io(error)
        }
    })?;
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
        return Err(PhysicalIdentityError::UnsupportedLink);
    }
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(PhysicalIdentityError::UnsupportedType);
    }
    let (platform, volume_id, _, _) = platform_identity(path, &metadata);
    volume_id.map(|volume| (platform, volume)).ok_or_else(|| {
        PhysicalIdentityError::Io(io::Error::new(
            io::ErrorKind::Unsupported,
            "source volume identity unavailable",
        ))
    })
}

pub fn capture_physical_identity(
    path: &Path,
) -> Result<PhysicalFileIdentity, PhysicalIdentityError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            PhysicalIdentityError::Missing
        } else {
            PhysicalIdentityError::Io(error)
        }
    })?;
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
        return Err(PhysicalIdentityError::UnsupportedLink);
    }
    if !metadata.is_file() {
        return Err(PhysicalIdentityError::UnsupportedType);
    }

    let size = metadata.len();
    let modified_ns = modified_ns(&metadata);
    let (platform_kind, volume_id, file_id, link_count) = platform_identity(path, &metadata);
    let physical_key = match (&volume_id, &file_id) {
        (Some(volume), Some(file)) => Some(format!(
            "{}:v1:{}:{}",
            platform_kind.as_str(),
            stable_identity_component(volume),
            stable_identity_component(file)
        )),
        _ => None,
    };

    Ok(PhysicalFileIdentity {
        size,
        modified_ns,
        platform_kind,
        platform_volume_id: volume_id,
        platform_file_id: file_id,
        physical_key,
        link_count,
    })
}

fn stable_identity_component(value: &str) -> String {
    // Native IDs are numeric on the supported platforms.  Escaping the small
    // set of separators makes the stored key unambiguous even if a future
    // platform returns a textual identifier.
    value
        .replace('%', "%25")
        .replace(':', "%3A")
        .replace('/', "%2F")
        .replace('\\', "%5C")
}

fn modified_ns(metadata: &fs::Metadata) -> Option<i64> {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i128::try_from(duration.as_nanos()).ok())
        .and_then(|value| i64::try_from(value).ok())
}

#[cfg(unix)]
fn platform_identity(
    _path: &Path,
    metadata: &fs::Metadata,
) -> (
    PhysicalPlatform,
    Option<String>,
    Option<String>,
    Option<u64>,
) {
    use std::os::unix::fs::MetadataExt;

    (
        if cfg!(target_os = "macos") {
            PhysicalPlatform::MacOs
        } else {
            PhysicalPlatform::Unix
        },
        Some(metadata.dev().to_string()),
        Some(metadata.ino().to_string()),
        Some(metadata.nlink()),
    )
}

#[cfg(windows)]
fn platform_identity(
    path: &Path,
    _metadata: &fs::Metadata,
) -> (
    PhysicalPlatform,
    Option<String>,
    Option<String>,
    Option<u64>,
) {
    use std::fs::OpenOptions;
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT,
    };

    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path);
    let Ok(file) = file else {
        return (PhysicalPlatform::Windows, None, None, None);
    };
    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return (PhysicalPlatform::Windows, None, None, None);
    }
    let file_id = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    (
        PhysicalPlatform::Windows,
        Some(info.dwVolumeSerialNumber.to_string()),
        Some(file_id.to_string()),
        Some(u64::from(info.nNumberOfLinks)),
    )
}

#[cfg(not(any(unix, windows)))]
fn platform_identity(
    _path: &Path,
    _metadata: &fs::Metadata,
) -> (
    PhysicalPlatform,
    Option<String>,
    Option<String>,
    Option<u64>,
) {
    (PhysicalPlatform::Other, None, None, None)
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn identity_does_not_read_file_contents_and_is_stable_for_same_inode() {
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-physical-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("fixture");
        let first = root.join("first.bin");
        let alias = root.join("alias.bin");
        fs::write(&first, vec![7_u8; 8192]).expect("write");
        #[cfg(unix)]
        std::fs::hard_link(&first, &alias).expect("hardlink");
        #[cfg(windows)]
        std::fs::hard_link(&first, &alias).expect("hardlink");

        let left = capture_physical_identity(&first).expect("left identity");
        let right = capture_physical_identity(&alias).expect("right identity");
        assert_eq!(left.size, right.size);
        assert_eq!(left.physical_key, right.physical_key);
        assert_eq!(left.link_count, right.link_count);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_rejected_without_following_it() {
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-physical-link-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("fixture");
        let target = root.join("target");
        let link = root.join("link");
        fs::write(&target, b"fixture").expect("write");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        assert!(matches!(
            capture_physical_identity(&link),
            Err(PhysicalIdentityError::UnsupportedLink)
        ));
        let _ = fs::remove_dir_all(root);
    }
}
