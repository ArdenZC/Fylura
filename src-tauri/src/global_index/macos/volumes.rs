//! One explicitly enabled source and native watcher lifecycle per physical volume.
//! Discovery reads mount identity only; it never enumerates files on those disks.
use super::*;
use crate::global_index::{INDEX_STATUS_DISCOVERED, SCOPE_AUTHORIZATION_NOT_REQUIRED};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
pub(crate) struct MacosVolumeProvider {
    providers: Mutex<HashMap<String, Arc<MacosSpotlightProvider>>>,
    wake: Arc<GlobalIndexWakeSlot>,
}
impl MacosVolumeProvider {
    pub(crate) fn with_wake(wake: Arc<GlobalIndexWakeSlot>) -> Self {
        Self {
            providers: Mutex::new(HashMap::new()),
            wake,
        }
    }
    fn source_provider(
        &self,
        source: &GlobalSourceDescriptor,
    ) -> Result<Arc<MacosSpotlightProvider>, GlobalIndexError> {
        let physical = if source.volume.mount_path == "/" {
            Path::new("/System/Volumes/Data")
        } else {
            Path::new(&source.volume.mount_path)
        };
        if !source.volume.enabled
            || volume_uuid(physical).as_deref() != Some(source.volume.stable_volume_id.as_str())
        {
            return Err(GlobalIndexError::PermissionRequired(
                "macos_volume_identity_changed_or_disabled".into(),
            ));
        }
        let mut providers = self
            .providers
            .lock()
            .map_err(|_| GlobalIndexError::Provider("macos_volume_provider_poisoned".into()))?;
        if let Some(old) = providers
            .get(&source.volume.id)
            .filter(|p| p.root != Path::new(&source.volume.mount_path))
            .cloned()
        {
            old.shutdown()?;
            providers.remove(&source.volume.id);
        }
        Ok(providers
            .entry(source.volume.id.clone())
            .or_insert_with(|| {
                Arc::new(MacosSpotlightProvider::with_root(
                    self.wake.clone(),
                    PathBuf::from(&source.volume.mount_path),
                ))
            })
            .clone())
    }
    fn all(&self) -> Vec<Arc<MacosSpotlightProvider>> {
        self.providers
            .lock()
            .map(|p| p.values().cloned().collect())
            .unwrap_or_default()
    }
}
pub(super) fn source_id_matches(id: &str, root: &Path) -> bool {
    if !id.starts_with("gv_macos_") {
        return true;
    }
    let physical = if root == Path::new("/") {
        Path::new("/System/Volumes/Data")
    } else {
        root
    };
    volume_uuid(physical)
        .is_some_and(|uuid| id == format!("gv_macos_{}", blake3::hash(uuid.as_bytes()).to_hex()))
}
fn volume_uuid(path: &Path) -> Option<String> {
    use objc2_foundation::{NSArray, NSString, NSURLVolumeUUIDStringKey, NSURL};
    objc2::rc::autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?));
        let key = unsafe { NSURLVolumeUUIDStringKey };
        let values = url
            .resourceValuesForKeys_error(&NSArray::from_slice(&[key]))
            .ok()?;
        let uuid = values
            .objectForKey(key)?
            .downcast::<NSString>()
            .ok()?
            .to_string();
        (!uuid.is_empty()).then(|| format!("macos:volume-uuid:{uuid}"))
    })
}
fn discover_volumes() -> Vec<GlobalSourceDescriptor> {
    let mut roots = vec![(PathBuf::from("/"), PathBuf::from("/System/Volumes/Data"))];
    if let Ok(mounts) = std::fs::read_dir("/Volumes") {
        roots.extend(mounts.filter_map(Result::ok).filter_map(|m| {
            // Alias volumes and network mounts are deliberately not authorizable.
            let p = m.path();
            let metadata = std::fs::symlink_metadata(&p).ok()?;
            (metadata.is_dir() && !metadata.file_type().is_symlink()).then_some((p.clone(), p))
        }));
    }
    let mut identities = HashSet::new();
    roots
        .into_iter()
        .filter_map(|(root, physical)| {
            let semantics = crate::platform::macos::volume::inspect(&physical);
            if semantics.is_local != Some(true) {
                return None;
            }
            let stable = volume_uuid(&physical)?;
            if !identities.insert(stable.clone()) {
                return None;
            }
            let mut volume = GlobalVolume::macos_spotlight_local_computer();
            volume.id = format!("gv_macos_{}", blake3::hash(stable.as_bytes()).to_hex());
            volume.stable_volume_id = stable;
            volume.display_name = if root == Path::new("/") {
                "Mac startup disk".into()
            } else {
                root.file_name()?.to_string_lossy().into_owned()
            };
            volume.mount_path = root.to_string_lossy().into_owned();
            volume.filesystem_type = semantics
                .filesystem_type
                .unwrap_or_else(|| "unknown".into());
            volume.drive_kind = if semantics.is_removable == Some(true) {
                "removable"
            } else {
                "fixed"
            }
            .into();
            volume.enabled = false;
            // Source enable is the explicit per-volume opt-in on macOS. OS access
            // (TCC/FDA) remains independent and cannot be granted by a coverage rule.
            volume.scope_authorization_state = SCOPE_AUTHORIZATION_NOT_REQUIRED.into();
            volume.index_status = INDEX_STATUS_DISCOVERED.into();
            Some(GlobalSourceDescriptor { volume })
        })
        .collect()
}
impl GlobalIndexProvider for MacosVolumeProvider {
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        Ok(discover_volumes())
    }
    fn start_initial_index(
        &self,
        s: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        c: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.source_provider(s)?.start_initial_index(s, sink, c)
    }
    fn resume_incremental_sync(
        &self,
        s: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        c: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.source_provider(s)?.resume_incremental_sync(s, sink, c)
    }
    fn requires_background_admission(&self, s: &GlobalSourceDescriptor) -> bool {
        self.source_provider(s)
            .map(|p| p.requires_background_admission(s))
            .unwrap_or(true)
    }
    fn update_source_lifecycle(
        &self,
        id: &str,
        update: &mut dyn FnMut() -> Result<bool, GlobalIndexError>,
    ) -> Result<(), GlobalIndexError> {
        let p = self
            .providers
            .lock()
            .map_err(|_| GlobalIndexError::Provider("macos_volume_provider_poisoned".into()))?
            .get(id)
            .cloned();
        match p {
            Some(p) => p.update_source_lifecycle(id, update),
            None => update().map(|_| ()),
        }
    }
    fn source_enabled_changed(&self, id: &str, enabled: bool) {
        if !enabled {
            if let Some(provider) = self.providers.lock().ok().and_then(|p| p.get(id).cloned()) {
                let _ = provider.pause();
            }
        }
    }
    fn pause(&self) -> Result<(), GlobalIndexError> {
        for p in self.all() {
            p.pause()?;
        }
        Ok(())
    }
    fn shutdown(&self) -> Result<(), GlobalIndexError> {
        for p in self.all() {
            p.shutdown()?;
        }
        Ok(())
    }
    fn status(&self) -> Result<String, GlobalIndexError> {
        Ok(format!(
            "{PROVIDER_MACOS_SPOTLIGHT}+{PROVIDER_MACOS_FSEVENTS_RECONCILE}"
        ))
    }
    fn audit_source_topology(
        &self,
    ) -> Result<Option<Vec<GlobalSourceDescriptor>>, GlobalIndexError> {
        Ok(Some(discover_volumes()))
    }
    fn topology_audit_interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boot_scope_never_swallows_other_mounts_or_name_lookalikes() {
        assert!(spotlight::path_in_source(
            Path::new("/"),
            Path::new("/Users/me/Design/build/logo.ai")
        ));
        assert!(!spotlight::path_in_source(
            Path::new("/"),
            Path::new("/Volumes/External/private.txt")
        ));
        assert!(!spotlight::path_in_source(
            Path::new("/Volumes/Work"),
            Path::new("/Volumes/Work2/private.txt")
        ));
        assert!(spotlight::path_in_source(
            Path::new("/Volumes/Work"),
            Path::new("/Volumes/Work/资料/README.md")
        ));
        assert!(!spotlight::path_in_source(
            Path::new("/Volumes/Work"),
            Path::new("/Volumes/Work/../Other/private.txt")
        ));
    }
}
