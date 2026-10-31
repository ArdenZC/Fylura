//! Real Apple Silicon Quick Look lifecycle harness.
//!
//! The harness uses the production host, access registry, read gate and
//! retained AppKit view owner. It supplies only bounded PDF fixtures and
//! reports lifecycle metrics; it is not a fake native host.

use super::{view, MacQuickLookPreviewHost};
use crate::file_workspace::{
    contracts::{PreviewHostKind, PreviewSourceRef},
    integration::types::{
        PreviewNativeBounds, PreviewNativePresentation, PreviewSessionStateDto, PreviewSnapshotDto,
    },
    native_preview::access::{
        NativePreviewAccessConfig, NativePreviewAccessRegistry, NativePreviewAccessRequest,
        NativePreviewAccessResolveRequest,
    },
    preview::{
        PreviewCancellation, PreviewCapabilities, PreviewCompleteness, PreviewOperationContext,
        PreviewRepresentation, PreviewRepresentationEnvelope,
    },
    read_gate::{
        MaterializationReadGate, ReadGateConfig, ReadGateSourceResolver, ResolvedContentSource,
        SourceResolutionError,
    },
};
use crate::scheduler::{PermissiveResourcePolicy, SchedulerConfig, WorkScheduler};
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) fn harness_dispatcher() -> view::MainThreadDispatcher {
    Arc::new(|task| {
        task();
        Ok(())
    })
}

struct HarnessSourceResolver {
    path: PathBuf,
}

impl ReadGateSourceResolver for HarnessSourceResolver {
    fn resolve_source(
        &self,
        _source: &PreviewSourceRef,
    ) -> Result<ResolvedContentSource, SourceResolutionError> {
        Ok(ResolvedContentSource::from_backend_path(self.path.clone()))
    }
}

struct HarnessAccess {
    registry: Arc<NativePreviewAccessRegistry>,
    source: PreviewSourceRef,
    source_version: String,
    session_id: String,
    host: PreviewHostKind,
}

impl HarnessAccess {
    fn new(source_path: PathBuf, stage_root: PathBuf, entry_id: &str) -> Result<Self, String> {
        let read_gate = Arc::new(
            MaterializationReadGate::new(
                Arc::new(HarnessSourceResolver { path: source_path }),
                ReadGateConfig::default(),
            )
            .map_err(|error| format!("native_qa_read_gate_{error}"))?,
        );
        let source = PreviewSourceRef::Ephemeral {
            browse_session_id: "native-qa-session".to_string(),
            entry_id: entry_id.to_string(),
        };
        let source_version = read_gate
            .current_source_version(&source)
            .map_err(|error| format!("native_qa_source_version_{error}"))?;
        let scheduler = Arc::new(WorkScheduler::new(
            SchedulerConfig::default().with_policy(Arc::new(PermissiveResourcePolicy)),
        ));
        let registry = NativePreviewAccessRegistry::new(
            stage_root,
            read_gate,
            scheduler,
            NativePreviewAccessConfig::default(),
        )
        .map_err(|error| format!("native_qa_access_{error}"))?;
        Ok(Self {
            registry,
            source,
            source_version,
            session_id: "native-qa-session".to_string(),
            host: PreviewHostKind::ZenFloating,
        })
    }

    fn stage(&self, request_id: &str) -> Result<NativePreviewAccessHandleForHarness, String> {
        let context = PreviewOperationContext::for_backend_content_read(
            &self.session_id,
            request_id,
            &self.source_version,
            PreviewCancellation::default(),
            Instant::now() + Duration::from_secs(5),
        );
        let request = NativePreviewAccessRequest {
            session_id: self.session_id.clone(),
            request_id: request_id.to_string(),
            source: self.source.clone(),
            source_version: self.source_version.clone(),
            host: self.host,
        };
        let handle = self
            .registry
            .stage(request, &context)
            .map_err(|error| format!("native_qa_stage_{error}"))?;
        let bind_request = NativePreviewAccessResolveRequest {
            token: handle.token.clone(),
            session_id: self.session_id.clone(),
            request_id: request_id.to_string(),
            source_version: self.source_version.clone(),
            host: self.host,
        };
        let staged_path = self
            .registry
            .resolve(&bind_request)
            .map_err(|error| format!("native_qa_resolve_{error}"))?;
        Ok(NativePreviewAccessHandleForHarness {
            token: handle.token,
            staged_path,
        })
    }
}

struct NativePreviewAccessHandleForHarness {
    token: String,
    staged_path: PathBuf,
}

