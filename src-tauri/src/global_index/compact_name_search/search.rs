use super::query::{
    add_path_candidates, change_tracking_ready, hydrate_page, is_safe_file_name,
    load_active_sources, load_overlay_rows, match_name, push_candidate, RankedCandidate,
};
use super::*;
use rusqlite::{params, Connection};
use std::borrow::Cow;
use std::collections::BinaryHeap;
use std::sync::Arc;

impl CompactNameSearchManager {
    pub(super) fn search_page_inner(
        self: &Arc<Self>,
        conn: &Connection,
        query: &str,
        target: usize,
        page_skip: usize,
        page_len: usize,
        already_seen: &HashSet<String>,
    ) -> io::Result<Option<SearchPageOutcome>> {
        let lock = self.lock_file()?;
        lock.lock_shared()?;
        let manifest_overlay_started = Instant::now();

        let manifest: Option<(i64, i64, i64, i64, Option<String>)> = conn
            .query_row(
                "SELECT schema_version, format_version, base_generation, change_floor, file_name
                 FROM compact_name_search_state WHERE singleton=1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .map_err(io::Error::other)?;
        let Some((schema_version, format_version, base_generation, change_floor, Some(file_name))) =
            manifest
        else {
            return Ok(None);
        };
        if schema_version != SCHEMA_VERSION
            || format_version != FORMAT_VERSION as i64
            || base_generation < 0
            || base_generation < change_floor
            || !is_safe_file_name(&file_name)
        {
            return Ok(None);
        }
        if !change_tracking_ready(conn).map_err(io::Error::other)? {
            return Ok(None);
        }
        let index = match self.index_for_manifest(&file_name, base_generation) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };
        let latest_generation: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM compact_name_search_changes",
                [],
                |row| row.get(0),
            )
            .map_err(io::Error::other)?;
        if latest_generation < base_generation {
            return Ok(None);
        }
        let event_rows = conn
            .prepare(
                "SELECT seq, event_count,
                        batch_ids IS NOT NULL,
                        CASE WHEN batch_ids IS NULL THEN 0 ELSE json_array_length(batch_ids) END
                 FROM compact_name_search_changes WHERE seq > ?1 ORDER BY seq",
            )
            .map_err(io::Error::other)?
            .query_map([base_generation], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(io::Error::other)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(io::Error::other)?;
        let mut next_sequence = base_generation.saturating_add(1);
        let mut overlay_event_count = 0u64;
        for (sequence_end, event_count, is_batch, encoded_count) in event_rows {
            if event_count <= 0
                || (is_batch && encoded_count != event_count)
                || (!is_batch && event_count != 1)
                || sequence_end.saturating_sub(event_count).saturating_add(1) != next_sequence
            {
                return Ok(None);
            }
            overlay_event_count = overlay_event_count.saturating_add(event_count as u64);
            if overlay_event_count > MAX_OVERLAY_EVENTS {
                return Ok(None);
            }
            next_sequence = sequence_end.saturating_add(1);
        }
        if next_sequence != latest_generation.saturating_add(1)
            || overlay_event_count != latest_generation.saturating_sub(base_generation) as u64
        {
            return Ok(None);
        }
        let cached_overlay = self
            .overlay_cache
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .filter(|cached| {
                cached.file_name == file_name
                    && cached.base_generation == base_generation
                    && cached.latest_generation == latest_generation
            })
            .map(|cached| (Arc::clone(&cached.changed_ids), Arc::clone(&cached.rows)));
        let (changed_id_set, overlay_rows) = if let Some(cached) = cached_overlay {
            cached
        } else {
            let mut changed_ids = conn
                .prepare(
                    "SELECT DISTINCT changed_id FROM (
                        SELECT COALESCE(c.entry_id, ge.id) AS changed_id
                        FROM compact_name_search_changes c
                        LEFT JOIN global_entries ge ON ge.rowid=c.entry_rowid
                        WHERE c.seq > ?1 AND c.batch_ids IS NULL
                        UNION ALL
                        SELECT CAST(j.value AS TEXT) AS changed_id
                        FROM compact_name_search_changes c, json_each(c.batch_ids) AS j
                        WHERE c.seq > ?1 AND c.batch_ids IS NOT NULL
                     ) WHERE changed_id IS NOT NULL
                     ORDER BY changed_id COLLATE BINARY",
                )
                .map_err(io::Error::other)?
                .query_map([base_generation], |row| row.get::<_, String>(0))
                .map_err(io::Error::other)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(io::Error::other)?;
            if changed_ids.len() > MAX_OVERLAY_IDS {
                return Ok(None);
            }
            changed_ids.sort_unstable();
            let changed_id_set = Arc::new(changed_ids.iter().cloned().collect::<HashSet<_>>());
            let overlay_rows = Arc::new(load_overlay_rows(conn, &changed_ids)?);
            let cache_bytes = changed_ids.iter().map(String::len).sum::<usize>()
                + overlay_rows
                    .iter()
                    .map(|row| {
                        row.id.len()
                            + row.volume_id.len()
                            + row.name_normalized.len()
                            + row.extension.len()
                    })
                    .sum::<usize>();
            if changed_ids.len() <= 4_096 && cache_bytes <= 4 * 1024 * 1024 {
                *self
                    .overlay_cache
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(CachedOverlay {
                    file_name: file_name.clone(),
                    base_generation,
                    latest_generation,
                    changed_ids: Arc::clone(&changed_id_set),
                    rows: Arc::clone(&overlay_rows),
                });
            }
            (changed_id_set, overlay_rows)
        };
        let active_sources = load_active_sources(conn)?;
        let mut timings = SearchTimings {
            manifest_overlay_micros: elapsed_micros(manifest_overlay_started),
            ..SearchTimings::default()
        };

