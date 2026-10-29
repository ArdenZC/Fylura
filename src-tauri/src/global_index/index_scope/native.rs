//! Bounded, metadata-only context acquisition, outside the SQLite writer lock.
use super::*;
use crate::db::DbError;
use crate::global_index::{GlobalEntryInput, GlobalVolume};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

#[cfg(test)]
thread_local! { static METADATA_ATTEMPTS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
pub(super) fn take_metadata_attempts() -> u64 {
    METADATA_ATTEMPTS.with(|n| n.replace(0))
}
fn metadata(path: &Path) -> std::io::Result<fs::Metadata> {
    #[cfg(test)]
    METADATA_ATTEMPTS.with(|n| n.set(n.get() + 1));
    fs::symlink_metadata(path)
}
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
    metadata(path).is_ok_and(|m| !fenced(&m) && m.is_dir() == directory)
}
fn identity(path: &Path) -> Option<String> {
    let m = metadata(path).ok()?;
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
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        };
        let f = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
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
fn single_link(path: &Path, m: &fs::Metadata) -> bool {
    let _ = (path, m);
    #[cfg(windows)]
    {
        if m.is_file() {
            use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
            use windows_sys::Win32::Storage::FileSystem::{
                GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
                FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
            };
            let Ok(file) = fs::OpenOptions::new()
                .access_mode(FILE_READ_ATTRIBUTES)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(path)
            else {
                return false;
            };
            #[cfg(test)]
            METADATA_ATTEMPTS.with(|n| n.set(n.get() + 1));
            let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
            return unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } != 0
                && info.nNumberOfLinks == 1;
        }
    }
    true
}
fn checked_chain(path: &Path, root: &Path) -> bool {
    if !path.is_absolute()
        || !path.starts_with(root)
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return false;
    }
    for (depth, ancestor) in path.ancestors().enumerate() {
        if depth >= 64
            || !metadata(ancestor).is_ok_and(|m| !fenced(&m) && single_link(ancestor, &m))
        {
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
                    && [".package-lock.json", ".modules.yaml", ".yarn-integrity"]
                        .iter()
                        .any(|marker| plain(&dir.join(marker), false)) =>
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
fn protected_roots(source_root: &Path) -> Vec<PathBuf> {
    let _ = source_root;
    let mut roots = Vec::new();
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
        let mut buffer = vec![0u16; 32768];
        let len =
            unsafe { GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
        if len > 0 && len < buffer.len() {
            let root = PathBuf::from(String::from_utf16_lossy(&buffer[..len]));
            if root.starts_with(source_root) && identity(&root).is_some() {
                roots.push(root);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if Path::new("/System").starts_with(source_root) && identity(Path::new("/System")).is_some()
        {
            roots.push(PathBuf::from("/System"));
        }
    }
    // Keep an explicit mutable collection on unsupported test platforms too.
    roots.shrink_to_fit();
    roots
}
fn protected(path: &Path, source_root: &Path) -> bool {
    protected_roots(source_root)
        .iter()
        .any(|root| path.starts_with(root))
}
fn known_cache_roots(source_root: &Path) -> Vec<PathBuf> {
    let _ = source_root;
    let mut roots = Vec::new();
    #[cfg(target_os = "macos")]
    {
        if let Some(root) = dirs::cache_dir() {
            if root.starts_with(source_root) && identity(&root).is_some() {
                roots.push(root);
            }
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            System::Com::CoTaskMemFree,
            UI::Shell::{FOLDERID_InternetCache, SHGetKnownFolderPath},
        };
        let mut ptr = std::ptr::null_mut();
        if unsafe {
            SHGetKnownFolderPath(&FOLDERID_InternetCache, 0, std::ptr::null_mut(), &mut ptr)
        } >= 0
            && !ptr.is_null()
        {
            let mut len = 0;
            while len < 32768 && unsafe { *ptr.add(len) } != 0 {
                len += 1;
            }
            if len < 32768 {
                let path = PathBuf::from(String::from_utf16_lossy(unsafe {
                    std::slice::from_raw_parts(ptr, len)
                }));
                if path.starts_with(source_root) && identity(&path).is_some() {
                    roots.push(path);
                }
            }
            unsafe { CoTaskMemFree(ptr.cast()) };
        }
    }
    roots.shrink_to_fit();
    roots
}
pub(crate) fn source_identity_current(source: &GlobalVolume) -> bool {
    #[cfg(windows)]
    if source.platform == "windows" {
        return crate::global_index::windows::volumes::source_identity_current(source);
    }
    #[cfg(target_os = "macos")]
    if source.platform == "macos" {
        return crate::global_index::macos::source_identity_current(source);
    }
    let _ = source;
    true
}
pub fn bind_rule(
    source: &GlobalVolume,
    path: &str,
    include: bool,
) -> Result<CoverageRule, DbError> {
    if !source.enabled
        || (source.platform == "windows" && source.scope_authorization_state != "authorized")
        || !source_identity_current(source)
    {
        return Err(DbError::Validation("coverage_source_not_authorized".into()));
    }
    let path = PathBuf::from(path);
    let root = Path::new(&source.mount_path);
    if !checked_chain(&path, root) || protected(&path, root) {
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
    let boundaries = protected_roots(root);
    let caches = known_cache_roots(root);
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
            if boundaries.iter().any(|boundary| path.starts_with(boundary)) {
                return Decision::Exclude("protected_boundary");
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
                e.protected = false;
                let dir = if entry.is_directory {
                    path
                } else {
                    path.parent().unwrap_or(root)
                };
                if contexts.len() >= 4096 {
                    contexts.clear();
                }
                e.performance_reason = *contexts.entry(dir.to_path_buf()).or_insert_with(|| {
                    if caches.iter().any(|cache| dir.starts_with(cache)) {
                        Some("known_user_cache")
                    } else {
                        performance_root(dir, root)
                    }
                });
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
