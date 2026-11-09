use super::{
    fixture::WorkspaceFixture,
    harness::{open_fixture, open_path, runtime_for, try_open_fixture},
    metrics,
    resources::{self, ProcessResources},
};
use crate::file_workspace::{
    browse::{
        BrowseQuerySpecV1, BrowseSortDirection, BrowseSortField, BrowseSortSpec,
        DEFAULT_MAX_BROWSE_ENTRY_REFS, DEFAULT_MAX_BROWSE_PATH_REFS,
        DEFAULT_MAX_BROWSE_PROCESS_ENTRY_REFS, DEFAULT_MAX_BROWSE_PROCESS_PATH_REFS,
    },
    integration::types::{
        BrowseCompletionDto, BrowseNextPageRequest, BrowseSessionRequest,
        BrowseStartEnumerationRequest,
    },
};
use serde_json::json;
use std::time::Instant;

const PAGE_SIZE: usize = 256;
const WORKLOAD_ENTRIES: usize = 100_000;

struct EnumerationObservation {
    pages: usize,
    entries: usize,
    first_page_entries: usize,
    first_page_ms: u128,
    total_ms: u128,
    completion: &'static str,
    error: Option<String>,
    live_entry_refs: usize,
    live_path_refs: usize,
    peak_process: ProcessResources,
}

