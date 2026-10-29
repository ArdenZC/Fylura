use super::{
    memory::{MemoryBudget, MAX_ACTIVE_VIEW_BYTES},
    view::{SearchView, MAX_CANDIDATE_WINDOW, MAX_INCREMENTAL_CHANGES},
    worker::supported_candidate_query,
};
use crate::{
    db::{schema_global_search_changes, Database},
    global_index::{
        models::{GlobalEntryInput, GlobalVolume},
        search,
    },
};
use r2d2::PooledConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::{
    cmp::Ordering,
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering as AtomicOrdering},
        Arc, Barrier,
    },
    time::Instant,
};

static TEST_DATABASE_ID: AtomicU64 = AtomicU64::new(0);

struct TestConnection {
    path: PathBuf,
    connection: Option<PooledConnection<SqliteConnectionManager>>,
    database: Option<Database>,
}

impl Deref for TestConnection {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        self.connection.as_deref().unwrap()
    }
}

impl DerefMut for TestConnection {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.connection.as_deref_mut().unwrap()
    }
}

impl Drop for TestConnection {
    fn drop(&mut self) {
        self.connection.take();
        self.database.take();
        let _ = std::fs::remove_file(&self.path);
        let mut wal = self.path.clone();
        wal.set_extension("db-wal");
        let _ = std::fs::remove_file(wal);
        let mut shm = self.path.clone();
        shm.set_extension("db-shm");
        let _ = std::fs::remove_file(shm);
    }
}

fn database() -> TestConnection {
    let sequence = TEST_DATABASE_ID.fetch_add(1, AtomicOrdering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "zen-canvas-search-view-{}-{sequence}.db",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let database = Database::open(&path).unwrap();
    let connection = database.conn().unwrap();
    TestConnection {
        path,
        connection: Some(connection),
        database: Some(database),
    }
}

fn insert_volume(conn: &Connection) {
    conn.execute(
        "INSERT INTO global_volumes (id, platform, stable_volume_id, display_name, mount_path, filesystem_type, drive_kind, enabled, provider, index_status, created_at, updated_at) VALUES ('volume-a', 'windows', 'volume-id-a', 'A', 'A:\\', 'ntfs', 'fixed', 1, 'windows_mft', 'ready', 1, 1)",
        [],
    )
    .unwrap();
}

fn insert_entry(
    conn: &Connection,
    id: &str,
    name: &str,
    path: &str,
    extension: &str,
    modified_at: Option<i64>,
) {
    conn.execute(
        "INSERT INTO global_entries (id, volume_id, platform_file_id, parent_platform_file_id, name, name_normalized, path, path_normalized, extension, is_directory, size, modified_at_fs, file_attributes, is_hidden, is_system, is_stale, source_provider, last_seen_at) VALUES (?1, 'volume-a', ?2, 'parent-identity', ?3, lower(?3), ?4, ?5, ?6, 0, 10, ?7, 0, 0, 0, 0, 'windows_mft', 1)",
        (
            id,
            id,
            name,
            path,
            path.to_ascii_lowercase().replace('\\', "/"),
            extension,
            modified_at,
        ),
    )
    .unwrap();
}

fn generation(conn: &Connection) -> i64 {
    schema_global_search_changes::read_state(conn)
        .unwrap()
        .generation
}

fn oracle_ids(conn: &Connection, query: &str) -> Vec<(String, Option<i64>)> {
    let mut statement = conn
        .prepare(
            "SELECT entry.id, entry.name, entry.path, entry.extension, entry.modified_at_fs FROM global_entries entry JOIN global_volumes volume ON volume.id=entry.volume_id WHERE entry.is_stale=0 AND volume.enabled=1",
        )
        .unwrap();
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })
        .unwrap();
    let rows = rows.collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let query = query.to_ascii_lowercase();
    let mut matches = rows
        .into_iter()
        .filter(|(_, name, path, extension, _)| {
            [name, path, extension]
                .iter()
                .any(|value| value.to_ascii_lowercase().contains(&query))
        })
        .map(|(id, _, _, _, modified_at)| (id, modified_at))
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| match (right.1, left.1) {
        (Some(right_time), Some(left_time)) => right_time
            .cmp(&left_time)
            .then_with(|| left.0.cmp(&right.0)),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => left.0.cmp(&right.0),
    });
    matches
}

fn candidate_ids(
    view: &SearchView,
    query: &str,
    budget: &std::sync::Arc<MemoryBudget>,
) -> Vec<String> {
    view.candidate_window(query, budget.clone())
        .unwrap()
        .ids
        .into_iter()
        .map(str::to_owned)
        .collect()
}

