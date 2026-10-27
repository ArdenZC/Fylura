use crate::global_index::coordinator::GlobalIndexError;
use crate::global_index::models::{
    GlobalSourceDescriptor, GlobalVolume, INDEX_STATUS_DISCOVERED, INDEX_STATUS_UNAVAILABLE,
    PROVIDER_WINDOWS_MFT_USN, PROVIDER_WINDOWS_UNSUPPORTED, SCOPE_AUTHORIZATION_NOT_AUTHORIZED,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW, GetVolumeNameForVolumeMountPointW,
};
use windows_sys::Win32::System::WindowsProgramming::{
    DRIVE_CDROM, DRIVE_FIXED, DRIVE_NO_ROOT_DIR, DRIVE_RAMDISK, DRIVE_REMOTE, DRIVE_REMOVABLE,
    DRIVE_UNKNOWN,
};

pub fn discover_windows_volumes() -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
    let drives = unsafe { GetLogicalDrives() };
    if drives == 0 {
        return Err(GlobalIndexError::Provider(
            "GetLogicalDrives failed".to_string(),
        ));
    }
    let mut sources = Vec::new();
    for index in 0..26u32 {
        if drives & (1 << index) == 0 {
            continue;
        }
        let letter = (b'A' + index as u8) as char;
        let mount_path = format!("{letter}:\\");
        let drive_kind_code = unsafe { GetDriveTypeW(to_wide(&mount_path).as_ptr()) };
        if matches!(drive_kind_code, DRIVE_NO_ROOT_DIR | DRIVE_UNKNOWN) {
            continue;
        }
        let Some(source) = describe_volume(&mount_path, drive_kind_code) else {
            continue;
        };
        sources.push(source);
    }
    Ok(sources)
}

fn describe_volume(mount_path: &str, drive_kind_code: u32) -> Option<GlobalSourceDescriptor> {
    let root = to_wide(mount_path);
    let mut volume_name = [0u16; 261];
    let mut filesystem_name = [0u16; 64];
    let mut serial_number = 0u32;
    let mut max_component_length = 0u32;
    let mut filesystem_flags = 0u32;
    let volume_ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            volume_name.as_mut_ptr(),
            volume_name.len() as u32,
            &mut serial_number,
            &mut max_component_length,
            &mut filesystem_flags,
            filesystem_name.as_mut_ptr(),
            filesystem_name.len() as u32,
        )
    } != 0;
    if !volume_ok {
        return None;
    }
    let volume_guid = volume_guid(mount_path);
    let stable_volume_id = volume_guid
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("serial:{serial_number:08x}"));
    let filesystem_type = wide_string(&filesystem_name);
    let is_ntfs = filesystem_type.eq_ignore_ascii_case("ntfs");
    let provider = if is_ntfs {
        PROVIDER_WINDOWS_MFT_USN
    } else {
        PROVIDER_WINDOWS_UNSUPPORTED
    };
    let now = crate::global_index::models::unix_now();
    Some(GlobalSourceDescriptor {
        volume: GlobalVolume {
            id: format!("gv_{}", blake3::hash(stable_volume_id.as_bytes()).to_hex()),
            platform: "windows".to_string(),
            stable_volume_id,
            display_name: {
                let label = wide_string(&volume_name);
                if label.is_empty() {
                    mount_path.trim_end_matches('\\').to_string()
                } else {
                    label
                }
            },
            mount_path: mount_path.to_string(),
            filesystem_type: filesystem_type.clone(),
            drive_kind: drive_kind(drive_kind_code).to_string(),
            enabled: source_enabled_by_default(drive_kind_code, is_ntfs),
            scope_authorization_state: SCOPE_AUTHORIZATION_NOT_AUTHORIZED.to_string(),
            provider: provider.to_string(),
            index_status: if is_ntfs {
                INDEX_STATUS_DISCOVERED.to_string()
            } else {
                INDEX_STATUS_UNAVAILABLE.to_string()
            },
            last_error: (!is_ntfs)
                .then(|| format!("windows_global_index_unsupported_filesystem:{filesystem_type}")),
            journal_id: None,
            journal_cursor: None,
            last_full_index_at: None,
            last_incremental_sync_at: None,
            entry_count: 0,
            created_at: now,
            updated_at: now,
        },
    })
}

fn source_enabled_by_default(_drive_kind_code: u32, _native_provider_supported: bool) -> bool {
    false
}

fn volume_guid(mount_path: &str) -> Option<String> {
    let root = to_wide(mount_path);
    let mut buffer = [0u16; 51];
    let success = unsafe {
        GetVolumeNameForVolumeMountPointW(root.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32)
    } != 0;
    success.then(|| wide_string(&buffer))
}

fn drive_kind(value: u32) -> &'static str {
    match value {
        DRIVE_FIXED => "fixed",
        DRIVE_REMOVABLE => "removable",
        DRIVE_REMOTE => "network",
        DRIVE_CDROM => "optical",
        DRIVE_RAMDISK => "ramdisk",
        _ => "unknown",
    }
}

pub fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn wide_string(value: &[u16]) -> String {
    let end = value
        .iter()
        .position(|item| *item == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end])
}

pub fn volume_device_path(mount_path: &str) -> String {
    let trimmed = mount_path.trim_end_matches(['\\', '/']);
    format!(r"\\.\{}", trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_device_path_uses_nt_namespace() {
        assert_eq!(volume_device_path("C:\\"), r"\\.\C:");
    }

    #[test]
    fn wide_string_ignores_trailing_buffer() {
        assert_eq!(wide_string(&[b'A' as u16, 0, b'B' as u16]), "A");
    }

    #[test]
    fn windows_volumes_wait_for_explicit_authorization() {
        assert!(!source_enabled_by_default(DRIVE_FIXED, true));
        assert!(!source_enabled_by_default(DRIVE_FIXED, false));
        assert!(!source_enabled_by_default(DRIVE_REMOVABLE, true));
        assert!(!source_enabled_by_default(DRIVE_REMOTE, true));
    }
}
