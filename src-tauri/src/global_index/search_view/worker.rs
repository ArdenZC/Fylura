use super::{
    memory::{BudgetCharge, MemoryBudget, DEFAULT_TOTAL_BUDGET_BYTES},
    view::{SearchView, ViewError, MAX_CANDIDATE_WINDOW, MAX_INCREMENTAL_CHANGES},
};
use crate::{
    db::schema_global_search_changes,
    global_index::{models::GlobalSearchResult, search, search::SearchRoute},
};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError},
        Arc, Mutex, RwLock, Weak,
    },
    thread,
    time::{Duration, Instant},
};

const SHADOW_ENVIRONMENT_VARIABLE: &str = "ZEN_CANVAS_GLOBAL_SEARCH_SHADOW";
const MIN_DENSE_SHADOW_PAGE_SIZE: usize = 80;
const MAX_SHADOW_PROBE_ENTRIES: usize = 100_000;
const RETRY_INITIAL_DELAY: Duration = Duration::from_millis(100);
const RETRY_MAX_DELAY: Duration = Duration::from_secs(5);
const MAX_CONSECUTIVE_RETRIES: u32 = 5;

#[derive(Clone)]
pub(crate) struct SearchViewManager {
    shared: Arc<SharedState>,
}

#[derive(Debug, PartialEq, Eq)]
enum CandidateProbeRoute {
    SqliteTier,
    EmptyOrSparse,
    UnsupportedQuery,
    Probe(String),
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
    sqlite_tier_skip: AtomicU64,
    empty_or_sparse_skip: AtomicU64,
    large_corpus_skip: AtomicU64,
    candidate_probe: AtomicU64,
    missing_or_stale_view: AtomicU64,
    feed_gap_or_source_change: AtomicU64,
    feed_saturation: AtomicU64,
    update_budget: AtomicU64,
    retries: AtomicU64,
    retry_exhausted: AtomicU64,
    memory_refusal: AtomicU64,
    database_error: AtomicU64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchViewDiagnostics {
    pub(crate) worker_enabled: bool,
    pub(crate) feed_generation: i64,
    pub(crate) view_generation: Option<i64>,
    pub(crate) generation_lag: i64,
    pub(crate) feed_saturated: bool,
    pub(crate) fallback_reason: Option<&'static str>,
    pub(crate) accounted_memory_used_bytes: usize,
    pub(crate) accounted_memory_peak_bytes: usize,
    pub(crate) refresh_requests: u64,
    pub(crate) full_builds: u64,
    pub(crate) incremental_batches: u64,
    pub(crate) comparisons: u64,
    pub(crate) comparable_membership_checks: u64,
    pub(crate) full_membership_equal: u64,
    pub(crate) window_membership_equal: u64,
    pub(crate) ordered_equal: u64,
    pub(crate) page_equal: u64,
    pub(crate) production_page_equal: u64,
    pub(crate) sqlite_tier_skips: u64,
    pub(crate) empty_or_sparse_skips: u64,
    pub(crate) large_corpus_skips: u64,
    pub(crate) unsupported_queries: u64,
    pub(crate) candidate_probes: u64,
    pub(crate) missing_or_stale_views: u64,
    pub(crate) feed_saturation_observations: u64,
    pub(crate) feed_gap_or_source_changes: u64,
    pub(crate) update_budget_failures: u64,
    pub(crate) retries: u64,
    pub(crate) retry_exhausted: u64,
    pub(crate) memory_refusals: u64,
    pub(crate) database_errors: u64,
}

impl SearchViewManager {
    pub(crate) fn requested() -> bool {
        std::env::var(SHADOW_ENVIRONMENT_VARIABLE)
            .ok()
            .is_some_and(|value| value == "1")
    }

