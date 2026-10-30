//! Optional, disposable filename index used by the Global Search adapter.
//!
//! The SQLite tables remain authoritative. The versioned mmap contains only
//! compact search keys; row hydration and source admission are performed in
//! the caller's SQLite read transaction.

use super::models::GlobalSearchResult;
use crate::db::{Database, DbError};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use uuid::Uuid;

mod build;
mod format;
mod query;
mod search;

use format::{write_index_file, CompactIndex, CompactIndexData};
use query::{
    batch_schema_ready, change_tracking_ready, ensure_change_schema, is_safe_file_name,
    supported_query, OverlayRow,
};

const FORMAT_VERSION: u32 = 1;
const SCHEMA_VERSION: i64 = 3;
const MAGIC: &[u8; 8] = b"ZCNAMES1";
const NAME_BITMAP_WIDTH: usize = 36;
const SECTION_COUNT: usize = 25;
const HEADER_SIZE: usize = 8 + 4 + 8 + 6 * 4 + SECTION_COUNT * 8 + 32;
const NO_PARENT: u32 = u32::MAX;
const NULL_MTIME: i64 = i64::MIN;
const MAX_OVERLAY_EVENTS: u64 = 65_536;
const MAX_OVERLAY_IDS: usize = 32_768;
const MAX_INDEX_BYTES: u64 = 4 * 1024 * 1024 * 1024 - 1;

const CHANGE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS compact_name_search_changes (
    seq INTEGER PRIMARY KEY,
    entry_rowid INTEGER NOT NULL,
    entry_id TEXT,
    batch_ids TEXT,
    event_count INTEGER NOT NULL DEFAULT 1 CHECK(event_count > 0)
);
CREATE TABLE IF NOT EXISTS compact_name_search_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL,
    format_version INTEGER NOT NULL,
    base_generation INTEGER NOT NULL,
    file_name TEXT,
    change_floor INTEGER NOT NULL DEFAULT 0,
    batch_active INTEGER NOT NULL DEFAULT 0 CHECK(batch_active IN (0, 1))
);
"#;

const CHANGE_TRIGGERS: &str = r#"
CREATE TRIGGER IF NOT EXISTS compact_name_search_entries_ai
AFTER INSERT ON global_entries
WHEN NEW.id IS NOT NULL
 AND COALESCE((SELECT batch_active FROM compact_name_search_state WHERE singleton=1), 0)=0
BEGIN
    INSERT INTO compact_name_search_changes(entry_rowid) VALUES (NEW.rowid);
END;
CREATE TRIGGER IF NOT EXISTS compact_name_search_entries_au
AFTER UPDATE ON global_entries
WHEN COALESCE((SELECT batch_active FROM compact_name_search_state WHERE singleton=1), 0)=0
 AND (OLD.rowid IS NOT NEW.rowid
  OR OLD.id IS NOT NEW.id
  OR OLD.volume_id IS NOT NEW.volume_id
  OR OLD.name_normalized IS NOT NEW.name_normalized
  OR OLD.extension IS NOT NEW.extension
  OR OLD.path_normalized IS NOT NEW.path_normalized
  OR OLD.modified_at_fs IS NOT NEW.modified_at_fs
  OR OLD.is_directory IS NOT NEW.is_directory
  OR OLD.is_hidden IS NOT NEW.is_hidden
  OR OLD.is_system IS NOT NEW.is_system
  OR OLD.is_stale IS NOT NEW.is_stale
  OR OLD.parent_platform_file_id IS NOT NEW.parent_platform_file_id
  OR OLD.scope_decision IS NOT NEW.scope_decision)
BEGIN
    INSERT INTO compact_name_search_changes(entry_rowid, entry_id)
    SELECT OLD.rowid, OLD.id
    WHERE OLD.id IS NOT NULL AND (OLD.rowid IS NOT NEW.rowid OR OLD.id IS NOT NEW.id);
    INSERT INTO compact_name_search_changes(entry_rowid)
    VALUES (NEW.rowid);
