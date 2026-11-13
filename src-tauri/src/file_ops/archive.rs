//! Streaming ZIP create/extract helpers. Mutation authority, path validation,
//! cancellation, and journal ownership stay with the Files workspace.

use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use zip::{
    write::{SimpleFileOptions, ZipWriter},
    CompressionMethod, ZipArchive,
};

const MAX_ARCHIVE_ENTRIES: usize = 20_000;
const MAX_ENTRY_DEPTH: usize = 128;
const MAX_ENTRY_NAME_BYTES: usize = 4 * 1024;
const MAX_EXPANDED_BYTES: u64 = 100 * 1024 * 1024 * 1024;
const MAX_COMPRESSION_RATIO: u64 = 1_000;
const IO_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct ZipSource {
    pub path: PathBuf,
    pub identity: super::FileIdentityFingerprint,
    pub directory_tree_fingerprint: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ZipConflictPolicy {
    Skip,
    KeepBoth,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ZipProgress {
    pub phase: String,
    pub processed_bytes: u64,
    pub total_bytes: u64,
    pub processed_items: u64,
    pub total_items: u64,
    pub current_path: String,
}

struct SourceEntry {
    path: PathBuf,
    archive_name: String,
    is_directory: bool,
}

struct ExtractEntry {
    index: usize,
    path: PathBuf,
    is_directory: bool,
    size: u64,
}

pub(crate) fn create_zip(
    sources: &[ZipSource],
    output: &Path,
    compression_level: i32,
    cancel: &AtomicBool,
    mut progress: impl FnMut(ZipProgress),
) -> Result<PathBuf, String> {
    if !cfg!(test) {
        crate::fs_safety::platform_support::ensure_supported_file_mutation()
            .map_err(|error| error.to_string())?;
    }
    if sources.is_empty() || sources.len() > 1_000 {
        return Err("zip_source_selection_invalid".into());
    }
    let output = if cfg!(all(
        test,
        not(any(target_os = "windows", target_os = "macos"))
    )) {
        validate_test_output_path(output)?
    } else {
        super::validation::validate_target_path(output)?
    };
    if !output
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    {
        return Err("zip_output_extension_required".into());
    }
    if path_exists(&output)? {
        return Err("transfer_target_exists".into());
    }
    let parent = output.parent().ok_or("zip_output_parent_invalid")?;
    super::validation::ensure_general_file_operation_allowed(parent)?;

    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut source_roots = HashSet::new();
    let mut total_bytes = 0u64;
    for source in sources {
        verify_zip_source(source)?;
        if source.path == output {
            return Err("zip_output_is_source".into());
        }
        let original_name = source
            .path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("zip_source_name_invalid")?;
        let name = unique_source_root(original_name, &mut source_roots)?;
        let metadata = fs::symlink_metadata(&source.path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("zip_symlink_source_unsupported".into());
        }
        if metadata.is_dir() {
            walk_directory(
                &source.path,
                Path::new(&name),
                0,
                &mut entries,
                &mut total_bytes,
                &mut seen,
                cancel,
            )?;
        } else if metadata.is_file() {
            add_source_file(
                &source.path,
                Path::new(&name),
                &mut entries,
                &mut total_bytes,
                &mut seen,
            )?;
        } else {
            return Err("zip_special_source_unsupported".into());
        }
        if entries.len() > MAX_ARCHIVE_ENTRIES {
            return Err("zip_entry_limit_exceeded".into());
        }
    }

    let temp = temp_path_for(&output)?;
    let result = (|| {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|error| format!("zip_temp_create_failed: {error}"))?;
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(i64::from(compression_level.clamp(0, 9))));
        let mut writer = ZipWriter::new(file);
        // Files are streamed, so the writer cannot know the compressed size up
        // front. ZipWriter upgrades streamed entries when they exceed ZIP32.
        writer = writer.set_auto_large_file();
        let mut buffer = vec![0; IO_BUFFER_BYTES];
        let mut processed_bytes = 0u64;
        for (index, entry) in entries.iter().enumerate() {
            check_cancel(cancel)?;
            let name = &entry.archive_name;
            if entry.is_directory {
                writer
                    .add_directory(name, options)
                    .map_err(|error| format!("zip_create_failed: {error}"))?;
            } else {
                writer
                    .start_file(name, options)
                    .map_err(|error| format!("zip_create_failed: {error}"))?;
                let mut input = File::open(&entry.path)
                    .map_err(|error| format!("zip_source_open_failed: {error}"))?;
                let before = super::file_namespace_fingerprint(&entry.path)?;
                if !same_identity(&before, &super::file_namespace_fingerprint(&entry.path)?) {
                    return Err("transfer_source_changed".into());
                }
                loop {
                    check_cancel(cancel)?;
                    let read = input
                        .read(&mut buffer)
                        .map_err(|error| format!("zip_source_read_failed: {error}"))?;
                    if read == 0 {
                        break;
                    }
                    writer
                        .write_all(&buffer[..read])
                        .map_err(|error| format!("zip_write_failed: {error}"))?;
                    processed_bytes = processed_bytes.saturating_add(read as u64);
                    progress(ZipProgress {
                        phase: "compressing".into(),
                        processed_bytes,
                        total_bytes,
                        processed_items: index as u64,
                        total_items: entries.len() as u64,
                        current_path: entry.path.to_string_lossy().into_owned(),
                    });
                }
                if !same_identity(&before, &super::file_namespace_fingerprint(&entry.path)?) {
                    return Err("transfer_source_changed".into());
                }
            }
            progress(ZipProgress {
                phase: "compressing".into(),
                processed_bytes,
                total_bytes,
                processed_items: index as u64 + 1,
                total_items: entries.len() as u64,
                current_path: entry.path.to_string_lossy().into_owned(),
            });
        }
        check_cancel(cancel)?;
        let file = writer
            .finish()
            .map_err(|error| format!("zip_finalize_failed: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("zip_sync_failed: {error}"))?;
        for source in sources {
            verify_zip_source(source)?;
        }
        publish_temp(&temp, &output, cancel)?;
        Ok(output.clone())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub(crate) fn extract_zip(
    archive_path: &Path,
    expected_identity: &super::FileIdentityFingerprint,
    destination: &Path,
    conflict_policy: ZipConflictPolicy,
    cancel: &AtomicBool,
    mut progress: impl FnMut(ZipProgress),
) -> Result<Vec<PathBuf>, String> {
    if !cfg!(test) {
        crate::fs_safety::platform_support::ensure_supported_file_mutation()
            .map_err(|error| error.to_string())?;
    }
    let archive_path = super::validation::validate_source_path(archive_path)?;
    if !archive_path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    {
        return Err("zip_archive_required".into());
    }
    if !same_identity(
        expected_identity,
        &super::file_namespace_fingerprint(&archive_path)?,
    ) {
        return Err("transfer_source_changed".into());
    }
    let destination = validate_destination_directory(destination)?;
    let file = File::open(&archive_path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(format_zip_read_error)?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err("zip_entry_limit_exceeded".into());
    }

    let mut total_bytes = 0u64;
    let mut plan = Vec::with_capacity(archive.len());
    let mut path_types = HashMap::<PathBuf, bool>::new();
    for index in 0..archive.len() {
        if index % 64 == 0 {
            check_cancel(cancel)?;
        }
        let entry = archive.by_index(index).map_err(format_zip_read_error)?;
        if entry.encrypted() {
            return Err("zip_encrypted_archive_unsupported".into());
        }
        let entry_name = entry.name().map_err(format_zip_read_error)?;
        if entry_name.len() > MAX_ENTRY_NAME_BYTES {
            return Err("zip_entry_name_limit_exceeded".into());
        }
        let path = safe_archive_path(&entry_name, entry.is_dir())?;
        if path_types
            .insert(path.clone(), entry.is_dir())
            .is_some_and(|previous_is_dir| previous_is_dir != entry.is_dir())
        {
            return Err("zip_entry_type_mismatch".into());
        }
        let depth = path.components().count();
        if depth > MAX_ENTRY_DEPTH {
            return Err("zip_entry_depth_limit_exceeded".into());
        }
        if let Some(mode) = entry.unix_mode() {
            let file_type = mode & 0o170000;
            if file_type != 0 && file_type != 0o100000 && file_type != 0o040000 {
                return Err("zip_special_entry_unsupported".into());
            }
            if entry.is_dir() && file_type == 0o100000 {
                return Err("zip_entry_type_mismatch".into());
            }
            if !entry.is_dir() && file_type == 0o040000 {
                return Err("zip_entry_type_mismatch".into());
            }
        }
        let size = entry.size();
        let compressed_size = entry.compressed_size();
        total_bytes = total_bytes
            .checked_add(size)
            .filter(|total| *total <= MAX_EXPANDED_BYTES)
            .ok_or("zip_expanded_size_limit_exceeded")?;
        if size > 0
            && (compressed_size == 0
                || size > compressed_size.saturating_mul(MAX_COMPRESSION_RATIO))
        {
            return Err("zip_compression_ratio_limit_exceeded".into());
        }
        if entry.enclosed_name().is_none() {
            return Err("zip_unsafe_path".into());
        }
        plan.push(ExtractEntry {
            index,
            path,
            is_directory: entry.is_dir(),
            size,
        });
    }
    validate_archive_path_types(&path_types)?;

    let resolved = resolve_extract_conflicts(&destination, plan, conflict_policy)?;
    let total_bytes = resolved
        .iter()
        .filter(|entry| !entry.is_directory)
        .map(|entry| entry.size)
        .sum::<u64>();
    let mut written = Vec::new();
    let mut created_directories = Vec::new();
    let total_files = resolved.iter().filter(|entry| !entry.is_directory).count();
    let mut processed_bytes = 0u64;
    for (position, entry) in resolved.iter().enumerate() {
        if let Err(error) = check_cancel(cancel) {
            return Err(extract_failure(
                &written,
                &created_directories,
                total_files,
                &entry.path,
                error,
            ));
        }
        if entry.is_directory {
            if let Err(error) = create_directory_chain_record(
                &destination.join(&entry.path),
                &mut created_directories,
            ) {
                return Err(extract_failure(
                    &written,
                    &created_directories,
                    total_files,
                    &entry.path,
                    error,
                ));
            }
            progress(ZipProgress {
                phase: "extracting".into(),
                processed_bytes,
                total_bytes,
                processed_items: position as u64 + 1,
                total_items: resolved.len() as u64,
                current_path: entry.path.to_string_lossy().into_owned(),
            });
            continue;
        }
        let target = destination.join(&entry.path);
        let parent = target.parent().ok_or("zip_target_invalid")?;
        if let Err(error) = create_directory_chain_record(parent, &mut created_directories) {
            return Err(extract_failure(
                &written,
                &created_directories,
                total_files,
                &entry.path,
                error,
            ));
        }
        let temp = temp_path_for(&target)?;
        let result = (|| {
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(|error| format!("zip_temp_create_failed: {error}"))?;
            let mut input = archive
                .by_index(entry.index)
                .map_err(format_zip_read_error)?;
            let mut buffer = vec![0; IO_BUFFER_BYTES];
            let mut entry_bytes = 0u64;
            loop {
                check_cancel(cancel)?;
                let read = input
                    .read(&mut buffer)
                    .map_err(|error| format!("zip_entry_read_failed: {error}"))?;
                if read == 0 {
                    break;
                }
                entry_bytes = entry_bytes
                    .checked_add(read as u64)
                    .filter(|bytes| *bytes <= entry.size)
                    .ok_or("zip_entry_size_mismatch")?;
                processed_bytes = processed_bytes.saturating_add(read as u64);
                output
                    .write_all(&buffer[..read])
                    .map_err(|error| format!("zip_extract_write_failed: {error}"))?;
                progress(ZipProgress {
                    phase: "extracting".into(),
                    processed_bytes,
                    total_bytes,
                    processed_items: position as u64,
                    total_items: resolved.len() as u64,
                    current_path: entry.path.to_string_lossy().into_owned(),
                });
            }
            if entry_bytes != entry.size {
                return Err("zip_entry_size_mismatch".into());
            }
            output
                .sync_all()
                .map_err(|error| format!("zip_sync_failed: {error}"))?;
            publish_temp(&temp, &target, cancel)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
            return Err(extract_failure(
                &written,
                &created_directories,
                total_files,
                &entry.path,
                result.expect_err("failure result"),
            ));
        }
        written.push(target);
        progress(ZipProgress {
            phase: "extracting".into(),
            processed_bytes,
            total_bytes,
            processed_items: position as u64 + 1,
            total_items: resolved.len() as u64,
            current_path: entry.path.to_string_lossy().into_owned(),
        });
    }
    if !same_identity(
        expected_identity,
        &super::file_namespace_fingerprint(&archive_path)?,
    ) {
        return Err(extract_failure(
            &written,
            &created_directories,
            total_files,
            &archive_path,
            "transfer_source_changed".into(),
        ));
    }
    Ok(written)
}

fn extract_failure(
    written: &[PathBuf],
    created_directories: &[PathBuf],
    total_files: usize,
    current_path: &Path,
    error: String,
) -> String {
    if written.is_empty() && created_directories.is_empty() {
        error
    } else {
        format!(
            "zip_extract_partial:{}/{} files; {} directories; {}:{}",
            written.len(),
            total_files,
            created_directories.len(),
            current_path.display(),
            error
        )
    }
}

fn verify_zip_source(source: &ZipSource) -> Result<(), String> {
    if !same_identity(
        &source.identity,
        &super::file_namespace_fingerprint(&source.path)?,
    ) {
        return Err("transfer_source_changed".into());
    }
    if let Some(expected_tree) = source.directory_tree_fingerprint.as_deref() {
        if super::manual_transfer::directory_tree_snapshot(&source.path)? != expected_tree {
            return Err("transfer_source_tree_changed".into());
        }
    }
    Ok(())
}

fn same_identity(
    expected: &super::FileIdentityFingerprint,
    actual: &super::FileIdentityFingerprint,
) -> bool {
    expected == actual
}

fn walk_directory(
    directory: &Path,
    relative: &Path,
    depth: usize,
    entries: &mut Vec<SourceEntry>,
    total_bytes: &mut u64,
    seen: &mut HashSet<String>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    check_cancel(cancel)?;
    if depth > MAX_ENTRY_DEPTH {
        return Err("zip_entry_depth_limit_exceeded".into());
    }
    let metadata = fs::symlink_metadata(directory).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("zip_symlink_or_special_source_unsupported".into());
    }
    let name = archive_name(relative, true)?;
    if seen.insert(name.clone()) {
        entries.push(SourceEntry {
            path: directory.to_path_buf(),
            archive_name: name,
            is_directory: true,
        });
    }
    let mut children = fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    children.sort();
    for child in children {
        check_cancel(cancel)?;
        let child_name = child
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("zip_source_name_invalid")?;
        let child_relative = relative.join(child_name);
        let metadata = fs::symlink_metadata(&child).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("zip_symlink_source_unsupported".into());
        }
        if metadata.is_dir() {
            walk_directory(
                &child,
                &child_relative,
                depth + 1,
                entries,
                total_bytes,
                seen,
                cancel,
            )?;
        } else if metadata.is_file() {
            add_source_file(&child, &child_relative, entries, total_bytes, seen)?;
        } else {
            return Err("zip_special_source_unsupported".into());
        }
        if entries.len() > MAX_ARCHIVE_ENTRIES {
            return Err("zip_entry_limit_exceeded".into());
        }
    }
    Ok(())
}

fn add_source_file(
    path: &Path,
    relative: &Path,
    entries: &mut Vec<SourceEntry>,
    total_bytes: &mut u64,
    seen: &mut HashSet<String>,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("zip_special_source_unsupported".into());
    }
    let name = archive_name(relative, false)?;
    if !seen.insert(name.clone()) {
        return Err("zip_duplicate_source_name".into());
    }
    *total_bytes = total_bytes
        .checked_add(metadata.len())
        .ok_or("zip_source_size_overflow")?;
    entries.push(SourceEntry {
        path: path.to_path_buf(),
        archive_name: name,
        is_directory: false,
    });
    Ok(())
}

fn unique_source_root(name: &str, seen: &mut HashSet<String>) -> Result<String, String> {
    if seen.insert(name.to_string()) {
        return Ok(name.to_string());
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|part| part.to_str())
        .unwrap_or(name);
    let extension = path.extension().and_then(|part| part.to_str());
    for index in 2..=10_000 {
        let candidate = match extension {
            Some(extension) => format!("{stem} ({index}).{extension}"),
            None => format!("{stem} ({index})"),
        };
        super::validation::validate_safe_file_name(&candidate)?;
        if seen.insert(candidate.clone()) {
            return Ok(candidate);
        }
    }
    Err("zip_conflict_name_exhausted".into())
}

fn archive_name(path: &Path, is_directory: bool) -> Result<String, String> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                components.push(value.to_str().ok_or("zip_source_name_invalid")?)
            }
            _ => return Err("zip_source_path_invalid".into()),
        }
    }
    if components.is_empty() || components.len() > MAX_ENTRY_DEPTH {
        return Err("zip_source_path_invalid".into());
    }
    for component in &components {
        super::validation::validate_safe_file_name(component)?;
    }
    let name = components.join("/");
    if name.len() > MAX_ENTRY_NAME_BYTES {
        return Err("zip_entry_name_limit_exceeded".into());
    }
    Ok(if is_directory {
        format!("{name}/")
    } else {
        name
    })
}

