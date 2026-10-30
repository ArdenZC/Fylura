use super::super::models::GlobalSearchResult;
use rusqlite::{Connection, OptionalExtension};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap, HashSet};
use std::io;
use std::path::Path;

#[derive(Clone)]
pub(super) struct OverlayRow {
    pub(super) id: String,
    pub(super) volume_id: String,
    pub(super) name_normalized: String,
    pub(super) extension: String,
    pub(super) modified_at: Option<i64>,
    pub(super) is_stale: bool,
}

#[derive(Clone)]
pub(super) struct RankedCandidate<'a> {
    pub(super) class: u8,
    pub(super) modified_at: Option<i64>,
    pub(super) id: Cow<'a, str>,
}

impl PartialEq for RankedCandidate<'_> {
    fn eq(&self, other: &Self) -> bool {
        compare_candidate(self, other) == Ordering::Equal
    }
}
impl Eq for RankedCandidate<'_> {}
impl PartialOrd for RankedCandidate<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for RankedCandidate<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_candidate(self, other)
    }
}

// BinaryHeap keeps the worst item at its root, so this comparator is the
// inverse of the user-visible order after class: newest mtime, then ID ASC.
fn compare_candidate(left: &RankedCandidate<'_>, right: &RankedCandidate<'_>) -> Ordering {
    left.class
        .cmp(&right.class)
        .then_with(|| right.modified_at.cmp(&left.modified_at))
        .then_with(|| left.id.cmp(&right.id))
}

pub(super) fn push_candidate<'a>(
    heap: &mut BinaryHeap<RankedCandidate<'a>>,
    capacity: usize,
    candidate: RankedCandidate<'a>,
) {
    if capacity == 0 {
        return;
    }
    if heap.len() < capacity {
        heap.push(candidate);
    } else if heap
        .peek()
        .is_some_and(|worst| compare_candidate(&candidate, worst) == Ordering::Less)
    {
        heap.pop();
        heap.push(candidate);
    }
}

pub(super) fn supported_query(query: &str) -> bool {
    !query.is_empty()
        && query.len() <= 256
        && query.chars().count() <= 256
        && query.chars().all(|character| !character.is_control())
}

pub(super) fn add_path_candidates(
    rows: impl Iterator<Item = rusqlite::Result<(String, Option<i64>)>>,
    heap: &mut BinaryHeap<RankedCandidate<'_>>,
    target: usize,
    already_seen: &HashSet<String>,
) -> io::Result<u64> {
    let mut count = 0u64;
    for row in rows {
        let (id, modified_at) = row.map_err(io::Error::other)?;
        if already_seen.contains(&id) {
            continue;
        }
        count += 1;
        push_candidate(
            heap,
            target,
            RankedCandidate {
                class: 2,
                modified_at,
                id: Cow::Owned(id),
            },
        );
    }
    Ok(count)
}

fn word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Tests every occurrence so a later bounded occurrence is never hidden by
/// an earlier substring. ASCII candidate queries may still touch Unicode names.
pub(super) fn has_bounded_occurrence(name: &str, query: &str) -> bool {
    name.match_indices(query).any(|(start, matched)| {
        let before = name[..start].chars().next_back();
        let after = name[start + matched.len()..].chars().next();
        !before.is_some_and(word_character) && !after.is_some_and(word_character)
    })
}

pub(super) fn match_name(name: &str, query: &str) -> Option<u8> {
    if name.contains(query) {
        Some(if has_bounded_occurrence(name, query) {
            1
        } else {
            2
        })
    } else {
        None
    }
}

pub(super) fn is_safe_file_name(value: &str) -> bool {
    !value.is_empty()
        && Path::new(value).file_name().and_then(|name| name.to_str()) == Some(value)
        && !value.contains(['/', '\\'])
}

pub(super) fn load_active_sources(conn: &Connection) -> io::Result<HashSet<String>> {
    let mut statement = conn
        .prepare(
            "SELECT id FROM global_volumes
             WHERE enabled=1 AND coverage_pending=0",
        )
        .map_err(io::Error::other)?;
    let sources = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(io::Error::other)?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(io::Error::other)?;
    Ok(sources)
}

