use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    sync::atomic::{AtomicBool, Ordering as AtomicOrdering},
    time::UNIX_EPOCH,
};
use thiserror::Error;

const SAMPLE_SIZE: u64 = 1024 * 1024;
const HASH_BUFFER_SIZE: usize = 1024 * 1024;
pub const DIRECTORY_IDENTITY_ENTRY_LIMIT: usize = 5000;
pub const DIRECTORY_IDENTITY_DEPTH_LIMIT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExpectedFileIdentity {
    pub size: u64,
    pub modified_ns: Option<i128>,
    pub platform_volume_id: Option<String>,
    pub platform_file_id: Option<String>,
    pub sample_hash: Option<String>,
    pub full_hash: Option<String>,
}

/// Namespace facts used to bind a mutation to one filesystem object without
/// reading its bytes.  Content verification is intentionally a separate
/// decision because rename, move, trash, restore, and delete do not need an
/// O(file-size) read before their PREPARED journal row is durable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceIdentity {
    pub size: u64,
    pub modified_ns: Option<i128>,
    pub platform_volume_id: Option<String>,
    pub platform_file_id: Option<String>,
}

/// Optional byte-equivalence evidence used by copy/duplicate, cross-volume
/// staging, and recovery.  A missing hash is meaningful: it says that this
/// operation has not requested content verification yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentVerificationIdentity {
    pub sample_hash: Option<String>,
    pub full_hash: Option<String>,
}

impl NamespaceIdentity {
    fn into_expected(self) -> ExpectedFileIdentity {
        ExpectedFileIdentity {
            size: self.size,
            modified_ns: self.modified_ns,
            platform_volume_id: self.platform_volume_id,
            platform_file_id: self.platform_file_id,
            sample_hash: None,
            full_hash: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("source_missing")]
    SourceMissing,
    #[error("symlink_or_reparse_point")]
    Symlink,
    #[error("unsupported_file_type")]
    UnsupportedFileType,
    #[error("directory_manifest_name_encoding_failed")]
    DirectoryManifestNameEncodingFailed,
    #[error("identity_cancelled")]
    Cancelled,
    #[error("content_read_rejected: {0}")]
    ContentReadRejected(&'static str),
    #[error("io: {0}")]
    Io(#[from] io::Error),
}

pub fn capture_identity(
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<ExpectedFileIdentity, IdentityError> {
    ensure_supported_entry(path)?;
    if is_cancelled(cancel) {
        return Err(IdentityError::Cancelled);
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            IdentityError::SourceMissing
        } else {
            IdentityError::Io(error)
        }
    })?;
    let (size, sample_hash, full_hash) = if metadata.is_file() {
        hash_file(path, metadata.len(), cancel)?
    } else if metadata.is_dir() {
        hash_directory(path, cancel)?
    } else {
        return Err(IdentityError::UnsupportedFileType);
    };
    Ok(ExpectedFileIdentity {
        size,
        modified_ns: modified_ns(&metadata),
        platform_volume_id: platform_volume_id(path, &metadata),
        platform_file_id: platform_file_id(path, &metadata),
        sample_hash: Some(sample_hash),
        full_hash: Some(full_hash),
    })
}

/// Captures only namespace metadata.  Unlike `capture_identity`, this never
/// opens a regular file for content reads and never walks a directory tree.
pub fn capture_namespace_identity_only(
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<ExpectedFileIdentity, IdentityError> {
    if is_cancelled(cancel) {
        return Err(IdentityError::Cancelled);
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            IdentityError::SourceMissing
        } else {
            IdentityError::Io(error)
        }
    })?;
    if !(metadata.is_file() || metadata.is_dir() || metadata.file_type().is_symlink()) {
        return Err(IdentityError::UnsupportedFileType);
    }
    Ok(NamespaceIdentity {
        size: metadata.len(),
        modified_ns: modified_ns(&metadata),
        platform_volume_id: platform_volume_id(path, &metadata),
        platform_file_id: platform_file_id(path, &metadata),
    }
    .into_expected())
}

pub fn identity_matches(expected: &ExpectedFileIdentity, actual: &ExpectedFileIdentity) -> bool {
    identity_components_match(expected, actual)
        && modified_time_matches(expected.modified_ns, actual.modified_ns)
}

/// Recovery can observe a filesystem move whose timestamp was refreshed by
/// the operating system or by the crash-recovery fixture itself. Permit only
/// a small forward adjustment while still rejecting stale or older replacements.
pub fn recovery_identity_matches(
    expected: &ExpectedFileIdentity,
    actual: &ExpectedFileIdentity,
) -> bool {
    identity_components_match(expected, actual)
        && recovery_modified_time_matches(expected.modified_ns, actual.modified_ns)
}

fn identity_components_match(
    expected: &ExpectedFileIdentity,
    actual: &ExpectedFileIdentity,
) -> bool {
    let platform_matches = optional_identity_field_matches(
        expected.platform_volume_id.as_deref(),
        actual.platform_volume_id.as_deref(),
    ) && optional_identity_field_matches(
        expected.platform_file_id.as_deref(),
        actual.platform_file_id.as_deref(),
    );
    let content_matches = optional_identity_field_matches(
        expected.sample_hash.as_deref(),
        actual.sample_hash.as_deref(),
    ) && optional_identity_field_matches(
        expected.full_hash.as_deref(),
        actual.full_hash.as_deref(),
    );
    expected.size == actual.size && platform_matches && content_matches
}

fn modified_time_matches(expected: Option<i128>, actual: Option<i128>) -> bool {
    expected.is_none_or(|expected| actual == Some(expected))
}

fn recovery_modified_time_matches(expected: Option<i128>, actual: Option<i128>) -> bool {
    const MAX_FORWARD_ADJUSTMENT_NS: i128 = 5_000_000_000;

    match (expected, actual) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(expected), Some(actual)) => {
            actual == expected
                || (actual > expected && actual - expected <= MAX_FORWARD_ADJUSTMENT_NS)
        }
    }
}