fn safe_archive_path(name: &str, is_directory: bool) -> Result<PathBuf, String> {
    if name.is_empty()
        || name.len() > MAX_ENTRY_NAME_BYTES
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.starts_with("//")
        || name.contains('\0')
    {
        return Err("zip_unsafe_path".into());
    }
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/')
        || normalized
            .split('/')
            .next()
            .is_some_and(|part| part.contains(':'))
    {
        return Err("zip_unsafe_path".into());
    }
    let without_trailing = if is_directory {
        normalized.strip_suffix('/').unwrap_or(&normalized)
    } else {
        normalized.as_str()
    };
    let components = without_trailing.split('/').collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == "..")
        || components.len() > MAX_ENTRY_DEPTH
    {
        return Err("zip_unsafe_path".into());
    }
    let mut path = PathBuf::new();
    for component in components {
        super::validation::validate_safe_file_name(component).map_err(|_| "zip_unsafe_path")?;
        path.push(component);
    }
    Ok(path)
}

fn validate_archive_path_types(path_types: &HashMap<PathBuf, bool>) -> Result<(), String> {
    for (path, is_directory) in path_types {
        if !is_directory
            && path_types
                .keys()
                .any(|candidate| candidate != path && candidate.starts_with(path))
        {
            return Err("zip_entry_type_mismatch".into());
        }
        for ancestor in path.ancestors().skip(1) {
            if path_types.get(ancestor) == Some(&false) {
                return Err("zip_entry_type_mismatch".into());
            }
        }
    }
    Ok(())
}

