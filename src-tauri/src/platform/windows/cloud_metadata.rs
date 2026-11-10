//! Metadata-only Cloud Files observations and a conservative local-byte gate.
use std::os::windows::{
    fs::{MetadataExt, OpenOptionsExt},
    io::AsRawHandle,
};
use std::{fs, io, path::Path};
use windows_sys::Win32::Storage::CloudFilters::{
    CfGetPlaceholderStateFromAttributeTag, CF_PLACEHOLDER_STATE_INVALID,
    CF_PLACEHOLDER_STATE_PARTIAL, CF_PLACEHOLDER_STATE_PARTIALLY_ON_DISK,
    CF_PLACEHOLDER_STATE_PLACEHOLDER,
};
use windows_sys::Win32::Storage::FileSystem::{
    FileAttributeTagInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_OFFLINE,
    FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_RECALL_ON_OPEN,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
};

const CF_CONTENT_NOT_READY: u32 =
    CF_PLACEHOLDER_STATE_PARTIAL | CF_PLACEHOLDER_STATE_PARTIALLY_ON_DISK;

pub fn needs_download(attributes: u32) -> bool {
    attributes
        & (FILE_ATTRIBUTE_OFFLINE
            | FILE_ATTRIBUTE_RECALL_ON_OPEN
            | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
        != 0
}

pub fn is_cloud_tag(tag: u32) -> bool {
    // CLOUD and CLOUD_1..CLOUD_F are non-name-surrogate provider tags.
    tag & !0x0000_f000 == 0x9000_001a
}

pub fn reparse_tag(path: &Path) -> io::Result<u32> {
    let file = fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL,
        )
        .open(path)?;
    handle_tag(&file)
}

fn attribute_tag_info(file: &fs::File) -> io::Result<FILE_ATTRIBUTE_TAG_INFO> {
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of_val(&info) as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

fn placeholder_state(attributes: u32, tag: u32) -> u32 {
    unsafe { CfGetPlaceholderStateFromAttributeTag(attributes, tag) }
}

pub fn placeholder_state_for_path(path: &Path, metadata: &fs::Metadata) -> io::Result<u32> {
    let tag = reparse_tag(path)?;
    let state = placeholder_state(metadata.file_attributes(), tag);
    if state == CF_PLACEHOLDER_STATE_INVALID {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "cloud_file_state_invalid",
        ));
    }
    Ok(state)
}

fn ensure_placeholder_state_ready(state: u32) -> io::Result<()> {
    if state == CF_PLACEHOLDER_STATE_INVALID {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "cloud_file_state_invalid",
        ));
    }
    if state & CF_CONTENT_NOT_READY != 0 {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "cloud_file_requires_download",
        ));
    }
    Ok(())
}

pub fn ensure_placeholder_availability(state: u32) -> io::Result<()> {
    if state & CF_PLACEHOLDER_STATE_PLACEHOLDER == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "cloud_file_state_invalid",
        ));
    }
    ensure_placeholder_state_ready(state)
}

pub fn handle_tag(file: &fs::File) -> io::Result<u32> {
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of_val(&info) as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(info.ReparseTag)
}

/// Known Cloud Files objects do not redirect names. Symlinks, junctions,
/// unknown provider tags and a failed native probe remain unsupported.
pub fn unsupported_reparse(path: &Path, metadata: &fs::Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        && !reparse_tag(path).is_ok_and(is_cloud_tag)
}

pub fn unsupported_reparse_handle(file: &fs::File) -> io::Result<bool> {
    Ok(
        file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            && !is_cloud_tag(handle_tag(file)?),
    )
}

pub fn metadata_needs_download(metadata: &fs::Metadata) -> bool {
    needs_download(metadata.file_attributes())
}

/// Metadata projection used before any content path is opened. CFAPI PARTIAL
/// includes verification-pending placeholders whose bytes may already all be
/// on disk, so attributes alone are insufficient. Query errors fail closed.
pub fn path_needs_download(path: &Path, metadata: &fs::Metadata) -> bool {
    if metadata_needs_download(metadata) {
        return true;
    }
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
        return false;
    }
    let Ok(tag) = reparse_tag(path) else {
        return true;
    };
    if !is_cloud_tag(tag) {
        return false;
    }
    let state = placeholder_state(metadata.file_attributes(), tag);
    ensure_placeholder_availability(state).is_err()
}

/// Only ordinary cached files. Visibility of a provider reparse object does
/// not grant byte access or materialization consent. Check the opened handle
/// again before returning a reader to Preview, content analysis or hashing.
pub fn open_local_content(path: &Path) -> io::Result<fs::File> {
    open_cached_content(path, false)
}

/// Explicit file-operation content access only. The copy engine can consume
/// a locally cached Cloud Files data stream, never a cloud reparse payload.
/// Pinning/sparse layout is provider state, not permission to recall bytes.
pub fn open_transfer_content(path: &Path) -> io::Result<fs::File> {
    open_cached_content(path, true)
}

