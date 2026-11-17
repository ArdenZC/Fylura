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

/// Filename-first remains an explicit opt-in experiment until persistence
/// latency and writer contention meet the production targets.
pub(crate) fn filename_first_experimental_enabled() -> bool {
    filename_first_experimental_enabled_for_value(
        std::env::var("ZC_GLOBAL_INDEX_FILENAME_FIRST")
            .ok()
            .as_deref(),
    )
}

fn filename_first_experimental_enabled_for_value(value: Option<&str>) -> bool {
    value == Some("1")
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod filename_first_flag_tests {
    #[test]
    fn filename_first_is_explicit_opt_in() {
        assert!(!super::filename_first_experimental_enabled_for_value(None));
        assert!(!super::filename_first_experimental_enabled_for_value(Some(
            "0"
        )));
        assert!(!super::filename_first_experimental_enabled_for_value(Some(
            "true"
        )));
        assert!(super::filename_first_experimental_enabled_for_value(Some(
            "1"
        )));
    }
}

#[cfg(test)]
mod lock_contention_tests;
