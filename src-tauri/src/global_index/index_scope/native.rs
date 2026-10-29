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
    object_identity(path, true)
}
fn object_identity(path: &Path, directory_only: bool) -> Option<String> {
    let m = metadata(path).ok()?;
    if fenced(&m) || (directory_only && !m.is_dir()) {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(format!("mac:dev:{:x}:ino:{:x}", m.dev(), m.ino()))
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
        #[cfg(test)]
        METADATA_ATTEMPTS.with(|n| n.set(n.get() + 1));
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
#[cfg(test)]
pub(super) fn enrich_fixture_identity(entry: &mut GlobalEntryInput) {
    let path = Path::new(&entry.path);
    let to_provider_id = |stamp: String| {
        #[cfg(windows)]
        {
            stamp.rsplit(':').next().unwrap().to_string()
        }
        #[cfg(not(windows))]
        {
            stamp
        }
    };
    if let Some(id) = object_identity(path, false) {
        entry.platform_file_id = to_provider_id(id);
    }
    if let Some(id) = path.parent().and_then(|p| object_identity(p, true)) {
        entry.parent_platform_file_id = to_provider_id(id);
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
            "CMakeFiles" if plain(&parent.join("CMakeCache.txt"), false) => "cmake_build_output",
            _ if plain(&dir.join("CMakeCache.txt"), false)
                && plain(&dir.join("CMakeFiles"), true)
                && !plain(&dir.join("CMakeLists.txt"), false) =>
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
#[derive(Debug)]
struct DirectoryFacts {
    safe: bool,
    identity: Option<String>,
    performance_reason: Option<&'static str>,
    explicit_rule: Option<bool>,
    rule_uncertain: bool,
}
fn parent_identity_matches(expected: &str, actual: Option<&str>) -> bool {
    if expected.starts_with("mac:dev:") {
        return actual == Some(expected);
    }
    if expected.len() == 16 && expected.bytes().all(|c| c.is_ascii_hexdigit()) {
        return actual
            .is_some_and(|id| id.ends_with(&format!(":{}", expected.to_ascii_lowercase())));
    }
    true // Native path-fallback providers still require the no-alias directory proof.
}
pub(crate) fn observe_batch(
    policy: &CoveragePolicy,
    source: &GlobalVolume,
    entries: &[GlobalEntryInput],
) -> Vec<Decision> {
    // Identity-proven directory context lives only in this immutable source /
    // policy batch. Native parent IDs bind the key; source/policy/mount changes
    // cannot reuse previous batches. File link/alias checks remain per candidate.
    let mut contexts = HashMap::<(PathBuf, String), DirectoryFacts>::new();
    let root = Path::new(&source.mount_path);
    let boundaries = protected_roots(root);
    let caches = known_cache_roots(root);
    let rules = if policy.mode == CoverageMode::Custom {
        policy
            .rules
            .iter()
            .filter(|r| r.source_id == source.id && r.stable_volume_id == source.stable_volume_id)
            .map(|r| (r, identity(Path::new(&r.path))))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    entries
        .iter()
        .map(|entry| {
            let path = Path::new(&entry.path);
            if entry.volume_id != source.id
                || !path.is_absolute()
                || !path.starts_with(root)
                || path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Decision::Exclude("outside_authorized_source");
            }
            if boundaries.iter().any(|b| path.starts_with(b)) {
                return Decision::Exclude("protected_boundary");
            }
            if entry.file_attributes
                & (crate::global_index::MACOS_FILE_ATTRIBUTE_CLOUD_NOT_LOCAL
                    | crate::global_index::MACOS_FILE_ATTRIBUTE_CONTENT_UNKNOWN)
                != 0
            {
                return Decision::Undetermined("cloud_availability_unverified");
            }
            let dir = if entry.is_directory {
                path
            } else {
                path.parent().unwrap_or(root)
            };
            let parent_id = if entry.is_directory {
                &entry.platform_file_id
            } else {
                &entry.parent_platform_file_id
            };
            if contexts.len() >= 4096 {
                contexts.clear();
            }
            let facts = contexts
                .entry((dir.to_path_buf(), parent_id.clone()))
                .or_insert_with(|| {
                    let safe = checked_chain(dir, root);
                    let id = if safe { identity(dir) } else { None };
                    let performance_reason = if !safe || policy.mode == CoverageMode::BroadCoverage
                    {
                        None
                    } else if caches.iter().any(|cache| dir.starts_with(cache)) {
                        Some("known_user_cache")
                    } else {
                        performance_root(dir, root)
                    };
                    // Directory identities resolve native case/Unicode spellings without
                    // applying an invented filesystem-wide case-folding rule. The current
                    // saved path must still resolve to its bound object.
                    let anchors = if safe && !rules.is_empty() {
                        dir.ancestors()
                            .take_while(|p| p.starts_with(root))
                            .take(64)
                            .filter_map(|p| identity(p).map(|id| (p.components().count(), id)))
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                    let mut best: Option<(usize, bool, bool)> = None;
                    for (rule, current) in &rules {
                        let literal = path.starts_with(&rule.path);
                        let current_matches = current.as_ref() == Some(&rule.directory_identity);
                        let anchor = if current_matches {
                            anchors
                                .iter()
                                .find(|(_, id)| id == &rule.directory_identity)
                                .map(|(depth, _)| *depth)
                        } else {
                            None
                        };
                        if !literal && anchor.is_none() {
                            continue;
                        }
                        let depth =
                            anchor.unwrap_or_else(|| Path::new(&rule.path).components().count());
                        if best.is_none_or(|(prior, include, _)| {
                            (depth, !rule.include) > (prior, !include)
                        }) {
                            best = Some((depth, rule.include, !current_matches));
                        }
                    }
                    DirectoryFacts {
                        safe,
                        identity: id,
                        performance_reason,
                        explicit_rule: best.map(|(_, include, _)| include),
                        rule_uncertain: best.is_some_and(|(_, _, uncertain)| uncertain),
                    }
                });
            let candidate_safe = entry.is_directory
                || metadata(path).is_ok_and(|m| !fenced(&m) && single_link(path, &m));
            let mut evidence = Evidence {
                directory: entry.is_directory,
                performance_reason: facts.performance_reason,
                explicit_rule: facts.explicit_rule,
                ..Evidence::default()
            };
            if !facts.safe
                || facts.identity.is_none()
                || !candidate_safe
                || !parent_identity_matches(parent_id, facts.identity.as_deref())
            {
                evidence.uncertain = Some("identity_permission_or_alias_unverified");
            }
            if facts.rule_uncertain {
                evidence.uncertain = Some("rule_directory_identity_changed");
            }
            evaluate(policy, &evidence)
        })
        .collect()
}
