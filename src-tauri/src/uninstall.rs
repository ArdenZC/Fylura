//! Explicit, current-user-only application-data cleanup used by native
//! uninstall helpers. Safe Trash contains user files and is always retained.

use std::{fs, path::Path};

#[cfg(any(
    test,
    all(
        any(target_os = "windows", target_os = "macos"),
        not(feature = "native-qa")
    )
))]
use std::path::PathBuf;

const APP_IDENTIFIER: &str = "com.startlan.fylura";
const DATABASE_NAME: &str = "fylura.sqlite3";
const SAFE_TRASH_NAMES: &[&str] = &["safe-trash", ".fylura-trash", ".zen-canvas-trash"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct CleanupResult {
    preserved_safe_trash_roots: usize,
}

fn safe_trash_name(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    SAFE_TRASH_NAMES
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

fn clean_owned_root(root: &Path) -> Result<CleanupResult, String> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CleanupResult {
                preserved_safe_trash_roots: 0,
            });
        }
        Err(_) => return Err("could not inspect a Fylura data directory".to_string()),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(
            "a Fylura data directory is not a real directory; it was preserved".to_string(),
        );
    }

    let mut preserved = 0;
    clean_directory_contents(root, &mut preserved)?;
    if preserved == 0 {
        fs::remove_dir(root).map_err(|_| "could not remove a Fylura data directory".to_string())?;
    }
    verify_only_safe_trash_remains(root)?;
    Ok(CleanupResult {
        preserved_safe_trash_roots: preserved,
    })
}

fn validate_owned_root(root: &Path) -> Result<(), String> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => {
            Err("a Fylura data directory is not a real directory; it was preserved".to_string())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("could not inspect a Fylura data directory".to_string()),
    }
}

fn clean_application_data_roots(
    data_root: &Path,
    cache_root: &Path,
    credentials: &impl crate::ai::settings::CredentialStore,
) -> Result<(CleanupResult, CleanupResult), String> {
    validate_owned_root(data_root)?;
    validate_owned_root(cache_root)?;
    let database_path = data_root.join(DATABASE_NAME);
    if let Ok(metadata) = fs::symlink_metadata(&database_path) {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("Fylura database is not a regular file; cleanup was stopped".to_string());
        }
    }

    crate::ai::settings::clear_current_ai_credentials(credentials)?;
    let data_result = clean_owned_root(data_root)?;
    let cache_result = clean_owned_root(cache_root)?;
    Ok((data_result, cache_result))
}

fn clean_directory_contents(directory: &Path, preserved: &mut usize) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|_| "could not inspect a Fylura data directory".to_string())?;
    for entry in entries {
        let entry = entry.map_err(|_| "could not inspect a Fylura data item".to_string())?;
        let path = entry.path();
        let name = entry.file_name();
        if safe_trash_name(&name) {
            *preserved += 1;
            continue;
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| "could not inspect a Fylura data item".to_string())?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            clean_directory_contents(&path, preserved)?;
            match fs::remove_dir(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                Err(_) => return Err("could not remove a Fylura data item".to_string()),
            }
        } else {
            fs::remove_file(&path)
                .map_err(|_| "could not remove a Fylura data item".to_string())?;
        }
    }
    Ok(())
}

fn verify_only_safe_trash_remains(root: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("could not verify Fylura data cleanup".to_string()),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("could not verify Fylura data cleanup".to_string());
    }
    let entries =
        fs::read_dir(root).map_err(|_| "could not verify Fylura data cleanup".to_string())?;
    for entry in entries {
        let entry = entry.map_err(|_| "could not verify Fylura data cleanup".to_string())?;
        let path = entry.path();
        let name = entry.file_name();
        if safe_trash_name(&name) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| "could not verify Fylura data cleanup".to_string())?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            verify_only_safe_trash_remains(&path)?;
        } else {
            return Err("Fylura data cleanup left an unexpected item".to_string());
        }
    }
    Ok(())
}

#[cfg(all(
    any(target_os = "windows", target_os = "macos"),
    not(feature = "native-qa")
))]
fn application_data_roots() -> Result<(PathBuf, PathBuf), String> {
    let data_root = dirs::data_dir()
        .filter(|path| path.is_absolute())
        .ok_or_else(|| "Fylura user data directory is unavailable".to_string())?;
    let cache_root = dirs::cache_dir()
        .filter(|path| path.is_absolute())
        .ok_or_else(|| "Fylura cache directory is unavailable".to_string())?;
    Ok((
        data_root.join(APP_IDENTIFIER),
        cache_root.join(APP_IDENTIFIER),
    ))
}

#[cfg(all(
    any(target_os = "windows", target_os = "macos"),
    not(feature = "native-qa")
))]
pub fn clean_current_user_data() -> Result<String, String> {
    use crate::ai::settings::SystemCredentialStore;

    cleanup_checkpoint("locate-app-data");
    let (data_root, cache_root) = application_data_roots()?;
    cleanup_checkpoint("enumerate-and-remove-ai-credentials");
    let (data_result, cache_result) =
        clean_application_data_roots(&data_root, &cache_root, &SystemCredentialStore)?;

    #[cfg(target_os = "macos")]
    cleanup_checkpoint("remove-preferences-and-login-item");
    #[cfg(target_os = "macos")]
    let logs_result = clean_macos_preferences_and_launch_agent()?;
    #[cfg(not(target_os = "macos"))]
    let logs_result = CleanupResult {
        preserved_safe_trash_roots: 0,
    };

    #[cfg(target_os = "macos")]
    let removed_scopes = "database/cache/logs/preferences/login-item=removed";
    #[cfg(target_os = "windows")]
    let removed_scopes = "database/cache/logs/startup=removed";
    cleanup_checkpoint("complete");
    Ok(format!(
        "Fylura data cleanup verified; credentials=removed; {removed_scopes}; Safe Trash roots preserved={}",
        data_result.preserved_safe_trash_roots
            + cache_result.preserved_safe_trash_roots
            + logs_result.preserved_safe_trash_roots
    ))
}

#[cfg(any(target_os = "windows", test))]
fn current_windows_autostart_command_matches(main_executable: &Path, command: &str) -> bool {
    let expected = format!("{} --background", main_executable.to_string_lossy());
    command.eq_ignore_ascii_case(&expected)
}

#[cfg(any(target_os = "windows", test))]
fn current_windows_startup_approval_is_owned(bytes: &[u8]) -> bool {
    bytes.len() == 12 && matches!(bytes[0], 0x02 | 0x03) && bytes[1..].iter().all(|byte| *byte == 0)
}

#[cfg(any(target_os = "windows", test))]
fn verified_un_elevated_user_sid(
    sid: Result<String, String>,
    session_sid: Result<String, String>,
    elevated: Result<bool, String>,
) -> Option<String> {
    match (sid, session_sid, elevated) {
        (Ok(sid), Ok(session_sid), Ok(false))
            if sid.starts_with("S-1-") && sid.eq_ignore_ascii_case(&session_sid) =>
        {
            Some(sid)
        }
        _ => None,
    }
}

