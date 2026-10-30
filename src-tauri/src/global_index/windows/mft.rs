use super::volumes::{to_wide, volume_device_path};
use crate::global_index::coordinator::{GlobalIndexError, GlobalIndexSink};
use crate::global_index::models::{
    GlobalEntryInput, GlobalSourceDescriptor, PROVIDER_WINDOWS_MFT_USN,
};
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_HANDLE_EOF, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    CREATE_USN_JOURNAL_DATA, FSCTL_CREATE_USN_JOURNAL, FSCTL_ENUM_USN_DATA,
    FSCTL_QUERY_USN_JOURNAL, MFT_ENUM_DATA_V0, USN_JOURNAL_DATA_V0, USN_RECORD_COMMON_HEADER,
    USN_RECORD_V2, USN_RECORD_V3,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

const FILETIME_UNIX_EPOCH: i64 = 116_444_736_000_000_000;
const ENUM_BUFFER_SIZE: usize = 1024 * 1024;
const STAGING_READ_BATCH: i64 = 2048;
const NTFS_ROOT_FILE_RECORD_NUMBER: u64 = 5;
const NTFS_FILE_RECORD_NUMBER_MASK: u64 = 0x0000_ffff_ffff_ffff;
/// `ERROR_JOURNAL_NOT_ACTIVE`: the volume has no active USN change journal.
/// This is distinct from a deleted/invalid journal history (1178/1181): the
/// journal was never created, so a rebuild alone cannot recover it.
const ERROR_JOURNAL_NOT_ACTIVE: u32 = 1179;
/// Bounded default when Zen Canvas has to create the change journal on a
/// volume the user already authorized for indexing. Matches the Windows
/// default order of magnitude and keeps the footprint predictable.
const USN_JOURNAL_MAXIMUM_BYTES: u64 = 32 * 1024 * 1024;
const USN_JOURNAL_ALLOCATION_DELTA: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct MftRecord {
    pub(crate) file_reference: String,
    pub(crate) parent_reference: String,
    pub(crate) name: String,
    pub(crate) timestamp: Option<i64>,
    pub(crate) reason: u32,
    pub(crate) attributes: u32,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct MftJournalState {
    pub(crate) journal_id: u64,
    pub(crate) next_usn: i64,
    pub(crate) baseline: MftBaselineStats,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MftBaselineStats {
    pub(crate) staged_records: u64,
    pub(crate) resolved_entries: u64,
    pub(crate) emitted_entries: u64,
    pub(crate) checkpoint_usn: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MftEntryStreamStats {
    resolved_entries: u64,
    emitted_entries: u64,
}

pub(crate) fn enumerate_volume(
    source: &GlobalSourceDescriptor,
    sink: &mut dyn GlobalIndexSink,
    cancel: &AtomicBool,
) -> Result<MftJournalState, GlobalIndexError> {
    let handle = open_volume(&source.volume.mount_path)?;
    let result = enumerate_with_handle(source, handle, sink, cancel);
    unsafe {
        CloseHandle(handle);
    }
    result
}

fn enumerate_with_handle(
    source: &GlobalSourceDescriptor,
    handle: HANDLE,
    sink: &mut dyn GlobalIndexSink,
    cancel: &AtomicBool,
) -> Result<MftJournalState, GlobalIndexError> {
    let journal = query_journal(handle)?;
    let staging_path = std::env::temp_dir().join(format!(
        "zen-canvas-mft-{}-{}.sqlite",
        source.volume.id,
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let mut staging = open_staging_database(&staging_path)?;
        let staged_records = stage_mft_records(&mut staging, handle, journal.NextUsn, cancel)?;
        let directories = load_staged_directories(&staging)?;
        let (directory_paths, parent_paths) =
            resolve_directory_paths(&source.volume.mount_path, &directories);
        let stream = stream_staged_entries(
            source,
            &staging,
            &directory_paths,
            &parent_paths,
            sink,
            cancel,
        )?;
        let baseline = MftBaselineStats {
            staged_records,
            resolved_entries: stream.resolved_entries,
            emitted_entries: stream.emitted_entries,
            checkpoint_usn: journal.NextUsn,
        };
        validate_initial_baseline(baseline)?;
        sink.checkpoint(
            &source.volume.id,
            Some(&journal.UsnJournalID.to_string()),
            Some(&journal.NextUsn.to_string()),
        )?;
        Ok(MftJournalState {
            journal_id: journal.UsnJournalID,
            next_usn: journal.NextUsn,
            baseline,
        })
    })();
    let _ = std::fs::remove_file(&staging_path);
    result
}

fn open_staging_database(path: &Path) -> Result<Connection, GlobalIndexError> {
    let connection = Connection::open(path)
        .map_err(|error| GlobalIndexError::Provider(format!("mft_staging_open_failed: {error}")))?;
    connection
        .execute_batch(
            r#"
            PRAGMA journal_mode = OFF;
            PRAGMA synchronous = OFF;
            PRAGMA temp_store = MEMORY;
            CREATE TABLE mft_records (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                file_reference TEXT NOT NULL,
                parent_reference TEXT NOT NULL,
                name TEXT NOT NULL,
                timestamp INTEGER,
                reason INTEGER NOT NULL,
                attributes INTEGER NOT NULL
            );
            CREATE INDEX idx_mft_records_reference ON mft_records(file_reference);
            CREATE INDEX idx_mft_records_parent ON mft_records(parent_reference);
            "#,
        )
        .map_err(|error| {
            GlobalIndexError::Provider(format!("mft_staging_schema_failed: {error}"))
        })?;
    Ok(connection)
}

fn stage_mft_records(
    staging: &mut Connection,
    handle: HANDLE,
    high_usn: i64,
    cancel: &AtomicBool,
) -> Result<u64, GlobalIndexError> {
    let mut cursor = 0u64;
    let mut staged_records = 0u64;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(GlobalIndexError::Paused);
        }
        let input = MFT_ENUM_DATA_V0 {
            StartFileReferenceNumber: cursor,
            LowUsn: 0,
            HighUsn: high_usn,
        };
        let input_bytes = unsafe {
            std::slice::from_raw_parts(
                (&input as *const MFT_ENUM_DATA_V0).cast::<u8>(),
                std::mem::size_of::<MFT_ENUM_DATA_V0>(),
            )
        };
        let mut output = vec![0u8; ENUM_BUFFER_SIZE];
        let bytes = match unsafe {
            device_io_control_bytes(handle, FSCTL_ENUM_USN_DATA, input_bytes, &mut output)
        } {
            Ok(bytes) => bytes,
            Err(error) if is_win32_error(&error, ERROR_HANDLE_EOF) => break,
            Err(error) => return Err(error),
        };
        let (next_cursor, records) = parse_mft_page(&output[..bytes])?;
        if next_cursor <= cursor {
            return Err(mft_integrity_error(
                "MFT continuation cursor did not advance",
            ));
        }
        let transaction = staging.transaction().map_err(|error| {
            GlobalIndexError::Provider(format!("mft_staging_transaction_failed: {error}"))
        })?;
        staged_records = staged_records.saturating_add(records.len() as u64);
        {
            let mut insert = transaction
                .prepare_cached(
                    "INSERT INTO mft_records (file_reference, parent_reference, name, timestamp, reason, attributes) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .map_err(|error| {
                    GlobalIndexError::Provider(format!("mft_staging_prepare_failed: {error}"))
                })?;
            for record in records {
                insert
                    .execute(params![
                        record.file_reference,
                        record.parent_reference,
                        record.name,
                        record.timestamp,
                        i64::from(record.reason),
                        i64::from(record.attributes)
                    ])
                    .map_err(|error| {
                        GlobalIndexError::Provider(format!("mft_staging_write_failed: {error}"))
                    })?;
            }
        }
        transaction.commit().map_err(|error| {
            GlobalIndexError::Provider(format!("mft_staging_commit_failed: {error}"))
        })?;
        cursor = next_cursor;
    }
    Ok(staged_records)
}

