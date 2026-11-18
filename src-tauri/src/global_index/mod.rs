pub mod commands;
pub mod coordinator;
#[path = "index_scope/mod.rs"]
pub mod coverage;
pub(crate) mod filename_search;
mod generation_repository;
mod legacy_queue;
pub mod managed_scope;
#[path = "managed_worker_hardened.rs"]
pub mod managed_worker;
pub mod models;
pub(crate) mod qa_trace;
mod repository;
pub mod search;
pub(crate) mod wake;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

pub use commands::*;
pub use coordinator::{
    GlobalIndexCoordinator, GlobalIndexProvider, GlobalIndexSink, SourceOperationKind,
    SourceOperationRoute,
};
pub use managed_worker::ManagedAiWorker;
pub use models::*;
pub(crate) use repository::enqueue_managed_ai_for_library_files;
pub use search::search_global_entries;

/// Filename-first is enabled by default for Windows MFT initial indexing and
/// rebuilds. Set the environment variable to `0` to disable it or `1` to
/// enable it explicitly on another platform. macOS Spotlight indexing is
/// unaffected.
pub(crate) fn filename_first_enabled() -> bool {
    filename_first_enabled_for_value(
        std::env::var("ZC_GLOBAL_INDEX_FILENAME_FIRST")
            .ok()
            .as_deref(),
    )
}

fn filename_first_enabled_for_value(value: Option<&str>) -> bool {
    match value {
        Some("0") => false,
        Some("1") => true,
        None => cfg!(target_os = "windows"),
        Some(_) => false,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod filename_first_flag_tests {
    #[test]
    fn filename_first_defaults_to_windows_and_environment_overrides() {
        assert_eq!(
            super::filename_first_enabled_for_value(None),
            cfg!(target_os = "windows")
        );
        assert!(!super::filename_first_enabled_for_value(Some("0")));
        assert!(!super::filename_first_enabled_for_value(Some("true")));
        assert!(super::filename_first_enabled_for_value(Some("1")));
    }
}

#[cfg(test)]
mod lock_contention_tests;
