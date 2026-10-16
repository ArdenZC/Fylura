use super::{
    fixture::WorkspaceFixture,
    harness::{open_fixture, runtime_for_with_renderer},
    metrics,
    resources::{self, ProcessHeapResources, ProcessResources},
};
use crate::file_workspace::{
    contracts::{BrowseEntryRef, PreviewHostKind, PreviewSourceRef, WorkClass},
    integration::types::{
        BrowseEntryKindDto, BrowsePageDto, BrowseSessionRequest, BrowseStartEnumerationRequest,
        PreviewCreateRequest, PreviewSessionRequest, ThumbnailRequestDto, ThumbnailVariantDto,
    },
    thumbnail::{
        ThumbnailRenderContext, ThumbnailRenderOutput, ThumbnailRenderRequest, ThumbnailRenderer,
        ThumbnailRendererDescriptor, ThumbnailRendererError,
    },
};
use crate::scheduler::ResourceHints;
use serde_json::json;
use std::{ffi::OsStr, sync::Arc, thread, time::Duration};

const EPOCH_COUNT: usize = 5;
const CYCLES_PER_EPOCH: usize = 20;
const PREVIEW_EXECUTION_WARMUP_CYCLES: usize = 1;
const THUMBNAIL_CACHE_WARMUP_ENTRIES: usize = 128;
const WINDOWS_HEAP_DIAGNOSTICS_ENV: &str = "ZEN_CANVAS_W1_11_HEAP_DIAGNOSTICS";