#[cfg(all(target_os = "windows", not(feature = "native-qa")))]
mod windows_uninstall {
    use std::{
        fs,
        os::windows::ffi::OsStrExt,
        path::{Path, PathBuf},
        process::Command,
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, LocalFree, ERROR_CANCELLED, HANDLE},
        Security::Authorization::ConvertSidToStringSidW,
        Security::{GetTokenInformation, LookupAccountNameW, TokenUser, TOKEN_QUERY, TOKEN_USER},
        Storage::FileSystem::SYNCHRONIZE,
        System::Threading::{
            GetCurrentProcess, GetCurrentProcessId, GetExitCodeProcess, OpenProcess,
            OpenProcessToken, WaitForSingleObject, INFINITE,
        },
        UI::{
            Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
            WindowsAndMessaging::{
                MessageBoxW, IDCANCEL, IDNO, IDYES, MB_DEFBUTTON2, MB_ICONERROR,
                MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_YESNO,
                MB_YESNOCANCEL, SW_SHOWNORMAL,
            },
        },
    };

    const WORKER_FLAG: &str = "--fylura-uninstall-worker";
    const RETRY_CLEANUP_FLAG: &str = "--fylura-uninstall-worker-cleanup";
    const MAX_SID_LENGTH: usize = 192;

    pub(super) const ERROR: u32 = MB_OK | MB_ICONERROR | MB_SETFOREGROUND;

    type LauncherError = (i32, String);

    pub(super) fn windows_message_box(message: &str, title: &str, flags: u32) -> i32 {
        let message = wide(message);
        let title = wide(title);
        unsafe { MessageBoxW(ptr::null_mut(), message.as_ptr(), title.as_ptr(), flags) }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    fn current_windows_user_sid() -> Result<String, String> {
        let mut token: HANDLE = ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err("Windows could not verify the current account identity".to_string());
        }
        let token_guard = TokenGuard(token);
        let mut required_bytes = 0u32;
        unsafe {
            GetTokenInformation(
                token_guard.0,
                TokenUser,
                ptr::null_mut(),
                0,
                &mut required_bytes,
            );
        }
        if required_bytes < std::mem::size_of::<TOKEN_USER>() as u32 || required_bytes > 64 * 1024 {
            return Err("Windows could not verify the current account identity".to_string());
        }
        let mut user_bytes =
            vec![0usize; required_bytes.div_ceil(std::mem::size_of::<usize>() as u32) as usize];
        if unsafe {
            GetTokenInformation(
                token_guard.0,
                TokenUser,
                user_bytes.as_mut_ptr().cast(),
                required_bytes,
                &mut required_bytes,
            )
        } == 0
        {
            return Err("Windows could not verify the current account identity".to_string());
        }
        let user = unsafe { &*user_bytes.as_ptr().cast::<TOKEN_USER>() };
        let mut sid_text: *mut u16 = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid_text) } == 0
            || sid_text.is_null()
        {
            return Err("Windows could not verify the current account identity".to_string());
        }
        let sid = (0..MAX_SID_LENGTH)
            .find(|index| unsafe { *sid_text.add(*index) == 0 })
            .map(|length| {
                String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(sid_text, length) })
            });
        unsafe { LocalFree(sid_text.cast()) };
        sid.filter(|sid| sid.starts_with("S-1-") && sid.len() < MAX_SID_LENGTH)
            .ok_or_else(|| "Windows could not verify the current account identity".to_string())
    }

    fn current_windows_user_is_elevated() -> Result<bool, String> {
        use windows_sys::Win32::Security::{TokenElevation, TOKEN_ELEVATION};

        let mut token: HANDLE = ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err("Windows could not verify the current account token".to_string());
        }
        let token_guard = TokenGuard(token);
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned_bytes = 0u32;
        if unsafe {
            GetTokenInformation(
                token_guard.0,
                TokenElevation,
                (&mut elevation as *mut TOKEN_ELEVATION).cast(),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned_bytes,
            )
        } == 0
            || returned_bytes < std::mem::size_of::<TOKEN_ELEVATION>() as u32
        {
            return Err("Windows could not verify the current account token".to_string());
        }
        Ok(elevation.TokenIsElevated != 0)
    }

    fn current_windows_interactive_session_sid() -> Result<String, String> {
        use std::ffi::c_void;
        use windows_sys::Win32::{
            Foundation::ERROR_INSUFFICIENT_BUFFER,
            Security::SID_NAME_USE,
            System::RemoteDesktop::{
                ProcessIdToSessionId, WTSDomainName, WTSFreeMemory, WTSQuerySessionInformationW,
                WTSUserName, WTS_CURRENT_SERVER_HANDLE,
            },
        };

        fn session_value(session_id: u32, information: i32) -> Result<String, String> {
            let mut buffer: *mut u16 = ptr::null_mut();
            let mut returned_bytes = 0u32;
            if unsafe {
                WTSQuerySessionInformationW(
                    WTS_CURRENT_SERVER_HANDLE,
                    session_id,
                    information,
                    &mut buffer,
                    &mut returned_bytes,
                )
            } == 0
                || buffer.is_null()
            {
                return Err("Windows could not verify the interactive account".to_string());
            }
            let value = (|| {
                let max_units = returned_bytes as usize / std::mem::size_of::<u16>();
                let length = (0..max_units)
                    .find(|index| unsafe { *buffer.add(*index) == 0 })
                    .ok_or_else(|| {
                        "Windows returned invalid interactive account metadata".to_string()
                    })?;
                let value = unsafe { std::slice::from_raw_parts(buffer, length) };
                String::from_utf16(value).map_err(|_| {
                    "Windows returned invalid interactive account metadata".to_string()
                })
            })();
            unsafe { WTSFreeMemory(buffer.cast::<c_void>()) };
            match value {
                Ok(value) if !value.is_empty() => Ok(value),
                Ok(_) => Err("Windows returned empty interactive account metadata".to_string()),
                Err(error) => Err(error),
            }
        }

        let mut session_id = 0u32;
        if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session_id) } == 0 {
            return Err("Windows could not verify the interactive account".to_string());
        }
        let username = session_value(session_id, WTSUserName)?;
        let domain = session_value(session_id, WTSDomainName)?;
        let account_name = wide(&format!("{domain}\\{username}"));
        let mut sid_length = 0u32;
        let mut domain_length = 0u32;
        let mut name_use: SID_NAME_USE = 0;
        let first_result = unsafe {
            LookupAccountNameW(
                ptr::null(),
                account_name.as_ptr(),
                ptr::null_mut(),
                &mut sid_length,
                ptr::null_mut(),
                &mut domain_length,
                &mut name_use,
            )
        };
        if first_result != 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
            return Err("Windows could not verify the interactive account identity".to_string());
        }
        if sid_length == 0 || sid_length > 64 * 1024 || domain_length > 32 * 1024 {
            return Err("Windows returned invalid interactive account metadata".to_string());
        }
        let mut sid_storage =
            vec![0usize; sid_length.div_ceil(std::mem::size_of::<usize>() as u32) as usize];
        let mut resolved_domain = vec![0u16; (domain_length as usize).max(1)];
        if unsafe {
            LookupAccountNameW(
                ptr::null(),
                account_name.as_ptr(),
                sid_storage.as_mut_ptr().cast(),
                &mut sid_length,
                resolved_domain.as_mut_ptr(),
                &mut domain_length,
                &mut name_use,
            )
        } == 0
        {
            return Err("Windows could not verify the interactive account identity".to_string());
        }
        let mut sid_text: *mut u16 = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(sid_storage.as_mut_ptr().cast(), &mut sid_text) } == 0
            || sid_text.is_null()
        {
            return Err("Windows could not verify the interactive account identity".to_string());
        }
        let sid = (0..MAX_SID_LENGTH)
            .find(|index| unsafe { *sid_text.add(*index) == 0 })
            .map(|length| {
                String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(sid_text, length) })
            });
        unsafe { LocalFree(sid_text.cast()) };
        sid.filter(|sid| sid.starts_with("S-1-") && sid.len() < MAX_SID_LENGTH)
            .ok_or_else(|| "Windows could not verify the interactive account identity".to_string())
    }

    struct TokenGuard(HANDLE);
    impl Drop for TokenGuard {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    fn language_is_chinese() -> bool {
        use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
        unsafe { GetUserDefaultUILanguage() & 0x03ff == 0x0004 }
    }

    fn confirm_data_cleanup(sid: &str) -> Result<bool, LauncherError> {
        let (question, second) = if language_is_chinese() {
            (
                format!(
                    "将仅清理当前 Windows 账户 {sid} 的 Fylura 设置、数据库、索引、缓存、日志和 Fylura AI 凭据。History、Journal 和 Restore 记录会删除；Safe Trash 中的真实文件和已整理文件会保留。\n\n是：彻底清理并继续卸载。否：保留个人数据并继续卸载。取消：中止卸载。",
                ),
                format!(
                    "请再次确认：删除账户 {sid} 的 Fylura 数据后，Safe Trash 中的文件可能失去 History/Restore 恢复记录。是否彻底清理？"
                ),
            )
        } else {
            (
                format!(
                    "This will clean Fylura settings, database, index, cache, logs, and Fylura AI credentials only for the current Windows account {sid}. History, Journal, and Restore records will be removed. Real files in Safe Trash and organized files stay in place.\n\nYes: clean data and continue. No: preserve data and continue. Cancel: stop uninstall."
                ),
                format!(
                    "Confirm full cleanup for account {sid}. Files left in Safe Trash may lose their History/Restore recovery records. Continue?"
                ),
            )
        };
        match windows_message_box(
            &question,
            "Fylura Uninstall",
            MB_YESNOCANCEL | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SETFOREGROUND,
        ) {
            IDNO => Ok(false),
            IDCANCEL => Err((
                1223,
                "Uninstall was cancelled. No Fylura data was changed.".to_string(),
            )),
            IDYES => match windows_message_box(
                &second,
                "Fylura Uninstall",
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SETFOREGROUND,
            ) {
                IDYES => Ok(true),
                _ => Ok(false),
            },
            _ => Err((
                1,
                "The account cleanup choice could not be verified.".to_string(),
            )),
        }
    }

    fn remove_current_user_autostart(main_executable: &Path) -> Result<(), String> {
        use winreg::{
            enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_BINARY},
            RegKey,
        };

        const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
        const STARTUP_APPROVED_KEY: &str =
            "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";
        const VALUE_NAME: &str = "Fylura";

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let run = match hkcu.open_subkey_with_flags(RUN_KEY, KEY_QUERY_VALUE | KEY_SET_VALUE) {
            Ok(key) => Some(key),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => {
                return Err("could not inspect the current user's Fylura startup entry".to_string())
            }
        };
        let run_command = match run.as_ref() {
            Some(key) => match key.get_value::<String, _>(VALUE_NAME) {
                Ok(command) => Some(command),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(_) => {
                    return Err(
                        "could not inspect the current user's Fylura startup entry".to_string()
                    )
                }
            },
            None => None,
        };
        if run_command.as_deref().is_some_and(|command| {
            !super::current_windows_autostart_command_matches(main_executable, command)
        }) {
            return Err(
                "the current user's Fylura startup entry could not be verified and was preserved"
                    .to_string(),
            );
        }

        let approved = match hkcu
            .open_subkey_with_flags(STARTUP_APPROVED_KEY, KEY_QUERY_VALUE | KEY_SET_VALUE)
        {
            Ok(key) => Some(key),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => {
                return Err(
                    "could not inspect the current user's Fylura Startup Apps entry".to_string(),
                )
            }
        };
        let approved_value = match approved.as_ref() {
            Some(key) => match key.get_raw_value(VALUE_NAME) {
                Ok(value) => Some(value),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(_) => {
                    return Err(
                        "could not inspect the current user's Fylura Startup Apps entry"
                            .to_string(),
                    )
                }
            },
            None => None,
        };
        if approved_value.as_ref().is_some_and(|value| {
            value.vtype != REG_BINARY
                || !super::current_windows_startup_approval_is_owned(&value.bytes)
        }) {
            return Err("the current user's Fylura Startup Apps entry could not be verified and was preserved".to_string());
        }

        if run_command.is_some() {
            run.as_ref()
                .expect("the Run key exists when its Fylura value exists")
                .delete_value(VALUE_NAME)
                .map_err(|_| {
                    "could not remove the current user's Fylura startup entry".to_string()
                })?;
            let still_present = run
                .as_ref()
                .expect("the Run key exists when its Fylura value exists")
                .get_value::<String, _>(VALUE_NAME);
            if !matches!(still_present, Err(ref error) if error.kind() == std::io::ErrorKind::NotFound)
            {
                return Err(
                    "the current user's Fylura startup entry remains after cleanup".to_string(),
                );
            }
        }
        if approved_value.is_some() {
            approved
                .as_ref()
                .expect("the Startup Apps key exists when its Fylura value exists")
                .delete_value(VALUE_NAME)
                .map_err(|_| {
                    "could not remove the current user's Fylura Startup Apps entry".to_string()
                })?;
            let still_present = approved
                .as_ref()
                .expect("the Startup Apps key exists when its Fylura value exists")
                .get_raw_value(VALUE_NAME);
            if !matches!(still_present, Err(ref error) if error.kind() == std::io::ErrorKind::NotFound)
            {
                return Err(
                    "the current user's Fylura Startup Apps entry remains after cleanup"
                        .to_string(),
                );
            }
        }
        Ok(())
    }

    pub(super) fn windows_uninstall_launcher(arguments: &[String]) -> Result<(), LauncherError> {
        let source_executable = std::env::current_exe()
            .map_err(|_| (1, "Could not locate the Fylura application.".to_string()))?;
        let install_dir = source_executable
            .parent()
            .filter(|path| path.is_absolute())
            .ok_or_else(|| {
                (
                    1,
                    "Could not verify the Fylura installation folder.".to_string(),
                )
            })?
            .to_path_buf();
        verify_registered_installation(&install_dir, &source_executable)?;
        let uninstaller = install_dir.join("uninstall.exe");
        require_regular_executable(&uninstaller)?;

        let wants_cleanup = contains_option(arguments, "/DELETE_APP_DATA");
        let confirmed_cleanup = contains_option(arguments, "/CONFIRM_DELETE_APP_DATA");
        if wants_cleanup && !confirmed_cleanup {
            return Err((
                2,
                "Full cleanup requires an explicit confirmation. No data was changed.".to_string(),
            ));
        }
        if confirmed_cleanup && !wants_cleanup {
            return Err((
                2,
                "Full cleanup confirmation did not include an explicit cleanup choice. No data was changed.".to_string(),
            ));
        }
        let updating = contains_option(arguments, "/UPDATE");
        let silent = contains_option(arguments, "/S") || contains_option(arguments, "/P");
        if wants_cleanup && silent {
            return Err((
                2,
                "Full cleanup requires the interactive account confirmation. No data was changed."
                    .to_string(),
            ));
        }
        let target_sid = super::verified_un_elevated_user_sid(
            current_windows_user_sid(),
            current_windows_interactive_session_sid(),
            current_windows_user_is_elevated(),
        );
        if wants_cleanup && target_sid.is_none() {
            return Err((
                1,
                "Full cleanup was not started because Windows could not verify the initiating user's account. Personal data and startup settings were preserved; rerun uninstall from the intended user's normal desktop session.".to_string(),
            ));
        }
        if target_sid.is_none() && !updating && !silent {
            windows_message_box(
                "Windows could not verify the initiating account because this process is elevated or its account token is unavailable. Fylura will be removed for this computer, while personal data and startup settings are retained.",
                "Fylura Uninstall",
                MB_OK | MB_ICONWARNING | MB_SETFOREGROUND,
            );
        }
        let delete_user_data = if updating || silent || target_sid.is_none() {
            false
        } else {
            confirm_data_cleanup(target_sid.as_deref().expect("verified SID is present"))?
        };
        let temp_root = dirs::cache_dir()
            .filter(|path| path.is_absolute())
            .map(|path| path.join("Temp"))
            .ok_or_else(|| {
                (
                    1,
                    "The current user's temporary folder is unavailable.".to_string(),
                )
            })?;
        fs::create_dir_all(&temp_root).map_err(|_| {
            (
                1,
                "Could not prepare a current-user uninstall helper.".to_string(),
            )
        })?;
        let worker = temp_root.join(format!("FyluraUninstall-{}.exe", uuid::Uuid::new_v4()));
        fs::copy(&source_executable, &worker).map_err(|_| {
            (
                1,
                "Could not prepare a current-user uninstall helper.".to_string(),
            )
        })?;

        let mut worker_args = vec![
            WORKER_FLAG.to_string(),
            std::process::id().to_string(),
            install_dir.to_string_lossy().into_owned(),
            target_sid.unwrap_or_default(),
            if delete_user_data { "1" } else { "0" }.to_string(),
            "--".to_string(),
        ];
        worker_args.extend(
            arguments
                .iter()
                .filter(|argument| {
                    !is_option(argument, "/DELETE_APP_DATA")
                        && !is_option(argument, "/CONFIRM_DELETE_APP_DATA")
                })
                .cloned(),
        );
        match Command::new(&worker).args(&worker_args).spawn() {
            Ok(_) => Ok(()),
            Err(_) => {
                let _ = fs::remove_file(&worker);
                Err((
                    1,
                    "Could not start the current-user uninstall helper.".to_string(),
                ))
            }
        }
    }

    pub(super) fn windows_uninstall_worker(arguments: &[String]) -> Result<i32, LauncherError> {
        if arguments
            .first()
            .is_some_and(|argument| argument == RETRY_CLEANUP_FLAG)
        {
            return retry_cleanup(
                arguments.get(1).map(String::as_str).unwrap_or_default(),
                arguments.get(2).map(String::as_str).unwrap_or_default(),
            );
        }
        if arguments.len() < 6 || arguments[0] != WORKER_FLAG || arguments[5] != "--" {
            return Err((
                1,
                "The uninstall helper could not verify its request.".to_string(),
            ));
        }
        let parent_pid = arguments[1].parse::<u32>().map_err(|_| {
            (
                1,
                "The uninstall helper could not verify its parent process.".to_string(),
            )
        })?;
        wait_for_launcher_exit(parent_pid)?;
        let install_dir = PathBuf::from(&arguments[2]);
        if !install_dir.is_absolute() || !install_dir.is_dir() {
            return Err((
                1,
                "The Fylura installation folder could not be verified.".to_string(),
            ));
        }
        let source_executable = install_dir.join("fylura.exe");
        verify_registered_installation(&install_dir, &source_executable)?;
        let uninstaller = install_dir.join("uninstall.exe");
        require_regular_executable(&uninstaller)?;
        let target_sid = arguments[3].clone();
        let delete_user_data = arguments[4] == "1";
        if delete_user_data
            && (target_sid.is_empty()
                || current_windows_user_sid().ok().as_deref() != Some(target_sid.as_str()))
        {
            return Err((
                1,
                "The current Windows account does not match the confirmed target. Personal data was preserved and uninstall was stopped.".to_string(),
            ));
        }
        let nsis_arguments = &arguments[6..];
        let exit_code = launch_elevated_uninstaller(&uninstaller, &install_dir, nsis_arguments)?;
        if exit_code != 0 {
            schedule_worker_deletion();
            if exit_code == 1223 {
                return Ok(1223);
            }
            return Err((
                exit_code as i32,
                format!("Fylura machine uninstall did not complete (exit code {exit_code}). User data was preserved."),
            ));
        }
        let target_sid = &arguments[3];
        let main_executable = install_dir.join("fylura.exe");
        if target_sid.is_empty() {
            schedule_worker_deletion();
            return Ok(0);
        }
        if current_windows_user_sid().ok().as_deref() != Some(target_sid.as_str()) {
            return Err((
                1,
                incomplete_cleanup_message(
                    "Windows could not re-verify the initiating account. User data and startup settings were preserved.",
                    target_sid,
                    &main_executable,
                ),
            ));
        }
        if !contains_option(nsis_arguments, "/UPDATE") {
            if let Err(error) = remove_current_user_autostart(&main_executable) {
                return Err((
                    1,
                    incomplete_cleanup_message(
                        &format!("current-user startup cleanup is INCOMPLETE: {error}"),
                        target_sid,
                        &main_executable,
                    ),
                ));
            }
        }
        if !delete_user_data {
            schedule_worker_deletion();
            return Ok(0);
        }
        match super::clean_current_user_data() {
            Ok(summary) => {
                schedule_worker_deletion();
                if !contains_option(nsis_arguments, "/S") {
                    windows_message_box(
                        &summary,
                        "Fylura Uninstall",
                        MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
                    );
                }
                Ok(0)
            }
            Err(error) => Err((
                1,
                incomplete_cleanup_message(
                    &format!("user data cleanup is INCOMPLETE: {error}"),
                    target_sid,
                    &main_executable,
                ),
            )),
        }
    }

    fn retry_cleanup(target_sid: &str, main_executable: &str) -> Result<i32, LauncherError> {
        require_current_user_retry_helper()?;
        if target_sid.is_empty() || current_windows_user_sid().ok().as_deref() != Some(target_sid) {
            return Err((
                1,
                "The current Windows account does not match the original cleanup target. No data was changed.".to_string(),
            ));
        }
        let main_executable = PathBuf::from(main_executable);
        if !main_executable.is_absolute()
            || !main_executable
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("fylura.exe"))
        {
            return Err((
                1,
                "The original Fylura installation path could not be verified. No data was changed."
                    .to_string(),
            ));
        }
        remove_current_user_autostart(&main_executable).map_err(|error| {
            (
                1,
                format!("Fylura current-user startup cleanup remains INCOMPLETE: {error}"),
            )
        })?;
        super::clean_current_user_data().map_err(|error| {
            (
                1,
                format!("Fylura user data cleanup remains INCOMPLETE: {error}"),
            )
        })?;
        schedule_worker_deletion();
        Ok(0)
    }

    fn contains_option(arguments: &[String], option: &str) -> bool {
        arguments.iter().any(|argument| is_option(argument, option))
    }

    fn incomplete_cleanup_message(
        detail: &str,
        target_sid: &str,
        main_executable: &Path,
    ) -> String {
        let helper = std::env::current_exe()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "the current Fylura uninstall helper".to_string());
        let main_executable = quote_windows_argument(&main_executable.to_string_lossy());
        format!(
            "Fylura machine uninstall completed, but cleanup is INCOMPLETE: {detail}\n\nThe helper was kept at:\n{helper}\n\nAfter fixing the reported access issue, rerun it with:\n\"{helper}\" {RETRY_CLEANUP_FLAG} {target_sid} {main_executable}"
        )
    }

    fn wait_for_launcher_exit(parent_pid: u32) -> Result<(), LauncherError> {
        let process = unsafe { OpenProcess(SYNCHRONIZE, 0, parent_pid) };
        if process.is_null() {
            // The launcher can exit before the worker starts. Only treat an
            // invalid PID as already exited; other failures leave installation
            // files untouched and stop the uninstall.
            let error = unsafe { GetLastError() };
            if error == 87 {
                return Ok(());
            }
            return Err((
                1,
                "The uninstall helper could not verify the launching process.".to_string(),
            ));
        }
        let status = unsafe { WaitForSingleObject(process, INFINITE) };
        unsafe { CloseHandle(process) };
        if status == 0 {
            Ok(())
        } else {
            Err((
                1,
                "The uninstall helper could not verify the launching process ended.".to_string(),
            ))
        }
    }

    fn is_option(argument: &str, option: &str) -> bool {
        argument.eq_ignore_ascii_case(option)
    }

    fn require_regular_executable(path: &Path) -> Result<(), LauncherError> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(()),
            _ => Err((
                1,
                "The Fylura uninstaller could not be verified.".to_string(),
            )),
        }
    }

    fn verify_registered_installation(
        install_dir: &Path,
        main_executable: &Path,
    ) -> Result<(), LauncherError> {
        use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};

        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let registration = hklm
            .open_subkey("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Fylura")
            .map_err(|_| {
                (
                    1,
                    "The registered Fylura machine installation could not be verified.".to_string(),
                )
            })?;
        let display_name: String = registration.get_value("DisplayName").map_err(|_| {
            (
                1,
                "The registered Fylura machine installation could not be verified.".to_string(),
            )
        })?;
        let main_binary: String = registration.get_value("MainBinaryName").map_err(|_| {
            (
                1,
                "The registered Fylura machine installation could not be verified.".to_string(),
            )
        })?;
        let registered_dir: String = registration.get_value("InstallLocation").map_err(|_| {
            (
                1,
                "The registered Fylura machine installation could not be verified.".to_string(),
            )
        })?;
        let registered_uninstall: String =
            registration.get_value("UninstallString").map_err(|_| {
                (
                    1,
                    "The registered Fylura machine installation could not be verified.".to_string(),
                )
            })?;
        let registered_dir = PathBuf::from(registered_dir.trim_matches('"'));
        let canonical_registered = fs::canonicalize(registered_dir).map_err(|_| {
            (
                1,
                "The registered Fylura machine installation could not be verified.".to_string(),
            )
        })?;
        let canonical_install = fs::canonicalize(install_dir).map_err(|_| {
            (
                1,
                "The Fylura installation folder could not be verified.".to_string(),
            )
        })?;
        let expected_uninstall = format!(
            "\"{}\" --fylura-uninstall-launcher",
            main_executable.to_string_lossy()
        );
        if !display_name.eq_ignore_ascii_case("Fylura")
            || !main_binary.eq_ignore_ascii_case("fylura.exe")
            || canonical_registered != canonical_install
            || !registered_uninstall.eq_ignore_ascii_case(&expected_uninstall)
        {
            return Err((
                1,
                "The registered Fylura machine installation could not be verified. No personal data was changed.".to_string(),
            ));
        }
        Ok(())
    }

    fn launch_elevated_uninstaller(
        executable: &Path,
        install_dir: &Path,
        arguments: &[String],
    ) -> Result<u32, LauncherError> {
        let verb = wide("runas");
        let executable = executable
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let directory = install_dir
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let parameters = arguments
            .iter()
            .map(|argument| quote_windows_argument(argument))
            .collect::<Vec<_>>()
            .join(" ");
        let parameters = wide(&parameters);
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = executable.as_ptr();
        info.lpParameters = parameters.as_ptr();
        info.lpDirectory = directory.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        if unsafe { ShellExecuteExW(&mut info) } == 0 {
            let error = unsafe { GetLastError() };
            if error == ERROR_CANCELLED {
                return Ok(1223);
            }
            return Err((
                error as i32,
                "Windows could not start the Fylura machine uninstaller. User data was preserved."
                    .to_string(),
            ));
        }
        if info.hProcess.is_null() {
            return Err((
                1,
                "Windows could not verify the Fylura machine uninstaller process. User data was preserved.".to_string(),
            ));
        }
        let wait_status = unsafe { WaitForSingleObject(info.hProcess, INFINITE) };
        let mut exit_code = 1u32;
        let exit_code_available =
            wait_status == 0 && unsafe { GetExitCodeProcess(info.hProcess, &mut exit_code) } != 0;
        unsafe { CloseHandle(info.hProcess) };
        if !exit_code_available {
            return Err((
                1,
                "Windows could not verify the Fylura machine uninstall result. User data was preserved.".to_string(),
            ));
        }
        Ok(exit_code)
    }

    fn quote_windows_argument(argument: &str) -> String {
        if !argument.is_empty()
            && !argument
                .chars()
                .any(|character| matches!(character, ' ' | '\t' | '"'))
        {
            return argument.to_string();
        }
        let mut quoted = String::from("\"");
        let mut backslashes = 0usize;
        for character in argument.chars() {
            match character {
                '\\' => backslashes += 1,
                '"' => {
                    quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                    quoted.push('"');
                    backslashes = 0;
                }
                _ => {
                    quoted.push_str(&"\\".repeat(backslashes));
                    quoted.push(character);
                    backslashes = 0;
                }
            }
        }
        quoted.push_str(&"\\".repeat(backslashes * 2));
        quoted.push('"');
        quoted
    }

    fn schedule_worker_deletion() {
        use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};
        let Ok(worker) = std::env::current_exe() else {
            return;
        };
        if !is_current_user_retry_helper(&worker) {
            return;
        }
        let worker = worker
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        unsafe {
            MoveFileExW(worker.as_ptr(), ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT);
        }
    }

    fn require_current_user_retry_helper() -> Result<(), LauncherError> {
        let worker = std::env::current_exe().map_err(|_| {
            (
                1,
                "The Fylura retry helper could not be verified.".to_string(),
            )
        })?;
        if is_current_user_retry_helper(&worker) {
            Ok(())
        } else {
            Err((
                1,
                "The Fylura cleanup retry helper could not be verified. No data was changed."
                    .to_string(),
            ))
        }
    }

    fn is_current_user_retry_helper(path: &Path) -> bool {
        let file_name = path.file_name().and_then(|name| name.to_str());
        let parent = path.parent();
        let expected_parent = dirs::cache_dir().map(|root| root.join("Temp"));
        matches!(file_name, Some(name) if name.starts_with("FyluraUninstall-"))
            && parent
                .zip(expected_parent)
                .is_some_and(|(parent, expected)| {
                    fs::canonicalize(parent).ok() == fs::canonicalize(expected).ok()
                })
    }

    pub(super) fn run_launcher(arguments: &[String]) -> i32 {
        match windows_uninstall_launcher(arguments) {
            Ok(()) => 0,
            Err((exit_code, message)) => {
                windows_message_box(&message, "Fylura Uninstall", ERROR);
                exit_code
            }
        }
    }

    pub(super) fn run_worker(arguments: &[String]) -> i32 {
        match windows_uninstall_worker(arguments) {
            Ok(exit_code) => exit_code,
            Err((exit_code, message)) => {
                let rejected_cross_account_request =
                    message.contains("does not match the original cleanup target");
                if !contains_option(arguments, "/S") || !rejected_cross_account_request {
                    windows_message_box(&message, "Fylura Uninstall", ERROR);
                }
                exit_code
            }
        }
    }
}