#[test]
#[ignore = "W1-11 multi-session Browse capacity and ref-pressure evidence"]
fn browse_session_capacity_remains_bounded() {
    let session_fixture = WorkspaceFixture::smoke();
    let runtime = runtime_for(&session_fixture);
    let mut sessions = Vec::new();
    for index in 0..32 {
        sessions.push(
            open_fixture(&runtime, &session_fixture, &format!("capacity-{index}")).session_id,
        );
    }
    assert_eq!(sessions.len(), 32);
    assert_eq!(
        try_open_fixture(&runtime, &session_fixture, "capacity-overflow")
            .expect_err("the 33rd Browse session must fail closed"),
        "browse_session_capacity_exceeded"
    );
    for session_id in sessions {
        runtime
            .dispose_browse(BrowseSessionRequest { session_id })
            .expect("dispose bounded Browse session");
    }
    let capacity_settled = runtime.resource_counts();
    assert_eq!(capacity_settled.browse_service_sessions, 0);
    metrics::emit_metric(
        "browse_multi_session_capacity",
        metrics::HARD_PASS,
        [
            ("max_sessions".to_string(), json!(32)),
            ("overflow_behavior".to_string(), json!("fail closed")),
            (
                "settled_sessions".to_string(),
                json!(capacity_settled.browse_service_sessions),
            ),
            ("fixture_root_scope".to_string(), json!("repository-local")),
        ],
    );

    // Keep multiple real sessions populated at the same time. The fixture is
    // deliberately larger than the aggregate budget so the fourth session
    // must fail closed even though each individual session is below its
    // per-session 100k/16,384 limits.
    let entry_fixture = WorkspaceFixture::split("multi-session-entry-pressure", 4, 45_100, 5_000);
    let mut entry_sessions = Vec::new();
    for index in 0..4 {
        let opened = open_path(
            &runtime,
            &entry_fixture.child_path(index),
            &format!("entry-pressure-{index}"),
        );
        entry_sessions.push((opened.session_id, opened.root_path_ref));
    }
    let mut entry_observations = Vec::new();
    let mut pressure_peak = resources::snapshot();
    for (index, (session_id, path_ref)) in entry_sessions.iter().enumerate() {
        let observation = enumerate_without_releasing(
            &runtime,
            session_id.clone(),
            path_ref.clone(),
            &format!("entry-pressure-{index}"),
        );
        pressure_peak = pressure_peak.max(observation.peak_process);
        entry_observations.push(observation);
    }
    assert!(entry_observations[..3]
        .iter()
        .all(|observation| observation.error.is_none() && observation.entries == 50_100));
    assert_eq!(
        entry_observations[3].error.as_deref(),
        Some("browse_temporary_state_capacity_exceeded")
    );
    let entry_pressure_counts = runtime.resource_counts();
    assert!(entry_pressure_counts.browse_entry_refs > DEFAULT_MAX_BROWSE_ENTRY_REFS);
    assert!(entry_pressure_counts.browse_entry_refs <= DEFAULT_MAX_BROWSE_PROCESS_ENTRY_REFS);
    assert!(entry_pressure_counts.browse_path_refs <= DEFAULT_MAX_BROWSE_PROCESS_PATH_REFS);

    // Releasing two populated sessions makes the exact same session/cursor
    // capacity available again; no refs are evicted from the remaining
    // sessions to achieve this.
    for (session_id, _) in entry_sessions.iter().take(2) {
        runtime
            .dispose_browse(BrowseSessionRequest {
                session_id: session_id.clone(),
            })
            .expect("release populated entry-pressure session");
    }
    let entry_retry = enumerate_without_releasing(
        &runtime,
        entry_sessions[3].0.clone(),
        entry_sessions[3].1.clone(),
        "entry-pressure-retry",
    );
    assert_eq!(entry_retry.error, None);
    assert_eq!(entry_retry.entries, 50_100);
    pressure_peak = pressure_peak.max(entry_retry.peak_process);
    for (session_id, _) in entry_sessions.iter().skip(2) {
        runtime
            .dispose_browse(BrowseSessionRequest {
                session_id: session_id.clone(),
            })
            .expect("dispose entry-pressure session");
    }

    // Repeat the same admission proof with directory-heavy real paths so the
    // independent aggregate PathRef budget is exercised as well.
    let path_fixture = WorkspaceFixture::split("multi-session-path-pressure", 4, 500, 9_000);
    let mut path_sessions = Vec::new();
    for index in 0..4 {
        let opened = open_path(
            &runtime,
            &path_fixture.child_path(index),
            &format!("path-pressure-{index}"),
        );
        path_sessions.push((opened.session_id, opened.root_path_ref));
    }
    let mut path_observations = Vec::new();
    for (index, (session_id, path_ref)) in path_sessions.iter().enumerate() {
        let observation = enumerate_without_releasing(
            &runtime,
            session_id.clone(),
            path_ref.clone(),
            &format!("path-pressure-{index}"),
        );
        pressure_peak = pressure_peak.max(observation.peak_process);
        path_observations.push(observation);
    }
    assert!(path_observations[..3]
        .iter()
        .all(|observation| observation.error.is_none()));
    assert_eq!(
        path_observations[3].error.as_deref(),
        Some("browse_temporary_state_capacity_exceeded")
    );
    let path_pressure_counts = runtime.resource_counts();
    assert!(path_pressure_counts.browse_path_refs > DEFAULT_MAX_BROWSE_PATH_REFS);
    assert!(path_pressure_counts.browse_path_refs <= DEFAULT_MAX_BROWSE_PROCESS_PATH_REFS);
    assert!(path_pressure_counts.browse_entry_refs <= DEFAULT_MAX_BROWSE_PROCESS_ENTRY_REFS);

    for (session_id, _) in path_sessions.iter().take(2) {
        runtime
            .dispose_browse(BrowseSessionRequest {
                session_id: session_id.clone(),
            })
            .expect("release populated path-pressure session");
    }
    let path_retry = enumerate_without_releasing(
        &runtime,
        path_sessions[3].0.clone(),
        path_sessions[3].1.clone(),
        "path-pressure-retry",
    );
    assert_eq!(path_retry.error, None);
    pressure_peak = pressure_peak.max(path_retry.peak_process);
    for (session_id, _) in path_sessions.iter().skip(2) {
        runtime
            .dispose_browse(BrowseSessionRequest {
                session_id: session_id.clone(),
            })
            .expect("dispose path-pressure session");
    }

    let settled = runtime.resource_counts();
    assert_eq!(settled.browse_service_sessions, 0);
    assert_eq!(settled.browse_entry_refs, 0);
    assert_eq!(settled.browse_path_refs, 0);
    assert_eq!(settled.browse_active_enumerations, 0);
    assert!(runtime.dispose());
    metrics::emit_metric(
        "browse_multi_session_ref_pressure",
        metrics::HARD_PASS,
        [
            ("max_sessions".to_string(), json!(32)),
            (
                "per_session_max_entry_refs".to_string(),
                json!(DEFAULT_MAX_BROWSE_ENTRY_REFS),
            ),
            (
                "per_session_max_path_refs".to_string(),
                json!(DEFAULT_MAX_BROWSE_PATH_REFS),
            ),
            (
                "aggregate_max_entry_refs".to_string(),
                json!(DEFAULT_MAX_BROWSE_PROCESS_ENTRY_REFS),
            ),
            (
                "aggregate_max_path_refs".to_string(),
                json!(DEFAULT_MAX_BROWSE_PROCESS_PATH_REFS),
            ),
            (
                "entry_pressure_overflow".to_string(),
                json!("browse_temporary_state_capacity_exceeded"),
            ),
            (
                "path_pressure_overflow".to_string(),
                json!("browse_temporary_state_capacity_exceeded"),
            ),
            (
                "entry_peak_live_entry_refs".to_string(),
                json!(entry_pressure_counts.browse_entry_refs),
            ),
            (
                "entry_peak_live_path_refs".to_string(),
                json!(entry_pressure_counts.browse_path_refs),
            ),
            (
                "path_peak_live_entry_refs".to_string(),
                json!(path_pressure_counts.browse_entry_refs),
            ),
            (
                "path_peak_live_path_refs".to_string(),
                json!(path_pressure_counts.browse_path_refs),
            ),
            ("peak_rss_bytes".to_string(), json!(pressure_peak.rss_bytes)),
            (
                "peak_handle_count".to_string(),
                json!(pressure_peak.handle_count),
            ),
            ("peak_fd_count".to_string(), json!(pressure_peak.fd_count)),
            (
                "capacity_recovery".to_string(),
                json!("dispose_populated_sessions_then_retry"),
            ),
            (
                "settled_sessions".to_string(),
                json!(settled.browse_service_sessions),
            ),
            (
                "settled_entry_refs".to_string(),
                json!(settled.browse_entry_refs),
            ),
            (
                "settled_path_refs".to_string(),
                json!(settled.browse_path_refs),
            ),
            (
                "settled_active_enumerations".to_string(),
                json!(settled.browse_active_enumerations),
            ),
            ("fixture_root_scope".to_string(), json!("repository-local")),
            ("frontend_entry_ref_eviction".to_string(), json!(false)),
            ("history_path_ref_eviction".to_string(), json!(false)),
        ],
    );
}