fn heap_diagnostics_enabled_from(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

fn heap_diagnostics_enabled() -> bool {
    heap_diagnostics_enabled_from(std::env::var_os(WINDOWS_HEAP_DIAGNOSTICS_ENV).as_deref())
}

fn heap_snapshot_if_enabled<T>(enabled: bool, capture: impl FnOnce() -> Option<T>) -> Option<T> {
    if enabled {
        capture()
    } else {
        None
    }
}

fn process_heap_snapshot(enabled: bool) -> Option<ProcessHeapResources> {
    heap_snapshot_if_enabled(enabled, resources::process_heap_snapshot)
}

struct PerformanceThumbnailRenderer;

impl ThumbnailRenderer for PerformanceThumbnailRenderer {
    fn descriptor(&self) -> ThumbnailRendererDescriptor {
        ThumbnailRendererDescriptor::new(
            "test.performance",
            "w1-11-performance-v1",
            ResourceHints {
                cpu: 1,
                io: 1,
                open_handles: 0,
                decoder: 0,
                native_preview: 0,
                provider_network: 0,
            },
        )
    }

    fn render(
        &self,
        _request: ThumbnailRenderRequest,
        context: &ThumbnailRenderContext,
    ) -> Result<ThumbnailRenderOutput, ThumbnailRendererError> {
        context.ensure_active()?;
        let _ = context.read_all_bounded(64)?;
        Ok(ThumbnailRenderOutput { bytes: vec![1] })
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct EpochObservation {
    preview_before: ProcessResources,
    preview_before_heaps: Option<ProcessHeapResources>,
    preview_after: ProcessResources,
    preview_after_heaps: Option<ProcessHeapResources>,
    preview_peak: ProcessResources,
    thumbnail_after: ProcessResources,
    thumbnail_after_heaps: Option<ProcessHeapResources>,
    thumbnail_peak: ProcessResources,
    target_switch_before: ProcessResources,
    target_switch_before_heaps: Option<ProcessHeapResources>,
    target_switch_after: ProcessResources,
    target_switch_after_heaps: Option<ProcessHeapResources>,
    target_switch_peak: ProcessResources,
    settled: ProcessResources,
    settled_heaps: Option<ProcessHeapResources>,
}

fn enumerate_fixture(
    runtime: &crate::file_workspace::integration::FileWorkspaceRuntime,
    session_id: &str,
    path_ref: &crate::file_workspace::BrowsePathRef,
    request_id: &str,
) -> Vec<BrowsePageDto> {
    let first = runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: session_id.to_string(),
            request_id: request_id.to_string(),
            path_ref: path_ref.clone(),
            page_size: 256,
            query: Default::default(),
        })
        .expect("10k Browse first page");
    assert!(!first.entries.is_empty());
    let mut pages = vec![first];
    while let Some(cursor) = pages.last().and_then(|page| page.next_cursor.clone()) {
        pages.push(
            runtime
                .next_page(
                    crate::file_workspace::integration::types::BrowseNextPageRequest {
                        session_id: session_id.to_string(),
                        cursor,
                        page_size: 256,
                    },
                )
                .expect("10k Browse next page"),
        );
    }
    assert_eq!(
        pages.iter().map(|page| page.entries.len()).sum::<usize>(),
        10_000
    );
    pages
}

fn warm_thumbnail_cache(
    runtime: &crate::file_workspace::integration::FileWorkspaceRuntime,
    fixture: &WorkspaceFixture,
) -> usize {
    let opened = open_fixture(runtime, fixture, "resource-thumbnail-cache-warmup");
    let pages = enumerate_fixture(
        runtime,
        &opened.session_id,
        &opened.root_path_ref,
        "resource-thumbnail-cache-warmup-browse",
    );
    let mut warmed = 0;
    for (index, entry) in pages
        .iter()
        .flat_map(|page| page.entries.iter())
        .filter(|entry| matches!(entry.kind, BrowseEntryKindDto::File))
        .take(THUMBNAIL_CACHE_WARMUP_ENTRIES)
        .enumerate()
    {
        runtime
            .request_thumbnail(ThumbnailRequestDto {
                request_id: format!("resource-thumbnail-cache-warmup-{index}"),
                source: entry.entry_ref.clone().into(),
                variant: ThumbnailVariantDto::Small,
                work_class: WorkClass::Foreground,
                session_id: Some(opened.session_id.clone()),
            })
            .expect("thumbnail cache warmup cycle");
        warmed += 1;
    }
    assert_eq!(warmed, THUMBNAIL_CACHE_WARMUP_ENTRIES);
    runtime
        .dispose_browse(BrowseSessionRequest {
            session_id: opened.session_id,
        })
        .expect("dispose thumbnail cache warmup target");
    let counts = runtime.resource_counts();
    assert_eq!(counts.browse_sessions, 0);
    assert_eq!(counts.browse_service_sessions, 0);
    assert_eq!(counts.browse_entry_refs, 0);
    assert_eq!(counts.browse_path_refs, 0);
    assert_eq!(counts.thumbnail_requests, 0);
    runtime.inner.thumbnail.memory_cache_len()
}

fn warm_preview_execution(
    runtime: &crate::file_workspace::integration::FileWorkspaceRuntime,
    fixture: &WorkspaceFixture,
) {
    let opened = open_fixture(runtime, fixture, "resource-preview-execution-warmup");
    let pages = enumerate_fixture(
        runtime,
        &opened.session_id,
        &opened.root_path_ref,
        "resource-preview-execution-warmup-browse",
    );
    let entry = pages
        .first()
        .and_then(|page| page.entries.first())
        .expect("preview execution warmup entry");
    let source = match &entry.entry_ref {
        BrowseEntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        } => PreviewSourceRef::Ephemeral {
            browse_session_id: browse_session_id.clone(),
            entry_id: entry_id.clone(),
        },
    };
    let preview = runtime
        .create_preview(PreviewCreateRequest {
            request_id: "resource-preview-execution-warmup".to_string(),
            source,
            host_kind: PreviewHostKind::ZenFloating,
        })
        .expect("create preview execution warmup");
    let started = runtime
        .start_preview(PreviewSessionRequest {
            preview_id: preview.preview_id.clone(),
            native_presentation: None,
        })
        .expect("start preview execution warmup");
    assert_eq!(
        started.state,
        super::super::types::PreviewSessionStateDto::Ready
    );
    runtime
        .dispose_preview(PreviewSessionRequest {
            preview_id: preview.preview_id,
            native_presentation: None,
        })
        .expect("dispose preview execution warmup");
    runtime
        .dispose_browse(BrowseSessionRequest {
            session_id: opened.session_id,
        })
        .expect("dispose preview execution warmup target");
    let counts = runtime.resource_counts();
    assert_eq!(counts.browse_sessions, 0);
    assert_eq!(counts.browse_service_sessions, 0);
    assert_eq!(counts.browse_entry_refs, 0);
    assert_eq!(counts.browse_path_refs, 0);
    assert_eq!(counts.preview_sessions, 0);
}