END;
CREATE TRIGGER IF NOT EXISTS compact_name_search_entries_ad
AFTER DELETE ON global_entries
WHEN OLD.id IS NOT NULL
 AND COALESCE((SELECT batch_active FROM compact_name_search_state WHERE singleton=1), 0)=0
BEGIN
    INSERT INTO compact_name_search_changes(entry_rowid, entry_id)
    VALUES (OLD.rowid, OLD.id);
END;
CREATE TRIGGER IF NOT EXISTS compact_name_search_changes_cap
AFTER INSERT ON compact_name_search_changes
WHEN NEW.seq > (SELECT change_floor FROM compact_name_search_state WHERE singleton=1) + 65536
BEGIN
    UPDATE compact_name_search_state
    SET base_generation=-1, file_name=NULL, change_floor=NEW.seq-NEW.event_count
    WHERE singleton=1;
    DELETE FROM compact_name_search_changes WHERE seq < NEW.seq;
END;
"#;

#[derive(Default)]
#[allow(dead_code)]
pub struct CompactNameSearchDiagnostics {
    pub enabled: bool,
    pub queries: u64,
    pub accepted_queries: u64,
    pub fallbacks: u64,
    pub rebuilds: u64,
    pub last_overlay_events: u64,
    pub last_name_candidates: u64,
    pub last_path_candidates: u64,
    pub path_relation_complete: bool,
    pub manifest_overlay_micros: u64,
    pub name_prefilter_micros: u64,
    pub name_scan_heap_micros: u64,
    pub path_scan_heap_micros: u64,
    pub top_k_sort_page_micros: u64,
    pub page_hydration_micros: u64,
    pub last_error: Option<String>,
}

#[derive(Default)]
struct Counters {
    queries: AtomicU64,
    accepted_queries: AtomicU64,
    fallbacks: AtomicU64,
    rebuilds: AtomicU64,
    overlay_events: AtomicU64,
    name_candidates: AtomicU64,
    path_candidates: AtomicU64,
    manifest_overlay_micros: AtomicU64,
    name_prefilter_micros: AtomicU64,
    name_scan_heap_micros: AtomicU64,
    path_scan_heap_micros: AtomicU64,
    top_k_sort_page_micros: AtomicU64,
    page_hydration_micros: AtomicU64,
}

#[derive(Clone, Copy, Default)]
struct SearchTimings {
    manifest_overlay_micros: u64,
    name_prefilter_micros: u64,
    name_scan_heap_micros: u64,
    path_scan_heap_micros: u64,
    top_k_sort_page_micros: u64,
    page_hydration_micros: u64,
}

struct SearchPageOutcome {
    page: Vec<GlobalSearchResult>,
    overlay_events: u64,
    name_candidates: u64,
    path_candidates: u64,
    timings: SearchTimings,
}

fn elapsed_micros(started: Instant) -> u64 {
    started.elapsed().as_micros().min(u64::MAX as u128) as u64
}

pub(crate) struct CompactNameSearchManager {
    sidecar_dir: PathBuf,
    pool: Pool<SqliteConnectionManager>,
    owner_lease: Mutex<Option<File>>,
    feed_batching: Arc<AtomicBool>,
    enabled: AtomicBool,
    loaded: RwLock<Option<LoadedIndex>>,
    overlay_cache: RwLock<Option<CachedOverlay>>,
    rebuild_in_progress: AtomicBool,
    last_error: Mutex<Option<String>>,
    counters: Counters,
}

struct LoadedIndex {
    file_name: String,
    index: Arc<CompactIndex>,
}

struct CachedOverlay {
    file_name: String,
    base_generation: i64,
    latest_generation: i64,
    changed_ids: Arc<HashSet<String>>,
    rows: Arc<Vec<OverlayRow>>,
}

