//! Durable, bounded change feed for the disposable Global Search view.
//!
//! The main `user_version` migration chain is currently shared with an
//! independent Global Index authorization change. This additive sidecar has
//! its own schema revision and does not change Global Index file/source
//! authority or `user_version`. It is installed atomically after the main
//! schema is ready; failure leaves the existing SQLite search available.

use super::DbError;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

pub(crate) const DDL: &str = include_str!("schema_global_search_changes.sql");
pub(crate) const MAX_RETAINED_CHANGES: i64 = 100_000;
const REQUIRED_TRIGGERS: [&str; 6] = [
    "global_search_changes_entry_ai",
    "global_search_changes_entry_ad",
    "global_search_changes_entry_au",
    "global_search_changes_source_ai",
    "global_search_changes_source_ad",
    "global_search_changes_source_au",
];
const AUTHORIZATION_TRIGGER: &str = r#"
CREATE TRIGGER IF NOT EXISTS global_search_changes_scope_authorization_au
AFTER UPDATE ON global_volumes
WHEN OLD.scope_authorization_state IS NOT NEW.scope_authorization_state
BEGIN
    UPDATE global_search_change_state
    SET generation = generation + 1,
        event_count = event_count + CASE
            WHEN feed_saturated = 0 AND event_count < 100000 THEN 1 ELSE 0 END,
        feed_saturated = CASE
            WHEN feed_saturated = 1 THEN 1
            WHEN event_count >= 100000 THEN 1 ELSE 0 END
    WHERE singleton_id = 1;
    INSERT INTO global_search_changes(change_kind, entity_id, source_id, generation)
    SELECT 'invalidate', 'all', NEW.id, generation
    FROM global_search_change_state
    WHERE singleton_id = 1 AND feed_saturated = 0
    ;
