//! Root-aware Managed Scan path eligibility.
//!
//! This is an in-memory policy decision. The existing scan root and watcher
//! settings remain the only durable root authorities.

use crate::fs_safety::{capture_source_volume_identity, PhysicalPlatform};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EligibilityPurpose {
    Discovery,
    Watcher,
    Upsert,
    StaleReconciliation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EligibilityReason {
    RootNotEnabled,
    WrongRootSource,
    OutsideSelectedRoot,
    ProtectedSystemLocation,
    SourceIdentityUnavailable,
    SourceVolumeChanged,
    LinkOrReparseBoundary,
    AccessBoundary,
    HiddenPath,
    DependencyTree,
    IdeMetadata,
    GeneratedOutput,
    CacheTree,
    ReservedTrash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EligibilityDecision {
    Include,
    Exclude(EligibilityReason),
    Incomplete(EligibilityReason),
}

#[derive(Debug, Clone)]
struct SourceVolume {
    platform: PhysicalPlatform,
    id: String,
}

/// Root context is assembled only from an active scan run or an enabled,
/// settings-owned watcher root. It is deliberately not stored durably.
#[derive(Debug, Clone)]
pub(crate) struct ManagedScanRootContext {
    pub(crate) id: String,
    pub(crate) path: String,
    pub(crate) enabled: bool,
    pub(crate) source_kind: String,
    source_volume: Result<SourceVolume, EligibilityReason>,
}

impl ManagedScanRootContext {
    pub(crate) fn new(
        id: impl Into<String>,
        path: impl Into<String>,
        enabled: bool,
        source_kind: impl Into<String>,
    ) -> Self {
        let id = id.into();
        let path = normalize_policy_path(&path.into());
        let source = capture_source_volume_identity(Path::new(&path))
            .map(|(platform, id)| SourceVolume { platform, id })
            .map_err(|error| match error {
                crate::fs_safety::PhysicalIdentityError::Missing => {
                    EligibilityReason::AccessBoundary
                }
                crate::fs_safety::PhysicalIdentityError::UnsupportedLink => {
                    EligibilityReason::LinkOrReparseBoundary
                }
                crate::fs_safety::PhysicalIdentityError::UnsupportedType => {
                    EligibilityReason::AccessBoundary
                }
                crate::fs_safety::PhysicalIdentityError::Io(error)
                    if error.kind() == std::io::ErrorKind::PermissionDenied =>
                {
                    EligibilityReason::AccessBoundary
                }
                crate::fs_safety::PhysicalIdentityError::Io(error)
                    if error.kind() == std::io::ErrorKind::Unsupported =>
                {
                    EligibilityReason::SourceIdentityUnavailable
                }
                crate::fs_safety::PhysicalIdentityError::Io(_) => EligibilityReason::AccessBoundary,
            });
        Self {
            id,
            path,
            enabled,
            source_kind: source_kind.into(),
            source_volume: source,
        }
    }
}

pub(crate) fn most_specific_root<'a>(
    roots: &'a [ManagedScanRootContext],
    path: &str,
) -> Option<&'a ManagedScanRootContext> {
    roots
        .iter()
        .filter(|root| {
            root.enabled
                && root.source_kind == "file_library"
                && path_is_within_root(&root.path, path)
        })
        .max_by_key(|root| normalize_policy_path(&root.path).len())
}

