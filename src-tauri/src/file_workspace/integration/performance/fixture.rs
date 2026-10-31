use std::{
    fs::{self, File},
    io::{self, Cursor, Write},
    path::{Path, PathBuf},
};

use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};

/// Stable logical fixture descriptions used by the Phase A Preview suite.
/// Paths remain test-private and are never emitted as performance evidence.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PreviewFixtureSpec {
    pub(crate) id: &'static str,
    pub(crate) file_name: &'static str,
    pub(crate) provider_id: &'static str,
    pub(crate) representation_family: &'static str,
    pub(crate) fixture_class: &'static str,
    pub(crate) is_directory: bool,
}

pub(crate) const PREVIEW_FIXTURE_SPECS: &[PreviewFixtureSpec] = &[
    PreviewFixtureSpec {
        id: "text-normal",
        file_name: "preview-text.txt",
        provider_id: "builtin.text",
        representation_family: "text",
        fixture_class: "normal",
        is_directory: false,
    },
    PreviewFixtureSpec {
        id: "source-normal",
        file_name: "preview-source.rs",
        provider_id: "builtin.source-code",
        representation_family: "text",
        fixture_class: "normal",
        is_directory: false,
    },
    PreviewFixtureSpec {
        id: "markdown-normal",
        file_name: "preview-markdown.md",
        provider_id: "builtin.markdown",
        representation_family: "safe_html",
        fixture_class: "normal",
        is_directory: false,
    },
    PreviewFixtureSpec {
        id: "json-normal",
        file_name: "preview-structured.json",
        provider_id: "builtin.structured-json",
        representation_family: "structured_tree",
        fixture_class: "normal",
        is_directory: false,
    },
    PreviewFixtureSpec {
        id: "yaml-normal",
        file_name: "preview-config.yaml",
        provider_id: "builtin.structured-yaml",
        representation_family: "structured_tree",
        fixture_class: "normal",
        is_directory: false,
    },
    PreviewFixtureSpec {
        id: "xml-normal",
        file_name: "preview-markup.xml",
        provider_id: "builtin.structured-xml",
        representation_family: "structured_tree",
        fixture_class: "normal",
        is_directory: false,
    },
    PreviewFixtureSpec {
        id: "csv-normal",
        file_name: "preview-records.csv",
        provider_id: "builtin.table-csv",
        representation_family: "table",
        fixture_class: "normal",
        is_directory: false,
    },
    PreviewFixtureSpec {
        id: "tsv-normal",
        file_name: "preview-records.tsv",
        provider_id: "builtin.table-tsv",
        representation_family: "table",
        fixture_class: "normal",
        is_directory: false,
    },
    PreviewFixtureSpec {
        id: "png-normal",
        file_name: "preview-image.png",
        provider_id: "builtin.image",
        representation_family: "image",
        fixture_class: "normal",
        is_directory: false,
    },
    PreviewFixtureSpec {
        id: "jpeg-normal",
        file_name: "preview-image.jpg",
        provider_id: "builtin.image",
        representation_family: "image",
        fixture_class: "normal",
        is_directory: false,
    },
    PreviewFixtureSpec {
        id: "folder-normal",
        file_name: "preview-folder",
        provider_id: "builtin.folder",
        representation_family: "folder_summary",
        fixture_class: "normal",
        is_directory: true,
    },
    PreviewFixtureSpec {
        id: "archive-normal",
        file_name: "preview-archive.zip",
        provider_id: "builtin.archive-zip",
        representation_family: "archive_tree",
        fixture_class: "normal",
        is_directory: false,
    },
    PreviewFixtureSpec {
        id: "text-large-bounded",
        file_name: "preview-large.txt",
        provider_id: "builtin.text",
        representation_family: "text",
        fixture_class: "large-bounded",
        is_directory: false,
    },
    PreviewFixtureSpec {
        id: "malformed-json",
        file_name: "preview-malformed.json",
        provider_id: "metadata-fallback",
        representation_family: "metadata",
        fixture_class: "corrupt-malformed",
        is_directory: false,
    },
    PreviewFixtureSpec {
        id: "corrupt-image",
        file_name: "preview-corrupt.png",
        provider_id: "metadata-fallback",
        representation_family: "metadata",
        fixture_class: "corrupt-malformed",
        is_directory: false,
    },
    PreviewFixtureSpec {
        id: "unavailable-source",
        file_name: "preview-unavailable.txt",
        provider_id: "terminal-source",
        representation_family: "metadata",
        fixture_class: "permission-unavailable",
        is_directory: false,
    },
    PreviewFixtureSpec {
        id: "cancel-during-load",
        file_name: "preview-cancel.txt",
        provider_id: "builtin.text",
        representation_family: "text",
        fixture_class: "cancel",
        is_directory: false,
    },
];

