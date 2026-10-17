use std::{
    collections::HashSet,
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub fn is_ignored_dir_name(name: &OsStr) -> bool {
    let name = name.to_string_lossy();
    let lower = name.to_ascii_lowercase();
    let lower = lower.as_str();

    skip_dir_names().contains(lower) || has_generated_dir_variant(lower)
}

pub(crate) fn legacy_ignored_dir_names() -> &'static [&'static str] {
    LEGACY_SKIP_DIR_NAMES
}

pub(crate) fn generated_dir_variant_bases() -> &'static [&'static str] {
    GENERATED_DIR_VARIANT_BASES
}

const LEGACY_SKIP_DIR_NAMES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".idea",
    ".vscode",
    ".cache",
    ".zen-canvas-trash",
    ".parcel-cache",
    ".turbo",
    ".next",
    ".nuxt",
    ".venv",
    "__pycache__",
    "node_modules",
    "target",
    "dist",
    "build",
    "coverage",
    "vendor",
    "venv",
    "pods",
    "deriveddata",
    "appdata",
    "system volume information",
    "$recycle.bin",
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "$windows.~bt",
    "$winreagent",
    "recovery",
];

const GENERATED_DIR_VARIANT_BASES: &[&str] = &[".git", ".cache", "__pycache__", "node_modules"];

fn skip_dir_names() -> &'static HashSet<&'static str> {
    static SKIP_DIR_NAMES: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SKIP_DIR_NAMES.get_or_init(|| {
        legacy_ignored_dir_names()
            .iter()
            .copied()
            .filter(|name| !CONTEXTUAL_NAMES.contains(name))
            .collect()
    })
}

fn has_generated_dir_variant(lower: &str) -> bool {
    generated_dir_variant_bases().iter().any(|base| {
        lower
            .strip_prefix(base)
            .is_some_and(|suffix| matches!(suffix.as_bytes().first(), Some(b'.' | b'-' | b'_')))
    })
}

// These names require path authority (OS locations) or project evidence (build).
// Keep the legacy list above only as a conservative historical stale guard: this
// patch must not turn previously unobserved rows into a blanket stale sweep.
const CONTEXTUAL_NAMES: &[&str] = &[
    "build",
    "appdata",
    "windows",
    "program files",
    "program files (x86)",
];

/// One scan/upsert/Watcher context. A selected root overrides generated-build
/// inference, never protected locations or dependency/cache defaults. SourceUID,
/// RootEpoch, permissions and ownership CAS remain with their existing owners.
pub(crate) struct ManagedPathFilter {
    root: Option<String>,
    resolved_root_excluded: bool,
}

impl ManagedPathFilter {
    pub(crate) fn new(root: &Path) -> Self {
        let resolved_root_excluded = root.canonicalize().ok().is_some_and(|resolved| {
            protected_path(&resolved)
                || resolved
                    .components()
                    .any(|part| is_ignored_dir_name(part.as_os_str()))
        });
        Self {
            root: Some(normalize(root)),
            resolved_root_excluded,
        }
    }

    pub(crate) fn unselected(path: &Path) -> Self {
        let mut filter = Self::new(path);
        filter.root = None;
        filter
    }

    /// Direct events/imports do not traverse the directory chain. Check that
    /// chain inside the selected root before applying a file or absence fact.
    /// Ancestors above an explicitly selected root keep existing platform rules
    /// (in particular macOS's trusted /var and /tmp aliases).
    pub(crate) fn direct_path_has_no_links(&self, path: &Path) -> std::io::Result<bool> {
        let Some(root) = self.root.as_ref() else {
            return Ok(true);
        };
        for ancestor in path
            .ancestors()
            .take_while(|ancestor| within(&normalize(ancestor), root))
        {
            match std::fs::symlink_metadata(ancestor) {
                Ok(metadata) if metadata_is_link_or_reparse(&metadata) => return Ok(false),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(true)
    }

    pub(crate) fn excludes(&self, path: &Path, is_dir: bool) -> bool {
        if self.resolved_root_excluded || protected_path(path) {
            return true;
        }
        let normalized = normalize(path);
        let path = Path::new(&normalized);
        if path
            .components()
            .any(|part| is_ignored_dir_name(part.as_os_str()))
        {
            return true;
        }
        let directory = if is_dir { Some(path) } else { path.parent() };
        directory.into_iter().flat_map(Path::ancestors).any(|dir| {
            // Do not infer generated output from a selected root or its parents.
            let dir_text = normalize(dir);
            self.root
                .as_ref()
                .is_none_or(|root| !within(root, &dir_text) && within(&dir_text, root))
                && dir
                    .file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case("build"))
                && generated_build(dir)
        })
    }
}

fn generated_build(dir: &Path) -> bool {
    // Positive build-system evidence, not a basename. An unreadable signature
    // remains excluded; absence alone is not evidence of generated output.
    let signature = |path: PathBuf| path.try_exists().unwrap_or(true);
    ["CMakeCache.txt", "build.ninja"]
        .iter()
        .any(|name| signature(dir.join(name)))
        || dir.parent().is_some_and(|parent| {
            [
                "CMakeLists.txt",
                "package.json",
                "Cargo.toml",
                "pyproject.toml",
                "pom.xml",
                "build.gradle",
                "build.gradle.kts",
            ]
            .iter()
            .any(|name| signature(parent.join(name)))
        })
}

fn normalize(path: &Path) -> String {
    crate::db::normalize_path_text(&path.to_string_lossy())
        .trim_end_matches('/')
        .to_string()
}

fn within(path: &str, root: &str) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        path.eq_ignore_ascii_case(root)
            || path
                .get(..root.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(root))
                && path.as_bytes().get(root.len()) == Some(&b'/')
    } else {
        path == root
            || path
                .strip_prefix(root)
                .is_some_and(|suffix| suffix.starts_with('/'))
    }
}