END;
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChangeState {
    pub(crate) generation: i64,
    pub(crate) feed_floor_generation: i64,
    pub(crate) event_count: i64,
    pub(crate) feed_saturated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    Entry,
    Invalidate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChangeEvent {
    pub(crate) kind: ChangeKind,
    pub(crate) entity_id: String,
    pub(crate) source_id: Option<String>,
    pub(crate) generation: i64,
}

/// Install or validate the optional search-view tables/triggers. All objects
/// and the initial state row commit together. The file facts are never scanned
/// or copied during installation.
pub(crate) fn install(conn: &Connection) -> Result<(), DbError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(DDL)?;
    tx.execute(
        "INSERT OR IGNORE INTO global_search_change_state(singleton_id, schema_version) VALUES (1, 1)",
        [],
    )?;
    if table_has_column(&tx, "global_volumes", "scope_authorization_state")? {
        tx.execute_batch(AUTHORIZATION_TRIGGER)?;
    }
    validate(&tx)?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn validate(conn: &Connection) -> Result<(), DbError> {
    if !relation_exists(conn, "table", "global_search_change_state")?
        || !relation_exists(conn, "table", "global_search_changes")?
        || !relation_exists(conn, "index", "idx_global_search_changes_generation")?
    {
        return Err(DbError::Validation(
            "global_search_change_schema_missing".into(),
        ));
    }
    let object_names = [
        "global_search_change_state",
        "global_search_changes",
        "idx_global_search_changes_generation",
        "global_search_changes_entry_ai",
        "global_search_changes_entry_ad",
        "global_search_changes_entry_au",
        "global_search_changes_source_ai",
        "global_search_changes_source_ad",
        "global_search_changes_source_au",
    ];
    let definitions = DDL.split("-- @object").skip(1);
    for (name, definition) in object_names.into_iter().zip(definitions) {
        let actual: Option<String> = conn
            .query_row(
                "SELECT sql FROM main.sqlite_schema WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()?;
        if actual.as_deref().map(schema_signature)
            != Some(schema_signature(definition.trim().trim_end_matches(';')))
        {
            return Err(DbError::Validation(format!(
                "global_search_change_schema_conflict:{name}"
            )));
        }
    }
    for name in REQUIRED_TRIGGERS {
        if !relation_exists(conn, "trigger", name)? {
            return Err(DbError::Validation(format!(
                "global_search_change_trigger_missing:{name}"
            )));
        }
    }
    let has_authorization = table_has_column(conn, "global_volumes", "scope_authorization_state")?;
    let authorization_trigger = relation_sql(conn, "global_search_changes_scope_authorization_au")?;
    if has_authorization != authorization_trigger.is_some()
        || authorization_trigger.as_deref().map(schema_signature)
            != has_authorization.then_some(schema_signature(AUTHORIZATION_TRIGGER))
    {
        return Err(DbError::Validation(
            "global_search_change_authorization_trigger_mismatch".into(),
        ));
    }
    let state = read_state(conn)?;
    if state.event_count > MAX_RETAINED_CHANGES
        || state.generation < state.feed_floor_generation
        || (!state.feed_saturated
            && state.generation - state.feed_floor_generation != state.event_count)
        || (state.feed_saturated && state.event_count != MAX_RETAINED_CHANGES)
    {
        return Err(DbError::Validation(
            "global_search_change_state_invalid".into(),
        ));
    }
    let actual_events: i64 =
        conn.query_row("SELECT COUNT(*) FROM global_search_changes", [], |row| {
            row.get(0)
        })?;
    let (minimum_generation, maximum_generation): (Option<i64>, Option<i64>) = conn.query_row(
        "SELECT MIN(generation), MAX(generation) FROM global_search_changes",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let schema_version: i64 = conn.query_row(
        "SELECT schema_version FROM global_search_change_state WHERE singleton_id = 1",
        [],
        |row| row.get(0),
    )?;
    if state.event_count != actual_events || schema_version != 1 {
        return Err(DbError::Validation(
            "global_search_change_state_invalid".into(),
        ));
    }
    if state.event_count > 0
        && (minimum_generation != Some(state.feed_floor_generation + 1)
            || maximum_generation != Some(state.feed_floor_generation + state.event_count))
    {
        return Err(DbError::Validation(
            "global_search_change_generation_gap".into(),
        ));
    }
    Ok(())
}

pub(crate) fn read_state(conn: &Connection) -> Result<ChangeState, DbError> {
    conn.query_row(
        "SELECT generation, feed_floor_generation, event_count, feed_saturated FROM global_search_change_state WHERE singleton_id = 1",
        [],
        |row| {
            Ok(ChangeState {
                generation: row.get(0)?,
                feed_floor_generation: row.get(1)?,
                event_count: row.get(2)?,
                feed_saturated: row.get::<_, i64>(3)? != 0,
            })
        },
    )
    .optional()?
    .ok_or_else(|| DbError::Validation("global_search_change_state_missing".into()))
}

pub(crate) fn changes_since(
    conn: &Connection,
    generation: i64,
    limit: usize,
) -> Result<Vec<ChangeEvent>, DbError> {
    let mut statement = conn.prepare(
        "SELECT change_kind, entity_id, source_id, generation FROM global_search_changes WHERE generation > ?1 ORDER BY generation ASC LIMIT ?2",
    )?;
    let events = statement
        .query_map((generation, limit as i64), |row| {
            let kind: String = row.get(0)?;
            Ok(ChangeEvent {
                kind: match kind.as_str() {
                    "entry" => ChangeKind::Entry,
                    "invalidate" => ChangeKind::Invalidate,
                    _ => return Err(rusqlite::Error::InvalidQuery),
                },
                entity_id: row.get(1)?,
                source_id: row.get(2)?,
                generation: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(events)
}

/// A full rebuild can retire old feed entries only when its captured database
/// generation is still current under the SQLite writer lock. A concurrent
/// write leaves the old feed intact and the caller must apply or reject it.
pub(crate) fn reset_after_full_snapshot(
    conn: &mut Connection,
    snapshot_generation: i64,
) -> Result<bool, DbError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let state = read_state(&tx)?;
    if state.generation != snapshot_generation {
        tx.rollback()?;
        return Ok(false);
    }
    tx.execute("DELETE FROM global_search_changes", [])?;
    tx.execute(
        "UPDATE global_search_change_state SET feed_floor_generation = ?1, event_count = 0, feed_saturated = 0 WHERE singleton_id = 1",
        [snapshot_generation],
    )?;
    tx.commit()?;
    Ok(true)
}

#[cfg(test)]
#[path = "schema_global_search_changes_tests.rs"]
mod tests;

fn relation_exists(conn: &Connection, relation_type: &str, name: &str) -> Result<bool, DbError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.sqlite_schema WHERE type = ?1 AND name = ?2)",
        (relation_type, name),
        |row| row.get(0),
    )?)
}

fn relation_sql(conn: &Connection, name: &str) -> Result<Option<String>, DbError> {
    Ok(conn
        .query_row(
            "SELECT sql FROM main.sqlite_schema WHERE name = ?1",
            [name],
            |row| row.get(0),
        )
        .optional()?)
}

fn schema_signature(sql: &str) -> String {
    sql.chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
        .replace("ifnotexists", "")
        .trim_end_matches(';')
        .to_string()
}

fn table_has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, DbError> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(columns.iter().any(|name| name == column))
}