/// Evaluate one path in the context of its selected root. `parent_verified`
/// is used only by recursive traversals: the caller has already admitted the
/// parent directory and the no-follow walker cannot cross a link between it
/// and this child. Event paths and historical rows always use full boundary
/// verification. Watcher and stale-reconciliation paths may contain genuinely
/// missing ancestors, but every surviving directory up to the selected root
/// must still pass access, link and source-volume checks.
pub(crate) fn evaluate_managed_path(
    path: &Path,
    root: &ManagedScanRootContext,
    purpose: EligibilityPurpose,
    candidate_is_directory: Option<bool>,
    parent_verified: bool,
) -> EligibilityDecision {
    if !root.enabled {
        return EligibilityDecision::Exclude(EligibilityReason::RootNotEnabled);
    }
    if root.source_kind != "file_library" || root.id.trim().is_empty() {
        return EligibilityDecision::Exclude(EligibilityReason::WrongRootSource);
    }

    let candidate_text = normalize_policy_path(&path.to_string_lossy());
    if !path_is_within_root(&root.path, &candidate_text) {
        return EligibilityDecision::Exclude(EligibilityReason::OutsideSelectedRoot);
    }
    if is_protected_system_path(&candidate_text) {
        return EligibilityDecision::Exclude(EligibilityReason::ProtectedSystemLocation);
    }
    if fs::symlink_metadata(path)
        .ok()
        .is_some_and(|metadata| metadata_is_link_or_reparse(&metadata))
    {
        return EligibilityDecision::Incomplete(EligibilityReason::LinkOrReparseBoundary);
    }

    let expected_volume = match &root.source_volume {
        Ok(volume) => volume,
        Err(reason) => return EligibilityDecision::Incomplete(*reason),
    };
    let is_selected_root = path_identity_key(&candidate_text) == path_identity_key(&root.path);
    if is_selected_root {
        if let Err(reason) = verify_source_object(path, expected_volume) {
            return EligibilityDecision::Incomplete(reason);
        }
    } else if !parent_verified {
        match verify_directory_chain(
            path,
            &root.path,
            expected_volume,
            candidate_is_directory,
            matches!(
                purpose,
                EligibilityPurpose::Watcher | EligibilityPurpose::StaleReconciliation
            ),
        ) {
            Ok(()) => {}
            Err(reason) => return EligibilityDecision::Incomplete(reason),
        }
    } else if candidate_is_directory == Some(true) {
        match verify_single_directory(path, expected_volume) {
            Ok(()) => {}
            Err(reason) => return EligibilityDecision::Incomplete(reason),
        }
    }

    // The chosen root owns its basename. A selected `build` directory is
    // eligible as a root; only context-proven generated descendants are
    // excluded below it. Protected system locations and access boundaries
    // have already taken precedence.
    let relative_components = relative_path_components(&root.path, &candidate_text);
    if relative_components.is_empty() {
        return EligibilityDecision::Include;
    }

    if relative_components
        .iter()
        .any(|component| component.starts_with('.'))
    {
        return EligibilityDecision::Exclude(EligibilityReason::HiddenPath);
    }

    let mut directory = PathBuf::from(&root.path);
    let is_directory = candidate_is_directory.or_else(|| {
        fs::symlink_metadata(path)
            .ok()
            .map(|metadata| metadata.is_dir())
    });
    let component_count = relative_components.len();
    for (index, component) in relative_components.iter().enumerate() {
        directory.push(component);
        let final_component = index + 1 == component_count;
        let component_is_directory = if final_component {
            is_directory.unwrap_or(false)
        } else {
            true
        };
        if !component_is_directory {
            continue;
        }
        if let Some(reason) = contextual_directory_exclusion(&directory, component, &root.path) {
            return EligibilityDecision::Exclude(reason);
        }
    }

    EligibilityDecision::Include
}

pub(crate) fn has_legacy_exclusion_component(root: &str, path: &Path) -> bool {
    let normalized_root = normalize_policy_path(root);
    let candidate_text = normalize_policy_path(&path.to_string_lossy());
    if !path_is_within_root(&normalized_root, &candidate_text) {
        return false;
    }
    let selected_root_name_is_legacy = normalized_root
        .rsplit('/')
        .find(|component| !component.is_empty())
        .is_some_and(is_legacy_exclusion_component);
    selected_root_name_is_legacy
        || relative_path_components(&normalized_root, &candidate_text)
            .iter()
            .any(|component| is_legacy_exclusion_component(component))
}

pub(crate) fn path_is_within_root(root: &str, path: &str) -> bool {
    let root = path_identity_key(root);
    let path = path_identity_key(path);
    let root = trim_policy_root(&root);
    path == root
        || if root.ends_with('/') {
            path.starts_with(&root)
        } else {
            path.strip_prefix(&root)
                .is_some_and(|suffix| suffix.starts_with('/'))
        }
}