    pub(crate) fn start(
        pool: Pool<SqliteConnectionManager>,
        change_schema_ready: bool,
        requested: bool,
    ) -> Self {
        let enabled = change_schema_ready && requested;
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

    pub(crate) fn diagnostics(
        &self,
        conn: &Connection,
    ) -> Result<Option<SearchViewDiagnostics>, crate::db::DbError> {
        let feed_installed = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM main.sqlite_schema WHERE type='table' AND name='global_search_change_state')",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if !feed_installed {
            return Ok(None);
        }
        let state = schema_global_search_changes::read_state(conn)?;
        let view_generation = self
            .shared
            .current
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|view| view.generation);
        let generation_lag = view_generation
            .map(|generation| state.generation.saturating_sub(generation).max(0))
            .unwrap_or(state.generation);
        let metrics = &self.shared.metrics;
        let memory_refusals = metrics.memory_refusal.load(Ordering::Relaxed);
        let retries_exhausted = metrics.retry_exhausted.load(Ordering::Relaxed);
        let feed_gaps = metrics.feed_gap_or_source_change.load(Ordering::Relaxed);
        let database_errors = metrics.database_error.load(Ordering::Relaxed);
        let fallback_reason = if !self.shared.enabled.load(Ordering::Acquire) {
            Some("worker_disabled")
        } else if state.feed_saturated {
            Some("feed_saturated")
        } else if memory_refusals > 0 {
            Some("memory_budget")
        } else if retries_exhausted > 0 {
            Some("retry_exhausted")
        } else if feed_gaps > 0 {
            Some("feed_gap_or_source_change")
        } else if database_errors > 0 {
            Some("database_error")
        } else if view_generation.is_none() {
            Some("view_missing")
        } else if generation_lag > 0 {
            Some("generation_lag")
        } else {
            None
        };

        Ok(Some(SearchViewDiagnostics {
            worker_enabled: self.shared.enabled.load(Ordering::Acquire),
            feed_generation: state.generation,
            view_generation,
            generation_lag,
            feed_saturated: state.feed_saturated,
            fallback_reason,
            accounted_memory_used_bytes: self.shared.budget.used(),
            accounted_memory_peak_bytes: self.shared.budget.peak(),
            refresh_requests: metrics.refresh_requests.load(Ordering::Relaxed),
            full_builds: metrics.full_builds.load(Ordering::Relaxed),
            incremental_batches: metrics.incremental_batches.load(Ordering::Relaxed),
            comparisons: metrics.comparisons.load(Ordering::Relaxed),
            comparable_membership_checks: metrics
                .comparable_membership_checks
                .load(Ordering::Relaxed),
            full_membership_equal: metrics.full_membership_equal.load(Ordering::Relaxed),
            window_membership_equal: metrics.window_membership_equal.load(Ordering::Relaxed),
            ordered_equal: metrics.ordered_equal.load(Ordering::Relaxed),
            page_equal: metrics.page_equal.load(Ordering::Relaxed),
            production_page_equal: metrics.production_page_equal.load(Ordering::Relaxed),
            sqlite_tier_skips: metrics.sqlite_tier_skip.load(Ordering::Relaxed),
            empty_or_sparse_skips: metrics.empty_or_sparse_skip.load(Ordering::Relaxed),
            large_corpus_skips: metrics.large_corpus_skip.load(Ordering::Relaxed),
            unsupported_queries: metrics.unsupported_query.load(Ordering::Relaxed),
            candidate_probes: metrics.candidate_probe.load(Ordering::Relaxed),
            missing_or_stale_views: metrics.missing_or_stale_view.load(Ordering::Relaxed),
            feed_saturation_observations: metrics.feed_saturation.load(Ordering::Relaxed),
            feed_gap_or_source_changes: feed_gaps,
            update_budget_failures: metrics.update_budget.load(Ordering::Relaxed),
            retries: metrics.retries.load(Ordering::Relaxed),
            retry_exhausted: retries_exhausted,
            memory_refusals,
            database_errors,
        }))
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
        route: SearchRoute,
        production_results: &[GlobalSearchResult],
    ) {
        if !self.shared.enabled.load(Ordering::Acquire) {
            return;
        }
        let candidate_query =
            match candidate_probe_route(query, limit, offset, route, production_results.len()) {
                CandidateProbeRoute::SqliteTier => {
                    self.shared
                        .metrics
                        .sqlite_tier_skip
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                }
                CandidateProbeRoute::EmptyOrSparse => {
                    self.shared
                        .metrics
                        .empty_or_sparse_skip
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                }
                CandidateProbeRoute::UnsupportedQuery => {
                    self.shared
                        .metrics
                        .unsupported_query
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                }
                CandidateProbeRoute::Probe(query) => query,
            };
        self.shared
            .metrics
            .candidate_probe
            .fetch_add(1, Ordering::Relaxed);
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
        if state.feed_saturated {
            self.shared
                .metrics
                .feed_saturation
                .fetch_add(1, Ordering::Relaxed);
            self.request_refresh();
            return;
        }
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
        if !candidate_probe_corpus_supported(view.indexed_entry_upper_bound()) {
            self.shared
                .metrics
                .large_corpus_skip
                .fetch_add(1, Ordering::Relaxed);
            return;
        }

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

    #[cfg(test)]
    pub(crate) fn request_refresh_for_test(&self) {
        self.request_refresh();
    }

    #[cfg(test)]
    pub(crate) fn current_generation_for_test(&self) -> Option<i64> {
        self.shared
            .current
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|view| view.generation)
    }

    #[cfg(test)]
    pub(crate) fn enabled_for_test(&self) -> bool {
        self.shared.enabled.load(Ordering::Acquire)
    }
}