fn load_staged_directories(staging: &Connection) -> Result<Vec<MftRecord>, GlobalIndexError> {
    let mut statement = staging
        .prepare(
            "SELECT file_reference, parent_reference, name, timestamp, reason, attributes FROM mft_records WHERE (attributes & ?1) != 0",
        )
        .map_err(|error| {
            GlobalIndexError::Provider(format!("mft_staging_directory_prepare_failed: {error}"))
        })?;
    let rows = statement
        .query_map(
            params![i64::from(FILE_ATTRIBUTE_DIRECTORY)],
            staged_record_from_row,
        )
        .map_err(|error| {
            GlobalIndexError::Provider(format!("mft_staging_directory_query_failed: {error}"))
        })?;
    let directories = rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
        GlobalIndexError::Provider(format!("mft_staging_directory_decode_failed: {error}"))
    })?;
    Ok(directories)
}

fn resolve_directory_paths(
    root: &str,
    directories: &[MftRecord],
) -> (HashMap<String, String>, HashMap<String, String>) {
    let mut by_key = HashMap::new();
    let mut by_reference = HashMap::new();
    let mut unresolved = directories.iter().collect::<Vec<_>>();
    for directory in directories {
        let file_reference_is_root = is_ntfs_root_reference(&directory.file_reference);
        let parent_reference_is_root = is_ntfs_root_reference(&directory.parent_reference);
        if file_reference_is_root {
            by_key.insert(record_key(directory), root.to_string());
            by_reference
                .entry(directory.file_reference.clone())
                .or_insert_with(|| root.to_string());
        }
        if parent_reference_is_root {
            // FSCTL_ENUM_USN_DATA may include child records without yielding a
            // conventional self-parenting root record. NTFS reserves MFT
            // record 5 for the volume root, so its parent reference is enough
            // to seed path resolution without inventing a root record.
            by_reference
                .entry(directory.parent_reference.clone())
                .or_insert_with(|| root.to_string());
        }
        if directory.name == "." || directory.parent_reference == directory.file_reference {
            by_key.insert(record_key(directory), root.to_string());
            by_reference
                .entry(directory.file_reference.clone())
                .or_insert_with(|| root.to_string());
        }
    }
    loop {
        let mut progressed = false;
        unresolved.retain(|directory| {
            let key = record_key(directory);
            if by_key.contains_key(&key) {
                return false;
            }
            let Some(parent) = by_reference.get(&directory.parent_reference) else {
                return true;
            };
            let path = join_windows_path(parent, &directory.name);
            by_key.insert(key, path.clone());
            by_reference
                .entry(directory.file_reference.clone())
                .and_modify(|current| {
                    if path.len() < current.len() {
                        *current = path.clone();
                    }
                })
                .or_insert(path);
            progressed = true;
            false
        });
        if !progressed || unresolved.is_empty() {
            break;
        }
    }
    (by_key, by_reference)
}

