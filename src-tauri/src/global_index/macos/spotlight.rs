use super::{KnownEntries, PendingUpdates};
use crate::global_index::models::{
    normalize_path, GlobalEntryInput, MACOS_FILE_ATTRIBUTE_CLOUD_NOT_LOCAL,
    MACOS_FILE_ATTRIBUTE_CONTENT_DOWNLOADING, MACOS_FILE_ATTRIBUTE_CONTENT_UNKNOWN,
    MACOS_FILE_ATTRIBUTE_FILE_PROVIDER, MACOS_FILE_ATTRIBUTE_ICLOUD, MACOS_FILE_ATTRIBUTE_PACKAGE,
    PROVIDER_MACOS_SPOTLIGHT,
};
use crate::global_index::wake::{GlobalIndexWakeReason, GlobalIndexWakeSlot};
use crate::platform::macos::run_loop::{cross_thread_stop_action, NativeRunLoopStopSignal};
use block2::RcBlock;
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_foundation::{
    NSArray, NSDate, NSMetadataItem, NSMetadataItemFSContentChangeDateKey,
    NSMetadataItemFSCreationDateKey, NSMetadataItemFSNameKey, NSMetadataItemFSSizeKey,
    NSMetadataItemPathKey, NSMetadataItemURLKey, NSMetadataQuery,
    NSMetadataQueryIndexedLocalComputerScope, NSMetadataQueryUpdateAddedItemsKey,
    NSMetadataQueryUpdateChangedItemsKey, NSMetadataQueryUpdateRemovedItemsKey, NSNotification,
    NSNotificationCenter, NSNumber, NSObjectProtocol, NSPredicate, NSRunLoop, NSString, NSURL,
};
use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};

// Spotlight metadata query strings use `==` with glob wildcards; `LIKE` is
// NSPredicate syntax. This matches every indexed item that has a file name.
const SPOTLIGHT_MATCH_ALL_QUERY: &str = "kMDItemFSName == \"*\"";

pub(super) struct SpotlightWatcherHandle {
    stopped: Arc<AtomicBool>,
    stop_signal: Arc<NativeRunLoopStopSignal>,
    thread: Option<JoinHandle<()>>,
}

