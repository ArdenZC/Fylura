//! Error taxonomy stays structured at the public IPC boundary.
use super::{classify, decode};
use crate::global_index::coordinator::GlobalIndexError;
use crate::global_index::windows::service::{IndexServiceResponse, IPC_PROTOCOL_VERSION};
use std::mem::discriminant;

fn trusted_response(code: &str) -> IndexServiceResponse {
    IndexServiceResponse {
        protocol_version: IPC_PROTOCOL_VERSION,
        request_id: "p1-boundary-probe".into(),
        ok: false,
        error_code: Some(code.into()),
        // Identical text: taxonomy evidence cannot depend on message matching.
        message: Some("same diagnostic for every semantic class".into()),
        status: None,
    }
}

#[test]
fn remote_sqlite_busy_service_busy_provider_permission_and_unknown_remain_typed() {
    for (code, expected) in [
        ("index_database_busy", "DatabaseBusy"),
        ("index_service_busy", "ServiceBusy"),
        ("index_provider_error", "Provider"),
        ("index_permission_required", "PermissionRequired"),
        ("index_windows_io:5", "WindowsIo"),
        ("index_outcome_unknown", "ServiceProtocol"),
    ] {
        let decoded = decode(&trusted_response(code));
        let actual = match &decoded {
            GlobalIndexError::DatabaseBusy(_) => "DatabaseBusy",
            GlobalIndexError::ServiceBusy(_) => "ServiceBusy",
            GlobalIndexError::Provider(_) => "Provider",
            GlobalIndexError::PermissionRequired(_) => "PermissionRequired",
            GlobalIndexError::WindowsIo(_) => "WindowsIo",
            GlobalIndexError::ServiceProtocol(_) => "ServiceProtocol",
            other => panic!("unexpected category for {code}: {other:?}"),
        };
        assert_eq!(actual, expected);
        println!(
            "global_index_error_taxonomy code={code} expected={expected} actual={decoded:?} typed_contract_pass=true"
        );
    }
}

#[test]
fn provider_error_is_not_encoded_as_permission() {
    let (code, status) = classify(&GlobalIndexError::Provider("ordinary failure".into()));
    assert_eq!(code, "index_provider_error");
    assert_eq!(status, "error");
    println!(
        "global_index_error_taxonomy server_input=Provider actual_code={code} provider_permission_distinct=true"
    );
}

#[test]
fn real_sqlite_busy_stays_distinct_from_remote_database_busy_and_service_busy() {
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
    let remote_database_busy = decode(&trusted_response("index_database_busy"));
    let service_busy = decode(&trusted_response("index_service_busy"));
    assert_ne!(discriminant(&local), discriminant(&remote_database_busy));
    assert_ne!(
        discriminant(&remote_database_busy),
        discriminant(&service_busy)
    );
    assert!(crate::global_index::coordinator::sqlite_contention_kind(&local).is_some());
    assert!(
        crate::global_index::coordinator::sqlite_contention_kind(&remote_database_busy).is_some()
    );
    assert!(crate::global_index::coordinator::sqlite_contention_kind(&service_busy).is_none());
    writer.execute_batch("ROLLBACK").unwrap();
    println!(
        "global_index_sqlite_busy sqlite_version={} actual_sqlite_code=5 local=Database remote_database_busy=DatabaseBusy service_contention=ServiceBusy sqlite_service_distinct=true",
        rusqlite::version()
    );
}
