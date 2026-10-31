//! Isolated identifiers and an RAII registration seam for real-host evidence.
//!
//! This module is compiled only for tests or with `test-registration`. It is
//! not part of the production DLL. Registration is deliberately limited to
//! HKCU and the dedicated extension/ProgID/CLSID below. The guard refuses to
//! overwrite an existing exact target, records every value mutation, and
//! removes only keys it created when it is explicitly cleaned or unwound.

use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use fylura_windows_preview_registration as registration;
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR},
        System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegOpenKeyExW,
            RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE,
            KEY_WRITE, REG_CREATED_NEW_KEY, REG_OPTION_NON_VOLATILE, REG_SZ,
        },
    },
};
use windows_core::GUID;

pub const TEST_CLSID: GUID = GUID::from_u128(0x5b6e7f80_91a2_43b4_c5d6_e7f8091a2b3c);
pub const TEST_EXTENSION: &str = ".zcv2preview";
pub const TEST_PROGID: &str = "ZenCanvas.W4_03_V2.Test";
pub const PREVIEW_HANDLER_SHELLEX_CLSID: GUID =
    GUID::from_u128(registration::SHELLEX_CATEGORY_U128);
pub const PREVHOST_APP_ID: GUID = GUID::from_u128(registration::PREVHOST_APP_ID_U128);
pub const FRIENDLY_NAME: &str = "Fylura W4-04 Test Preview Handler";

pub fn clsid_string() -> String {
    format!("{{{TEST_CLSID:?}}}")
}

pub fn shellex_clsid_string() -> String {
    format!("{{{PREVIEW_HANDLER_SHELLEX_CLSID:?}}}")
}

pub fn prevhost_app_id_string() -> String {
    format!("{{{PREVHOST_APP_ID:?}}}")
}

#[derive(Debug)]
pub struct RegistryError {
    operation: String,
    code: Option<u32>,
}

impl RegistryError {
    fn win32(operation: impl Into<String>, code: WIN32_ERROR) -> Self {
        Self {
            operation: operation.into(),
            code: Some(code.0),
        }
    }

    fn message(message: impl Into<String>) -> Self {
        Self {
            operation: message.into(),
            code: None,
        }
    }
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.code {
            Some(code) => write!(formatter, "{} (Win32 error {})", self.operation, code),
            None => formatter.write_str(&self.operation),
        }
    }
}

impl Error for RegistryError {}

struct KeyHandle(HKEY);

impl Drop for KeyHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

/// Exact registry mutations used by the real Explorer/prevhost seam.
///
/// The official preview-handler registration shape is file-association based:
/// extension -> ProgID -> `shellex` preview-handler CLSID, plus the handler's
/// HKCU CLSID/InprocServer32 registration and the PreviewHandlers list entry.
/// No `DisableLowILProcessIsolation` value or custom AppID/DllSurrogate key is
/// created here; the x64 handler points at the system Prevhost AppID.
pub struct Registration {
    created_paths: Vec<String>,
    created_values: Vec<(String, String)>,
    mutations: Vec<String>,
    cleaned: bool,
}

impl Registration {
    pub fn install(dll_path: &Path) -> Result<Self, RegistryError> {
        let dll_registration_path = normal_registry_path(dll_path);
        if !dll_registration_path.is_file() {
            return Err(RegistryError::message(format!(
                "preview handler DLL does not exist: {}",
                dll_registration_path.display()
            )));
        }

        let mut registration = Self {
            created_paths: Vec::new(),
            created_values: Vec::new(),
            mutations: Vec::new(),
            cleaned: false,
        };
        registration.ensure_targets_absent()?;

        let extension_path = format!(r"Software\Classes\{TEST_EXTENSION}");
        let extension_preview_shellex_path =
            format!(r"{extension_path}\shellex\{}", shellex_clsid_string());
        let progid_path = format!(r"Software\Classes\{TEST_PROGID}");
        let preview_shellex_path = format!(r"{progid_path}\shellex\{}", shellex_clsid_string());
        let clsid_path = format!(r"Software\Classes\CLSID\{}", clsid_string());
        let inproc_path = format!(r"{clsid_path}\InprocServer32");
        let preview_handlers_path = r"Software\Microsoft\Windows\CurrentVersion\PreviewHandlers";

        registration.set_string(&extension_path, "", TEST_PROGID)?;
        // Current Explorer registrations commonly expose the same preview
        // handler directly under the extension's ShellEx key. Keep the
        // documented ProgID chain below as well; both paths resolve to this
        // isolated test CLSID and are removed by the same guard.
        registration.set_string(&extension_preview_shellex_path, "", &clsid_string())?;
        registration.set_string(&preview_shellex_path, "", &clsid_string())?;

        registration.set_string(&clsid_path, "AppID", &prevhost_app_id_string())?;
        registration.set_string(&inproc_path, "", &dll_registration_path.to_string_lossy())?;
        registration.set_string(&inproc_path, "ThreadingModel", "Apartment")?;
        registration.set_string(&inproc_path, "ProgID", TEST_PROGID)?;
        registration.set_string(&inproc_path, "VersionIndependentProgID", TEST_PROGID)?;

        // Keep the official friendly-name list value. It is diagnostic-only
        // according to Microsoft, but some Explorer versions use the list as
        // an enumeration/cache hint even when the file-association chain is
        // already present.
        registration.set_string(preview_handlers_path, &clsid_string(), FRIENDLY_NAME)?;

        Ok(registration)
    }