fn run_epoch(
    runtime: &crate::file_workspace::integration::FileWorkspaceRuntime,
    fixture: &WorkspaceFixture,
    epoch: usize,
    heap_diagnostics_enabled: bool,
) -> EpochObservation {
    let opened = open_fixture(runtime, fixture, &format!("resource-10k-epoch-{epoch}"));
    let pages = enumerate_fixture(
        runtime,
        &opened.session_id,
        &opened.root_path_ref,
        &format!("resource-browse-epoch-{epoch}"),
    );
    let preview_entry = pages
        .first()
        .and_then(|page| page.entries.first())
        .expect("10k fixture entry");
    let source = match &preview_entry.entry_ref {
        BrowseEntryRef::Ephemeral {
            browse_session_id,
            entry_id,
        } => PreviewSourceRef::Ephemeral {
            browse_session_id: browse_session_id.clone(),
            entry_id: entry_id.clone(),
        },
    };
    let preview_before = resources::snapshot();
    let preview_before_heaps = process_heap_snapshot(heap_diagnostics_enabled);
    let mut preview_peak = preview_before;
    for index in 0..CYCLES_PER_EPOCH {
        let preview = runtime
            .create_preview(PreviewCreateRequest {
                request_id: format!("resource-preview-{epoch}-{index}"),
                source: source.clone(),
                host_kind: PreviewHostKind::ZenFloating,
            })
            .expect("create preview cycle");
        let started = runtime
            .start_preview(PreviewSessionRequest {
                preview_id: preview.preview_id.clone(),
                native_presentation: None,
            })
            .expect("start preview cycle");
        assert_eq!(
            started.state,
            super::super::types::PreviewSessionStateDto::Ready
        );
        runtime
            .dispose_preview(PreviewSessionRequest {
                preview_id: preview.preview_id,
                native_presentation: None,
            })
            .expect("dispose preview cycle");
        preview_peak = preview_peak.max(resources::snapshot());
        assert_eq!(runtime.resource_counts().preview_sessions, 0);
    }
    let preview_after = resources::snapshot();
    let preview_after_heaps = process_heap_snapshot(heap_diagnostics_enabled);

    let mut thumbnail_peak = preview_after;
    for index in 0..CYCLES_PER_EPOCH {
        let thumbnail_entry = pages
            .iter()
            .flat_map(|page| page.entries.iter())
            .filter(|entry| matches!(entry.kind, BrowseEntryKindDto::File))
            .nth(index)
            .expect("distinct thumbnail fixture entry");
        let artifact = runtime
            .request_thumbnail(ThumbnailRequestDto {
                request_id: format!("resource-thumbnail-{epoch}-{index}"),
                source: thumbnail_entry.entry_ref.clone().into(),
                variant: ThumbnailVariantDto::Small,
                work_class: WorkClass::Foreground,
                session_id: Some(opened.session_id.clone()),
            })
            .expect("test renderer thumbnail cycle");
        assert!(!artifact.bytes.is_empty());
        thumbnail_peak = thumbnail_peak.max(resources::snapshot());
        assert_eq!(runtime.resource_counts().thumbnail_requests, 0);
    }
    let thumbnail_after = resources::snapshot();
    let thumbnail_after_heaps = process_heap_snapshot(heap_diagnostics_enabled);

    runtime
        .dispose_browse(BrowseSessionRequest {
            session_id: opened.session_id,
        })
        .expect("dispose 10k Browse target");
    let counts = runtime.resource_counts();
    assert_eq!(counts.browse_sessions, 0);
    assert_eq!(counts.browse_service_sessions, 0);
    assert_eq!(counts.browse_entry_refs, 0);
    assert_eq!(counts.browse_path_refs, 0);

    let target_switch_before = resources::snapshot();
    let target_switch_before_heaps = process_heap_snapshot(heap_diagnostics_enabled);
    let mut target_switch_peak = target_switch_before;
    for index in 0..CYCLES_PER_EPOCH {
        let switched = open_fixture(
            runtime,
            fixture,
            &format!("resource-switch-{epoch}-{index}"),
        );
        runtime
            .start_enumeration(BrowseStartEnumerationRequest {
                session_id: switched.session_id.clone(),
                request_id: format!("resource-switch-enumeration-{epoch}-{index}"),
                path_ref: switched.root_path_ref,
                page_size: 64,
                query: Default::default(),
            })
            .expect("target-switch Browse page");
        runtime
            .dispose_browse(BrowseSessionRequest {
                session_id: switched.session_id,
            })
            .expect("dispose target switch");
        let counts = runtime.resource_counts();
        assert_eq!(counts.browse_sessions, 0);
        assert_eq!(counts.browse_service_sessions, 0);
        assert_eq!(counts.browse_entry_refs, 0);
        assert_eq!(counts.browse_path_refs, 0);
        target_switch_peak = target_switch_peak.max(resources::snapshot());
    }
    let target_switch_after = resources::snapshot();
    let target_switch_after_heaps = process_heap_snapshot(heap_diagnostics_enabled);

    EpochObservation {
        preview_before,
        preview_before_heaps,
        preview_after,
        preview_after_heaps,
        preview_peak,
        thumbnail_after,
        thumbnail_after_heaps,
        thumbnail_peak,
        target_switch_before,
        target_switch_before_heaps,
        target_switch_after,
        target_switch_after_heaps,
        target_switch_peak,
        settled: ProcessResources::default(),
        settled_heaps: None,
    }
}

fn sustained_growth<F>(samples: &[ProcessResources], select: F) -> bool
where
    F: Fn(ProcessResources) -> Option<u64>,
{
    let Some(values) = samples
        .iter()
        .copied()
        .map(select)
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    strictly_increasing(&values)
}

fn strictly_increasing(values: &[u64]) -> bool {
    values.len() >= 3 && values.windows(2).all(|pair| pair[1] > pair[0])
}

fn optional_series<F>(samples: &[ProcessResources], select: F) -> Vec<Option<u64>>
where
    F: Fn(ProcessResources) -> Option<u64>,
{
    samples.iter().copied().map(select).collect()
}