pub(super) fn change_tracking_ready(conn: &Connection) -> io::Result<bool> {
    let mut statement = conn
        .prepare(
            "SELECT name, sql FROM sqlite_master
             WHERE type='trigger' AND name IN (
                'compact_name_search_entries_ai',
                'compact_name_search_entries_au',
                'compact_name_search_entries_ad',
                'compact_name_search_changes_cap'
             )",
        )
        .map_err(io::Error::other)?;
    let triggers = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            ))
        })
        .map_err(io::Error::other)?
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(io::Error::other)?;
    let normalized = triggers
        .into_iter()
        .map(|(name, sql)| {
            (
                name,
                sql.split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_ascii_lowercase(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let has = |name: &str, required: &[&str]| {
        normalized
            .get(name)
            .is_some_and(|sql| required.iter().all(|part| sql.contains(part)))
    };
    Ok(has(
        "compact_name_search_entries_ai",
        &[
            "after insert on global_entries",
            "insert into compact_name_search_changes(entry_rowid) values (new.rowid)",
        ],
    ) && has(
        "compact_name_search_entries_au",
        &[
            "after update on global_entries",
            "old.rowid is not new.rowid",
            "old.name_normalized is not new.name_normalized",
            "old.rowid, old.id",
            "values (new.rowid)",
        ],
    ) && has(
        "compact_name_search_entries_ad",
        &[
            "after delete on global_entries",
            "insert into compact_name_search_changes(entry_rowid, entry_id)",
            "values (old.rowid, old.id)",
        ],
    ) && has(
        "compact_name_search_changes_cap",
        &[
            "after insert on compact_name_search_changes",
            "new.seq > (select change_floor from compact_name_search_state where singleton=1) + 65536",
            "base_generation=-1",
            "delete from compact_name_search_changes where seq < new.seq",
        ],
    ))
}

pub(super) fn ensure_change_schema(conn: &Connection) -> io::Result<()> {
    let state_columns = table_columns(conn, "compact_name_search_state")?;
    let missing_change_floor = !state_columns.contains("change_floor");
    if missing_change_floor {
        conn.execute_batch(
            "ALTER TABLE compact_name_search_state
             ADD COLUMN change_floor INTEGER NOT NULL DEFAULT 0",
        )
        .map_err(io::Error::other)?;
    }

    let change_columns = table_columns(conn, "compact_name_search_changes")?;
    let legacy_change_table =
        !change_columns.contains("entry_rowid") || !change_columns.contains("entry_id");
    let mut migrated = missing_change_floor || legacy_change_table;
    if legacy_change_table {
        let legacy_generation: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM compact_name_search_changes",
                [],
                |row| row.get(0),
            )
            .map_err(io::Error::other)?;
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS compact_name_search_entries_ai;
             DROP TRIGGER IF EXISTS compact_name_search_entries_au;
             DROP TRIGGER IF EXISTS compact_name_search_entries_ad;
             DROP TRIGGER IF EXISTS compact_name_search_changes_cap;
             DROP TABLE compact_name_search_changes;
             CREATE TABLE compact_name_search_changes (
                 seq INTEGER PRIMARY KEY,
                 entry_rowid INTEGER NOT NULL,
                 entry_id TEXT
             );",
        )
        .map_err(io::Error::other)?;
        if legacy_generation > 0 {
            conn.execute(
                "INSERT INTO compact_name_search_changes(seq, entry_rowid)
                 VALUES (?1, 0)",
                [legacy_generation],
            )
            .map_err(io::Error::other)?;
        }
        conn.execute(
            "UPDATE compact_name_search_state
             SET base_generation=-1, file_name=NULL, change_floor=?1
             WHERE singleton=1",
            [legacy_generation],
        )
        .map_err(io::Error::other)?;
    }

    let state: Option<(i64, i64)> = conn
        .query_row(
            "SELECT schema_version, base_generation
             FROM compact_name_search_state WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(io::Error::other)?;
    if let Some((version, _)) = state {
        if version != 1 && version != 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "compact_name_search_schema_version_mismatch",
            ));
        }
        if version != 2 || missing_change_floor {
            let latest_generation: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(seq), 0) FROM compact_name_search_changes",
                    [],
                    |row| row.get(0),
                )
                .map_err(io::Error::other)?;
            conn.execute(
                "UPDATE compact_name_search_state
                 SET schema_version=2, base_generation=-1, file_name=NULL,
                     change_floor=?1 WHERE singleton=1",
                [latest_generation],
            )
            .map_err(io::Error::other)?;
            migrated = true;
        }
    } else {
        conn.execute(
            "INSERT INTO compact_name_search_state(
                 singleton, schema_version, format_version, base_generation, file_name, change_floor
             ) VALUES (1, 2, 1, -1, NULL, 0)",
            [],
        )
        .map_err(io::Error::other)?;
    }

    let latest_generation: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM compact_name_search_changes",
            [],
            |row| row.get(0),
        )
        .map_err(io::Error::other)?;
    let (base_generation, change_floor): (i64, i64) = conn
        .query_row(
            "SELECT base_generation, change_floor FROM compact_name_search_state WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(io::Error::other)?;
    let invalid_base = base_generation >= 0
        && (latest_generation < base_generation || base_generation < change_floor);
    let oversized_feed = latest_generation.saturating_sub(change_floor) > 65_536;
    if migrated || invalid_base || oversized_feed {
        conn.execute(
            "UPDATE compact_name_search_state
             SET base_generation=-1, file_name=NULL, change_floor=?1
             WHERE singleton=1",
            [if oversized_feed {
                latest_generation.saturating_sub(1)
            } else {
                latest_generation
            }],
        )
        .map_err(io::Error::other)?;
        if oversized_feed && latest_generation > 0 {
            conn.execute(
                "DELETE FROM compact_name_search_changes WHERE seq < ?1",
                [latest_generation],
            )
            .map_err(io::Error::other)?;
        }
    }
    Ok(())
}

