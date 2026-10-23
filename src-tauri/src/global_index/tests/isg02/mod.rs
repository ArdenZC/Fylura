//! ISG-02: in-memory conformance only, frozen by #371 comment 6107816377.
//! No production caller, native adapter, disk, database, network or AI access.
//! Also runnable with `rustc --test .../isg02/mod.rs` on a non-product host.
mod matrix;
mod model;
mod oracle;