fn is_ntfs_root_reference(reference: &str) -> bool {
    let bytes = reference.as_bytes();
    if bytes.len() == 16 {
        return u64::from_str_radix(reference, 16)
            .ok()
            .is_some_and(|value| {
                value & NTFS_FILE_RECORD_NUMBER_MASK == NTFS_ROOT_FILE_RECORD_NUMBER
            });
    }
    if bytes.len() != 32 {
        return false;
    }

    // USN_RECORD_V3 carries NTFS's 48-bit MFT index in the low bits of its
    // little-endian 128-bit file ID; the upper 64 bits are unused on NTFS.
    let mut low_file_id = [0u8; 8];
    for (index, byte) in low_file_id.iter_mut().enumerate() {
        let offset = index * 2;
        let Ok(pair) = std::str::from_utf8(&bytes[offset..offset + 2]) else {
            return false;
        };
        let Ok(parsed) = u8::from_str_radix(pair, 16) else {
            return false;
        };
        *byte = parsed;
    }
    for pair in bytes[16..].chunks_exact(2) {
        if pair != b"00" {
            return false;
        }
    }
    u64::from_le_bytes(low_file_id) & NTFS_FILE_RECORD_NUMBER_MASK == NTFS_ROOT_FILE_RECORD_NUMBER
}

fn stream_staged_entries(
    source: &GlobalSourceDescriptor,
    staging: &Connection,
    directory_paths: &HashMap<String, String>,
    parent_paths: &HashMap<String, String>,
    sink: &mut dyn GlobalIndexSink,
    cancel: &AtomicBool,
) -> Result<MftEntryStreamStats, GlobalIndexError> {
    let mut last_sequence = 0i64;
    let mut batch = Vec::with_capacity(512);
    let mut stats = MftEntryStreamStats::default();
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(GlobalIndexError::Paused);
        }
        let rows = {
            let mut statement = staging
                .prepare(
                    "SELECT sequence, file_reference, parent_reference, name, timestamp, reason, attributes FROM mft_records WHERE sequence > ?1 ORDER BY sequence LIMIT ?2",
                )
                .map_err(|error| {
                    GlobalIndexError::Provider(format!("mft_staging_read_prepare_failed: {error}"))
                })?;
            let mapped = statement
                .query_map(params![last_sequence, STAGING_READ_BATCH], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        staged_record_from_row_offset(row, 1)?,
                    ))
                })
                .map_err(|error| {
                    GlobalIndexError::Provider(format!("mft_staging_read_failed: {error}"))
                })?;
            mapped.collect::<Result<Vec<_>, _>>().map_err(|error| {
                GlobalIndexError::Provider(format!("mft_staging_decode_failed: {error}"))
            })?
        };
        if rows.is_empty() {
            break;
        }
        for (sequence, record) in rows {
            last_sequence = sequence;
            if record.name.is_empty() {
                continue;
            }
            let is_directory = record.attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
            let path = if is_directory {
                directory_paths.get(&record_key(&record)).cloned()
            } else {
                parent_paths
                    .get(&record.parent_reference)
                    .map(|parent| join_windows_path(parent, &record.name))
            };
            let Some(path) = path else {
                // Fail closed: do not invent a root-level path for an orphaned
                // MFT record. A later USN update or rebuild can reconcile it.
                continue;
            };
            stats.resolved_entries = stats.resolved_entries.saturating_add(1);
            let extension = if is_directory {
                String::new()
            } else {
                PathBuf::from(&record.name)
                    .extension()
                    .map(|value| value.to_string_lossy().to_lowercase())
                    .unwrap_or_default()
            };
            let metadata = (!is_directory)
                .then(|| std::fs::symlink_metadata(&path).ok())
                .flatten();
            batch.push(GlobalEntryInput {
                volume_id: source.volume.id.clone(),
                platform_file_id: record.file_reference,
                parent_platform_file_id: record.parent_reference,
                name: record.name,
                path,
                extension,
                is_directory,
                size: metadata
                    .as_ref()
                    .map(|value| value.len() as i64)
                    .unwrap_or(0),
                created_at_fs: record.timestamp,
                modified_at_fs: metadata
                    .as_ref()
                    .and_then(|value| value.modified().ok())
                    .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|value| value.as_secs() as i64)
                    .or(record.timestamp),
                file_attributes: i64::from(record.attributes),
                is_hidden: record.attributes & FILE_ATTRIBUTE_HIDDEN != 0,
                is_system: record.attributes & FILE_ATTRIBUTE_SYSTEM != 0,
                source_provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
                last_seen_at: crate::global_index::models::unix_now(),
            });
            if batch.len() >= 512 {
                stats.emitted_entries = stats
                    .emitted_entries
                    .saturating_add(sink.write_batch(&batch)? as u64);
                batch.clear();
            }
        }
    }
    if !batch.is_empty() {
        stats.emitted_entries = stats
            .emitted_entries
            .saturating_add(sink.write_batch(&batch)? as u64);
    }
    Ok(stats)
}

