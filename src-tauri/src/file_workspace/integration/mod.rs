//! Bounded W1-10 integration adapters.
//!
//! This module composes the existing W1 services into a process-local Tauri
//! surface.  It is deliberately not a new authority: Browse owns ephemeral
//! refs, the read gate owns byte-read admission, ThumbnailService owns
//! thumbnail work, and PreviewSession owns only disposable lifecycle state.

mod browse;
mod change;
pub mod commands;
mod folder;
mod known_locations;
mod location;
pub(crate) mod places;
mod preview;
mod runtime;
mod runtime_owner;
mod thumbnail;
pub mod types;

pub use known_locations::{KnownLocation, KnownLocationKind};
pub use runtime::FileWorkspaceRuntime;
pub use runtime_owner::{main_generation_from_window, FileWorkspaceRuntimeOwner};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod preview_lifecycle_tests;

#[cfg(test)]
mod native_preview_tests;

#[cfg(test)]
mod folder_preview_tests;

#[cfg(test)]
mod performance;