fn optional_identity_field_matches(expected: Option<&str>, actual: Option<&str>) -> bool {
    expected.is_none() || expected == actual
}

pub fn content_identity_matches(
    expected: &ExpectedFileIdentity,
    actual: &ExpectedFileIdentity,
) -> bool {
    expected.size == actual.size
        && expected
            .sample_hash
            .as_deref()
            .is_none_or(|expected| actual.sample_hash.as_deref() == Some(expected))
        && expected
            .full_hash
            .as_deref()
            .is_none_or(|expected| actual.full_hash.as_deref() == Some(expected))
}

pub fn ensure_supported_entry(path: &Path) -> Result<(), IdentityError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            IdentityError::SourceMissing
        } else {
            IdentityError::Io(error)
        }
    })?;
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
        return Err(IdentityError::Symlink);
    }
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(IdentityError::UnsupportedFileType);
    }
    Ok(())
}

fn hash_file(
    path: &Path,
    size: u64,
    cancel: Option<&AtomicBool>,
) -> Result<(u64, String, String), IdentityError> {
    #[cfg(target_os = "macos")]
    {
        let file = crate::platform::macos::file_semantics::open_content_read(path)
            .map_err(IdentityError::ContentReadRejected)?;
        hash_file_reader(file, size, cancel)
    }

    #[cfg(not(target_os = "macos"))]
    {
        hash_file_reader(File::open(path)?, size, cancel)
    }
}

