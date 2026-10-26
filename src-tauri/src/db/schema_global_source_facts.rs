//! Exact Global Index facts, maintained by the entries writer transaction.
//! Independent of the legacy, externally writable global_volumes.entry_count.
use super::DbError;
use rusqlite::{Connection, OptionalExtension};

pub(crate) const DDL: &str = include_str!("schema_global_source_facts.sql");
const OBJECTS: [&str; 6] = [
    "global_entry_source_facts",
    "global_entry_source_facts_ai",
    "global_entry_source_facts_ad",
    "global_entry_source_facts_au",
    "global_entry_source_facts_volume_ai",
    "global_entry_source_facts_volume_au",
];

pub(crate) fn install(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(DDL)?;
    validate(conn)?;
    // One migration/explicit repair scan, never an ordinary startup/read scan.
    conn.execute_batch(
        "DELETE FROM global_entry_source_facts;
        INSERT INTO global_entry_source_facts(volume_id, active_rows, active_ids, max_last_seen_at)
        SELECT volume_id, COUNT(*), COUNT(id), MAX(last_seen_at)
        FROM global_entries WHERE is_stale = 0 GROUP BY volume_id;
        INSERT INTO global_entry_source_facts(volume_id,active_rows,active_ids,max_last_seen_at)
        SELECT id,0,0,NULL FROM global_volumes WHERE true ON CONFLICT(volume_id) DO NOTHING;",
    )?;
    Ok(())
}

pub(crate) fn validate(conn: &Connection) -> Result<(), DbError> {
    let normalize = |sql: &str| {
        sql.chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase()
            .replace("ifnotexists", "")
    };
    for (name, definition) in OBJECTS.iter().zip(DDL.split("-- @object").skip(1)) {
        let sql: Option<String> = conn
            .query_row(
                "SELECT sql FROM main.sqlite_schema WHERE name=?1",
                [name],
                |r| r.get(0),
            )
            .optional()?;
        if sql.as_deref().map(normalize) != Some(normalize(definition.trim().trim_end_matches(';')))
        {
            return Err(DbError::Validation(format!(
                "global_source_facts_schema_conflict:{name}"
            )));
        }
    }
    Ok(())
}

/// TEMP fixture/shadow entries are a different relation. Never apply main facts
/// to them. Such connections retain the canonical COUNT(id)/COUNT(*) read path.
pub(crate) fn materialized_relation_available(conn: &Connection) -> Result<bool, DbError> {
    Ok(!conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_temp_schema WHERE name='global_entries')",
        [],
        |r| r.get::<_, bool>(0),
    )?)
}

#[cfg(test)]
#[path = "schema_global_source_facts_tests.rs"]
mod tests;