impl CompactNameSearchManager {
    pub(crate) fn new(
        database_path: PathBuf,
        pool: Pool<SqliteConnectionManager>,
        enabled: bool,
        feed_batching: Arc<AtomicBool>,
    ) -> Self {
        let mut sidecar_dir = database_path.as_os_str().to_os_string();
        sidecar_dir.push(".compact-name-search");
        Self {
            sidecar_dir: PathBuf::from(sidecar_dir),
            pool,
            owner_lease: Mutex::new(None),
            feed_batching,
            enabled: AtomicBool::new(enabled),
            loaded: RwLock::new(None),
            overlay_cache: RwLock::new(None),
            rebuild_in_progress: AtomicBool::new(false),
            last_error: Mutex::new(None),
            counters: Counters::default(),
        }
    }

    pub(crate) fn opted_in() -> bool {
        std::env::var("ZC_COMPACT_NAME_SEARCH").is_ok_and(|value| value == "1")
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled.load(AtomicOrdering::Acquire)
    }

    pub(crate) fn activate(self: &Arc<Self>) {
        if !self.is_enabled() {
            match self.try_owner_lease(false) {
                Ok(Some(lease)) => {
                    if let Err(error) = self.disable_change_tracking() {
                        self.set_error(format!("compact_name_search_disable:{error}"));
                    } else {
                        self.feed_batching.store(false, AtomicOrdering::Release);
                    }
                    drop(lease);
                }
                Ok(None) => {
                    if self.feed_is_batchable().unwrap_or(false) {
                        self.feed_batching.store(true, AtomicOrdering::Release);
                    }
                }
                Err(error) => {
                    self.set_error(format!("compact_name_search_owner_lease:{error}"));
                }
            }
            return;
        }
        let lease = match self.try_owner_lease(true) {
            Ok(Some(lease)) => lease,
            Ok(None) => {
                self.enabled.store(false, AtomicOrdering::Release);
                if self.feed_is_batchable().unwrap_or(false) {
                    self.feed_batching.store(true, AtomicOrdering::Release);
                }
                self.set_error("compact_name_search_owner_already_active".into());
                self.counters
                    .fallbacks
                    .fetch_add(1, AtomicOrdering::Relaxed);
                return;
            }
            Err(error) => {
                self.enabled.store(false, AtomicOrdering::Release);
                self.set_error(format!("compact_name_search_owner_lease:{error}"));
                self.counters
                    .fallbacks
                    .fetch_add(1, AtomicOrdering::Relaxed);
                return;
            }
        };
        if let Err(error) = self.install_change_tracking() {
            self.enabled.store(false, AtomicOrdering::Release);
            self.set_error(format!("compact_name_search_init:{error}"));
            self.counters
                .fallbacks
                .fetch_add(1, AtomicOrdering::Relaxed);
            drop(lease);
            return;
        }
        *self
            .owner_lease
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(lease);
        self.feed_batching.store(true, AtomicOrdering::Release);
        if let Err(error) = self.load_manifest_index() {
            self.set_error(format!("compact_name_search_init:{error}"));
        }
        self.request_rebuild();
    }

