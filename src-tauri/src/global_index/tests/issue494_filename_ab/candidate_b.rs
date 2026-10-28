//! Candidate B: a disposable, snapshot-scoped, read-only in-memory filename
//! view rebuilt from SQLite facts.
//!
//! Design constraints taken from #494:
//!
//! * SQLite stays the only durable file/volume authority. Nothing here is
//!   persisted and no path is copied outside process memory.
//! * Names live in one contiguous UTF-8 buffer with `(offset, length)` spans, so
//!   a query is a bulk byte search plus a span lookup — no per-row `String`.
//! * Directory prefixes are interned once per distinct directory instead of
//!   duplicating a full path per file.
//! * Ranking is a bounded Top-K heap; the view never materialises every hit.
//! * The view is bound to one explicit, read-only SQLite snapshot. The view is
//!   never reused by a later transaction or a different connection. SQLite
//!   remains the authority; snapshot identity is in-memory lifetime state, not
//!   a persistent change generation.
//! * Every returned row is hydrated inside that same snapshot. A mismatch or a
//!   dropped/reclassified top-K candidate returns an empty fallback result;
//!   callers must answer through the authoritative SQL path.

use super::{extension_class, fold, proposed_class, sqlite_lower, Class, Hit, MAX_WINDOW};
use crate::db::DbError;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::Arc;

/// Upper bound for the incremental narrowing cache. A query that matched more
/// slots than this is not cached, because filtering it would cost more than a
/// fresh blob scan.
const NARROW_CACHE_CAPACITY: usize = 300_000;
/// Default RAM budget for the view. Overridable so the budget-degradation path
/// can be exercised without building a 1M fixture.
const DEFAULT_MEMORY_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// Build-time facts for the report only. These values are not used as a cache
/// generation or invalidation token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SourceSummary {
    pub entry_count: i64,
    pub enabled_volumes: i64,
    pub stale_entries: i64,
}

/// A bounded read transaction with a unique process-local identity.
///
/// `query_only` prevents writes through this connection while the view is
/// alive. Writers on other connections may commit concurrently, but SQLite
/// keeps this snapshot stable. A later transaction receives a different
/// identity, so an old view is rejected before candidate selection.
pub(super) struct ReadSnapshot<'conn> {
    conn: &'conn Connection,
    identity: Arc<()>,
    query_only_before: bool,
}

impl<'conn> ReadSnapshot<'conn> {
    pub(super) fn begin(conn: &'conn Connection) -> rusqlite::Result<Self> {
        let query_only_before: bool = conn.query_row("PRAGMA query_only", [], |row| row.get(0))?;
        conn.pragma_update(None, "query_only", true)?;
        if let Err(error) = conn.execute_batch("BEGIN DEFERRED") {
            let _ = conn.pragma_update(None, "query_only", query_only_before);
            return Err(error);
        }
        Ok(Self {
            conn,
            identity: Arc::new(()),
            query_only_before,
        })
    }

    pub(super) fn connection(&self) -> &Connection {
        self.conn
    }

    fn identity(&self) -> &Arc<()> {
        &self.identity
    }
}

impl Drop for ReadSnapshot<'_> {
    fn drop(&mut self) {
        let _ = self.conn.execute_batch("ROLLBACK");
        let _ = self
            .conn
            .pragma_update(None, "query_only", self.query_only_before);
    }
}

/// Build-time source summary, read once from the same snapshot as the view.
fn read_source_summary(conn: &Connection) -> rusqlite::Result<SourceSummary> {
    conn.query_row(
        "SELECT COUNT(*),
                (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1),
                (SELECT COUNT(*) FROM global_entries WHERE is_stale = 1)
         FROM global_entries ge",
        [],
        |row| {
            Ok(SourceSummary {
                entry_count: row.get(0)?,
                enabled_volumes: row.get(1)?,
                stale_entries: row.get(2)?,
            })
        },
    )
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub(super) struct BuildStats {
    pub build_ms: f64,
    pub rows: usize,
    pub distinct_directories: usize,
    pub distinct_extensions: usize,
    pub name_bytes: usize,
    pub directory_bytes: usize,
    pub id_bytes: usize,
    pub slot_bytes: usize,
    pub estimated_total_bytes: usize,
}

/// Why a lookup could not answer from RAM.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Fence {
    Valid,
    /// The view belongs to another transaction or connection.
    StaleSnapshot,
    /// Hydration did not reproduce the selected top-K facts exactly.
    HydrationMismatch,
    /// Building the view would exceed the configured RAM budget.
    BudgetExceeded,
}

