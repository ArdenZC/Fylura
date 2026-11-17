//! Windows Shell context-menu launch on backend-owned Browse entries.

use super::{runtime::FileWorkspaceRuntime, types::BrowseShellMenuRequest};
use crate::{
    file_workspace::{browse::BrowseEntryKind, BrowseEntryRef},
    fs_safety::capture_browse_identity,
    platform::windows::shell_context_menu,
};
use std::os::windows::process::CommandExt;
use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_SHELL_MENU_ITEMS: usize = 256;
const HELPER_EXE: &str = "fylura-shell-menu-helper.exe";
static SHELL_MENU_ACTIVE: AtomicBool = AtomicBool::new(false);

struct ShellMenuGuard;

impl ShellMenuGuard {
    fn acquire() -> Result<Self, String> {
        SHELL_MENU_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| "shell_menu_already_open".into())
    }
}

impl Drop for ShellMenuGuard {
    fn drop(&mut self) {
        SHELL_MENU_ACTIVE.store(false, Ordering::Release);
    }
}

impl FileWorkspaceRuntime {
    pub(crate) fn open_windows_shell_menu(
        &self,
        request: BrowseShellMenuRequest,
        owner_hwnd: usize,
        resource_dir: Option<&Path>,
    ) -> Result<super::types::BrowseShellMenuResponse, String> {
        self.ensure_live()?;
        let _active = ShellMenuGuard::acquire()?;
        let entries = self.resolve_shell_menu_entries(request.entry_refs)?;
        let helper = resolve_helper_path(resource_dir)?;
        let ipc = shell_context_menu::helper_request(owner_hwnd, entries);
        let payload =
            serde_json::to_vec(&ipc).map_err(|_| "shell_menu_request_invalid".to_string())?;

        let mut child = Command::new(helper)
            .arg("--fylura-shell-menu-v1")
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW; same user token, no elevation.
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| "shell_menu_helper_unavailable".to_string())?;
        let write_result = child
            .stdin
            .take()
            .ok_or_else(|| "shell_menu_helper_unavailable".to_string())?
            .write_all(&payload);
        let output = child
            .wait_with_output()
            .map_err(|_| "shell_menu_helper_failed".to_string())?;
        if write_result.is_err() {
            return Err("shell_menu_helper_failed".into());
        }
        let (selected, error) = shell_context_menu::helper_response(&output.stdout)?;
        if let Some(error) = error {
            return Err(error);
        }
        if !output.status.success() {
            return Err("shell_menu_helper_failed".into());
        }

        // The selected Shell verb is outside Fylura's Transfer/Journal/Restore
        // authority. BrowseMode performs a fresh enumeration when this IPC
        // returns; filesystem watchers continue observing later async work.
        Ok(super::types::BrowseShellMenuResponse { selected })
    }

    fn resolve_shell_menu_entries(
        &self,
        entry_refs: Vec<BrowseEntryRef>,
    ) -> Result<Vec<shell_context_menu::HelperMenuEntry>, String> {
        if entry_refs.is_empty() || entry_refs.len() > MAX_SHELL_MENU_ITEMS {
            return Err("shell_menu_selection_limit".into());
        }
        let first_session = browse_session_id(&entry_refs[0])?;
        let mut seen = HashSet::with_capacity(entry_refs.len());
        let mut parent = None;
        let mut entries = Vec::with_capacity(entry_refs.len());

        for entry_ref in &entry_refs {
            let (session_id, entry_id) = browse_entry_id(entry_ref)?;
            if session_id != first_session || !seen.insert(entry_id.to_string()) {
                return Err("shell_menu_selection_invalid".into());
            }
            let entry = self
                .inner
                .browse
                .resolve_current_entry(entry_ref)
                .map_err(|_| "shell_menu_entry_changed_or_unavailable".to_string())?;
            if !matches!(
                entry.kind,
                BrowseEntryKind::File | BrowseEntryKind::Directory
            ) {
                return Err("shell_menu_entry_unavailable".into());
            }
            let expected = entry
                .identity
                .as_ref()
                .ok_or_else(|| "shell_menu_identity_unavailable".to_string())?;
            let current = capture_browse_identity(&entry.path)
                .map_err(|_| "shell_menu_entry_changed_or_unavailable".to_string())?;
            if &current != expected {
                return Err("shell_menu_entry_changed_or_unavailable".into());
            }
            let metadata = std::fs::symlink_metadata(&entry.path)
                .map_err(|_| "shell_menu_entry_changed_or_unavailable".to_string())?;
            if (entry.kind == BrowseEntryKind::File && !metadata.is_file())
                || (entry.kind == BrowseEntryKind::Directory && !metadata.is_dir())
            {
                return Err("shell_menu_entry_changed_or_unavailable".into());
            }

            // Check both the Browse spelling and its resolved filesystem
            // location so a junction or symbolic ancestor cannot disguise a
            // protected Windows directory. canonicalize reads path metadata;
            // it does not open file contents.
            let resolved_path = std::fs::canonicalize(&entry.path)
                .map_err(|_| "shell_menu_entry_changed_or_unavailable".to_string())?;
            crate::file_ops::ensure_not_safe_trash_operation(&entry.path)?;
            crate::file_ops::ensure_not_safe_trash_operation(&resolved_path)?;
            crate::file_ops::ensure_general_file_operation_allowed_for_os_with_temp(
                &entry.path,
                "windows",
                None,
            )?;
            crate::file_ops::ensure_general_file_operation_allowed_for_os_with_temp(
                &resolved_path,
                "windows",
                None,
            )?;

            let current_parent = windows_parent_key(&entry.path)?;
            if parent
                .as_ref()
                .is_some_and(|parent| parent != &current_parent)
            {
                return Err("shell_menu_selection_mixed_folders".into());
            }
            parent = Some(current_parent);
            entries.push(shell_context_menu::helper_entry(&entry.path, &current)?);
        }
        Ok(entries)
    }
}

