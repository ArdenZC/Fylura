//! Additional correctness and resource evidence for the single authorized 1M run.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct ProcessPeak {
    samples: u64,
    private_bytes: Option<u64>,
    rss_bytes: Option<u64>,
}

impl ProcessPeak {
    fn observe(&mut self, sample: Option<NativeProcessSample>) {
        let Some(sample) = sample else {
            return;
        };
        self.samples += 1;
        self.private_bytes = max_option(self.private_bytes, sample.private_bytes);
        self.rss_bytes = max_option(self.rss_bytes, sample.rss_bytes);
    }
}

fn max_option(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    }
}

/// Samples the native process while the production candidate index is built.
/// The internal heap estimate remains separately reported by diagnostics.
pub(super) fn rebuild_index_with_sampled_peak(
    db: &Database,
) -> (
    Option<NativeProcessSample>,
    Option<NativeProcessSample>,
    JsonValue,
) {
    let before = native_process_sample();
    let stop = Arc::new(AtomicBool::new(false));
    let peak = Arc::new(Mutex::new(ProcessPeak {
        samples: 0,
        private_bytes: None,
        rss_bytes: None,
    }));
    let sampler_stop = Arc::clone(&stop);
    let sampler_peak = Arc::clone(&peak);
    let sampler = std::thread::spawn(move || loop {
        sampler_peak
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .observe(native_process_sample());
        if sampler_stop.load(Ordering::Acquire) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    });

    db.filename_search.enable_for_test();
    let build_started = Instant::now();
    let result = db.filename_search.rebuild_now_for_test();
    let build_wall_ms = build_started.elapsed().as_secs_f64() * 1_000.0;
    stop.store(true, Ordering::Release);
    sampler.join().expect("join process peak sampler");
    result.expect("synchronously build 1M filename candidate");
    let after = native_process_sample();
    let peak = peak
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(
        peak.samples > 0,
        "native process sampler must observe the build"
    );

    let delta = |high: Option<u64>, base: Option<u64>| Some(high? as i64 - base? as i64);
    let record = json!({
        "measured": true,
        "sampling_interval_ms": 5,
        "sample_count": peak.samples,
        "build_wall_ms": round_ms(build_wall_ms),
        "peak_private_bytes": peak.private_bytes,
        "peak_private_delta_from_prebuild_bytes": delta(peak.private_bytes, before.and_then(|sample| sample.private_bytes)),
        "peak_working_set_bytes": peak.rss_bytes,
        "peak_working_set_delta_from_prebuild_bytes": delta(peak.rss_bytes, before.and_then(|sample| sample.rss_bytes)),
        "prebuild": native_sample_json(before),
        "postbuild": native_sample_json(after),
        "sampling_method": if cfg!(target_os = "windows") {
            "5ms polling of Windows PrivateUsage and working set"
        } else {
            "5ms polling of macOS phys_footprint and resident size"
        }
    });
    (before, after, record)
}

/// Uses a rare, complete filename phrase from the unchanged 1M fixture.
/// The SQL oracle compares active filename substrings and the product sort key.
pub(super) fn sparse_filename_case(connection: &Connection) -> CandidateCase {
    const QUERY: &str = "annual report 0999982";
    let expected_total: u64 = connection
        .query_row(
            "SELECT COUNT(*)
             FROM global_entries ge
             JOIN global_volumes gv ON gv.id=ge.volume_id
             WHERE gv.enabled=1 AND gv.coverage_pending=0 AND ge.is_stale=0
               AND instr(ge.name_normalized, ?1) > 0",
            [QUERY],
            |row| row.get::<_, i64>(0),
        )
        .expect("count sparse filename matches") as u64;
    let mut statement = connection
        .prepare(
            "SELECT ge.id
             FROM global_entries ge
             JOIN global_volumes gv ON gv.id=ge.volume_id
             WHERE gv.enabled=1 AND gv.coverage_pending=0 AND ge.is_stale=0
               AND instr(ge.name_normalized, ?1) > 0
             ORDER BY ge.modified_at_fs DESC, ge.id COLLATE BINARY ASC
             LIMIT 80",
        )
        .expect("prepare sparse filename oracle");
    let expected_ids = statement
        .query_map([QUERY], |row| row.get::<_, String>(0))
        .expect("query sparse filename oracle")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect sparse filename oracle");
    assert!(
        (1..=100).contains(&expected_total),
        "sparse filename fixture must have between one and 100 matches, got {expected_total}"
    );
    assert_eq!(expected_ids.len() as u64, expected_total);
    CandidateCase {
        class: "sparse_filename_substring",
        query: QUERY,
        offset: 0,
        expected_ids,
        expected_total,
    }
}

/// Retrieves every page through the public Search route and checks the full
/// 4,096-candidate ordering against the independent generated-entry oracle.
pub(super) fn verify_complete_candidate_window(
    db: &Database,
    oracle: &QueryExpectation,
    context: &JsonValue,
) -> JsonValue {
    db.filename_search.set_enabled_for_test(true);
    let expected = oracle
        .expected_page(0, SEARCH_CANDIDATE_LIMIT)
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut actual = Vec::with_capacity(SEARCH_CANDIDATE_LIMIT);
    let mut page_count = 0_u32;
    let started = Instant::now();
    for offset in (0..SEARCH_CANDIDATE_LIMIT).step_by(SEARCH_RESULT_LIMIT as usize) {
        let limit = (SEARCH_CANDIDATE_LIMIT - offset).min(SEARCH_RESULT_LIMIT as usize) as u32;
        let page = db
            .search_global_entries("report", limit, offset as u32)
            .expect("read complete candidate window through public Search route");
        actual.extend(page.into_iter().map(|result| result.id));
        page_count += 1;
    }
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    assert_eq!(actual.len(), SEARCH_CANDIDATE_LIMIT);
    assert_eq!(actual, expected, "complete 4,096 candidate IDs and order");
    json!({
        "schema_version": 1,
        "record_type": "issue494_1m_full_candidate_window",
        "context": context,
        "query": "report",
        "offset": 0,
        "candidate_window_size": SEARCH_CANDIDATE_LIMIT,
        "page_size": SEARCH_RESULT_LIMIT,
        "page_count": page_count,
        "full_window_wall_ms": round_ms(elapsed_ms),
        "candidate_ids": actual,
        "independent_oracle_ids": expected,
        "full_candidate_window_and_order_equal": true
    })
}

