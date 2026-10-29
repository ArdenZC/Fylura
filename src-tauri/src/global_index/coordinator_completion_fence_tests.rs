// A source may be disabled after provider state reporting but before the
// Coordinator's terminal write. Fencing must preserve the disable and allow
// the remaining source operations to continue.
struct CompletionFenceProvider {
    sources: Vec<GlobalSourceDescriptor>,
    control: Database,
}
impl GlobalIndexProvider for CompletionFenceProvider {
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        Ok(self.sources.clone())
    }
    fn start_initial_index(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        _: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        sink.checkpoint(&source.volume.id, Some("fixture-journal"), Some("43"))?;
        if source.volume.id == "completion-fence-a" {
            sink.set_source_state(
                &source.volume.id,
                INDEX_STATUS_WATCHER_UNAVAILABLE,
                Some("injected watcher failure"),
            )?;
            self.control
                .set_global_volume_enabled(&source.volume.id, false)?;
        }
        Ok(())
    }
    fn resume_incremental_sync(
        &self,
        source: &GlobalSourceDescriptor,
        sink: &mut dyn GlobalIndexSink,
        cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.start_initial_index(source, sink, cancel)
    }
    fn pause(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }
    fn status(&self) -> Result<String, GlobalIndexError> {
        Ok("fixture".into())
    }
    fn shutdown(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }
}
#[test]
fn disable_after_provider_health_report_fences_completion_without_ending_other_sources() {
    let path = TestDatabasePath::new();
    let db = Database::open(&path.0).unwrap();
    let mut sources = Vec::new();
    for id in ["completion-fence-a", "completion-fence-b"] {
        let mut volume = crate::global_index::tests::test_volume();
        volume.mount_path = format!("fixture-{id}");
        volume.id = id.into();
        volume.stable_volume_id = format!("stable-{id}");
        db.upsert_global_volume(&volume).unwrap();
        sources.push(GlobalSourceDescriptor { volume });
    }
    // Stable list order is mount_path; force the fenced source to run first.
    let provider = CompletionFenceProvider {
        sources,
        control: Database::open(&path.0).unwrap(),
    };
    run_index_cycle(
        &provider,
        &db,
        &Arc::new(AtomicBool::new(false)),
        &GlobalIndexWakeSlot::default(),
    )
    .expect("fenced completion must not terminate the entire worker cycle");
    let disabled = db.get_global_volume("completion-fence-a").unwrap().unwrap();
    assert!(!disabled.enabled);
    assert_eq!(
        disabled.last_full_index_at, None,
        "stale owner cannot save a successful full-index timestamp"
    );
    assert_eq!(
        db.global_index_operation_state(&disabled.id)
            .unwrap()
            .unwrap()
            .owner_id,
        None
    );
    let other = db.get_global_volume("completion-fence-b").unwrap().unwrap();
    assert_eq!(other.index_status, INDEX_STATUS_READY);
    assert!(other.last_full_index_at.is_some());
    assert_eq!(other.journal_cursor.as_deref(), Some("43"));
    assert_eq!(
        db.global_index_operation_state(&other.id)
            .unwrap()
            .unwrap()
            .owner_id,
        None
    );
}