#[test]
fn windows_heap_diagnostics_are_explicit_and_default_capture_is_inert() {
    use std::cell::Cell;

    assert!(!heap_diagnostics_enabled_from(None));
    assert!(!heap_diagnostics_enabled_from(Some(OsStr::new("0"))));
    assert!(!heap_diagnostics_enabled_from(Some(OsStr::new("true"))));
    assert!(heap_diagnostics_enabled_from(Some(OsStr::new("1"))));

    let capture_calls = Cell::new(0);
    let disabled_sample = heap_snapshot_if_enabled(false, || {
        capture_calls.set(capture_calls.get() + 1);
        Some(42)
    });
    assert_eq!(disabled_sample, None);
    assert_eq!(capture_calls.get(), 0);

    let enabled_sample = heap_snapshot_if_enabled(true, || {
        capture_calls.set(capture_calls.get() + 1);
        Some(42)
    });
    assert_eq!(enabled_sample, Some(42));
    assert_eq!(capture_calls.get(), 1);
}

#[test]
#[ignore = "W1-11 Windows PrivateUsage detector correctness evidence"]
fn windows_private_usage_detector_catches_sustained_retention() {
    #[cfg(target_os = "windows")]
    {
        const SELF_TEST_EPOCHS: usize = 4;
        const RETAINED_BLOCK_BYTES: usize = 4 * 1024 * 1024;
        const PAGE_TOUCH_STRIDE: usize = 4096;

        let mut retained: Vec<Vec<u8>> = Vec::with_capacity(SELF_TEST_EPOCHS);
        let mut private_samples = Vec::with_capacity(SELF_TEST_EPOCHS);
        for epoch in 0..SELF_TEST_EPOCHS {
            for block in &mut retained {
                touch_retained_memory(block, epoch as u8, PAGE_TOUCH_STRIDE);
            }
            let mut block = vec![0u8; RETAINED_BLOCK_BYTES];
            touch_retained_memory(&mut block, epoch as u8, PAGE_TOUCH_STRIDE);
            retained.push(block);
            std::hint::black_box(&retained);

            // This intentionally trims the working set before sampling. The
            // hard detector must still see the retained committed pages.
            resources::settle_allocator();
            private_samples.push(
                resources::snapshot()
                    .private_committed_bytes
                    .expect("Windows PrivateUsage sampler is available"),
            );
        }
        let sustained_growth_detected = strictly_increasing(&private_samples);
        metrics::emit_metric(
            "windows_private_usage_detector_self_test",
            if sustained_growth_detected {
                metrics::HARD_PASS
            } else {
                metrics::BLOCKED
            },
            [
                (
                    "metric".to_string(),
                    json!("PROCESS_MEMORY_COUNTERS_EX::PrivateUsage"),
                ),
                (
                    "retained_block_bytes".to_string(),
                    json!(RETAINED_BLOCK_BYTES),
                ),
                ("epoch_count".to_string(), json!(SELF_TEST_EPOCHS)),
                (
                    "private_committed_samples".to_string(),
                    json!(private_samples),
                ),
                (
                    "sustained_growth_detected".to_string(),
                    json!(sustained_growth_detected),
                ),
                ("working_set_trim_applied".to_string(), json!(true)),
                (
                    "virtual_address_space_metric_used".to_string(),
                    json!(false),
                ),
            ],
        );
        assert!(
            sustained_growth_detected,
            "PrivateUsage did not detect intentionally retained committed memory: {private_samples:?}"
        );
        drop(retained);
        resources::settle_allocator();
    }

    #[cfg(not(target_os = "windows"))]
    metrics::emit_metric(
        "windows_private_usage_detector_self_test",
        "UNVERIFIED",
        [
            (
                "reason".to_string(),
                json!("PROCESS_MEMORY_COUNTERS_EX::PrivateUsage is Windows-only"),
            ),
            ("working_set_trim_applied".to_string(), json!(false)),
        ],
    );
}

#[cfg(target_os = "windows")]
fn touch_retained_memory(block: &mut [u8], value: u8, stride: usize) {
    for byte in block.iter_mut().step_by(stride) {
        *byte = value;
    }
    if let Some(last) = block.last_mut() {
        *last = value;
    }
}