impl SpotlightWatcherHandle {
    pub(super) fn stop(mut self) {
        self.stopped.store(true, Ordering::Release);
        self.stop_signal.request_stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpotlightCollectionSummary {
    pub processed: usize,
    pub path_fallbacks: usize,
    pub skipped: usize,
    pub full_disk_access_required: bool,
    pub external_volume_not_indexed: bool,
}

#[derive(Debug)]
struct ExternalVolumeProbe {
    root: PathBuf,
    has_readable_content: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpotlightUpdateAction {
    Upsert,
    Stale,
    Reconcile,
}

fn classify_spotlight_update(removed: bool, path_identity_fallback: bool) -> SpotlightUpdateAction {
    if path_identity_fallback {
        SpotlightUpdateAction::Reconcile
    } else if removed {
        SpotlightUpdateAction::Stale
    } else {
        SpotlightUpdateAction::Upsert
    }
}

pub fn stream_local_computer_entries<F>(
    volume_id: &str,
    cancel: &AtomicBool,
    mut on_batch: F,
) -> Result<SpotlightCollectionSummary, String>
where
    F: FnMut(&[GlobalEntryInput]) -> Result<(), String>,
{
    if cancel.load(Ordering::Acquire) {
        return Err("macos_spotlight_query_paused".to_string());
    }
    autoreleasepool(|_| {
        let query = new_local_computer_query()?;
        if !query.startQuery() {
            return Err("macos_spotlight_query_unavailable".to_string());
        }
        let run_loop = NSRunLoop::currentRunLoop();
        let external_volumes = mounted_external_volume_probes();
        let mut external_volume_hits = HashSet::new();
        while query.isGathering() {
            if cancel.load(Ordering::Acquire) {
                query.stopQuery();
                return Err("macos_spotlight_query_paused".to_string());
            }
            let deadline = NSDate::dateWithTimeIntervalSinceNow(0.2);
            run_loop.runUntilDate(&deadline);
        }
        let mut entries = Vec::with_capacity(512);
        let mut summary = SpotlightCollectionSummary::default();
        for index in 0..query.resultCount() {
            if cancel.load(Ordering::Acquire) {
                query.stopQuery();
                return Err("macos_spotlight_query_paused".to_string());
            }
            let object = query.resultAtIndex(index);
            if let Some(path) = path_from_object(&object) {
                if let Some(volume) = external_volumes
                    .iter()
                    .find(|volume| Path::new(&path).starts_with(&volume.root))
                {
                    external_volume_hits.insert(volume.root.clone());
                }
            }
            let Some(entry) = metadata_item_to_entry(volume_id, &object) else {
                summary.skipped += 1;
                continue;
            };
            summary.processed += 1;
            if has_path_identity(&entry) {
                summary.path_fallbacks += 1;
            }
            entries.push(entry);
            if entries.len() >= 512 {
                if let Err(error) = on_batch(&entries) {
                    query.stopQuery();
                    return Err(format!("callback:{error}"));
                }
                entries.clear();
            }
        }
        if !entries.is_empty() {
            if let Err(error) = on_batch(&entries) {
                query.stopQuery();
                return Err(format!("callback:{error}"));
            }
        }
        query.stopQuery();
        summary.full_disk_access_required = full_disk_access_required();
        summary.external_volume_not_indexed = external_volumes.iter().any(|volume| {
            volume.has_readable_content && !external_volume_hits.contains(&volume.root)
        });
        Ok(summary)
    })
}

pub fn spawn_update_watcher(
    volume_id: String,
    pending: Arc<Mutex<PendingUpdates>>,
    known_entries: Arc<Mutex<KnownEntries>>,
    stopped: Arc<AtomicBool>,
    wake: Arc<GlobalIndexWakeSlot>,
) -> Result<SpotlightWatcherHandle, String> {
    let stop_signal = Arc::new(NativeRunLoopStopSignal::default());
    let stop_signal_for_thread = stop_signal.clone();
    let stopped_for_handle = stopped.clone();
    let thread = thread::Builder::new()
        .name("zen-canvas-macos-spotlight".to_string())
        .spawn(move || {
            autoreleasepool(|_| {
                run_update_watcher(
                    &volume_id,
                    &pending,
                    &known_entries,
                    &stopped,
                    &wake,
                    &stop_signal_for_thread,
                )
            });
        })
        .map_err(|error| format!("macos_spotlight_thread_start_failed: {error}"))?;
    Ok(SpotlightWatcherHandle {
        stopped: stopped_for_handle,
        stop_signal,
        thread: Some(thread),
    })
}

fn run_update_watcher(
    volume_id: &str,
    pending: &Arc<Mutex<PendingUpdates>>,
    known_entries: &Arc<Mutex<KnownEntries>>,
    stopped: &AtomicBool,
    wake: &Arc<GlobalIndexWakeSlot>,
    stop_signal: &NativeRunLoopStopSignal,
) {
    let query = match new_local_computer_query() {
        Ok(query) => query,
        Err(error) => {
            eprintln!("Spotlight watcher query initialization failed: {error}");
            super::record_async_watcher_error(pending, wake, error);
            return;
        }
    };
    let center = NSNotificationCenter::defaultCenter();
    let pending_for_block = pending.clone();
    let known_entries_for_block = known_entries.clone();
    let volume_id_for_block = volume_id.to_string();
    let wake_for_block = wake.clone();
    let update_block = RcBlock::new(move |notification: NonNull<NSNotification>| {
        let notification = unsafe { notification.as_ref() };
        let Some(user_info) = notification.userInfo() else {
            return;
        };
        let typed_info = unsafe { user_info.cast_unchecked() };
        let mut entries = Vec::new();
        let mut stale_entry_ids = Vec::new();
        let mut full_reconcile = false;
        let added_start = entries.len();
        collect_update_items(
            typed_info,
            unsafe { NSMetadataQueryUpdateAddedItemsKey },
            &volume_id_for_block,
            &known_entries_for_block,
            &mut entries,
            &mut stale_entry_ids,
            &mut full_reconcile,
            false,
        );
        let added_count = entries.len() - added_start;
        let changed_start = entries.len();
        collect_update_items(
            typed_info,
            unsafe { NSMetadataQueryUpdateChangedItemsKey },
            &volume_id_for_block,
            &known_entries_for_block,
            &mut entries,
            &mut stale_entry_ids,
            &mut full_reconcile,
            false,
        );
        let changed_count = entries.len() - changed_start;
        let removed_start = stale_entry_ids.len();
        collect_update_items(
            typed_info,
            unsafe { NSMetadataQueryUpdateRemovedItemsKey },
            &volume_id_for_block,
            &known_entries_for_block,
            &mut entries,
            &mut stale_entry_ids,
            &mut full_reconcile,
            true,
        );
        let removed_count = stale_entry_ids.len() - removed_start;
        if entries.is_empty() && stale_entry_ids.is_empty() && !full_reconcile {
            return;
        }
        if let Ok(mut pending) = pending_for_block.lock() {
            pending.append_incremental(entries, stale_entry_ids, full_reconcile);
            #[cfg(feature = "native-qa")]
            super::super::qa_trace::record(&format!(
                "provider_change_source\tspotlight_incremental\tadded={added_count}\tchanged={changed_count}\tremoved={removed_count}\treconcile={}",
                u8::from(full_reconcile)
            ));
            wake_for_block.notify(GlobalIndexWakeReason::ProviderChange);
        }
    });
    let query_object: &AnyObject = query.as_ref();
    let observer = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(objc2_foundation::NSMetadataQueryDidUpdateNotification),
            Some(query_object),
            None,
            &update_block,
        )
    };
    if !query.startQuery() {
        record_realtime_start_failure(pending, wake);
        let protocol_object: &ProtocolObject<dyn NSObjectProtocol> = observer.as_ref();
        let observer_object: &AnyObject = protocol_object.as_ref();
        unsafe { center.removeObserver(observer_object) };
        return;
    }
    let run_loop = NSRunLoop::currentRunLoop();
    let stop_action = cross_thread_stop_action(run_loop.getCFRunLoop());
    stop_signal.install(stop_action);
    if !stopped.load(Ordering::Acquire) {
        run_loop.run();
    }
    stop_signal.clear();
    query.stopQuery();
    let protocol_object: &ProtocolObject<dyn NSObjectProtocol> = observer.as_ref();
    let observer_object: &AnyObject = protocol_object.as_ref();
    unsafe { center.removeObserver(observer_object) };
}

fn record_realtime_start_failure(pending: &Mutex<PendingUpdates>, wake: &GlobalIndexWakeSlot) {
    super::record_async_watcher_error(
        pending,
        wake,
        "macos_spotlight_realtime_updates_unavailable",
    );
}

fn new_local_computer_query() -> Result<Retained<NSMetadataQuery>, String> {
    let query = NSMetadataQuery::new();
    let query_string = NSString::from_str(SPOTLIGHT_MATCH_ALL_QUERY);
    let predicate = NSPredicate::predicateFromMetadataQueryString(&query_string)
        .ok_or_else(|| "macos_spotlight_query_predicate_invalid".to_string())?;
    query.setPredicate(Some(&predicate));
    let scope: &AnyObject = unsafe { NSMetadataQueryIndexedLocalComputerScope }.as_ref();
    let scopes: Retained<NSArray<AnyObject>> = NSArray::from_slice(&[scope]);
    unsafe { query.setSearchScopes(&scopes) };
    Ok(query)
}

fn full_disk_access_required() -> bool {
    let Some(home) = crate::platform::macos::file_provider::native_home_directory() else {
        return false;
    };
    [
        home.join("Library/Mail"),
        home.join("Library/Messages"),
        home.join("Library/Safari"),
        home.join("Library/Calendars"),
    ]
    .iter()
    .any(|path| match fs::symlink_metadata(path) {
        Err(error) => error.kind() == std::io::ErrorKind::PermissionDenied,
        Ok(metadata) if metadata.is_dir() => match fs::read_dir(path) {
            Err(error) => error.kind() == std::io::ErrorKind::PermissionDenied,
            Ok(_) => false,
        },
        Ok(_) => false,
    })
}

fn mounted_external_volume_probes() -> Vec<ExternalVolumeProbe> {
    let Some(root_device) = fs::symlink_metadata("/")
        .ok()
        .map(|metadata| metadata.dev())
    else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir("/Volumes") else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let root = entry.path();
            let metadata = fs::symlink_metadata(&root).ok()?;
            if !metadata.is_dir() || metadata.dev() == root_device {
                return None;
            }
            let has_readable_content = match fs::read_dir(&root) {
                Ok(mut children) => children.next().is_some(),
                Err(_) => false,
            };
            Some(ExternalVolumeProbe {
                root,
                has_readable_content,
            })
        })
        .collect()
}

