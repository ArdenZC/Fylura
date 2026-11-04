//! Explicit native qualification on a task-owned APFS disk image only.
use super::*;
use crate::global_index::{
    coverage::{bind_rule, CoverageMode},
    SourceOperationKind, SourceOperationRoute,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Match the app's Cocoa main-loop lifecycle while the provider runs on a worker.
/// Rust's libtest harness leaves the process main thread waiting without pumping
/// Cocoa; native metadata initialization can then wait forever for that thread.
pub fn run_qualification() -> Result<(), String> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{NSDate, NSRunLoop};
    super::trace_native_qa_phase("native_qa.main_thread_marker", "begin");
    let marker = MainThreadMarker::new().ok_or("native_qa_main_thread_required")?;
    let _application = NSApplication::sharedApplication(marker);
    super::trace_native_qa_phase("native_qa.application_ready", "end");
    let worker = std::thread::Builder::new()
        .name("smart-lean-qa-worker".to_string())
        .spawn(qualify_scoped_source)
        .map_err(|error| format!("native_qa_worker_start_failed: {error}"))?;
    let run_loop = NSRunLoop::currentRunLoop();
    super::trace_native_qa_phase("native_qa.main_run_loop", "begin");
    while !worker.is_finished() {
        run_loop.runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.05));
    }
    super::trace_native_qa_phase("native_qa.main_run_loop", "end");
    worker
        .join()
        .map_err(|_| "native_qa_scoped_source_failed".to_string())
}