struct HarnessHostGuard(MacQuickLookPreviewHost);

impl Drop for HarnessHostGuard {
    fn drop(&mut self) {
        let _ = self.0.dispose();
    }
}

struct HarnessCleanup(PathBuf);

impl Drop for HarnessCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn harness_snapshot(access: &HarnessAccess, request_id: &str, token: &str) -> PreviewSnapshotDto {
    PreviewSnapshotDto {
        preview_id: "native-qa-preview".to_string(),
        session_id: access.session_id.clone(),
        request_id: request_id.to_string(),
        source: access.source.clone(),
        host_kind: access.host,
        state: PreviewSessionStateDto::Ready,
        source_version: Some(access.source_version.clone()),
        representation: Some(PreviewRepresentationEnvelope {
            source_version: access.source_version.clone(),
            representation: PreviewRepresentation::NativeOpaque {
                host: access.host,
                token: token.to_string(),
            },
            completeness: PreviewCompleteness::Complete,
            warnings: Vec::new(),
            capabilities: PreviewCapabilities::all(),
        }),
        effective_capabilities: PreviewCapabilities::all(),
        active_provider_id: Some("native.macos.quick-look".to_string()),
    }
}

fn harness_presentation(
    access: &HarnessAccess,
    token: &str,
    bounds: PreviewNativeBounds,
) -> PreviewNativePresentation {
    PreviewNativePresentation {
        host: access.host,
        token: token.to_string(),
        source_version: access.source_version.clone(),
        bounds,
    }
}

