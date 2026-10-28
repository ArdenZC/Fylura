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

/// `FILE_SUPPORTS_USN_JOURNAL` — reported by `GetVolumeInformationW` for a
/// filesystem that can host a USN change journal. This is a capability probe,
/// not a name match: it distinguishes a real NTFS volume from a virtual/cloud
/// mount (e.g. Google Drive for desktop) that may advertise NTFS-like metadata
/// over a local cache while being unable to host a journal at all.
const FILE_SUPPORTS_USN_JOURNAL: u32 = 0x0200_0000;

/// Identity + capability decision for one discovered volume.
///
/// Pure so it can be exercised deterministically without touching real drives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VolumeClassification {
    pub(crate) stable_volume_id: String,
    pub(crate) provider: &'static str,
    pub(crate) native_provider_eligible: bool,
    pub(crate) unsupported_reason: Option<String>,
}

pub(crate) fn classify_volume(
    volume_guid: Option<&str>,
    serial_number: u32,
    filesystem_type: &str,
    drive_kind_code: u32,
    filesystem_flags: u32,
) -> VolumeClassification {
    let trusted_guid = volume_guid.filter(|value| !value.is_empty());
    // A GUID is the only identity strong enough to anchor whole-volume
    // consent; `serial:` is a display/back-compat fallback only and must not
    // authorize native indexing (review #5678861648 defect 2).
    let identity_trusted = trusted_guid.is_some();
    let stable_volume_id = trusted_guid
        .map(str::to_string)
        .unwrap_or_else(|| format!("serial:{serial_number:08x}"));
    let is_ntfs = filesystem_type.eq_ignore_ascii_case("ntfs");
    let supports_usn_journal = filesystem_flags & FILE_SUPPORTS_USN_JOURNAL != 0;
    let kind = drive_kind(drive_kind_code);
    let native_provider_eligible =
        is_ntfs && identity_trusted && supports_usn_journal && drive_kind_code == DRIVE_FIXED;
    let provider = if native_provider_eligible {
        PROVIDER_WINDOWS_MFT_USN
    } else {
        PROVIDER_WINDOWS_UNSUPPORTED
    };
    let unsupported_reason = if native_provider_eligible {
        None
    } else if !is_ntfs {
        Some(format!(
            "windows_global_index_unsupported_filesystem:{filesystem_type}"
        ))
    } else if !identity_trusted {
        // Fail closed rather than fall back to a serial-derived identity.
        Some("windows_global_index_unstable_volume_identity".to_string())
    } else if !supports_usn_journal {
        // A cloud/virtual mount presenting NTFS metadata without USN support.
        Some("windows_global_index_unsupported_virtual_volume".to_string())
    } else {
        Some(format!(
            "windows_global_index_unsupported_drive_kind:{kind}"
        ))
    };
    VolumeClassification {
        stable_volume_id,
        provider,
        native_provider_eligible,
        unsupported_reason,
    }
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
    let filesystem_type = wide_string(&filesystem_name);
    let is_ntfs = filesystem_type.eq_ignore_ascii_case("ntfs");
    let kind = drive_kind(drive_kind_code);
    let classification = classify_volume(
        volume_guid(mount_path).as_deref(),
        serial_number,
        &filesystem_type,
        drive_kind_code,
        filesystem_flags,
    );
    let stable_volume_id = classification.stable_volume_id;
    let provider = classification.provider;
    let native_provider_eligible = classification.native_provider_eligible;
    let unsupported_reason = classification.unsupported_reason;
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
            drive_kind: kind.to_string(),
            enabled: source_enabled_by_default(drive_kind_code, is_ntfs),
            scope_authorization_state: SCOPE_AUTHORIZATION_NOT_AUTHORIZED.to_string(),
            provider: provider.to_string(),
            index_status: if native_provider_eligible {
                INDEX_STATUS_DISCOVERED.to_string()
            } else {
                INDEX_STATUS_UNAVAILABLE.to_string()
            },
            last_error: unsupported_reason,
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

    const GUID: &str = r"\\?\Volume{11111111-2222-3333-4444-555555555555}\";
    const USN: u32 = FILE_SUPPORTS_USN_JOURNAL;

    #[test]
    fn real_ntfs_fixed_volume_with_guid_is_eligible() {
        let c = classify_volume(Some(GUID), 0xabcd, "NTFS", DRIVE_FIXED, USN);
        assert_eq!(c.stable_volume_id, GUID);
        assert_eq!(c.provider, PROVIDER_WINDOWS_MFT_USN);
        assert!(c.native_provider_eligible);
        assert_eq!(c.unsupported_reason, None);
    }

    #[test]
    fn missing_guid_fails_closed_instead_of_using_serial() {
        // Review #5678861648 defect 2: a serial-only identity must never reach
        // the MFT/USN provider, because a cloned or colliding serial could
        // inherit another volume's whole-volume authorization.
        let c = classify_volume(None, 0xdead_beef, "NTFS", DRIVE_FIXED, USN);
        assert_eq!(c.stable_volume_id, "serial:deadbeef");
        assert_eq!(c.provider, PROVIDER_WINDOWS_UNSUPPORTED);
        assert!(!c.native_provider_eligible);
        assert_eq!(
            c.unsupported_reason.as_deref(),
            Some("windows_global_index_unstable_volume_identity")
        );
    }

    #[test]
    fn empty_guid_is_treated_as_missing() {
        let c = classify_volume(Some(""), 7, "NTFS", DRIVE_FIXED, USN);
        assert_eq!(c.stable_volume_id, "serial:00000007");
        assert!(!c.native_provider_eligible);
    }

    #[test]
    fn serial_collision_never_yields_native_eligibility() {
        // Two physically distinct volumes reporting the same serial must both
        // be rejected; there is no way to prove they are the same volume.
        let a = classify_volume(None, 0x1234, "NTFS", DRIVE_FIXED, USN);
        let b = classify_volume(None, 0x1234, "NTFS", DRIVE_FIXED, USN);
        assert_eq!(a.stable_volume_id, b.stable_volume_id);
        assert!(!a.native_provider_eligible);
        assert!(!b.native_provider_eligible);
    }

    #[test]
    fn guid_to_serial_drift_changes_identity() {
        // If a volume later fails to resolve its GUID, the derived identity
        // differs, so the authorization row is not reused.
        let with_guid = classify_volume(Some(GUID), 0x1234, "NTFS", DRIVE_FIXED, USN);
        let without_guid = classify_volume(None, 0x1234, "NTFS", DRIVE_FIXED, USN);
        assert_ne!(with_guid.stable_volume_id, without_guid.stable_volume_id);
        assert!(!without_guid.native_provider_eligible);
    }

    #[test]
    fn same_guid_on_a_new_drive_letter_keeps_one_identity() {
        // Drive-letter reuse is only dangerous when identity is derived from
        // the letter; here identity comes from the GUID, so re-mounting the
        // same volume under another letter stays the same authorized source.
        let on_c = classify_volume(Some(GUID), 1, "NTFS", DRIVE_FIXED, USN);
        let on_d = classify_volume(Some(GUID), 1, "NTFS", DRIVE_FIXED, USN);
        assert_eq!(on_c.stable_volume_id, on_d.stable_volume_id);
        assert!(on_c.native_provider_eligible && on_d.native_provider_eligible);
    }

    #[test]
    fn google_drive_style_virtual_volume_is_rejected() {
        // Owner-confirmed: G:\ is Google Drive for desktop. Its virtual drive
        // may present NTFS-like metadata but cannot host a USN journal, so it
        // must never enter the MFT/USN path or inherit NTFS authorization.
        let c = classify_volume(Some(GUID), 0, "NTFS", DRIVE_FIXED, 0);
        assert_eq!(c.provider, PROVIDER_WINDOWS_UNSUPPORTED);
        assert!(!c.native_provider_eligible);
        assert_eq!(
            c.unsupported_reason.as_deref(),
            Some("windows_global_index_unsupported_virtual_volume")
        );
    }

    #[test]
    fn google_drive_fat32_is_rejected_as_unsupported_filesystem() {
        let c = classify_volume(Some(GUID), 0, "FAT32", DRIVE_FIXED, 0);
        assert_eq!(c.provider, PROVIDER_WINDOWS_UNSUPPORTED);
        assert_eq!(
            c.unsupported_reason.as_deref(),
            Some("windows_global_index_unsupported_filesystem:FAT32")
        );
    }

    #[test]
    fn non_fixed_drive_kinds_are_rejected_even_with_ntfs() {
        for kind in [DRIVE_REMOVABLE, DRIVE_REMOTE, DRIVE_CDROM, DRIVE_RAMDISK] {
            let c = classify_volume(Some(GUID), 1, "NTFS", kind, USN);
            assert!(!c.native_provider_eligible, "kind {kind}");
            assert_eq!(c.provider, PROVIDER_WINDOWS_UNSUPPORTED);
        }
    }
}