fn enumerate_without_releasing(
    runtime: &crate::file_workspace::integration::FileWorkspaceRuntime,
    session_id: String,
    path_ref: crate::file_workspace::BrowsePathRef,
    request_id: &str,
) -> EnumerationObservation {
    let started = Instant::now();
    let first = runtime.start_enumeration(BrowseStartEnumerationRequest {
        session_id: session_id.clone(),
        request_id: request_id.to_string(),
        path_ref,
        page_size: PAGE_SIZE,
        query: Default::default(),
    });
    let first_page_ms = started.elapsed().as_millis();
    let mut pages = 0;
    let mut entries = 0;
    let mut first_page_entries = 0;
    let mut cursor = None;
    let mut error = None;
    let mut owned_pages = Vec::new();
    let mut peak_process = resources::snapshot();

    match first {
        Ok(page) => {
            pages += 1;
            entries += page.entries.len();
            first_page_entries = page.entries.len();
            cursor = page.next_cursor.clone();
            owned_pages.push(page);
            peak_process = peak_process.max(resources::snapshot());
        }
        Err(message) => error = Some(message),
    }

    while let Some(next_cursor) = cursor.take() {
        match runtime.next_page(BrowseNextPageRequest {
            session_id: session_id.clone(),
            cursor: next_cursor,
            page_size: PAGE_SIZE,
        }) {
            Ok(page) => {
                pages += 1;
                entries += page.entries.len();
                cursor = page.next_cursor.clone();
                owned_pages.push(page);
                peak_process = peak_process.max(resources::snapshot());
            }
            Err(message) => {
                error = Some(message);
                break;
            }
        }
    }

    let total_ms = started.elapsed().as_millis();
    let counts = runtime.resource_counts();
    let completion = if error.is_some() {
        "capacity_error"
    } else if owned_pages
        .last()
        .is_some_and(|page| page.completion == BrowseCompletionDto::Complete)
    {
        "complete"
    } else {
        "incomplete"
    };

    // The vector intentionally remains live until this observation returns: it
    // mirrors the W1-10 controller's published-page ownership contract.
    drop(owned_pages);
    EnumerationObservation {
        pages,
        entries,
        first_page_entries,
        first_page_ms,
        total_ms,
        completion,
        error,
        live_entry_refs: counts.browse_entry_refs,
        live_path_refs: counts.browse_path_refs,
        peak_process,
    }
}