pub(crate) fn normalize_policy_path(path: &str) -> String {
    let mut value = path.replace('\\', "/");
    if value
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("//?/UNC/"))
    {
        value = format!("//{}", &value[8..]);
    } else if value
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("//?/"))
    {
        value = value[4..].to_string();
    }

    let (prefix, remainder) = if let Some(network_path) = value.strip_prefix("//") {
        let mut parts = network_path.split('/').filter(|part| !part.is_empty());
        let server = parts.next().unwrap_or_default();
        let share = parts.next().unwrap_or_default();
        let prefix = if server.is_empty() {
            "//".to_string()
        } else if share.is_empty() {
            format!("//{server}")
        } else {
            format!("//{server}/{share}")
        };
        let remainder = value
            .get(prefix.len()..)
            .unwrap_or_default()
            .trim_start_matches('/');
        (prefix, remainder)
    } else if value.as_bytes().get(1) == Some(&b':') {
        let prefix = value[..2].to_string();
        let remainder = value[2..].trim_start_matches('/');
        (prefix, remainder)
    } else if value.starts_with('/') {
        ("/".to_string(), value.trim_start_matches('/'))
    } else {
        (String::new(), value.as_str())
    };

    let mut components = Vec::new();
    for component in remainder.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.last().is_some_and(|part| *part != "..") {
                    components.pop();
                } else if prefix.is_empty() {
                    components.push("..".to_string());
                }
            }
            other => components.push(other.to_string()),
        }
    }
    if prefix == "/" {
        if components.is_empty() {
            return prefix;
        }
        return format!("/{components}", components = components.join("/"));
    }
    let joined = components.join("/");
    if joined.is_empty() {
        if prefix.ends_with(':') {
            format!("{prefix}/")
        } else {
            prefix
        }
    } else if prefix.ends_with(':') || prefix.starts_with("//") {
        format!("{prefix}/{joined}")
    } else if prefix.is_empty() {
        joined
    } else {
        format!("{prefix}/{joined}")
    }
}

fn path_identity_key(path: &str) -> String {
    let normalized = normalize_policy_path(path);
    if cfg!(windows) || is_windows_style_path(&normalized) {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
}

fn trim_policy_root(path: &str) -> String {
    if path == "/" || (path.len() == 3 && path.as_bytes().get(1) == Some(&b':')) {
        path.to_string()
    } else {
        path.trim_end_matches('/').to_string()
    }
}

fn is_windows_style_path(path: &str) -> bool {
    path.starts_with("//") || (path.as_bytes().get(1) == Some(&b':'))
}

fn relative_path_components(root: &str, path: &str) -> Vec<String> {
    let root = trim_policy_root(&path_identity_key(root));
    let path = path_identity_key(path);
    if path == root {
        return Vec::new();
    }
    let Some(relative) = path.strip_prefix(&root) else {
        return Vec::new();
    };
    relative
        .trim_start_matches('/')
        .split('/')
        .filter(|component| !component.is_empty())
        .map(str::to_string)
        .collect()
}

fn verify_directory_chain(
    candidate: &Path,
    root: &str,
    expected_volume: &SourceVolume,
    candidate_is_directory: Option<bool>,
    allow_missing_ancestors: bool,
) -> Result<(), EligibilityReason> {
    let mut current = if candidate_is_directory
        .or_else(|| {
            fs::symlink_metadata(candidate)
                .ok()
                .map(|metadata| metadata.is_dir())
        })
        .unwrap_or(false)
    {
        candidate.to_path_buf()
    } else {
        candidate.parent().unwrap_or(candidate).to_path_buf()
    };
    loop {
        if path_identity_key(&current.to_string_lossy()) == path_identity_key(root) {
            return verify_single_directory(&current, expected_volume);
        }
        match verify_single_directory(&current, expected_volume) {
            Ok(()) => {}
            Err(EligibilityReason::AccessBoundary)
                if allow_missing_ancestors
                    && matches!(
                        fs::symlink_metadata(&current),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound
                    )
                    && path_is_within_root(root, &current.to_string_lossy()) =>
            {
                // Removed watcher paths have no leaf to inspect. Continue to
                // the last existing parent so rename/remove events retain the
                // correct root and volume context.
            }
            Err(reason) => return Err(reason),
        }
        let Some(parent) = current.parent() else {
            return Err(EligibilityReason::OutsideSelectedRoot);
        };
        if parent == current {
            return Err(EligibilityReason::OutsideSelectedRoot);
        }
        current = parent.to_path_buf();
    }
}

fn verify_single_directory(
    path: &Path,
    expected_volume: &SourceVolume,
) -> Result<(), EligibilityReason> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(EligibilityReason::AccessBoundary)
        }
        Err(_) => return Err(EligibilityReason::AccessBoundary),
    };
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
        return Err(EligibilityReason::LinkOrReparseBoundary);
    }
    if !metadata.is_dir() {
        return Err(EligibilityReason::AccessBoundary);
    }
    drop(fs::read_dir(path).map_err(|_| EligibilityReason::AccessBoundary)?);
    let (platform, volume) = capture_source_volume_identity(path).map_err(|error| match error {
        crate::fs_safety::PhysicalIdentityError::UnsupportedLink => {
            EligibilityReason::LinkOrReparseBoundary
        }
        crate::fs_safety::PhysicalIdentityError::Io(error)
            if error.kind() == std::io::ErrorKind::Unsupported =>
        {
            EligibilityReason::SourceIdentityUnavailable
        }
        _ => EligibilityReason::AccessBoundary,
    })?;
    if platform != expected_volume.platform || volume != expected_volume.id {
        return Err(EligibilityReason::SourceVolumeChanged);
    }
    Ok(())
}

