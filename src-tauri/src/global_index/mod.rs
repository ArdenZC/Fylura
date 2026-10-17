pub mod commands;
pub mod coordinator;
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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod lock_contention_tests;