pub(super) struct Lookup {
    pub fence: Fence,
    pub name_path: Vec<Hit>,
    pub extension: Vec<Hit>,
    /// Rows hydrated from SQLite and dropped because the fact changed,
    /// disappeared, went stale or lost its volume authorisation.
    pub hydrated_dropped: u64,
    /// True when the answer came from the incremental narrowing cache.
    pub narrowed: bool,
}

struct Span {
    start: u32,
    len: u32,
}

pub(super) struct CompactView {
    snapshot_identity: Arc<()>,
    view_identity: Arc<()>,
    source_summary: SourceSummary,
    names: String,
    name_span: Vec<Span>,
    directories: String,
    directory_span: Vec<Span>,
    slot_directory: Vec<u32>,
    ids: String,
    id_span: Vec<Span>,
    modified: Vec<i64>,
    extension_index: Vec<u16>,
    extensions: Vec<String>,
    stats: BuildStats,
}

pub(super) fn memory_budget_bytes() -> usize {
    std::env::var("ZC_ISSUE494_B_MEMORY_BUDGET_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MEMORY_BUDGET_BYTES)
}

/// Directory part of a normalized path, without the final component.
fn directory_prefix(path: &str) -> &str {
    match path.rfind(['\\', '/']) {
        Some(index) => &path[..index],
        None => "",
    }
}

/// One ranked slot. Ordering matches [`Hit`] exactly, but the id is compared as
/// a borrowed slice so a query that matches 50 000 names does not allocate
/// 50 000 `String` ids before the Top-K is even known.
struct SlotRank<'view> {
    view: &'view CompactView,
    slot: u32,
    class: Class,
}

impl PartialEq for SlotRank<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.slot == other.slot
    }
}

impl Eq for SlotRank<'_> {}

impl Ord for SlotRank<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.class
            .cmp(&other.class)
            .then_with(|| {
                super::modified_desc(
                    self.view.modified_at(self.slot as usize),
                    other.view.modified_at(other.slot as usize),
                )
            })
            .then_with(|| {
                self.view
                    .id_slice(self.slot as usize)
                    .cmp(other.view.id_slice(other.slot as usize))
            })
    }
}

impl PartialOrd for SlotRank<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

struct SlotTopK<'view> {
    limit: usize,
    heap: std::collections::BinaryHeap<SlotRank<'view>>,
}

impl<'view> SlotTopK<'view> {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            heap: std::collections::BinaryHeap::with_capacity(limit.min(1024)),
        }
    }

    fn offer(&mut self, view: &'view CompactView, slot: u32, class: Class) {
        if self.limit == 0 {
            return;
        }
        let candidate = SlotRank { view, slot, class };
        if self.heap.len() < self.limit {
            self.heap.push(candidate);
        } else if let Some(worst) = self.heap.peek() {
            if candidate < *worst {
                self.heap.pop();
                self.heap.push(candidate);
            }
        }
    }

    fn finish(self, view: &CompactView) -> Vec<Hit> {
        self.heap
            .into_sorted_vec()
            .into_iter()
            .map(|ranked| view.hit(ranked.slot as usize, ranked.class))
            .collect()
    }
}

impl CompactView {
    pub(super) fn stats(&self) -> &BuildStats {
        &self.stats
    }

    pub(super) fn source_summary(&self) -> SourceSummary {
        self.source_summary
    }

    pub(super) fn rows(&self) -> usize {
        self.name_span.len()
    }

    /// Estimated resident bytes. Capacity-based, not an RSS measurement; the
    /// benchmark reports real process RSS separately.
    fn estimated_bytes(&self) -> usize {
        self.names.capacity()
            + self.directories.capacity()
            + self.ids.capacity()
            + self.name_span.capacity() * std::mem::size_of::<Span>()
            + self.directory_span.capacity() * std::mem::size_of::<Span>()
            + self.id_span.capacity() * std::mem::size_of::<Span>()
            + self.slot_directory.capacity() * std::mem::size_of::<u32>()
            + self.modified.capacity() * std::mem::size_of::<i64>()
            + self.extension_index.capacity() * std::mem::size_of::<u16>()
            + self.extensions.iter().map(String::len).sum::<usize>()
    }

