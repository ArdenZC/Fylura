//! Metadata-only Cloud Files observations and a conservative local-byte gate.
use std::os::windows::{
    fs::{MetadataExt, OpenOptionsExt},
    io::AsRawHandle,
};
use std::{fs, io, path::Path};
use windows_sys::Win32::Storage::FileSystem::{
    FileAttributeTagInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_OFFLINE,
    FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_RECALL_ON_OPEN,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
};

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
    if !metadata.is_file() || metadata.file_type().is_symlink() || unsupported_reparse_handle(file)?
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cloud_or_link_content_not_supported",
        ));
    }
    Ok(())
}

fn open_cached_content(path: &Path, allow_cloud: bool) -> io::Result<fs::File> {
    fn check(metadata: &fs::Metadata, safe_cloud: bool) -> io::Result<()> {
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
        Ok(())
    }
    let metadata = fs::symlink_metadata(path)?;
    let safe_cloud = allow_cloud && !unsupported_reparse(path, &metadata);
    check(&metadata, safe_cloud)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL)
        .open(path)?;
    check(
        &file.metadata()?,
        allow_cloud && !unsupported_reparse_handle(&file)?,
    )?;
    Ok(file)
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
    }
}