fn resolve_extract_conflicts(
    root: &Path,
    entries: Vec<ExtractEntry>,
    policy: ZipConflictPolicy,
) -> Result<Vec<ExtractEntry>, String> {
    let mut resolved = Vec::with_capacity(entries.len());
    let mut directory_map = HashMap::<PathBuf, PathBuf>::new();
    let mut blocked_directories = HashSet::<PathBuf>::new();
    let mut reserved = HashSet::<PathBuf>::new();
    for mut entry in entries {
        let original = entry.path.clone();
        let parts = original
            .components()
            .filter_map(|part| match part {
                Component::Normal(value) => Some(value.to_os_string()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut mapped = PathBuf::new();
        let mut skip = false;
        for (index, part) in parts.iter().enumerate() {
            let original_prefix = parts.iter().take(index + 1).collect::<PathBuf>();
            let last = index + 1 == parts.len();
            let dir_component = !last || entry.is_directory;
            if blocked_directories
                .iter()
                .any(|blocked| original_prefix.starts_with(blocked))
            {
                skip = true;
                break;
            }
            if let Some(existing) = directory_map.get(&original_prefix) {
                mapped = existing.clone();
                continue;
            }
            let mut candidate = part.clone();
            let mut candidate_path = mapped.join(&candidate);
            if dir_component {
                let occupied =
                    path_exists(&root.join(&candidate_path))? || reserved.contains(&candidate_path);
                if occupied {
                    match policy {
                        ZipConflictPolicy::Skip => {
                            blocked_directories.insert(original_prefix);
                            skip = true;
                            break;
                        }
                        ZipConflictPolicy::KeepBoth => {
                            candidate = unique_component(&mapped, part, root, &reserved)?;
                            candidate_path = mapped.join(&candidate);
                        }
                    }
                }
                reserved.insert(candidate_path.clone());
                directory_map.insert(original_prefix, candidate_path.clone());
                mapped = candidate_path;
            } else {
                candidate_path = mapped.join(&candidate);
                let target = root.join(&candidate_path);
                if path_exists(&target)? || reserved.contains(&candidate_path) {
                    match policy {
                        ZipConflictPolicy::Skip => {
                            skip = true;
                            break;
                        }
                        ZipConflictPolicy::KeepBoth => {
                            candidate = unique_component(&mapped, part, root, &reserved)?;
                            candidate_path = mapped.join(candidate);
                        }
                    }
                }
                mapped = candidate_path;
                reserved.insert(mapped.clone());
            }
        }
        if skip {
            continue;
        }
        entry.path = mapped;
        if entry.is_directory {
            // Directories which were mapped above are already reserved and
            // represent the same archive directory. Only insert an unmapped
            // explicit directory here when the archive contains an empty one.
            if !reserved.contains(&entry.path) {
                reserved.insert(entry.path.clone());
            }
        }
        resolved.push(entry);
    }
    Ok(resolved)
}

fn unique_component(
    parent: &Path,
    name: &std::ffi::OsStr,
    root: &Path,
    reserved: &HashSet<PathBuf>,
) -> Result<std::ffi::OsString, String> {
    let value = name.to_str().ok_or("zip_target_name_invalid")?;
    let path = Path::new(value);
    let stem = path
        .file_stem()
        .and_then(|part| part.to_str())
        .unwrap_or(value);
    let extension = path.extension().and_then(|part| part.to_str());
    for n in 2..=10_000 {
        let candidate = match extension {
            Some(extension) => format!("{stem} ({n}).{extension}"),
            None => format!("{stem} ({n})"),
        };
        super::validation::validate_safe_file_name(&candidate)?;
        let relative = parent.join(&candidate);
        if !reserved.contains(&relative) && !path_exists(&root.join(&relative))? {
            return Ok(candidate.into());
        }
    }
    Err("zip_conflict_name_exhausted".into())
}

pub(crate) fn validate_destination_directory(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path.to_string_lossy().contains('\0')
        || path.components().any(|part| part == Component::ParentDir)
    {
        return Err("zip_destination_path_invalid".into());
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_dir() {
        return Err("zip_destination_not_directory".into());
    }
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        crate::fs_safety::VerifiedDirectory::open_existing(path)
            .map_err(|error| format!("zip_destination_path_rejected: {error}"))?;
    }
    super::validation::ensure_general_file_operation_allowed(path)?;
    path.canonicalize().map_err(|error| error.to_string())
}

pub(crate) fn same_name_extract_path(archive_path: &Path) -> Result<PathBuf, String> {
    let parent =
        validate_destination_directory(archive_path.parent().ok_or("zip_archive_parent_invalid")?)?;
    let name = archive_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or("zip_archive_name_invalid")?;
    super::validation::validate_safe_file_name(name)?;
    Ok(parent.join(name))
}

pub(crate) fn ensure_extract_destination(destination: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err("zip_extract_destination_conflict".into())
        }
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_directory_chain(destination)?;
            Ok(true)
        }
        Err(error) => Err(error.to_string()),
    }
}