    /// Build from one explicit read snapshot. The caller must use this view
    /// only with the same `ReadSnapshot` identity.
    pub(super) fn build(
        snapshot: &ReadSnapshot<'_>,
        budget_bytes: usize,
    ) -> Result<Option<Self>, DbError> {
        let conn = snapshot.connection();
        let source_summary = read_source_summary(conn)?;
        let mut view = Self {
            snapshot_identity: Arc::clone(snapshot.identity()),
            view_identity: Arc::new(()),
            source_summary,
            names: String::new(),
            name_span: Vec::new(),
            directories: String::new(),
            directory_span: Vec::new(),
            slot_directory: Vec::new(),
            ids: String::new(),
            id_span: Vec::new(),
            modified: Vec::new(),
            extension_index: Vec::new(),
            extensions: Vec::new(),
            stats: BuildStats::default(),
        };
        let mut directory_ids = HashMap::<String, u32>::new();
        let mut extension_ids = HashMap::<String, u16>::new();

        let started = std::time::Instant::now();
        let mut statement = conn.prepare(
            "SELECT ge.id, ge.name_normalized, ge.path_normalized,
                    ge.extension, ge.modified_at_fs
             FROM global_entries ge
             JOIN global_volumes gv ON gv.id = ge.volume_id
             WHERE gv.enabled = 1 AND ge.is_stale = 0
             ORDER BY ge.rowid",
        )?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let path: String = row.get(2)?;
            let extension: String = row.get(3)?;
            let modified: Option<i64> = row.get(4)?;

            let directory = directory_prefix(&path);
            let directory_id = match directory_ids.get(directory) {
                Some(id) => *id,
                None => {
                    let id = view.directory_span.len() as u32;
                    let start = view.directories.len() as u32;
                    view.directories.push_str(directory);
                    view.directory_span.push(Span {
                        start,
                        len: directory.len() as u32,
                    });
                    directory_ids.insert(directory.to_string(), id);
                    id
                }
            };
            let extension_id = match extension_ids.get(&extension) {
                Some(id) => *id,
                None => {
                    // A 16-bit extension index keeps the slot array small. A
                    // source with more distinct extensions than that degrades
                    // instead of silently aliasing two extensions.
                    if view.extensions.len() >= u16::MAX as usize {
                        return Ok(None);
                    }
                    let id = view.extensions.len() as u16;
                    view.extensions.push(extension.clone());
                    extension_ids.insert(extension.clone(), id);
                    id
                }
            };

            let name_start = view.names.len() as u32;
            view.names.push_str(&name);
            view.name_span.push(Span {
                start: name_start,
                len: name.len() as u32,
            });
            let id_start = view.ids.len() as u32;
            view.ids.push_str(&id);
            view.id_span.push(Span {
                start: id_start,
                len: id.len() as u32,
            });
            view.slot_directory.push(directory_id);
            view.modified.push(modified.unwrap_or(i64::MIN));
            view.extension_index.push(extension_id);

            // Fail fast instead of exhausting memory on an oversized source.
            if view.name_span.len().is_multiple_of(65_536) && view.estimated_bytes() > budget_bytes
            {
                return Ok(None);
            }
        }
        view.stats.build_ms = started.elapsed().as_secs_f64() * 1_000.0;
        view.stats.rows = view.name_span.len();
        view.stats.distinct_directories = view.directory_span.len();
        view.stats.distinct_extensions = view.extensions.len();
        view.stats.name_bytes = view.names.len();
        view.stats.directory_bytes = view.directories.len();
        view.stats.id_bytes = view.ids.len();
        view.stats.slot_bytes = view.name_span.capacity() * std::mem::size_of::<Span>()
            + view.slot_directory.capacity() * std::mem::size_of::<u32>()
            + view.modified.capacity() * std::mem::size_of::<i64>()
            + view.extension_index.capacity() * std::mem::size_of::<u16>()
            + view.id_span.capacity() * std::mem::size_of::<Span>();
        view.stats.estimated_total_bytes = view.estimated_bytes();
        if view.stats.estimated_total_bytes > budget_bytes {
            return Ok(None);
        }
        Ok(Some(view))
    }

    fn name(&self, slot: usize) -> &str {
        let span = &self.name_span[slot];
        &self.names[span.start as usize..(span.start + span.len) as usize]
    }

    fn directory(&self, slot: usize) -> &str {
        let span = &self.directory_span[self.slot_directory[slot] as usize];
        &self.directories[span.start as usize..(span.start + span.len) as usize]
    }

    fn id_slice(&self, slot: usize) -> &str {
        let span = &self.id_span[slot];
        &self.ids[span.start as usize..(span.start + span.len) as usize]
    }

    fn modified_at(&self, slot: usize) -> Option<i64> {
        if self.modified[slot] == i64::MIN {
            None
        } else {
            Some(self.modified[slot])
        }
    }

    fn extension(&self, slot: usize) -> &str {
        &self.extensions[self.extension_index[slot] as usize]
    }

    fn slot_of_offset(&self, offset: usize) -> Option<usize> {
        let index = self
            .name_span
            .partition_point(|span| (span.start as usize) <= offset);
        index.checked_sub(1)
    }

    /// Every slot whose name contains `needle`, in ascending slot order.
    ///
    /// The scan advances one character past a match that straddles a name
    /// boundary so a real occurrence cannot be skipped by an overlapping false
    /// positive.
    fn name_matches(&self, needle: &str) -> Vec<u32> {
        let mut slots = Vec::new();
        if needle.is_empty() || needle.len() > self.names.len() {
            return slots;
        }
        let blob = self.names.as_str();
        let mut cursor = 0usize;
        while cursor + needle.len() <= blob.len() {
            let Some(relative) = blob[cursor..].find(needle) else {
                break;
            };
            let offset = cursor + relative;
            let mut in_slot = false;
            if let Some(slot) = self.slot_of_offset(offset) {
                let span = &self.name_span[slot];
                if offset + needle.len() <= (span.start + span.len) as usize {
                    if slots.last() != Some(&(slot as u32)) {
                        slots.push(slot as u32);
                    }
                    in_slot = true;
                }
            }
            cursor = if in_slot {
                offset + needle.len()
            } else {
                // Advance exactly one character past the match start.
                offset + blob[offset..].chars().next().map_or(1, char::len_utf8)
            };
        }
        slots
    }

    /// Interned directory ids whose prefix contains `needle`.
    fn directory_matches(&self, needle: &str) -> Vec<u32> {
        let mut ids = Vec::new();
        if needle.is_empty() || needle.len() > self.directories.len() {
            return ids;
        }
        let blob = self.directories.as_str();
        let mut cursor = 0usize;
        while cursor + needle.len() <= blob.len() {
            let Some(relative) = blob[cursor..].find(needle) else {
                break;
            };
            let offset = cursor + relative;
            let index = self
                .directory_span
                .partition_point(|span| (span.start as usize) <= offset);
            let mut in_span = false;
            if let Some(index) = index.checked_sub(1) {
                let span = &self.directory_span[index];
                if offset + needle.len() <= (span.start + span.len) as usize {
                    if ids.last() != Some(&(index as u32)) {
                        ids.push(index as u32);
                    }
                    in_span = true;
                }
            }
            cursor = if in_span {
                offset + needle.len()
            } else {
                offset + blob[offset..].chars().next().map_or(1, char::len_utf8)
            };
        }
        ids
    }

    fn classify(&self, slot: usize, folded: &str, sql_lower: &str) -> Option<Class> {
        proposed_class(
            self.name(slot),
            self.directory(slot),
            self.extension(slot),
            folded,
            sql_lower,
        )
    }

    fn hit(&self, slot: usize, class: Class) -> Hit {
        Hit {
            id: self.id_slice(slot).to_string(),
            class,
            modified: self.modified_at(slot),
        }
    }

    /// Filename/path channel from RAM only.
    ///
    /// `previous` enables incremental narrowing: when the new query extends the
    /// previous one, every match of the new query is necessarily a match of the
    /// old one, so only the cached slots need re-classification. The correctness
    /// tests assert this equals the full scan.
    fn name_path_channel(
        &self,
        folded: &str,
        sql_lower: &str,
        window: usize,
        previous: Option<&NarrowCache>,
    ) -> (Vec<Hit>, bool) {
        let mut collector = SlotTopK::new(window);
        let mut seen = vec![0u64; self.name_span.len().div_ceil(64) + 1];
        let mut narrowed = false;

        let cached = previous.filter(|cache| {
            Arc::ptr_eq(&cache.view_identity, &self.view_identity)
                && cache
                    .slots
                    .iter()
                    .all(|slot| (*slot as usize) < self.name_span.len())
                && !cache.query.is_empty()
                && folded.starts_with(cache.query.as_str())
                && cache.query.len() < folded.len()
                && cache.slots.len() <= NARROW_CACHE_CAPACITY
        });

        match cached {
            Some(cache) => {
                narrowed = true;
                for slot in &cache.slots {
                    let slot = *slot as usize;
                    if let Some(class) = self.classify(slot, folded, sql_lower) {
                        seen[slot / 64] |= 1 << (slot % 64);
                        collector.offer(self, slot as u32, class);
                    }
                }
            }
            None => {
                for slot in self.name_matches(folded) {
                    let slot = slot as usize;
                    if let Some(class) = self.classify(slot, folded, sql_lower) {
                        seen[slot / 64] |= 1 << (slot % 64);
                        collector.offer(self, slot as u32, class);
                    }
                }
            }
        }

        // Path-only hits: the interned directory matches but the name does not.
        //
        // One pass over the slot array with a binary search into the matched
        // directory list. The obvious "for each matched directory, scan every
        // slot" shape is O(directories x slots) and cost ~22 ms per query on the
        // 100k fixture; this shape is O(slots).
        let matched_directories = self.directory_matches(folded);
        if !matched_directories.is_empty() {
            for slot in 0..self.slot_directory.len() {
                if matched_directories
                    .binary_search(&self.slot_directory[slot])
                    .is_err()
                {
                    continue;
                }
                if seen[slot / 64] & (1 << (slot % 64)) != 0 {
                    continue;
                }
                if let Some(class) = self.classify(slot, folded, sql_lower) {
                    seen[slot / 64] |= 1 << (slot % 64);
                    collector.offer(self, slot as u32, class);
                }
            }
        }

        (collector.finish(self), narrowed)
    }

    fn extension_channel(&self, sql_lower: &str, window: usize) -> Vec<Hit> {
        let mut collector = SlotTopK::new(window);
        for slot in 0..self.extension_index.len() {
            if let Some(class) = extension_class(self.extension(slot), sql_lower) {
                collector.offer(self, slot as u32, class);
            }
        }
        collector.finish(self)
    }
}