fn validate_initial_baseline(stats: MftBaselineStats) -> Result<(), GlobalIndexError> {
    if stats.emitted_entries == 0 {
        return Err(GlobalIndexError::RebuildRequired(format!(
            "windows_mft_baseline_empty: staged_records={} resolved_entries={} emitted_entries={} checkpoint_usn={}",
            stats.staged_records,
            stats.resolved_entries,
            stats.emitted_entries,
            stats.checkpoint_usn
        )));
    }
    Ok(())
}

fn staged_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MftRecord> {
    staged_record_from_row_offset(row, 0)
}

fn staged_record_from_row_offset(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<MftRecord> {
    Ok(MftRecord {
        file_reference: row.get(offset)?,
        parent_reference: row.get(offset + 1)?,
        name: row.get(offset + 2)?,
        timestamp: row.get(offset + 3)?,
        reason: row.get::<_, i64>(offset + 4)? as u32,
        attributes: row.get::<_, i64>(offset + 5)? as u32,
    })
}

pub(crate) fn parse_mft_page(buffer: &[u8]) -> Result<(u64, Vec<MftRecord>), GlobalIndexError> {
    if buffer.len() < 8 {
        return Err(mft_integrity_error(
            "MFT page is shorter than the continuation cursor",
        ));
    }
    let next_start = u64::from_ne_bytes(
        buffer[..8]
            .try_into()
            .map_err(|_| mft_integrity_error("invalid MFT cursor"))?,
    );
    let mut offset = 8usize;
    let mut records = Vec::new();
    let header_size = std::mem::size_of::<USN_RECORD_COMMON_HEADER>();
    while offset < buffer.len() {
        if buffer.len() - offset < header_size {
            if buffer[offset..].iter().any(|byte| *byte != 0) {
                return Err(mft_integrity_error(
                    "MFT page has a non-zero truncated record header",
                ));
            }
            break;
        }
        let header = unsafe {
            ptr::read_unaligned(buffer.as_ptr().add(offset) as *const USN_RECORD_COMMON_HEADER)
        };
        let record_length = header.RecordLength as usize;
        if record_length == 0 {
            if buffer[offset..].iter().any(|byte| *byte != 0) {
                return Err(mft_integrity_error(
                    "MFT page has data after a zero-length record",
                ));
            }
            break;
        }
        let end = offset
            .checked_add(record_length)
            .ok_or_else(|| mft_integrity_error("MFT record length overflow"))?;
        if end > buffer.len() || record_length < 8 {
            return Err(mft_integrity_error("MFT record extends past page boundary"));
        }
        let record = match header.MajorVersion {
            2 => parse_v2(&buffer[offset..end])?,
            3 => parse_v3(&buffer[offset..end])?,
            major => {
                return Err(mft_integrity_error(format!(
                    "unsupported USN record version {major}"
                )))
            }
        };
        records.push(record);
        offset = end;
    }
    Ok((next_start, records))
}

fn parse_v2(bytes: &[u8]) -> Result<MftRecord, GlobalIndexError> {
    if bytes.len() < std::mem::size_of::<USN_RECORD_V2>() {
        return Err(mft_integrity_error("USN_RECORD_V2 is truncated"));
    }
    let record = unsafe { ptr::read_unaligned(bytes.as_ptr() as *const USN_RECORD_V2) };
    parse_name(
        bytes,
        record.FileNameOffset as usize,
        record.FileNameLength as usize,
    )
    .map(|name| MftRecord {
        file_reference: format!("{:016x}", record.FileReferenceNumber),
        parent_reference: format!("{:016x}", record.ParentFileReferenceNumber),
        name,
        timestamp: windows_filetime_to_unix(record.TimeStamp),
        reason: record.Reason,
        attributes: record.FileAttributes,
    })
}

fn parse_v3(bytes: &[u8]) -> Result<MftRecord, GlobalIndexError> {
    if bytes.len() < std::mem::size_of::<USN_RECORD_V3>() {
        return Err(mft_integrity_error("USN_RECORD_V3 is truncated"));
    }
    let record = unsafe { ptr::read_unaligned(bytes.as_ptr() as *const USN_RECORD_V3) };
    parse_name(
        bytes,
        record.FileNameOffset as usize,
        record.FileNameLength as usize,
    )
    .map(|name| MftRecord {
        file_reference: hex_id(&record.FileReferenceNumber.Identifier),
        parent_reference: hex_id(&record.ParentFileReferenceNumber.Identifier),
        name,
        timestamp: windows_filetime_to_unix(record.TimeStamp),
        reason: record.Reason,
        attributes: record.FileAttributes,
    })
}

fn parse_name(bytes: &[u8], offset: usize, length: usize) -> Result<String, GlobalIndexError> {
    let Some(end) = offset.checked_add(length) else {
        return Err(mft_integrity_error("USN file name range overflows"));
    };
    if !length.is_multiple_of(2) || offset > bytes.len() || end > bytes.len() {
        return Err(mft_integrity_error("USN file name range is invalid"));
    }
    let utf16 = bytes[offset..end]
        .chunks_exact(2)
        .map(|chunk| u16::from_ne_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    Ok(String::from_utf16_lossy(&utf16))
}

pub(crate) fn mft_integrity_error(message: impl Into<String>) -> GlobalIndexError {
    GlobalIndexError::RebuildRequired(format!("mft_integrity: {}", message.into()))
}

pub(crate) fn is_integrity_error(error: &GlobalIndexError) -> bool {
    matches!(error, GlobalIndexError::RebuildRequired(_))
}

fn join_windows_path(parent: &str, name: &str) -> String {
    if parent.ends_with(['\\', '/']) {
        format!("{parent}{name}")
    } else {
        format!(r"{parent}\{name}")
    }
}

fn record_key(record: &MftRecord) -> String {
    format!(
        "{}\0{}\0{}",
        record.file_reference, record.parent_reference, record.name
    )
}

#[cfg(test)]
fn reconstruct_paths(root: &str, records: &[MftRecord]) -> HashMap<String, String> {
    let directories = records
        .iter()
        .filter(|record| record.attributes & FILE_ATTRIBUTE_DIRECTORY != 0)
        .cloned()
        .collect::<Vec<_>>();
    let (mut paths, parent_paths) = resolve_directory_paths(root, &directories);
    for record in records {
        if record.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            continue;
        }
        let path = parent_paths
            .get(&record.parent_reference)
            .map(|parent| join_windows_path(parent, &record.name))
            .unwrap_or_else(|| join_windows_path(root, &record.name));
        paths.insert(record_key(record), path);
    }
    paths
}

pub(crate) fn open_volume(mount_path: &str) -> Result<HANDLE, GlobalIndexError> {
    let path = volume_device_path(mount_path);
    let wide = to_wide(&path);
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        // GetLastError must be captured before any other Win32 call. Keep the
        // numeric code typed so permission failures survive IPC and status UI.
        let error = unsafe { GetLastError() };
        return Err(open_volume_error(error));
    }
    Ok(handle)
}