fn create_directory_chain(path: &Path) -> Result<(), String> {
    if cfg!(all(
        not(test),
        any(target_os = "windows", target_os = "macos")
    )) {
        crate::fs_safety::create_directory_chain_no_links(path)
            .map_err(|error| format!("zip_destination_path_rejected: {error}"))
    } else {
        fs::create_dir_all(path).map_err(|error| error.to_string())
    }
}

fn create_directory_chain_record(path: &Path, created: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut missing = Vec::new();
    let mut candidate = Some(path);
    while let Some(current) = candidate {
        match fs::symlink_metadata(current) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(current.to_path_buf());
                candidate = current.parent();
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    if let Err(error) = create_directory_chain(path) {
        for path in missing.iter().rev() {
            if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
                created.push(path.to_path_buf());
            }
        }
        return Err(error);
    }
    missing.reverse();
    created.extend(missing);
    Ok(())
}

fn temp_path_for(target: &Path) -> Result<PathBuf, String> {
    let parent = target.parent().ok_or("zip_target_invalid")?;
    for _ in 0..10 {
        let temp = parent.join(format!(".fylura-{}.zip-tmp", uuid::Uuid::new_v4()));
        if !path_exists(&temp)? {
            return Ok(temp);
        }
    }
    Err("zip_temp_name_exhausted".into())
}