        let normalized_query = query.to_lowercase();
        let name_prefilter_started = Instant::now();
        let name_classes = index.matching_names(&normalized_query);
        let extension_classes = index.matching_extensions(&normalized_query);
        timings.name_prefilter_micros = elapsed_micros(name_prefilter_started);
        let mut heap = BinaryHeap::with_capacity(target.min(4096));
        let mut name_candidate_count = 0u64;
        let name_scan_started = Instant::now();

        for (name_id, &class) in name_classes.iter().enumerate() {
            if class == NO_MATCH {
                continue;
            }
            let (start, end) = index.name_posting_range(name_id);
            for posting in start..end {
                let entry_index = index.u32_at(index.offsets[4], posting);
                if index.entry_flag(entry_index, FLAG_STALE) {
                    continue;
                }
                let source = index.entry_source(entry_index);
                if !active_sources.contains(index.source(source as usize)) {
                    continue;
                }
                let id = index.entry_id(entry_index);
                if changed_id_set.contains(id) || already_seen.contains(id) {
                    continue;
                }
                name_candidate_count += 1;
                push_candidate(
                    &mut heap,
                    target,
                    RankedCandidate {
                        class: class - 1,
                        modified_at: index.entry_mtime(entry_index),
                        id: Cow::Borrowed(id),
                    },
                );
            }
        }

        for (extension_id, &extension_class) in extension_classes.iter().enumerate() {
            if extension_class == NO_MATCH {
                continue;
            }
            let (start, end) = index.extension_posting_range(extension_id);
            for posting in start..end {
                let entry_index = index.u32_at(index.offsets[10], posting);
                if index.entry_flag(entry_index, FLAG_STALE)
                    || name_classes[index.entry_name(entry_index) as usize] != NO_MATCH
                {
                    continue;
                }
                let source = index.entry_source(entry_index);
                if !active_sources.contains(index.source(source as usize)) {
                    continue;
                }
                let id = index.entry_id(entry_index);
                if changed_id_set.contains(id) || already_seen.contains(id) {
                    continue;
                }
                name_candidate_count += 1;
                push_candidate(
                    &mut heap,
                    target,
                    RankedCandidate {
                        class: 1,
                        modified_at: index.entry_mtime(entry_index),
                        id: Cow::Borrowed(id),
                    },
                );
            }
        }