fn verify_source_object(
    path: &Path,
    expected_volume: &SourceVolume,
) -> Result<(), EligibilityReason> {
    let metadata = fs::symlink_metadata(path).map_err(|_| EligibilityReason::AccessBoundary)?;
    if metadata_is_link_or_reparse(&metadata) {
        return Err(EligibilityReason::LinkOrReparseBoundary);
    }
    let (platform, volume) = capture_source_volume_identity(path).map_err(|error| match error {
        crate::fs_safety::PhysicalIdentityError::UnsupportedLink => {
            EligibilityReason::LinkOrReparseBoundary
        }
        crate::fs_safety::PhysicalIdentityError::Io(error)
            if error.kind() == std::io::ErrorKind::Unsupported =>
        {
            EligibilityReason::SourceIdentityUnavailable
        }
        _ => EligibilityReason::AccessBoundary,
    })?;
    if platform != expected_volume.platform || volume != expected_volume.id {
        return Err(EligibilityReason::SourceVolumeChanged);
    }
    Ok(())
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

pub(crate) fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink() || is_reparse_point(metadata)
}

pub(crate) fn is_protected_system_path(path: &str) -> bool {
    is_protected_system_location(&normalize_policy_path(path))
}

#[cfg(any(windows, test))]
fn windows_appdata_protection_roots(
    user_profile: Option<&str>,
    appdata: Option<&str>,
    local_appdata: Option<&str>,
) -> Vec<String> {
    let mut roots = Vec::new();
    for value in [appdata, local_appdata].into_iter().flatten() {
        let normalized = normalize_policy_path(value);
        roots.push(normalized.clone());
        if let Some(parent) = standard_appdata_container_parent(&normalized) {
            roots.push(parent);
        }
    }
    if let Some(profile) = user_profile {
        let profile = normalize_policy_path(profile);
        roots.push(format!("{}/AppData", profile.trim_end_matches('/')));
    }
    roots
}

#[cfg(any(windows, test))]
fn standard_appdata_container_parent(path: &str) -> Option<String> {
    let normalized = normalize_policy_path(path);
    let (parent, leaf) = normalized.rsplit_once('/')?;
    if !["local", "locallow", "roaming"]
        .iter()
        .any(|name| leaf.eq_ignore_ascii_case(name))
    {
        return None;
    }
    parent
        .rsplit('/')
        .next()
        .is_some_and(|component| component.eq_ignore_ascii_case("appdata"))
        .then(|| parent.to_string())
}