/// Cache of the slot list a query matched. Bounded by [`NARROW_CACHE_CAPACITY`].
pub(super) struct NarrowCache {
    view_identity: Arc<()>,
    query: String,
    slots: Vec<u32>,
}

impl NarrowCache {
    pub(super) fn build(view: &CompactView, query: &str) -> Option<Self> {
        let folded = fold(query.trim());
        if folded.is_empty() {
            return None;
        }
        let slots = view.name_matches(&folded);
        if slots.len() > NARROW_CACHE_CAPACITY {
            return None;
        }
        Some(Self {
            view_identity: Arc::clone(&view.view_identity),
            query: folded,
            slots,
        })
    }

    pub(super) fn len(&self) -> usize {
        self.slots.len()
    }
}

/// RAM-only filename/path channel.
///
/// This is the number that must NEVER be reported as "B search latency": it has
/// no SQLite completion, no hydration and no revision read. It exists so the
/// cost of those steps can be attributed instead of hidden.
pub(super) fn filename_only(
    view: &CompactView,
    query: &str,
    window: usize,
    previous: Option<&NarrowCache>,
) -> (Vec<Hit>, bool) {
    let folded = fold(query.trim());
    let sql_lower = sqlite_lower(query.trim());
    view.name_path_channel(&folded, &sql_lower, window.min(MAX_WINDOW), previous)
}