#[test]
fn compact_view_matches_independent_ascii_oracle_and_fts_for_names_paths_and_extensions() {
    let conn = database();
    insert_volume(&conn);
    insert_entry(
        &conn,
        "entry-report",
        "Report-03.PDF",
        "A:\\Projects\\Month-03\\Report-03.PDF",
        "PDF",
        Some(10),
    );
    insert_entry(
        &conn,
        "entry-invoice",
        "Invoice Final.pdf",
        "A:\\Invoices\\2025\\Invoice Final.pdf",
        "pdf",
        Some(20),
    );
    insert_entry(
        &conn,
        "entry-unicode",
        "Résumé.pdf",
        "A:\\Archive\\Résumé.pdf",
        "pdf",
        None,
    );
    let generation = generation(&conn);
    let budget = MemoryBudget::new(256 * 1024 * 1024);
    let view = SearchView::build(&conn, generation, budget.clone()).unwrap();
    assert!(view.active_bytes() <= MAX_ACTIVE_VIEW_BYTES);
    assert!(budget.used() <= 256 * 1024 * 1024);
    assert!(budget.peak() <= 256 * 1024 * 1024);

    for query in ["month-03", "REPORT", "invoice", "pdf", "port-03", "sum"] {
        let view_ids = candidate_ids(&view, query, &budget);
        let oracle = oracle_ids(&conn, query);
        assert_eq!(
            view_ids,
            oracle.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>()
        );
        let mut fts_ids =
            search::search_global_fts_shadow_reference_ids(&conn, query, 4096).unwrap();
        fts_ids.sort();
        let mut oracle_ids = oracle.into_iter().map(|(id, _)| id).collect::<Vec<_>>();
        oracle_ids.sort();
        assert_eq!(fts_ids, oracle_ids, "independent oracle query: {query}");
    }
}

#[test]
fn incremental_insert_rename_move_sort_change_and_delete_publish_complete_views() {
    let mut conn = database();
    insert_volume(&conn);
    insert_entry(
        &conn,
        "entry-old",
        "Old Report.txt",
        "A:\\Reports\\Old Report.txt",
        "txt",
        Some(10),
    );
    let budget = MemoryBudget::new(256 * 1024 * 1024);
    let start_generation = generation(&conn);
    let mut view = SearchView::build(&conn, start_generation, budget.clone()).unwrap();
    assert!(
        schema_global_search_changes::reset_after_full_snapshot(&mut conn, start_generation)
            .unwrap()
    );

    insert_entry(
        &conn,
        "entry-new",
        "Invoice.pdf",
        "A:\\Finance\\Invoice.pdf",
        "pdf",
        Some(20),
    );
    let events = schema_global_search_changes::changes_since(&conn, view.generation, 10).unwrap();
    view = view
        .apply_changes(&conn, &events, budget.clone(), Instant::now())
        .unwrap();
    assert!(candidate_ids(&view, "invoice", &budget).contains(&"entry-new".to_owned()));

    conn.execute(
        "UPDATE global_entries SET name='Quarterly Report.pdf', name_normalized='quarterly report.pdf', path='A:\\Archive\\Quarterly Report.pdf', path_normalized='a:/archive/quarterly report.pdf', extension='pdf', modified_at_fs=99 WHERE id='entry-new'",
        [],
    )
    .unwrap();
    let events = schema_global_search_changes::changes_since(&conn, view.generation, 10).unwrap();
    view = view
        .apply_changes(&conn, &events, budget.clone(), Instant::now())
        .unwrap();
    assert!(!candidate_ids(&view, "invoice", &budget).contains(&"entry-new".to_owned()));
    assert!(candidate_ids(&view, "archive\\quarter", &budget).contains(&"entry-new".to_owned()));
    assert_eq!(candidate_ids(&view, "report", &budget)[0], "entry-new");

    conn.execute("DELETE FROM global_entries WHERE id='entry-new'", [])
        .unwrap();
    let events = schema_global_search_changes::changes_since(&conn, view.generation, 10).unwrap();
    view = view
        .apply_changes(&conn, &events, budget.clone(), Instant::now())
        .unwrap();
    assert!(!candidate_ids(&view, "quarterly", &budget).contains(&"entry-new".to_owned()));
    assert!(view.active_bytes() <= MAX_ACTIVE_VIEW_BYTES);
    assert!(budget.used() <= 256 * 1024 * 1024);
    assert!(budget.peak() <= 256 * 1024 * 1024);
}