fn validate_test_output_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path.to_string_lossy().contains('\0')
        || path.components().any(|part| part == Component::ParentDir)
    {
        return Err("zip_output_path_invalid".into());
    }
    if path_exists(path)? {
        return Err("transfer_target_exists".into());
    }
    let name = path.file_name().ok_or("zip_output_path_invalid")?;
    let parent = path
        .parent()
        .ok_or("zip_output_path_invalid")?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    Ok(parent.join(name))
}

fn publish_temp(temp: &Path, target: &Path, cancel: &AtomicBool) -> Result<(), String> {
    check_cancel(cancel)?;
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        let identity = super::file_operation_fingerprint(temp, "move")?;
        let expected = crate::fs_safety::ExpectedFileIdentity {
            size: identity.size,
            modified_ns: identity.modified_ns,
            platform_volume_id: identity.platform_volume_id,
            platform_file_id: identity.platform_file_id,
            sample_hash: identity.quick_hash,
            full_hash: identity.full_hash,
        };
        crate::fs_safety::atomic_move::atomic_move_noreplace(
            temp,
            target,
            Some(&expected),
            Some(cancel),
        )
        .map_err(|error| format!("zip_publish_failed: {error}"))?;
    } else {
        fs::hard_link(temp, target).map_err(|error| format!("zip_publish_failed: {error}"))?;
        fs::remove_file(temp).map_err(|error| format!("zip_temp_cleanup_failed: {error}"))?;
    }
    Ok(())
}

