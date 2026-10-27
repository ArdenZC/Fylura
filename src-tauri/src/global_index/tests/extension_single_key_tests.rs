//! Production Extension Prefix single-key collapse: exact result/error parity.
use super::*;
use rusqlite::{params, Connection};

fn database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE global_volumes (
            id TEXT PRIMARY KEY, enabled INTEGER NOT NULL
        );
        INSERT INTO global_volumes VALUES ('on', 1), ('off', 0);
        CREATE TABLE global_entries (
            id TEXT PRIMARY KEY, volume_id TEXT, platform_file_id TEXT,
            name TEXT, path TEXT, extension TEXT, is_directory INTEGER,
            size INTEGER, created_at_fs INTEGER, modified_at_fs INTEGER,
            file_attributes INTEGER, is_hidden INTEGER, is_system INTEGER,
            source_provider TEXT, is_stale INTEGER
        );
        CREATE INDEX idx_global_entries_active_extension_order
            ON global_entries(extension, modified_at_fs DESC, id ASC)
            WHERE is_stale = 0;
        CREATE TABLE managed_scopes (id TEXT PRIMARY KEY, enabled INTEGER);
        CREATE TABLE managed_entries (
            id TEXT PRIMARY KEY, global_entry_id TEXT,
            managed_scope_id TEXT, enabled INTEGER
        );
        INSERT INTO managed_scopes VALUES ('scope', 1);
        "#,
    )
    .unwrap();
    for i in 0..500 {
        let ext = match i % 5 {
            0 | 3 => "jpg",
            1 => "json",
            2 => "pdf",
            _ => "txt",
        };
        let id = format!("ge_{i:064x}");
        conn.execute(
            "INSERT INTO global_entries VALUES
             (?1, ?2, ?1, ?3, ?4, ?5, 0, ?6, 0, ?7, 0, 0, 0, 'windows', ?8)",
            params![
                id,
                if i % 17 == 0 { "off" } else { "on" },
                format!("file-{i}.{ext}"),
                format!("C:/files/{i}.{ext}"),
                ext,
                i,
                1_700_000_000_i64 + (i / 4) as i64,
                i % 19 == 0,
            ],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO managed_entries VALUES
         ('managed', 'ge_0000000000000000000000000000000000000000000000000000000000000003', 'scope', 1)",
        [],
    )
    .unwrap();
    conn
}

fn legacy(
    conn: &Connection,
    query: &str,
    limit: u32,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    let sql = extension_prefix_legacy_sql();
    let mut statement = conn.prepare(&sql)?;
    collect_candidates(
        &mut statement,
        params![query, format!("{}*", escape_glob(query)), limit],
    )
}

#[test]
fn single_key_zero_sparse_multi_volume_ties_and_window_match_legacy() {
    let conn = database();
    conn.execute(
        "INSERT INTO global_entries VALUES
         ('extra-png', 'on', 'extra', 'image.png', 'C:/image.png', 'png',
          0, 1, 0, 1700000999, 0, 0, 0, 'windows', 0)",
        [],
    )
    .unwrap();

    for query in ["jp", "jso", "p", "no-match", "json", "Jp", "j[*]", "txt"] {
        for limit in [1, 2, 80, 160, 4096] {
            assert_eq!(
                search_extension_prefix(&conn, query, limit).unwrap(),
                legacy(&conn, query, limit).unwrap(),
                "query={query}, tier_limit={limit}",
            );
        }
    }

    // Reuse a caller-owned Snapshot read transaction; never nest BEGIN.
    conn.execute_batch("BEGIN DEFERRED").unwrap();
    assert_eq!(
        search_extension_prefix(&conn, "jp", 80).unwrap(),
        legacy(&conn, "jp", 80).unwrap(),
    );
    conn.execute_batch("ROLLBACK").unwrap();

    let (probe, exact) = diagnostic_extension_prefix_strategy_sqls();
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {probe}")).unwrap();
    let plan = stmt
        .query_map(params!["jp", "jp*"], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(plan.iter().any(|part| part.contains("COVERING INDEX")));

    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {exact}")).unwrap();
    let plan = stmt
        .query_map(params!["jp", "jp*", 80, "jpg"], |row| {
            row.get::<_, String>(3)
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(plan.iter().any(|part| {
        part.contains("idx_global_entries_active_extension_order")
    }));
    assert!(!plan.iter().any(|part| {
        part.contains("USE TEMP B-TREE FOR ORDER BY")
    }));
}

#[test]
fn nullable_id_and_dynamic_extension_values_preserve_production_errors() {
    let conn = database();
    // Rowid-table SQLite TEXT PRIMARY KEY still permits legacy NULL IDs.
    conn.execute(
        "UPDATE global_entries SET id=NULL
         WHERE rowid=(
            SELECT ge.rowid FROM global_entries ge
            JOIN global_volumes gv ON gv.id=ge.volume_id
            WHERE gv.enabled=1 AND ge.is_stale=0 AND ge.extension='jpg'
            ORDER BY ge.modified_at_fs DESC LIMIT 1
         )",
        [],
    )
    .unwrap();
    let expected = legacy(&conn, "jp", 80).map_err(|e| e.to_string());
    let actual = search_extension_prefix(&conn, "jp", 80)
        .map_err(|e| e.to_string());
    assert_eq!(actual, expected);

    // GLOB may accept a non-TEXT extension after SQLite coercion. If found,
    // the strategy must route to the original SQL rather than cast the key.
    conn.execute(
        "UPDATE global_entries SET extension=X'6A7067'
         WHERE id='ge_0000000000000000000000000000000000000000000000000000000000000001'",
        [],
    )
    .unwrap();
    let expected = legacy(&conn, "jp", 80).map_err(|e| e.to_string());
    let actual = search_extension_prefix(&conn, "jp", 80)
        .map_err(|e| e.to_string());
    assert_eq!(actual, expected);
}
