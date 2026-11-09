//! Explicit OS launch adapter. Browse remains the sole path authority; this
//! module neither reads content nor admits indexing or filesystem operations.
use super::{runtime::FileWorkspaceRuntime, types::BrowseLaunchRequest};
use crate::file_workspace::browse::BrowseEntryKind;
use crate::fs_safety::{capture_source_object_identity, PhysicalFileIdentity};
use std::path::PathBuf;

impl FileWorkspaceRuntime {
    pub(crate) fn resolve_browse_launch_path(
        &self,
        request: &BrowseLaunchRequest,
    ) -> Result<(PathBuf, PhysicalFileIdentity), String> {
        self.ensure_live()?;
        let entry = self
            .inner
            .browse
            .resolve_current_entry(&request.entry_ref)
            .map_err(|error| error.to_string())?;
        // Folder activation is navigation inside Files, never a shell launch.
        if entry.kind != BrowseEntryKind::File {
            return Err("workspace_launch_not_file".into());
        }
        let identity = capture_source_object_identity(&entry.path)
            .map_err(|_| "workspace_launch_entry_unavailable".to_string())?;
        if !std::fs::symlink_metadata(&entry.path)
            .map_err(|_| "workspace_launch_entry_unavailable".to_string())?
            .is_file()
        {
            return Err("workspace_launch_not_file".into());
        }
        Ok((entry.path, identity))
    }

    pub(crate) fn launch_browse_entry(&self, request: BrowseLaunchRequest) -> Result<(), String> {
        let (path, identity) = self.resolve_browse_launch_path(&request)?;
        // A chooser can stay open while the source changes. Re-resolve the
        // window-owned ref and physical identity immediately before dispatch.
        let revalidate = || {
            let (current, evidence) = self.resolve_browse_launch_path(&request)?;
            if current != path || evidence != identity {
                return Err("workspace_launch_entry_changed".into());
            }
            Ok(())
        };
        native::launch(&path, request.action, revalidate)
    }
}

#[cfg(target_os = "windows")]
mod native {
    use super::super::types::BrowseLaunchAction;
    use std::{os::windows::ffi::OsStrExt, path::Path};
    use windows_sys::Win32::{
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
        UI::{
            Shell::{
                SHOpenWithDialog, ShellExecuteExW, OAIF_EXEC, OPENASINFO, SEE_MASK_NOASYNC,
                SHELLEXECUTEINFOW,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    };

    pub(super) fn launch(
        path: &Path,
        action: BrowseLaunchAction,
        revalidate: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if wide[..wide.len() - 1].contains(&0) {
            return Err("workspace_launch_invalid_path".into());
        }
        // Shell extensions/Open With require COM on this blocking worker.
        let initialized =
            unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        if initialized < 0 {
            return Err(format!("workspace_launch_com_failed:{initialized}"));
        }
        struct ComGuard;
        impl Drop for ComGuard {
            fn drop(&mut self) {
                unsafe { CoUninitialize() };
            }
        }
        let _guard = ComGuard;
        revalidate()?;
        match action {
            BrowseLaunchAction::Open => {
                let verb: Vec<u16> = "open\0".encode_utf16().collect();
                let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
                info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
                info.fMask = SEE_MASK_NOASYNC;
                info.lpVerb = verb.as_ptr();
                info.lpFile = wide.as_ptr();
                info.nShow = SW_SHOWNORMAL;
                if unsafe { ShellExecuteExW(&mut info) } == 0 {
                    return Err(format!(
                        "workspace_launch_failed:{}",
                        std::io::Error::last_os_error()
                    ));
                }
                Ok(())
            }
            BrowseLaunchAction::OpenWith => {
                // The OS owns selection/launch; no command line, application
                // path or "change default association" flag comes from JS.
                let info = OPENASINFO {
                    pcszFile: wide.as_ptr(),
                    pcszClass: std::ptr::null(),
                    oaifInFlags: OAIF_EXEC,
                };
                let result = unsafe { SHOpenWithDialog(std::ptr::null_mut(), &info) };
                if result >= 0 || result as u32 == 0x800704c7 {
                    Ok(())
                } else {
                    Err(format!("workspace_open_with_failed:{result}"))
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::super::types::BrowseLaunchAction;
    use std::{
        path::{Path, PathBuf},
        process::Command,
    };

    pub(super) fn launch(
        path: &Path,
        action: BrowseLaunchAction,
        revalidate: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut command = Command::new("/usr/bin/open");
        if action == BrowseLaunchAction::OpenWith {
            // Standard Additions presents the native application chooser. The
            // fixed script contains no file name, path, or renderer input.
            let picked = Command::new("/usr/bin/osascript")
                .args(["-e", "POSIX path of (choose application as alias)"])
                .output()
                .map_err(|error| format!("workspace_open_with_failed:{error}"))?;
            if !picked.status.success() {
                if String::from_utf8_lossy(&picked.stderr).contains("(-128)") {
                    return Ok(());
                }
                return Err("workspace_open_with_failed".into());
            }
            let application = PathBuf::from(
                String::from_utf8(picked.stdout)
                    .map_err(|_| "workspace_open_with_invalid_application")?
                    .trim_end_matches(['\r', '\n']),
            );
            if !application.is_absolute() || application.extension().is_none_or(|ext| ext != "app")
            {
                return Err("workspace_open_with_invalid_application".into());
            }
            command.arg("-a").arg(application);
        }
        revalidate()?;
        // Argument-array dispatch and -- preserve punctuation literally.
        let result = command
            .arg("--")
            .arg(path)
            .status()
            .map_err(|error| format!("workspace_launch_failed:{error}"))?;
        if result.success() {
            Ok(())
        } else {
            Err("workspace_launch_failed".into())
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod native {
    use super::super::types::BrowseLaunchAction;
    use std::path::Path;
    pub(super) fn launch(
        _path: &Path,
        _action: BrowseLaunchAction,
        _revalidate: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        Err("workspace_launch_platform_unsupported".into())
    }
}