pub fn ensure_transfer_content(file: &fs::File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if metadata_needs_download(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "cloud_file_requires_download",
        ));
    }
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cloud_or_link_content_not_supported",
        ));
    }
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        let info = attribute_tag_info(file)?;
        if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cloud_file_state_changed",
            ));
        }
        if !is_cloud_tag(info.ReparseTag) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "cloud_or_link_content_not_supported",
            ));
        }
        let state = placeholder_state(info.FileAttributes, info.ReparseTag);
        ensure_placeholder_availability(state)?;
    }
    Ok(())
}

/// Automatic readers (Preview, analysis and hashing) only accept ordinary
/// local files. Rechecking the retained handle around a read closes the race
/// where a provider converts or dehydrates the object after it was opened.
pub fn ensure_local_content(file: &fs::File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if metadata_needs_download(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "cloud_file_requires_download",
        ));
    }
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cloud_or_link_content_not_supported",
        ));
    }
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        let info = attribute_tag_info(file)?;
        if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "cloud_or_link_content_not_supported",
            ));
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "cloud_file_state_changed",
        ));
    }
    Ok(())
}

/// Run one bounded/streaming content operation with local-only state checks
/// both before and after the read. FILE_FLAG_OPEN_NO_RECALL keeps a concurrent
/// transition from hydrating remote bytes; the post-check suppresses output if
/// the provider state changed while the operation was in progress.
pub fn with_local_content_read<T, E>(
    file: &fs::File,
    read: impl FnOnce() -> Result<T, E>,
) -> Result<T, E>
where
    E: From<io::Error>,
{
    with_transfer_state_checks(|| ensure_local_content(file), read)
}

/// Explicit Copy/Move reads may consume a fully cached provider placeholder,
/// but never a partial or verification-pending file.
pub fn with_transfer_content_read<T, E>(
    file: &fs::File,
    read: impl FnOnce() -> Result<T, E>,
) -> Result<T, E>
where
    E: From<io::Error>,
{
    with_transfer_state_checks(|| ensure_transfer_content(file), read)
}

fn open_cached_content(path: &Path, allow_cloud: bool) -> io::Result<fs::File> {
    fn check(path: &Path, metadata: &fs::Metadata, safe_cloud: bool) -> io::Result<()> {
        if metadata_needs_download(metadata) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "cloud_file_requires_download",
            ));
        }
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 && !safe_cloud
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "cloud_or_link_content_not_supported",
            ));
        }
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            let state = placeholder_state_for_path(path, metadata)?;
            ensure_placeholder_availability(state)?;
        }
        Ok(())
    }
    let metadata = fs::symlink_metadata(path)?;
    let safe_cloud = allow_cloud && !unsupported_reparse(path, &metadata);
    check(path, &metadata, safe_cloud)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL)
        .open(path)?;
    check(
        path,
        &file.metadata()?,
        allow_cloud && !unsupported_reparse_handle(&file)?,
    )?;
    if allow_cloud {
        ensure_transfer_content(&file)?;
    } else {
        ensure_local_content(&file)?;
    }
    Ok(file)
}

fn with_transfer_state_checks<T, E>(
    mut check: impl FnMut() -> io::Result<()>,
    read: impl FnOnce() -> Result<T, E>,
) -> Result<T, E>
where
    E: From<io::Error>,
{
    check().map_err(E::from)?;
    let value = read()?;
    check().map_err(E::from)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cloud_flags_do_not_confuse_links_or_pinning_with_content_availability() {
        for variant in 0..16 {
            assert!(is_cloud_tag(0x9000_001a | variant << 12));
        }
        for tag in [0xa000_0003, 0xa000_000c, 0x8000_001b, 0] {
            assert!(!is_cloud_tag(tag));
        }
        assert!(needs_download(FILE_ATTRIBUTE_OFFLINE));
        assert!(needs_download(FILE_ATTRIBUTE_RECALL_ON_OPEN));
        assert!(needs_download(FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS));
        assert!(!needs_download(0x0008_0000)); // PINNED is not a download state.
        assert!(ensure_placeholder_availability(CF_PLACEHOLDER_STATE_PLACEHOLDER).is_ok());
        assert_eq!(
            ensure_placeholder_availability(
                CF_PLACEHOLDER_STATE_PLACEHOLDER | CF_PLACEHOLDER_STATE_PARTIAL
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(
            ensure_placeholder_availability(
                CF_PLACEHOLDER_STATE_PLACEHOLDER | CF_PLACEHOLDER_STATE_PARTIALLY_ON_DISK
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(
            ensure_placeholder_state_ready(CF_PLACEHOLDER_STATE_INVALID)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn rejects_provider_state_change_after_read_without_publishing_bytes() {
        use std::cell::Cell;

        let observations = [
            CF_PLACEHOLDER_STATE_PLACEHOLDER,
            CF_PLACEHOLDER_STATE_PLACEHOLDER | CF_PLACEHOLDER_STATE_PARTIAL,
        ];
        let index = Cell::new(0);
        let read_started = Cell::new(false);
        let result = with_transfer_state_checks(
            || {
                let state = observations[index.get().min(observations.len() - 1)];
                index.set(index.get() + 1);
                ensure_placeholder_state_ready(state)
            },
            || {
                read_started.set(true);
                Ok::<_, io::Error>(b"must not escape".to_vec())
            },
        );
        assert!(read_started.get());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WouldBlock);
    }
}
