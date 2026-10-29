use super::memory::{BudgetCharge, MemoryBudget, MAX_ACTIVE_VIEW_BYTES};
use crate::db::{schema_global_search_changes, DbError};
use rusqlite::Connection;
use std::{
    cmp::Ordering,
    collections::{hash_map::DefaultHasher, BinaryHeap, HashMap},
    hash::{Hash, Hasher},
    mem::size_of,
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) const MAX_INCREMENTAL_CHANGES: usize = 5_000;
pub(super) const MAX_INCREMENTAL_WORK: Duration = Duration::from_millis(50);
pub(super) const MAX_CANDIDATE_WINDOW: usize = 4_096;

const BUILD_RESERVE_ENTRIES: usize = 256;
const BUILD_RESERVE_BYTES: usize = 64 * 1024;
const STRING_ALLOCATION_OVERHEAD: usize = 16;
const INTERN_SLOT_ESTIMATE: usize = 96;

#[derive(Debug)]
pub(super) enum ViewError {
    Database(String),
    MemoryBudget,
    ViewBudget,
    UpdateBudget,
    TooManyChanges,
    RebuildRequired,
    InvalidFeed,
}

impl From<DbError> for ViewError {
    fn from(error: DbError) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<rusqlite::Error> for ViewError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<rusqlite::types::FromSqlError> for ViewError {
    fn from(error: rusqlite::types::FromSqlError) -> Self {
        Self::Database(error.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextRange {
    start: u32,
    len: u32,
}

impl TextRange {
    fn new(start: usize, len: usize) -> Result<Self, ViewError> {
        Ok(Self {
            start: u32::try_from(start).map_err(|_| ViewError::ViewBudget)?,
            len: u32::try_from(len).map_err(|_| ViewError::ViewBudget)?,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct BaseEntry {
    id: TextRange,
    source: u32,
    name: TextRange,
    directory: u32,
    path_tail: TextRange,
    extension: TextRange,
    modified_at: i64,
    has_modified_at: bool,
}

#[derive(Debug, Clone, Copy)]
struct Directory {
    path: TextRange,
}

struct BaseView {
    entries: Vec<BaseEntry>,
    directories: Vec<Directory>,
    source_ids: Vec<TextRange>,
    strings: Vec<u8>,
    _charge: Arc<BudgetCharge>,
}

#[derive(Debug)]
struct DeltaDocument {
    id: Arc<str>,
    source_id: Arc<str>,
    name: Arc<str>,
    path: Arc<str>,
    extension: Arc<str>,
    modified_at: Option<i64>,
}

struct DeltaView {
    entries: HashMap<Arc<str>, Option<Arc<DeltaDocument>>>,
    _map_charge: Arc<BudgetCharge>,
    document_charges: Vec<Arc<BudgetCharge>>,
    applied_events: usize,
}

pub(super) struct SearchView {
    pub(super) generation: i64,
    base: Arc<BaseView>,
    delta: Arc<DeltaView>,
}

pub(super) struct CandidateWindow<'a> {
    pub(super) ids: Vec<&'a str>,
    pub(super) matching_count: usize,
    _charge: Arc<BudgetCharge>,
}

#[derive(Clone, Copy)]
struct HeapCandidate<'a> {
    id: &'a str,
    modified_at: Option<i64>,
}

impl PartialEq for HeapCandidate<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.modified_at == other.modified_at && self.id == other.id
    }
}

impl Eq for HeapCandidate<'_> {}

impl PartialOrd for HeapCandidate<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// BinaryHeap's maximum is the least desirable candidate so a bounded heap can
// discard its root. The final window uses modified_at DESC, id ASC, matching
// the indexed SQLite order shape while leaving BM25 as the user-visible rank.
impl Ord for HeapCandidate<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .modified_at
            .cmp(&self.modified_at)
            .then_with(|| self.id.cmp(other.id))
    }
}

impl SearchView {
    pub(super) fn build(
        conn: &Connection,
        generation: i64,
        budget: Arc<MemoryBudget>,
    ) -> Result<Self, ViewError> {
        let base = Arc::new(BaseView::build(conn, budget.clone())?);
        let delta_charge = BudgetCharge::empty(budget);
        Ok(Self {
            generation,
            base,
            delta: Arc::new(DeltaView {
                entries: HashMap::new(),
                _map_charge: delta_charge,
                document_charges: Vec::new(),
                applied_events: 0,
            }),
        })
    }

    #[cfg(test)]
    pub(super) fn active_bytes(&self) -> usize {
        self.base._charge.bytes()
            + self.delta._map_charge.bytes()
            + self
                .delta
                .document_charges
                .iter()
                .map(|charge| charge.bytes())
                .sum::<usize>()
    }

    pub(super) fn candidate_window<'a>(
        &'a self,
        query: &str,
        budget: Arc<MemoryBudget>,
    ) -> Result<CandidateWindow<'a>, ViewError> {
        let query_charge = BudgetCharge::reserved(
            budget,
            MAX_CANDIDATE_WINDOW * (size_of::<HeapCandidate<'_>>() + size_of::<&str>() * 2) + 4096,
        )
        .ok_or(ViewError::MemoryBudget)?;
        let query = query.to_ascii_lowercase().replace('\\', "/");
        let mut candidates = BinaryHeap::with_capacity(MAX_CANDIDATE_WINDOW);
        let mut matching_count = 0usize;

        for entry in &self.base.entries {
            let id = self.base.text(entry.id);
            if self.delta.entries.contains_key(id) {
                continue;
            }
            if self.base.matches(entry, &query) {
                matching_count = matching_count.saturating_add(1);
                push_top_k(
                    &mut candidates,
                    HeapCandidate {
                        id,
                        modified_at: entry.has_modified_at.then_some(entry.modified_at),
                    },
                );
            }
        }

        for document in self.delta.entries.values().flatten() {
            let _source_identity = document.source_id.as_ref();
            if document.name.contains(&query)
                || document.path.contains(&query)
                || document.extension.contains(&query)
            {
                matching_count = matching_count.saturating_add(1);
                push_top_k(
                    &mut candidates,
                    HeapCandidate {
                        id: &document.id,
                        modified_at: document.modified_at,
                    },
                );
            }
        }

        let mut ordered = candidates.into_vec();
        ordered.sort_by(|left, right| {
            right
                .modified_at
                .cmp(&left.modified_at)
                .then_with(|| left.id.cmp(right.id))
        });
        let ids = ordered.into_iter().map(|candidate| candidate.id).collect();
        Ok(CandidateWindow {
            ids,
            matching_count,
            _charge: query_charge,
        })
    }

    pub(super) fn apply_changes(
        &self,
        conn: &Connection,
        events: &[schema_global_search_changes::ChangeEvent],
        budget: Arc<MemoryBudget>,
        started_at: Instant,
    ) -> Result<Self, ViewError> {
        if events.is_empty() || events.len() > MAX_INCREMENTAL_CHANGES {
            return Err(ViewError::TooManyChanges);
        }
        if events[0].generation != self.generation.saturating_add(1)
            || events
                .windows(2)
                .any(|pair| pair[1].generation != pair[0].generation.saturating_add(1))
        {
            return Err(ViewError::InvalidFeed);
        }
        if events
            .iter()
            .any(|event| event.kind == schema_global_search_changes::ChangeKind::Invalidate)
        {
            return Err(ViewError::RebuildRequired);
        }
        if self.delta.applied_events.saturating_add(events.len()) > MAX_INCREMENTAL_CHANGES {
            return Err(ViewError::RebuildRequired);
        }

        let operation_charge = BudgetCharge::reserved(
            budget.clone(),
            events.len().saturating_mul(
                size_of::<(Arc<str>, Option<Arc<DeltaDocument>>)>()
                    .saturating_mul(3)
                    .saturating_add(32),
            ),
        )
        .ok_or(ViewError::MemoryBudget)?;
        let document_charge = BudgetCharge::empty(budget.clone());
        let mut latest_by_id =
            HashMap::<&str, (&schema_global_search_changes::ChangeEvent, i64)>::new();
        latest_by_id
            .try_reserve(events.len())
            .map_err(|error| ViewError::Database(format!("search_view_batch_merge:{error}")))?;
        for event in events {
            latest_by_id
                .entry(event.entity_id.as_str())
                .and_modify(|(latest, _)| *latest = event)
                .or_insert((event, event.generation));
        }
        let mut latest_events = Vec::new();
        latest_events
            .try_reserve_exact(latest_by_id.len())
            .map_err(|error| ViewError::Database(format!("search_view_batch_sort:{error}")))?;
        latest_events.extend(
            latest_by_id
                .values()
                .map(|(event, first_generation)| (*event, *first_generation)),
        );
        // Process IDs by their first event in the batch. A time-limited batch
        // can then advance to the earliest event belonging to any unprocessed
        // ID without skipping a fact change.
        latest_events.sort_unstable_by_key(|(_, first_generation)| *first_generation);
        let mut operations = Vec::new();
        operations
            .try_reserve_exact(latest_events.len())
            .map_err(|error| ViewError::Database(format!("search_view_batch_reserve:{error}")))?;
        let mut delta_entry = conn.prepare(
            "SELECT entry.id, entry.volume_id, entry.name, entry.path_normalized, entry.extension, entry.modified_at_fs FROM global_entries entry JOIN global_volumes volume ON volume.id = entry.volume_id WHERE entry.id = ?1 AND entry.is_stale = 0 AND volume.enabled = 1",
        )?;

        let mut processed_unique = 0usize;
        for (event, _) in &latest_events {
            if started_at.elapsed() >= MAX_INCREMENTAL_WORK && processed_unique > 0 {
                break;
            }
            if started_at.elapsed() >= MAX_INCREMENTAL_WORK {
                return Err(ViewError::UpdateBudget);
            }
            let mut rows = delta_entry.query([&event.entity_id])?;
            let current = if let Some(row) = rows.next()? {
                let id = row.get_ref(0)?.as_str()?;
                let source_id = row.get_ref(1)?.as_str()?;
                let name = row.get_ref(2)?.as_str()?;
                let path = row.get_ref(3)?.as_str()?;
                let extension = row.get_ref(4)?.as_str()?;
                let modified_at = row.get::<_, Option<i64>>(5)?;
                let document_bytes = id
                    .len()
                    .saturating_add(source_id.len())
                    .saturating_add(name.len())
                    .saturating_add(path.len())
                    .saturating_add(extension.len())
                    .saturating_add(5 * STRING_ALLOCATION_OVERHEAD)
                    .saturating_add(size_of::<DeltaDocument>() + 16);
                if !document_charge.grow(document_bytes) {
                    return Err(ViewError::MemoryBudget);
                }
                Some(Arc::new(DeltaDocument {
                    id: Arc::from(id),
                    source_id: Arc::from(source_id),
                    name: Arc::from(name.to_ascii_lowercase()),
                    path: Arc::from(path.to_ascii_lowercase()),
                    extension: Arc::from(extension.to_ascii_lowercase()),
                    modified_at,
                }))
            } else {
                None
            };
            drop(rows);
            let key = if let Some(document) = &current {
                document.id.clone()
            } else {
                let tombstone_bytes = event.entity_id.len() + STRING_ALLOCATION_OVERHEAD;
                if !document_charge.grow(tombstone_bytes) {
                    return Err(ViewError::MemoryBudget);
                }
                Arc::from(event.entity_id.as_str())
            };
            operations.push((key, current));
            processed_unique += 1;
        }
        drop(delta_entry);

        let safe_generation = if processed_unique == latest_events.len() {
            events.last().map(|event| event.generation)
        } else {
            latest_events[processed_unique..]
                .iter()
                .map(|(_, first_generation)| *first_generation)
                .min()
                .map(|first_unprocessed| first_unprocessed.saturating_sub(1))
        }
        .ok_or(ViewError::InvalidFeed)?;
        if safe_generation <= self.generation {
            return Err(ViewError::UpdateBudget);
        }

        let next_entries = self.delta.entries.len().saturating_add(operations.len());
        let map_capacity = self.delta.entries.capacity().max(next_entries);
        // Account for cloned hash buckets and retained document-charge handles
        // before allocating the next immutable overlay.
        let map_bytes = map_capacity
            .saturating_mul(size_of::<(Arc<str>, Option<Arc<DeltaDocument>>)>())
            .saturating_mul(3)
            .saturating_add(map_capacity.saturating_mul(32))
            .saturating_add(
                (self.delta.document_charges.len() + 1) * size_of::<Arc<BudgetCharge>>(),
            );
        let map_charge =
            BudgetCharge::reserved(budget, map_bytes).ok_or(ViewError::MemoryBudget)?;
        let retained_document_bytes = self
            .delta
            .document_charges
            .iter()
            .map(|charge| charge.bytes())
            .sum::<usize>()
            .saturating_add(document_charge.bytes());
        if self
            .base
            ._charge
            .bytes()
            .saturating_add(map_charge.bytes())
            .saturating_add(retained_document_bytes)
            > MAX_ACTIVE_VIEW_BYTES
        {
            return Err(ViewError::ViewBudget);
        }

        let mut entries = self.delta.entries.clone();
        entries.reserve(operations.len());
        for (key, current) in operations {
            entries.insert(key, current);
        }
        drop(operation_charge);
        let mut document_charges = self.delta.document_charges.clone();
        document_charges.push(document_charge);
        let applied_events = usize::try_from(safe_generation - self.generation)
            .map_err(|_| ViewError::InvalidFeed)?;
        Ok(Self {
            generation: safe_generation,
            base: self.base.clone(),
            delta: Arc::new(DeltaView {
                entries,
                _map_charge: map_charge,
                document_charges,
                applied_events: self.delta.applied_events + applied_events,
            }),
        })
    }
}

impl BaseView {
    fn build(conn: &Connection, budget: Arc<MemoryBudget>) -> Result<Self, ViewError> {
        let charge = BudgetCharge::empty(budget.clone());
        let mut entries: Vec<BaseEntry> = Vec::new();
        let mut directories: Vec<Directory> = Vec::new();
        let mut source_ids: Vec<TextRange> = Vec::new();
        let mut strings = Vec::new();
        let mut directory_hashes: HashMap<u64, Vec<u32>> = HashMap::new();
        let mut source_map: HashMap<String, u32> = HashMap::new();
        let temporary_intern_charge = BudgetCharge::empty(budget.clone());
        let mut statement = conn.prepare(
            "SELECT entry.id, entry.volume_id, entry.name_normalized, entry.path_normalized, entry.extension, entry.modified_at_fs FROM global_entries entry JOIN global_volumes volume ON volume.id = entry.volume_id WHERE entry.is_stale = 0 AND volume.enabled = 1 ORDER BY entry.id ASC",
        )?;
        let mut rows = statement.query([])?;

        while let Some(row) = rows.next()? {
            if entries.len() == entries.capacity() {
                let reserve = BUILD_RESERVE_ENTRIES.max(entries.capacity());
                let before = entries.capacity();
                reserve_vec(&mut entries, reserve, &charge, size_of::<BaseEntry>())?;
                let grown = entries.capacity().saturating_sub(before);
                if charge.bytes() > MAX_ACTIVE_VIEW_BYTES
                    || grown.saturating_mul(size_of::<BaseEntry>()) > MAX_ACTIVE_VIEW_BYTES
                {
                    return Err(ViewError::ViewBudget);
                }
            }
            if directories.len() == directories.capacity() {
                reserve_vec(
                    &mut directories,
                    BUILD_RESERVE_ENTRIES,
                    &charge,
                    size_of::<Directory>(),
                )?;
            }
            if source_ids.len() == source_ids.capacity() {
                reserve_vec(&mut source_ids, 8, &charge, size_of::<TextRange>())?;
            }

            let id = row.get_ref(0)?.as_str()?;
            let source_text = row.get_ref(1)?.as_str()?;
            let name = row.get_ref(2)?.as_str()?;
            let path = row.get_ref(3)?.as_str()?;
            let extension = row.get_ref(4)?.as_str()?;
            let modified_at = row.get::<_, Option<i64>>(5)?;
            let normalized_text_bytes = name
                .len()
                .saturating_add(path.len())
                .saturating_add(extension.len())
                .saturating_add(3 * STRING_ALLOCATION_OVERHEAD);
            let _normalization_charge =
                BudgetCharge::reserved(budget.clone(), normalized_text_bytes)
                    .ok_or(ViewError::MemoryBudget)?;
            let name = name.to_ascii_lowercase();
            let path = path.to_ascii_lowercase();
            let extension = extension.to_ascii_lowercase();
            let parent_end = path.rfind(['/', '\\']).map_or(0, |position| position + 1);
            let directory_path = &path[..parent_end];

            let source = if let Some(source) = source_map.get(source_text) {
                *source
            } else {
                let source = u32::try_from(source_ids.len()).map_err(|_| ViewError::ViewBudget)?;
                let source_range = append_text(&mut strings, source_text, &charge)?;
                temporary_intern_charge
                    .grow(INTERN_SLOT_ESTIMATE.saturating_add(source_text.len()))
                    .then_some(())
                    .ok_or(ViewError::MemoryBudget)?;
                source_map.try_reserve(1).map_err(|error| {
                    ViewError::Database(format!("search_view_source_map:{error}"))
                })?;
                source_map.insert(source_text.to_owned(), source);
                source_ids.push(source_range);
                source
            };

            let mut directory_hasher = DefaultHasher::new();
            directory_path.hash(&mut directory_hasher);
            let directory_hash = directory_hasher.finish();
            let directory = if let Some(candidates) = directory_hashes.get(&directory_hash) {
                candidates.iter().copied().find(|candidate| {
                    directories
                        .get(*candidate as usize)
                        .is_some_and(|directory| {
                            text_in(&strings, directory.path) == directory_path
                        })
                })
            } else {
                None
            };
            let directory = if let Some(directory) = directory {
                directory
            } else {
                let directory =
                    u32::try_from(directories.len()).map_err(|_| ViewError::ViewBudget)?;
                let range = append_text(&mut strings, directory_path, &charge)?;
                temporary_intern_charge
                    .grow(INTERN_SLOT_ESTIMATE)
                    .then_some(())
                    .ok_or(ViewError::MemoryBudget)?;
                directory_hashes.try_reserve(1).map_err(|error| {
                    ViewError::Database(format!("search_view_directory_map:{error}"))
                })?;
                let candidates = directory_hashes.entry(directory_hash).or_default();
                candidates.try_reserve(1).map_err(|error| {
                    ViewError::Database(format!("search_view_directory_bucket:{error}"))
                })?;
                candidates.push(directory);
                directories.push(Directory { path: range });
                directory
            };

            let name_range = append_text(&mut strings, &name, &charge)?;
            let path_tail = &path[parent_end..];
            let path_tail_range = if path_tail == name {
                name_range
            } else {
                append_text(&mut strings, path_tail, &charge)?
            };
            let entry = BaseEntry {
                id: append_text(&mut strings, id, &charge)?,
                source,
                name: name_range,
                directory,
                path_tail: path_tail_range,
                extension: append_text(&mut strings, &extension, &charge)?,
                modified_at: modified_at.unwrap_or(i64::MIN),
                has_modified_at: modified_at.is_some(),
            };
            entries.push(entry);
            if charge.bytes() > MAX_ACTIVE_VIEW_BYTES {
                return Err(ViewError::ViewBudget);
            }
        }
        drop(directory_hashes);
        drop(source_map);
        drop(temporary_intern_charge);
        if charge.bytes() > MAX_ACTIVE_VIEW_BYTES {
            return Err(ViewError::ViewBudget);
        }
        Ok(Self {
            entries,
            directories,
            source_ids,
            strings,
            _charge: charge,
        })
    }