pub(crate) fn managed_protected_roots() -> &'static [PathBuf] {
    static ROOTS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    ROOTS.get_or_init(|| {
        let mut roots =
            crate::file_ops::general_file_operation_protected_roots_for_os(std::env::consts::OS);
        if let Some(cache) = dirs::cache_dir() {
            roots.push(cache);
        }
        #[cfg(windows)]
        {
            // Reuse OS-resolved user known folders, plus existing platform
            // system-root policy. Never manufacture an AppData identity from
            // an arbitrary component with that name.
            for path in [dirs::data_dir(), dirs::data_local_dir()]
                .into_iter()
                .flatten()
            {
                if let Some(parent) = path.parent().filter(|parent| {
                    parent
                        .file_name()
                        .is_some_and(|name| name.eq_ignore_ascii_case("AppData"))
                }) {
                    roots.push(parent.to_path_buf());
                }
                roots.push(path);
            }
            for key in [
                "SystemRoot",
                "ProgramFiles",
                "ProgramFiles(x86)",
                "ProgramData",
            ] {
                if let Some(path) = std::env::var_os(key) {
                    roots.push(PathBuf::from(path));
                }
            }
        }
        roots
    })
}

fn protected_path(path: &Path) -> bool {
    let normalized = normalize(path);
    // Preserve the existing macOS current-temp exception used by the platform
    // path owner (/var and /tmp may resolve beneath /private).
    #[cfg(target_os = "macos")]
    {
        static TEMP: OnceLock<Option<String>> = OnceLock::new();
        if TEMP
            .get_or_init(|| {
                std::env::temp_dir()
                    .canonicalize()
                    .ok()
                    .map(|temp| normalize(&temp))
            })
            .as_ref()
            .is_some_and(|temp| within(&normalized, temp))
        {
            return false;
        }
    }
    static NORMALIZED_ROOTS: OnceLock<Vec<String>> = OnceLock::new();
    NORMALIZED_ROOTS
        .get_or_init(|| {
            managed_protected_roots()
                .iter()
                .map(|root| normalize(root))
                .collect()
        })
        .iter()
        .any(|root| within(&normalized, root))
}

/// Keep the same no-link/no-junction rule in traversal, direct upsert and Watcher.
pub(crate) fn metadata_is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue375_path_policy_distinguishes_user_names_and_protected_locations() {
        let root = std::env::temp_dir().join("zen-canvas-policy-user");
        let filter = ManagedPathFilter::new(&root);
        for name in [
            "Projects/Windows/README.md",
            "Design/build/logo.ai",
            "Configs/AppData/important.json",
            "Program Files/notes.txt",
            "设计/build/图.ai",
        ] {
            assert!(!filter.excludes(&root.join(name), false), "{name}");
        }
        for name in [
            "node_modules/pkg/a.js",
            ".venv/lib/a.py",
            ".vscode/settings.json",
            ".cache/file",
        ] {
            assert!(filter.excludes(&root.join(name), false), "{name}");
        }
        #[cfg(unix)]
        {
            let fixture = std::env::temp_dir().join(format!(
                "zen-canvas-policy-link-{}",
                crate::ids::new_job_id("fixture")
            ));
            let selected = fixture.join("selected");
            let outside = fixture.join("outside");
            std::fs::create_dir_all(&selected).unwrap();
            std::fs::create_dir_all(&outside).unwrap();
            std::fs::write(outside.join("fact.txt"), b"outside selected root").unwrap();
            std::os::unix::fs::symlink(&outside, selected.join("Windows")).unwrap();
            let filter = ManagedPathFilter::new(&selected);
            assert!(!filter
                .direct_path_has_no_links(&selected.join("Windows/fact.txt"))
                .unwrap());
            assert!(!filter
                .direct_path_has_no_links(&selected.join("Windows/missing.txt"))
                .unwrap());
            std::fs::remove_dir_all(&fixture).unwrap();
        }
        for protected in managed_protected_roots() {
            assert!(
                ManagedPathFilter::new(protected).excludes(&protected.join("file"), false),
                "{}",
                protected.display()
            );
        }
    }
}
