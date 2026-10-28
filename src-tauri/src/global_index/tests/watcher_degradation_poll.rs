//! Regression coverage for the bounded poll fallback.
//!
//! Review #5678861648 (recovery coupling): a watcher that never starts used to
//! abort the whole MFT/USN baseline. It now degrades the source to
//! `watcher_unavailable`, and the coordinator keeps reconciling that source on
//! its low-frequency audit tick. `has_watcher_unavailable_sources` is the probe
//! that keeps that fallback alive, so it must be true exactly while an enabled
//! source cannot observe changes.

use super::{test_db_path, test_volume};
use crate::db::Database;
use crate::global_index::{INDEX_STATUS_READY, INDEX_STATUS_WATCHER_UNAVAILABLE};

fn volume(id: &str, enabled: bool) -> crate::global_index::GlobalVolume {
    let mut volume = test_volume();
    volume.id = id.to_string();
    volume.stable_volume_id = format!("stable-{id}");
    volume.mount_path = format!(r"C:\Global\{id}\");
    volume.enabled = enabled;
    volume.index_status = INDEX_STATUS_WATCHER_UNAVAILABLE.to_string();
    volume
}

#[test]
fn degraded_enabled_source_keeps_the_poll_fallback_alive() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open db");
    db.upsert_global_volume(&volume("gv_poll", true))
        .expect("upsert");

    assert!(
        db.has_watcher_unavailable_sources().expect("probe"),
        "an enabled frozen source must schedule the bounded poll"
    );

    db.update_global_volume_state("gv_poll", INDEX_STATUS_READY, None, None, None, None, None)
        .expect("recover");
    assert!(
        !db.has_watcher_unavailable_sources()
            .expect("probe after recovery"),
        "recovery to ready must stop the extra cycles"
    );

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn disabled_frozen_source_does_not_schedule_cycles() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open db");
    db.upsert_global_volume(&volume("gv_off", false))
        .expect("upsert");

    assert!(
        !db.has_watcher_unavailable_sources().expect("probe"),
        "consent is required before a degraded source may cost any work"
    );

    drop(db);
    let _ = std::fs::remove_file(path);
}