#[test]
fn concurrent_readers_keep_the_complete_old_view_while_a_new_generation_is_built() {
    let conn = database();
    insert_volume(&conn);
    insert_entry(
        &conn,
        "entry-stable",
        "Old filename.txt",
        "A:\\Old filename.txt",
        "txt",
        Some(1),
    );
    let budget = MemoryBudget::new(256 * 1024 * 1024);
    let old_view = Arc::new(SearchView::build(&conn, generation(&conn), budget.clone()).unwrap());
    let old_generation = old_view.generation;

    conn.execute(
        "UPDATE global_entries SET name='New filename.txt', name_normalized='new filename.txt', path='A:\\Archive\\New filename.txt', path_normalized='a:\\archive\\new filename.txt', modified_at_fs=2 WHERE id='entry-stable'",
        [],
    )
    .unwrap();
    let events = schema_global_search_changes::changes_since(&conn, old_generation, 10).unwrap();
    let rendezvous = Arc::new(Barrier::new(2));
    let read_view = old_view.clone();
    let read_budget = budget.clone();
    let read_rendezvous = rendezvous.clone();
    let reader = std::thread::spawn(move || {
        read_rendezvous.wait();
        candidate_ids(&read_view, "old", &read_budget)
    });
    rendezvous.wait();
    let new_view = Arc::new(
        old_view
            .apply_changes(&conn, &events, budget.clone(), Instant::now())
            .unwrap(),
    );
    assert_eq!(reader.join().unwrap(), ["entry-stable"]);
    assert!(candidate_ids(&new_view, "old", &budget).is_empty());
    assert_eq!(candidate_ids(&new_view, "new", &budget), ["entry-stable"]);
    assert_eq!(old_view.generation, old_generation);
    assert!(new_view.generation > old_generation);
}

#[test]
fn candidate_window_is_bounded_to_the_best_4096_independent_oracle_rows() {
    let conn = database();
    insert_volume(&conn);
    for index in 0..(MAX_CANDIDATE_WINDOW + 9) {
        let id = format!("entry-{index:05}");
        let name = format!("Common record {index:05}.txt");
        let path = format!("A:\\Records\\{name}");
        insert_entry(&conn, &id, &name, &path, "txt", Some(index as i64));
    }
    let budget = MemoryBudget::new(256 * 1024 * 1024);
    let view = SearchView::build(&conn, generation(&conn), budget.clone()).unwrap();
    let window = view.candidate_window("common", budget.clone()).unwrap();
    let oracle = oracle_ids(&conn, "common");
    assert_eq!(window.matching_count, MAX_CANDIDATE_WINDOW + 9);
    assert_eq!(window.ids.len(), MAX_CANDIDATE_WINDOW);
    assert_eq!(
        window.ids,
        oracle[..MAX_CANDIDATE_WINDOW]
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        window
            .ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        MAX_CANDIDATE_WINDOW
    );
}

#[test]
fn unsafe_source_change_and_excess_delta_require_a_full_rebuild() {
    let conn = database();
    insert_volume(&conn);
    insert_entry(
        &conn,
        "entry-a",
        "Example.txt",
        "A:\\Example.txt",
        "txt",
        Some(1),
    );
    let budget = MemoryBudget::new(256 * 1024 * 1024);
    let view = SearchView::build(&conn, generation(&conn), budget.clone()).unwrap();
    conn.execute(
        "UPDATE global_volumes SET enabled=0 WHERE id='volume-a'",
        [],
    )
    .unwrap();
    let events = schema_global_search_changes::changes_since(&conn, view.generation, 10).unwrap();
    assert!(view
        .apply_changes(&conn, &events, budget, Instant::now())
        .is_err());
    assert_eq!(MAX_INCREMENTAL_CHANGES, 5_000);
}

#[test]
fn low_memory_refuses_the_view_before_publishing_it() {
    let conn = database();
    insert_volume(&conn);
    insert_entry(
        &conn,
        "entry-a",
        "Example.txt",
        "A:\\Example.txt",
        "txt",
        Some(1),
    );
    let budget = MemoryBudget::new(128);
    assert!(SearchView::build(&conn, generation(&conn), budget).is_err());
}

