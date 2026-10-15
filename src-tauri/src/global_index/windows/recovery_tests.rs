#[test]
fn usn_discontinuities_remain_durable_and_typed_through_provider() {
    use crate::db::Database;
    for case in 0..11 {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp-tests")
            .join(format!("usn-recovery-{}.sqlite3", uuid::Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let mut source = windows_source();
        source.volume.journal_id = Some("7".into());
        source.volume.journal_cursor = Some("42".into());
        db.upsert_global_volume(&source.volume).unwrap();
        let mut sink = SourceStateSink {
            database: Some(db.clone()),
            ..Default::default()
        };
        let outcome: Result<usn::UsnSyncResult, GlobalIndexError> = match case {
            0 => usn::validate_checkpoint(&source, &mut sink, 8, 0, 100).map(|_| unreachable!()),
            1 => usn::validate_checkpoint(&source, &mut sink, 7, 43, 100).map(|_| unreachable!()),
            2 => usn::validate_checkpoint(&source, &mut sink, 7, 0, 41).map(|_| unreachable!()),
            3..=5 => usn::classify_read_error(
                "volume",
                &mut sink,
                GlobalIndexError::WindowsIo([1178, 1179, 1181][case - 3]),
            ),
            6 => usn::parse_sync_page("volume", &mut sink, &[1], 42, 100).map(|_| unreachable!()),
            7 => {
                let mut page = 50u64.to_ne_bytes().to_vec();
                page.push(1);
                usn::parse_sync_page("volume", &mut sink, &page, 42, 100).map(|_| unreachable!())
            }
            8 => usn::parse_sync_page("volume", &mut sink, &42u64.to_ne_bytes(), 42, 100)
                .map(|_| unreachable!()),
            10 => usn::parse_sync_page("volume", &mut sink, &101u64.to_ne_bytes(), 42, 100)
                .map(|_| unreachable!()),
            _ => Ok(usn::UsnSyncResult {
                directory_path_changed: true,
            }),
        };
        let returned = finish_incremental_result("volume", &mut sink, outcome);
        assert!(
            matches!(returned, Err(GlobalIndexError::RebuildRequired(_))),
            "case {case}"
        );
        assert_eq!(
            db.get_global_volume("volume")
                .unwrap()
                .unwrap()
                .index_status,
            INDEX_STATUS_REBUILD_REQUIRED,
            "case {case}"
        );
        drop(sink);
        drop(db);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }
}

#[test]
fn diagnostic_strings_never_decide_windows_recovery() {
    let mut sink = SourceStateSink::default();
    let message = "rebuild required mft_integrity: Win32 error 1181 indexing paused";
    let error = GlobalIndexError::Provider(message.into());
    assert!(!mft::is_integrity_error(&error));
    assert!(!mft::is_win32_error(&error, 1181));
    let result = usn::classify_read_error("volume", &mut sink, error);
    assert!(matches!(
        finish_incremental_result("volume", &mut sink, result),
        Err(GlobalIndexError::Provider(_))
    ));
    assert_eq!(
        sink.status.as_deref(),
        Some(crate::global_index::models::INDEX_STATUS_ERROR)
    );
    assert!(matches!(
        finish_incremental_result("volume", &mut sink, Err(GlobalIndexError::Paused)),
        Err(GlobalIndexError::Paused)
    ));
}
