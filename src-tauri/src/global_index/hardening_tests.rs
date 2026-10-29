use super::models::*;
use crate::db::Database;
use rusqlite::{params, OptionalExtension};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn test_db_path() -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "zen-canvas-managed-ai-hardening-{}-{id}.db",
        std::process::id()
    ))
}

fn volume() -> GlobalVolume {
    GlobalVolume {
        id: "gv_hardening".to_string(),
        platform: "windows".to_string(),
        stable_volume_id: "hardening-volume".to_string(),
        display_name: "Hardening".to_string(),
        mount_path: r"C:\Managed".to_string(),
        filesystem_type: "ntfs".to_string(),
        drive_kind: "fixed".to_string(),
        enabled: true,
        scope_authorization_state: "authorized".to_string(),
        provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
        index_status: INDEX_STATUS_READY.to_string(),
        last_error: None,
        journal_id: Some("1".to_string()),
        journal_cursor: Some("1".to_string()),
        last_full_index_at: Some(1),
        last_incremental_sync_at: Some(1),
        entry_count: 0,
        created_at: 1,
        updated_at: 1,
    }
}

fn entry(path: &str) -> GlobalEntryInput {
    GlobalEntryInput {
        volume_id: "gv_hardening".to_string(),
        platform_file_id: format!("frn:{path}"),
        parent_platform_file_id: "frn:parent".to_string(),
        name: path.rsplit('\\').next().unwrap_or(path).to_string(),
        path: path.to_string(),
        extension: "txt".to_string(),
        is_directory: false,
        size: 42,
        created_at_fs: Some(1),
        modified_at_fs: Some(2),
        file_attributes: 0,
        is_hidden: false,
        is_system: false,
        source_provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
        last_seen_at: 2,
    }
}

fn setup(entry_path: &str) -> (PathBuf, Database, GlobalEntryInput) {
    let path = test_db_path();
    let db = Database::open(&path).expect("open database");
    db.upsert_global_volume(&volume()).expect("insert volume");
    let input = entry(entry_path);
    db.upsert_global_entries_batch(std::slice::from_ref(&input))
        .expect("insert entry");
    (path, db, input)
}

fn job_status(db: &Database) -> String {
    db.conn()
        .expect("connection")
        .query_row("SELECT status FROM ai_jobs ORDER BY created_at LIMIT 1", [], |row| row.get(0))
        .expect("job status")
}