fn write_harness_pdf(path: &Path, label: &str) -> Result<(), String> {
    let escaped_label = label.replace('(', "\\(").replace(')', "\\)");
    let content = format!("BT /F1 18 Tf 20 100 Td ({escaped_label}) Tj ET\n");
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 360 220] /Resources << /Font << /F1 6 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}endstream",
            content.len(),
            content
        )
        .into_bytes(),
        format!("<< /Title (Fylura Quick Look {escaped_label}) >>").into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0usize; objects.len() + 1];
    for (index, object) in objects.iter().enumerate() {
        let object_number = index + 1;
        offsets[object_number] = bytes.len();
        bytes.extend_from_slice(format!("{object_number} 0 obj\n").as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref_offset = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n", offsets.len()).as_bytes());
    bytes.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info 5 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    fs::write(path, bytes).map_err(|error| format!("native_qa_pdf_{error}"))
}

pub(super) fn run_native_preview_lifecycle_harness() -> Result<(), String> {
    if !view::available() {
        return Err("native_qa_quick_look_unavailable".to_string());
    }
    let marker =
        MainThreadMarker::new().ok_or_else(|| "native_qa_main_thread_unavailable".to_string())?;
    let _application = NSApplication::sharedApplication(marker);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| "native_qa_worktree_unavailable".to_string())?
        .join(".tmp-tests")
        .join(format!(
            "macos-native-preview-lifecycle-{}",
            std::process::id()
        ));
    if root.exists() {
        fs::remove_dir_all(&root).map_err(|error| format!("native_qa_root_reset_{error}"))?;
    }
    fs::create_dir_all(&root).map_err(|error| format!("native_qa_root_create_{error}"))?;
    let _cleanup = HarnessCleanup(root.clone());
    let pdf_a = root.join("fixture-a.pdf");
    let pdf_b = root.join("fixture-b.pdf");
    write_harness_pdf(&pdf_a, "fixture A")?;
    write_harness_pdf(&pdf_b, "fixture B")?;
    let access_a = HarnessAccess::new(pdf_a, root.join("staging-a"), "fixture-a")?;
    let access_b = HarnessAccess::new(pdf_b, root.join("staging-b"), "fixture-b")?;
    let parent = NSView::initWithFrame(
        NSView::alloc(marker),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(800.0, 600.0)),
    );
    let parent_ptr = (&*parent as *const NSView) as usize;
    let host = MacQuickLookPreviewHost::new();
    let _host_guard = HarnessHostGuard(host.clone());
    view::reset_native_view_metrics();

    let handle_a = access_a.stage("request-a")?;
    let snapshot_a = harness_snapshot(&access_a, "request-a", &handle_a.token);
    let presentation_a = harness_presentation(
        &access_a,
        &handle_a.token,
        PreviewNativeBounds {
            x: 10,
            y: 20,
            width: 420,
            height: 320,
        },
    );
    let staged_a = handle_a.staged_path.clone();
    host.attach_for_harness(
        parent_ptr,
        Arc::clone(&access_a.registry),
        &snapshot_a,
        &presentation_a,
    )?;
    let view_a = host
        .current_view_id()
        .ok_or_else(|| "native_qa_view_a_missing".to_string())?;
    if !view::native_view_is_attached(view_a) {
        return Err("native_qa_view_a_not_attached".to_string());
    }
    let mut resized_a = presentation_a.clone();
    resized_a.bounds.width += 20;
    resized_a.bounds.height += 20;
    host.attach_for_harness(
        parent_ptr,
        Arc::clone(&access_a.registry),
        &snapshot_a,
        &resized_a,
    )?;
    if host.current_view_id() != Some(view_a) || !view::native_view_is_attached(view_a) {
        return Err("native_qa_geometry_replaced_view".to_string());
    }
    resized_a.bounds.x += 10;
    host.update_geometry_for_harness(
        parent_ptr,
        Arc::clone(&access_a.registry),
        &snapshot_a,
        &resized_a,
    )?;
    resized_a.bounds.y += 10;
    host.update_geometry_for_harness(
        parent_ptr,
        Arc::clone(&access_a.registry),
        &snapshot_a,
        &resized_a,
    )?;

    let handle_b = access_b.stage("request-b")?;
    let snapshot_b = harness_snapshot(&access_b, "request-b", &handle_b.token);
    let presentation_b = harness_presentation(
        &access_b,
        &handle_b.token,
        PreviewNativeBounds {
            x: 30,
            y: 40,
            width: 460,
            height: 340,
        },
    );
    let staged_b = handle_b.staged_path.clone();
    host.attach_for_harness(
        parent_ptr,
        Arc::clone(&access_b.registry),
        &snapshot_b,
        &presentation_b,
    )?;
    if view::native_view_is_attached(view_a) {
        return Err("native_qa_switch_left_old_view_attached".to_string());
    }
    if staged_a.exists() {
        return Err("native_qa_switch_left_old_stage".to_string());
    }
    let view_b = host
        .current_view_id()
        .ok_or_else(|| "native_qa_view_b_missing".to_string())?;
    if view_b == view_a || !view::native_view_is_attached(view_b) {
        return Err("native_qa_view_b_not_attached".to_string());
    }
    let mut resized_b = presentation_b.clone();
    resized_b.bounds.width += 12;
    host.update_geometry_for_harness(
        parent_ptr,
        Arc::clone(&access_b.registry),
        &snapshot_b,
        &resized_b,
    )?;
    resized_b.bounds.height += 12;
    host.update_geometry_for_harness(
        parent_ptr,
        Arc::clone(&access_b.registry),
        &snapshot_b,
        &resized_b,
    )?;
    host.detach("native-qa-preview", Some(&snapshot_b))?;
    if host.current_view_id().is_some()
        || view::native_view_is_attached(view_b)
        || staged_b.exists()
    {
        return Err("native_qa_detach_left_view_or_stage".to_string());
    }
    host.detach("native-qa-preview", Some(&snapshot_b))?;

    for cycle in 0..3 {
        let request_id = format!("steady-{cycle}");
        let handle = access_a.stage(&request_id)?;
        let snapshot = harness_snapshot(&access_a, &request_id, &handle.token);
        let presentation = harness_presentation(
            &access_a,
            &handle.token,
            PreviewNativeBounds {
                x: 5 + cycle * 3,
                y: 8 + cycle * 3,
                width: 300,
                height: 240,
            },
        );
        let staged = handle.staged_path.clone();
        host.attach_for_harness(
            parent_ptr,
            Arc::clone(&access_a.registry),
            &snapshot,
            &presentation,
        )?;
        let view = host
            .current_view_id()
            .ok_or_else(|| "native_qa_steady_view_missing".to_string())?;
        host.update_geometry_for_harness(
            parent_ptr,
            Arc::clone(&access_a.registry),
            &snapshot,
            &presentation,
        )?;
        if !view::native_view_is_attached(view) {
            return Err("native_qa_steady_view_not_attached".to_string());
        }
        host.detach("native-qa-preview", Some(&snapshot))?;
        if view::native_view_is_attached(view) || staged.exists() {
            return Err("native_qa_steady_cleanup_failed".to_string());
        }
    }

    let metrics = view::native_view_metrics();
    if metrics.creations != 5
        || metrics.binds != 5
        || metrics.refreshes != 5
        || metrics.frame_updates < 8
        || metrics.detachments != 5
    {
        return Err(format!("native_qa_metrics_invalid:{metrics:?}"));
    }
    access_a.registry.dispose();
    access_b.registry.dispose();
    Ok(())
}