#[test]
fn real_repository_upserts_rename_stale_and_source_disable_survive_database_reopen() {
    let sequence = TEST_DATABASE_ID.fetch_add(1, AtomicOrdering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "zen-canvas-search-view-{}-{sequence}.db",
        std::process::id()
    ));
    let db = Database::open(&path).unwrap();
    let volume = GlobalVolume {
        id: "volume-real-path".into(),
        platform: "macos".into(),
        stable_volume_id: "synthetic-volume".into(),
        display_name: "Fixture volume".into(),
        mount_path: "/fixture".into(),
        filesystem_type: "apfs".into(),
        drive_kind: "fixed".into(),
        enabled: true,
        provider: "macos_spotlight".into(),
        index_status: "ready".into(),
        last_error: None,
        journal_id: None,
        journal_cursor: None,
        last_full_index_at: None,
        last_incremental_sync_at: None,
        entry_count: 0,
        created_at: 1,
        updated_at: 1,
    };
    db.upsert_global_volume(&volume).unwrap();
    let original = GlobalEntryInput {
        volume_id: volume.id.clone(),
        platform_file_id: "mac:device:1:inode:2".into(),
        parent_platform_file_id: "mac:device:1:inode:1".into(),
        name: "Report.pdf".into(),
        path: "/fixture/Reports/Report.pdf".into(),
        extension: "pdf".into(),
        is_directory: false,
        size: 10,
        created_at_fs: Some(1),
        modified_at_fs: Some(10),
        file_attributes: 0,
        is_hidden: false,
        is_system: false,
        source_provider: "macos_spotlight".into(),
        last_seen_at: 1,
    };
    let entry_id = original.entry_id();
    db.upsert_global_entries_batch(std::slice::from_ref(&original))
        .unwrap();
    let initial_generation = schema_global_search_changes::read_state(&db.conn().unwrap())
        .unwrap()
        .generation;
    let budget = MemoryBudget::new(256 * 1024 * 1024);
    let mut conn = db.conn().unwrap();
    let mut view = SearchView::build(&conn, initial_generation, budget.clone()).unwrap();
    assert!(
        schema_global_search_changes::reset_after_full_snapshot(&mut conn, initial_generation)
            .unwrap()
    );
    drop(conn);

    let renamed = GlobalEntryInput {
        name: "Monthly Invoice.pdf".into(),
        path: "/fixture/Archive/Monthly Invoice.pdf".into(),
        modified_at_fs: Some(99),
        last_seen_at: 2,
        ..original
    };
    assert_eq!(renamed.entry_id(), entry_id);
    db.upsert_global_entries_batch(std::slice::from_ref(&renamed))
        .unwrap();
    let conn = db.conn().unwrap();
    let events = schema_global_search_changes::changes_since(&conn, view.generation, 20).unwrap();
    view = view
        .apply_changes(&conn, &events, budget.clone(), Instant::now())
        .unwrap();
    assert!(!candidate_ids(&view, "report", &budget).contains(&entry_id));
    assert!(candidate_ids(&view, "archive/month", &budget).contains(&entry_id));
    assert_eq!(
        db.search_global_entries_snapshot("monthly", 10, 0)
            .unwrap()
            .results
            .len(),
        1
    );
    drop(conn);

    assert!(db.mark_global_entry_stale(&entry_id).unwrap());
    let conn = db.conn().unwrap();
    let events = schema_global_search_changes::changes_since(&conn, view.generation, 20).unwrap();
    view = view
        .apply_changes(&conn, &events, budget.clone(), Instant::now())
        .unwrap();
    assert!(!candidate_ids(&view, "monthly", &budget).contains(&entry_id));
    drop(conn);

    db.set_global_volume_enabled(&volume.id, false).unwrap();
    let conn = db.conn().unwrap();
    let events = schema_global_search_changes::changes_since(&conn, view.generation, 20).unwrap();
    assert!(view
        .apply_changes(&conn, &events, budget.clone(), Instant::now())
        .is_err());
    let persisted_generation = schema_global_search_changes::read_state(&conn)
        .unwrap()
        .generation;
    drop(conn);
    drop(db);

    let restored = Database::open(&path).unwrap();
    let conn = restored.conn().unwrap();
    assert_eq!(
        schema_global_search_changes::read_state(&conn)
            .unwrap()
            .generation,
        persisted_generation
    );
    let restored_view = SearchView::build(&conn, persisted_generation, budget.clone()).unwrap();
    assert!(candidate_ids(&restored_view, "monthly", &budget).is_empty());
    assert!(restored_view.generation >= view.generation);
    drop(conn);
    drop(restored);
    let _ = std::fs::remove_file(&path);
    let mut wal = PathBuf::from(&path);
    wal.set_extension("db-wal");
    let _ = std::fs::remove_file(wal);
    let mut shm = PathBuf::from(&path);
    shm.set_extension("db-shm");
    let _ = std::fs::remove_file(shm);
}

#[test]
fn shadow_router_accepts_ascii_punctuation_and_falls_back_for_short_or_unicode_queries() {
    assert_eq!(
        supported_candidate_query(" Month-03 ").as_deref(),
        Some("month-03")
    );
    assert_eq!(
        supported_candidate_query("archive\\quarter").as_deref(),
        Some("archive/quarter")
    );
    assert_eq!(supported_candidate_query("pdf").as_deref(), Some("pdf"));
    assert!(supported_candidate_query("jp").is_none());
    assert!(supported_candidate_query("résumé").is_none());
    assert!(supported_candidate_query("中文查询").is_none());
}
