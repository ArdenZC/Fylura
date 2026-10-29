use super::*;
use crate::db::Database;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, RwLock},
};

#[test]
fn candidate_query_router_short_circuits_sqlite_tiers_empty_sparse_and_ambiguous_queries() {
    let full_text = SearchRoute {
        deepest_tier: search::SearchTier::FullText,
        earlier_tiers_returned_rows: false,
    };
    let route = |query, limit, offset, search_route, count| {
        candidate_probe_route(query, limit, offset, search_route, count)
    };

    assert_eq!(
        route("report", 80, 0, full_text, 80),
        CandidateProbeRoute::Probe("report".into())
    );
    assert_eq!(
        route(
            "pdf",
            80,
            0,
            SearchRoute {
                deepest_tier: search::SearchTier::ExactExtension,
                earlier_tiers_returned_rows: false,
            },
            80,
        ),
        CandidateProbeRoute::SqliteTier
    );
    assert_eq!(
        route(
            "Month-03",
            80,
            0,
            SearchRoute {
                deepest_tier: search::SearchTier::PunctuationPrefix,
                earlier_tiers_returned_rows: false,
            },
            80,
        ),
        CandidateProbeRoute::SqliteTier
    );
    assert_eq!(
        route("report", 80, 0, full_text, 0),
        CandidateProbeRoute::EmptyOrSparse
    );
    assert_eq!(
        route("invoice", 80, 0, full_text, 1),
        CandidateProbeRoute::EmptyOrSparse
    );
    assert_eq!(
        route("report", 10, 0, full_text, 10),
        CandidateProbeRoute::EmptyOrSparse
    );
    assert_eq!(
        route("中文查询", 80, 0, full_text, 80),
        CandidateProbeRoute::UnsupportedQuery
    );
    assert_eq!(
        route(
            "report",
            80,
            0,
            SearchRoute {
                deepest_tier: search::SearchTier::FullText,
                earlier_tiers_returned_rows: true,
            },
            80,
        ),
        CandidateProbeRoute::SqliteTier
    );
}

#[test]
fn candidate_probe_never_scans_corpora_above_the_measured_100k_budget() {
    assert!(candidate_probe_corpus_supported(100_000));
    assert!(!candidate_probe_corpus_supported(100_001));
    assert!(!candidate_probe_corpus_supported(1_000_000));
}