    pub fn created_key_paths(&self) -> &[String] {
        &self.created_paths
    }

    pub fn mutation_lines(&self) -> &[String] {
        &self.mutations
    }

    /// Remove all values and keys created by this guard and prove that every
    /// exact target is absent. Calling this is the normal `finally` path;
    /// `Drop` retries it on panic/unwind or an early return.
    pub fn cleanup(&mut self) -> Result<Vec<String>, RegistryError> {
        if self.cleaned {
            return Ok(Vec::new());
        }

        let mut report = Vec::new();
        let mut first_error = None;
        for (path, value_name) in self.created_values.iter().rev() {
            match delete_value(path, value_name) {
                Ok(()) => report.push(format!(
                    "HKCU\\{path}\\{}: deleted or already absent",
                    display_value_name(value_name)
                )),
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }

        for (path, value_name) in &self.created_values {
            match value_exists(path, value_name) {
                Ok(false) => report.push(format!(
                    "HKCU\\{path}\\{}: absent (PASS)",
                    display_value_name(value_name)
                )),
                Ok(true) => {
                    first_error.get_or_insert_with(|| {
                        RegistryError::message(format!(
                            "HKCU\\{path}\\{} remained after cleanup",
                            display_value_name(value_name)
                        ))
                    });
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }

        for path in self.created_paths.iter().rev() {
            let wide = wide(path);
            let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, pcwstr(&wide)) };
            if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
                first_error.get_or_insert_with(|| {
                    RegistryError::win32(format!("delete HKCU\\{path}"), status)
                });
            } else {
                report.push(format!("HKCU\\{path}: deleted or already absent"));
            }
        }

        for path in &self.created_paths {
            match key_exists(path) {
                Ok(false) => report.push(format!("HKCU\\{path}: absent (PASS)")),
                Ok(true) => {
                    first_error.get_or_insert_with(|| {
                        RegistryError::message(format!("HKCU\\{path} remained after cleanup"))
                    });
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }

        if let Some(error) = first_error {
            Err(error)
        } else {
            self.cleaned = true;
            Ok(report)
        }
    }

    fn ensure_targets_absent(&self) -> Result<(), RegistryError> {
        let paths = self.target_paths();
        for path in paths {
            if key_exists(&path)? {
                return Err(RegistryError::message(format!(
                    "refusing to overwrite existing test registration key HKCU\\{path}"
                )));
            }
        }
        let preview_handlers_path = r"Software\Microsoft\Windows\CurrentVersion\PreviewHandlers";
        let preview_handlers_value = clsid_string();
        if value_exists(preview_handlers_path, &preview_handlers_value)? {
            return Err(RegistryError::message(format!(
                "refusing to overwrite existing test registration value HKCU\\{preview_handlers_path}\\{preview_handlers_value}"
            )));
        }
        Ok(())
    }

    fn target_paths(&self) -> Vec<String> {
        let extension_path = format!(r"Software\Classes\{TEST_EXTENSION}");
        let extension_preview_shellex_path =
            format!(r"{extension_path}\shellex\{}", shellex_clsid_string());
        let progid_path = format!(r"Software\Classes\{TEST_PROGID}");
        let preview_shellex_path = format!(r"{progid_path}\shellex\{}", shellex_clsid_string());
        let clsid_path = format!(r"Software\Classes\CLSID\{}", clsid_string());
        let inproc_path = format!(r"{clsid_path}\InprocServer32");
        let preview_handlers_path = r"Software\Microsoft\Windows\CurrentVersion\PreviewHandlers";
        vec![
            extension_path,
            extension_preview_shellex_path,
            progid_path,
            preview_shellex_path,
            clsid_path,
            inproc_path,
            format!(r"{preview_handlers_path}\{}", clsid_string()),
        ]
    }

    fn create_key_handle(&mut self, path: &str) -> Result<KeyHandle, RegistryError> {
        let mut prefix = String::new();
        let mut leaf = None;
        for component in path.split('\\') {
            if component.is_empty() {
                return Err(RegistryError::message(format!(
                    "invalid empty registry path component in HKCU\\{path}"
                )));
            }
            if !prefix.is_empty() {
                prefix.push('\\');
            }
            prefix.push_str(component);

            let wide = wide(&prefix);
            let mut handle = HKEY::default();
            let mut disposition = Default::default();
            let status = unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    pcwstr(&wide),
                    None,
                    PCWSTR::null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_READ | KEY_WRITE,
                    None,
                    &mut handle,
                    Some(&mut disposition),
                )
            };
            if status != ERROR_SUCCESS {
                return Err(RegistryError::win32(
                    format!("create HKCU\\{prefix}"),
                    status,
                ));
            }
            if disposition.0 == REG_CREATED_NEW_KEY.0 {
                self.created_paths.push(prefix.clone());
            }
            leaf = Some(KeyHandle(handle));
        }

        leaf.ok_or_else(|| RegistryError::message("empty registry path"))
    }