pub(crate) struct WorkspaceFixture {
    root: PathBuf,
    state_root: PathBuf,
    cleanup_root: PathBuf,
}

impl WorkspaceFixture {
    pub(crate) fn smoke() -> Self {
        Self::new("smoke", 1, 1)
    }

    pub(crate) fn large(label: &str, file_count: usize, directory_count: usize) -> Self {
        Self::new(label, file_count, directory_count)
    }

    pub(crate) fn split(
        label: &str,
        root_count: usize,
        files_per_root: usize,
        directories_per_root: usize,
    ) -> Self {
        let cleanup_root = performance_root();
        let identity = uuid::Uuid::new_v4();
        let root = cleanup_root.join(format!("{label}-{identity}"));
        let state_root = cleanup_root.join(format!("state-{identity}"));
        let result = (|| -> io::Result<()> {
            fs::create_dir_all(&root)?;
            for index in 0..root_count {
                Self::try_create(
                    &root.join(format!("scan-root-{index:03}")),
                    files_per_root,
                    directories_per_root,
                )?;
            }
            fs::create_dir_all(&state_root)
        })();
        if let Err(error) = result {
            remove_task_path(&root);
            remove_task_path(&state_root);
            panic!("create split workspace performance fixture {label}: {error}");
        }
        Self {
            root,
            state_root,
            cleanup_root,
        }
    }

    pub(crate) fn preview(label: &str, rapid_switch_entries: usize) -> Self {
        let cleanup_root = performance_root();
        let identity = uuid::Uuid::new_v4();
        let root = cleanup_root.join(format!("{label}-{identity}"));
        let state_root = cleanup_root.join(format!("state-{identity}"));
        let result = (|| -> io::Result<()> {
            fs::create_dir_all(&root)?;
            Self::try_create_preview(&root, rapid_switch_entries)?;
            fs::create_dir_all(&state_root)
        })();
        if let Err(error) = result {
            remove_task_path(&root);
            remove_task_path(&state_root);
            panic!("create Preview performance fixture {label}: {error}");
        }
        Self {
            root,
            state_root,
            cleanup_root,
        }
    }

    pub(crate) fn preview_scale(
        label: &str,
        rapid_switch_entries: usize,
        folder_entry_count: usize,
        archive_entry_count: usize,
    ) -> Self {
        let cleanup_root = performance_root();
        let identity = uuid::Uuid::new_v4();
        let root = cleanup_root.join(format!("{label}-{identity}"));
        let state_root = cleanup_root.join(format!("state-{identity}"));
        let result = (|| -> io::Result<()> {
            fs::create_dir_all(&root)?;
            Self::try_create_preview(&root, rapid_switch_entries)?;
            Self::try_create_folder(&root.join("preview-folder-scale"), folder_entry_count)?;
            Self::try_create_zip(&root.join("preview-archive-scale.zip"), archive_entry_count)?;
            let archive = fs::read(root.join("preview-archive-scale.zip"))?;
            let truncated_len = archive.len().saturating_sub(8).max(1);
            fs::write(
                root.join("preview-archive-truncated.zip"),
                &archive[..truncated_len],
            )?;
            fs::create_dir_all(&state_root)
        })();
        if let Err(error) = result {
            remove_task_path(&root);
            remove_task_path(&state_root);
            panic!("create Preview scale performance fixture {label}: {error}");
        }
        Self {
            root,
            state_root,
            cleanup_root,
        }
    }

