//! Explicit native qualification on a task-owned APFS disk image only.
use super::*;
use crate::global_index::{SourceOperationKind, SourceOperationRoute};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires the isolated APFS volume created by qualifySmartLeanMacos.sh"]
fn scoped_spotlight_fsevents_qualification() {
    let root = PathBuf::from(
        std::env::var_os("ZEN_SMART_LEAN_VOLUME_ROOT").expect("controlled APFS root required"),
    );
    let task = root.parent().expect("task boundary");
    let runner = PathBuf::from(std::env::var_os("RUNNER_TEMP").expect("runner root required"));
    assert!(task.starts_with(&runner));
    assert_eq!(
        fs::read_to_string(task.join("owned-volume.txt")).unwrap(),
        root.to_string_lossy()
    );
    eprintln!("SMART_LEAN_NATIVE stage=volume_identity");
    let stable = volumes::volume_uuid(&root).expect("native APFS volume UUID");
    let mut source = crate::global_index::tests::test_volume();
    source.id = format!("gv_macos_{}", blake3::hash(stable.as_bytes()).to_hex());
    source.stable_volume_id = stable;
    source.platform = "macos".into();
    source.mount_path = root.to_string_lossy().into_owned();
    source.provider = PROVIDER_MACOS_SPOTLIGHT.into();
    source.enabled = true;
    source.scope_authorization_state = "not_required".into();
    let db = crate::db::Database::open(task.join("index.sqlite3")).unwrap();
    db.upsert_global_volume(&source).unwrap();
    let project = root.join("project");
    let lib = project.join("node_modules/lib");
    let docs = project.join("src");
    fs::create_dir_all(&lib).unwrap();
    fs::create_dir_all(&docs).unwrap();
    fs::write(project.join("package.json"), b"{}").unwrap();
    fs::write(project.join("node_modules/.package-lock.json"), b"{}").unwrap();
    let included = docs.join("zcsmartlean-native-source.txt");
    let excluded = lib.join("zcsmartlean-native-dependency.txt");
    fs::write(&included, b"controlled source").unwrap();
    fs::write(&excluded, b"controlled dependency").unwrap();
    let _ = std::process::Command::new("/usr/bin/mdimport")
        .arg("-i")
        .arg(&root)
        .output();
    let wake = Arc::new(GlobalIndexWakeSlot::default());
    let provider = MacosSpotlightProvider::with_root(wake.clone(), root.clone());
    let mut report = serde_json::json!({"status":"UNVERIFIED","scope":root,"volumeId":source.stable_volume_id,"provider":"scoped Spotlight/FSEvents","nativeGui":"not exercised"});
    let result = (|| -> Result<(), String> {
        let mut sink =
            crate::global_index::coordinator::database_sink_for_qualification(db.clone());
        sink.admit_source_operation(
            &source,
            &uuid::Uuid::new_v4().to_string(),
            SourceOperationKind::Initial,
            SourceOperationRoute::Coordinator,
        )
        .map_err(|e| e.to_string())?;
        let cancel = bounded_cancel();
        eprintln!("SMART_LEAN_NATIVE stage=scoped_initial_provider");
        if let Err(error) = provider.start_initial_index(
            &GlobalSourceDescriptor {
                volume: source.clone(),
            },
            &mut sink,
            &cancel,
        ) {
            report["capabilityGap"] = error.to_string().into();
            return Ok(());
        }
        if !visible(&db, &included) {
            report["capabilityGap"] =
                "Hosted Spotlight did not gather the controlled APFS fixture".into();
            report["sourceStatus"] = db
                .get_global_volume(&source.id)
                .unwrap()
                .unwrap()
                .index_status
                .into();
            return Ok(());
        }
        if visible(&db, &excluded) {
            return Err(
                "native Spotlight adapter admitted a proven dependency under Smart Lean".into(),
            );
        }
        sink.finish_source_operation(
            &source.id,
            "ready",
            None,
            Some(crate::global_index::models::unix_now()),
            None,
            false,
        )
        .map_err(|e| e.to_string())?;
        drop(sink);
        let restored = docs.join("restored-lib");
        fs::rename(&lib, &restored).map_err(|e| e.to_string())?;
        let restored_file = restored.join(excluded.file_name().unwrap());
        let _ = std::process::Command::new("/usr/bin/mdimport")
            .arg("-i")
            .arg(&root)
            .output();
        let begin = Instant::now();
        while !visible(&db, &restored_file) && begin.elapsed() < Duration::from_secs(35) {
            let _ = wake.wait(&AtomicBool::new(false), Some(Duration::from_secs(1)));
            let current = db.get_global_volume(&source.id).unwrap().unwrap();
            let mut sink =
                crate::global_index::coordinator::database_sink_for_qualification(db.clone());
            sink.admit_source_operation(
                &current,
                &uuid::Uuid::new_v4().to_string(),
                SourceOperationKind::Incremental,
                SourceOperationRoute::Coordinator,
            )
            .map_err(|e| e.to_string())?;
            provider
                .resume_incremental_sync(
                    &GlobalSourceDescriptor { volume: current },
                    &mut sink,
                    &bounded_cancel(),
                )
                .map_err(|e| e.to_string())?;
            sink.finish_source_operation(
                &source.id,
                "ready",
                None,
                None,
                Some(crate::global_index::models::unix_now()),
                false,
            )
            .map_err(|e| e.to_string())?;
        }
        if !visible(&db, &restored_file) {
            return Err(
                "scoped Spotlight/FSEvents did not backfill the moved dependency subtree".into(),
            );
        }
        if visible(&db, &excluded) {
            return Err("old excluded path survived scoped reconciliation".into());
        }
        provider
            .update_source_lifecycle(&source.id, &mut || {
                db.set_global_volume_enabled(&source.id, false)?;
                Ok(true)
            })
            .map_err(|e| e.to_string())?;
        if provider.spotlight_watcher.lock().unwrap().is_some()
            || provider.fsevents_watcher.lock().unwrap().is_some()
        {
            return Err("source disable left native watchers registered".into());
        }
        let counters = wake.snapshot();
        report["status"] = "PASS".into();
        report["initialAdmission"] = true.into();
        report["excludedSubtreeBackfill"] = true.into();
        report["disableStopsWatchers"] = true.into();
        report["nativeNotifications"] = counters.notifications.into();
        report["nativeWakeDeliveries"] = counters.delivered.into();
        report["coalescedNotifications"] = counters.coalesced.into();
        report["coverage"] = serde_json::to_value(db.global_index_coverage().unwrap()).unwrap();
        Ok(())
    })();
    let _ = provider.shutdown();
    if let Err(error) = &result {
        report["status"] = "FAIL".into();
        report["error"] = error.clone().into();
    }
    if let Some(path) = std::env::var_os("ZEN_SMART_LEAN_NATIVE_REPORT") {
        fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("SMART_LEAN_NATIVE {report}");
    result.unwrap();
}
fn visible(db: &crate::db::Database, path: &Path) -> bool {
    db.search_global_entries(&path.file_name().unwrap().to_string_lossy(), 25, 0)
        .unwrap()
        .iter()
        .any(|e| Path::new(&e.path) == path)
}
fn bounded_cancel() -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let timeout = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(40));
        timeout.store(true, Ordering::Release);
    });
    cancel
}
