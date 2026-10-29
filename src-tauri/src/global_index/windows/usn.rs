use super::mft::{
    device_io_control_bytes, mft_integrity_error, open_volume, parse_mft_page, query_journal,
    MftRecord,
};
use super::{mft, volumes::volume_device_path};
use crate::global_index::coordinator::{GlobalIndexError, GlobalIndexSink};
use crate::global_index::models::{
    GlobalEntry, GlobalEntryInput, GlobalSourceDescriptor, PROVIDER_WINDOWS_MFT_USN,
};
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_HANDLE_EOF, HANDLE};
use windows_sys::Win32::System::Ioctl::{
    FSCTL_READ_USN_JOURNAL, READ_USN_JOURNAL_DATA_V0, USN_REASON_BASIC_INFO_CHANGE,
    USN_REASON_CLOSE, USN_REASON_DATA_EXTEND, USN_REASON_DATA_OVERWRITE,
    USN_REASON_DATA_TRUNCATION, USN_REASON_EA_CHANGE, USN_REASON_FILE_CREATE,
    USN_REASON_FILE_DELETE, USN_REASON_HARD_LINK_CHANGE, USN_REASON_RENAME_NEW_NAME,
    USN_REASON_RENAME_OLD_NAME, USN_REASON_SECURITY_CHANGE,
};

const READ_BUFFER_SIZE: usize = 1024 * 1024;
const CHANGE_REASONS: u32 = USN_REASON_DATA_OVERWRITE
    | USN_REASON_DATA_EXTEND
    | USN_REASON_DATA_TRUNCATION
    | USN_REASON_BASIC_INFO_CHANGE
    | USN_REASON_EA_CHANGE
    | USN_REASON_SECURITY_CHANGE
    | USN_REASON_FILE_CREATE
    | USN_REASON_FILE_DELETE
    | USN_REASON_RENAME_OLD_NAME
    | USN_REASON_RENAME_NEW_NAME
    | USN_REASON_HARD_LINK_CHANGE
    | USN_REASON_CLOSE;

pub(crate) struct UsnSyncResult {
    pub(crate) directory_path_changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UsnChangeAction {
    Stale,
    Upsert,
    Ignore,
}

pub(crate) fn sync_volume(
    source: &GlobalSourceDescriptor,
    sink: &mut dyn GlobalIndexSink,
    cancel: &AtomicBool,
) -> Result<UsnSyncResult, GlobalIndexError> {
    let handle = open_volume(&source.volume.mount_path)?;
    let result = sync_with_handle(source, handle, sink, cancel);
    unsafe {
        CloseHandle(handle);
    }
    result
}

fn sync_with_handle(
    source: &GlobalSourceDescriptor,
    handle: HANDLE,
    sink: &mut dyn GlobalIndexSink,
    cancel: &AtomicBool,
) -> Result<UsnSyncResult, GlobalIndexError> {
    let journal = query_journal(handle)?;
    let saved_cursor = validate_checkpoint(
        source,
        sink,
        journal.UsnJournalID,
        journal.FirstUsn,
        journal.NextUsn,
    )?;
    let Some(saved_cursor) = saved_cursor else {
        sink.checkpoint(
            &source.volume.id,
            Some(&journal.UsnJournalID.to_string()),
            Some(&journal.NextUsn.to_string()),
        )?;
        return Ok(UsnSyncResult {
            directory_path_changed: false,
        });
    };

    let mut cursor = saved_cursor;
    if cursor == journal.NextUsn {
        sink.checkpoint(
            &source.volume.id,
            Some(&journal.UsnJournalID.to_string()),
            Some(&cursor.to_string()),
        )?;
        return Ok(UsnSyncResult {
            directory_path_changed: false,
        });
    }
    let mut directory_path_changed = false;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(GlobalIndexError::Paused);
        }
        let input = READ_USN_JOURNAL_DATA_V0 {
            StartUsn: cursor,
            ReasonMask: CHANGE_REASONS,
            ReturnOnlyOnClose: 0,
            Timeout: 0,
            BytesToWaitFor: 0,
            UsnJournalID: journal.UsnJournalID,
        };
        let input_bytes = unsafe {
            std::slice::from_raw_parts(
                (&input as *const READ_USN_JOURNAL_DATA_V0).cast::<u8>(),
                std::mem::size_of::<READ_USN_JOURNAL_DATA_V0>(),
            )
        };
        let mut output = vec![0u8; READ_BUFFER_SIZE];
        let bytes = match unsafe {
            device_io_control_bytes(handle, FSCTL_READ_USN_JOURNAL, input_bytes, &mut output)
        } {
            Ok(bytes) => bytes,
            Err(error) if mft::is_win32_error(&error, ERROR_HANDLE_EOF) => break,
            Err(error) => return classify_read_error(&source.volume.id, sink, error),
        };
        let (next_cursor, records) = parse_sync_page(
            &source.volume.id,
            sink,
            &output[..bytes],
            cursor,
            journal.NextUsn,
        )?;

