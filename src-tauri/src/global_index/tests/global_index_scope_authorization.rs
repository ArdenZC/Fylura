use crate::db::Database;
use crate::global_index::coordinator::{
    run_index_cycle_for_test, GlobalIndexError, GlobalIndexProvider, GlobalIndexSink,
};
use crate::global_index::models::*;
use crate::global_index::wake::GlobalIndexWakeSlot;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct CountingProvider {
    sources: Mutex<Vec<GlobalSourceDescriptor>>,
    operations: AtomicUsize,
}

impl GlobalIndexProvider for CountingProvider {
    fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
        Ok(self.sources.lock().unwrap().clone())
    }

    fn start_initial_index(
        &self,
        _source: &GlobalSourceDescriptor,
        _sink: &mut dyn GlobalIndexSink,
        _cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.operations.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn resume_incremental_sync(
        &self,
        _source: &GlobalSourceDescriptor,
        _sink: &mut dyn GlobalIndexSink,
        _cancel: &AtomicBool,
    ) -> Result<(), GlobalIndexError> {
        self.operations.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn pause(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }
    fn status(&self) -> Result<String, GlobalIndexError> {
        Ok("test".into())
    }
    fn shutdown(&self) -> Result<(), GlobalIndexError> {
        Ok(())
    }
}

fn source(id: &str, authorization: &str, enabled: bool) -> GlobalSourceDescriptor {
    let mut volume = super::test_volume();
    volume.id = id.to_string();
    volume.stable_volume_id = format!("stable-{id}");
    volume.mount_path = format!(r"{id}:\");
    volume.scope_authorization_state = authorization.to_string();
    volume.enabled = enabled;
    volume.index_status = INDEX_STATUS_DISCOVERED.to_string();
    volume.last_full_index_at = None;
    volume.journal_id = None;
    volume.journal_cursor = None;
    GlobalSourceDescriptor { volume }
}

fn cycle(provider: &CountingProvider, db: &Database, cancel: &AtomicBool) {
    run_index_cycle_for_test(
        provider,
        db,
        &Arc::new(AtomicBool::new(cancel.load(Ordering::Acquire))),
        &GlobalIndexWakeSlot::default(),
    )
    .expect("simulated global index cycle");
}

#[test]
fn windows_sources_require_authorization_across_discovery_restart_pause_resume_and_rebuild() {
    let path = super::test_db_path();
    let db = Database::open(&path).expect("open isolated test database");
    assert!(db.list_global_volumes().unwrap().is_empty());

    let first = source("vol-a", SCOPE_AUTHORIZATION_NOT_AUTHORIZED, false);
    let second = source("vol-b", SCOPE_AUTHORIZATION_NOT_AUTHORIZED, false);
    let provider = CountingProvider {
        sources: Mutex::new(vec![first.clone(), second.clone()]),
        operations: AtomicUsize::new(0),
    };
    let cancel = AtomicBool::new(false);

    cycle(&provider, &db, &cancel);
    assert_eq!(provider.operations.load(Ordering::Acquire), 0);
    assert_eq!(db.list_global_volumes().unwrap().len(), 2);
    assert!(db.list_global_volumes().unwrap().iter().all(|volume| {
        !volume.enabled && volume.scope_authorization_state == SCOPE_AUTHORIZATION_NOT_AUTHORIZED
    }));

    let third = source("vol-c", SCOPE_AUTHORIZATION_NOT_AUTHORIZED, false);
    provider.sources.lock().unwrap().push(third);
    cycle(&provider, &db, &cancel);
    assert_eq!(provider.operations.load(Ordering::Acquire), 0);
    assert_eq!(db.list_global_volumes().unwrap().len(), 3);

    drop(db);
    let db = Database::open(&path).expect("restart against same isolated database");
    cycle(&provider, &db, &cancel);
    assert_eq!(provider.operations.load(Ordering::Acquire), 0);

    let mut legacy = source(
        "legacy-default",
        SCOPE_AUTHORIZATION_LEGACY_CONFIRMATION_REQUIRED,
        true,
    );
    legacy.volume.index_status = INDEX_STATUS_READY.to_string();
    legacy.volume.journal_id = Some("17".into());
    legacy.volume.journal_cursor = Some("900".into());
    legacy.volume.last_full_index_at = Some(100);
    legacy.volume.entry_count = 4;
    db.upsert_global_volume(&legacy.volume).unwrap();

    let mut already_authorized = source("old-authorized", SCOPE_AUTHORIZATION_AUTHORIZED, true);
    already_authorized.volume.last_full_index_at = Some(100);
    db.upsert_global_volume(&already_authorized.volume).unwrap();
    provider
        .sources
        .lock()
        .unwrap()
        .extend([legacy.clone(), already_authorized.clone()]);
    cycle(&provider, &db, &cancel);
    assert_eq!(provider.operations.load(Ordering::Acquire), 1);
    let legacy_after = db.get_global_volume(&legacy.volume.id).unwrap().unwrap();
    assert!(legacy_after.enabled);
    assert_eq!(legacy_after.index_status, INDEX_STATUS_READY);
    assert_eq!(legacy_after.journal_id.as_deref(), Some("17"));
    assert_eq!(legacy_after.journal_cursor.as_deref(), Some("900"));
    assert_eq!(legacy_after.last_full_index_at, Some(100));
    assert_eq!(legacy_after.entry_count, 4);
    assert_eq!(
        db.global_index_status().unwrap().status,
        "authorization_required"
    );
    assert!(!db.global_index_status().unwrap().collection_complete);

    assert!(db
        .request_global_index_rebuild(&legacy.volume.id)
        .map(|requested| !requested)
        .unwrap());
    assert!(db
        .request_global_index_rebuild(&already_authorized.volume.id)
        .unwrap());
    cycle(&provider, &db, &cancel);
    assert_eq!(provider.operations.load(Ordering::Acquire), 2);

    cancel.store(true, Ordering::Release);
    cycle(&provider, &db, &cancel);
    assert_eq!(provider.operations.load(Ordering::Acquire), 2);
    cancel.store(false, Ordering::Release);
    cycle(&provider, &db, &cancel);
    assert_eq!(provider.operations.load(Ordering::Acquire), 3);

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn explicit_authorization_is_required_to_enable_and_disable_never_grants_consent() {
    let path = super::test_db_path();
    let db = Database::open(&path).expect("open isolated test database");
    let unapproved = source("unapproved", SCOPE_AUTHORIZATION_NOT_AUTHORIZED, false);
    db.upsert_global_volume(&unapproved.volume).unwrap();

    let enable = db.set_global_volume_enabled(&unapproved.volume.id, true);
    assert!(
        matches!(enable, Err(crate::db::DbError::Validation(message)) if message == "global_index_source_authorization_required")
    );
    assert!(!db
        .request_global_index_rebuild(&unapproved.volume.id)
        .unwrap());
    assert!(db
        .authorize_global_volume(&unapproved.volume.id, false, &unapproved.volume)
        .is_err());
    let unchanged = db
        .get_global_volume(&unapproved.volume.id)
        .unwrap()
        .unwrap();
    assert!(!unchanged.enabled);
    assert_eq!(
        unchanged.scope_authorization_state,
        SCOPE_AUTHORIZATION_NOT_AUTHORIZED
    );

    db.authorize_global_volume(&unapproved.volume.id, true, &unapproved.volume)
        .unwrap();
    let approved = db
        .get_global_volume(&unapproved.volume.id)
        .unwrap()
        .unwrap();
    assert!(approved.enabled);
    assert_eq!(
        approved.scope_authorization_state,
        SCOPE_AUTHORIZATION_AUTHORIZED
    );
    db.set_global_volume_enabled(&unapproved.volume.id, false)
        .unwrap();
    db.set_global_volume_enabled(&unapproved.volume.id, true)
        .unwrap();
    let reenabled = db
        .get_global_volume(&unapproved.volume.id)
        .unwrap()
        .unwrap();
    assert!(reenabled.enabled);
    assert_eq!(
        reenabled.scope_authorization_state,
        SCOPE_AUTHORIZATION_AUTHORIZED
    );

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn busy_source_statuses_remain_incomplete_and_distinct_in_the_aggregate() {
    let path = super::test_db_path();
    let db = Database::open(&path).expect("open isolated status database");
    let mut busy = source("busy-status", SCOPE_AUTHORIZATION_AUTHORIZED, true);
    busy.volume.index_status = INDEX_STATUS_DATABASE_BUSY.to_string();
    busy.volume.last_error = Some("database error: database is locked".to_string());
    db.upsert_global_volume(&busy.volume).unwrap();

    let database_status = db.global_index_status().expect("database busy status");
    assert_eq!(database_status.status, INDEX_STATUS_ERROR);
    assert!(!database_status.collection_complete);
    assert_eq!(
        db.get_global_volume(&busy.volume.id)
            .unwrap()
            .unwrap()
            .index_status,
        INDEX_STATUS_DATABASE_BUSY
    );

    db.update_global_volume_state(
        &busy.volume.id,
        INDEX_STATUS_SERVICE_BUSY,
        Some("Another global index operation is already running"),
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let service_status = db.global_index_status().expect("service busy status");
    assert_eq!(service_status.status, INDEX_STATUS_ERROR);
    assert!(!service_status.collection_complete);
    assert_eq!(
        db.get_global_volume(&busy.volume.id)
            .unwrap()
            .unwrap()
            .index_status,
        INDEX_STATUS_SERVICE_BUSY
    );

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn concurrent_enable_disable_cannot_grant_or_retain_unauthorized_scope() {
    let path = super::test_db_path();
    let db = Database::open(&path).expect("open isolated test database");
    let mut unconfirmed = source(
        "legacy-race",
        SCOPE_AUTHORIZATION_LEGACY_CONFIRMATION_REQUIRED,
        true,
    );
    unconfirmed.volume.index_status = INDEX_STATUS_READY.to_string();
    db.upsert_global_volume(&unconfirmed.volume).unwrap();
    drop(db);

    let enable_db = Database::open(&path).expect("open concurrent enabler");
    let disable_db = Database::open(&path).expect("open concurrent disabler");
    let start = Arc::new(std::sync::Barrier::new(2));
    let enable_start = Arc::clone(&start);
    let enable = std::thread::spawn(move || {
        enable_start.wait();
        for _ in 0..32 {
            assert!(matches!(
                enable_db.set_global_volume_enabled("legacy-race", true),
                Err(crate::db::DbError::Validation(message))
                    if message == "global_index_source_authorization_required"
            ));
        }
    });
    let disable_start = Arc::clone(&start);
    let disable = std::thread::spawn(move || {
        disable_start.wait();
        for _ in 0..32 {
            disable_db
                .set_global_volume_enabled("legacy-race", false)
                .expect("disable remains available during concurrent enable attempts");
        }
    });
    enable.join().expect("concurrent enable attempts complete");
    disable
        .join()
        .expect("concurrent disable attempts complete");

    let db = Database::open(&path).expect("reopen isolated test database");
    let persisted = db
        .get_global_volume("legacy-race")
        .unwrap()
        .expect("legacy source remains present");
    assert!(!persisted.enabled);
    assert_eq!(
        persisted.scope_authorization_state,
        SCOPE_AUTHORIZATION_LEGACY_CONFIRMATION_REQUIRED
    );
    drop(db);
    let _ = std::fs::remove_file(path);
}