    fn set_string(
        &mut self,
        path: &str,
        value_name: &str,
        value: &str,
    ) -> Result<(), RegistryError> {
        let handle = self.create_key_handle(path)?;
        let value_wide = wide(value);
        let value_bytes = value_wide
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect::<Vec<_>>();
        let value_name_wide = wide(value_name);
        let status = unsafe {
            RegSetValueExW(
                handle.0,
                pcwstr(&value_name_wide),
                None,
                REG_SZ,
                Some(&value_bytes),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(RegistryError::win32(
                format!("set HKCU\\{path}\\{}", display_value_name(value_name)),
                status,
            ));
        }

        self.mutations.push(format!(
            "HKCU\\{path}\\{} = REG_SZ {value}",
            display_value_name(value_name)
        ));
        self.created_values
            .push((path.to_string(), value_name.to_string()));
        Ok(())
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if !self.cleaned {
            if let Err(error) = self.cleanup() {
                eprintln!("REGISTRY cleanup FAILED: {error}");
            }
        }
    }
}

fn display_value_name(name: &str) -> &str {
    if name.is_empty() {
        "(Default)"
    } else {
        name
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn pcwstr(value: &[u16]) -> PCWSTR {
    PCWSTR(value.as_ptr())
}

fn normal_registry_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(unc_path) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{unc_path}"))
    } else if let Some(local_path) = text.strip_prefix(r"\\?\") {
        PathBuf::from(local_path)
    } else {
        path.to_path_buf()
    }
}

fn key_exists(path: &str) -> Result<bool, RegistryError> {
    let path_wide = wide(path);
    let mut handle = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            pcwstr(&path_wide),
            None,
            KEY_READ,
            &mut handle,
        )
    };
    match status {
        ERROR_SUCCESS => {
            unsafe {
                let _ = RegCloseKey(handle);
            }
            Ok(true)
        }
        ERROR_FILE_NOT_FOUND => Ok(false),
        status => Err(RegistryError::win32(format!("probe HKCU\\{path}"), status)),
    }
}

fn delete_value(path: &str, value_name: &str) -> Result<(), RegistryError> {
    let path_wide = wide(path);
    let value_wide = wide(value_name);
    let mut handle = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            pcwstr(&path_wide),
            None,
            KEY_SET_VALUE,
            &mut handle,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    if status != ERROR_SUCCESS {
        return Err(RegistryError::win32(
            format!("open HKCU\\{path} for value cleanup"),
            status,
        ));
    }

    let status = unsafe { RegDeleteValueW(handle, pcwstr(&value_wide)) };
    unsafe {
        let _ = RegCloseKey(handle);
    }
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(RegistryError::win32(
            format!("delete HKCU\\{path}\\{}", display_value_name(value_name)),
            status,
        ))
    }
}

fn value_exists(path: &str, value_name: &str) -> Result<bool, RegistryError> {
    let path_wide = wide(path);
    let mut handle = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            pcwstr(&path_wide),
            None,
            KEY_READ,
            &mut handle,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(false);
    }
    if status != ERROR_SUCCESS {
        return Err(RegistryError::win32(format!("probe HKCU\\{path}"), status));
    }

    let value_wide = wide(value_name);
    let status = unsafe { RegQueryValueExW(handle, pcwstr(&value_wide), None, None, None, None) };
    unsafe {
        let _ = RegCloseKey(handle);
    }
    match status {
        ERROR_SUCCESS => Ok(true),
        ERROR_FILE_NOT_FOUND => Ok(false),
        status => Err(RegistryError::win32(
            format!("probe HKCU\\{path}\\{}", display_value_name(value_name)),
            status,
        )),
    }
}
