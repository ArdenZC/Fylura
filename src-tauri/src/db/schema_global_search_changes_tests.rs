use super::*;
use rusqlite::{Connection, TransactionBehavior};

fn database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    super::super::schema::migrate(&conn).unwrap();
    install(&conn).unwrap();
    conn
}

fn insert_volume(conn: &Connection) {
    conn.execute(
        "INSERT INTO global_volumes (id, platform, stable_volume_id, display_name, mount_path, filesystem_type, drive_kind, enabled, provider, index_status, created_at, updated_at) VALUES ('volume-a', 'windows', 'volume-id-a', 'A', 'A:\\', 'ntfs', 'fixed', 1, 'windows_mft', 'ready', 1, 1)",
        [],
    )
    .unwrap();
}

fn insert_entry(conn: &Connection, id: &str, name: &str) {
    conn.execute(
        "INSERT INTO global_entries (id, volume_id, platform_file_id, parent_platform_file_id, name, name_normalized, path, path_normalized, extension, is_directory, size, modified_at_fs, file_attributes, is_hidden, is_system, is_stale, source_provider, last_seen_at) VALUES (?1, 'volume-a', 'file-identity', 'parent-identity', ?2, lower(?2), ?3, lower(?3), 'txt', 0, 10, 100, 0, 0, 0, 0, 'windows_mft', 1)",
        (id, name, format!("A:\\{name}")),
    )
    .unwrap();
}

#[test]
fn entry_facts_and_generation_change_in_the_same_transaction_and_rollback_together() {
    let mut conn = database();
    insert_volume(&conn);
    let initial = read_state(&conn).unwrap();
    insert_entry(&conn, "entry-a", "Report.txt");
    let inserted = read_state(&conn).unwrap();
    assert_eq!(inserted.generation, initial.generation + 1);
    assert_eq!(inserted.event_count, 2); // source barrier + entry projection

    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    transaction
        .execute("UPDATE global_entries SET name='Renamed.txt', name_normalized='renamed.txt' WHERE id='entry-a'", [])
        .unwrap();
    let inside_transaction = read_state(&transaction).unwrap();
    assert_eq!(inside_transaction.generation, inserted.generation + 1);
    transaction.rollback().unwrap();

    assert_eq!(read_state(&conn).unwrap(), inserted);
    let name: String = conn
        .query_row(
            "SELECT name FROM global_entries WHERE id='entry-a'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(name, "Report.txt");
}

#[test]
fn rename_move_stale_delete_and_source_disable_are_recorded_or_force_rebuild() {
    let conn = database();
    insert_volume(&conn);
    insert_entry(&conn, "entry-a", "Report.txt");
    let before = read_state(&conn).unwrap();

    conn.execute(
        "UPDATE global_entries SET name='Moved.txt', name_normalized='moved.txt', path='A:\\Archive\\Moved.txt', path_normalized='a:\\archive\\moved.txt', modified_at_fs=200 WHERE id='entry-a'",
        [],
    )
    .unwrap();
    let renamed = read_state(&conn).unwrap();
    assert_eq!(renamed.generation, before.generation + 1);
    let entry_event: (String, i64) = conn
        .query_row(
            "SELECT entity_id, generation FROM global_search_changes WHERE change_kind='entry' ORDER BY generation DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(entry_event, ("entry-a".into(), renamed.generation));

    conn.execute(
        "UPDATE global_entries SET is_stale=1 WHERE id='entry-a'",
        [],
    )
    .unwrap();
    conn.execute("DELETE FROM global_entries WHERE id='entry-a'", [])
        .unwrap();
    conn.execute(
        "UPDATE global_volumes SET enabled=0 WHERE id='volume-a'",
        [],
    )
    .unwrap();
    let final_state = read_state(&conn).unwrap();
    assert!(final_state.generation > renamed.generation);
    assert!(conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM global_search_changes WHERE change_kind='invalidate' AND entity_id='all')",
            [],
            |row| row.get::<_, bool>(0),
        )
        .unwrap());
}

#[test]
fn feed_preserves_every_mutation_generation_for_gap_detection() {
    let conn = database();
    insert_volume(&conn);
    insert_entry(&conn, "entry-a", "Report.txt");
    let first = read_state(&conn).unwrap();
    conn.execute(
        "UPDATE global_entries SET name='Report 2.txt', name_normalized='report 2.txt' WHERE id='entry-a'",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE global_entries SET name='Report 3.txt', name_normalized='report 3.txt' WHERE id='entry-a'",
        [],
    )
    .unwrap();
    let latest = read_state(&conn).unwrap();
    let events = changes_since(&conn, first.generation, 100).unwrap();
    let entry_events = events
        .iter()
        .filter(|event| event.kind == ChangeKind::Entry)
        .collect::<Vec<_>>();
    assert_eq!(entry_events.len(), 2);
    assert_eq!(entry_events[0].entity_id, "entry-a");
    assert_eq!(entry_events[1].entity_id, "entry-a");
    assert_eq!(entry_events[1].generation, latest.generation);
    assert_eq!(entry_events[1].generation, entry_events[0].generation + 1);
    assert!(latest.generation > first.generation);
}

#[test]
fn full_snapshot_can_clear_only_the_feed_captured_at_its_generation() {
    let mut conn = database();
    insert_volume(&conn);
    insert_entry(&conn, "entry-a", "Report.txt");
    let snapshot_generation = read_state(&conn).unwrap().generation;
    assert!(reset_after_full_snapshot(&mut conn, snapshot_generation).unwrap());
    let reset = read_state(&conn).unwrap();
    assert_eq!(reset.feed_floor_generation, snapshot_generation);
    assert_eq!(reset.event_count, 0);
    assert!(!reset.feed_saturated);

    insert_entry(&conn, "entry-b", "Invoice.txt");
    assert!(!reset_after_full_snapshot(&mut conn, snapshot_generation).unwrap());
    let current = read_state(&conn).unwrap();
    assert!(current.generation > snapshot_generation);
    assert_eq!(
        changes_since(&conn, snapshot_generation, 20).unwrap().len(),
        1
    );
}

#[test]
fn authorization_changes_are_an_invalidation() {
    let conn = database();
    insert_volume(&conn);
    let before = read_state(&conn).unwrap();
    conn.execute(
        "UPDATE global_volumes SET scope_authorization_state='authorized' WHERE id='volume-a'",
        [],
    )
    .unwrap();
    let after = read_state(&conn).unwrap();
    assert_eq!(after.generation, before.generation + 1);
    assert!(changes_since(&conn, before.generation, 10)
        .unwrap()
        .iter()
        .any(|event| event.kind == ChangeKind::Invalidate));
}

#[test]
fn saturated_feed_stays_bounded_and_marks_incremental_view_unusable() {
    let conn = database();
    insert_volume(&conn);
    conn.execute(
        "UPDATE global_search_change_state SET event_count=100000 WHERE singleton_id=1",
        [],
    )
    .unwrap();
    let before = read_state(&conn).unwrap();
    insert_entry(&conn, "entry-a", "Report.txt");
    let after = read_state(&conn).unwrap();
    assert_eq!(after.generation, before.generation + 1);
    assert!(after.feed_saturated);
    assert_eq!(after.event_count, MAX_RETAINED_CHANGES);
    assert!(changes_since(&conn, before.generation, 10)
        .unwrap()
        .is_empty());
}