        for row in overlay_rows.iter() {
            if row.is_stale
                || !active_sources.contains(row.volume_id.as_str())
                || already_seen.contains(&row.id)
            {
                continue;
            }
            let class = if let Some(class) = match_name(&row.name_normalized, &normalized_query) {
                class
            } else if row.extension.contains(&normalized_query) {
                2
            } else {
                continue;
            };
            name_candidate_count += 1;
            push_candidate(
                &mut heap,
                target,
                RankedCandidate {
                    class: class - 1,
                    modified_at: row.modified_at,
                    id: Cow::Borrowed(&row.id),
                },
            );
        }
        timings.name_scan_heap_micros = elapsed_micros(name_scan_started);

        let mut path_candidate_count = 0u64;
        let path_scan_started = Instant::now();
        if heap.len() < target {
            let fts_query = format!("\"{}\"", normalized_query.replace('"', "\"\""));
            let fts_safe = normalized_query.chars().count() >= 3
                && normalized_query
                    .chars()
                    .all(|character| character.is_alphanumeric() || character.is_whitespace());
            let sql = if fts_safe {
                r#"
                    SELECT ge.id, ge.modified_at_fs
                    FROM global_entries_fts
                    CROSS JOIN global_entries ge
                    CROSS JOIN global_volumes gv
                    WHERE global_entries_fts MATCH ?1
                      AND ge.rowid = global_entries_fts.rowid
                      AND gv.id = ge.volume_id
                      AND gv.enabled = 1
                      AND gv.coverage_pending = 0
                      AND ge.is_stale = 0
                      AND instr(ge.path_normalized, ?2) > 0
                      AND instr(ge.name_normalized, ?2) = 0
                      AND instr(ge.extension, ?2) = 0
                "#
            } else {
                r#"
                    SELECT ge.id, ge.modified_at_fs
                    FROM global_entries ge
                    JOIN global_volumes gv ON gv.id = ge.volume_id
                    WHERE gv.enabled = 1
                      AND gv.coverage_pending = 0
                      AND ge.is_stale = 0
                      AND instr(ge.path_normalized, ?1) > 0
                      AND instr(ge.name_normalized, ?1) = 0
                      AND instr(ge.extension, ?1) = 0
                "#
            };
            let mut statement = conn.prepare(sql).map_err(io::Error::other)?;
            path_candidate_count = if fts_safe {
                let rows = statement
                    .query_map(params![fts_query, normalized_query], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?))
                    })
                    .map_err(io::Error::other)?;
                add_path_candidates(rows, &mut heap, target, already_seen)?
            } else {
                let rows = statement
                    .query_map([&normalized_query], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?))
                    })
                    .map_err(io::Error::other)?;
                add_path_candidates(rows, &mut heap, target, already_seen)?
            };
        }
        timings.path_scan_heap_micros = elapsed_micros(path_scan_started);

        if heap.len() < target {
            // An incomplete feed cannot prove absence. Require the observed
            // event range to have begun strictly after the mapped base.
            let first_event: Option<i64> = conn
                .query_row(
                    "SELECT MIN(seq) FROM compact_name_search_changes WHERE seq > ?1",
                    [base_generation],
                    |row| row.get(0),
                )
                .map_err(io::Error::other)?;
            if latest_generation > base_generation && first_event.is_none() {
                return Ok(None);
            }
        }

        let top_k_sort_started = Instant::now();
        let mut ranked = heap.into_vec();
        ranked.sort();
        let selected = ranked
            .into_iter()
            .skip(page_skip)
            .take(page_len)
            .collect::<Vec<_>>();
        timings.top_k_sort_page_micros = elapsed_micros(top_k_sort_started);
        let ids = selected
            .iter()
            .map(|candidate| candidate.id.as_ref())
            .collect::<Vec<_>>();
        let hydration_started = Instant::now();
        let hydrated = hydrate_page(conn, &ids, &selected)?;
        timings.page_hydration_micros = elapsed_micros(hydration_started);
        Ok(Some(SearchPageOutcome {
            page: hydrated,
            overlay_events: overlay_event_count,
            name_candidates: name_candidate_count,
            path_candidates: path_candidate_count,
            timings,
        }))
    }
}