#[test]
#[ignore = "W1-11 100k Browse capacity baseline; run before changing BrowseLimits"]
fn browse_100k_capacity_baseline() {
    let cases = [
        ("entry-heavy-100k", 99_000, 1_000),
        ("path-heavy-100k", 90_000, 10_000),
    ];

    for (label, file_count, directory_count) in cases {
        let fixture = WorkspaceFixture::large(label, file_count, directory_count);
        let runtime =
            super::harness::runtime_for_browse_limits(&fixture, 4_096, 1_024, 4_096, 1_024);
        let opened = open_fixture(&runtime, &fixture, label);
        let observation = enumerate_without_releasing(
            &runtime,
            opened.session_id.clone(),
            opened.root_path_ref,
            label,
        );

        assert_eq!(
            observation.error.as_deref(),
            Some("browse_temporary_state_capacity_exceeded"),
            "legacy bounds should fail closed for {label}"
        );
        assert!(
            observation.entries > 0,
            "{label} should publish progressive pages"
        );
        assert!(
            observation.pages > 0,
            "{label} should publish at least one page"
        );
        assert!(runtime.dispose());
        let settled = runtime.resource_counts();
        assert_eq!(settled.browse_service_sessions, 0);

        metrics::emit_metric(
            "browse_100k_capacity_baseline",
            metrics::OBSERVED,
            [
                ("case".to_string(), json!(label)),
                ("fixture_entries".to_string(), json!(WORKLOAD_ENTRIES)),
                ("fixture_files".to_string(), json!(file_count)),
                ("fixture_directories".to_string(), json!(directory_count)),
                ("requested_page_size".to_string(), json!(PAGE_SIZE)),
                ("legacy_max_entry_refs".to_string(), json!(4_096)),
                ("legacy_max_path_refs".to_string(), json!(1_024)),
                ("pages".to_string(), json!(observation.pages)),
                ("published_entries".to_string(), json!(observation.entries)),
                (
                    "first_page_ms".to_string(),
                    json!(observation.first_page_ms),
                ),
                ("total_ms".to_string(), json!(observation.total_ms)),
                ("completion".to_string(), json!(observation.completion)),
                ("error".to_string(), json!(observation.error)),
                (
                    "peak_live_entry_refs".to_string(),
                    json!(observation.live_entry_refs),
                ),
                (
                    "peak_live_path_refs".to_string(),
                    json!(observation.live_path_refs),
                ),
                ("fixture_root_scope".to_string(), json!("repository-local")),
                ("raw_path_authority".to_string(), json!(false)),
                ("query_v2_involved".to_string(), json!(false)),
            ],
        );
    }
}

