//! Bounded, metadata-only context acquisition, outside the SQLite writer lock.
use super::*;
use crate::db::DbError;
use crate::global_index::{GlobalEntryInput, GlobalVolume};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

fn fenced(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Reparse, offline, recall-on-open/data-access. Never hydrate or resolve aliases.
        if metadata.file_attributes() & (0x400 | 0x1000 | 0x40000 | 0x400000) != 0 {
            return true;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.is_file() && metadata.nlink() > 1 {
            return true;
        }
    }
    false
}
fn plain(path: &Path, directory: bool) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| !fenced(&m) && m.is_dir() == directory)
}
fn identity(path: &Path) -> Option<String> {
    let m = fs::symlink_metadata(path).ok()?;
    if fenced(&m) || !m.is_dir() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(format!("{}:{}", m.dev(), m.ino()))
    }
    #[cfg(windows)]
    {
        use std::fs::OpenOptions;
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT,
        };
        let f = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .ok()?;
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(f.as_raw_handle(), &mut info) } == 0 {
            return None;
        }
        Some(format!(
            "{}:{:08x}{:08x}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ))
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}
fn checked_chain(path: &Path, root: &Path) -> bool {
    if !path.is_absolute() || !path.starts_with(root) {
        return false;
    }
    for (depth, ancestor) in path.ancestors().enumerate() {
        if depth >= 64 || !fs::symlink_metadata(ancestor).is_ok_and(|m| !fenced(&m)) {
            return false;
        }
        if ancestor == root {
            return true;
        }
    }
    false
}
fn performance_root(path: &Path, root: &Path) -> Option<&'static str> {
    for dir in path.ancestors().take(64) {
        if !dir.starts_with(root) {
            break;
        }
        let name = dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let parent = dir.parent().unwrap_or(root);
        let reason = match name {
            "node_modules"
                if plain(&parent.join("package.json"), false)
                    && plain(&dir.join(".package-lock.json"), false) =>
            {
                "npm_dependencies"
            }
            ".venv" | "venv"
                if plain(&dir.join("pyvenv.cfg"), false)
                    && (plain(&dir.join("bin"), true) || plain(&dir.join("Scripts"), true)) =>
            {
                "python_environment"
            }
            "target"
                if plain(&parent.join("Cargo.toml"), false)
                    && plain(&dir.join(".rustc_info.json"), false) =>
            {
                "rust_build_output"
            }
            _ if plain(&dir.join("CMakeCache.txt"), false)
                && plain(&dir.join("CMakeFiles"), true) =>
            {
                "cmake_build_output"
            }
            _ => continue,
        };
        return Some(reason);
    }
    None
}
fn protected(path: &Path) -> bool {
    #[cfg(windows)]
    {
        // GetWindowsDirectoryW obtains the OS-owned location, not a basename/env guess.
        use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
        let mut buffer = [0u16; 32768];
        let len =
            unsafe { GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
        if len > 0 && len < buffer.len() {
            let os_root = PathBuf::from(String::from_utf16_lossy(&buffer[..len]));
            if path.starts_with(&os_root) && identity(&os_root).is_some() {
                return true;
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if path.starts_with("/System") && identity(Path::new("/System")).is_some() {
            return true;
        }
    }
    let _ = path;
    false
}
pub fn bind_rule(
    source: &GlobalVolume,
    path: &str,
    include: bool,
) -> Result<CoverageRule, DbError> {
    if !source.enabled
        || (source.platform == "windows" && source.scope_authorization_state != "authorized")
    {
        return Err(DbError::Validation("coverage_source_not_authorized".into()));
    }
    let path = PathBuf::from(path);
    let root = Path::new(&source.mount_path);
    if !checked_chain(&path, root) || protected(&path) {
        return Err(DbError::Validation("coverage_folder_not_permitted".into()));
    }
    let directory_identity = identity(&path)
        .ok_or_else(|| DbError::Validation("coverage_directory_identity_unavailable".into()))?;
    Ok(CoverageRule {
        source_id: source.id.clone(),
        stable_volume_id: source.stable_volume_id.clone(),
        path: path.to_string_lossy().into_owned(),
        directory_identity,
        include,
    })
}
pub(crate) fn observe_batch(
    policy: &CoveragePolicy,
    source: &GlobalVolume,
    entries: &[GlobalEntryInput],
) -> Vec<Decision> {
    // Context cache is batch-local: never reuses facts across moves, policy revisions,
    // mounts or permission epochs. No filesystem I/O while SQLite owns the writer.
    let mut contexts: HashMap<PathBuf, Option<&'static str>> = HashMap::new();
    let root = Path::new(&source.mount_path);
    entries
        .iter()
        .map(|entry| {
            let path = Path::new(&entry.path);
            let mut e = Evidence {
                directory: entry.is_directory,
                ..Evidence::default()
            };
            if entry.volume_id != source.id
                || !path.is_absolute()
                || !path.starts_with(root)
                || path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Decision::Exclude("outside_authorized_source");
            }
            if entry.file_attributes
                & (crate::global_index::MACOS_FILE_ATTRIBUTE_CLOUD_NOT_LOCAL
                    | crate::global_index::MACOS_FILE_ATTRIBUTE_CONTENT_UNKNOWN)
                != 0
            {
                return Decision::Undetermined("cloud_availability_unverified");
            }
            if !checked_chain(path, root) {
                e.uncertain = Some("identity_permission_or_alias_unverified");
            } else {
                e.protected = protected(path);
                let dir = if entry.is_directory {
                    path
                } else {
                    path.parent().unwrap_or(root)
                };
                if contexts.len() >= 4096 {
                    contexts.clear();
                }
                e.performance_reason = *contexts
                    .entry(dir.to_path_buf())
                    .or_insert_with(|| performance_root(dir, root));
                if policy.mode == CoverageMode::Custom {
                    if let Some(rule) = policy
                        .rules
                        .iter()
                        .filter(|r| {
                            r.source_id == source.id
                                && r.stable_volume_id == source.stable_volume_id
                                && path.starts_with(&r.path)
                        })
                        .max_by_key(|r| (r.path.len(), !r.include))
                    {
                        if identity(Path::new(&rule.path)).as_ref()
                            == Some(&rule.directory_identity)
                        {
                            e.explicit_rule = Some(rule.include);
                        } else {
                            e.uncertain = Some("rule_directory_identity_changed");
                        }
                    }
                }
            }
            evaluate(policy, &e)
        })
        .collect()
}