#[test]
fn canceled_jobs_do_not_revive_when_the_entry_is_reindexed() {
    let (path, db, input) = setup(r"C:\Managed\Canceled\note.txt");
    db.add_managed_scope(AddManagedScopeRequest {
        path: r"C:\Managed\Canceled".to_string(),
        global_entry_id: None,
        enabled: true,
        allow_local_ai: true,
        allow_cloud_ai: false,
    })
    .expect("add scope");
    db.cancel_managed_ai_queue().expect("cancel jobs");
    assert_eq!(job_status(&db), AI_JOB_CANCELED);

    db.upsert_global_entries_batch(std::slice::from_ref(&input))
        .expect("reindex entry");
    assert_eq!(job_status(&db), AI_JOB_CANCELED);

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn provider_policy_switch_requeues_for_the_current_provider() {
    let (path, db, _) = setup(r"C:\Managed\Provider\note.txt");
    let scope = db
        .add_managed_scope(AddManagedScopeRequest {
            path: r"C:\Managed\Provider".to_string(),
            global_entry_id: None,
            enabled: true,
            allow_local_ai: true,
            allow_cloud_ai: false,
        })
        .expect("add scope");
    db.update_managed_scope_policy(UpdateManagedScopePolicyRequest {
        id: scope.id,
        enabled: None,
        allow_local_ai: Some(false),
        allow_cloud_ai: Some(true),
    })
    .expect("switch provider policy");

    let job = db
        .claim_next_managed_ai_job("cloud")
        .expect("claim cloud job")
        .expect("cloud job available");
    assert_eq!(job.provider, "cloud");

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn overlapping_scopes_claim_only_the_most_specific_policy() {
    let (path, db, _) = setup(r"C:\Managed\Shared\note.txt");
    let broad = db
        .add_managed_scope(AddManagedScopeRequest {
            path: r"C:\Managed".to_string(),
            global_entry_id: None,
            enabled: true,
            allow_local_ai: true,
            allow_cloud_ai: false,
        })
        .expect("add broad scope");
    let narrow = db
        .add_managed_scope(AddManagedScopeRequest {
            path: r"C:\Managed\Shared".to_string(),
            global_entry_id: None,
            enabled: true,
            allow_local_ai: true,
            allow_cloud_ai: false,
        })
        .expect("add narrow scope");

    let job = db
        .claim_next_managed_ai_job("local")
        .expect("claim job")
        .expect("job available");
    assert_eq!(job.managed_scope_id, narrow.id);
    assert_ne!(job.managed_scope_id, broad.id);

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn disabling_scope_prevents_an_inflight_job_from_completing() {
    let (path, db, _) = setup(r"C:\Managed\Race\note.txt");
    let scope = db
        .add_managed_scope(AddManagedScopeRequest {
            path: r"C:\Managed\Race".to_string(),
            global_entry_id: None,
            enabled: true,
            allow_local_ai: true,
            allow_cloud_ai: false,
        })
        .expect("add scope");
    let job = db
        .claim_next_managed_ai_job("local")
        .expect("claim job")
        .expect("job available");
    db.update_managed_scope_policy(UpdateManagedScopePolicyRequest {
        id: scope.id,
        enabled: Some(false),
        allow_local_ai: None,
        allow_cloud_ai: None,
    })
    .expect("disable scope");

    let result = serde_json::json!({
        "refId": format!("managed:{}", job.global_entry_id),
        "fileType": "document",
        "purpose": "work",
        "lifecycle": "active",
        "riskLevel": "low",
        "suggestedAction": "keep",
        "confidence": 0.9,
        "reason": "test"
    })
    .to_string();
    db.complete_managed_ai_job(&job, "test-model", &result)
        .expect("completion is ignored safely");

    let conn = db.conn().expect("connection");
    let status: String = conn
        .query_row("SELECT status FROM ai_jobs WHERE id = ?1", params![job.id], |row| row.get(0))
        .expect("job status");
    assert_eq!(status, AI_JOB_BLOCKED_BY_POLICY);
    let analysis: Option<String> = conn
        .query_row(
            "SELECT classification_json FROM ai_analysis_state WHERE global_entry_id = ?1",
            params![job.global_entry_id],
            |row| row.get(0),
        )
        .optional()
        .expect("analysis state")
        .flatten();
    assert!(analysis.is_none());

    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(path);
}


#[test]
fn initial_scope_backfill_caps_jobs_but_manages_every_entry() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open database");
    db.upsert_global_volume(&volume()).expect("insert volume");
    let entries = (0..120)
        .map(|index| entry(&format!(r"C:\Managed\Bulk\file-{index:03}.txt")))
        .collect::<Vec<_>>();
    for batch in entries.chunks(40) {
        db.upsert_global_entries_batch(batch).expect("insert batch");
    }

    db.add_managed_scope(AddManagedScopeRequest {
        path: r"C:\Managed\Bulk".to_string(),
        global_entry_id: None,
        enabled: true,
        allow_local_ai: true,
        allow_cloud_ai: false,
    })
    .expect("add scope");

    let conn = db.conn().expect("connection");
    let managed_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM managed_entries WHERE enabled = 1",
            [],
            |row| row.get(0),
        )
        .expect("managed count");
    let job_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ai_jobs", [], |row| row.get(0))
        .expect("job count");
    assert_eq!(managed_count, 120);
    assert_eq!(job_count, 100);

    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(path);
}