fn worker_loop(
    pool: Pool<SqliteConnectionManager>,
    receiver: Receiver<()>,
    shared: Weak<SharedState>,
) {
    let mut retry_pending = false;
    let mut consecutive_retries = 0u32;
    let mut retry_delay = RETRY_INITIAL_DELAY;
    loop {
        let wake = if retry_pending {
            wait_for_retry_delay(&receiver, retry_delay)
        } else {
            receiver.recv().is_ok()
        };
        if !wake {
            break;
        }
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
        match result {
            Ok(()) => {
                retry_pending = false;
                consecutive_retries = 0;
                retry_delay = RETRY_INITIAL_DELAY;
            }
            Err(error) => {
                record_worker_error(&shared, &error);
                match retry_delay_after_error(&error, consecutive_retries) {
                    Some(next_delay) => {
                        consecutive_retries += 1;
                        retry_pending = true;
                        retry_delay = next_delay;
                        shared.metrics.retries.fetch_add(1, Ordering::Relaxed);
                    }
                    None => {
                        shared
                            .metrics
                            .retry_exhausted
                            .fetch_add(1, Ordering::Relaxed);
                        disable_worker(&shared);
                        break;
                    }
                }
            }
        }
    }
}

fn wait_for_retry_delay(receiver: &Receiver<()>, delay: Duration) -> bool {
    let started = Instant::now();
    match receiver.recv_timeout(delay) {
        Ok(()) => {
            thread::sleep(delay.saturating_sub(started.elapsed()));
            true
        }
        Err(RecvTimeoutError::Timeout) => true,
        Err(RecvTimeoutError::Disconnected) => false,
    }
}

fn retry_delay_after_error(error: &ViewError, consecutive_retries: u32) -> Option<Duration> {
    if matches!(error, ViewError::MemoryBudget | ViewError::ViewBudget)
        || consecutive_retries >= MAX_CONSECUTIVE_RETRIES
    {
        return None;
    }
    if !matches!(error, ViewError::UpdateBudget | ViewError::Database(_)) {
        return None;
    }
    let multiplier = 1u32
        .checked_shl(consecutive_retries.min(8))
        .unwrap_or(u32::MAX);
    Some(
        RETRY_INITIAL_DELAY
            .saturating_mul(multiplier)
            .min(RETRY_MAX_DELAY),
    )
}

fn disable_worker(shared: &Arc<SharedState>) {
    shared.enabled.store(false, Ordering::Release);
    *shared
        .current
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    shared
        .sender
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
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
            .all(|character| character.is_ascii_alphabetic())
    {
        return None;
    }
    Some(query.to_ascii_lowercase())
}

fn candidate_probe_route(
    query: &str,
    limit: u32,
    offset: u32,
    route: SearchRoute,
    result_count: usize,
) -> CandidateProbeRoute {
    if route.deepest_tier != search::SearchTier::FullText || route.earlier_tiers_returned_rows {
        return CandidateProbeRoute::SqliteTier;
    }
    let page_size = limit.clamp(1, 200) as usize;
    if offset != 0 || page_size < MIN_DENSE_SHADOW_PAGE_SIZE || result_count < page_size {
        return CandidateProbeRoute::EmptyOrSparse;
    }
    supported_candidate_query(query)
        .map(CandidateProbeRoute::Probe)
        .unwrap_or(CandidateProbeRoute::UnsupportedQuery)
}

fn candidate_probe_corpus_supported(indexed_entries: usize) -> bool {
    indexed_entries <= MAX_SHADOW_PROBE_ENTRIES
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