        for record in records {
            if cancel.load(Ordering::Acquire) {
                return Err(GlobalIndexError::Paused);
            }
            match classify_usn_change(record.reason) {
                UsnChangeAction::Stale => {
                    if let Some(entry) = sink.find_entry_by_identity(
                        &source.volume.id,
                        &record.file_reference,
                        &record.parent_reference,
                        &record.name,
                    )? {
                        sink.mark_entry_stale(&entry.id)?;
                    }
                    if record.reason & USN_REASON_RENAME_OLD_NAME != 0 {
                        directory_path_changed |= record.attributes & 0x10 != 0;
                    }
                    continue;
                }
                UsnChangeAction::Ignore => continue,
                UsnChangeAction::Upsert => {}
            }
            let existing = sink.find_entry_by_identity(
                &source.volume.id,
                &record.file_reference,
                &record.parent_reference,
                &record.name,
            )?;
            let path = resolve_change_path(source, sink, &record)?;
            let input = change_to_entry(source, &record, path, existing.as_ref());
            sink.write_batch(std::slice::from_ref(&input))?;
            if record.reason & USN_REASON_RENAME_NEW_NAME != 0 && record.attributes & 0x10 != 0 {
                directory_path_changed = true;
            }
        }
        cursor = next_cursor as i64;
        if cursor <= saved_cursor || cursor >= journal.NextUsn {
            break;
        }
    }
    sink.checkpoint(
        &source.volume.id,
        Some(&journal.UsnJournalID.to_string()),
        Some(&cursor.to_string()),
    )?;
    Ok(UsnSyncResult {
        directory_path_changed,
    })
}

pub(crate) fn require_rebuild<T>(
    volume_id: &str,
    sink: &mut dyn GlobalIndexSink,
    message: impl Into<String>,
) -> Result<T, GlobalIndexError> {
    let message = message.into();
    sink.set_source_state(
        volume_id,
        crate::global_index::models::INDEX_STATUS_REBUILD_REQUIRED,
        Some(&message),
    )?;
    Err(GlobalIndexError::RebuildRequired(message))
}

pub(super) fn validate_checkpoint(
    source: &GlobalSourceDescriptor,
    sink: &mut dyn GlobalIndexSink,
    journal_id: u64,
    first_usn: i64,
    next_usn: i64,
) -> Result<Option<i64>, GlobalIndexError> {
    if source
        .volume
        .journal_id
        .as_deref()
        .is_some_and(|saved| saved != journal_id.to_string())
    {
        return require_rebuild(
            &source.volume.id,
            sink,
            "USN Journal ID changed; a volume rebuild is required",
        );
    }
    let cursor = source
        .volume
        .journal_cursor
        .as_deref()
        .and_then(|value| value.parse::<i64>().ok());
    if cursor.is_some_and(|cursor| cursor < first_usn || cursor > next_usn) {
        return require_rebuild(
            &source.volume.id,
            sink,
            "USN Journal cursor is outside the available history",
        );
    }
    Ok(cursor)
}

pub(super) fn parse_sync_page(
    volume_id: &str,
    sink: &mut dyn GlobalIndexSink,
    page: &[u8],
    cursor: i64,
    next_usn: i64,
) -> Result<(u64, Vec<MftRecord>), GlobalIndexError> {
    if page.len() < 8 {
        return require_rebuild(
            volume_id,
            sink,
            mft_integrity_error("USN page is shorter than the continuation cursor").to_string(),
        );
    }
    let (next_cursor, records) = match parse_mft_page(page) {
        Ok(parsed) => parsed,
        Err(error) => {
            return require_rebuild(
                volume_id,
                sink,
                format!("USN journal data could not be parsed: {error}"),
            )
        }
    };
    if next_cursor as i64 <= cursor || next_cursor as i64 > next_usn {
        return require_rebuild(
            volume_id,
            sink,
            "USN journal cursor is not continuous; a volume rebuild is required",
        );
    }
    Ok((next_cursor, records))
}

fn classify_usn_change(reason: u32) -> UsnChangeAction {
    if reason & (USN_REASON_RENAME_OLD_NAME | USN_REASON_FILE_DELETE) != 0 {
        UsnChangeAction::Stale
    } else if reason & (CHANGE_REASONS & !USN_REASON_RENAME_OLD_NAME) != 0 {
        UsnChangeAction::Upsert
    } else {
        UsnChangeAction::Ignore
    }
}

pub(super) fn classify_read_error<T>(
    volume_id: &str,
    sink: &mut dyn GlobalIndexSink,
    error: GlobalIndexError,
) -> Result<T, GlobalIndexError> {
    if is_journal_history_error(&error) {
        require_rebuild(
            volume_id,
            sink,
            format!("USN journal history is no longer readable: {error}"),
        )
    } else {
        Err(error)
    }
}

fn is_journal_history_error(error: &GlobalIndexError) -> bool {
    matches!(error, GlobalIndexError::WindowsIo(1181 | 1178 | 1179))
}

