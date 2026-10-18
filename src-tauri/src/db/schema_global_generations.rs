//! Schema 42 is dormant: no legacy table, trigger or reader is redirected.
use super::DbError;
use rusqlite::Connection;

pub(super) const DDL: &str = include_str!("schema_global_generations.sql");

pub(super) fn install(conn: &Connection) -> Result<(), DbError> {
    // No IF NOT EXISTS: a partial or foreign installation must roll back.
    conn.execute_batch(DDL)?;
    conn.execute(
        "INSERT INTO global_index_generation_heads(volume_id) SELECT id FROM global_volumes",
        [],
    )?;
    validate(conn)
}

pub(super) fn validate(conn: &Connection) -> Result<(), DbError> {
    // Compare the complete owned namespace, including FTS shadow tables and
    // triggers. This validates constraints/index definitions, not just columns.
    // No corpus read, backfill, FTS rebuild or application wake occurs here.
    fn signature(conn: &Connection) -> Result<Vec<(String, String, String)>, DbError> {
        let mut stmt = conn.prepare(
            "SELECT type, name, COALESCE(sql, '') FROM sqlite_schema
             WHERE name GLOB 'global_index_generation*'
                OR tbl_name GLOB 'global_index_generation*' ORDER BY type, name",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
    }
    let expected = Connection::open_in_memory()?;
    expected.execute_batch(DDL)?;
    if signature(conn)? != signature(&expected)? {
        return Err(DbError::Validation(
            "global_index_schema42_generation_signature_conflict".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn drop_for_test(conn: &Connection) {
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS global_index_generation_entries_ai;
         DROP TRIGGER IF EXISTS global_index_generation_entries_insert_guard;
         DROP TRIGGER IF EXISTS global_index_generation_entries_no_update;
         DROP TRIGGER IF EXISTS global_index_generation_entries_no_delete;
         DROP TABLE IF EXISTS global_index_generation_fts;
         DROP TABLE IF EXISTS global_index_generation_entries;
         DROP TABLE IF EXISTS global_index_generation_heads;
         DROP TABLE IF EXISTS global_index_generations;",
    )
    .expect("remove only Schema42 objects for a historical fixture");
}

#[cfg(test)]
mod tests;