fn open_volume_error(code: u32) -> GlobalIndexError {
    GlobalIndexError::WindowsIo(code)
}

pub(crate) fn query_journal(handle: HANDLE) -> Result<USN_JOURNAL_DATA_V0, GlobalIndexError> {
    match query_journal_once(handle) {
        Ok(journal) => Ok(journal),
        // A freshly formatted or virtual NTFS volume may have no active change
        // journal. Creating it is the documented Windows prerequisite for any
        // USN-based indexing, and only runs for a volume the user already
        // authorized. Note `FSCTL_CREATE_USN_JOURNAL` also *modifies* an
        // existing journal, so it is issued only after a definitive 1179.
        Err(GlobalIndexError::WindowsIo(ERROR_JOURNAL_NOT_ACTIVE)) => {
            recover_absent_journal(|| create_journal(handle), || query_journal_once(handle))
        }
        Err(error) => Err(error),
    }
}

/// Recovers a volume whose change journal is not active (`ERROR_JOURNAL_NOT_ACTIVE`).
///
/// Split out so the race and failure paths are testable without a real volume:
/// - create succeeds → re-query and return the journal;
/// - create fails but a journal appeared meanwhile (another process won the
///   race) → the re-query still succeeds;
/// - create fails and the journal is still absent → preserve the typed create
///   error (notably privilege denial 5/1314). The original absence remains in
///   diagnostic context; a successful competing creator always wins.
fn recover_absent_journal<Create, Query>(
    create: Create,
    mut query: Query,
) -> Result<USN_JOURNAL_DATA_V0, GlobalIndexError>
where
    Create: FnOnce() -> Result<(), GlobalIndexError>,
    Query: FnMut() -> Result<USN_JOURNAL_DATA_V0, GlobalIndexError>,
{
    if let Err(error) = create() {
        eprintln!("global_index usn_journal_create_failed: {error}");
        // Do not trust the failure alone: a concurrent creator may have
        // installed the journal between the failed query and the create.
        return match query() {
            Ok(journal) => Ok(journal),
            Err(GlobalIndexError::WindowsIo(ERROR_JOURNAL_NOT_ACTIVE)) => {
                eprintln!(
                    "global_index usn_journal_still_absent after Win32 1179 recovery: {error}"
                );
                Err(error)
            }
            // A different I/O failure (device removed, volume gone) is surfaced
            // as itself rather than being masked as "journal not active".
            Err(other) => Err(other),
        };
    }
    query()
}

