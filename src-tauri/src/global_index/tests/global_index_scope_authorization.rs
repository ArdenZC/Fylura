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
fn global_search_filters_unconfirmed_windows_sources_on_all_tiers_and_revalidates_after_revoke() {
    let path = super::test_db_path();
    let db = Database::open(&path).expect("open isolated search authorization database");

    let mut authorized = super::test_volume();
    authorized.index_status = INDEX_STATUS_READY.to_string();
    authorized.enabled = false;
    authorized.scope_authorization_state = SCOPE_AUTHORIZATION_NOT_AUTHORIZED.to_string();
    let not_authorized = source(
        "search-not-authorized",
        SCOPE_AUTHORIZATION_NOT_AUTHORIZED,
        true,
    );
    let legacy = source(
        "search-legacy-confirmation",
        SCOPE_AUTHORIZATION_LEGACY_CONFIRMATION_REQUIRED,
        true,
    );
    let disabled = source("search-disabled", SCOPE_AUTHORIZATION_AUTHORIZED, false);
    let pending = source(
        "search-coverage-pending",
        SCOPE_AUTHORIZATION_AUTHORIZED,
        true,
    );
    let mut macos = super::test_volume();
    macos.id = "search-macos".to_string();
    macos.stable_volume_id = "stable-search-macos".to_string();
    macos.platform = "macos".to_string();
    macos.mount_path = "/Volumes/Search".to_string();
    macos.filesystem_type = "apfs".to_string();
    macos.scope_authorization_state = SCOPE_AUTHORIZATION_NOT_REQUIRED.to_string();
    macos.provider = PROVIDER_MACOS_SPOTLIGHT.to_string();
    macos.index_status = INDEX_STATUS_READY.to_string();

    for volume in [
        &authorized,
        &not_authorized.volume,
        &legacy.volume,
        &disabled.volume,
        &pending.volume,
        &macos,
    ] {
        db.upsert_global_volume(volume)
            .expect("insert search source");
    }

    let make_entry =
        |volume_id: &str, platform_file_id: &str, name: &str, extension: &str, modified_at: i64| {
            let mut entry = super::test_entry(&format!(r"C:\Global\{name}"), name, false);
            entry.volume_id = volume_id.to_string();
            entry.platform_file_id = platform_file_id.to_string();
            entry.extension = extension.to_string();
            entry.modified_at_fs = Some(modified_at);
            entry
        };

    let windows_entry = make_entry(
        &authorized.id,
        "allowed-main",
        "Needle-Search.txt",
        "txt",
        100,
    );
    let only_authorized_entry = make_entry(
        &authorized.id,
        "allowed-only",
        "OnlyAuthorizedWindows.qxy",
        "qxy",
        100,
    );
    let authorized_fts_entry =
        make_entry(&authorized.id, "allowed-fts", "Authorized.txt", "txt", 100);
    let entries = vec![
        windows_entry.clone(),
        only_authorized_entry.clone(),
        authorized_fts_entry,
        make_entry(
            &not_authorized.volume.id,
            "not-authorized-main",
            "Needle-Search.txt",
            "txt",
            900,
        ),
        make_entry(
            &legacy.volume.id,
            "legacy-main",
            "Needle-Search.txt",
            "txt",
            800,
        ),
        make_entry(
            &disabled.volume.id,
            "disabled-main",
            "Needle-Search.txt",
            "txt",
            700,
        ),
        make_entry(
            &pending.volume.id,
            "pending-main",
            "Needle-Search.txt",
            "txt",
            600,
        ),
        make_entry(
            &not_authorized.volume.id,
            "not-authorized-extension",
            "Needle-Search.txb",
            "txb",
            900,
        ),
        make_entry(
            &legacy.volume.id,
            "legacy-extension",
            "Needle-Search.txc",
            "txc",
            800,
        ),
        make_entry(
            &disabled.volume.id,
            "disabled-extension",
            "Needle-Search.txd",
            "txd",
            700,
        ),
        make_entry(
            &pending.volume.id,
            "pending-extension",
            "Needle-Search.txe",
            "txe",
            600,
        ),
        make_entry(
            &authorized.id,
            "allowed-extension",
            "Needle-Search.txa",
            "txa",
            100,
        ),
        make_entry(
            &not_authorized.volume.id,
            "not-authorized-single-extension",
            "PrivateWindows.qxy",
            "qxy",
            900,
        ),
        make_entry(&macos.id, "macos-main", "MacOS-Needle.txt", "txt", 50),
    ];
    let windows_entry_id = windows_entry.entry_id();
    let only_authorized_entry_id = only_authorized_entry.entry_id();
    let macos_entry_id = entries.last().expect("macOS search entry").entry_id();
    db.upsert_global_entries_batch(&entries)
        .expect("insert retained historical search entries");
    db.conn()
        .expect("open pending source update connection")
        .execute(
            "UPDATE global_volumes SET coverage_pending = 1 WHERE id = ?1",
            [&pending.volume.id],
        )
        .expect("mark source coverage pending");

    let pending_revision = db
        .search_global_entries_snapshot("needle", 20, 0)
        .expect("capture pending source revision")
        .source_revision;
    db.conn()
        .expect("open coverage completion connection")
        .execute(
            "UPDATE global_volumes SET coverage_pending = 0 WHERE id = ?1",
            [&pending.volume.id],
        )
        .expect("complete pending source coverage");
    let covered_revision = db
        .search_global_entries_snapshot("needle", 20, 0)
        .expect("capture completed source revision")
        .source_revision;
    assert_ne!(pending_revision, covered_revision);
    db.conn()
        .expect("re-open pending source update connection")
        .execute(
            "UPDATE global_volumes SET coverage_pending = 1 WHERE id = ?1",
            [&pending.volume.id],
        )
        .expect("restore pending state for result exclusion assertions");

    let before_authorization = db
        .search_global_entries_snapshot("onlyauthorizedwindows", 10, 0)
        .expect("search remains empty before explicit Windows authorization");
    assert!(before_authorization.results.is_empty());
    db.authorize_global_volume(&authorized.id, true, &authorized)
        .expect("explicitly authorize the eligible Windows source");
    let after_authorization = db
        .search_global_entries_snapshot("onlyauthorizedwindows", 10, 0)
        .expect("existing indexed entry becomes visible after authorization");
    assert_eq!(after_authorization.results.len(), 1);
    assert_eq!(
        after_authorization.results[0].id,
        only_authorized_entry.entry_id()
    );
    assert_ne!(
        before_authorization.source_revision,
        after_authorization.source_revision
    );

    let conn = db.conn().expect("open direct search tier connection");
    for (tier, query) in [
        ("exact_name", "needle-search.txt"),
        ("name_prefix", "needle-"),
        ("exact_extension", "txt"),
        ("extension_prefix", "tx"),
        ("fts", "needle"),
        ("punctuation_prefix", "needle-"),
    ] {
        let results = crate::global_index::search::diagnostic_search_tier(&conn, tier, query, 1)
            .unwrap_or_else(|error| panic!("run {tier} tier: {error}"));
        assert_eq!(
            results.len(),
            1,
            "{tier} must skip unauthorized rows before LIMIT"
        );
        assert!(
            results[0].volume_id == authorized.id || results[0].volume_id == macos.id,
            "{tier} returned an ineligible source: {}",
            results[0].volume_id
        );
    }

    let authorized_fts =
        crate::global_index::search::diagnostic_search_tier(&conn, "fts", "authorized", 10)
            .expect("search an authorized Windows-only FTS token");
    assert!(!authorized_fts.is_empty());
    assert!(authorized_fts
        .iter()
        .all(|result| result.volume_id == authorized.id));

    let legacy_prefix =
        crate::global_index::search::with_legacy_name_prefix_search_candidate(|| {
            crate::global_index::search::diagnostic_search_tier(&conn, "name_prefix", "needle-", 1)
        })
        .expect("run legacy name-prefix fallback");
    assert_eq!(legacy_prefix[0].volume_id, authorized.id);
    let legacy_fts = crate::global_index::search::with_legacy_fts_search_candidate(|| {
        crate::global_index::search::diagnostic_search_tier(&conn, "fts", "authorized", 1)
    })
    .expect("run legacy FTS fallback");
    assert_eq!(legacy_fts[0].volume_id, authorized.id);
    let key_only_extension =
        crate::global_index::search::with_key_only_cte_search_candidate(|| {
            crate::global_index::search::diagnostic_search_tier(&conn, "extension_prefix", "qx", 1)
        })
        .expect("run single-key extension prefix query");
    assert_eq!(key_only_extension[0].volume_id, authorized.id);
    drop(conn);

    let first_page = db
        .search_global_entries("needle", 1, 0)
        .expect("search the authorized Windows and macOS sources");
    assert_eq!(first_page.len(), 1);
    assert_eq!(first_page[0].volume_id, authorized.id);
    assert_eq!(first_page[0].id, windows_entry_id);
    let second_page = db
        .search_global_entries("needle", 1, 1)
        .expect("read the next page without unauthorized rows consuming the offset");
    assert_eq!(second_page.len(), 1);
    assert_eq!(second_page[0].volume_id, authorized.id);
    assert_ne!(second_page[0].id, first_page[0].id);
    let third_page = db
        .search_global_entries("needle", 1, 2)
        .expect("continue paging through only currently searchable sources");
    assert_eq!(third_page.len(), 1);
    assert_eq!(third_page[0].volume_id, macos.id);
    let macos_result = db
        .search_global_entries("macos-needle", 10, 0)
        .expect("macOS Spotlight source remains searchable without Windows consent");
    assert_eq!(macos_result.len(), 1);
    assert_eq!(macos_result[0].id, macos_entry_id);

    let warmed = db
        .search_global_entries_snapshot("onlyauthorizedwindows", 10, 0)
        .expect("warm a result and source revision before revocation");
    assert_eq!(warmed.results.len(), 1);
    assert_eq!(warmed.results[0].id, only_authorized_entry_id);
    db.conn()
        .expect("open revocation connection")
        .execute(
            "UPDATE global_volumes SET scope_authorization_state = ?2 WHERE id = ?1",
            rusqlite::params![authorized.id, SCOPE_AUTHORIZATION_NOT_AUTHORIZED],
        )
        .expect("revoke Windows source without changing its timestamp or enabled flag");
    let revoked = db
        .search_global_entries_snapshot("onlyauthorizedwindows", 10, 0)
        .expect("requery immediately after revocation");
    assert!(
        revoked.results.is_empty(),
        "revoked Windows rows must clear immediately"
    );
    assert_ne!(
        warmed.source_revision, revoked.source_revision,
        "authorization state must invalidate the source revision even within one timestamp tick"
    );
    assert_eq!(
        crate::global_index::commands::revalidated_global_entry(&db, &only_authorized_entry_id)
            .unwrap_err(),
        "global_search_result_not_found",
        "an old result ID must not activate after its source is revoked"
    );
    assert_eq!(
        db.search_global_entries("needle", 20, 0)
            .expect("other eligible sources remain searchable")
            .iter()
            .map(|result| result.volume_id.as_str())
            .collect::<std::collections::HashSet<_>>(),
        [macos.id.as_str()].into_iter().collect()
    );

    drop(db);
    let reopened = Database::open(&path).expect("restart against persisted revocation");
    assert_eq!(
        reopened
            .get_global_volume(&authorized.id)
            .unwrap()
            .unwrap()
            .scope_authorization_state,
        SCOPE_AUTHORIZATION_NOT_AUTHORIZED
    );
    assert!(reopened
        .search_global_entries("onlyauthorizedwindows", 10, 0)
        .expect("search after restart")
        .is_empty());
    let reopened_macos = reopened
        .search_global_entries("macos-needle", 10, 0)
        .expect("macOS source remains visible after Windows revocation and restart");
    assert_eq!(reopened_macos.len(), 1);
    assert_eq!(reopened_macos[0].id, macos_entry_id);

    drop(reopened);
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