    fn new(label: &str, file_count: usize, directory_count: usize) -> Self {
        let cleanup_root = performance_root();
        let identity = uuid::Uuid::new_v4();
        let root = cleanup_root.join(format!("{label}-{identity}"));
        let state_root = cleanup_root.join(format!("state-{identity}"));
        if let Err(error) = Self::try_create(&root, file_count, directory_count) {
            remove_task_path(&root);
            panic!("create workspace performance fixture {label}: {error}");
        }
        if let Err(error) = fs::create_dir_all(&state_root) {
            remove_task_path(&root);
            remove_task_path(&state_root);
            panic!("create workspace performance state root {label}: {error}");
        }
        Self {
            root,
            state_root,
            cleanup_root,
        }
    }

    fn try_create(root: &Path, file_count: usize, directory_count: usize) -> io::Result<()> {
        fs::create_dir_all(root)?;
        for index in 0..file_count {
            File::create(root.join(format!("file-{index:06}.bin")))?;
        }
        for index in 0..directory_count {
            fs::create_dir(root.join(format!("directory-{index:06}")))?;
        }
        Ok(())
    }

    fn try_create_preview(root: &Path, rapid_switch_entries: usize) -> io::Result<()> {
        fs::write(
            root.join("preview-text.txt"),
            b"Fylura Preview Phase A text fixture\n",
        )?;
        fs::write(
            root.join("preview-source.rs"),
            b"fn preview_phase_a() -> &'static str { \"source\" }\n",
        )?;
        fs::write(
            root.join("preview-markdown.md"),
            b"# Preview Phase A\n\nUseful **Markdown** representation.\n",
        )?;
        fs::write(
            root.join("preview-structured.json"),
            br#"{"name":"Fylura","phase":"A","bounded":true}"#,
        )?;
        fs::write(
            root.join("preview-config.yaml"),
            b"name: Fylura\nphase: A\nbounded: true\n",
        )?;
        fs::write(
            root.join("preview-markup.xml"),
            b"<preview><name>Fylura</name><phase>A</phase></preview>",
        )?;
        fs::write(root.join("preview-records.csv"), b"name,phase\nFylura,A\n")?;
        fs::write(
            root.join("preview-records.tsv"),
            b"name\tphase\nFylura\tA\n",
        )?;
        write_fixture_image(&root.join("preview-image.png"), ImageFormat::Png)?;
        write_fixture_image(&root.join("preview-image.jpg"), ImageFormat::Jpeg)?;
        Self::try_create_folder(&root.join("preview-folder"), 32)?;
        Self::try_create_zip(&root.join("preview-archive.zip"), 32)?;
        fs::write(root.join("preview-corrupt.zip"), b"not-a-valid-zip")?;
        fs::write(root.join("preview-large.txt"), vec![b'x'; 768 * 1024])?;
        fs::write(root.join("preview-malformed.json"), b"{ malformed")?;
        fs::write(root.join("preview-corrupt.png"), b"not-an-image")?;
        fs::write(
            root.join("preview-unavailable.txt"),
            b"source becomes unavailable in the dedicated fixture scenario",
        )?;
        fs::write(root.join("preview-cancel.txt"), vec![b'c'; 768 * 1024])?;
        for index in 0..rapid_switch_entries {
            fs::write(
                root.join(format!("rapid-{index:03}.txt")),
                format!("rapid preview source {index}\n"),
            )?;
        }
        Ok(())
    }

    fn try_create_folder(path: &Path, entry_count: usize) -> io::Result<()> {
        fs::create_dir_all(path)?;
        for index in 0..entry_count {
            File::create(path.join(format!("child-{index:06}.txt")))?;
        }
        Ok(())
    }

    fn try_create_zip(path: &Path, entry_count: usize) -> io::Result<()> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for index in 0..entry_count {
            writer
                .start_file(format!("folder/entry-{index:06}.txt"), options)
                .map_err(io::Error::other)?;
            writer.write_all(b"").map_err(io::Error::other)?;
        }
        let bytes = writer.finish().map_err(io::Error::other)?.into_inner();
        fs::write(path, bytes)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.root
    }

    pub(crate) fn state_path(&self) -> &Path {
        &self.state_root
    }

    pub(crate) fn child_path(&self, index: usize) -> PathBuf {
        self.root.join(format!("scan-root-{index:03}"))
    }
}

