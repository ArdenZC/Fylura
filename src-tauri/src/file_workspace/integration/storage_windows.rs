use super::{root_identity, VolumeState};
use std::{
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Devices::DeviceAndDriverInstallation::*,
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        CreateFileW, GetDiskFreeSpaceExW, GetDriveTypeW, GetVolumeInformationW, GetVolumePathNameW,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    },
    System::{
        Ioctl::{GUID_DEVINTERFACE_DISK, IOCTL_STORAGE_GET_DEVICE_NUMBER, STORAGE_DEVICE_NUMBER},
        WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOTE, DRIVE_REMOVABLE},
        IO::DeviceIoControl,
    },
};

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        if self.0 != INVALID_HANDLE_VALUE && !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
fn device_number(path: &[u16]) -> Option<u32> {
    let handle = Handle(unsafe {
        CreateFileW(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    });
    if handle.0 == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut number: STORAGE_DEVICE_NUMBER = unsafe { zeroed() };
    let mut bytes = 0;
    let ok = unsafe {
        DeviceIoControl(
            handle.0,
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            ptr::null(),
            0,
            (&mut number as *mut STORAGE_DEVICE_NUMBER).cast(),
            size_of::<STORAGE_DEVICE_NUMBER>() as u32,
            &mut bytes,
            ptr::null_mut(),
        )
    };
    (ok != 0 && bytes as usize >= size_of::<STORAGE_DEVICE_NUMBER>()).then_some(number.DeviceNumber)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiskDevice {
    devinst: u32,
    instance_id: String,
}
struct DeviceSet(HDEVINFO);
impl Drop for DeviceSet {
    fn drop(&mut self) {
        unsafe {
            SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

/// Resolve a live volume handle to its disk devnode. A fixed USB SSD can be
/// ejectable even when GetDriveType reports DRIVE_FIXED.
fn disk_device_by_number(number: u32) -> Option<DiskDevice> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    let set = DeviceSet(unsafe {
        SetupDiGetClassDevsW(
            &GUID_DEVINTERFACE_DISK,
            ptr::null(),
            ptr::null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    });
    if set.0 == -1 {
        return None;
    }
    for index in 0..256 {
        let mut interface: SP_DEVICE_INTERFACE_DATA = unsafe { zeroed() };
        interface.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
        if unsafe {
            SetupDiEnumDeviceInterfaces(
                set.0,
                ptr::null(),
                &GUID_DEVINTERFACE_DISK,
                index,
                &mut interface,
            )
        } == 0
        {
            break;
        }
        let mut length = 0;
        unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                set.0,
                &interface,
                ptr::null_mut(),
                0,
                &mut length,
                ptr::null_mut(),
            );
        }
        if length < size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 || length > 65536 {
            continue;
        }
        // u64 storage provides the native detail structure's alignment.
        let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
        let detail = buffer
            .as_mut_ptr()
            .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
        let mut device: SP_DEVINFO_DATA = unsafe { zeroed() };
        device.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
        unsafe {
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
        }
        if unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                set.0,
                &interface,
                detail,
                length,
                ptr::null_mut(),
                &mut device,
            )
        } == 0
        {
            continue;
        }
        let path = unsafe {
            std::slice::from_raw_parts((*detail).DevicePath.as_ptr(), (length as usize - 4) / 2)
        };
        let Some(end) = path.iter().position(|unit| *unit == 0) else {
            continue;
        };
        if device_number(&path[..=end]) == Some(number) {
            let mut instance_id = [0u16; 512];
            let mut required = 0u32;
            if unsafe {
                SetupDiGetDeviceInstanceIdW(
                    set.0,
                    &device,
                    instance_id.as_mut_ptr(),
                    instance_id.len() as u32,
                    &mut required,
                )
            } == 0
            {
                return None;
            }
            let end = instance_id.iter().position(|unit| *unit == 0)?;
            let id = OsString::from_wide(&instance_id[..end])
                .to_string_lossy()
                .into_owned();
            if id.is_empty() {
                return None;
            }
            return Some(DiskDevice {
                devinst: device.DevInst,
                instance_id: id,
            });
        }
    }
    None
}

fn volume_root(path: &Path) -> Option<String> {
    let encoded = wide(&path.to_string_lossy());
    let mut root = [0u16; 32768];
    if unsafe { GetVolumePathNameW(encoded.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return None;
    }
    let end = root.iter().position(|unit| *unit == 0)?;
    let root = String::from_utf16(&root[..end]).ok()?;
    (root.len() == 3 && root.as_bytes().get(1) == Some(&b':')).then_some(root)
}

fn disk_device_for_path(path: &Path) -> Option<DiskDevice> {
    let root = volume_root(path)?;
    let number = device_number(&wide(&format!("\\\\.\\{}", &root[..2])))?;
    disk_device_by_number(number)
}

fn system_device_instance_id() -> Option<String> {
    use std::sync::OnceLock;

    static SYSTEM_DEVICE: OnceLock<Option<String>> = OnceLock::new();
    SYSTEM_DEVICE
        .get_or_init(|| {
            let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
            let root = PathBuf::from(format!("{}\\", system.trim_end_matches(['\\', '/'])));
            disk_device_for_path(&root).map(|device| device.instance_id)
        })
        .clone()
}

pub(super) fn physical_device_identity(path: &Path) -> Option<String> {
    if is_network_path(path) {
        return None;
    }
    disk_device_for_path(path).map(|device| device_identity_key(&device.instance_id))
}

pub(super) fn is_network_path(path: &Path) -> bool {
    let path = path.to_string_lossy();
    if path.starts_with(r"\\") {
        return true;
    }
    let encoded = wide(&path);
    unsafe { GetDriveTypeW(encoded.as_ptr()) == DRIVE_REMOTE }
}

pub(super) fn same_physical_device_identity(expected: &str, candidate: &Path) -> bool {
    physical_device_identity(candidate)
        .is_some_and(|candidate| same_device_instance(expected, &candidate))
}

fn same_device_instance(left: &str, right: &str) -> bool {
    !left.is_empty() && !right.is_empty() && left.eq_ignore_ascii_case(right)
}

fn device_identity_key(instance_id: &str) -> String {
    blake3::hash(instance_id.to_ascii_lowercase().as_bytes())
        .to_hex()
        .to_string()
}
fn ejectable(node: u32) -> bool {
    let mut capabilities = 0u32;
    let mut length = size_of::<u32>() as u32;
    let status = unsafe {
        CM_Get_DevNode_Registry_PropertyW(
            node,
            CM_DRP_CAPABILITIES,
            ptr::null_mut(),
            (&mut capabilities as *mut u32).cast(),
            &mut length,
            0,
        )
    };
    status == CR_SUCCESS && capabilities & (CM_DEVCAP_REMOVABLE | CM_DEVCAP_EJECTSUPPORTED) != 0
}

fn removal_node(disk: u32) -> Option<u32> {
    if ejectable(disk) {
        return Some(disk);
    }
    // UASP/fixed USB disks can advertise removal on their storage parent.
    // Never climb arbitrary ancestors (which could remove a USB hub).
    let mut parent = 0;
    if unsafe { CM_Get_Parent(&mut parent, disk, 0) } != CR_SUCCESS {
        return None;
    }
    let mut service = [0u16; 128];
    let mut length = std::mem::size_of_val(&service) as u32;
    if unsafe {
        CM_Get_DevNode_Registry_PropertyW(
            parent,
            CM_DRP_SERVICE,
            ptr::null_mut(),
            service.as_mut_ptr().cast(),
            &mut length,
            0,
        )
    } != CR_SUCCESS
    {
        return None;
    }
    let end = service.iter().position(|unit| *unit == 0)?;
    let service = String::from_utf16(&service[..end]).ok()?;
    ((service.eq_ignore_ascii_case("USBSTOR") || service.eq_ignore_ascii_case("UASPStor"))
        && ejectable(parent))
    .then_some(parent)
}

pub(super) fn inspect(path: &Path, removal: bool) -> VolumeState {
    let mut state = VolumeState::default();
    let encoded = wide(&path.to_string_lossy());
    let mut root = [0u16; 32768];
    if unsafe { GetVolumePathNameW(encoded.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return state;
    }
    let Some(end) = root.iter().position(|unit| *unit == 0) else {
        return state;
    };
    let root_name = String::from_utf16_lossy(&root[..end]);
    let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
    state.kind = if kind == DRIVE_REMOTE {
        "network"
    } else if kind == DRIVE_REMOVABLE {
        "removable"
    } else {
        "local"
    };
    let mut flags = 0;
    if unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut flags,
            ptr::null_mut(),
            0,
        )
    } != 0
    {
        state.read_only = Some(flags & 0x0008_0000 != 0); // FILE_READ_ONLY_VOLUME
    }
    let mut free = 0;
    let mut total = 0;
    if unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut free, &mut total, ptr::null_mut()) } != 0 {
        state.available_bytes = Some(free);
        state.total_bytes = Some(total);
    }
    let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let is_system_root = root_name
        .trim_end_matches('\\')
        .eq_ignore_ascii_case(&system);
    let disk = (removal && !is_system_root)
        .then(|| disk_device_for_path(Path::new(&root_name)))
        .flatten();
    let is_system_device = if is_system_root {
        true
    } else if let Some(disk) = disk.as_ref() {
        system_device_instance_id()
            .is_none_or(|system| same_device_instance(&disk.instance_id, &system))
    } else {
        false
    };
    if removal && matches!(kind, DRIVE_FIXED | DRIVE_REMOVABLE) && !is_system_device {
        state.can_eject = disk
            .as_ref()
            .and_then(|disk| removal_node(disk.devinst))
            .is_some();
    }
    if state.can_eject {
        state.physical_device_identity = disk
            .as_ref()
            .map(|device| device_identity_key(&device.instance_id));
    }
    if state.can_eject && state.kind == "local" {
        state.kind = "removable";
    }
    state
}