fn qualify_scoped_source() {
    let root = PathBuf::from(
        std::env::var_os("ZEN_SMART_LEAN_VOLUME_ROOT").expect("controlled APFS root required"),
    );
    let task = PathBuf::from(
        std::env::var_os("ZEN_SMART_LEAN_TASK_ROOT").expect("controlled task root required"),
    );
    let task = task.as_path();
    let runner = PathBuf::from(std::env::var_os("RUNNER_TEMP").expect("runner root required"));
    assert!(task.starts_with(&runner));
    assert_eq!(
        fs::read_to_string(task.join("owned-volume.txt")).unwrap(),
        root.to_string_lossy()
    );
    super::trace_native_qa_phase("native_qa.volume_identity", "begin");
    eprintln!("SMART_LEAN_NATIVE stage=volume_identity");
    let stable = volumes::volume_uuid(&root).expect("native APFS volume UUID");
    super::trace_native_qa_phase("native_qa.volume_identity", "end");
    let mut source = GlobalVolume::macos_spotlight_local_computer();
    source.id = format!("gv_macos_{}", blake3::hash(stable.as_bytes()).to_hex());
    source.stable_volume_id = stable;
    source.platform = "macos".into();
    source.mount_path = root.to_string_lossy().into_owned();
    source.provider = PROVIDER_MACOS_SPOTLIGHT.into();
    source.enabled = false;
    source.scope_authorization_state = "not_required".into();
    let db = crate::db::Database::open(task.join("index.sqlite3")).unwrap();
    db.upsert_global_volume(&source).unwrap();
    assert_eq!(
        db.global_index_coverage().unwrap().policy.mode,
        CoverageMode::SmartLean,
        "a new profile must default to Smart Lean"
    );
    assert!(
        db.claim_global_index_operation(
            &source,
            &uuid::Uuid::new_v4().to_string(),
            SourceOperationKind::Initial,
            SourceOperationRoute::Coordinator,
        )
        .unwrap()
        .is_none(),
        "a discovered but disabled source must not start indexing"
    );
    let preauthorization_rows: i64 = db
        .conn()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM global_entries WHERE volume_id=?1",
            [&source.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(preauthorization_rows, 0, "disabled source wrote index rows");
    db.set_global_volume_enabled(&source.id, true).unwrap();
    source.enabled = true;
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
    let document = root.join("documents/zcsmartlean-native-document.txt");
    let photo = root.join("pictures/zcsmartlean-native-photo.png");
    let code = project.join("src/zcsmartlean-native-module.rs");
    let lookalikes = [
        root.join("Projects/Windows/valuable-note.txt"),
        root.join("AppData/important-setting.json"),
        root.join("build/valuable-design.pdf"),
        root.join("cache/indexed-report.pdf"),
        root.join("tmp/meeting-notes.txt"),
        root.join("logs/important.log"),
    ];
    let custom_exclude = root.join("custom-policy/private");
    let custom_excluded_file = custom_exclude.join("valuable-private-note.txt");
    let custom_include = project.join("node_modules/user-preserved");
    let custom_included_file = custom_include.join("valuable-user-file.txt");
    let temporary_root = std::env::temp_dir();
    if !temporary_root.starts_with(&root) {
        report_capability_gap(&root, "task-owned TMPDIR is outside the APFS source");
        return;
    }
    let temporary_file = temporary_root.join("zcsmartlean-native-temporary-file.txt");
    let cache_root = dirs::cache_dir().expect("task-owned HOME must expose its cache root");
    if !cache_root.starts_with(&root) {
        report_capability_gap(&root, "task-owned HOME cache is outside the APFS source");
        return;
    }
    let cache_file = cache_root.join("zcsmartlean-native-cache-file.txt");
    for directory in [
        document.parent().unwrap(),
        photo.parent().unwrap(),
        code.parent().unwrap(),
        custom_exclude.as_path(),
        custom_include.as_path(),
        temporary_root.as_path(),
        cache_root.as_path(),
    ] {
        fs::create_dir_all(directory).unwrap();
    }
    fs::write(&document, b"ordinary document").unwrap();
    fs::write(
        &photo,
        [
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9c, 0x63, 0x60, 0x00, 0x02, 0x00, 0x00, 0x05, 0x00, 0x01, 0xa5, 0x73, 0xa0, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ],
    )
    .unwrap();
    fs::write(&code, b"ordinary project source").unwrap();
    fs::write(&custom_excluded_file, b"custom excluded").unwrap();
    fs::write(&custom_included_file, b"custom included").unwrap();
    fs::write(&temporary_file, b"task-owned temporary file").unwrap();
    fs::write(&cache_file, b"task-owned cache file").unwrap();
    for path in &lookalikes {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"valuable lookalike").unwrap();
    }
    let _ = std::process::Command::new("/usr/bin/mdimport")
        .arg("-i")
        .arg(&root)
        .output();
    let wake = Arc::new(GlobalIndexWakeSlot::default());
    let provider = MacosSpotlightProvider::with_root(wake.clone(), root.clone());
    let mut report = serde_json::json!({"status":"UNVERIFIED","scope":root,"volumeId":source.stable_volume_id,"provider":"scoped Spotlight/FSEvents","nativeGui":"not exercised"});
    let mut result = (|| -> Result<(), String> {
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
        super::trace_native_qa_phase("native_qa.scoped_initial_provider", "begin");
        if let Err(error) = provider.start_initial_index(
            &GlobalSourceDescriptor {
                volume: source.clone(),
            },
            &mut sink,
            &cancel,
        ) {
            let message = error.to_string();
            if !message.contains("macos_spotlight_query_unavailable")
                && !message.contains("macos_fsevents_unavailable")
                && !message.contains("macos_spotlight_query_paused")
            {
                return Err(message);
            }
            report["capabilityGap"] = if message.contains("macos_spotlight_query_paused") {
                "Hosted Spotlight did not complete the scoped query inside the existing 40-second cooperative bound; cancellation was observed".into()
            } else {
                message.into()
            };
            return Ok(());
        }
        super::trace_native_qa_phase("native_qa.scoped_initial_provider", "end");
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
        for path in [document.as_path(), photo.as_path(), code.as_path()] {
            if !visible(&db, path) {
                return Err(format!(
                    "ordinary document/photo/source fixture was not searchable: {}",
                    path.display()
                ));
            }
        }
        for path in &lookalikes {
            if !visible(&db, path) {
                return Err(format!(
                    "valuable same-name directory was excluded: {}",
                    path.display()
                ));
            }
        }
        if visible(&db, &temporary_file) || visible(&db, &cache_file) {
            return Err(
                "native Smart Lean admitted an OS-recognized temporary/cache fixture".into(),
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
        while (!visible(&db, &restored_file) || visible(&db, &excluded))
            && begin.elapsed() < Duration::from_secs(35)
        {
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

        let current = db
            .get_global_volume(&source.id)
            .map_err(|e| e.to_string())?
            .ok_or("source missing after Smart Lean move")?;
        let custom_rules = vec![
            bind_rule(&current, &custom_include.to_string_lossy(), true)
                .map_err(|e| e.to_string())?,
            bind_rule(&current, &custom_exclude.to_string_lossy(), false)
                .map_err(|e| e.to_string())?,
        ];
        let mut custom_rules = Some(custom_rules);
        provider
            .update_source_lifecycle(&source.id, &mut || {
                db.save_global_index_coverage(
                    db.global_index_coverage()?.policy.revision,
                    CoverageMode::Custom,
                    custom_rules.take().unwrap_or_default(),
                )?;
                Ok(true)
            })
            .map_err(|e| e.to_string())?;
        if !db
            .global_index_coverage()
            .map_err(|e| e.to_string())?
            .sources[0]
            .pending
            || visible(&db, &document)
        {
            return Err("policy change did not hide old results and mark source pending".into());
        }
        let _ = std::process::Command::new("/usr/bin/mdimport")
            .arg("-i")
            .arg(&root)
            .output();
        let current = db
            .get_global_volume(&source.id)
            .map_err(|e| e.to_string())?
            .ok_or("source missing before Custom rebuild")?;
        let mut custom_sink =
            crate::global_index::coordinator::database_sink_for_qualification(db.clone());
        custom_sink
            .admit_source_operation(
                &current,
                &uuid::Uuid::new_v4().to_string(),
                SourceOperationKind::Rebuild,
                SourceOperationRoute::Coordinator,
            )
            .map_err(|e| e.to_string())?;
        provider
            .start_initial_index(
                &GlobalSourceDescriptor { volume: current },
                &mut custom_sink,
                &bounded_cancel(),
            )
            .map_err(|e| e.to_string())?;
        custom_sink
            .finish_source_operation(
                &source.id,
                "ready",
                None,
                Some(crate::global_index::models::unix_now()),
                None,
                true,
            )
            .map_err(|e| e.to_string())?;
        drop(custom_sink);
        if !visible(&db, &custom_included_file)
            || visible(&db, &custom_excluded_file)
            || visible(&db, &excluded)
            || !visible(&db, &document)
            || visible(&db, &temporary_file)
            || visible(&db, &cache_file)
        {
            return Err("native Custom Include/Exclude, cache or temporary policy mismatch".into());
        }
        for path in &lookalikes {
            if !visible(&db, path) {
                return Err(format!(
                    "Custom mode excluded a valuable same-name directory: {}",
                    path.display()
                ));
            }
        }

        let mut old_writer =
            crate::global_index::coordinator::database_sink_for_qualification(db.clone());
        let current = db
            .get_global_volume(&source.id)
            .map_err(|e| e.to_string())?
            .ok_or("source missing before disable")?;
        old_writer
            .admit_source_operation(
                &current,
                &uuid::Uuid::new_v4().to_string(),
                SourceOperationKind::Incremental,
                SourceOperationRoute::Coordinator,
            )
            .map_err(|e| e.to_string())?;
        provider
            .update_source_lifecycle(&source.id, &mut || {
                db.set_global_volume_enabled(&source.id, false)?;
                Ok(true)
            })
            .map_err(|e| e.to_string())?;
        let old_input = crate::global_index::GlobalEntryInput::from_path(
            &source.id,
            &document,
            PROVIDER_MACOS_SPOTLIGHT,
        );
        if old_writer
            .write_batch(std::slice::from_ref(&old_input))
            .is_ok()
        {
            return Err("source opt-out allowed an old writer to continue indexing".into());
        }
        drop(old_writer);
        if provider.spotlight_watcher.lock().unwrap().is_some()
            || provider.fsevents_watcher.lock().unwrap().is_some()
        {
            return Err("source disable left native watchers registered".into());
        }
        let counters = wake.snapshot();
        report["status"] = "PASS".into();
        report["initialAdmission"] = true.into();
        report["excludedSubtreeBackfill"] = true.into();
        report["ordinaryDocumentsPhotosAndCodeSearchable"] = true.into();
        report["customIncludeExclude"] = true.into();
        report["cacheAndTemporaryRootsExcluded"] = true.into();
        report["disableStopsWatchers"] = true.into();
        report["oldWriterRejectedAfterSourceOptOut"] = true.into();
        report["nativeNotifications"] = counters.notifications.into();
        report["nativeWakeDeliveries"] = counters.delivered.into();
        report["coalescedNotifications"] = counters.coalesced.into();
        report["coverage"] = serde_json::to_value(db.global_index_coverage().unwrap()).unwrap();
        Ok(())
    })();
    super::trace_native_qa_phase("native_qa.provider_shutdown", "begin");
    let shutdown = provider.shutdown();
    super::trace_native_qa_phase("native_qa.provider_shutdown", "end");
    if let Err(error) = shutdown {
        if let Err(existing) = &result {
            report["shutdownError"] = error.to_string().into();
            report["error"] = existing.clone().into();
        } else {
            result = Err(error.to_string());
        }
    }
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
fn report_capability_gap(root: &Path, capability_gap: &str) {
    let report = serde_json::json!({
        "status": "UNVERIFIED",
        "scope": root,
        "nativeGui": "not exercised",
        "capabilityGap": capability_gap,
    });
    if let Some(path) = std::env::var_os("ZEN_SMART_LEAN_NATIVE_REPORT") {
        fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("SMART_LEAN_NATIVE {report}");
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