/// Captures the physical identity of a namespace object without following a
/// final symlink.  Regular files and directories retain the existing complete
/// content identity; a symlink is represented by its lstat size/time and
/// platform device/inode because following its target would violate mutation
/// semantics.
pub fn capture_namespace_identity(
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<ExpectedFileIdentity, IdentityError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            IdentityError::SourceMissing
        } else {
            IdentityError::Io(error)
        }
    })?;
    if metadata.file_type().is_symlink() {
        #[cfg(not(target_os = "macos"))]
        return Err(IdentityError::Symlink);
        #[cfg(target_os = "macos")]
        {
            if is_cancelled(cancel) {
                return Err(IdentityError::Cancelled);
            }
            let target = fs::read_link(path)?;
            let target = raw_os_name_bytes(target.as_os_str())?;
            let mut hasher = blake3::Hasher::new();
            hasher.update(b"symlink\0");
            hasher.update(&(target.len() as u64).to_le_bytes());
            hasher.update(&target);
            let hash = hasher.finalize().to_hex().to_string();
            return Ok(ExpectedFileIdentity {
                size: metadata.len(),
                modified_ns: modified_ns(&metadata),
                platform_volume_id: platform_volume_id(path, &metadata),
                platform_file_id: platform_file_id(path, &metadata),
                sample_hash: Some(hash.clone()),
                full_hash: Some(hash),
            });
        }
    }
    capture_identity(path, cancel)
}

fn hash_file_reader(
    mut file: File,
    size: u64,
    cancel: Option<&AtomicBool>,
) -> Result<(u64, String, String), IdentityError> {
    file.seek(SeekFrom::Start(0))?;
    let mut full_hasher = blake3::Hasher::new();
    full_hasher.update(b"file\0");
    full_hasher.update(&size.to_le_bytes());
    let mut first = Vec::with_capacity(SAMPLE_SIZE.min(size) as usize);
    let mut small = if size <= SAMPLE_SIZE * 2 {
        Some(Vec::with_capacity(size as usize))
    } else {
        None
    };
    let mut buffer = vec![0_u8; HASH_BUFFER_SIZE];
    loop {
        if is_cancelled(cancel) {
            return Err(IdentityError::Cancelled);
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let chunk = &buffer[..read];
        full_hasher.update(chunk);
        if first.len() < SAMPLE_SIZE as usize {
            let take = (SAMPLE_SIZE as usize - first.len()).min(read);
            first.extend_from_slice(&chunk[..take]);
        }
        if let Some(small) = small.as_mut() {
            small.extend_from_slice(chunk);
        }
    }
    let mut sample_hasher = blake3::Hasher::new();
    sample_hasher.update(b"sample-file\0");
    sample_hasher.update(&size.to_le_bytes());
    if let Some(small) = small {
        sample_hasher.update(&small);
    } else {
        sample_hasher.update(&first);
        file.seek(SeekFrom::End(-(SAMPLE_SIZE as i64)))?;
        let mut last = vec![0_u8; SAMPLE_SIZE as usize];
        file.read_exact(&mut last)?;
        sample_hasher.update(&last);
    }
    Ok((
        size,
        sample_hasher.finalize().to_hex().to_string(),
        full_hasher.finalize().to_hex().to_string(),
    ))
}

pub(crate) fn capture_identity_from_handle(
    handle: &File,
    path_hint: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<ExpectedFileIdentity, IdentityError> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };

        let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
        if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &mut info) } == 0 {
            return Err(IdentityError::Io(io::Error::last_os_error()));
        }
        let file_id =
            (u64::from(info.nFileIndexHigh) << 32 | u64::from(info.nFileIndexLow)).to_string();
        let volume_id = info.dwVolumeSerialNumber.to_string();
        let metadata = fs::metadata(path_hint)?;
        let is_directory = info.dwFileAttributes
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY
            != 0;
        let (size, sample_hash, full_hash) = if is_directory {
            hash_directory(path_hint, cancel)?
        } else {
            let size = (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow);
            hash_file_reader(handle.try_clone()?, size, cancel)?
        };
        Ok(ExpectedFileIdentity {
            size,
            modified_ns: modified_ns(&metadata),
            platform_volume_id: Some(volume_id),
            platform_file_id: Some(file_id),
            sample_hash: Some(sample_hash),
            full_hash: Some(full_hash),
        })
    }

    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::MetadataExt;

        let metadata = handle.metadata()?;
        let (size, sample_hash, full_hash) = if metadata.is_file() {
            hash_file_reader(handle.try_clone()?, metadata.len(), cancel)?
        } else if metadata.is_dir() {
            hash_directory(path_hint, cancel)?
        } else {
            return Err(IdentityError::UnsupportedFileType);
        };
        Ok(ExpectedFileIdentity {
            size,
            modified_ns: modified_ns(&metadata),
            platform_volume_id: Some(metadata.dev().to_string()),
            platform_file_id: Some(metadata.ino().to_string()),
            sample_hash: Some(sample_hash),
            full_hash: Some(full_hash),
        })
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = handle;
        capture_identity(path_hint, cancel)
    }
}