fn contextual_directory_exclusion(
    directory: &Path,
    component: &str,
    root: &str,
) -> Option<EligibilityReason> {
    #[cfg(target_os = "macos")]
    if crate::platform::macos::package::is_package(directory) {
        return Some(EligibilityReason::GeneratedOutput);
    }
    let name = component.to_ascii_lowercase();
    let parent = directory.parent().unwrap_or(directory);
    match name.as_str() {
        ".zen-canvas-trash" => Some(EligibilityReason::ReservedTrash),
        "node_modules"
            if has_any_file(
                parent,
                &[
                    "package.json",
                    "package-lock.json",
                    "yarn.lock",
                    "pnpm-lock.yaml",
                ],
            ) || has_any_file(directory, &[".package-lock.json", ".yarn-integrity"]) =>
        {
            Some(EligibilityReason::DependencyTree)
        }
        "vendor"
            if has_any_file(directory, &["autoload.php", "modules.txt"])
                || has_any_file(parent, &["composer.json", "go.mod"]) =>
        {
            Some(EligibilityReason::DependencyTree)
        }
        "pods"
            if has_any_file(parent, &["Podfile", "Podfile.lock"])
                || directory.join("Manifest.lock").is_file() =>
        {
            Some(EligibilityReason::DependencyTree)
        }
        "venv" | ".venv"
            if directory.join("pyvenv.cfg").is_file() || has_any_file(parent, &["pyvenv.cfg"]) =>
        {
            Some(EligibilityReason::DependencyTree)
        }
        "target"
            if has_any_file(parent, &["Cargo.toml"])
                || directory.join(".rustc_info.json").is_file() =>
        {
            Some(EligibilityReason::GeneratedOutput)
        }
        "build"
            if has_any_file(directory, &["CMakeCache.txt", "build.ninja"])
                || directory.join("CMakeFiles").is_dir()
                || (has_any_file(parent, &["CMakeLists.txt"])
                    && has_any_file(directory, &["Makefile"])) =>
        {
            Some(EligibilityReason::GeneratedOutput)
        }
        "dist"
            if has_any_file(
                parent,
                &[
                    "vite.config.js",
                    "vite.config.ts",
                    "webpack.config.js",
                    "rollup.config.js",
                ],
            ) && parent.join("package.json").is_file() =>
        {
            Some(EligibilityReason::GeneratedOutput)
        }
        "coverage"
            if directory.join("lcov.info").is_file()
                || directory.join("lcov-report").is_dir()
                || parent.join(".nyc_output").is_dir() =>
        {
            Some(EligibilityReason::GeneratedOutput)
        }
        "__pycache__" if directory.is_dir() => Some(EligibilityReason::CacheTree),
        ".cache"
            if directory.join("CACHEDIR.TAG").is_file() || is_user_cache_directory(directory) =>
        {
            Some(EligibilityReason::CacheTree)
        }
        ".next"
            if has_any_file(
                parent,
                &["next.config.js", "next.config.mjs", "next.config.ts"],
            ) || directory.join("BUILD_ID").is_file() =>
        {
            Some(EligibilityReason::GeneratedOutput)
        }
        ".nuxt"
            if has_any_file(parent, &["nuxt.config.js", "nuxt.config.ts"])
                || directory.join("builds").is_dir() =>
        {
            Some(EligibilityReason::GeneratedOutput)
        }
        ".parcel-cache"
            if has_any_file(parent, &[".parcelrc"])
                || has_any_file(directory, &["data.mdb", "meta.json"]) =>
        {
            Some(EligibilityReason::CacheTree)
        }
        ".turbo" if parent.join("turbo.json").is_file() || directory.join("cache").is_dir() => {
            Some(EligibilityReason::CacheTree)
        }
        "deriveddata" if is_xcode_derived_data(directory, root) => {
            Some(EligibilityReason::GeneratedOutput)
        }
        ".git" if has_any_file(directory, &["HEAD", "config"]) => {
            Some(EligibilityReason::IdeMetadata)
        }
        ".hg" if directory.join(".hg").exists() || has_any_file(directory, &["hgrc", "store"]) => {
            Some(EligibilityReason::IdeMetadata)
        }
        ".svn" if has_any_file(directory, &["entries", "wc.db"]) => {
            Some(EligibilityReason::IdeMetadata)
        }
        ".idea" if has_any_file(directory, &["workspace.xml", "modules.xml", "misc.xml"]) => {
            Some(EligibilityReason::IdeMetadata)
        }
        ".vscode"
            if has_any_file(
                directory,
                &[
                    "settings.json",
                    "launch.json",
                    "tasks.json",
                    "extensions.json",
                ],
            ) =>
        {
            Some(EligibilityReason::IdeMetadata)
        }
        _ => None,
    }
}