    fn try_owner_lease(&self, create: bool) -> io::Result<Option<File>> {
        if create {
            fs::create_dir_all(&self.sidecar_dir)?;
        } else if !self.sidecar_dir.exists() {
            return Ok(None);
        }
        let path = self.sidecar_dir.join("owner.lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        if create || !path.exists() {
            options.create(true);
        }
        let file = match options.truncate(false).open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error),
        }
    }

    fn feed_is_batchable(&self) -> io::Result<bool> {
        let conn = self.pool.get().map_err(io::Error::other)?;
        Ok(change_tracking_ready(&conn)? && batch_schema_ready(&conn)?)
    }

    fn disable_change_tracking(&self) -> io::Result<()> {
        let conn = self.pool.get().map_err(io::Error::other)?;
        let state_exists: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master
                    WHERE type='table' AND name='compact_name_search_state'
                 )",
                [],
                |row| row.get(0),
            )
            .map_err(io::Error::other)?;
        if !state_exists {
            return Ok(());
        }
        let has_change_floor: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM pragma_table_info('compact_name_search_state')
                    WHERE name='change_floor'
                 )",
                [],
                |row| row.get(0),
            )
            .map_err(io::Error::other)?;
        let tx = conn.unchecked_transaction().map_err(io::Error::other)?;
        let invalidate = if has_change_floor {
            "UPDATE compact_name_search_state SET base_generation=-1, file_name=NULL, change_floor=0 WHERE singleton=1"
        } else {
            "UPDATE compact_name_search_state SET base_generation=-1, file_name=NULL WHERE singleton=1"
        };
        tx.execute(invalidate, []).map_err(io::Error::other)?;
        if tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('compact_name_search_state') WHERE name='batch_active')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false)
        {
            tx.execute(
                "UPDATE compact_name_search_state SET batch_active=0 WHERE singleton=1",
                [],
            )
            .map_err(io::Error::other)?;
        }
        tx.execute_batch(
            "DROP TRIGGER IF EXISTS compact_name_search_entries_ai;
             DROP TRIGGER IF EXISTS compact_name_search_entries_au;
             DROP TRIGGER IF EXISTS compact_name_search_entries_ad;
             DROP TRIGGER IF EXISTS compact_name_search_changes_cap;",
        )
        .map_err(io::Error::other)?;
        tx.execute("DELETE FROM compact_name_search_changes", [])
            .map_err(io::Error::other)?;
        tx.commit().map_err(io::Error::other)
    }

    fn install_change_tracking(&self) -> io::Result<()> {
        let conn = self.pool.get().map_err(io::Error::other)?;
        let tx = conn.unchecked_transaction().map_err(io::Error::other)?;
        tx.execute_batch(CHANGE_SCHEMA).map_err(io::Error::other)?;
        ensure_change_schema(&tx)?;
        if !change_tracking_ready(&tx)? || !batch_schema_ready(&tx)? {
            tx.execute(
                "UPDATE compact_name_search_state
                 SET base_generation=-1, file_name=NULL WHERE singleton=1",
                [],
            )
            .map_err(io::Error::other)?;
            tx.execute_batch(
                "DROP TRIGGER IF EXISTS compact_name_search_entries_ai;
                 DROP TRIGGER IF EXISTS compact_name_search_entries_au;
                 DROP TRIGGER IF EXISTS compact_name_search_entries_ad;
                 DROP TRIGGER IF EXISTS compact_name_search_changes_cap;",
            )
            .map_err(io::Error::other)?;
        }
        tx.execute_batch(CHANGE_TRIGGERS)
            .map_err(io::Error::other)?;
        if !change_tracking_ready(&tx)? || !batch_schema_ready(&tx)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "compact_name_search_change_feed_invalid",
            ));
        }
        let schema_version: i64 = tx
            .query_row(
                "SELECT schema_version FROM compact_name_search_state WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(io::Error::other)?;
        if schema_version != SCHEMA_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "compact_name_search_schema_version_mismatch",
            ));
        }
        tx.commit().map_err(io::Error::other)
    }

    fn lock_file(&self) -> io::Result<File> {
        fs::create_dir_all(&self.sidecar_dir)?;
        OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(self.sidecar_dir.join("index.lock"))
    }

    fn load_manifest_index(&self) -> io::Result<()> {
        let _lock = self.lock_file()?;
        _lock.lock_shared()?;
        let conn = self.pool.get().map_err(io::Error::other)?;
        let row: Option<(i64, i64, i64, Option<String>)> = conn
            .query_row(
                "SELECT schema_version, format_version, base_generation, file_name
                 FROM compact_name_search_state WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(io::Error::other)?;
        let Some((schema_version, format_version, generation, Some(file_name))) = row else {
            return Ok(());
        };
        if schema_version != SCHEMA_VERSION
            || format_version != FORMAT_VERSION as i64
            || generation < 0
            || !is_safe_file_name(&file_name)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "compact_name_search_manifest_invalid",
            ));
        }
        let index = Arc::new(CompactIndex::open(&self.sidecar_dir.join(&file_name))?);
        if index.generation != generation {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "compact_name_search_generation_mismatch",
            ));
        }
        *self
            .loaded
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(LoadedIndex { file_name, index });
        Ok(())
    }

    fn index_for_manifest(
        &self,
        file_name: &str,
        generation: i64,
    ) -> io::Result<Arc<CompactIndex>> {
        if !is_safe_file_name(file_name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "compact_name_search_file_name_invalid",
            ));
        }
        if let Some(loaded) = self
            .loaded
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            if loaded.file_name == file_name && loaded.index.generation == generation {
                return Ok(Arc::clone(&loaded.index));
            }
        }
        let index = Arc::new(CompactIndex::open(&self.sidecar_dir.join(file_name))?);
        if index.generation != generation {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "compact_name_search_generation_mismatch",
            ));
        }
        *self
            .loaded
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(LoadedIndex {
            file_name: file_name.to_owned(),
            index: Arc::clone(&index),
        });
        Ok(index)
    }

    fn request_rebuild(self: &Arc<Self>) {
        if !self.is_enabled()
            || self
                .rebuild_in_progress
                .compare_exchange(false, true, AtomicOrdering::AcqRel, AtomicOrdering::Acquire)
                .is_err()
        {
            return;
        }
        let manager = Arc::clone(self);
        let spawn = std::thread::Builder::new()
            .name("compact-name-search-build".into())
            .spawn(move || {
                let result = manager.rebuild_and_install();
                manager
                    .rebuild_in_progress
                    .store(false, AtomicOrdering::Release);
                if let Err(error) = result {
                    manager.set_error(format!("compact_name_search_rebuild:{error}"));
                    manager
                        .counters
                        .fallbacks
                        .fetch_add(1, AtomicOrdering::Relaxed);
                }
            });
        if let Err(error) = spawn {
            self.rebuild_in_progress
                .store(false, AtomicOrdering::Release);
            self.set_error(format!("compact_name_search_worker:{error}"));
        }
    }

    fn rebuild_and_install(&self) -> io::Result<()> {
        if !self.is_enabled()
            || self
                .owner_lease
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
            || !self.sidecar_dir.is_dir()
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "compact_name_search_rebuild_without_owner_lease",
            ));
        }
        self.counters.rebuilds.fetch_add(1, AtomicOrdering::Relaxed);
        self.install_change_tracking()?;
        let mut conn = self.pool.get().map_err(io::Error::other)?;
        let transaction = conn.transaction().map_err(io::Error::other)?;
        let data = CompactIndexData::build(&transaction)?;
        let generation = data.generation;
        transaction.commit().map_err(io::Error::other)?;

        let file_name = format!("index-{generation}-{}.cns", Uuid::new_v4());
        let index_path = self.sidecar_dir.join(&file_name);
        write_index_file(&index_path, &data)?;
        let mapped = Arc::new(CompactIndex::open(&index_path)?);

        let lock = self.lock_file()?;
        lock.lock()?;
        let mut writer = self.pool.get().map_err(io::Error::other)?;
        let tx = writer
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(io::Error::other)?;
        let (current_format, current_generation, current_file, current_change_floor): (
            i64,
            i64,
            Option<String>,
            i64,
        ) = tx
            .query_row(
                "SELECT format_version, base_generation, file_name, change_floor
                 FROM compact_name_search_state WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(io::Error::other)?;
        if current_change_floor > generation {
            drop(tx);
            let _ = fs::remove_file(&index_path);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "compact_name_search_feed_gap_during_rebuild",
            ));
        }
        if current_generation > generation
            || (current_generation == generation
                && current_format == FORMAT_VERSION as i64
                && current_file.as_deref().is_some_and(|name| {
                    is_safe_file_name(name)
                        && self.sidecar_dir.join(name).exists()
                        && CompactIndex::open(&self.sidecar_dir.join(name))
                            .is_ok_and(|index| index.generation == generation)
                }))
        {
            drop(tx);
            let _ = fs::remove_file(&index_path);
            return Ok(());
        }
        tx.execute(
            "UPDATE compact_name_search_state
             SET format_version=?1, base_generation=?2, file_name=?3, change_floor=?2
             WHERE singleton=1 AND schema_version=?4",
            params![FORMAT_VERSION as i64, generation, file_name, SCHEMA_VERSION],
        )
        .map_err(io::Error::other)?;
        // Keep the highest row as a monotonic rowid sentinel. The next change
        // then receives a generation above the base even though the feed table
        // uses SQLite's cheaper plain INTEGER PRIMARY KEY allocator.
        tx.execute(
            "DELETE FROM compact_name_search_changes WHERE seq < ?1",
            [generation],
        )
        .map_err(io::Error::other)?;
        tx.commit().map_err(io::Error::other)?;

        let old = {
            let mut loaded = self
                .loaded
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            loaded.replace(LoadedIndex {
                file_name: file_name.clone(),
                index: mapped,
            })
        };
        drop(old);
        self.collect_old_indexes(&file_name);
        Ok(())
    }

    fn collect_old_indexes(&self, keep: &str) {
        let Ok(entries) = fs::read_dir(&self.sidecar_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if name != keep && name.starts_with("index-") && name.ends_with(".cns") {
                let _ = fs::remove_file(path);
            }
        }
    }

    fn set_error(&self, value: String) {
        *self
            .last_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(value);
    }

    #[allow(dead_code)]
    pub fn diagnostics(&self) -> CompactNameSearchDiagnostics {
        let path_relation_complete = self
            .loaded
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|loaded| loaded.index.path_relation_complete);
        CompactNameSearchDiagnostics {
            enabled: self.is_enabled(),
            queries: self.counters.queries.load(AtomicOrdering::Relaxed),
            accepted_queries: self.counters.accepted_queries.load(AtomicOrdering::Relaxed),
            fallbacks: self.counters.fallbacks.load(AtomicOrdering::Relaxed),
            rebuilds: self.counters.rebuilds.load(AtomicOrdering::Relaxed),
            last_overlay_events: self.counters.overlay_events.load(AtomicOrdering::Relaxed),
            last_name_candidates: self.counters.name_candidates.load(AtomicOrdering::Relaxed),
            last_path_candidates: self.counters.path_candidates.load(AtomicOrdering::Relaxed),
            path_relation_complete,
            manifest_overlay_micros: self
                .counters
                .manifest_overlay_micros
                .load(AtomicOrdering::Relaxed),
            name_prefilter_micros: self
                .counters
                .name_prefilter_micros
                .load(AtomicOrdering::Relaxed),
            name_scan_heap_micros: self
                .counters
                .name_scan_heap_micros
                .load(AtomicOrdering::Relaxed),
            path_scan_heap_micros: self
                .counters
                .path_scan_heap_micros
                .load(AtomicOrdering::Relaxed),
            top_k_sort_page_micros: self
                .counters
                .top_k_sort_page_micros
                .load(AtomicOrdering::Relaxed),
            page_hydration_micros: self
                .counters
                .page_hydration_micros
                .load(AtomicOrdering::Relaxed),
            last_error: self
                .last_error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        }
    }

    /// Return `None` when the compact index cannot prove a complete result.
    /// The caller then runs the existing SQLite tier in the same snapshot.
    pub(super) fn search_page(
        self: &Arc<Self>,
        conn: &Connection,
        query: &str,
        target: usize,
        page_skip: usize,
        page_len: usize,
        already_seen: &HashSet<String>,
    ) -> Result<Option<Vec<GlobalSearchResult>>, DbError> {
        self.counters.queries.fetch_add(1, AtomicOrdering::Relaxed);
        if !self.is_enabled() || !supported_query(query) || target == 0 {
            self.counters
                .fallbacks
                .fetch_add(1, AtomicOrdering::Relaxed);
            return Ok(None);
        }

        let outcome =
            self.search_page_inner(conn, query, target, page_skip, page_len, already_seen);
        match outcome {
            Ok(Some(outcome)) => {
                self.counters
                    .accepted_queries
                    .fetch_add(1, AtomicOrdering::Relaxed);
                self.counters
                    .overlay_events
                    .store(outcome.overlay_events, AtomicOrdering::Relaxed);
                self.counters
                    .name_candidates
                    .store(outcome.name_candidates, AtomicOrdering::Relaxed);
                self.counters
                    .path_candidates
                    .store(outcome.path_candidates, AtomicOrdering::Relaxed);
                self.counters.manifest_overlay_micros.store(
                    outcome.timings.manifest_overlay_micros,
                    AtomicOrdering::Relaxed,
                );
                self.counters.name_prefilter_micros.store(
                    outcome.timings.name_prefilter_micros,
                    AtomicOrdering::Relaxed,
                );
                self.counters.name_scan_heap_micros.store(
                    outcome.timings.name_scan_heap_micros,
                    AtomicOrdering::Relaxed,
                );
                self.counters.path_scan_heap_micros.store(
                    outcome.timings.path_scan_heap_micros,
                    AtomicOrdering::Relaxed,
                );
                self.counters.top_k_sort_page_micros.store(
                    outcome.timings.top_k_sort_page_micros,
                    AtomicOrdering::Relaxed,
                );
                self.counters.page_hydration_micros.store(
                    outcome.timings.page_hydration_micros,
                    AtomicOrdering::Relaxed,
                );
                Ok(Some(outcome.page))
            }
            Ok(None) => {
                self.counters
                    .fallbacks
                    .fetch_add(1, AtomicOrdering::Relaxed);
                self.request_rebuild();
                Ok(None)
            }
            Err(error) => {
                self.set_error(format!("compact_name_search_query:{error}"));
                self.counters
                    .fallbacks
                    .fetch_add(1, AtomicOrdering::Relaxed);
                self.request_rebuild();
                Ok(None)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn enable_for_test(self: &Arc<Self>) -> io::Result<()> {
        let lease = self
            .try_owner_lease(true)?
            .ok_or_else(|| io::Error::other("compact_name_search_owner_already_active"))?;
        self.enabled.store(true, AtomicOrdering::Release);
        self.install_change_tracking()?;
        *self
            .owner_lease
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(lease);
        self.feed_batching.store(true, AtomicOrdering::Release);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn rebuild_now(&self) -> io::Result<()> {
        self.rebuild_and_install()
    }

    #[cfg(test)]
    pub(crate) fn drop_mapping_for_test(&self) {
        *self
            .loaded
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

impl Database {
    #[cfg(feature = "compact-name-search-candidate")]
    pub(crate) fn compact_name_search(&self) -> &Arc<CompactNameSearchManager> {
        &self.compact_name_search
    }

    #[cfg(all(test, feature = "compact-name-search-candidate"))]
    pub(crate) fn enable_compact_name_search_for_test(&self) -> io::Result<()> {
        self.compact_name_search.enable_for_test()
    }

    #[cfg(all(test, feature = "compact-name-search-candidate"))]
    pub(crate) fn rebuild_compact_name_search_for_test(&self) -> io::Result<()> {
        self.compact_name_search.rebuild_now()
    }
}

const FLAG_DIRECTORY: u8 = 1;
const FLAG_HIDDEN: u8 = 2;
const FLAG_SYSTEM: u8 = 4;
const FLAG_STALE: u8 = 8;
const NO_MATCH: u8 = 0;

#[cfg(test)]
#[path = "../tests/compact_name_search.rs"]
mod tests;