pub(super) fn eject(path: &Path, expected: &str) -> Result<(), String> {
    let disk = disk_device_for_path(path).ok_or("storage_device_disconnected")?;
    let node = removal_node(disk.devinst).ok_or("storage_eject_unsupported")?;
    if root_identity(path).as_deref() != Some(expected) {
        return Err("storage_device_changed".into());
    }
    if disk_device_for_path(path)
        .is_none_or(|current| !same_device_instance(&current.instance_id, &disk.instance_id))
    {
        return Err("storage_device_changed".into());
    }
    let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let system_root = PathBuf::from(format!("{}\\", system.trim_end_matches(['\\', '/'])));
    let selected_root = volume_root(path);
    let is_system_root = selected_root
        .as_deref()
        .is_some_and(|root| root.trim_end_matches('\\').eq_ignore_ascii_case(&system));
    let system_device = disk_device_for_path(&system_root).ok_or("storage_eject_unsupported")?;
    if is_system_root || same_device_instance(&system_device.instance_id, &disk.instance_id) {
        return Err("storage_eject_unsupported".into());
    }
    // PnP owns flush, dismount and veto. Never force removal or retry a veto
    // by dismounting/closing another process's handles.
    let result = unsafe { CM_Request_Device_EjectW(node, ptr::null_mut(), ptr::null_mut(), 0, 0) };
    map_eject_result(result).map_err(str::to_string)
}

