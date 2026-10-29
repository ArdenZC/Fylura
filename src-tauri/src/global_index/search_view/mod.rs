//! Default-off, disposable Global Search substring view.
//!
//! SQLite remains the sole file/source authority and the only result router.
//! The optional worker builds a compact filename/path/extension projection,
//! applies bounded change-feed batches, and compares it with FTS/BM25 inside
//! the same SQLite snapshot. It never supplies user-visible results.

mod memory;
mod view;
mod worker;

pub(crate) use worker::SearchViewManager;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "benchmark.rs"]
mod benchmark;