fn has_any_file(directory: &Path, names: &[&str]) -> bool {
    names.iter().any(|name| directory.join(name).is_file())
}

fn is_user_cache_directory(directory: &Path) -> bool {
    env::var_os("XDG_CACHE_HOME")
        .or_else(|| {
            env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache").into_os_string())
        })
        .is_some_and(|cache| {
            path_is_within_root(&cache.to_string_lossy(), &directory.to_string_lossy())
        })
}

fn is_xcode_derived_data(directory: &Path, root: &str) -> bool {
    let home = env::var_os("HOME").map(PathBuf::from);
    home.is_some_and(|home| {
        let derived = home.join("Library/Developer/Xcode/DerivedData");
        path_is_within_root(&derived.to_string_lossy(), &directory.to_string_lossy())
    }) || (path_is_within_root(root, &directory.to_string_lossy())
        && directory.join("Build/Intermediates.noindex").is_dir())
}

fn is_legacy_exclusion_component(component: &str) -> bool {
    let lower = component.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        ".git"
            | ".hg"
            | ".svn"
            | ".idea"
            | ".vscode"
            | ".cache"
            | ".zen-canvas-trash"
            | ".parcel-cache"
            | ".turbo"
            | ".next"
            | ".nuxt"
            | ".venv"
            | "__pycache__"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | "coverage"
            | "vendor"
            | "venv"
            | "pods"
            | "deriveddata"
            | "appdata"
            | "system volume information"
            | "$recycle.bin"
            | "windows"
            | "program files"
            | "program files (x86)"
            | "programdata"
            | "$windows.~bt"
            | "$winreagent"
            | "recovery"
    ) || [".git", ".cache", "__pycache__", "node_modules"]
        .iter()
        .any(|base| {
            lower
                .strip_prefix(base)
                .is_some_and(|suffix| matches!(suffix.as_bytes().first(), Some(b'.' | b'-' | b'_')))
        })
}

fn is_protected_system_location(path: &str) -> bool {
    #[cfg(windows)]
    {
        let mut protected = Vec::new();
        for variable in [
            "SystemRoot",
            "windir",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramData",
        ] {
            if let Some(value) = env::var_os(variable) {
                protected.push(normalize_policy_path(
                    &PathBuf::from(value).to_string_lossy(),
                ));
            }
        }
        if protected.iter().any(|root| path_is_within_root(root, path)) {
            return true;
        }
        let profile = env::var_os("USERPROFILE").map(|value| value.to_string_lossy().into_owned());
        let appdata = env::var_os("APPDATA").map(|value| value.to_string_lossy().into_owned());
        let local_appdata =
            env::var_os("LOCALAPPDATA").map(|value| value.to_string_lossy().into_owned());
        let appdata_roots = windows_appdata_protection_roots(
            profile.as_deref(),
            appdata.as_deref(),
            local_appdata.as_deref(),
        );
        let is_appdata = appdata_roots
            .iter()
            .any(|root| path_is_within_root(root, path));
        let is_user_temp = ["TEMP", "TMP"]
            .iter()
            .filter_map(env::var_os)
            .map(|path| normalize_policy_path(&PathBuf::from(path).to_string_lossy()))
            .any(|root| path_is_within_root(&root, path));
        if is_appdata && !is_user_temp {
            return true;
        }
        if windows_volume_root_component_is_protected(path) {
            return true;
        }
    }
    #[cfg(target_os = "macos")]
    {
        const PROTECTED: &[&str] = &[
            "/system", "/private", "/usr", "/bin", "/sbin", "/etc", "/var", "/dev",
        ];
        let path = path.to_ascii_lowercase();
        if PROTECTED
            .iter()
            .any(|root| path_is_within_root(root, &path))
        {
            return true;
        }
        if macos_volume_root_component_is_protected(&path) {
            return true;
        }
    }
    #[cfg(target_os = "linux")]
    {
        const PROTECTED: &[&str] = &["/proc", "/sys", "/dev", "/run"];
        if PROTECTED.iter().any(|root| path_is_within_root(root, path)) {
            return true;
        }
    }
    let _ = path;
    false
}

