use super::access_test_support::{
    assert_no_stage_roots, context, setup, setup_with_config, CancelingWriter,
};
use super::*;
use crate::file_workspace::{
    contracts::ContentReadEligibility,
    preview::{PreviewCancellation, PreviewOperationContext},
    read_gate::{VerifiedCopyBounds, VerifiedCopyError},
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

#[test]
fn wrong_host_and_host_provided_input_fail_closed() {
    let (_fixture, _gate, registry, source, source_version, _resolver) = setup(b"bytes");
    let operation = context(&source_version);
    assert_eq!(
        registry.stage(
            NativePreviewAccessRequest {
                session_id: "session-1".to_string(),
                request_id: "request-1".to_string(),
                source: source.clone(),
                source_version: source_version.clone(),
                host: PreviewHostKind::WindowsPreviewHandler,
            },
            &operation,
        ),
        Err(NativePreviewAccessError::UnsupportedHost)
    );
    assert_eq!(
        registry.stage(
            NativePreviewAccessRequest {
                session_id: "session-1".to_string(),
                request_id: "request-1".to_string(),
                source: PreviewSourceRef::HostProvided {
                    host_token: "host-1".to_string(),
                },
                source_version,
                host: PreviewHostKind::ZenFloating,
            },
            &operation,
        ),
        Err(NativePreviewAccessError::UnsupportedSource)
    );
}

#[test]
fn stage_maps_read_gate_terminal_eligibility_without_allocating_staging() {
    for (eligibility, expected_error) in [
        (
            ContentReadEligibility::MaterializationRequired,
            NativePreviewAccessError::MaterializationRequired,
        ),
        (
            ContentReadEligibility::Downloading,
            NativePreviewAccessError::MaterializationRequired,
        ),
        (
            ContentReadEligibility::MetadataOnly,
            NativePreviewAccessError::MetadataOnly,
        ),
        (
            ContentReadEligibility::PermissionRequired,
            NativePreviewAccessError::PermissionDenied,
        ),
        (
            ContentReadEligibility::SourceUnavailable,
            NativePreviewAccessError::SourceUnavailable,
        ),
    ] {
        let (_fixture, gate, registry, source, source_version, _resolver) = setup(b"bytes");
        gate.set_test_eligibility(Some(eligibility));

        assert_eq!(
            registry.stage(
                NativePreviewAccessRequest {
                    session_id: "session-1".to_string(),
                    request_id: "request-1".to_string(),
                    source,
                    source_version: source_version.clone(),
                    host: PreviewHostKind::ZenFloating,
                },
                &context(&source_version),
            ),
            Err(expected_error),
            "eligibility {eligibility:?} maps to the wrong native access error"
        );
        assert_eq!(registry.counts(), (0, 0, 0));
        assert_eq!(gate.active_lease_count(), 0);
        assert_no_stage_roots(&registry);
    }
}

#[test]
fn stages_complete_source_behind_opaque_host_bound_token() {
    let (_fixture, _gate, registry, source, source_version, _resolver) =
        setup(b"native preview bytes");
    let operation = context(&source_version);
    let handle = registry
        .stage(
            NativePreviewAccessRequest {
                session_id: "session-1".to_string(),
                request_id: "request-1".to_string(),
                source,
                source_version: source_version.clone(),
                host: PreviewHostKind::ZenFloating,
            },
            &operation,
        )
        .unwrap();
    assert!(!handle.token.contains("document.pdf"));
    let path = registry
        .resolve(&NativePreviewAccessResolveRequest {
            token: handle.token,
            session_id: "session-1".to_string(),
            request_id: "request-1".to_string(),
            source_version,
            host: PreviewHostKind::ZenFloating,
        })
        .unwrap();
    assert_eq!(fs::read(path).unwrap(), b"native preview bytes");
    assert_eq!(registry.counts(), (1, 0, 20));
}

#[test]
fn complete_multi_chunk_staging_uses_one_fresh_source_resolution_for_the_copy() {
    let bytes = vec![b'x'; 64 * 1024 + 17];
    let (_fixture, _gate, registry, source, source_version, resolver) = setup(&bytes);
    let operation = context(&source_version);
    let handle = registry
        .stage(
            NativePreviewAccessRequest {
                session_id: "session-1".to_string(),
                request_id: "request-1".to_string(),
                source,
                source_version: source_version.clone(),
                host: PreviewHostKind::ZenFloating,
            },
            &operation,
        )
        .unwrap();
    let path = registry
        .resolve(&NativePreviewAccessResolveRequest {
            token: handle.token,
            session_id: "session-1".to_string(),
            request_id: "request-1".to_string(),
            source_version,
            host: PreviewHostKind::ZenFloating,
        })
        .unwrap();
    assert_eq!(fs::read(path).unwrap(), bytes);
    // setup's version lookup, the staging-name hint and the final
    // publication revalidation account for three resolutions. The copy
    // itself contributes exactly one fresh resolution, regardless of the
    // number of chunks.
    assert_eq!(resolver.resolve_count(), 4);
}

#[test]
fn streamed_copy_rejects_source_path_replacement_without_second_resolution() {
    let bytes = vec![b'x'; 32];
    let (fixture, gate, _registry, source, source_version, resolver) = setup(&bytes);
    let source_path = fixture.root.join("document.pdf");
    let original_path = fixture.root.join("original.pdf");
    let mut writer = ReplaceSourcePathOnFirstWrite {
        source_path: source_path.clone(),
        original_path,
        replaced: false,
    };
    let resolutions_before_copy = resolver.resolve_count();

    assert_eq!(
        gate.stream_verified_source_to_writer(
            &source,
            &source_version,
            &context(&source_version),
            VerifiedCopyBounds {
                max_total_bytes: 1024,
                chunk_bytes: 4,
            },
            || Ok(()),
            &mut writer,
        ),
        Err(VerifiedCopyError::Access(
            PreviewReadAccessError::SourceVersionMismatch
        ))
    );
    assert!(writer.replaced);
    assert_eq!(resolver.resolve_count(), resolutions_before_copy + 1);
    assert_eq!(gate.active_lease_count(), 0);
}

struct ReplaceSourcePathOnFirstWrite {
    source_path: std::path::PathBuf,
    original_path: std::path::PathBuf,
    replaced: bool,
}

impl std::io::Write for ReplaceSourcePathOnFirstWrite {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if !self.replaced {
            std::fs::rename(&self.source_path, &self.original_path)?;
            std::fs::write(&self.source_path, b"replacement source")?;
            self.replaced = true;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn over_budget_copy_deletes_partial_staging_and_releases_capacity() {
    let (_fixture, _gate, registry, source, source_version, _resolver) = setup_with_config(
        b"too-large",
        NativePreviewAccessConfig {
            max_records: 2,
            max_file_bytes: 4,
            max_total_bytes: 4,
            ttl: Duration::from_secs(30),
            read_chunk_bytes: 2,
            max_acquisition_duration: Duration::from_secs(20),
        },
    );
    let operation = context(&source_version);
    assert_eq!(
        registry.stage(
            NativePreviewAccessRequest {
                session_id: "session-1".to_string(),
                request_id: "request-1".to_string(),
                source,
                source_version,
                host: PreviewHostKind::ZenFloating,
            },
            &operation,
        ),
        Err(NativePreviewAccessError::SourceTooLarge)
    );
    assert_eq!(registry.counts(), (0, 0, 0));
    assert_no_stage_roots(&registry);
}

#[test]
fn final_source_version_drift_discards_completed_copy() {
    let (fixture, _gate, registry, source, source_version, resolver) = setup(b"original");
    let replacement = fixture.root.join("replacement.pdf");
    fs::write(&replacement, b"replacement").unwrap();
    resolver.replace_on_resolve(4, replacement);
    let operation = context(&source_version);
    assert_eq!(
        registry.stage(
            NativePreviewAccessRequest {
                session_id: "session-1".to_string(),
                request_id: "request-1".to_string(),
                source,
                source_version,
                host: PreviewHostKind::ZenFloating,
            },
            &operation,
        ),
        Err(NativePreviewAccessError::IdentityChanged)
    );
    assert_eq!(registry.counts(), (0, 0, 0));
    assert_no_stage_roots(&registry);
}

#[test]
fn verified_copy_fails_closed_on_cancel_and_read_gate_revoke() {
    let (_fixture, gate, _registry, source, source_version, _resolver) =
        setup(b"copy cancellation fixture");
    let cancellation = PreviewCancellation::default();
    let canceled_context = PreviewOperationContext::for_backend_content_read(
        "session-1",
        "request-1",
        source_version.clone(),
        cancellation.clone(),
        Instant::now() + Duration::from_secs(2),
    );
    let mut canceled_writer = CancelingWriter {
        bytes: Vec::new(),
        cancellation: Some(cancellation),
        gate: None,
        lease_revoked: None,
    };
    assert_eq!(
        gate.stream_verified_source_to_writer(
            &source,
            &source_version,
            &canceled_context,
            VerifiedCopyBounds {
                max_total_bytes: 1024,
                chunk_bytes: 4,
            },
            || Ok(()),
            &mut canceled_writer,
        ),
        Err(VerifiedCopyError::Access(PreviewReadAccessError::Cancelled))
    );
    assert!(!canceled_writer.bytes.is_empty());
    assert_eq!(gate.active_lease_count(), 0);

    let lease_revoked = Arc::new(AtomicBool::new(false));
    let mut lease_revoked_writer = CancelingWriter {
        bytes: Vec::new(),
        cancellation: None,
        gate: None,
        lease_revoked: Some(Arc::clone(&lease_revoked)),
    };
    assert_eq!(
        gate.stream_verified_source_to_writer(
            &source,
            &source_version,
            &context(&source_version),
            VerifiedCopyBounds {
                max_total_bytes: 1024,
                chunk_bytes: 4,
            },
            || {
                if lease_revoked.load(Ordering::Acquire) {
                    Err(PreviewReadAccessError::Cancelled)
                } else {
                    Ok(())
                }
            },
            &mut lease_revoked_writer,
        ),
        Err(VerifiedCopyError::Access(PreviewReadAccessError::Cancelled))
    );
    assert!(!lease_revoked_writer.bytes.is_empty());
    assert_eq!(gate.active_lease_count(), 0);

    let operation = context(&source_version);
    let mut revoked_writer = CancelingWriter {
        bytes: Vec::new(),
        cancellation: None,
        gate: Some(Arc::clone(&gate)),
        lease_revoked: None,
    };
    assert_eq!(
        gate.stream_verified_source_to_writer(
            &source,
            &source_version,
            &operation,
            VerifiedCopyBounds {
                max_total_bytes: 1024,
                chunk_bytes: 4,
            },
            || Ok(()),
            &mut revoked_writer,
        ),
        Err(VerifiedCopyError::Access(
            PreviewReadAccessError::LeaseInvalid
        ))
    );
    assert!(!revoked_writer.bytes.is_empty());
    assert_eq!(gate.active_lease_count(), 0);
}