#[test]
#[ignore = "W1-11 resource and lifecycle steady-state observations"]
fn resource_and_registry_steady_state_after_browse_preview_switches() {
    let heap_diagnostics_enabled = heap_diagnostics_enabled();
    let fixture = WorkspaceFixture::large("resource-10k", 9_000, 1_000);
    let runtime = runtime_for_with_renderer(&fixture, Arc::new(PerformanceThumbnailRenderer));
    // Warm the runtime/session before collecting the idle baseline. Fixture
    // construction and cache/database setup are not workload measurements.
    let warm = open_fixture(&runtime, &fixture, "resource-warmup");
    runtime
        .start_enumeration(BrowseStartEnumerationRequest {
            session_id: warm.session_id.clone(),
            request_id: "resource-warmup-browse".to_string(),
            path_ref: warm.root_path_ref,
            page_size: 64,
            query: Default::default(),
        })
        .expect("warm Browse page");
    runtime
        .dispose_browse(BrowseSessionRequest {
            session_id: warm.session_id,
        })
        .expect("dispose warm Browse target");
    // The shared bounded Preview executor is created lazily by the first
    // session. Warm it before the idle baseline so worker/thread-stack setup
    // is not misclassified as epoch growth. Measured epochs still perform the
    // full 100 Preview cycles below and retain strict sustained-growth failure.
    warm_preview_execution(&runtime, &fixture);
    // The production ThumbnailService memory cache is intentionally bounded
    // at 128 entries. Fill that existing cache with the real renderer and
    // Read Gate before measured epochs so allowed cache warm-up is not
    // misclassified as a leak; post-warmup monotonic growth still fails.
    let warmed_thumbnail_cache_entries = warm_thumbnail_cache(&runtime, &fixture);
    resources::settle_allocator();
    let idle_process = resources::snapshot();
    let idle_process_heaps = process_heap_snapshot(heap_diagnostics_enabled);

    let mut epochs = Vec::with_capacity(EPOCH_COUNT);
    for epoch in 0..EPOCH_COUNT {
        let mut observation = run_epoch(&runtime, &fixture, epoch, heap_diagnostics_enabled);
        // Every measured epoch is a bounded workload followed immediately by
        // its own settle/sample boundary. This prevents a final plateau from
        // hiding growth that occurred during an earlier epoch.
        thread::sleep(Duration::from_millis(250));
        resources::settle_allocator();
        observation.settled = resources::snapshot();
        observation.settled_heaps = process_heap_snapshot(heap_diagnostics_enabled);
        let counts = runtime.resource_counts();
        assert_eq!(counts.browse_sessions, 0);
        assert_eq!(counts.browse_service_sessions, 0);
        assert_eq!(counts.browse_entry_refs, 0);
        assert_eq!(counts.browse_path_refs, 0);
        assert_eq!(counts.browse_active_enumerations, 0);
        epochs.push(observation);
    }

    if cfg!(target_os = "macos") {
        assert!(
            epochs.iter().all(|epoch| epoch.settled.fd_count.is_some()),
            "macOS Workspace Foundation evidence requires an observable FD count"
        );
    }

    assert!(runtime.dispose());
    let after_runtime_dispose_process = resources::snapshot();
    let after_runtime_dispose_heaps = process_heap_snapshot(heap_diagnostics_enabled);
    let settled = runtime.resource_counts();
    assert_eq!(settled.browse_sessions, 0);
    assert_eq!(settled.browse_service_sessions, 0);
    assert_eq!(settled.browse_entry_refs, 0);
    assert_eq!(settled.browse_path_refs, 0);
    assert_eq!(settled.browse_active_enumerations, 0);
    assert_eq!(settled.change_monitors, 0);
    assert_eq!(settled.preview_sessions, 0);
    assert_eq!(settled.thumbnail_requests, 0);

    let settled_samples = epochs.iter().map(|epoch| epoch.settled).collect::<Vec<_>>();
    let rss_sustained_growth = sustained_growth(&settled_samples, |sample| sample.rss_bytes);
    let private_committed_sustained_growth =
        sustained_growth(&settled_samples, |sample| sample.private_committed_bytes);
    let handle_sustained_growth = sustained_growth(&settled_samples, |sample| sample.handle_count);
    let fd_sustained_growth = sustained_growth(&settled_samples, |sample| sample.fd_count);
    let hard_memory_metric_available = if cfg!(target_os = "windows") {
        settled_samples
            .iter()
            .all(|sample| sample.private_committed_bytes.is_some())
    } else {
        settled_samples
            .iter()
            .all(|sample| sample.rss_bytes.is_some())
    };
    let hard_handle_metric_available = if cfg!(target_os = "windows") {
        settled_samples
            .iter()
            .all(|sample| sample.handle_count.is_some())
    } else {
        true
    };
    let hard_resource_growth = if cfg!(target_os = "windows") {
        private_committed_sustained_growth || handle_sustained_growth
    } else {
        rss_sustained_growth || handle_sustained_growth || fd_sustained_growth
    };
    let hard_resource_signal_available =
        hard_memory_metric_available && hard_handle_metric_available;
    let sustained_resource_growth = hard_resource_growth;
    let rss_measurement_classification = if cfg!(target_os = "windows") {
        "OBSERVED: trimmed-working-set diagnostic; SetProcessWorkingSetSize precedes settled samples"
    } else {
        "OBSERVED: native resident RSS sample"
    };
    let hard_memory_metric = if cfg!(target_os = "windows") {
        "PROCESS_MEMORY_COUNTERS_EX::PrivateUsage"
    } else {
        "native resident RSS"
    };
    let final_settled = *settled_samples.last().expect("settled epoch sample");
    let settled_resource_sample_stable = settled_samples.windows(2).all(|samples| {
        samples[0].rss_bytes == samples[1].rss_bytes
            && samples[0].handle_count == samples[1].handle_count
            && samples[0].fd_count == samples[1].fd_count
    });
    let preview_peak = epochs
        .iter()
        .fold(idle_process, |peak, epoch| peak.max(epoch.preview_peak));
    let thumbnail_peak = epochs
        .iter()
        .fold(idle_process, |peak, epoch| peak.max(epoch.thumbnail_peak));
    let target_switch_peak = epochs.iter().fold(idle_process, |peak, epoch| {
        peak.max(epoch.target_switch_peak)
    });
    let preview_before_samples = epochs
        .iter()
        .map(|epoch| epoch.preview_before)
        .collect::<Vec<_>>();
    let preview_after_samples = epochs
        .iter()
        .map(|epoch| epoch.preview_after)
        .collect::<Vec<_>>();
    let target_switch_before_samples = epochs
        .iter()
        .map(|epoch| epoch.target_switch_before)
        .collect::<Vec<_>>();
    let target_switch_after_samples = epochs
        .iter()
        .map(|epoch| epoch.target_switch_after)
        .collect::<Vec<_>>();
    let thumbnail_after_samples = epochs
        .iter()
        .map(|epoch| epoch.thumbnail_after)
        .collect::<Vec<_>>();
    let preview_before_heap_samples = epochs
        .iter()
        .map(|epoch| epoch.preview_before_heaps)
        .collect::<Vec<_>>();
    let preview_after_heap_samples = epochs
        .iter()
        .map(|epoch| epoch.preview_after_heaps)
        .collect::<Vec<_>>();
    let thumbnail_after_heap_samples = epochs
        .iter()
        .map(|epoch| epoch.thumbnail_after_heaps)
        .collect::<Vec<_>>();
    let target_switch_before_heap_samples = epochs
        .iter()
        .map(|epoch| epoch.target_switch_before_heaps)
        .collect::<Vec<_>>();
    let target_switch_after_heap_samples = epochs
        .iter()
        .map(|epoch| epoch.target_switch_after_heaps)
        .collect::<Vec<_>>();
    let settled_heap_samples = epochs
        .iter()
        .map(|epoch| epoch.settled_heaps)
        .collect::<Vec<_>>();

    if cfg!(target_os = "windows") && heap_diagnostics_enabled {
        for sample in std::iter::once(idle_process_heaps)
            .chain(preview_before_heap_samples.iter().copied())
            .chain(preview_after_heap_samples.iter().copied())
            .chain(thumbnail_after_heap_samples.iter().copied())
            .chain(target_switch_before_heap_samples.iter().copied())
            .chain(target_switch_after_heap_samples.iter().copied())
            .chain(settled_heap_samples.iter().copied())
            .chain(std::iter::once(after_runtime_dispose_heaps))
        {
            let sample = sample.expect("enabled Windows heap diagnostic must return a sample");
            assert!(
                sample.available,
                "Windows heap diagnostic unavailable: {sample:?}"
            );
            assert!(
                sample.heap_count > 0,
                "Windows heap diagnostic found no heaps: {sample:?}"
            );
            assert!(
                sample.heap_region_count > 0,
                "Windows heap diagnostic found no committed-region samples: {sample:?}"
            );
            assert_eq!(
                sample.heaps_walked, sample.heap_count,
                "Windows heap diagnostic did not walk every heap: {sample:?}"
            );
            assert_eq!(
                sample.error_code, None,
                "Windows heap diagnostic API error: {sample:?}"
            );
        }
    }

    metrics::emit_metric(
        "resource_observations",
        metrics::OBSERVED,
        [
            ("epoch_count".to_string(), json!(EPOCH_COUNT)),
            (
                "windows_heap_diagnostics_enabled".to_string(),
                json!(heap_diagnostics_enabled),
            ),
            (
                "thumbnail_cache_warmup_entries".to_string(),
                json!(THUMBNAIL_CACHE_WARMUP_ENTRIES),
            ),
            (
                "thumbnail_cache_entries_after_warmup".to_string(),
                json!(warmed_thumbnail_cache_entries),
            ),
            (
                "preview_cycles_total".to_string(),
                json!(EPOCH_COUNT * CYCLES_PER_EPOCH),
            ),
            (
                "preview_execution_warmup_cycles".to_string(),
                json!(PREVIEW_EXECUTION_WARMUP_CYCLES),
            ),
            (
                "thumbnail_cycles_total".to_string(),
                json!(EPOCH_COUNT * CYCLES_PER_EPOCH),
            ),
            (
                "target_switch_cycles_total".to_string(),
                json!(EPOCH_COUNT * CYCLES_PER_EPOCH),
            ),
            ("idle_rss_bytes".to_string(), json!(idle_process.rss_bytes)),
            (
                "windows_process_heap_idle".to_string(),
                json!(idle_process_heaps),
            ),
            (
                "windows_process_heap_preview_before_samples".to_string(),
                json!(preview_before_heap_samples),
            ),
            (
                "windows_process_heap_preview_after_samples".to_string(),
                json!(preview_after_heap_samples),
            ),
            (
                "windows_process_heap_thumbnail_after_samples".to_string(),
                json!(thumbnail_after_heap_samples),
            ),
            (
                "windows_process_heap_target_switch_before_samples".to_string(),
                json!(target_switch_before_heap_samples),
            ),
            (
                "windows_process_heap_target_switch_after_samples".to_string(),
                json!(target_switch_after_heap_samples),
            ),
            (
                "windows_process_heap_settled_samples".to_string(),
                json!(settled_heap_samples),
            ),
            (
                "windows_process_heap_after_runtime_dispose".to_string(),
                json!(after_runtime_dispose_heaps),
            ),
            (
                "process_private_committed_after_runtime_dispose_bytes".to_string(),
                json!(after_runtime_dispose_process.private_committed_bytes),
            ),
            (
                "rss_measurement_classification".to_string(),
                json!(rss_measurement_classification),
            ),
            ("hard_memory_metric".to_string(), json!(hard_memory_metric)),
            (
                "idle_private_committed_bytes".to_string(),
                json!(idle_process.private_committed_bytes),
            ),
            (
                "preview_100_cycle_peak_rss_bytes".to_string(),
                json!(preview_peak.rss_bytes),
            ),
            (
                "preview_100_cycle_peak_private_committed_bytes".to_string(),
                json!(preview_peak.private_committed_bytes),
            ),
            (
                "thumbnail_100_cycle_peak_rss_bytes".to_string(),
                json!(thumbnail_peak.rss_bytes),
            ),
            (
                "thumbnail_100_cycle_peak_private_committed_bytes".to_string(),
                json!(thumbnail_peak.private_committed_bytes),
            ),
            (
                "target_switch_100_cycle_peak_rss_bytes".to_string(),
                json!(target_switch_peak.rss_bytes),
            ),
            (
                "target_switch_100_cycle_peak_private_committed_bytes".to_string(),
                json!(target_switch_peak.private_committed_bytes),
            ),
            (
                "settled_rss_bytes".to_string(),
                json!(final_settled.rss_bytes),
            ),
            (
                "settled_private_committed_bytes".to_string(),
                json!(final_settled.private_committed_bytes),
            ),
            (
                "idle_handle_count".to_string(),
                json!(idle_process.handle_count),
            ),
            (
                "preview_100_cycle_peak_handle_count".to_string(),
                json!(preview_peak.handle_count),
            ),
            (
                "thumbnail_100_cycle_peak_handle_count".to_string(),
                json!(thumbnail_peak.handle_count),
            ),
            (
                "target_switch_100_cycle_peak_handle_count".to_string(),
                json!(target_switch_peak.handle_count),
            ),
            (
                "settled_handle_count".to_string(),
                json!(final_settled.handle_count),
            ),
            ("idle_fd_count".to_string(), json!(idle_process.fd_count)),
            (
                "preview_100_cycle_peak_fd_count".to_string(),
                json!(preview_peak.fd_count),
            ),
            (
                "thumbnail_100_cycle_peak_fd_count".to_string(),
                json!(thumbnail_peak.fd_count),
            ),
            (
                "target_switch_100_cycle_peak_fd_count".to_string(),
                json!(target_switch_peak.fd_count),
            ),
            (
                "settled_fd_count".to_string(),
                json!(final_settled.fd_count),
            ),
            (
                "settled_resource_sample_stable".to_string(),
                json!(settled_resource_sample_stable),
            ),
            (
                "preview_before_rss_bytes".to_string(),
                json!(optional_series(&preview_before_samples, |sample| sample.rss_bytes)),
            ),
            (
                "preview_before_private_committed_bytes".to_string(),
                json!(optional_series(&preview_before_samples, |sample| {
                    sample.private_committed_bytes
                })),
            ),
            (
                "preview_after_rss_bytes".to_string(),
                json!(optional_series(&preview_after_samples, |sample| sample.rss_bytes)),
            ),
            (
                "preview_after_private_committed_bytes".to_string(),
                json!(optional_series(&preview_after_samples, |sample| {
                    sample.private_committed_bytes
                })),
            ),
            (
                "thumbnail_after_rss_bytes".to_string(),
                json!(optional_series(&thumbnail_after_samples, |sample| sample.rss_bytes)),
            ),
            (
                "thumbnail_after_private_committed_bytes".to_string(),
                json!(optional_series(&thumbnail_after_samples, |sample| {
                    sample.private_committed_bytes
                })),
            ),
            (
                "target_switch_before_rss_bytes".to_string(),
                json!(optional_series(&target_switch_before_samples, |sample| {
                    sample.rss_bytes
                })),
            ),
            (
                "target_switch_before_private_committed_bytes".to_string(),
                json!(optional_series(&target_switch_before_samples, |sample| {
                    sample.private_committed_bytes
                })),
            ),
            (
                "target_switch_after_rss_bytes".to_string(),
                json!(optional_series(&target_switch_after_samples, |sample| {
                    sample.rss_bytes
                })),
            ),
            (
                "target_switch_after_private_committed_bytes".to_string(),
                json!(optional_series(&target_switch_after_samples, |sample| {
                    sample.private_committed_bytes
                })),
            ),
            (
                "settled_rss_samples".to_string(),
                json!(optional_series(&settled_samples, |sample| sample.rss_bytes)),
            ),
            (
                "settled_private_committed_samples".to_string(),
                json!(optional_series(&settled_samples, |sample| {
                    sample.private_committed_bytes
                })),
            ),
            (
                "settled_handle_samples".to_string(),
                json!(optional_series(&settled_samples, |sample| sample.handle_count)),
            ),
            (
                "settled_fd_samples".to_string(),
                json!(optional_series(&settled_samples, |sample| sample.fd_count)),
            ),
            (
                "thumbnail_cache_entries_final".to_string(),
                json!(runtime.inner.thumbnail.memory_cache_len()),
            ),
            ("thumbnail_renderer".to_string(), json!("test.performance")),
            ("native_thumbnail".to_string(), json!(false)),
            ("fixture_root_scope".to_string(), json!("repository-local")),
        ],
    );
    metrics::emit_metric(
        "resource_epoch_trend",
        if !hard_resource_signal_available || sustained_resource_growth {
            metrics::BLOCKED
        } else {
            metrics::HARD_PASS
        },
        [
            ("epoch_count".to_string(), json!(EPOCH_COUNT)),
            (
                "rss_sustained_growth".to_string(),
                json!(rss_sustained_growth),
            ),
            (
                "private_committed_sustained_growth".to_string(),
                json!(private_committed_sustained_growth),
            ),
            (
                "private_committed_metric_available".to_string(),
                json!(settled_samples
                    .iter()
                    .all(|sample| sample.private_committed_bytes.is_some())),
            ),
            (
                "handle_sustained_growth".to_string(),
                json!(handle_sustained_growth),
            ),
            (
                "handle_metric_available".to_string(),
                json!(settled_samples
                    .iter()
                    .all(|sample| sample.handle_count.is_some())),
            ),
            (
                "fd_sustained_growth".to_string(),
                json!(fd_sustained_growth),
            ),
            (
                "hard_memory_metric".to_string(),
                json!(hard_memory_metric),
            ),
            (
                "rss_measurement_classification".to_string(),
                json!(rss_measurement_classification),
            ),
            (
                "rss_is_hard_signal".to_string(),
                json!(!cfg!(target_os = "windows")),
            ),
            (
                "hard_resource_signal_available".to_string(),
                json!(hard_resource_signal_available),
            ),
            (
                "hard_resource_growth".to_string(),
                json!(hard_resource_growth),
            ),
            (
                "sustained_resource_growth".to_string(),
                json!(sustained_resource_growth),
            ),
            (
                "rule".to_string(),
                json!("Windows: strict PrivateUsage or handle increase across every settled epoch transition; RSS is diagnostic only. macOS retains resident RSS/FD rule."),
            ),
            ("allocator_cache_retention_allowed".to_string(), json!(true)),
        ],
    );
    metrics::emit_metric(
        "resource_registry_steady_state",
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
                "browse_entry_refs".to_string(),
                json!(settled.browse_entry_refs),
            ),
            (
                "browse_path_refs".to_string(),
                json!(settled.browse_path_refs),
            ),
            (
                "change_monitors".to_string(),
                json!(settled.change_monitors),
            ),
            (
                "preview_sessions".to_string(),
                json!(settled.preview_sessions),
            ),
            (
                "thumbnail_requests".to_string(),
                json!(settled.thumbnail_requests),
            ),
            (
                "thumbnail_cache_entries".to_string(),
                json!(runtime.inner.thumbnail.memory_cache_len()),
            ),
            (
                "thumbnail_cache_capacity_bound".to_string(),
                json!(THUMBNAIL_CACHE_WARMUP_ENTRIES),
            ),
            (
                "preview_cycles".to_string(),
                json!(EPOCH_COUNT * CYCLES_PER_EPOCH),
            ),
            (
                "thumbnail_cycles".to_string(),
                json!(EPOCH_COUNT * CYCLES_PER_EPOCH),
            ),
            (
                "target_switch_cycles".to_string(),
                json!(EPOCH_COUNT * CYCLES_PER_EPOCH),
            ),
            ("epoch_count".to_string(), json!(EPOCH_COUNT)),
            ("os_resource_trend_separate".to_string(), json!(true)),
            ("internal_registry_zero".to_string(), json!(true)),
        ],
    );
    assert!(
        hard_resource_signal_available,
        "required Windows hard resource metrics were unavailable"
    );
    assert!(
        !sustained_resource_growth,
        "process resources showed sustained per-epoch growth"
    );
}