fn write_fixture_image(path: &Path, format: ImageFormat) -> io::Result<()> {
    let image = DynamicImage::ImageRgba8(ImageBuffer::from_fn(16, 12, |x, y| {
        Rgba([x as u8, y as u8, 127, 255])
    }));
    let mut encoded = Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, format)
        .map_err(io::Error::other)?;
    fs::write(path, encoded.into_inner())
}

impl Drop for WorkspaceFixture {
    fn drop(&mut self) {
        remove_task_path(&self.root);
        remove_task_path(&self.state_root);
        remove_if_empty(&self.cleanup_root);
        if let Some(parent) = self.cleanup_root.parent() {
            remove_if_empty(parent);
        }
    }
}

fn performance_root() -> PathBuf {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .to_path_buf();
    let task_root = repository_root.join(".tmp-performance-fixtures");
    let shared_cache_root = task_root.join("cache");
    let configured = std::env::var_os("ZC_PERF_WORKSPACE_FIXTURE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| task_root.join("workspace-foundation"));
    let configured = if configured.is_absolute() {
        configured
    } else {
        repository_root.join(configured)
    };
    let normalized = lexical_normalize(&configured);
    let normalized_task_root = lexical_normalize(&task_root);
    let normalized_cache_root = lexical_normalize(&shared_cache_root);
    assert!(
        normalized.starts_with(&normalized_task_root)
            && normalized != normalized_task_root
            && !normalized.starts_with(&normalized_cache_root),
        "ZC_PERF_WORKSPACE_FIXTURE_ROOT must be a task-owned repository-local root under {} and outside the shared fixture cache; got {}",
        normalized_task_root.display(),
        normalized.display()
    );
    let canonical_repository_root = fs::canonicalize(&repository_root)
        .expect("repository root must be canonicalizable for performance fixtures");
    fs::create_dir_all(&normalized_task_root)
        .expect("create repository-local performance fixture parent");
    let canonical_task_root = fs::canonicalize(&normalized_task_root)
        .expect("performance fixture parent must be canonicalizable");
    assert!(
        canonical_task_root.starts_with(&canonical_repository_root),
        "repository-local performance fixture parent must not resolve outside the repository; got {}",
        canonical_task_root.display()
    );
    let mut existing_ancestor = normalized.clone();
    while !existing_ancestor.exists() {
        existing_ancestor = existing_ancestor
            .parent()
            .expect("performance fixture root must have an existing ancestor")
            .to_path_buf();
    }
    let canonical_ancestor = fs::canonicalize(&existing_ancestor)
        .expect("performance fixture root ancestor must be canonicalizable");
    let canonical_cache_root = fs::canonicalize(&shared_cache_root).ok();
    assert!(
        canonical_ancestor.starts_with(&canonical_task_root)
            && !canonical_cache_root
                .as_ref()
                .is_some_and(|cache| canonical_ancestor.starts_with(cache)),
        "performance fixture root must resolve to a task-owned repository-local directory outside the shared cache; got {}",
        canonical_ancestor.display()
    );
    normalized
}

fn remove_if_empty(path: &Path) {
    match fs::read_dir(path) {
        Ok(mut entries) => {
            if entries.next().is_none() {
                if let Err(error) = fs::remove_dir(path) {
                    if error.kind() != io::ErrorKind::NotFound {
                        eprintln!(
                            "[zc-perf] cleanup_failed path={} error={error}",
                            path.display()
                        );
                    }
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => eprintln!(
            "[zc-perf] cleanup_inspection_failed path={} error={error}",
            path.display()
        ),
    }
}

fn remove_task_path(path: &Path) {
    if let Err(error) = fs::remove_dir_all(path) {
        if error.kind() != io::ErrorKind::NotFound {
            eprintln!(
                "[zc-perf] cleanup_failed path={} error={error}",
                path.display()
            );
        }
    }
    if path.exists() {
        eprintln!("[zc-perf] cleanup_leftover path={}", path.display());
    }
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}
