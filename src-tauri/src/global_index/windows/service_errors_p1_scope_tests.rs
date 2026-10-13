//! Repaired P1 contract; pre-fix outputs remain in the committed research evidence.
use super::{classify, decode};
use crate::global_index::coordinator::GlobalIndexError;
use crate::global_index::windows::service::IndexServiceResponse;
use std::mem::discriminant;

fn trusted_response(code: &str) -> IndexServiceResponse {
    IndexServiceResponse {
        protocol_version: 3,
        request_id: "p1-boundary-probe".into(),
        ok: false,
        error_code: Some(code.into()),
        // Identical text: taxonomy evidence cannot depend on message matching.
        message: Some("same diagnostic for every semantic class".into()),
        status: None,
    }
}

#[test]
fn p1_scope_busy_permission_unknown_remain_distinct_at_public_error_boundary() {
    let provider = GlobalIndexError::Provider("ordinary provider failure".into());
    for (code, required) in [
        ("index_service_busy", "ServiceBusy"),
        ("index_permission_required", "PermissionRequired"),
        ("index_outcome_unknown", "ServiceProtocol"),
    ] {
        let decoded = decode(&trusted_response(code));
        assert!(!matches!(&decoded, GlobalIndexError::Provider(_)));
        assert_ne!(discriminant(&decoded), discriminant(&provider));
        println!(
            "p1_scope_blocker code={code} required={required} actual={decoded:?} typed_contract_pass=true"
        );
    }
}

#[test]
fn p1_scope_true_provider_is_not_encoded_as_permission() {
    let (code, status) = classify(&GlobalIndexError::Provider("ordinary failure".into()));
    assert_eq!(code, "index_provider_error");
    assert_eq!(status, "error");
    println!(
        "p1_scope_blocker server_input=Provider actual_code={code} provider_permission_distinct=true"
    );
}

#[test]
fn p1_scope_real_sqlite_busy_keeps_local_database_identity() {
    struct OwnedDirectory(std::path::PathBuf);
    impl Drop for OwnedDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "zen380-p1-sqlite-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    // Never adopt or delete an existing directory. No clock-based ordering.
    std::fs::create_dir(&path).unwrap();
    let directory = OwnedDirectory(path);
    let path = directory.0.join("isolated.sqlite");
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer
        .execute_batch("CREATE TABLE sample(id INTEGER); INSERT INTO sample VALUES(1);")
        .unwrap();
    let contender = rusqlite::Connection::open(&path).unwrap();
    contender.busy_timeout(std::time::Duration::ZERO).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let error = contender.execute("UPDATE sample SET id=2", []).unwrap_err();
    assert!(matches!(
        &error,
        rusqlite::Error::SqliteFailure(error, _) if error.extended_code == rusqlite::ffi::SQLITE_BUSY
    ));
    let local = GlobalIndexError::Database(error.into());
    let remote = decode(&trusted_response("index_service_busy"));
    assert_ne!(discriminant(&local), discriminant(&remote));
    writer.execute_batch("ROLLBACK").unwrap();
    println!(
        "p1_scope_sqlite sqlite_version={} actual_sqlite_code=5 local=Database remote_service_busy=ServiceBusy sqlite_service_distinct=true",
        rusqlite::version()
    );
}
