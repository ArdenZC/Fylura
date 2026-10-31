use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CapabilityStatus {
    Implemented,
    RuntimeDependent,
    Unavailable,
    NeedsMaterialization,
    NeedsPermission,
    ProviderOffline,
    FilesystemInsufficient,
    NotFixtureValidated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityLayers {
    pub platform_feature_availability: CapabilityStatus,
    pub runtime_environment_capability: CapabilityStatus,
    pub operation_eligibility: CapabilityStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCapabilities {
    pub platform: &'static str,
    pub architecture: &'static str,
    pub macos_version: Option<String>,
    pub ai_debug_available: bool,
    pub real_ai_classification_available: bool,
    pub credential_store_available: bool,
    pub file_mutation_available: bool,
    pub file_mutation_unavailable_code: Option<&'static str>,
    pub copy_available: bool,
    pub duplicate_available: bool,
    pub rename_available: bool,
    pub same_volume_move_available: bool,
    pub cross_volume_move_available: bool,
    pub replace_available: bool,
    pub safe_trash_available: bool,
    pub restore_available: bool,
    pub permanent_delete_available: bool,
    pub secure_removal_available: bool,
    pub package_mutation_available: bool,
    pub i_cloud_mutation_available: bool,
    pub file_provider_mutation_available: bool,
    pub external_volume_mutation_available: bool,
    pub network_volume_mutation_available: bool,
    pub backend_watcher_reconciliation: bool,
    pub macos_native_semantics_available: bool,
    pub macos_same_volume_mutation_available: bool,
    pub macos_rename_available: bool,
    pub macos_safe_trash_available: bool,
    pub macos_cloud_mutation_available: bool,
    pub macos_file_provider_mutation_available: bool,
    pub macos_package_mutation_available: bool,
    pub macos_cross_volume_mutation_available: bool,
    pub macos_lifecycle_available: bool,
    pub macos_finder_available: bool,
    pub macos_quick_look_thumbnail_available: bool,
    pub macos_quick_look_preview_available: bool,
    pub macos_restore_available: bool,
    pub macos_activity_policy_available: bool,
    pub macos_icloud_awareness_available: bool,
    pub macos_file_provider_awareness_available: bool,
    pub macos_package_awareness_available: bool,
    /// These layers deliberately separate a platform API existing from this
    /// runtime having the required identity/materialization evidence and a
    /// particular operation being eligible.
    pub file_provider_capabilities: CapabilityLayers,
    pub external_volume_capabilities: CapabilityLayers,
    pub network_volume_capabilities: CapabilityLayers,
}

fn capabilities(ai_debug_available: bool) -> RuntimeCapabilities {
    RuntimeCapabilities {
        platform: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        macos_version: crate::platform::macos::operating_system_version(),
        ai_debug_available,
        real_ai_classification_available: true,
        credential_store_available: cfg!(any(target_os = "windows", target_os = "macos")),
        // macOS uses the recoverable namespace transaction implemented by the
        // existing operation journal and Safe Trash/restore authorities.  The
        // capability means the strategy exists; per-volume/provider
        // eligibility is still evaluated by the backend at execution time.
        file_mutation_available: cfg!(any(windows, target_os = "macos")),
        file_mutation_unavailable_code: if cfg!(any(windows, target_os = "macos")) {
            None
        } else {
            Some(crate::fs_safety::platform_support::UNSUPPORTED_PLATFORM_LINUX)
        },
        copy_available: cfg!(any(windows, target_os = "macos")),
        duplicate_available: cfg!(any(windows, target_os = "macos")),
        rename_available: cfg!(any(windows, target_os = "macos")),
        same_volume_move_available: cfg!(any(windows, target_os = "macos")),
        // The backend implements a journaled copy/verify/publish/retire path on
        // both platforms. Per-volume and provider eligibility is still checked
        // at operation time, and is represented by the layered capabilities.
        cross_volume_move_available: cfg!(any(windows, target_os = "macos")),
        replace_available: cfg!(any(windows, target_os = "macos")),
        safe_trash_available: cfg!(any(windows, target_os = "macos")),
        restore_available: cfg!(any(windows, target_os = "macos")),
        permanent_delete_available: cfg!(target_os = "macos"),
        // The quarantine/delete transaction is available on macOS, but Zen
        // Canvas does not claim physical SSD-sector erasure.
        secure_removal_available: false,
        package_mutation_available: cfg!(target_os = "macos"),
        i_cloud_mutation_available: cfg!(target_os = "macos"),
        // These booleans are executable capability claims, not path hints.
        // Generic File Provider mutation exposes the coordinated user-visible
        // URL route; provider and volume eligibility remain fail-closed until
        // operation-time evidence exists.
        file_provider_mutation_available:
            crate::platform::macos::file_provider::GENERIC_FILE_PROVIDER_MUTATION_AVAILABLE,
        external_volume_mutation_available: false,
        network_volume_mutation_available: false,
        backend_watcher_reconciliation: crate::watcher::backend_watcher_reconciliation_enabled(),
        macos_native_semantics_available: cfg!(target_os = "macos"),
        macos_same_volume_mutation_available: cfg!(target_os = "macos"),
        macos_rename_available: cfg!(target_os = "macos"),
        macos_safe_trash_available: cfg!(target_os = "macos"),
        macos_cloud_mutation_available: cfg!(target_os = "macos"),
        macos_file_provider_mutation_available:
            crate::platform::macos::file_provider::GENERIC_FILE_PROVIDER_MUTATION_AVAILABLE,
        macos_package_mutation_available: cfg!(target_os = "macos"),
        macos_cross_volume_mutation_available: cfg!(target_os = "macos"),
        macos_lifecycle_available: cfg!(target_os = "macos"),
        macos_finder_available: cfg!(target_os = "macos"),
        macos_quick_look_thumbnail_available:
            crate::platform::macos::quick_look::thumbnail_available(),
        macos_quick_look_preview_available: crate::platform::macos::native_preview::available(),
        macos_restore_available: cfg!(target_os = "macos"),
        macos_activity_policy_available: crate::platform::macos::activity::AVAILABLE,
        // iCloud and generic File Provider awareness are native routing
        // capabilities. Real provider fixture validation remains a separate
        // evidence claim from this capability advertisement.
        macos_icloud_awareness_available: cfg!(target_os = "macos"),
        macos_file_provider_awareness_available:
            crate::platform::macos::file_provider::GENERIC_FILE_PROVIDER_AWARENESS_AVAILABLE,
        macos_package_awareness_available: cfg!(target_os = "macos"),
        file_provider_capabilities: CapabilityLayers {
            platform_feature_availability: if crate::platform::macos::file_provider::GENERIC_FILE_PROVIDER_CLIENT_IMPLEMENTED
                && crate::platform::macos::file_provider::GENERIC_FILE_PROVIDER_COORDINATED_URL_SUPPORTED
            {
                CapabilityStatus::Implemented
            } else {
                CapabilityStatus::Unavailable
            },
            runtime_environment_capability: if crate::platform::macos::file_provider::GENERIC_FILE_PROVIDER_COORDINATED_URL_SUPPORTED {
                CapabilityStatus::RuntimeDependent
            } else {
                CapabilityStatus::Unavailable
            },
            operation_eligibility: if crate::platform::macos::file_provider::GENERIC_FILE_PROVIDER_MUTATION_AVAILABLE {
                CapabilityStatus::NotFixtureValidated
            } else {
                CapabilityStatus::Unavailable
            },
        },
        external_volume_capabilities: CapabilityLayers {
            platform_feature_availability: if cfg!(target_os = "macos") {
                CapabilityStatus::Implemented
            } else {
                CapabilityStatus::Unavailable
            },
            runtime_environment_capability: CapabilityStatus::RuntimeDependent,
            operation_eligibility: CapabilityStatus::NotFixtureValidated,
        },
        network_volume_capabilities: CapabilityLayers {
            platform_feature_availability: if cfg!(target_os = "macos") {
                CapabilityStatus::Implemented
            } else {
                CapabilityStatus::Unavailable
            },
            runtime_environment_capability: CapabilityStatus::RuntimeDependent,
            operation_eligibility: CapabilityStatus::NotFixtureValidated,
        },
    }
}

#[tauri::command]
pub fn get_runtime_capabilities() -> RuntimeCapabilities {
    capabilities(cfg!(any(debug_assertions, feature = "ai-debug")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_capabilities_hide_ai_debug_without_disabling_real_ai() {
        let release = capabilities(false);
        assert!(!release.ai_debug_available);
        assert!(release.real_ai_classification_available);
        let windows = release.platform == "windows";
        assert_eq!(
            release.file_mutation_available,
            windows || release.platform == "macos"
        );
        assert_eq!(
            release.copy_available,
            windows || release.platform == "macos"
        );
        assert_eq!(
            release.cross_volume_move_available,
            windows || release.platform == "macos"
        );
        assert_eq!(
            release.replace_available,
            windows || release.platform == "macos"
        );
        assert_eq!(release.macos_restore_available, release.platform == "macos");
        assert_eq!(
            release.macos_same_volume_mutation_available,
            release.platform == "macos"
        );
        assert_eq!(release.macos_rename_available, release.platform == "macos");
        assert_eq!(
            release.macos_safe_trash_available,
            release.platform == "macos"
        );
        assert_eq!(
            release.macos_cloud_mutation_available,
            release.platform == "macos"
        );
        let provider_mutation_available =
            crate::platform::macos::file_provider::GENERIC_FILE_PROVIDER_MUTATION_AVAILABLE;
        assert_eq!(
            release.macos_file_provider_mutation_available,
            provider_mutation_available
        );
        assert_eq!(
            release.macos_package_mutation_available,
            release.platform == "macos"
        );
        assert_eq!(
            release.macos_cross_volume_mutation_available,
            release.platform == "macos"
        );
        assert_eq!(
            release.file_mutation_unavailable_code,
            if windows || release.platform == "macos" {
                None
            } else {
                Some(crate::fs_safety::platform_support::UNSUPPORTED_PLATFORM_LINUX)
            }
        );
        assert_eq!(
            release.macos_file_provider_awareness_available,
            release.platform == "macos"
        );
        assert_eq!(
            release.file_provider_mutation_available,
            provider_mutation_available
        );
        assert!(!release.external_volume_mutation_available);
        assert!(!release.network_volume_mutation_available);
        assert_eq!(
            release.file_provider_capabilities.operation_eligibility,
            if provider_mutation_available {
                CapabilityStatus::NotFixtureValidated
            } else {
                CapabilityStatus::Unavailable
            }
        );
    }

    #[test]
    fn debug_capabilities_expose_ai_debug() {
        assert!(capabilities(true).ai_debug_available);
    }
}