fn browse_session_id(entry_ref: &BrowseEntryRef) -> Result<&str, String> {
    browse_entry_id(entry_ref).map(|(session_id, _)| session_id)
}

fn browse_entry_id(entry_ref: &BrowseEntryRef) -> Result<(&str, &str), String> {
    match entry_ref {
        BrowseEntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        } => Ok((browse_session_id, entry_id)),
    }
}

fn windows_parent_key(path: &Path) -> Result<String, String> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| {
            parent
                .to_string_lossy()
                .replace('/', "\\")
                .trim_end_matches('\\')
                .to_ascii_lowercase()
        })
        .ok_or_else(|| "shell_menu_entry_unavailable".to_string())
}

fn resolve_helper_path(resource_dir: Option<&Path>) -> Result<PathBuf, String> {
    let triple = option_env!("TAURI_ENV_TARGET_TRIPLE").unwrap_or("x86_64-pc-windows-msvc");
    if let Some(resources) = resource_dir {
        let packaged = resources
            .join("binaries")
            .join(format!("fylura-shell-menu-helper-{triple}.exe"));
        if packaged.is_file() {
            return Ok(packaged);
        }
        let flat = resources.join(HELPER_EXE);
        if flat.is_file() {
            return Ok(flat);
        }
    }
    // Dev builds place the helper next to the app executable. Prefer bundle
    // resources above this fallback so a stale neighboring binary cannot
    // shadow the version shipped with an installed app.
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.join(HELPER_EXE)));
    if let Some(path) = sibling.filter(|path| path.is_file()) {
        return Ok(path);
    }
    Err("shell_menu_helper_unavailable".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_folder_comparison_ignores_windows_case_and_trailing_separator() {
        assert_eq!(
            windows_parent_key(Path::new(r"C:\Users\A\One.txt")).unwrap(),
            windows_parent_key(Path::new(r"c:\users\a\Two.txt")).unwrap()
        );
        assert_ne!(
            windows_parent_key(Path::new(r"C:\Users\A\One.txt")).unwrap(),
            windows_parent_key(Path::new(r"C:\Users\B\Two.txt")).unwrap()
        );
    }

    #[test]
    fn packaged_helper_resource_is_resolved_before_the_dev_sibling() {
        struct TestDirectory(PathBuf);
        impl Drop for TestDirectory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let root = std::env::temp_dir().join(format!(
            "fylura-shell-helper-resource-{}",
            uuid::Uuid::new_v4()
        ));
        let binary_dir = root.join("binaries");
        std::fs::create_dir_all(&binary_dir).expect("create packaged resource directory");
        let _cleanup = TestDirectory(root.clone());
        let triple = option_env!("TAURI_ENV_TARGET_TRIPLE").unwrap_or("x86_64-pc-windows-msvc");
        let packaged = binary_dir.join(format!("fylura-shell-menu-helper-{triple}.exe"));
        std::fs::write(&packaged, b"test helper placeholder").expect("stage helper placeholder");

        assert_eq!(resolve_helper_path(Some(&root)).unwrap(), packaged);
    }
}
