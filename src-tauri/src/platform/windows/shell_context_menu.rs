//! Isolated native Shell context menu process used by the Files Browse surface.
//!
//! The parent process supplies only backend-resolved UTF-16 paths and the
//! identity captured from their current BrowseEntryRefs. This process does not
//! accept a command, executable name, or renderer-owned path.

use crate::fs_safety::{capture_browse_identity, PhysicalFileIdentity, PhysicalPlatform};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

const IPC_VERSION: u32 = 1;
const MAX_ITEMS: usize = 256;
const MAX_PATH_UNITS: usize = 32_767;
const MAX_IPC_BYTES: u64 = 8 * 1024 * 1024;
const COMMAND_ID_FIRST: u32 = 1;
const COMMAND_ID_LAST: u32 = 0x7fff;

/// Stable IPC representation deliberately contains no execution verb.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperMenuRequest {
    version: u32,
    owner_hwnd: usize,
    entries: Vec<HelperMenuEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperMenuEntry {
    path_utf16: Vec<u16>,
    identity: HelperIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HelperIdentity {
    size: u64,
    modified_ns: Option<i64>,
    created_ns: Option<i64>,
    volume_id: Option<String>,
    file_id: Option<String>,
    physical_key: Option<String>,
    link_count: Option<u64>,
}

impl From<&PhysicalFileIdentity> for HelperIdentity {
    fn from(identity: &PhysicalFileIdentity) -> Self {
        Self {
            size: identity.size,
            modified_ns: identity.modified_ns,
            created_ns: identity.created_ns,
            volume_id: identity.platform_volume_id.clone(),
            file_id: identity.platform_file_id.clone(),
            physical_key: identity.physical_key.clone(),
            link_count: identity.link_count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HelperMenuResponse {
    selected: bool,
    error: Option<String>,
}

/// Entry point used only by the separately packaged Windows helper executable.
pub fn run_helper() -> i32 {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.as_slice() != [std::ffi::OsStr::new("--fylura-shell-menu-v1")] {
        return 2;
    }
    let response = match read_request() {
        Ok(request) => show_native_menu(request),
        Err(error) => HelperMenuResponse {
            selected: false,
            error: Some(error),
        },
    };
    if let Ok(payload) = serde_json::to_vec(&response) {
        let _ = std::io::stdout().write_all(&payload);
        let _ = std::io::stdout().write_all(b"\n");
    }
    i32::from(response.error.is_some())
}

fn read_request() -> Result<HelperMenuRequest, String> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(MAX_IPC_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "shell_menu_ipc_read_failed".to_string())?;
    if bytes.len() as u64 > MAX_IPC_BYTES {
        return Err("shell_menu_request_too_large".into());
    }
    let request: HelperMenuRequest =
        serde_json::from_slice(&bytes).map_err(|_| "shell_menu_request_invalid".to_string())?;
    validate_request(&request)?;
    Ok(request)
}

fn validate_request(request: &HelperMenuRequest) -> Result<(), String> {
    if request.version != IPC_VERSION || request.owner_hwnd == 0 {
        return Err("shell_menu_request_invalid".into());
    }
    if request.entries.is_empty() || request.entries.len() > MAX_ITEMS {
        return Err("shell_menu_selection_limit".into());
    }
    for entry in &request.entries {
        if entry.path_utf16.is_empty()
            || entry.path_utf16.len() > MAX_PATH_UNITS
            || entry.path_utf16.contains(&0)
            || entry.identity.physical_key.is_none()
            || entry.identity.volume_id.is_none()
            || entry.identity.file_id.is_none()
        {
            return Err("shell_menu_request_invalid".into());
        }
    }
    let parents = request
        .entries
        .iter()
        .map(|entry| path_from_entry(entry).and_then(|path| parent_key(&path)))
        .collect::<Result<Vec<_>, _>>()?;
    if parents.windows(2).any(|pair| pair[0] != pair[1]) {
        return Err("shell_menu_selection_mixed_folders".into());
    }
    Ok(())
}

fn path_from_entry(entry: &HelperMenuEntry) -> Result<PathBuf, String> {
    use std::os::windows::ffi::OsStringExt;
    let path = PathBuf::from(std::ffi::OsString::from_wide(&entry.path_utf16));
    if !path.is_absolute() {
        return Err("shell_menu_request_invalid".into());
    }
    Ok(path)
}

fn parent_key(path: &Path) -> Result<String, String> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| {
            parent
                .to_string_lossy()
                .replace('/', "\\")
                .trim_end_matches('\\')
                .to_ascii_lowercase()
        })
        .ok_or_else(|| "shell_menu_selection_mixed_folders".to_string())
}

fn identity_matches(entry: &HelperMenuEntry) -> Result<bool, String> {
    let path = path_from_entry(entry)?;
    let identity = capture_browse_identity(&path)
        .map_err(|_| "shell_menu_entry_changed_or_unavailable".to_string())?;
    Ok(identity.platform_kind == PhysicalPlatform::Windows
        && HelperIdentity::from(&identity) == entry.identity)
}

fn show_native_menu(request: HelperMenuRequest) -> HelperMenuResponse {
    match show_menu(&request) {
        Ok(selected) => HelperMenuResponse {
            selected,
            error: None,
        },
        Err((selected, error)) => HelperMenuResponse {
            selected,
            error: Some(error),
        },
    }
}

#[cfg(windows)]
fn show_menu(request: &HelperMenuRequest) -> Result<bool, (bool, String)> {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
    use windows::{
        core::{Interface, PCSTR, PCWSTR},
        Win32::{
            Foundation::{HWND, POINT},
            Graphics::Gdi::HBRUSH,
            System::{
                Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
                LibraryLoader::GetModuleHandleW,
            },
            UI::{
                HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
                Shell::{
                    Common::ITEMIDLIST, IContextMenu, IContextMenu2, IContextMenu3, IShellFolder,
                    SHBindToParent, SHParseDisplayName, CMF_EXTENDEDVERBS, CMF_NORMAL,
                    CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX,
                },
                WindowsAndMessaging::{
                    CreatePopupMenu, CreateWindowExW, DestroyMenu, DestroyWindow, GetCursorPos,
                    GetForegroundWindow, IsWindow, LoadCursorW, RegisterClassW,
                    SetForegroundWindow, SetWindowLongPtrW, TrackPopupMenuEx, UnregisterClassW,
                    GWLP_USERDATA, TPM_RETURNCMD, TPM_RIGHTBUTTON, WNDCLASSW, WS_EX_TOOLWINDOW,
                    WS_POPUP,
                },
            },
        },
    };

    if let Err(error) = validate_request(request) {
        return Err((false, error));
    }
    if request
        .entries
        .iter()
        .any(|entry| !identity_matches(entry).unwrap_or(false))
    {
        return Err((false, "shell_menu_entry_changed_or_unavailable".into()));
    }

    let owner = HWND(request.owner_hwnd as *mut _);
    if unsafe { IsWindow(Some(owner)) }.0 == 0 {
        return Err((false, "shell_menu_owner_unavailable".into()));
    }

    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if com.0 < 0 {
        return Err((false, "shell_menu_com_initialization_failed".into()));
    }
    struct ComGuard;
    impl Drop for ComGuard {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }
    let _com = ComGuard;

    let previous_dpi =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    struct DpiGuard(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT);
    impl Drop for DpiGuard {
        fn drop(&mut self) {
            if !self.0 .0.is_null() {
                unsafe { SetThreadDpiAwarenessContext(self.0) };
            }
        }
    }
    let _dpi = DpiGuard(previous_dpi);

    struct PidlGuard(Vec<*mut ITEMIDLIST>);
    impl Drop for PidlGuard {
        fn drop(&mut self) {
            for pidl in &self.0 {
                unsafe { windows::Win32::UI::Shell::ILFree(Some(*pidl)) };
            }
        }
    }
    let mut pidls = PidlGuard(Vec::with_capacity(request.entries.len()));
    let mut relative_pidls = Vec::with_capacity(request.entries.len());
    let mut parent_folder: Option<IShellFolder> = None;
    for entry in &request.entries {
        let path = path_from_entry(entry).map_err(|error| (false, error))?;
        let wide: Vec<u16> = OsStr::new(&path).encode_wide().chain(Some(0)).collect();
        let mut absolute = std::ptr::null_mut();
        unsafe { SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut absolute, 0, None) }
            .map_err(|_| (false, "shell_menu_entry_unavailable".into()))?;
        if absolute.is_null() {
            return Err((false, "shell_menu_entry_unavailable".into()));
        }
        pidls.0.push(absolute);
        let mut child = std::ptr::null_mut();
        let parent = unsafe { SHBindToParent::<IShellFolder>(absolute, Some(&mut child)) }
            .map_err(|_| (false, "shell_menu_folder_unavailable".into()))?;
        if child.is_null() {
            return Err((false, "shell_menu_entry_unavailable".into()));
        }
        if parent_folder.is_none() {
            parent_folder = Some(parent);
        }
        relative_pidls.push(child as *const ITEMIDLIST);
    }
    let parent_folder =
        parent_folder.ok_or_else(|| (false, "shell_menu_entry_unavailable".into()))?;

    let menu =
        unsafe { CreatePopupMenu() }.map_err(|_| (false, "shell_menu_create_failed".into()))?;
    struct MenuGuard(windows::Win32::UI::WindowsAndMessaging::HMENU);
    impl Drop for MenuGuard {
        fn drop(&mut self) {
            let _ = unsafe { DestroyMenu(self.0) };
        }
    }
    let _menu = MenuGuard(menu);
    const CLASS_NAME: &str = "Fylura.WindowsShellMenu.HelperWindow.v1";
    let class_name: Vec<u16> = CLASS_NAME.encode_utf16().chain(Some(0)).collect();
    let instance = unsafe { GetModuleHandleW(None) }
        .map_err(|_| (false, "shell_menu_window_init_failed".into()))?;
    let cursor = unsafe { LoadCursorW(None, windows::Win32::UI::WindowsAndMessaging::IDC_ARROW) }
        .unwrap_or_default();
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(menu_window_proc),
        hInstance: instance.into(),
        hCursor: cursor,
        hbrBackground: HBRUSH::default(),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    let atom = unsafe { RegisterClassW(&window_class) };
    if atom == 0 {
        return Err((false, "shell_menu_window_init_failed".into()));
    }
    struct ClassGuard(Vec<u16>, windows::Win32::Foundation::HINSTANCE);
    impl Drop for ClassGuard {
        fn drop(&mut self) {
            let _ = unsafe { UnregisterClassW(PCWSTR(self.0.as_ptr()), Some(self.1)) };
        }
    }
    let _class = ClassGuard(class_name, instance.into());

    let window = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR(_class.0.as_ptr()),
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            Some(owner),
            None,
            Some(instance.into()),
            None,
        )
    }
    .map_err(|_| (false, "shell_menu_window_init_failed".into()))?;
    struct WindowGuard(HWND);
    impl Drop for WindowGuard {
        fn drop(&mut self) {
            let _ = unsafe { DestroyWindow(self.0) };
        }
    }
    let _window = WindowGuard(window);

    // The helper owns the actual menu HWND and therefore also owns the
    // IContextMenu callback/message loop. The app HWND is only its owner for
    // activation and focus restoration; giving Shell the app HWND here would
    // route IContextMenu2/3 drawing callbacks to the wrong process.
    let context: IContextMenu =
        unsafe { parent_folder.GetUIObjectOf(window, &relative_pidls, None) }
            .map_err(|_| (false, "shell_menu_extension_unavailable".into()))?;
    let menu3 = context.cast::<IContextMenu3>().ok();
    let menu2 = context.cast::<IContextMenu2>().ok();
    let handler = MenuMessageHandler { menu3, menu2 };
    let handler_ptr = &handler as *const MenuMessageHandler as isize;
    unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, handler_ptr) };

    let query = unsafe {
        context.QueryContextMenu(
            menu,
            0,
            COMMAND_ID_FIRST,
            COMMAND_ID_LAST,
            CMF_NORMAL | CMF_EXTENDEDVERBS,
        )
    };
    if query.0 < 0 {
        return Err((false, "shell_menu_query_failed".into()));
    }

    let mut cursor_point = POINT::default();
    unsafe { GetCursorPos(&mut cursor_point) }
        .map_err(|_| (false, "shell_menu_cursor_unavailable".into()))?;
    let previous_foreground = unsafe { GetForegroundWindow() };
    let _ = unsafe { SetForegroundWindow(window) };
    let selected = unsafe {
        TrackPopupMenuEx(
            menu,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            cursor_point.x,
            cursor_point.y,
            window,
            None,
        )
    }
    .0 as u32;

    if selected == 0 {
        restore_foreground(owner, previous_foreground);
        return Ok(false);
    }
    if !(COMMAND_ID_FIRST..=COMMAND_ID_LAST).contains(&selected) {
        restore_foreground(owner, previous_foreground);
        return Err((true, "shell_menu_command_invalid".into()));
    }

    // Re-resolve the physical identities after the native menu closes, as
    // close as possible to handing the selected Shell verb to Windows.
    if request
        .entries
        .iter()
        .any(|entry| !identity_matches(entry).unwrap_or(false))
    {
        restore_foreground(owner, previous_foreground);
        return Err((true, "shell_menu_entry_changed_or_unavailable".into()));
    }

    let offset = (selected - COMMAND_ID_FIRST) as usize;
    let invoke = CMINVOKECOMMANDINFOEX {
        cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: CMIC_MASK_PTINVOKE | 0x0000_4000, // CMIC_MASK_UNICODE
        hwnd: window,
        lpVerb: PCSTR(offset as *const u8),
        lpVerbW: PCWSTR(offset as *const u16),
        nShow: windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL.0,
        ptInvoke: cursor_point,
        ..Default::default()
    };
    let result = unsafe {
        context
            .InvokeCommand((&invoke as *const CMINVOKECOMMANDINFOEX).cast::<CMINVOKECOMMANDINFO>())
    };
    restore_foreground(owner, previous_foreground);
    result.map_err(|_| (true, "shell_menu_command_failed".into()))?;
    Ok(true)
}