/// Captures namespace metadata from a retained source descriptor without
/// reading content.  The path hint is used only for the non-descriptor
/// fallback and for platform metadata that is not exposed by the standard
/// descriptor API.
pub(crate) fn capture_namespace_identity_from_handle(
    handle: &File,
    _path_hint: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<ExpectedFileIdentity, IdentityError> {
    if is_cancelled(cancel) {
        return Err(IdentityError::Cancelled);
    }

    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };

        let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
        if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &mut info) } == 0 {
            return Err(IdentityError::Io(io::Error::last_os_error()));
        }
        let metadata = handle.metadata()?;
        Ok(NamespaceIdentity {
            size: (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow),
            modified_ns: modified_ns(&metadata),
            platform_volume_id: Some(info.dwVolumeSerialNumber.to_string()),
            platform_file_id: Some(
                (u64::from(info.nFileIndexHigh) << 32 | u64::from(info.nFileIndexLow)).to_string(),
            ),
        }
        .into_expected())
    }

    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = handle.metadata()?;
        Ok(NamespaceIdentity {
            size: metadata.len(),
            modified_ns: modified_ns(&metadata),
            platform_volume_id: Some(metadata.dev().to_string()),
            platform_file_id: Some(metadata.ino().to_string()),
        }
        .into_expected())
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = handle;
        capture_namespace_identity_only(_path_hint, cancel)
    }
}