/// Answer a query from the view, then verify every returned row against the
/// authoritative SQLite snapshot.
///
/// Hydration is the safety property: a stale view can never leak a deleted,
/// renamed, stale or de-authorised file, because the returned ids are re-read
/// and re-classified inside the caller's read transaction.
pub(super) fn lookup(
    view: &CompactView,
    snapshot: &ReadSnapshot<'_>,
    query: &str,
    window: usize,
    previous: Option<&NarrowCache>,
) -> Result<Lookup, DbError> {
    let trimmed = query.trim();
    let window = window.min(MAX_WINDOW);
    if !Arc::ptr_eq(&view.snapshot_identity, snapshot.identity()) {
        return Ok(Lookup {
            fence: Fence::StaleSnapshot,
            name_path: Vec::new(),
            extension: Vec::new(),
            hydrated_dropped: 0,
            narrowed: false,
        });
    }
    let conn = snapshot.connection();
    let folded = fold(trimmed);
    let sql_lower = sqlite_lower(trimmed);
    let (name_path, narrowed) = view.name_path_channel(&folded, &sql_lower, window, previous);
    let extension = view.extension_channel(&sql_lower, window);

    let (name_path, dropped_name, changed_name) =
        hydrate(conn, name_path, Channel::NamePath, &folded, &sql_lower)?;
    let (extension, dropped_extension, changed_extension) =
        hydrate(conn, extension, Channel::Extension, &folded, &sql_lower)?;
    let hydrated_dropped = dropped_name + dropped_extension;
    if hydrated_dropped > 0 || changed_name || changed_extension {
        return Ok(Lookup {
            fence: Fence::HydrationMismatch,
            name_path: Vec::new(),
            extension: Vec::new(),
            hydrated_dropped,
            narrowed: false,
        });
    }

    Ok(Lookup {
        fence: Fence::Valid,
        name_path,
        extension,
        hydrated_dropped,
        narrowed,
    })
}