#[test]
#[ignore = "W1-11 100k Browse progressive bounded ownership gate"]
fn browse_100k_progressive_bounded_ownership() {
    let fixture = WorkspaceFixture::large("mixed-100k", 90_000, 10_000);
    let runtime = runtime_for(&fixture);
    let opened = open_fixture(&runtime, &fixture, "mixed-100k");
    // Establish the idle process baseline after the fixture/runtime setup and
    // before Browse enumeration. Setup time is not part of first-page timing.
    let idle_process = resources::snapshot();
    let observation = enumerate_without_releasing(
        &runtime,
        opened.session_id.clone(),
        opened.root_path_ref.clone(),
        "workspace-foundation-100k",
    );

    assert_eq!(
        observation.error, None,
        "100k Browse must not hit bounded capacity"
    );
    assert_eq!(observation.completion, "complete");
    assert_eq!(observation.entries, WORKLOAD_ENTRIES);
    assert!(observation.first_page_entries > 0);
    assert!(
        observation.first_page_entries < WORKLOAD_ENTRIES,
        "the first useful page must precede full enumeration"
    );
    assert!(
        observation.pages > 1,
        "100k Browse must publish multiple pages"
    );
    assert!(observation.live_entry_refs <= DEFAULT_MAX_BROWSE_ENTRY_REFS);
    assert!(observation.live_path_refs <= DEFAULT_MAX_BROWSE_PATH_REFS);

    metrics::emit_metric(
        "browse_100k_first_page",
        // This is one local sample, not a p95 measurement. Keep the latency
        // target visible while classifying the evidence honestly.
        metrics::OBSERVED,
        [
            ("fixture_entries".to_string(), json!(WORKLOAD_ENTRIES)),
            ("requested_page_size".to_string(), json!(PAGE_SIZE)),
            (
                "first_page_entries".to_string(),
                json!(observation.first_page_entries),
            ),
            (
                "first_page_ms".to_string(),
                json!(observation.first_page_ms),
            ),
            ("first_page_sample_count".to_string(), json!(1)),
            ("target_ms_p95".to_string(), json!(250)),
            ("fixture_root_scope".to_string(), json!("repository-local")),
        ],
    );
    metrics::emit_metric(
        "browse_100k_progressive_bounded_ownership",
        metrics::HARD_PASS,
        [
            ("fixture_entries".to_string(), json!(WORKLOAD_ENTRIES)),
            ("fixture_files".to_string(), json!(90_000)),
            ("fixture_directories".to_string(), json!(10_000)),
            ("requested_page_size".to_string(), json!(PAGE_SIZE)),
            ("effective_page_size".to_string(), json!(PAGE_SIZE)),
            ("pages".to_string(), json!(observation.pages)),
            ("published_entries".to_string(), json!(observation.entries)),
            (
                "first_page_ms".to_string(),
                json!(observation.first_page_ms),
            ),
            ("total_ms".to_string(), json!(observation.total_ms)),
            ("completion".to_string(), json!(observation.completion)),
            (
                "peak_live_entry_refs".to_string(),
                json!(observation.live_entry_refs),
            ),
            (
                "peak_live_path_refs".to_string(),
                json!(observation.live_path_refs),
            ),
            (
                "new_max_entry_refs".to_string(),
                json!(DEFAULT_MAX_BROWSE_ENTRY_REFS),
            ),
            (
                "new_max_path_refs".to_string(),
                json!(DEFAULT_MAX_BROWSE_PATH_REFS),
            ),
            (
                "boundedness_proof".to_string(),
                json!("fixed per-session and independent process-wide caps; no eviction or unbounded registry"),
            ),
            (
                "frontend_entry_ref_validity".to_string(),
                json!("published pages retained until teardown"),
            ),
            (
                "history_path_ref_lifetime".to_string(),
                json!("W1-10 pins remain valid until history disposal"),
            ),
            (
                "capacity_behavior".to_string(),
                json!("fail closed at exact bound"),
            ),
            ("max_sessions".to_string(), json!(32)),
            (
                "theoretical_process_entry_ref_bound".to_string(),
                json!(DEFAULT_MAX_BROWSE_PROCESS_ENTRY_REFS),
            ),
            (
                "theoretical_process_path_ref_bound".to_string(),
                json!(DEFAULT_MAX_BROWSE_PROCESS_PATH_REFS),
            ),
            (
                "new_max_process_entry_refs".to_string(),
                json!(DEFAULT_MAX_BROWSE_PROCESS_ENTRY_REFS),
            ),
            (
                "new_max_process_path_refs".to_string(),
                json!(DEFAULT_MAX_BROWSE_PROCESS_PATH_REFS),
            ),
            (
                "aggregate_budget_strategy".to_string(),
                json!("independent process-wide live-ref caps; max_sessions remains 32"),
            ),
            (
                "multi_session_capacity_behavior".to_string(),
                json!("32-session count cap plus real aggregate EntryRef/PathRef pressure"),
            ),
            ("timing_includes_resource_sampling".to_string(), json!(true)),
            ("fixture_root_scope".to_string(), json!("repository-local")),
            ("raw_path_authority".to_string(), json!(false)),
            ("query_v2_involved".to_string(), json!(false)),
            ("idle_rss_bytes".to_string(), json!(idle_process.rss_bytes)),
            (
                "peak_rss_bytes".to_string(),
                json!(observation.peak_process.rss_bytes),
            ),
            (
                "idle_handle_count".to_string(),
                json!(idle_process.handle_count),
            ),
            (
                "peak_handle_count".to_string(),
                json!(observation.peak_process.handle_count),
            ),
            ("idle_fd_count".to_string(), json!(idle_process.fd_count)),
            (
                "peak_fd_count".to_string(),
                json!(observation.peak_process.fd_count),
            ),
        ],
    );

    // Reuse the same 100k fixture and runtime for the sorted path. No extra
    // harness, fixture generation, or changed performance threshold.
    let sorted_started = Instant::now();
    let mut page = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: opened.session_id.clone(),
            request_id: "sorted-100k".into(),
            path_ref: opened.root_path_ref,
            page_size: PAGE_SIZE,
            query: BrowseQuerySpecV1 {
                sort: Some(BrowseSortSpec {
                    field: BrowseSortField::Name,
                    direction: BrowseSortDirection::Desc,
                    folders_first: true,
                }),
                ..Default::default()
            },
        })
        .expect("sorted 100k admission");
    let mut count = 0;
    let mut previous: Option<(bool, String)> = None;
    let mut first_useful_ms = None;
    let mut peak = resources::snapshot();
    loop {
        if !page.entries.is_empty() && first_useful_ms.is_none() {
            first_useful_ms = Some(sorted_started.elapsed().as_millis());
        }
        for entry in &page.entries {
            let is_file = entry.kind == super::super::types::BrowseEntryKindDto::File;
            if let Some((prior_file, prior_name)) = &previous {
                assert!(!*prior_file || is_file, "folders first across pages");
                if *prior_file == is_file {
                    assert!(
                        prior_name > &entry.name,
                        "global descending order across pages"
                    );
                }
            }
            previous = Some((is_file, entry.name.clone()));
            count += 1;
        }
        peak = peak.max(resources::snapshot());
        let Some(cursor) = page.next_cursor.take() else {
            break;
        };
        page = runtime
            .next_page(BrowseNextPageRequest {
                session_id: opened.session_id.clone(),
                cursor,
                page_size: PAGE_SIZE,
            })
            .expect("sorted 100k continuation");
    }
    assert_eq!(page.completion, BrowseCompletionDto::Complete);
    assert_eq!(count, WORKLOAD_ENTRIES);
    assert_eq!(page.known_count, Some(WORKLOAD_ENTRIES));
    metrics::emit_metric(
        "browse_100k_sorted",
        metrics::OBSERVED,
        [
            ("entries".into(), json!(count)),
            ("first_useful_page_ms".into(), json!(first_useful_ms)),
            (
                "total_ms".into(),
                json!(sorted_started.elapsed().as_millis()),
            ),
            ("peak_rss_bytes".into(), json!(peak.rss_bytes)),
            ("global_order_verified".into(), json!(true)),
            (
                "measurement_boundary".into(),
                json!("backend_full_directory_sort_then_pagination"),
            ),
        ],
    );

    assert!(runtime.dispose());
    let settled_process = resources::snapshot();
    let settled = runtime.resource_counts();
    assert_eq!(settled.browse_sessions, 0);
    assert_eq!(settled.browse_service_sessions, 0);
    assert_eq!(settled.browse_entry_refs, 0);
    assert_eq!(settled.browse_path_refs, 0);
    assert_eq!(settled.browse_active_enumerations, 0);
    metrics::emit_metric(
        "browse_100k_teardown",
        metrics::HARD_PASS,
        [
            (
                "browse_sessions".to_string(),
                json!(settled.browse_sessions),
            ),
            (
                "browse_service_sessions".to_string(),
                json!(settled.browse_service_sessions),
            ),
            (
                "live_entry_refs".to_string(),
                json!(settled.browse_entry_refs),
            ),
            (
                "live_path_refs".to_string(),
                json!(settled.browse_path_refs),
            ),
            (
                "active_enumerations".to_string(),
                json!(settled.browse_active_enumerations),
            ),
            (
                "settled_rss_bytes".to_string(),
                json!(settled_process.rss_bytes),
            ),
            (
                "settled_handle_count".to_string(),
                json!(settled_process.handle_count),
            ),
            (
                "settled_fd_count".to_string(),
                json!(settled_process.fd_count),
            ),
        ],
    );
}