fn hash_directory(
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<(u64, String, String), IdentityError> {
    let metadata = fs::symlink_metadata(path)?;
    let root_volume = platform_volume_id(path, &metadata);
    let mut entry_count = 0usize;
    hash_directory_at(path, cancel, 0, root_volume.as_deref(), &mut entry_count)
}

fn hash_directory_at(
    path: &Path,
    cancel: Option<&AtomicBool>,
    depth: usize,
    root_volume: Option<&str>,
    entry_count: &mut usize,
) -> Result<(u64, String, String), IdentityError> {
    if depth >= DIRECTORY_IDENTITY_DEPTH_LIMIT {
        return Err(IdentityError::ContentReadRejected(
            "directory_depth_limit_exceeded",
        ));
    }
    let mut entries_with_names = Vec::new();
    for entry in fs::read_dir(path)? {
        if is_cancelled(cancel) {
            return Err(IdentityError::Cancelled);
        }
        if *entry_count >= DIRECTORY_IDENTITY_ENTRY_LIMIT {
            return Err(IdentityError::ContentReadRejected(
                "directory_entry_limit_exceeded",
            ));
        }
        *entry_count += 1;
        let entry = entry?;
        let name = entry.file_name();
        let raw_name = raw_os_name_bytes(&name)?;
        entries_with_names.push((raw_name, entry));
    }
    entries_with_names.sort_by(|(left_name, left), (right_name, right)| {
        left_name
            .cmp(right_name)
            .then_with(|| left.path().cmp(&right.path()))
    });
    let mut size = 0_u64;
    let mut sample_hasher = blake3::Hasher::new();
    let mut full_hasher = blake3::Hasher::new();
    sample_hasher.update(b"directory\0");
    full_hasher.update(b"directory\0");
    for (raw_name, entry) in entries_with_names {
        if is_cancelled(cancel) {
            return Err(IdentityError::Cancelled);
        }
        let child_path = entry.path();
        let child_metadata = fs::symlink_metadata(&child_path)?;
        let child_volume = platform_volume_id(&child_path, &child_metadata);
        if root_volume.is_some() && child_volume.as_deref() != root_volume {
            return Err(IdentityError::ContentReadRejected(
                "nested_volume_entry_unsupported",
            ));
        }
        let child = if child_metadata.is_dir() {
            let (size, sample_hash, full_hash) =
                hash_directory_at(&child_path, cancel, depth + 1, root_volume, entry_count)?;
            ExpectedFileIdentity {
                size,
                modified_ns: modified_ns(&child_metadata),
                platform_volume_id: child_volume,
                platform_file_id: platform_file_id(&child_path, &child_metadata),
                sample_hash: Some(sample_hash),
                full_hash: Some(full_hash),
            }
        } else {
            capture_namespace_identity(&child_path, cancel)?
        };
        let kind = if child_metadata.file_type().is_symlink() {
            b"symlink\0".as_slice()
        } else if child_metadata.is_dir() {
            b"dir\0".as_slice()
        } else {
            b"file\0".as_slice()
        };
        size = size.saturating_add(child.size);
        update_directory_entry(
            &mut sample_hasher,
            &raw_name,
            kind,
            child.size,
            child.sample_hash.as_deref().unwrap_or_default().as_bytes(),
        );
        update_directory_entry(
            &mut full_hasher,
            &raw_name,
            kind,
            child.size,
            child.full_hash.as_deref().unwrap_or_default().as_bytes(),
        );
    }
    Ok((
        size,
        sample_hasher.finalize().to_hex().to_string(),
        full_hasher.finalize().to_hex().to_string(),
    ))
}

fn update_directory_entry(
    hasher: &mut blake3::Hasher,
    raw_name: &[u8],
    kind: &[u8],
    size: u64,
    child_hash: &[u8],
) {
    hasher.update(b"entry\0");
    hasher.update(&(raw_name.len() as u64).to_le_bytes());
    hasher.update(raw_name);
    hasher.update(&(kind.len() as u64).to_le_bytes());
    hasher.update(kind);
    hasher.update(&size.to_le_bytes());
    hasher.update(&(child_hash.len() as u64).to_le_bytes());
    hasher.update(child_hash);
}

fn raw_os_name_bytes(name: &OsStr) -> Result<Vec<u8>, IdentityError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        return Ok(name.as_bytes().to_vec());
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        return Ok(name
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>());
    }

    #[allow(unreachable_code)]
    Err(IdentityError::DirectoryManifestNameEncodingFailed)
}

fn modified_ns(metadata: &fs::Metadata) -> Option<i128> {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_nanos() as i128)
}

fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(AtomicOrdering::Acquire))
}

#[cfg(unix)]
fn platform_volume_id(_path: &Path, metadata: &fs::Metadata) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.dev().to_string())
}

#[cfg(windows)]
fn platform_volume_id(path: &Path, _metadata: &fs::Metadata) -> Option<String> {
    windows_file_identity(path).map(|(volume, _)| volume)
}

#[cfg(not(any(unix, windows)))]
fn platform_volume_id(_path: &Path, _metadata: &fs::Metadata) -> Option<String> {
    None
}

#[cfg(unix)]
fn platform_file_id(_path: &Path, metadata: &fs::Metadata) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.ino().to_string())
}

#[cfg(windows)]
fn platform_file_id(path: &Path, _metadata: &fs::Metadata) -> Option<String> {
    windows_file_identity(path).map(|(_, file)| file)
}

