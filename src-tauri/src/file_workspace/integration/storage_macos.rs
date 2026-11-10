use super::{root_identity, VolumeState};
use objc2::rc::autoreleasepool;
use objc2_foundation::{NSArray, NSNumber, NSString, NSURLVolumeIsEjectableKey, NSURL};
use std::path::Path;

pub(super) fn inspect(path: &Path, removal: bool) -> VolumeState {
    let volume = crate::platform::macos::volume::inspect(path);
    let mut state = VolumeState {
        kind: if volume.is_local == Some(false) {
            "network"
        } else if volume.is_removable == Some(true) {
            "removable"
        } else {
            "local"
        },
        read_only: volume.is_read_only,
        ..VolumeState::default()
    };
    if let Ok(path) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) {
        let mut stat = std::mem::MaybeUninit::<libc::statfs>::zeroed();
        if unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) } == 0 {
            let stat = unsafe { stat.assume_init() };
            state.available_bytes = stat.f_bavail.checked_mul(stat.f_bsize as u64);
            state.total_bytes = stat.f_blocks.checked_mul(stat.f_bsize as u64);
        }
    }
    if removal && path.starts_with("/Volumes") && volume.is_local == Some(true) {
        state.can_eject = autoreleasepool(|_| {
            let Some(path) = path.to_str() else {
                return false;
            };
            let url = NSURL::fileURLWithPath(&NSString::from_str(path));
            let key = unsafe { NSURLVolumeIsEjectableKey };
            url.resourceValuesForKeys_error(&NSArray::from_slice(&[key]))
                .ok()
                .and_then(|values| values.objectForKey(key))
                .and_then(|value| value.downcast::<NSNumber>().ok())
                .is_some_and(|value| value.as_bool())
        });
    }
    if state.can_eject && state.kind == "local" {
        state.kind = "removable";
    }
    state
}

pub(super) fn eject(path: &Path, expected: &str) -> Result<(), String> {
    if root_identity(path).as_deref() != Some(expected) {
        return Err("storage_device_changed".into());
    }
    autoreleasepool(|_| {
        let path = path.to_str().ok_or("storage_eject_unsupported")?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let url = url
            .fileReferenceURL()
            .ok_or("storage_device_disconnected")?;
        objc2_app_kit::NSWorkspace::sharedWorkspace()
            .unmountAndEjectDeviceAtURL_error(&url)
            .map_err(|_| "storage_eject_busy".into())
    })
}