/// Revokes the synthetic source after the paired write samples, then proves
/// candidate fallback and Repository Snapshot facts remain SQLite-consistent.
pub(super) fn verify_revoked_source_fallback(db: &Database, context: &JsonValue) -> JsonValue {
    db.filename_search.set_enabled_for_test(true);

    // The paired writer arm intentionally uses separate SQLite connections.
    // Reconcile their committed changes before testing a source-state change,
    // so the required fallback reason is authorization rather than a data gap.
    let rebuilds_before = db.filename_search.diagnostics().rebuilds;
    let _ = db
        .search_global_entries("report", SEARCH_RESULT_LIMIT, 0)
        .expect("probe and invalidate any externally modified candidate base");
    let recovery_deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let diagnostics = db.filename_search.diagnostics();
        if diagnostics.ready
            && !diagnostics.rebuild_in_progress
            && diagnostics.rebuilds > rebuilds_before
        {
            break;
        }
        assert!(
            Instant::now() < recovery_deadline,
            "candidate index did not recover from controlled paired writes"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let authorized_snapshot = db
        .search_global_entries_snapshot("report", SEARCH_RESULT_LIMIT, 0)
        .expect("read authorized snapshot before revocation");
    assert_eq!(
        authorized_snapshot.results.len(),
        SEARCH_RESULT_LIMIT as usize
    );
    let fallback_before = db.filename_search.diagnostics().fallback_source_change;
    db.invalidate_missing_global_index_source("gv_test", true)
        .expect("revoke synthetic source authorization");

    let revoked_candidate_snapshot = db
        .search_global_entries_snapshot("report", SEARCH_RESULT_LIMIT, 0)
        .expect("candidate route must safely fall back after source revocation");
    assert!(
        revoked_candidate_snapshot.results.is_empty(),
        "a revoked source must contribute no search results"
    );
    let diagnostics_after_fallback = db.filename_search.diagnostics();
    assert!(
        diagnostics_after_fallback.fallback_source_change > fallback_before,
        "source-state mismatch must be recorded as a safe candidate fallback"
    );
    assert!(revoked_candidate_snapshot
        .source_health
        .iter()
        .any(|fact| fact.source_id == "gv_test" && !fact.enabled));
    assert_ne!(
        authorized_snapshot.source_revision, revoked_candidate_snapshot.source_revision,
        "source revision must reflect revoked authorization"
    );

    let revocation_rebuild_deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let diagnostics = db.filename_search.diagnostics();
        if diagnostics.ready && !diagnostics.rebuild_in_progress {
            break;
        }
        assert!(
            Instant::now() < revocation_rebuild_deadline,
            "candidate worker did not settle after authorization fallback"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let settled_diagnostics = db.filename_search.diagnostics();

    db.filename_search.set_enabled_for_test(false);
    let sqlite_snapshot = db
        .search_global_entries_snapshot("report", SEARCH_RESULT_LIMIT, 0)
        .expect("read authoritative SQLite snapshot after revocation");
    assert_snapshot_equal(&sqlite_snapshot, &revoked_candidate_snapshot);
    let (enabled, authorization): (i64, String) = db
        .conn()
        .expect("read revoked source row")
        .query_row(
            "SELECT enabled, scope_authorization_state FROM global_volumes WHERE id='gv_test'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query revoked source facts");
    assert_eq!(enabled, 0);
    assert_eq!(authorization, "not_authorized");

    json!({
        "schema_version": 1,
        "record_type": "issue494_1m_source_revocation_fallback",
        "context": context,
        "pre_revocation_search_result_count": authorized_snapshot.results.len(),
        "revoked_candidate_result_count": revoked_candidate_snapshot.results.len(),
        "source_enabled_after_revocation": enabled,
        "scope_authorization_after_revocation": authorization,
        "source_health_reports_disabled": revoked_candidate_snapshot.source_health.iter().any(|fact| fact.source_id == "gv_test" && !fact.enabled),
        "source_revision_changed": authorized_snapshot.source_revision != revoked_candidate_snapshot.source_revision,
        "source_fallback_counter_before": fallback_before,
        "source_fallback_counter_after": diagnostics_after_fallback.fallback_source_change,
        "candidate_route_safely_fell_back_on_source_change": true,
        "revoked_candidate_snapshot_equals_sqlite_snapshot": true,
        "no_revoked_source_results_returned": true,
        "sqlite_snapshot_source_revision": sqlite_snapshot.source_revision,
        "revoked_snapshot_source_revision": revoked_candidate_snapshot.source_revision,
        "index_rebuilds_before_write_gap_probe": rebuilds_before,
        "index_rebuilds_after_write_gap_recovery": diagnostics_after_fallback.rebuilds,
        "index_rebuilds_after_revocation_worker_settled": settled_diagnostics.rebuilds,
        "candidate_rebuild_worker_settled": !settled_diagnostics.rebuild_in_progress,
        "fallback_counters": {
            "untracked_change": diagnostics_after_fallback.fallback_untracked_change,
            "source_change": diagnostics_after_fallback.fallback_source_change
        }
    })
}