#[cfg(windows)]
fn restore_foreground(
    owner: windows::Win32::Foundation::HWND,
    previous: windows::Win32::Foundation::HWND,
) {
    use windows::Win32::UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow};
    if unsafe { IsWindow(Some(owner)) }.0 != 0 {
        let _ = unsafe { SetForegroundWindow(owner) };
    } else if !previous.0.is_null() && unsafe { IsWindow(Some(previous)) }.0 != 0 {
        let _ = unsafe { SetForegroundWindow(previous) };
    }
}

#[cfg(windows)]
struct MenuMessageHandler {
    menu3: Option<windows::Win32::UI::Shell::IContextMenu3>,
    menu2: Option<windows::Win32::UI::Shell::IContextMenu2>,
}

#[cfg(windows)]
unsafe extern "system" fn menu_window_proc(
    window: windows::Win32::Foundation::HWND,
    message: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::{
        Foundation::LRESULT,
        UI::WindowsAndMessaging::{
            DefWindowProcW, GetWindowLongPtrW, GWLP_USERDATA, WM_DRAWITEM, WM_INITMENUPOPUP,
            WM_MEASUREITEM, WM_MENUCHAR, WM_MENUCOMMAND,
        },
    };
    if matches!(
        message,
        WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR | WM_MENUCOMMAND
    ) {
        let pointer =
            unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *const MenuMessageHandler;
        if !pointer.is_null() {
            let handler = unsafe { &*pointer };
            if let Some(menu3) = &handler.menu3 {
                let mut result = LRESULT::default();
                if unsafe { menu3.HandleMenuMsg2(message, wparam, lparam, Some(&mut result)) }
                    .is_ok()
                {
                    return result;
                }
            }
            if let Some(menu2) = &handler.menu2 {
                if unsafe { menu2.HandleMenuMsg(message, wparam, lparam) }.is_ok() {
                    return LRESULT::default();
                }
            }
        }
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

#[cfg(not(windows))]
fn show_menu(_request: &HelperMenuRequest) -> Result<bool, (bool, String)> {
    Err((false, "shell_menu_platform_unsupported".into()))
}

/// Parent-side identity capture produces the only input accepted by the
/// helper. UTF-16 is preserved so every valid Windows filename survives IPC.
pub fn helper_entry(
    path: &Path,
    identity: &PhysicalFileIdentity,
) -> Result<HelperMenuEntry, String> {
    use std::os::windows::ffi::OsStrExt;
    let path_utf16: Vec<u16> = path.as_os_str().encode_wide().collect();
    if path_utf16.is_empty() || path_utf16.len() > MAX_PATH_UNITS || path_utf16.contains(&0) {
        return Err("shell_menu_entry_unavailable".into());
    }
    if identity.platform_kind != PhysicalPlatform::Windows
        || identity.physical_key.is_none()
        || identity.platform_volume_id.is_none()
        || identity.platform_file_id.is_none()
    {
        return Err("shell_menu_identity_unavailable".into());
    }
    Ok(HelperMenuEntry {
        path_utf16,
        identity: HelperIdentity::from(identity),
    })
}

pub fn helper_request(owner_hwnd: usize, entries: Vec<HelperMenuEntry>) -> HelperMenuRequest {
    HelperMenuRequest {
        version: IPC_VERSION,
        owner_hwnd,
        entries,
    }
}

pub fn helper_response(bytes: &[u8]) -> Result<(bool, Option<String>), String> {
    let response: HelperMenuResponse =
        serde_json::from_slice(bytes).map_err(|_| "shell_menu_helper_failed".to_string())?;
    Ok((response.selected, response.error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_ipc_rejects_unknown_fields_and_invalid_identity() {
        let unknown: Result<HelperMenuRequest, _> =
            serde_json::from_str(r#"{"version":1,"ownerHwnd":1,"entries":[],"command":"cmd.exe"}"#);
        assert!(unknown.is_err());
        let invalid = HelperMenuRequest {
            version: IPC_VERSION,
            owner_hwnd: 1,
            entries: vec![HelperMenuEntry {
                path_utf16: vec![b'C' as u16, b':' as u16, b'\\' as u16],
                identity: HelperIdentity {
                    size: 1,
                    modified_ns: None,
                    created_ns: None,
                    volume_id: None,
                    file_id: None,
                    physical_key: None,
                    link_count: None,
                },
            }],
        };
        assert_eq!(
            validate_request(&invalid),
            Err("shell_menu_request_invalid".into())
        );
    }

    #[test]
    fn helper_response_reports_cancel_and_selection_without_shell_result_claims() {
        assert_eq!(
            helper_response(br#"{"selected":false,"error":null}"#).unwrap(),
            (false, None)
        );
        assert_eq!(
            helper_response(br#"{"selected":true,"error":"shell_menu_command_failed"}"#).unwrap(),
            (true, Some("shell_menu_command_failed".into()))
        );
        assert_eq!(
            helper_response(b"").unwrap_err(),
            "shell_menu_helper_failed"
        );
    }

    #[cfg(windows)]
    #[test]
    fn real_windows_identity_check_rejects_same_path_replacement() {
        struct TestDirectory(PathBuf);
        impl Drop for TestDirectory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let root = std::env::temp_dir().join(format!(
            "fylura-shell-menu-身份 replacement-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create test directory");
        let _cleanup = TestDirectory(root.clone());
        let path = root.join("中文 name [shell].txt");
        let held_original = root.join("held original.txt");
        std::fs::write(&path, b"the previewed physical file").expect("create original file");
        let original = capture_browse_identity(&path).expect("capture original identity");
        let entry = helper_entry(&path, &original).expect("construct helper entry");
        assert!(identity_matches(&entry).expect("check original identity"));

        let second_path = root.join("second file.txt");
        std::fs::write(&second_path, b"another selected file").expect("create second file");
        let second = helper_entry(
            &second_path,
            &capture_browse_identity(&second_path).expect("capture second identity"),
        )
        .expect("construct second helper entry");
        validate_request(&helper_request(1, vec![entry.clone(), second]))
            .expect("same-folder multi-selection is accepted");

        let other_folder = root.join("other folder");
        std::fs::create_dir(&other_folder).expect("create other folder");
        let other_path = other_folder.join("third file.txt");
        std::fs::write(&other_path, b"not in the selected folder").expect("create third file");
        let other = helper_entry(
            &other_path,
            &capture_browse_identity(&other_path).expect("capture third identity"),
        )
        .expect("construct third helper entry");
        assert_eq!(
            validate_request(&helper_request(1, vec![entry.clone(), other])),
            Err("shell_menu_selection_mixed_folders".into())
        );

        std::fs::rename(&path, &held_original).expect("move original away");
        std::fs::write(&path, b"a different object at the same path")
            .expect("create same-path replacement");
        assert!(!identity_matches(&entry).expect("check replacement identity"));
    }
}
