use super::{
    memory::{BudgetCharge, MemoryBudget, DEFAULT_TOTAL_BUDGET_BYTES},
    view::{SearchView, ViewError, MAX_CANDIDATE_WINDOW, MAX_INCREMENTAL_CHANGES},
};
use crate::{
    db::schema_global_search_changes,
    global_index::{models::GlobalSearchResult, search},
};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{sync_channel, Receiver, SyncSender, TrySendError},
        Arc, Mutex, RwLock, Weak,
    },
    thread,
    time::Instant,
};

const SHADOW_ENVIRONMENT_VARIABLE: &str = "ZEN_CANVAS_GLOBAL_SEARCH_SHADOW";

#[derive(Clone)]
pub(crate) struct SearchViewManager {
    shared: Arc<SharedState>,
}

struct SharedState {
    enabled: AtomicBool,
    current: RwLock<Option<Arc<SearchView>>>,
    budget: Arc<MemoryBudget>,
    sender: Mutex<Option<SyncSender<()>>>,
    metrics: ShadowMetrics,
}

#[derive(Default)]
struct ShadowMetrics {
    refresh_requests: AtomicU64,
    full_builds: AtomicU64,
    incremental_batches: AtomicU64,
    comparisons: AtomicU64,
    comparable_membership_checks: AtomicU64,
    full_membership_equal: AtomicU64,
    window_membership_equal: AtomicU64,
    ordered_equal: AtomicU64,
    page_equal: AtomicU64,
    production_page_equal: AtomicU64,
    unsupported_query: AtomicU64,
    missing_or_stale_view: AtomicU64,
    feed_gap_or_source_change: AtomicU64,
    update_budget: AtomicU64,
    memory_refusal: AtomicU64,
    database_error: AtomicU64,
}

impl SearchViewManager {
    pub(crate) fn start(pool: Pool<SqliteConnectionManager>, change_schema_ready: bool) -> Self {
        let enabled = change_schema_ready
            && std::env::var(SHADOW_ENVIRONMENT_VARIABLE)
                .ok()
                .is_some_and(|value| value == "1");
        let shared = Arc::new(SharedState {
            enabled: AtomicBool::new(enabled),
            current: RwLock::new(None),
            budget: MemoryBudget::new(DEFAULT_TOTAL_BUDGET_BYTES),
            sender: Mutex::new(None),
            metrics: ShadowMetrics::default(),
        });
        if enabled {
            let (sender, receiver) = sync_channel(1);
            *shared
                .sender
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(sender);
            let weak = Arc::downgrade(&shared);
            if thread::Builder::new()
                .name("global-search-view".into())
                .spawn(move || worker_loop(pool, receiver, weak))
                .is_err()
            {
                shared.enabled.store(false, Ordering::Release);
                shared
                    .sender
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
            }
        }
        Self { shared }
    }

    /// Observe a production snapshot without changing its result. The shadow
    /// candidate is compared only when its immutable view has exactly the
    /// snapshot's durable generation; otherwise SQLite remains the answer and
    /// one bounded worker wake is queued.
    pub(crate) fn observe_snapshot(
        &self,
        conn: &Connection,
        query: &str,
        limit: u32,
        offset: u32,
        production_results: &[GlobalSearchResult],
    ) {
        if !self.shared.enabled.load(Ordering::Acquire) {
            return;
        }
        let Some(candidate_query) = supported_candidate_query(query) else {
            self.shared
                .metrics
                .unsupported_query
                .fetch_add(1, Ordering::Relaxed);
            return;
        };
        let reference_query = query.trim().to_ascii_lowercase();
        let state = match schema_global_search_changes::read_state(conn) {
            Ok(state) => state,
            Err(_) => {
                self.shared
                    .metrics
                    .database_error
                    .fetch_add(1, Ordering::Relaxed);
                self.request_refresh();
                return;
            }
        };
        let view = self
            .shared
            .current
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let Some(view) = view.filter(|view| view.generation == state.generation) else {
            self.shared
                .metrics
                .missing_or_stale_view
                .fetch_add(1, Ordering::Relaxed);
            self.request_refresh();
            return;
        };

        let candidates = match view.candidate_window(&candidate_query, self.shared.budget.clone()) {
            Ok(candidates) => candidates,
            Err(ViewError::MemoryBudget | ViewError::ViewBudget) => {
                self.shared
                    .metrics
                    .memory_refusal
                    .fetch_add(1, Ordering::Relaxed);
                return;
            }
            Err(_) => {
                self.shared
                    .metrics
                    .database_error
                    .fetch_add(1, Ordering::Relaxed);
                return;
            }
        };
        let comparison_charge = match BudgetCharge::reserved(
            self.shared.budget.clone(),
            MAX_CANDIDATE_WINDOW
                .saturating_mul(384)
                .saturating_add(64 * 1024),
        ) {
            Some(charge) => charge,
            None => {
                self.shared
                    .metrics
                    .memory_refusal
                    .fetch_add(1, Ordering::Relaxed);
                return;
            }
        };
        let sqlite = match search::search_global_fts_shadow_reference_ids(
            conn,
            &reference_query,
            MAX_CANDIDATE_WINDOW as u32,
        ) {
            Ok(rows) => rows,
            Err(_) => {
                self.shared
                    .metrics
                    .database_error
                    .fetch_add(1, Ordering::Relaxed);
                return;
            }
        };
        self.compare_candidates(
            &candidates.ids,
            candidates.matching_count,
            &sqlite,
            limit,
            offset,
            production_results,
        );
        drop(comparison_charge);
    }