fn path_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("transfer_cancelled".into())
    } else {
        Ok(())
    }
}

fn format_zip_read_error(error: zip::result::ZipError) -> String {
    match error {
        zip::result::ZipError::UnsupportedArchive(_) => format!("zip_unsupported_archive: {error}"),
        _ => format!("zip_archive_invalid: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("fylura-zip-{label}-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).expect("create ZIP test directory");
            Self(fs::canonicalize(path).expect("canonicalize ZIP test directory"))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn zip_source(path: &Path) -> ZipSource {
        ZipSource {
            path: path.to_path_buf(),
            identity: super::super::file_namespace_fingerprint(path).expect("source identity"),
            directory_tree_fingerprint: path
                .is_dir()
                .then(|| super::super::manual_transfer::directory_tree_snapshot(path).unwrap()),
        }
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).expect("create fixture ZIP");
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(6));
        for (name, contents) in entries {
            writer
                .start_file(*name, options)
                .expect("start fixture entry");
            writer.write_all(contents).expect("write fixture entry");
        }
        writer.finish().expect("finish fixture ZIP");
    }

    #[test]
    fn creates_and_extracts_unicode_directory_tree_without_loading_files_whole() {
        let temp = TestDir::new("unicode-roundtrip");
        let source_dir = temp.path().join("資料");
        fs::create_dir_all(source_dir.join("内部")).expect("create nested source");
        fs::write(source_dir.join("内部").join("報告.txt"), b"hello from ZIP").unwrap();
        let zip_path = temp.path().join("資料.zip");
        let source = zip_source(&source_dir);
        create_zip(&[source], &zip_path, 6, &AtomicBool::new(false), |_| {}).unwrap();

        let zip = ZipArchive::new(File::open(&zip_path).unwrap()).unwrap();
        assert!(zip
            .file_names()
            .any(|name| name.expect("valid ZIP name") == "資料/内部/報告.txt"));
        drop(zip);

        let destination = temp.path().join("解凍済み");
        fs::create_dir(&destination).unwrap();
        let extracted = extract_zip(
            &zip_path,
            &super::super::file_namespace_fingerprint(&zip_path).unwrap(),
            &destination,
            ZipConflictPolicy::Skip,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(extracted.len(), 1);
        assert_eq!(
            fs::read(destination.join("資料/内部/報告.txt")).unwrap(),
            b"hello from ZIP"
        );
    }

    #[test]
    fn extraction_conflicts_skip_or_keep_both_without_overwriting() {
        let temp = TestDir::new("conflicts");
        let zip_path = temp.path().join("archive.zip");
        write_zip(&zip_path, &[("note.txt", b"archive")]);
        let destination = temp.path().join("destination");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("note.txt"), b"existing").unwrap();
        let identity = super::super::file_namespace_fingerprint(&zip_path).unwrap();

        assert!(extract_zip(
            &zip_path,
            &identity,
            &destination,
            ZipConflictPolicy::Skip,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap()
        .is_empty());
        assert_eq!(fs::read(destination.join("note.txt")).unwrap(), b"existing");

        let kept = extract_zip(
            &zip_path,
            &identity,
            &destination,
            ZipConflictPolicy::KeepBoth,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(kept, vec![destination.join("note (2).txt")]);
        assert_eq!(
            fs::read(destination.join("note (2).txt")).unwrap(),
            b"archive"
        );
    }

    #[test]
    fn rejects_zip_slip_path_forms_and_file_as_parent() {
        for path in [
            "../outside",
            "..\\outside",
            "/absolute",
            "C:/outside",
            "\\\\server\\share",
        ] {
            assert_eq!(
                safe_archive_path(path, false).unwrap_err(),
                "zip_unsafe_path"
            );
        }
        let mut types = HashMap::new();
        types.insert(PathBuf::from("parent"), false);
        types.insert(PathBuf::from("parent/child.txt"), false);
        assert_eq!(
            validate_archive_path_types(&types).unwrap_err(),
            "zip_entry_type_mismatch"
        );

        let temp = TestDir::new("zip-slip");
        let zip_path = temp.path().join("slip.zip");
        write_zip(&zip_path, &[("../escape.txt", b"unsafe")]);
        let destination = temp.path().join("destination");
        fs::create_dir(&destination).unwrap();
        let error = extract_zip(
            &zip_path,
            &super::super::file_namespace_fingerprint(&zip_path).unwrap(),
            &destination,
            ZipConflictPolicy::Skip,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(error, "zip_unsafe_path");
        assert!(!temp.path().join("escape.txt").exists());
    }

    #[test]
    fn rejects_symlink_archive_entries() {
        let temp = TestDir::new("symlink");
        let zip_path = temp.path().join("link.zip");
        let mut writer = ZipWriter::new(File::create(&zip_path).unwrap());
        writer
            .start_file(
                "link",
                SimpleFileOptions::default()
                    .system(zip::System::Unix)
                    .external_attributes(0o120777 << 16),
            )
            .unwrap();
        writer.write_all(b"target").unwrap();
        writer.finish().unwrap();
        let destination = temp.path().join("destination");
        fs::create_dir(&destination).unwrap();
        let error = extract_zip(
            &zip_path,
            &super::super::file_namespace_fingerprint(&zip_path).unwrap(),
            &destination,
            ZipConflictPolicy::Skip,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(error, "zip_special_entry_unsupported");
        assert!(!destination.join("link").exists());
    }

    #[test]
    fn rejects_extreme_expansion_corruption_and_cancellation() {
        let temp = TestDir::new("archive-limits");
        let compressed = temp.path().join("ratio.zip");
        let zeros = vec![0u8; 8 * 1024 * 1024];
        write_zip(&compressed, &[("zeros.bin", &zeros)]);
        let destination = temp.path().join("destination");
        fs::create_dir(&destination).unwrap();
        let ratio_error = extract_zip(
            &compressed,
            &super::super::file_namespace_fingerprint(&compressed).unwrap(),
            &destination,
            ZipConflictPolicy::Skip,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(ratio_error, "zip_compression_ratio_limit_exceeded");

        let corrupt = temp.path().join("corrupt.zip");
        fs::write(&corrupt, b"not a ZIP archive").unwrap();
        assert!(extract_zip(
            &corrupt,
            &super::super::file_namespace_fingerprint(&corrupt).unwrap(),
            &destination,
            ZipConflictPolicy::Skip,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap_err()
        .starts_with("zip_archive_invalid:"));

        let source = temp.path().join("cancel.txt");
        fs::write(&source, b"cancel me").unwrap();
        let output = temp.path().join("cancel.zip");
        let cancel = AtomicBool::new(true);
        assert_eq!(
            create_zip(&[zip_source(&source)], &output, 6, &cancel, |_| {}).unwrap_err(),
            "transfer_cancelled"
        );
        assert!(!output.exists());
    }

    #[test]
    fn extraction_cancellation_after_a_published_item_reports_partial_progress() {
        let temp = TestDir::new("partial-cancel");
        let zip_path = temp.path().join("two-files.zip");
        let first = (0..300_000)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let second = (0..300_000)
            .map(|index| ((index + 31) % 251) as u8)
            .collect::<Vec<_>>();
        write_zip(&zip_path, &[("first.bin", &first), ("second.bin", &second)]);
        let destination = temp.path().join("destination");
        fs::create_dir(&destination).unwrap();
        let cancel = AtomicBool::new(false);
        let error = extract_zip(
            &zip_path,
            &super::super::file_namespace_fingerprint(&zip_path).unwrap(),
            &destination,
            ZipConflictPolicy::Skip,
            &cancel,
            |progress| {
                if progress.current_path == "first.bin" && progress.processed_items == 1 {
                    cancel.store(true, Ordering::Release);
                }
            },
        )
        .unwrap_err();
        assert!(
            error.starts_with("zip_extract_partial:1/2 files;"),
            "{error}"
        );
        assert_eq!(fs::read(destination.join("first.bin")).unwrap(), first);
        assert!(!destination.join("second.bin").exists());
        assert_eq!(
            fs::read_dir(&destination)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().contains(".fylura-"))
                .count(),
            0
        );
    }

    #[test]
    fn zip_creation_refuses_existing_output_without_changing_it() {
        let temp = TestDir::new("no-overwrite");
        let source = temp.path().join("source.txt");
        fs::write(&source, b"source").unwrap();
        let output = temp.path().join("existing.zip");
        fs::write(&output, b"keep this file").unwrap();
        let error = create_zip(
            &[zip_source(&source)],
            &output,
            6,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap_err();
        assert!(
            error == "transfer_target_exists"
                || error.to_ascii_lowercase().contains("already exists"),
            "unexpected existing-target error: {error}"
        );
        assert_eq!(fs::read(output).unwrap(), b"keep this file");
    }

    #[test]
    fn same_name_destination_is_created_without_replacing_an_existing_file() {
        let temp = TestDir::new("same-name-destination");
        let archive = temp.path().join("資料.zip");
        fs::write(&archive, b"fixture").unwrap();
        let destination = same_name_extract_path(&archive).unwrap();
        assert_eq!(destination, temp.path().join("資料"));
        assert!(ensure_extract_destination(&destination).unwrap());
        assert!(!ensure_extract_destination(&destination).unwrap());

        fs::remove_dir(&destination).unwrap();
        fs::write(&destination, b"preserve").unwrap();
        assert_eq!(
            ensure_extract_destination(&destination).unwrap_err(),
            "zip_extract_destination_conflict"
        );
        assert_eq!(fs::read(destination).unwrap(), b"preserve");
    }

    #[test]
    fn compressing_same_named_sources_keeps_both_entries() {
        let temp = TestDir::new("duplicate-roots");
        let left = temp.path().join("left");
        let right = temp.path().join("right");
        fs::create_dir_all(&left).unwrap();
        fs::create_dir_all(&right).unwrap();
        fs::write(left.join("report.txt"), b"left").unwrap();
        fs::write(right.join("report.txt"), b"right").unwrap();
        let output = temp.path().join("reports.zip");
        create_zip(
            &[
                zip_source(&left.join("report.txt")),
                zip_source(&right.join("report.txt")),
            ],
            &output,
            6,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();

        let mut zip = ZipArchive::new(File::open(output).unwrap()).unwrap();
        assert_eq!(zip.len(), 2);
        assert_eq!(zip.by_index(0).unwrap().name().unwrap(), "report.txt");
        assert_eq!(zip.by_index(1).unwrap().name().unwrap(), "report (2).txt");
    }
}
