//! macOS Foundation-backed file, package, cloud, and volume metadata.

#[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
compile_error!("Fylura macOS builds require Apple Silicon (aarch64)");

pub mod activity;
pub mod cloud_item;
#[cfg(target_os = "macos")]
pub mod copy;
pub mod file_provider;
pub mod file_semantics;
pub mod finder;
pub mod identity;
pub mod lifecycle;
pub mod mutation;
pub mod native_preview;
pub mod package;
pub mod qos;
pub mod quick_look;
pub(crate) mod run_loop;
#[cfg(target_os = "macos")]
pub mod strategy;
pub mod types;
pub mod volume;

pub use file_provider::{
    MacFileProviderDetection, MacProviderIdentityEvidence, MacProviderMaterialization,
    MacProviderMaterializationEvidence,
};
pub use file_semantics::{MacContentReadEligibility, MacFileSemantics};
pub use types::{MacCloudBacking, MacContentAvailability};
pub use volume::MacVolumeSemantics;

#[cfg(target_os = "macos")]
pub fn operating_system_version() -> Option<String> {
    use objc2_foundation::NSProcessInfo;

    Some(
        NSProcessInfo::processInfo()
            .operatingSystemVersionString()
            .to_string(),
    )
}

#[cfg(not(target_os = "macos"))]
pub fn operating_system_version() -> Option<String> {
    None
}