#[expect(
    clippy::too_many_arguments,
    reason = "native metadata notifications keep callback state explicit for validation"
)]
fn collect_update_items(
    user_info: &objc2_foundation::NSDictionary<NSString, AnyObject>,
    key: &NSString,
    volume_id: &str,
    known_entries: &Arc<Mutex<KnownEntries>>,
    entries: &mut Vec<GlobalEntryInput>,
    stale_entry_ids: &mut Vec<String>,
    full_reconcile: &mut bool,
    removed: bool,
) {
    let Some(value) = user_info.objectForKey(key) else {
        return;
    };
    let Ok(items) = value.downcast::<NSArray>() else {
        return;
    };
    for index in 0..items.count() {
        let object = items.objectAtIndex(index);
        let path = path_from_object(&object).map(|path| normalize_path(&path));
        if removed {
            if let Some(entry_id) = path.as_deref().and_then(|path| {
                known_entries
                    .lock()
                    .ok()
                    .and_then(|known| known.current_entry_id_for_path(path))
            }) {
                stale_entry_ids.push(entry_id);
                continue;
            }
        }
        if let Some(entry) = object.downcast_ref::<NSMetadataItem>() {
            match metadata_item_to_entry(volume_id, entry.as_ref()) {
                Some(input) => {
                    match classify_spotlight_update(removed, has_path_identity(&input)) {
                        SpotlightUpdateAction::Upsert => entries.push(input),
                        SpotlightUpdateAction::Stale => stale_entry_ids.push(input.entry_id()),
                        SpotlightUpdateAction::Reconcile => *full_reconcile = true,
                    }
                }
                None => *full_reconcile = true,
            }
            continue;
        }
        if path.is_some() {
            *full_reconcile = true;
        }
    }
}