#[test]
fn worker_retry_policy_is_bounded_and_backs_off_after_transient_errors() {
    let delays = (0..MAX_CONSECUTIVE_RETRIES)
        .map(|attempt| retry_delay_after_error(&ViewError::UpdateBudget, attempt).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(delays[0], Duration::from_millis(100));
    assert_eq!(delays[1], Duration::from_millis(200));
    assert_eq!(delays[2], Duration::from_millis(400));
    assert!(delays.iter().all(|delay| *delay <= RETRY_MAX_DELAY));
    assert!(retry_delay_after_error(&ViewError::Database("busy".into()), 0).is_some());
    assert!(retry_delay_after_error(&ViewError::UpdateBudget, MAX_CONSECUTIVE_RETRIES).is_none());
    assert!(retry_delay_after_error(&ViewError::MemoryBudget, 0).is_none());
    assert!(retry_delay_after_error(&ViewError::InvalidFeed, 0).is_none());
}

#[test]
fn worker_retry_backoff_cannot_be_bypassed_by_queued_refresh_wakes() {
    let (sender, receiver) = sync_channel(1);
    sender.try_send(()).unwrap();
    let delay = Duration::from_millis(30);
    let started = Instant::now();
    assert!(wait_for_retry_delay(&receiver, delay));
    assert!(started.elapsed() >= delay);
}

fn candidate_ids(view: &SearchView, query: &str, budget: Arc<MemoryBudget>) -> Vec<String> {
    view.candidate_window(query, budget)
        .unwrap()
        .ids
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn remove_database(path: &PathBuf) {
    let _ = fs::remove_file(path);
    let mut wal = path.clone();
    wal.set_extension("db-wal");
    let _ = fs::remove_file(wal);
    let mut shm = path.clone();
    shm.set_extension("db-shm");
    let _ = fs::remove_file(shm);
}

#[test]
fn refresh_worker_rebuilds_incrementally_and_falls_back_on_source_invalidation() {
    let path = std::env::temp_dir().join(format!(
        "zen-canvas-search-view-worker-{}.db",
        std::process::id()
    ));
    remove_database(&path);
    let database = Database::open_with_search_change_feed_for_test(&path, true).unwrap();
    let mut conn = database.conn().unwrap();
    conn.execute(
        "INSERT INTO global_volumes (id, platform, stable_volume_id, display_name, mount_path, filesystem_type, drive_kind, enabled, provider, index_status, created_at, updated_at) VALUES ('worker-volume', 'windows', 'worker-volume-id', 'Worker fixture', 'C:\\Synthetic', 'ntfs', 'fixed', 1, 'windows_mft', 'ready', 1, 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO global_entries (id, volume_id, platform_file_id, parent_platform_file_id, name, name_normalized, path, path_normalized, extension, is_directory, size, modified_at_fs, file_attributes, is_hidden, is_system, is_stale, source_provider, last_seen_at) VALUES ('worker-entry', 'worker-volume', 'worker-file-id', 'worker-parent-id', 'Report.txt', 'report.txt', 'C:\\Synthetic\\Report.txt', 'c:\\synthetic\\report.txt', 'txt', 0, 10, 1, 0, 0, 0, 0, 'windows_mft', 1)",
        [],
    )
    .unwrap();

    let shared = Arc::new(SharedState {
        enabled: AtomicBool::new(true),
        current: RwLock::new(None),
        budget: MemoryBudget::new(DEFAULT_TOTAL_BUDGET_BYTES),
        sender: Mutex::new(None),
        metrics: ShadowMetrics::default(),
    });
    refresh_once(&mut conn, &shared).unwrap();
    let first_view = shared
        .current
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .unwrap();
    assert_eq!(
        first_view.generation,
        schema_global_search_changes::read_state(&conn)
            .unwrap()
            .generation
    );
    assert_eq!(
        candidate_ids(&first_view, "report", shared.budget.clone()),
        vec!["worker-entry".to_owned()]
    );

    conn.execute(
        "UPDATE global_entries SET name='Invoice.pdf', name_normalized='invoice.pdf', path='C:\\Synthetic\\Invoice.pdf', path_normalized='c:\\synthetic\\invoice.pdf', extension='pdf', modified_at_fs=2 WHERE id='worker-entry'",
        [],
    )
    .unwrap();
    refresh_once(&mut conn, &shared).unwrap();
    let second_view = shared
        .current
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .unwrap();
    assert!(second_view.generation > first_view.generation);
    assert_eq!(
        candidate_ids(&first_view, "report", shared.budget.clone()),
        vec!["worker-entry".to_owned()]
    );
    assert!(candidate_ids(&second_view, "report", shared.budget.clone()).is_empty());
    assert_eq!(
        candidate_ids(&second_view, "invoice", shared.budget.clone()),
        vec!["worker-entry".to_owned()]
    );

    conn.execute(
        "UPDATE global_volumes SET enabled=0 WHERE id='worker-volume'",
        [],
    )
    .unwrap();
    refresh_once(&mut conn, &shared).unwrap();
    let third_view = shared
        .current
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .unwrap();
    assert!(third_view.generation > second_view.generation);
    assert!(candidate_ids(&third_view, "invoice", shared.budget.clone()).is_empty());
    assert_eq!(shared.metrics.full_builds.load(Ordering::Relaxed), 2);
    assert_eq!(
        shared.metrics.incremental_batches.load(Ordering::Relaxed),
        1
    );

    drop(third_view);
    drop(second_view);
    drop(first_view);
    drop(conn);
    drop(database);
    remove_database(&path);
}
