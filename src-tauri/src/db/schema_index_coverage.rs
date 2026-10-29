//! Additive Index Scope state. No row/FK/AI/user-metadata deletion.
use super::DbError;
use rusqlite::Connection;
pub(crate) fn install(conn: &Connection) -> Result<(), DbError> {
    for (table, column, definition) in [
        (
            "global_entries",
            "scope_decision",
            "TEXT NOT NULL DEFAULT 'include'",
        ),
        (
            "global_volumes",
            "coverage_pending",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "global_volumes",
            "coverage_included",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "global_volumes",
            "coverage_excluded",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "global_volumes",
            "coverage_undetermined",
            "INTEGER NOT NULL DEFAULT 0",
        ),
    ] {
        let exists = conn
            .prepare(&format!("PRAGMA table_info('{table}')"))?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .any(|c| c == column);
        if !exists {
            conn.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition};"
            ))?;
        }
    }
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_global_entries_minimal_ancestors ON global_entries(volume_id,id) WHERE scope_decision='minimal_ancestor';")?;
    // Existing development profiles retain Broad until an explicit policy selection.
    // Fresh databases (no source/entry/settings state) use Smart Lean at first read.
    conn.execute("INSERT INTO app_settings(key,value)
        SELECT 'global_index_coverage_v1', '{\"version\":1,\"revision\":1,\"mode\":\"broad_coverage\",\"rules\":[]}'
        WHERE EXISTS(SELECT 1 FROM global_volumes) OR EXISTS(SELECT 1 FROM global_entries)
        ON CONFLICT(key) DO NOTHING", [])?;
    Ok(())
}