fn query_journal_once(handle: HANDLE) -> Result<USN_JOURNAL_DATA_V0, GlobalIndexError> {
    let mut output = USN_JOURNAL_DATA_V0::default();
    let output_bytes = unsafe {
        std::slice::from_raw_parts_mut(
            (&mut output as *mut USN_JOURNAL_DATA_V0).cast::<u8>(),
            std::mem::size_of::<USN_JOURNAL_DATA_V0>(),
        )
    };
    device_io_control(handle, FSCTL_QUERY_USN_JOURNAL, &[], output_bytes)?;
    Ok(output)
}

fn create_journal(handle: HANDLE) -> Result<(), GlobalIndexError> {
    let data = CREATE_USN_JOURNAL_DATA {
        MaximumSize: USN_JOURNAL_MAXIMUM_BYTES,
        AllocationDelta: USN_JOURNAL_ALLOCATION_DELTA,
    };
    let input_bytes = unsafe {
        std::slice::from_raw_parts(
            (&data as *const CREATE_USN_JOURNAL_DATA).cast::<u8>(),
            std::mem::size_of::<CREATE_USN_JOURNAL_DATA>(),
        )
    };
    device_io_control(handle, FSCTL_CREATE_USN_JOURNAL, input_bytes, &mut []).map(|_| ())
}

pub(crate) fn is_win32_error(error: &GlobalIndexError, code: u32) -> bool {
    matches!(error, GlobalIndexError::WindowsIo(actual) if *actual == code)
}

pub(crate) unsafe fn device_io_control_bytes(
    handle: HANDLE,
    code: u32,
    input: &[u8],
    output: &mut [u8],
) -> Result<usize, GlobalIndexError> {
    let mut returned = 0u32;
    let ok = DeviceIoControl(
        handle,
        code,
        input.as_ptr().cast(),
        input.len() as u32,
        output.as_mut_ptr().cast(),
        output.len() as u32,
        &mut returned,
        ptr::null_mut(),
    );
    if ok == 0 {
        return Err(GlobalIndexError::WindowsIo(GetLastError()));
    }
    Ok(returned as usize)
}

fn device_io_control(
    handle: HANDLE,
    code: u32,
    input: &[u8],
    output: &mut [u8],
) -> Result<usize, GlobalIndexError> {
    unsafe { device_io_control_bytes(handle, code, input, output) }
}

fn windows_filetime_to_unix(value: i64) -> Option<i64> {
    if value <= FILETIME_UNIX_EPOCH {
        return None;
    }
    Some((value - FILETIME_UNIX_EPOCH) / 10_000_000)
}