fn metadata_item_to_entry(volume_id: &str, object: &AnyObject) -> Option<GlobalEntryInput> {
    let item = object.downcast_ref::<NSMetadataItem>()?;
    let path = metadata_string(item, unsafe { NSMetadataItemPathKey })
        .or_else(|| metadata_url_path(item, unsafe { NSMetadataItemURLKey }))?;
    if path.trim().is_empty() {
        return None;
    }
    let path_buf = PathBuf::from(&path);
    let metadata = std::fs::symlink_metadata(&path_buf).ok();
    let is_directory = metadata.as_ref().is_some_and(std::fs::Metadata::is_dir);
    let semantics = crate::platform::macos::file_semantics::inspect(&path_buf);
    let mut file_attributes = 0;
    if semantics.is_package {
        file_attributes |= MACOS_FILE_ATTRIBUTE_PACKAGE;
    }
    match semantics.backing_kind {
        crate::platform::macos::MacCloudBacking::ICloud => {
            file_attributes |= MACOS_FILE_ATTRIBUTE_ICLOUD;
        }
        crate::platform::macos::MacCloudBacking::FileProvider => {
            file_attributes |= MACOS_FILE_ATTRIBUTE_FILE_PROVIDER;
        }
        crate::platform::macos::MacCloudBacking::Local
        | crate::platform::macos::MacCloudBacking::Unknown => {}
    }
    if !semantics.content_availability.is_local()
        && !matches!(
            semantics.content_availability,
            crate::platform::macos::MacContentAvailability::Unknown
        )
    {
        file_attributes |= MACOS_FILE_ATTRIBUTE_CLOUD_NOT_LOCAL;
    }
    match semantics.content_availability {
        crate::platform::macos::MacContentAvailability::Downloading => {
            file_attributes |= MACOS_FILE_ATTRIBUTE_CONTENT_DOWNLOADING;
        }
        crate::platform::macos::MacContentAvailability::Unknown
        | crate::platform::macos::MacContentAvailability::MetadataOnly => {
            file_attributes |= MACOS_FILE_ATTRIBUTE_CONTENT_UNKNOWN;
        }
        crate::platform::macos::MacContentAvailability::BoundaryReadable => {
            file_attributes |= MACOS_FILE_ATTRIBUTE_CONTENT_UNKNOWN;
        }
        crate::platform::macos::MacContentAvailability::Local
        | crate::platform::macos::MacContentAvailability::NotLocal => {}
    }
    let name = metadata_string(item, unsafe { NSMetadataItemFSNameKey })
        .or_else(|| {
            path_buf
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| path.clone());
    let extension = path_buf
        .extension()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let size = metadata_number(item, unsafe { NSMetadataItemFSSizeKey })
        .or_else(|| metadata.as_ref().map(|value| value.len() as i64))
        .unwrap_or_default();
    let platform_file_id = mac_file_identity(&path_buf, metadata.as_ref());
    let parent_platform_file_id = path_buf
        .parent()
        .map(|parent| mac_file_identity(parent, std::fs::symlink_metadata(parent).ok().as_ref()))
        .unwrap_or_default();
    Some(GlobalEntryInput {
        volume_id: volume_id.to_string(),
        platform_file_id,
        parent_platform_file_id,
        name,
        path,
        extension,
        is_directory,
        size: if is_directory { 0 } else { size },
        created_at_fs: metadata_date(item, unsafe { NSMetadataItemFSCreationDateKey }),
        modified_at_fs: metadata_date(item, unsafe { NSMetadataItemFSContentChangeDateKey }),
        file_attributes,
        is_hidden: path_buf
            .file_name()
            .is_some_and(|value| value.to_string_lossy().starts_with('.')),
        is_system: false,
        source_provider: PROVIDER_MACOS_SPOTLIGHT.to_string(),
        last_seen_at: crate::global_index::models::unix_now(),
    })
}

fn mac_file_identity(path: &Path, metadata: Option<&std::fs::Metadata>) -> String {
    metadata
        .map(|metadata| format!("mac:dev:{:x}:ino:{:x}", metadata.dev(), metadata.ino()))
        .unwrap_or_else(|| format!("path:{}", normalize_path(&path.to_string_lossy())))
}

fn has_path_identity(entry: &GlobalEntryInput) -> bool {
    entry.platform_file_id.starts_with("path:")
        || entry.parent_platform_file_id.starts_with("path:")
}

fn metadata_string(item: &NSMetadataItem, key: &NSString) -> Option<String> {
    item.valueForAttribute(key)
        .and_then(|value| value.downcast::<NSString>().ok())
        .map(|value| value.to_string())
}

fn metadata_url_path(item: &NSMetadataItem, key: &NSString) -> Option<String> {
    item.valueForAttribute(key)
        .and_then(|value| value.downcast::<NSURL>().ok())
        .and_then(|value| value.path())
        .map(|value| value.to_string())
}

fn metadata_number(item: &NSMetadataItem, key: &NSString) -> Option<i64> {
    item.valueForAttribute(key)
        .and_then(|value| value.downcast::<NSNumber>().ok())
        .map(|value| value.as_i64())
}

fn metadata_date(item: &NSMetadataItem, key: &NSString) -> Option<i64> {
    item.valueForAttribute(key)
        .and_then(|value| value.downcast::<NSDate>().ok())
        .map(|value| value.timeIntervalSince1970().max(0.0) as i64)
}

fn path_from_object(object: &AnyObject) -> Option<String> {
    if let Some(value) = object.downcast_ref::<NSString>() {
        return Some(value.to_string());
    }
    if let Some(value) = object.downcast_ref::<NSURL>() {
        return value.path().map(|path| path.to_string());
    }
    if let Some(value) = object.downcast_ref::<NSMetadataItem>() {
        return metadata_string(value, unsafe { NSMetadataItemPathKey })
            .or_else(|| metadata_url_path(value, unsafe { NSMetadataItemURLKey }));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::super::models::normalize_path;
    use super::{
        classify_spotlight_update, mac_file_identity, new_local_computer_query,
        record_realtime_start_failure, stream_local_computer_entries, SpotlightUpdateAction,
    };
    use std::path::Path;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    #[test]
    fn macos_paths_use_platform_normalization() {
        assert_eq!(normalize_path("/Users/test/"), "/Users/test");
    }

    #[test]
    fn macos_identity_prefers_device_and_inode_when_metadata_exists() {
        let path = Path::new(file!());
        let metadata = std::fs::symlink_metadata(path).expect("test source metadata");
        assert!(mac_file_identity(path, Some(&metadata)).starts_with("mac:dev:"));
    }

    #[test]
    fn macos_spotlight_notifications_map_add_update_delete_and_fallbacks() {
        assert_eq!(
            classify_spotlight_update(false, false),
            SpotlightUpdateAction::Upsert
        );
        assert_eq!(
            classify_spotlight_update(true, false),
            SpotlightUpdateAction::Stale
        );
        assert_eq!(
            classify_spotlight_update(false, true),
            SpotlightUpdateAction::Reconcile
        );
        assert_eq!(
            classify_spotlight_update(true, true),
            SpotlightUpdateAction::Reconcile
        );
    }

    #[test]
    fn macos_spotlight_query_uses_a_supported_metadata_predicate() {
        objc2::rc::autoreleasepool(|_| {
            new_local_computer_query().expect("construct Spotlight metadata query");
        });
    }

    #[test]
    fn realtime_start_failure_uses_shared_error_and_wake_invariant() {
        let pending = Arc::new(Mutex::new(super::super::PendingUpdates::default()));
        let wake = Arc::new(crate::global_index::wake::GlobalIndexWakeSlot::default());

        record_realtime_start_failure(&pending, &wake);

        assert_eq!(wake.snapshot().notifications, 1);
        assert_eq!(
            pending
                .lock()
                .expect("pending updates")
                .last_error
                .as_deref(),
            Some("macos_spotlight_realtime_updates_unavailable")
        );
    }

    #[test]
    fn macos_spotlight_cancellation_is_observed_before_batching() {
        let cancel = AtomicBool::new(true);
        let mut batches = 0;
        let result = stream_local_computer_entries("test", &cancel, |_| {
            batches += 1;
            Ok(())
        });
        match result {
            Err(error) => assert!(
                error == "macos_spotlight_query_paused"
                    || error == "macos_spotlight_query_unavailable",
                "unexpected cancellation result: {error}"
            ),
            Ok(summary) => assert_eq!(summary.processed, 0),
        }
        assert_eq!(batches, 0);
    }

    #[test]
    #[ignore = "requires a live macOS Spotlight database"]
    fn macos_spotlight_live_initial_collection_is_batched() {
        let cancel = AtomicBool::new(false);
        let mut batches = Vec::new();
        let summary = stream_local_computer_entries("test", &cancel, |batch| {
            assert!(!batch.is_empty());
            assert!(batch.len() <= 512);
            batches.push(batch.len());
            Ok(())
        })
        .expect("Spotlight initial collection");
        assert_eq!(summary.processed, batches.iter().sum::<usize>());
        assert!(summary.processed > 0, "Spotlight returned no local results");
    }
}