#[cfg(windows)]
fn windows_volume_root_component_is_protected(path: &str) -> bool {
    volume_root_component(
        path,
        &[
            "windows",
            "program files",
            "program files (x86)",
            "programdata",
            "system volume information",
            "$recycle.bin",
            "$windows.~bt",
            "$winreagent",
            "recovery",
        ],
    )
}

#[cfg(target_os = "macos")]
fn macos_volume_root_component_is_protected(path: &str) -> bool {
    const NAMES: &[&str] = &[
        ".spotlight-v100",
        ".trashes",
        ".fseventsd",
        ".documentrevisions-v100",
    ];
    if volume_root_component(path, NAMES) {
        return true;
    }

    let components = normalize_policy_path(path)
        .to_ascii_lowercase()
        .split('/')
        .filter(|component| !component.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mounted_volume_metadata = components
        .windows(3)
        .any(|parts| parts[0] == "volumes" && NAMES.contains(&parts[2].as_str()))
        || components.windows(4).any(|parts| {
            parts[0] == "system" && parts[1] == "volumes" && NAMES.contains(&parts[3].as_str())
        });
    mounted_volume_metadata
}

#[cfg(any(windows, target_os = "macos"))]
fn volume_root_component(path: &str, names: &[&str]) -> bool {
    let path = normalize_policy_path(path).to_ascii_lowercase();
    let remainder = if path.as_bytes().get(1) == Some(&b':') {
        path.get(3..).unwrap_or_default()
    } else if let Some(network_path) = path.strip_prefix("//") {
        let mut parts = network_path.split('/');
        let server = parts.next().unwrap_or_default();
        let share = parts.next().unwrap_or_default();
        let prefix_len = 3 + server.len() + share.len();
        path.get(prefix_len..)
            .unwrap_or_default()
            .trim_start_matches('/')
    } else if path.starts_with('/') {
        path.trim_start_matches('/')
    } else {
        return false;
    };
    let first = remainder.split('/').next().unwrap_or_default();
    names.contains(&first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn fixture_root(label: &str) -> PathBuf {
        let root = env::current_dir()
            .expect("workspace directory")
            .join("target")
            .join("issue381-fixtures")
            .join(format!("path-filter-{label}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create path-filter fixture");
        root
    }

    #[test]
    fn windows_and_extended_paths_normalize_by_component_and_source() {
        assert!(path_is_within_root(
            "C:/Fixture/Root",
            "c:\\fixture\\ROOT\\Design\\build\\logo.ai"
        ));
        assert!(!path_is_within_root(
            "C:/Fixture/Root",
            "C:/Fixture/Rooted/file.txt"
        ));
        assert_eq!(
            normalize_policy_path(r"\\?\UNC\server\share\folder\..\file.txt"),
            "//server/share/file.txt"
        );
        assert_eq!(
            normalize_policy_path(r"\\?\C:\Fixture\Root\file.txt"),
            "C:/Fixture/Root/file.txt"
        );
    }

    #[test]
    fn system_lookalike_components_are_not_platform_protected_by_name() {
        assert!(!is_protected_system_location(
            "/Users/zen/Projects/Windows/README.md"
        ));
        assert!(!is_protected_system_location(
            "/Users/zen/Configs/AppData/important.json"
        ));
        assert!(!is_protected_system_location(
            "/Users/zen/Design/build/logo.ai"
        ));
    }

    #[test]
    fn custom_appdata_environment_does_not_protect_sibling_paths() {
        let roots = windows_appdata_protection_roots(
            Some(r"C:\Users\zen"),
            Some(r"D:\a\_temp\qa\background\appdata"),
            Some(r"D:\a\_temp\qa\background\localappdata"),
        );
        assert!(roots.iter().any(|root| path_is_within_root(
            root,
            r"D:\a\_temp\qa\background\appdata\settings.json"
        )));
        assert!(!roots.iter().any(|root| {
            path_is_within_root(root, r"D:\a\_temp\qa\background\profile\resident-fixture")
        }));
        assert!(roots
            .iter()
            .any(|root| path_is_within_root(root, r"C:\Users\zen\AppData\Roaming\settings.json")));
        assert!(roots
            .iter()
            .any(|root| { path_is_within_root(root, r"C:\Users\zen\AppData\Local\cache\item") }));
    }

    #[cfg(windows)]
    #[test]
    fn actual_windows_system_locations_are_protected_but_user_lookalikes_are_not() {
        let system_root = env::var_os("SystemRoot").expect("Windows SystemRoot");
        assert!(is_protected_system_location(
            &PathBuf::from(&system_root).to_string_lossy()
        ));
        for variable in ["ProgramFiles", "ProgramData"] {
            if let Some(path) = env::var_os(variable) {
                assert!(
                    is_protected_system_location(&PathBuf::from(path).to_string_lossy()),
                    "actual Windows {variable} location is protected"
                );
            }
        }
        if let Some(path) = env::var_os("APPDATA") {
            assert!(
                is_protected_system_location(&PathBuf::from(path).to_string_lossy()),
                "the actual Windows AppData location remains protected"
            );
        }
        if let Some(path) = env::var_os("TEMP") {
            assert!(
                !is_protected_system_location(&PathBuf::from(path).to_string_lossy()),
                "the explicit per-user temporary directory remains available for managed scans"
            );
        }
        assert!(is_protected_system_location("C:/System Volume Information"));
        assert!(!is_protected_system_location(
            "C:/Users/zen/Projects/Windows/README.md"
        ));
        assert!(!is_protected_system_location(
            "C:/Users/zen/Configs/AppData/important.json"
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_unicode_user_paths_remain_eligible_and_packages_remain_atomic() {
        assert!(is_protected_system_location("/sYSTEM/Applications"));
        let fixture = fixture_root("macos");
        let root_path = fixture.join("Élodie").join("资料");
        let package = root_path.join("Design.app");
        let ordinary = root_path.join("Projects/Windows");
        fs::create_dir_all(&package).expect("create package fixture");
        fs::create_dir_all(&ordinary).expect("create ordinary fixture");
        let context = ManagedScanRootContext::new(
            "macos-unicode-root",
            root_path.to_string_lossy().into_owned(),
            true,
            "file_library",
        );
        assert!(path_is_within_root(
            &root_path.to_string_lossy(),
            &ordinary.join("README.md").to_string_lossy()
        ));
        assert_eq!(
            evaluate_managed_path(
                &ordinary,
                &context,
                EligibilityPurpose::Discovery,
                Some(true),
                false,
            ),
            EligibilityDecision::Include
        );
        assert_eq!(
            evaluate_managed_path(
                &package,
                &context,
                EligibilityPurpose::Discovery,
                Some(true),
                false,
            ),
            EligibilityDecision::Exclude(EligibilityReason::GeneratedOutput)
        );
        let _ = fs::remove_dir_all(fixture);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_watcher_path_is_an_incomplete_boundary() {
        let fixture = fixture_root("symlink");
        let root_path = fixture.join("root");
        let outside = fixture.join("outside");
        let link = root_path.join("linked");
        fs::create_dir_all(&root_path).expect("create selected root");
        fs::create_dir_all(&outside).expect("create outside directory");
        std::os::unix::fs::symlink(&outside, &link).expect("create directory symlink");
        let context = ManagedScanRootContext::new(
            "symlink-root",
            root_path.to_string_lossy().into_owned(),
            true,
            "file_library",
        );
        assert_eq!(
            evaluate_managed_path(
                &link,
                &context,
                EligibilityPurpose::Watcher,
                Some(true),
                false,
            ),
            EligibilityDecision::Incomplete(EligibilityReason::LinkOrReparseBoundary)
        );
        let _ = fs::remove_dir_all(fixture);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn directory_chain_rejects_a_cross_volume_mount_boundary() {
        let root = Path::new("/dev");
        let mounted_child = Path::new("/dev/shm");
        let (platform, id) =
            capture_source_volume_identity(root).expect("Linux device root source volume identity");
        let (mounted_platform, mounted_id) = capture_source_volume_identity(mounted_child)
            .expect("Linux shared-memory mount source volume identity");
        assert_ne!(
            (platform, id.clone()),
            (mounted_platform, mounted_id),
            "the fixture must cross from /dev to its /dev/shm mount"
        );

        assert_eq!(
            verify_directory_chain(
                mounted_child,
                &root.to_string_lossy(),
                &SourceVolume { platform, id },
                Some(true),
                false,
            ),
            Err(EligibilityReason::SourceVolumeChanged)
        );
    }
}