    fn matches(&self, entry: &BaseEntry, query: &str) -> bool {
        let name = self.text(entry.name);
        let directory = self
            .directories
            .get(entry.directory as usize)
            .map(|record| self.text(record.path))
            .unwrap_or("");
        let extension = self.text(entry.extension);
        let path_tail = self.text(entry.path_tail);
        let _source_identity = self.source_ids.get(entry.source as usize);
        name.contains(query)
            || directory.contains(query)
            || contains_joined(directory, path_tail, query)
            || extension.contains(query)
    }

    fn text(&self, range: TextRange) -> &str {
        text_in(&self.strings, range)
    }
}

fn reserve_vec<T>(
    values: &mut Vec<T>,
    additional: usize,
    charge: &BudgetCharge,
    item_bytes: usize,
) -> Result<(), ViewError> {
    let wanted = values.len().saturating_add(additional);
    if wanted <= values.capacity() {
        return Ok(());
    }
    let expected_capacity = wanted;
    let expected_bytes = expected_capacity
        .saturating_sub(values.capacity())
        .saturating_mul(item_bytes);
    if !charge.grow(expected_bytes) {
        return Err(ViewError::MemoryBudget);
    }
    let old_capacity = values.capacity();
    if let Err(error) = values.try_reserve_exact(additional) {
        charge.shrink(expected_bytes);
        return Err(ViewError::Database(format!(
            "search_view_allocation:{error}"
        )));
    }
    let actual_extra = values.capacity().saturating_sub(old_capacity);
    let charged_extra = expected_bytes / item_bytes.max(1);
    if actual_extra > charged_extra {
        let difference = actual_extra
            .saturating_sub(charged_extra)
            .saturating_mul(item_bytes);
        if !charge.grow(difference) {
            return Err(ViewError::MemoryBudget);
        }
    }
    Ok(())
}

fn append_text(
    pool: &mut Vec<u8>,
    value: &str,
    charge: &BudgetCharge,
) -> Result<TextRange, ViewError> {
    let start = pool.len();
    let end = start.saturating_add(value.len());
    if end > u32::MAX as usize {
        return Err(ViewError::ViewBudget);
    }
    if end > pool.capacity() {
        let additional = end.saturating_sub(pool.len()).max(BUILD_RESERVE_BYTES);
        if !charge.grow(additional) {
            return Err(ViewError::MemoryBudget);
        }
        let old_capacity = pool.capacity();
        if let Err(error) = pool.try_reserve_exact(additional) {
            charge.shrink(additional);
            return Err(ViewError::Database(format!(
                "search_view_pool_allocation:{error}"
            )));
        }
        let actual_extra = pool.capacity().saturating_sub(old_capacity);
        if actual_extra > additional && !charge.grow(actual_extra - additional) {
            return Err(ViewError::MemoryBudget);
        }
    }
    pool.extend_from_slice(value.as_bytes());
    TextRange::new(start, value.len())
}

fn text_in(pool: &[u8], range: TextRange) -> &str {
    let start = range.start as usize;
    let end = start.saturating_add(range.len as usize);
    std::str::from_utf8(pool.get(start..end).unwrap_or_default()).unwrap_or("")
}

fn contains_joined(left: &str, right: &str, needle: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let needle = needle.as_bytes();
    let max_left = left.len().min(needle.len().saturating_sub(1));
    (1..=max_left).any(|left_len| {
        let right_len = needle.len() - left_len;
        left[left.len() - left_len..] == needle[..left_len]
            && right.len() >= right_len
            && right[..right_len] == needle[left_len..]
    })
}

fn push_top_k<'a>(heap: &mut BinaryHeap<HeapCandidate<'a>>, candidate: HeapCandidate<'a>) {
    if heap.len() < MAX_CANDIDATE_WINDOW {
        heap.push(candidate);
    } else if heap.peek().is_some_and(|worst| candidate < *worst) {
        heap.pop();
        heap.push(candidate);
    }
}