fn map_eject_result(result: CONFIGRET) -> Result<(), &'static str> {
    match result {
        CR_SUCCESS => Ok(()),
        CR_REMOVE_VETOED => Err("storage_eject_busy"),
        CR_NO_SUCH_DEVNODE | CR_DEVICE_NOT_THERE | CR_INVALID_DEVICE_ID => {
            Err("storage_device_changed")
        }
        _ => Err("storage_eject_unsupported"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        device_identity_key, is_network_path, map_eject_result, same_device_instance,
        same_physical_device_identity, CR_DEVICE_NOT_THERE, CR_REMOVE_VETOED, CR_SUCCESS,
    };
    use std::path::Path;

    #[test]
    fn device_instance_identity_groups_partitions_without_matching_other_disks() {
        let first_partition = r"USBSTOR\DISK&VEN_FYLURA&PROD_TEST\SERIAL42";
        let second_partition = r"usbstor\disk&ven_fylura&prod_test\serial42";
        let other_disk = r"USBSTOR\DISK&VEN_FYLURA&PROD_TEST\SERIAL43";
        assert!(same_device_instance(first_partition, second_partition));
        assert!(!same_device_instance(first_partition, other_disk));
        assert!(!same_device_instance(first_partition, ""));
    }

    #[test]
    fn renderer_identity_fingerprint_hides_the_raw_device_instance_id() {
        let first_partition = r"USBSTOR\DISK&VEN_FYLURA&PROD_TEST\SERIAL42";
        let same_device = r"usbstor\disk&ven_fylura&prod_test\serial42";
        let other_disk = r"USBSTOR\DISK&VEN_FYLURA&PROD_TEST\SERIAL43";
        let key = device_identity_key(first_partition);
        assert_eq!(key, device_identity_key(same_device));
        assert_ne!(key, device_identity_key(other_disk));
        assert!(!key.contains("SERIAL42"));
    }

    #[test]
    fn unc_paths_are_never_physical_eject_targets() {
        let share = Path::new(r"\\nas.example\documents");
        assert!(is_network_path(share));
        assert!(!same_physical_device_identity("device-key", share));
    }

    #[test]
    fn pnp_veto_and_device_change_are_reported_without_retry() {
        assert_eq!(
            map_eject_result(CR_REMOVE_VETOED),
            Err("storage_eject_busy")
        );
        assert_eq!(
            map_eject_result(CR_DEVICE_NOT_THERE),
            Err("storage_device_changed")
        );
        assert_eq!(map_eject_result(CR_SUCCESS), Ok(()),);
    }
}