fn hex_id(value: &[u8; 16]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Storage::FileSystem::FILE_ID_128;

    #[test]
    fn journal_recovery_requeries_after_successful_create() {
        let journal = recover_absent_journal(
            || Ok(()),
            || {
                Ok(USN_JOURNAL_DATA_V0 {
                    UsnJournalID: 7,
                    ..Default::default()
                })
            },
        )
        .expect("a created journal must be re-queried");

        assert_eq!(journal.UsnJournalID, 7);
    }

    #[test]
    fn failed_create_and_absent_requery_preserve_actionable_create_error() {
        for code in [5, 1314, 19] {
            let mut queries = 0;
            let error = match recover_absent_journal(
                || Err(GlobalIndexError::WindowsIo(code)),
                || {
                    queries += 1;
                    Err(GlobalIndexError::WindowsIo(ERROR_JOURNAL_NOT_ACTIVE))
                },
            ) {
                Ok(_) => panic!("still absent"),
                Err(error) => error,
            };
            assert!(
                matches!(error, GlobalIndexError::WindowsIo(actual) if actual == code),
                "{error:?}"
            );
            assert_eq!(queries, 1);
            // Feed the recovery outcome through the production IPC taxonomy.
            let (wire_code, status) = super::super::service_errors::classify(&error);
            let response = super::super::service::IndexServiceResponse {
                protocol_version: super::super::service::IPC_PROTOCOL_VERSION,
                request_id: "journal-recovery-fixture".into(),
                ok: false,
                error_code: Some(wire_code),
                message: Some(
                    "1179 remained after create; diagnostic text is not a classifier".into(),
                ),
                status: Some(status.into()),
            };
            let bytes = serde_json::to_vec(&response).unwrap();
            let decoded =
                super::super::service_errors::decode(&serde_json::from_slice(&bytes).unwrap());
            assert!(matches!(decoded, GlobalIndexError::WindowsIo(actual) if actual == code));
            assert_eq!(
                status,
                if matches!(code, 5 | 1314) {
                    "permission_required"
                } else {
                    "error"
                }
            );
        }
    }

    #[test]
    fn journal_created_by_another_process_wins_the_race() {
        // Another process installed the journal between our failed query and
        // the denied create (5/1314); the re-query must accept it.
        let journal = recover_absent_journal(
            || Err(GlobalIndexError::WindowsIo(1314)),
            || {
                Ok(USN_JOURNAL_DATA_V0 {
                    UsnJournalID: 11,
                    ..Default::default()
                })
            },
        )
        .expect("a concurrently created journal is usable");

        assert_eq!(journal.UsnJournalID, 11);
    }

    #[test]
    fn requery_failure_other_than_1179_is_not_masked() {
        // A volume that disappeared after the create attempt must surface its
        // own I/O code instead of being reported as "journal not active".
        let error = match recover_absent_journal(
            || Err(GlobalIndexError::WindowsIo(5)),
            || Err(GlobalIndexError::WindowsIo(21)),
        ) {
            Ok(_) => panic!("a vanished volume must fail"),
            Err(error) => error,
        };

        assert!(
            matches!(error, GlobalIndexError::WindowsIo(21)),
            "unexpected classification: {error:?}"
        );
    }

    #[test]
    fn open_volume_preserves_actual_win32_permission_codes() {
        for code in [5, 1314, 32] {
            assert!(
                matches!(open_volume_error(code), GlobalIndexError::WindowsIo(actual) if actual == code)
            );
        }
    }

    fn fixture_record(major: u16, name: &str) -> Vec<u8> {
        let name_bytes = name.encode_utf16().collect::<Vec<_>>();
        let record_size = std::mem::size_of::<USN_RECORD_V2>() - 2 + name_bytes.len() * 2;
        let record_size = (record_size + 7) & !7;
        let mut bytes = vec![0u8; 8 + record_size];
        bytes[0..8].copy_from_slice(&42u64.to_ne_bytes());
        let record = USN_RECORD_V2 {
            RecordLength: record_size as u32,
            MajorVersion: major,
            FileReferenceNumber: 10,
            ParentFileReferenceNumber: 5,
            FileNameOffset: (std::mem::size_of::<USN_RECORD_V2>() - 2) as u16,
            FileNameLength: (name_bytes.len() * 2) as u16,
            ..Default::default()
        };
        unsafe {
            ptr::copy_nonoverlapping(
                (&record as *const USN_RECORD_V2).cast::<u8>(),
                bytes[8..].as_mut_ptr(),
                std::mem::size_of::<USN_RECORD_V2>() - 2,
            );
        }
        for (index, value) in name_bytes.iter().enumerate() {
            bytes[8 + record.FileNameOffset as usize + index * 2
                ..8 + record.FileNameOffset as usize + index * 2 + 2]
                .copy_from_slice(&value.to_ne_bytes());
        }
        bytes
    }

    fn fixture_record_v3(name: &str) -> Vec<u8> {
        let name_bytes = name.encode_utf16().collect::<Vec<_>>();
        let record_size = std::mem::size_of::<USN_RECORD_V3>() - 2 + name_bytes.len() * 2;
        let record_size = (record_size + 7) & !7;
        let mut bytes = vec![0u8; 8 + record_size];
        bytes[0..8].copy_from_slice(&84u64.to_ne_bytes());
        let record = USN_RECORD_V3 {
            RecordLength: record_size as u32,
            MajorVersion: 3,
            MinorVersion: 0,
            FileReferenceNumber: FILE_ID_128 {
                Identifier: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            },
            ParentFileReferenceNumber: FILE_ID_128 {
                Identifier: [16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1],
            },
            FileNameOffset: (std::mem::size_of::<USN_RECORD_V3>() - 2) as u16,
            FileNameLength: (name_bytes.len() * 2) as u16,
            ..Default::default()
        };
        unsafe {
            ptr::copy_nonoverlapping(
                (&record as *const USN_RECORD_V3).cast::<u8>(),
                bytes[8..].as_mut_ptr(),
                std::mem::size_of::<USN_RECORD_V3>() - 2,
            );
        }
        for (index, value) in name_bytes.iter().enumerate() {
            bytes[8 + record.FileNameOffset as usize + index * 2
                ..8 + record.FileNameOffset as usize + index * 2 + 2]
                .copy_from_slice(&value.to_ne_bytes());
        }
        bytes
    }

    #[test]
    fn parses_usn_record_v2_fixture() {
        let page = fixture_record(2, "报告.txt");
        let (cursor, records) = parse_mft_page(&page).expect("parse fixture");
        assert_eq!(cursor, 42);
        assert_eq!(records[0].file_reference, "000000000000000a");
        assert_eq!(records[0].parent_reference, "0000000000000005");
        assert_eq!(records[0].name, "报告.txt");
    }

    #[test]
    fn parses_usn_record_v3_fixture() {
        let page = fixture_record_v3("résumé.pdf");
        let (cursor, records) = parse_mft_page(&page).expect("parse v3 fixture");
        assert_eq!(cursor, 84);
        assert_eq!(
            records[0].file_reference,
            "0102030405060708090a0b0c0d0e0f10"
        );
        assert_eq!(
            records[0].parent_reference,
            "100f0e0d0c0b0a090807060504030201"
        );
        assert_eq!(records[0].name, "résumé.pdf");
    }

    #[test]
    fn reconstructs_parent_late_and_orphan_records_without_looping() {
        let records = vec![
            MftRecord {
                file_reference: "2".to_string(),
                parent_reference: "1".to_string(),
                name: "child.txt".to_string(),
                timestamp: None,
                reason: 0,
                attributes: 0,
            },
            MftRecord {
                file_reference: "1".to_string(),
                parent_reference: "1".to_string(),
                name: ".".to_string(),
                timestamp: None,
                reason: 0,
                attributes: FILE_ATTRIBUTE_DIRECTORY,
            },
            MftRecord {
                file_reference: "3".to_string(),
                parent_reference: "3".to_string(),
                name: "loop".to_string(),
                timestamp: None,
                reason: 0,
                attributes: FILE_ATTRIBUTE_DIRECTORY,
            },
        ];
        let paths = reconstruct_paths("C:\\", &records);
        assert_eq!(
            paths.get("2\u{0}1\u{0}child.txt"),
            Some(&"C:\\child.txt".to_string())
        );
        assert!(paths.contains_key("3\u{0}3\u{0}loop"));
    }

    #[test]
    fn preserves_multiple_directory_entries_for_a_hard_linked_file() {
        let records = vec![
            MftRecord {
                file_reference: "7".to_string(),
                parent_reference: "1".to_string(),
                name: "left.txt".to_string(),
                timestamp: None,
                reason: 0,
                attributes: 0,
            },
            MftRecord {
                file_reference: "7".to_string(),
                parent_reference: "1".to_string(),
                name: "right.txt".to_string(),
                timestamp: None,
                reason: 0,
                attributes: 0,
            },
            MftRecord {
                file_reference: "1".to_string(),
                parent_reference: "1".to_string(),
                name: ".".to_string(),
                timestamp: None,
                reason: 0,
                attributes: FILE_ATTRIBUTE_DIRECTORY,
            },
        ];
        let paths = reconstruct_paths("C:\\", &records);
        assert_eq!(
            paths.get("7\u{0}1\u{0}left.txt"),
            Some(&"C:\\left.txt".to_string())
        );
        assert_eq!(
            paths.get("7\u{0}1\u{0}right.txt"),
            Some(&"C:\\right.txt".to_string())
        );
    }

    #[test]
    fn malformed_mft_pages_are_integrity_errors() {
        let error = parse_mft_page(&[1, 2, 3]).expect_err("short page must fail closed");
        assert!(is_integrity_error(&error));

        let error = parse_mft_page(&[0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3])
            .expect_err("non-zero truncated tail must fail closed");
        assert!(is_integrity_error(&error));
    }

    #[test]
    fn resolves_ntfs_root_children_from_v2_parent_reference_without_root_record() {
        let root_reference = "0001000000000005";
        let directory = MftRecord {
            file_reference: "0001000000000010".to_string(),
            parent_reference: root_reference.to_string(),
            name: "Users".to_string(),
            timestamp: None,
            reason: 0,
            attributes: FILE_ATTRIBUTE_DIRECTORY,
        };
        let directory_key = record_key(&directory);
        let directories = [directory];

        let (paths, parents) = resolve_directory_paths("C:\\", &directories);

        assert_eq!(
            parents.get(root_reference).map(String::as_str),
            Some("C:\\")
        );
        assert_eq!(
            paths.get(&directory_key).map(String::as_str),
            Some("C:\\Users")
        );
    }

    #[test]
    fn recognizes_ntfs_root_file_references_in_v2_and_v3_records() {
        assert!(is_ntfs_root_reference("0001000000000005"));
        assert!(!is_ntfs_root_reference("0001000000000006"));
        assert!(is_ntfs_root_reference("05000000000001000000000000000000"));
        assert!(!is_ntfs_root_reference("06000000000001000000000000000000"));
    }

    #[test]
    fn initial_baseline_with_emitted_searchable_entries_succeeds() {
        let stats = MftBaselineStats {
            staged_records: 3,
            resolved_entries: 2,
            emitted_entries: 2,
            checkpoint_usn: 417,
        };

        validate_initial_baseline(stats).expect("non-empty baseline is complete");
    }

    #[test]
    fn zero_entry_initial_baseline_is_a_stable_integrity_failure() {
        let stats = MftBaselineStats {
            staged_records: 12,
            resolved_entries: 0,
            emitted_entries: 0,
            checkpoint_usn: 8192,
        };

        let error = validate_initial_baseline(stats).expect_err("empty baseline is incomplete");

        assert!(is_integrity_error(&error));
        assert!(error.to_string().contains(
            "windows_mft_baseline_empty: staged_records=12 resolved_entries=0 emitted_entries=0 checkpoint_usn=8192"
        ));
    }
}