fn table_columns(conn: &Connection, table: &str) -> io::Result<HashSet<String>> {
    let sql = format!("PRAGMA table_info({table})");
    let mut statement = conn.prepare(&sql).map_err(io::Error::other)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(io::Error::other)?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(io::Error::other)?;
    Ok(columns)
}

pub(super) fn load_overlay_rows(
    conn: &Connection,
    changed_ids: &[String],
) -> io::Result<Vec<OverlayRow>> {
    if changed_ids.is_empty() {
        return Ok(Vec::new());
    }
    let json = serde_json::to_string(changed_ids).map_err(io::Error::other)?;
    let sql = r#"
        SELECT ge.id, ge.volume_id, ge.name_normalized, ge.extension,
               ge.modified_at_fs, ge.is_stale
        FROM global_entries ge
        WHERE ge.id IN (SELECT value FROM json_each(?1))
    "#;
    let mut statement = conn.prepare(sql).map_err(io::Error::other)?;
    let rows = statement
        .query_map([json], |row| {
            Ok(OverlayRow {
                id: row.get(0)?,
                volume_id: row.get(1)?,
                name_normalized: row.get(2)?,
                extension: row.get(3)?,
                modified_at: row.get(4)?,
                is_stale: row.get::<_, i64>(5)? != 0,
            })
        })
        .map_err(io::Error::other)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(io::Error::other)?;
    Ok(rows)
}

pub(super) fn hydrate_page(
    conn: &Connection,
    ids: &[&str],
    ranked: &[RankedCandidate<'_>],
) -> io::Result<Vec<GlobalSearchResult>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = (1..=ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        r#"
        SELECT ge.id, ge.volume_id, ge.platform_file_id, ge.name, ge.path,
               ge.extension, ge.is_directory, ge.size, ge.created_at_fs,
               ge.modified_at_fs, ge.file_attributes, ge.is_hidden, ge.is_system,
               ge.source_provider,
               EXISTS (
                   SELECT 1 FROM managed_entries me
                   JOIN managed_scopes ms ON ms.id = me.managed_scope_id
                   WHERE me.global_entry_id = ge.id AND me.enabled = 1 AND ms.enabled = 1
               ) AS managed,
               0.0 AS rank
        FROM global_entries ge
        JOIN global_volumes gv ON gv.id = ge.volume_id
        WHERE ge.id IN ({placeholders})
          AND ge.is_stale=0 AND gv.enabled=1 AND gv.coverage_pending=0
    "#
    );
    let mut statement = conn.prepare(&sql).map_err(io::Error::other)?;
    let mut by_id = statement
        .query_map(
            rusqlite::params_from_iter(ids.iter().copied()),
            super::super::search::map_global_search_result,
        )
        .map_err(io::Error::other)?
        .map(|result| result.map(|entry| (entry.id.clone(), entry)))
        .collect::<Result<std::collections::HashMap<_, _>, _>>()
        .map_err(io::Error::other)?;
    if by_id.len() != ids.len() || ids.len() != ranked.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "compact_name_search_hydration_incomplete",
        ));
    }
    let mut output = Vec::with_capacity(ids.len());
    for candidate in ranked {
        let mut result = by_id.remove(candidate.id.as_ref()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "compact_name_search_hydration_missing_id",
            )
        })?;
        result.rank = f64::from(candidate.class);
        output.push(result);
    }
    Ok(output)
}