fn resolve_change_path(
    source: &GlobalSourceDescriptor,
    sink: &mut dyn GlobalIndexSink,
    record: &MftRecord,
) -> Result<String, GlobalIndexError> {
    let parent = sink
        .resolve_parent_path(&source.volume.id, &record.parent_reference)?
        .unwrap_or_else(|| {
            source
                .volume
                .mount_path
                .trim_end_matches(['\\', '/'])
                .to_string()
        });
    if parent.ends_with(['\\', '/']) {
        Ok(format!("{parent}{}", record.name))
    } else {
        Ok(format!(r"{parent}\{}", record.name))
    }
}

fn change_to_entry(
    source: &GlobalSourceDescriptor,
    record: &MftRecord,
    path: String,
    existing: Option<&GlobalEntry>,
) -> GlobalEntryInput {
    let is_directory = record.attributes & 0x10 != 0;
    let extension = if is_directory {
        String::new()
    } else {
        std::path::Path::new(&record.name)
            .extension()
            .map(|value| value.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    };
    GlobalEntryInput {
        volume_id: source.volume.id.clone(),
        platform_file_id: record.file_reference.clone(),
        parent_platform_file_id: record.parent_reference.clone(),
        name: record.name.clone(),
        path,
        extension,
        is_directory,
        size: existing.map(|value| value.size).unwrap_or_default(),
        created_at_fs: existing
            .and_then(|value| value.created_at_fs)
            .or(record.timestamp),
        modified_at_fs: record
            .timestamp
            .or_else(|| existing.and_then(|value| value.modified_at_fs)),
        file_attributes: i64::from(record.attributes),
        is_hidden: record.attributes & 0x2 != 0,
        is_system: record.attributes & 0x4 != 0,
        source_provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
        last_seen_at: crate::global_index::models::unix_now(),
    }
}

#[allow(dead_code)]
fn _volume_path_for_diagnostics(source: &GlobalSourceDescriptor) -> String {
    volume_device_path(&source.volume.mount_path)
}

#[allow(dead_code)]
fn _last_error() -> u32 {
    unsafe { GetLastError() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global_index::models::GlobalVolume;
    use crate::global_index::windows::mft::MftRecord;

    #[test]
    fn classifies_create_delete_and_rename_records_without_silent_drops() {
        assert_eq!(
            classify_usn_change(USN_REASON_FILE_DELETE),
            UsnChangeAction::Stale
        );
        assert_eq!(
            classify_usn_change(USN_REASON_RENAME_OLD_NAME),
            UsnChangeAction::Stale
        );
        assert_eq!(
            classify_usn_change(USN_REASON_FILE_CREATE),
            UsnChangeAction::Upsert
        );
        assert_eq!(
            classify_usn_change(USN_REASON_RENAME_NEW_NAME),
            UsnChangeAction::Upsert
        );
        assert_eq!(classify_usn_change(0), UsnChangeAction::Ignore);
    }

    #[test]
    fn change_to_entry_preserves_existing_size_without_reading_contents() {
        let source = GlobalSourceDescriptor {
            volume: GlobalVolume {
                id: "volume".to_string(),
                platform: "windows".to_string(),
                stable_volume_id: "stable".to_string(),
                display_name: "C".to_string(),
                mount_path: "C:\\".to_string(),
                filesystem_type: "NTFS".to_string(),
                drive_kind: "fixed".to_string(),
                enabled: true,
                scope_authorization_state: "authorized".to_string(),
                provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
                index_status: "ready".to_string(),
                last_error: None,
                journal_id: None,
                journal_cursor: None,
                last_full_index_at: None,
                last_incremental_sync_at: None,
                entry_count: 1,
                created_at: 0,
                updated_at: 0,
            },
        };
        let existing = GlobalEntry {
            id: "entry".to_string(),
            volume_id: "volume".to_string(),
            platform_file_id: "1".to_string(),
            parent_platform_file_id: "0".to_string(),
            name: "a.txt".to_string(),
            name_normalized: "a.txt".to_string(),
            path: "C:\\a.txt".to_string(),
            path_normalized: "c:/a.txt".to_string(),
            extension: "txt".to_string(),
            is_directory: false,
            size: 42,
            created_at_fs: Some(1),
            modified_at_fs: Some(2),
            file_attributes: 0,
            is_hidden: false,
            is_system: false,
            is_stale: false,
            source_provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
            last_seen_at: 3,
        };
        let record = MftRecord {
            file_reference: "1".to_string(),
            parent_reference: "0".to_string(),
            name: "a.txt".to_string(),
            timestamp: Some(4),
            reason: USN_REASON_DATA_EXTEND,
            attributes: 0,
        };
        let input = change_to_entry(&source, &record, "C:\\a.txt".to_string(), Some(&existing));
        assert_eq!(input.size, 42);
        assert_eq!(input.modified_at_fs, Some(4));
    }

    #[test]
    fn journal_history_errors_are_rebuild_signals() {
        for code in [1181, 1178, 1179] {
            assert!(is_journal_history_error(&GlobalIndexError::WindowsIo(code)));
        }
        assert!(!is_journal_history_error(&GlobalIndexError::WindowsIo(5)));
        assert!(!is_journal_history_error(&GlobalIndexError::Provider(
            "Win32 error 1181".into()
        )));
    }
}