#[cfg(all(target_os = "windows", not(feature = "native-qa")))]
pub fn windows_uninstall_launcher(arguments: &[String]) -> i32 {
    windows_uninstall::run_launcher(arguments)
}

#[cfg(all(target_os = "windows", not(feature = "native-qa")))]
pub fn windows_uninstall_worker(arguments: &[String]) -> i32 {
    windows_uninstall::run_worker(arguments)
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn cleanup_checkpoint(phase: &str) {
    if std::env::var_os("FYLURA_CLEANUP_DIAGNOSTICS").is_some() {
        eprintln!("Fylura cleanup phase: {phase}");
    }
}

#[cfg(target_os = "macos")]
fn clean_macos_preferences_and_launch_agent() -> Result<CleanupResult, String> {
    let home =
        dirs::home_dir().ok_or_else(|| "Fylura user home directory is unavailable".to_string())?;
    let preferences = home
        .join("Library")
        .join("Preferences")
        .join("com.startlan.fylura.plist");
    for name in ["Fylura.plist", "com.startlan.fylura.plist"] {
        remove_current_macos_launch_agent(&home, name)?;
    }
    remove_exact_regular_file(&preferences, "Fylura preferences")?;
    let logs = home.join("Library").join("Logs").join(APP_IDENTIFIER);
    clean_owned_root(&logs)
}

#[cfg(any(target_os = "macos", test))]
fn remove_exact_regular_file(path: &Path, item: &str) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(format!("could not inspect {item}")),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("{item} was not a regular file and was preserved"));
    }
    fs::remove_file(path).map_err(|_| format!("could not remove {item}"))?;
    if path.exists() {
        return Err(format!("{item} remains after cleanup"));
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn is_owned_macos_launch_agent(name: &str, document: &plist::Value, executable: &Path) -> bool {
    let Some(dictionary) = document.as_dictionary() else {
        return false;
    };
    let label = dictionary.get("Label").and_then(plist::Value::as_string);
    let expected_label = match name {
        "Fylura.plist" => "Fylura",
        "com.startlan.fylura.plist" => "com.startlan.fylura",
        _ => return false,
    };
    let Some(arguments) = dictionary
        .get("ProgramArguments")
        .and_then(plist::Value::as_array)
    else {
        return false;
    };
    let Some(program) = arguments.first().and_then(plist::Value::as_string) else {
        return false;
    };
    label == Some(expected_label)
        && fs::canonicalize(program)
            .map(|program| program == executable)
            .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn remove_current_macos_launch_agent(home: &Path, name: &str) -> Result<(), String> {
    use std::process::Command;

    let path = home.join("Library").join("LaunchAgents").join(name);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("could not inspect Fylura login item".to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Fylura login item was not a regular file and was preserved".to_string());
    }
    let document = plist::Value::from_file(&path)
        .map_err(|_| "could not read Fylura login item".to_string())?;
    let current_executable = std::env::current_exe()
        .map_err(|_| "could not verify the Fylura login item executable".to_string())?;
    let current_executable = fs::canonicalize(&current_executable)
        .map_err(|_| "could not verify the Fylura login item executable".to_string())?;
    if !is_owned_macos_launch_agent(name, &document, &current_executable) {
        return Err("Fylura login item was not recognized and was preserved".to_string());
    }

    let label = document
        .as_dictionary()
        .and_then(|dictionary| dictionary.get("Label"))
        .and_then(plist::Value::as_string)
        .ok_or_else(|| "Fylura login item was not recognized and was preserved".to_string())?;
    cleanup_checkpoint("inspect-login-item-state");
    let loaded = Command::new("/bin/launchctl")
        .arg("list")
        .output()
        .map_err(|_| "could not inspect the Fylura login item state".to_string())?;
    if !loaded.status.success() {
        return Err("could not determine whether the Fylura login item is loaded".to_string());
    }
    let is_loaded = String::from_utf8_lossy(&loaded.stdout)
        .lines()
        .any(|line| line.split_whitespace().last() == Some(label));
    if is_loaded {
        cleanup_checkpoint("stop-login-item");
        let uid = unsafe { libc::geteuid() };
        let bootout = Command::new("/bin/launchctl")
            .args([
                "bootout",
                &format!("gui/{uid}"),
                path.to_str()
                    .ok_or_else(|| "Fylura login item path is invalid".to_string())?,
            ])
            .output()
            .map_err(|_| "could not stop the Fylura login item".to_string())?;
        if !bootout.status.success() {
            return Err("could not stop the Fylura login item; it was preserved".to_string());
        }
        let still_loaded = Command::new("/bin/launchctl")
            .arg("list")
            .output()
            .map_err(|_| "could not verify the Fylura login item stopped".to_string())?;
        if !still_loaded.status.success() {
            return Err(
                "could not verify the Fylura login item stopped; its file was preserved"
                    .to_string(),
            );
        }
        if String::from_utf8_lossy(&still_loaded.stdout)
            .lines()
            .any(|line| line.split_whitespace().last() == Some(label))
        {
            return Err("Fylura login item is still loaded; its file was preserved".to_string());
        }
    }
    cleanup_checkpoint("remove-login-item-file");
    remove_exact_regular_file(&path, "Fylura login item")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    };

    #[derive(Default)]
    struct TestCredentials {
        main: Mutex<Option<String>>,
        profiles: Mutex<std::collections::HashMap<String, String>>,
        foreign_profiles: Mutex<std::collections::HashMap<String, String>>,
        fail_enumeration: AtomicBool,
        fail_delete_profile: Mutex<Option<String>>,
    }

    impl crate::ai::settings::CredentialStore for TestCredentials {
        fn set(&self, value: &str) -> Result<(), String> {
            *self.main.lock().unwrap() = Some(value.to_string());
            Ok(())
        }
        fn get(&self) -> Result<Option<String>, String> {
            Ok(self.main.lock().unwrap().clone())
        }
        fn delete(&self) -> Result<(), String> {
            *self.main.lock().unwrap() = None;
            Ok(())
        }
        fn set_profile(&self, profile_id: &str, value: &str) -> Result<(), String> {
            self.profiles
                .lock()
                .unwrap()
                .insert(profile_id.to_string(), value.to_string());
            Ok(())
        }
        fn get_profile(&self, profile_id: &str) -> Result<Option<String>, String> {
            Ok(self.profiles.lock().unwrap().get(profile_id).cloned())
        }
        fn delete_profile(&self, profile_id: &str) -> Result<(), String> {
            if self.fail_delete_profile.lock().unwrap().as_deref() == Some(profile_id) {
                return Err("simulated credential removal failure".to_string());
            }
            self.profiles.lock().unwrap().remove(profile_id);
            Ok(())
        }

        fn list_profile_credential_ids(&self) -> Result<Vec<String>, String> {
            if self.fail_enumeration.load(Ordering::SeqCst) {
                return Err("simulated credential store access failure".to_string());
            }
            let mut ids = self
                .profiles
                .lock()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            ids.sort();
            Ok(ids)
        }
    }

    fn test_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "fylura-uninstall-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn windows_autostart_cleanup_requires_the_exact_fylura_command() {
        let executable = Path::new(r"C:\Program Files\Fylura\fylura.exe");

        assert!(current_windows_autostart_command_matches(
            executable,
            r"c:\program files\fylura\fylura.exe --background"
        ));
        assert!(!current_windows_autostart_command_matches(
            executable,
            r"C:\Program Files\Other App\fylura.exe --background"
        ));
        assert!(!current_windows_autostart_command_matches(
            executable,
            r"C:\Program Files\Fylura\fylura.exe --background --unrelated"
        ));
    }

    #[test]
    fn windows_startup_approval_cleanup_accepts_only_known_values() {
        assert!(current_windows_startup_approval_is_owned(&[
            0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]));
        assert!(current_windows_startup_approval_is_owned(&[
            0x03, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]));
        assert!(!current_windows_startup_approval_is_owned(&[
            0x02, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0,
        ]));
        assert!(!current_windows_startup_approval_is_owned(&[0x02; 12]));
        assert!(!current_windows_startup_approval_is_owned(&[0x02; 11]));
    }

    #[test]
    fn elevated_or_unverifiable_launchers_cannot_select_a_personal_data_target() {
        assert_eq!(
            verified_un_elevated_user_sid(
                Ok("S-1-5-21-100".to_string()),
                Ok("S-1-5-21-100".to_string()),
                Ok(false),
            ),
            Some("S-1-5-21-100".to_string())
        );
        assert_eq!(
            verified_un_elevated_user_sid(
                Ok("S-1-5-21-100".to_string()),
                Ok("S-1-5-21-100".to_string()),
                Ok(true),
            ),
            None
        );
        assert_eq!(
            verified_un_elevated_user_sid(
                Err("unavailable".to_string()),
                Ok("S-1-5-21-100".to_string()),
                Ok(false),
            ),
            None
        );
        assert_eq!(
            verified_un_elevated_user_sid(
                Ok("not-a-sid".to_string()),
                Ok("not-a-sid".to_string()),
                Ok(false),
            ),
            None
        );
        assert_eq!(
            verified_un_elevated_user_sid(
                Ok("S-1-5-21-100".to_string()),
                Ok("S-1-5-21-200".to_string()),
                Ok(false),
            ),
            None
        );
    }

    #[test]
    fn explicit_cleanup_removes_only_current_app_data_and_preserves_safe_trash() {
        let profile = test_root("profile");
        let data_root = profile.join("Application Support").join(APP_IDENTIFIER);
        let cache_root = profile.join("Caches").join(APP_IDENTIFIER);
        let unrelated = profile.join("Documents").join("keep.txt");
        fs::create_dir_all(data_root.join("safe-trash").join("batch")).unwrap();
        fs::create_dir_all(cache_root.join("logs")).unwrap();
        fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
        fs::write(data_root.join(DATABASE_NAME), b"database sentinel").unwrap();
        fs::write(
            data_root
                .join("safe-trash")
                .join("batch")
                .join("real-file.txt"),
            b"user file",
        )
        .unwrap();
        fs::write(cache_root.join("logs").join("app.log"), b"log sentinel").unwrap();
        fs::write(&unrelated, b"unrelated sentinel").unwrap();

        let result = clean_owned_root(&data_root).unwrap();
        assert_eq!(result.preserved_safe_trash_roots, 1);
        clean_owned_root(&cache_root).unwrap();

        assert!(!data_root.join(DATABASE_NAME).exists());
        assert!(data_root
            .join("safe-trash")
            .join("batch")
            .join("real-file.txt")
            .exists());
        assert!(!cache_root.exists());
        assert_eq!(fs::read(unrelated).unwrap(), b"unrelated sentinel");
        let _ = fs::remove_dir_all(profile);
    }

    #[test]
    fn cleanup_refuses_a_symlinked_app_data_root() {
        let parent = test_root("symlink");
        let target = parent.join("target");
        let root = parent.join(APP_IDENTIFIER);
        fs::create_dir_all(&target).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &root).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&target, &root).unwrap();

        assert!(clean_owned_root(&root).is_err());
        assert!(target.is_dir());
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn cleanup_enumerates_multiple_credentials_even_when_database_cannot_supply_ids() {
        use crate::ai::settings::CredentialStore;
        let root = test_root("credentials");
        let data_root = root.join("Application Support").join(APP_IDENTIFIER);
        let cache_root = root.join("Caches").join(APP_IDENTIFIER);
        fs::create_dir_all(&data_root).unwrap();
        fs::create_dir_all(&cache_root).unwrap();
        let database = data_root.join(DATABASE_NAME);
        let conn = rusqlite::Connection::open(&database).unwrap();
        conn.execute(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)",
            rusqlite::params![
                crate::ai::settings::AI_SETTINGS_KEY,
                r#"{"customProfiles":[{"id":"alpha"},{"id":"beta"}]}"#,
            ],
        )
        .unwrap();
        drop(conn);

        let credentials = TestCredentials::default();
        credentials.set("main-key").unwrap();
        credentials.set_profile("alpha", "alpha-key").unwrap();
        credentials.set_profile("beta", "beta-key").unwrap();
        credentials.foreign_profiles.lock().unwrap().insert(
            "other-service-profile".to_string(),
            "foreign-key".to_string(),
        );
        let other_user = TestCredentials::default();
        other_user.set_profile("alpha", "other-user-key").unwrap();

        clean_application_data_roots(&data_root, &cache_root, &credentials).unwrap();
        assert!(credentials.get().unwrap().is_none());
        assert!(credentials.get_profile("alpha").unwrap().is_none());
        assert!(credentials.get_profile("beta").unwrap().is_none());
        assert_eq!(
            credentials
                .foreign_profiles
                .lock()
                .unwrap()
                .get("other-service-profile"),
            Some(&"foreign-key".to_string())
        );
        assert_eq!(
            other_user.get_profile("alpha").unwrap().as_deref(),
            Some("other-user-key")
        );
        assert!(!database.exists());
    }

    #[test]
    fn cleanup_finds_orphan_profile_credential_without_database() {
        use crate::ai::settings::CredentialStore;

        let root = test_root("missing-database");
        let data_root = root.join("Application Support").join(APP_IDENTIFIER);
        let cache_root = root.join("Caches").join(APP_IDENTIFIER);
        fs::create_dir_all(&data_root).unwrap();
        fs::create_dir_all(&cache_root).unwrap();
        let credentials = TestCredentials::default();
        credentials.set_profile("orphan", "orphan-key").unwrap();

        clean_application_data_roots(&data_root, &cache_root, &credentials).unwrap();
        assert!(credentials.get_profile("orphan").unwrap().is_none());
        assert!(!data_root.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_finds_credentials_for_profile_removed_from_settings() {
        use crate::ai::settings::CredentialStore;

        let root = test_root("removed-profile");
        let data_root = root.join("Application Support").join(APP_IDENTIFIER);
        let cache_root = root.join("Caches").join(APP_IDENTIFIER);
        fs::create_dir_all(&data_root).unwrap();
        fs::create_dir_all(&cache_root).unwrap();
        let database = data_root.join(DATABASE_NAME);
        let conn = rusqlite::Connection::open(&database).unwrap();
        conn.execute(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)",
            rusqlite::params![
                crate::ai::settings::AI_SETTINGS_KEY,
                r#"{"customProfiles":[]}"#,
            ],
        )
        .unwrap();
        drop(conn);
        let credentials = TestCredentials::default();
        credentials.set_profile("deleted", "orphan-key").unwrap();

        clean_application_data_roots(&data_root, &cache_root, &credentials).unwrap();
        assert!(credentials.get_profile("deleted").unwrap().is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_discovers_credentials_when_database_is_corrupt() {
        use crate::ai::settings::CredentialStore;

        let root = test_root("corrupt-database");
        let data_root = root.join("Application Support").join(APP_IDENTIFIER);
        let cache_root = root.join("Caches").join(APP_IDENTIFIER);
        fs::create_dir_all(&data_root).unwrap();
        fs::create_dir_all(&cache_root).unwrap();
        fs::write(data_root.join(DATABASE_NAME), b"not a sqlite database").unwrap();
        let credentials = TestCredentials::default();
        credentials.set_profile("orphan", "orphan-key").unwrap();

        clean_application_data_roots(&data_root, &cache_root, &credentials).unwrap();
        assert!(credentials.get_profile("orphan").unwrap().is_none());
        assert!(!data_root.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_preserves_credentials_when_database_path_is_unreadable() {
        use crate::ai::settings::CredentialStore;

        let root = test_root("unreadable-database");
        let data_root = root.join("Application Support").join(APP_IDENTIFIER);
        let cache_root = root.join("Caches").join(APP_IDENTIFIER);
        fs::create_dir_all(data_root.join(DATABASE_NAME)).unwrap();
        fs::create_dir_all(&cache_root).unwrap();
        let credentials = TestCredentials::default();
        credentials.set("main-key").unwrap();
        credentials.set_profile("orphan", "orphan-key").unwrap();

        let error = clean_application_data_roots(&data_root, &cache_root, &credentials)
            .expect_err("a database directory cannot be inspected as a regular file");
        assert!(error.contains("database"));
        assert!(data_root.join(DATABASE_NAME).is_dir());
        assert_eq!(credentials.get().unwrap().as_deref(), Some("main-key"));
        assert_eq!(
            credentials.get_profile("orphan").unwrap().as_deref(),
            Some("orphan-key")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_store_access_failure_keeps_database_and_credentials() {
        use crate::ai::settings::CredentialStore;

        let root = test_root("credential-store-failure");
        let data_root = root.join("Application Support").join(APP_IDENTIFIER);
        let cache_root = root.join("Caches").join(APP_IDENTIFIER);
        fs::create_dir_all(&data_root).unwrap();
        fs::create_dir_all(&cache_root).unwrap();
        let database = data_root.join(DATABASE_NAME);
        fs::write(&database, b"database sentinel").unwrap();
        let credentials = TestCredentials::default();
        credentials.set("main-key").unwrap();
        credentials.set_profile("alpha", "profile-key").unwrap();
        credentials.fail_enumeration.store(true, Ordering::SeqCst);

        let error = clean_application_data_roots(&data_root, &cache_root, &credentials)
            .expect_err("credential enumeration failure must be incomplete");
        assert!(error.contains("cleanup incomplete"));
        assert!(database.exists());
        assert_eq!(credentials.get().unwrap().as_deref(), Some("main-key"));
        assert_eq!(
            credentials.get_profile("alpha").unwrap().as_deref(),
            Some("profile-key")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn interrupted_credential_cleanup_is_retryable_and_preserves_foreign_scopes() {
        use crate::ai::settings::CredentialStore;

        let root = test_root("credential-retry");
        let data_root = root.join("Application Support").join(APP_IDENTIFIER);
        let cache_root = root.join("Caches").join(APP_IDENTIFIER);
        fs::create_dir_all(&data_root).unwrap();
        fs::create_dir_all(&cache_root).unwrap();
        let database = data_root.join(DATABASE_NAME);
        fs::write(&database, b"database sentinel").unwrap();
        let credentials = TestCredentials::default();
        credentials.set("main-key").unwrap();
        credentials.set_profile("alpha", "alpha-key").unwrap();
        credentials.set_profile("beta", "beta-key").unwrap();
        *credentials.fail_delete_profile.lock().unwrap() = Some("alpha".to_string());

        assert!(clean_application_data_roots(&data_root, &cache_root, &credentials).is_err());
        assert!(database.exists());
        assert!(credentials.get().unwrap().is_none());
        assert_eq!(
            credentials.get_profile("alpha").unwrap().as_deref(),
            Some("alpha-key")
        );
        assert_eq!(
            credentials.get_profile("beta").unwrap().as_deref(),
            Some("beta-key")
        );

        *credentials.fail_delete_profile.lock().unwrap() = None;
        clean_application_data_roots(&data_root, &cache_root, &credentials)
            .expect("the next run enumerates and removes remaining keys");
        assert!(credentials.get_profile("alpha").unwrap().is_none());
        assert!(credentials.get_profile("beta").unwrap().is_none());
        assert!(!data_root.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_profile_credential_metadata_aborts_before_any_deletion() {
        use crate::ai::settings::CredentialStore;

        let credentials = TestCredentials::default();
        credentials.set("main-key").unwrap();
        credentials.set_profile("../foreign", "key").unwrap();
        let error = crate::ai::settings::clear_current_ai_credentials(&credentials)
            .expect_err("invalid profile identifiers cannot be deleted");
        assert!(error.contains("could not be identified"));
        assert_eq!(credentials.get().unwrap().as_deref(), Some("main-key"));
        assert_eq!(
            credentials.get_profile("../foreign").unwrap().as_deref(),
            Some("key")
        );
    }

    #[test]
    fn macos_cleanup_matches_only_the_current_executable_launch_agent() {
        let executable = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert(
            "Label".to_string(),
            plist::Value::String("Fylura".to_string()),
        );
        dictionary.insert(
            "ProgramArguments".to_string(),
            plist::Value::Array(vec![
                plist::Value::String(executable.to_string_lossy().into_owned()),
                plist::Value::String("--background".to_string()),
            ]),
        );
        let document = plist::Value::Dictionary(dictionary.clone());
        assert!(is_owned_macos_launch_agent(
            "Fylura.plist",
            &document,
            &executable
        ));
        assert!(!is_owned_macos_launch_agent(
            "com.startlan.fylura.plist",
            &document,
            &executable
        ));

        dictionary.insert(
            "Label".to_string(),
            plist::Value::String("Other App".to_string()),
        );
        let foreign_label = plist::Value::Dictionary(dictionary);
        assert!(!is_owned_macos_launch_agent(
            "Fylura.plist",
            &foreign_label,
            &executable
        ));

        let other_executable = executable.with_file_name("other-app");
        let mut foreign = plist::Dictionary::new();
        foreign.insert(
            "Label".to_string(),
            plist::Value::String("Fylura".to_string()),
        );
        foreign.insert(
            "ProgramArguments".to_string(),
            plist::Value::Array(vec![plist::Value::String(
                other_executable.to_string_lossy().into_owned(),
            )]),
        );
        assert!(!is_owned_macos_launch_agent(
            "Fylura.plist",
            &plist::Value::Dictionary(foreign),
            &executable,
        ));
    }

    #[test]
    fn macos_exact_file_cleanup_removes_only_the_requested_regular_file() {
        let root = test_root("macos-exact-file");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("com.startlan.fylura.plist");
        fs::write(&target, b"Fylura plist fixture").unwrap();
        remove_exact_regular_file(&target, "Fylura preferences").unwrap();
        assert!(!target.exists());
        let _ = fs::remove_dir_all(root);
    }
}