#[cfg(not(any(unix, windows)))]
fn platform_file_id(_path: &Path, _metadata: &fs::Metadata) -> Option<String> {
    None
}

#[cfg(windows)]
fn windows_file_identity(path: &Path) -> Option<(String, String)> {
    use std::fs::OpenOptions;
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT,
    };
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    let success = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) };
    if success == 0 {
        return None;
    }
    let file_index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Some((
        info.dwVolumeSerialNumber.to_string(),
        file_index.to_string(),
    ))
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
    use std::{fs, path::PathBuf, sync::atomic::AtomicBool};

    fn fixture(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "zen-canvas-identity-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&path).expect("fixture");
        path
    }

    #[test]
    fn full_hash_detects_middle_change_when_sample_hash_matches() {
        let root = fixture("middle-change");
        let path = root.join("large.bin");
        let mut bytes = vec![b'a'; (SAMPLE_SIZE as usize * 3).max(3_000_000)];
        fs::write(&path, &bytes).expect("write");
        let first = capture_identity(&path, None).expect("identity");
        let middle = bytes.len() / 2;
        bytes[middle] = b'b';
        fs::write(&path, &bytes).expect("rewrite");
        let second = capture_identity(&path, None).expect("identity");
        assert_eq!(first.sample_hash, second.sample_hash);
        assert_ne!(first.full_hash, second.full_hash);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cancellation_is_fail_closed() {
        let root = fixture("cancel");
        let path = root.join("file");
        fs::write(&path, b"fixture").expect("write");
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            capture_identity(&path, Some(&cancel)),
            Err(IdentityError::Cancelled)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn namespace_identity_defers_content_verification() {
        let root = fixture("namespace-only");
        let path = root.join("file");
        fs::write(
            &path,
            b"content that must not be hashed for namespace identity",
        )
        .expect("write");

        let identity = capture_namespace_identity_only(&path, None).expect("namespace identity");

        assert!(identity.sample_hash.is_none());
        assert!(identity.full_hash.is_none());
        assert_eq!(identity.size, fs::metadata(&path).expect("metadata").len());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn directory_identity_stops_at_the_bounded_entry_limit() {
        let root = fixture("entry-limit");
        for index in 0..=DIRECTORY_IDENTITY_ENTRY_LIMIT {
            fs::write(root.join(format!("entry-{index:04}")), b"").expect("entry");
        }

        assert!(matches!(
            capture_identity(&root, None),
            Err(IdentityError::ContentReadRejected(
                "directory_entry_limit_exceeded"
            ))
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn directory_identity_stops_at_the_bounded_depth_limit() {
        let root = fixture("depth-limit");
        let mut current = root.clone();
        for _ in 0..DIRECTORY_IDENTITY_DEPTH_LIMIT {
            current = current.join("d");
            fs::create_dir(&current).expect("nested directory");
        }

        assert!(matches!(
            capture_identity(&root, None),
            Err(IdentityError::ContentReadRejected(
                "directory_depth_limit_exceeded"
            ))
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn directory_manifest_preserves_non_utf8_names_without_lossy_collisions() {
        use std::os::unix::ffi::OsStringExt;

        let root = fixture("raw-directory-names");
        let invalid = root.join("invalid");
        let replacement = root.join("replacement");
        fs::create_dir_all(&invalid).expect("invalid fixture");
        fs::create_dir_all(&replacement).expect("replacement fixture");
        fs::write(
            invalid.join(std::ffi::OsString::from_vec(vec![0xff])),
            b"same",
        )
        .expect("invalid name");
        fs::write(replacement.join("\u{fffd}"), b"same").expect("replacement name");

        let invalid_identity = capture_identity(&invalid, None).expect("invalid identity");
        let replacement_identity =
            capture_identity(&replacement, None).expect("replacement identity");
        assert_ne!(invalid_identity.full_hash, replacement_identity.full_hash);

        let _ = fs::remove_dir_all(root);
    }
}