    fn compare_candidates(
        &self,
        candidate_ids: &[&str],
        candidate_count: usize,
        sqlite: &[String],
        limit: u32,
        offset: u32,
        production_results: &[GlobalSearchResult],
    ) {
        let mut candidate_membership = candidate_ids.to_vec();
        candidate_membership.sort_unstable();
        let mut sqlite_membership = sqlite.iter().map(String::as_str).collect::<Vec<_>>();
        sqlite_membership.sort_unstable();
        let window_membership_equal = candidate_membership == sqlite_membership;
        let full_comparable = candidate_count <= MAX_CANDIDATE_WINDOW;
        let full_membership_equal =
            full_comparable && candidate_count == sqlite.len() && window_membership_equal;
        let ordered_equal = candidate_ids
            == sqlite
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice();
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let page_len = limit.clamp(1, 200) as usize;
        let candidate_page = candidate_ids
            .get(start..start.saturating_add(page_len).min(candidate_ids.len()))
            .unwrap_or_default();
        let sqlite_page = sqlite
            .get(start..start.saturating_add(page_len).min(sqlite.len()))
            .unwrap_or_default()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let page_equal = candidate_page == sqlite_page;
        let production_page_equal = candidate_page
            == production_results
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>()
                .as_slice();

        let metrics = &self.shared.metrics;
        metrics.comparisons.fetch_add(1, Ordering::Relaxed);
        if full_comparable {
            metrics
                .comparable_membership_checks
                .fetch_add(1, Ordering::Relaxed);
            if full_membership_equal {
                metrics
                    .full_membership_equal
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        if window_membership_equal {
            metrics
                .window_membership_equal
                .fetch_add(1, Ordering::Relaxed);
        }
        if ordered_equal {
            metrics.ordered_equal.fetch_add(1, Ordering::Relaxed);
        }
        if page_equal {
            metrics.page_equal.fetch_add(1, Ordering::Relaxed);
        }
        if production_page_equal {
            metrics
                .production_page_equal
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    fn request_refresh(&self) {
        request_refresh_shared(&self.shared);
    }
}

fn worker_loop(
    pool: Pool<SqliteConnectionManager>,
    receiver: Receiver<()>,
    shared: Weak<SharedState>,
) {
    while receiver.recv().is_ok() {
        let Some(shared) = shared.upgrade() else {
            break;
        };
        if !shared.enabled.load(Ordering::Acquire) {
            continue;
        }
        let result = pool
            .get()
            .map_err(|error| ViewError::Database(format!("search_view_pool:{error}")))
            .and_then(|mut conn| refresh_once(&mut conn, &shared));
        if let Err(error) = result {
            record_worker_error(&shared, &error);
        }
    }
}

fn refresh_once(conn: &mut Connection, shared: &Arc<SharedState>) -> Result<(), ViewError> {
    let current = shared
        .current
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let transaction = conn.transaction()?;
    let state = schema_global_search_changes::read_state(&transaction)?;
    if let Some(view) = &current {
        if view.generation == state.generation && !state.feed_saturated {
            transaction.commit()?;
            return Ok(());
        }
    }

    let needs_full_build = current.as_ref().is_none_or(|view| {
        view.generation > state.generation
            || state.feed_saturated
            || state.feed_floor_generation > view.generation
    });
    if !needs_full_build {
        let view = current.as_ref().expect("checked current search view");
        let mut events = schema_global_search_changes::changes_since(
            &transaction,
            view.generation,
            MAX_INCREMENTAL_CHANGES + 1,
        )?;
        if events.is_empty() {
            transaction.rollback()?;
            return rebuild_from_snapshot(conn, shared, current);
        }
        if events.len() <= MAX_INCREMENTAL_CHANGES
            && events.last().map(|event| event.generation) != Some(state.generation)
        {
            transaction.rollback()?;
            return rebuild_from_snapshot(conn, shared, current);
        }
        if events.len() > MAX_INCREMENTAL_CHANGES {
            events.truncate(MAX_INCREMENTAL_CHANGES);
        }
        let started_at = Instant::now();
        match view.apply_changes(&transaction, &events, shared.budget.clone(), started_at) {
            Ok(next_view) => {
                let has_more = next_view.generation < state.generation;
                transaction.commit()?;
                publish(shared, next_view);
                shared
                    .metrics
                    .incremental_batches
                    .fetch_add(1, Ordering::Relaxed);
                if has_more {
                    request_refresh_shared(shared);
                }
                return Ok(());
            }
            Err(
                ViewError::RebuildRequired | ViewError::TooManyChanges | ViewError::InvalidFeed,
            ) => {
                transaction.rollback()?;
                return rebuild_from_snapshot(conn, shared, current);
            }
            Err(error) => return Err(error),
        }
    }
    transaction.rollback()?;
    rebuild_from_snapshot(conn, shared, current)
}

fn rebuild_from_snapshot(
    conn: &mut Connection,
    shared: &Arc<SharedState>,
    previous: Option<Arc<SearchView>>,
) -> Result<(), ViewError> {
    *shared
        .current
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    drop(previous);
    let transaction = conn.transaction()?;
    let state = schema_global_search_changes::read_state(&transaction)?;
    let generation = state.generation;
    let view = SearchView::build(&transaction, generation, shared.budget.clone())?;
    transaction.commit()?;

    let _reset = schema_global_search_changes::reset_after_full_snapshot(conn, generation)?;
    let current_state = schema_global_search_changes::read_state(conn)?;
    if current_state.feed_saturated
        || current_state.feed_floor_generation > generation
        || current_state.generation < generation
    {
        return Err(ViewError::InvalidFeed);
    }
    publish(shared, view);
    shared.metrics.full_builds.fetch_add(1, Ordering::Relaxed);
    if current_state.generation > generation {
        request_refresh_shared(shared);
    }
    Ok(())
}

fn request_refresh_shared(shared: &Arc<SharedState>) {
    if !shared.enabled.load(Ordering::Acquire) {
        return;
    }
    let sender = shared
        .sender
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if let Some(sender) = sender {
        match sender.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => {
                shared
                    .metrics
                    .refresh_requests
                    .fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(())) => {
                shared.enabled.store(false, Ordering::Release);
            }
        }
    }
}

fn publish(shared: &Arc<SharedState>, view: SearchView) {
    *shared
        .current
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(view));
}

fn record_worker_error(shared: &Arc<SharedState>, error: &ViewError) {
    match error {
        ViewError::MemoryBudget | ViewError::ViewBudget => {
            shared
                .metrics
                .memory_refusal
                .fetch_add(1, Ordering::Relaxed);
            *shared
                .current
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        }
        ViewError::UpdateBudget | ViewError::TooManyChanges => {
            shared.metrics.update_budget.fetch_add(1, Ordering::Relaxed);
        }
        ViewError::RebuildRequired | ViewError::InvalidFeed => {
            shared
                .metrics
                .feed_gap_or_source_change
                .fetch_add(1, Ordering::Relaxed);
        }
        ViewError::Database(reason) => {
            shared
                .metrics
                .database_error
                .fetch_add(1, Ordering::Relaxed);
            eprintln!("global_search_view_unavailable reason={reason}");
        }
    }
}

pub(super) fn supported_candidate_query(query: &str) -> Option<String> {
    let query = query.trim();
    if query.len() > 64
        || query.len() < 3
        || !query
            .chars()
            .all(|character| character.is_ascii_graphic() || character == ' ')
    {
        return None;
    }
    Some(query.to_ascii_lowercase().replace('\\', "/"))
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