/// Which channel a hydrated row belongs to. The extension channel must be
/// re-classified with [`extension_class`]; using the name/path classifier there
/// would relabel every `.pdf` hit as a name match.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Channel {
    NamePath,
    Extension,
}

/// Re-read every candidate id from SQLite inside the caller's read transaction.
/// Rows that disappeared, went stale, lost authorisation or no longer match the
/// query are dropped and counted.
fn hydrate(
    conn: &Connection,
    ranked: Vec<Hit>,
    channel: Channel,
    folded: &str,
    sql_lower: &str,
) -> Result<(Vec<Hit>, u64, bool), DbError> {
    if ranked.is_empty() {
        return Ok((Vec::new(), 0, false));
    }
    let ids: Vec<&str> = ranked.iter().map(|hit| hit.id.as_str()).collect();
    let mut placeholders = String::with_capacity(ids.len() * 2);
    for index in 0..ids.len() {
        if index > 0 {
            placeholders.push(',');
        }
        placeholders.push('?');
    }
    // Deliberately no `JOIN global_volumes` here. With the join, SQLite drove
    // the query from `global_volumes` and scanned the whole entry table for
    // every batch, which cost ~0.3 ms per hydrated id. The authorisation
    // predicate is applied in Rust against the enabled-volume set instead, so
    // the plan is a primary-key lookup per id.
    let sql = format!(
        "SELECT ge.id, ge.name_normalized, ge.path_normalized, ge.extension,
                ge.modified_at_fs, ge.volume_id
         FROM global_entries ge
         WHERE ge.is_stale = 0 AND ge.id IN ({placeholders})"
    );
    let mut authorized = std::collections::HashSet::<String>::new();
    {
        let mut statement =
            conn.prepare_cached("SELECT id FROM global_volumes WHERE enabled = 1")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            authorized.insert(row?);
        }
    }
    let mut statement = conn.prepare(&sql)?;
    let mut facts = HashMap::<String, (String, String, String, Option<i64>)>::new();
    let rows = statement.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            (
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, String>(5)?,
            ),
        ))
    })?;
    for row in rows {
        let (id, (name, path, extension, modified, volume_id)) = row?;
        if !authorized.contains(&volume_id) {
            continue;
        }
        facts.insert(id, (name, path, extension, modified));
    }

    let mut kept = Vec::with_capacity(ranked.len());
    let mut dropped = 0u64;
    let mut changed = false;
    for hit in ranked {
        let Some((name, path, extension, modified)) = facts.get(&hit.id) else {
            dropped += 1;
            continue;
        };
        let class = match channel {
            Channel::NamePath => proposed_class(name, path, extension, folded, sql_lower),
            Channel::Extension => extension_class(extension, sql_lower),
        };
        let Some(class) = class else {
            // The stored fact no longer matches the query: a rename or a move.
            dropped += 1;
            continue;
        };
        if class != hit.class || *modified != hit.modified {
            changed = true;
        }
        kept.push(Hit {
            id: hit.id,
            class,
            modified: *modified,
        });
    }
    kept.sort();
    Ok((kept, dropped, changed))
}
