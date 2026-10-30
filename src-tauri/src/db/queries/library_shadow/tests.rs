use super::super::{library::*, library_identity::*};
use super::*;
use crate::{
    db::{Database, InsertFileRequest},
    fs_safety::{PhysicalFileIdentity, PhysicalPlatform},
};
use rusqlite::params;

// The product build mutex also serializes these fixtures' SQLite allocator budget
// observations; other test modules may conservatively consume budget headroom.
static TEST: Mutex<()> = Mutex::new(());
struct Fixture {
    base: PathBuf,
    db: Database,
    workspace: PathBuf,
    _cleanup: Cleanup,
}
struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn budget() -> BuildBudget {
    BuildBudget {
        peak_ram_bytes: 512 * 1024 * 1024,
        private_bytes: 32 * 1024 * 1024,
        staging_bytes: 4 * 1024 * 1024,
        main_read_age: Duration::from_secs(5),
        main_wal_bytes: 16 * 1024 * 1024,
        work_age: Duration::from_secs(30),
    }
}
fn fixture() -> Fixture {
    let base = std::env::temp_dir().join(format!("zen-shadow-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&base).unwrap();
    let db = Database::open(base.join("main.sqlite")).unwrap();
    let workspace = base.join("shadow");
    fs::create_dir(&workspace).unwrap();
    let f = Fixture {
        _cleanup: Cleanup(base.clone()),
        base,
        db,
        workspace,
    };
    let conn = f.db.conn().unwrap();
    conn.execute_batch("INSERT INTO scan_roots(id,normalized_path,display_name,source_kind,enabled,health_status,current_generation,last_successful_generation,last_full_scan_at,needs_reconciliation,created_at,updated_at)
        VALUES('a','/managed','A','file_library',1,'healthy',1,1,20,0,1,20);
        INSERT INTO scan_sessions(id,status,phase,created_at,updated_at) VALUES('session','completed','completed',1,20);
        INSERT INTO scan_session_roots(session_id,requested_index,requested_path,normalized_requested_path,resolution,effective_root_id,effective_path,effective_index,status,created_at,updated_at)
        VALUES('session',0,'/managed','/managed','effective','a','/managed',0,'completed',1,20);
        INSERT INTO scan_runs(id,scan_root_id,generation,parent_session_id,lease_token,status,phase,coverage_complete,stale_reconciliation_allowed,finished_at,created_at,updated_at)
        VALUES('run-a','a',1,'session','lease-a','completed','completed',1,1,20,1,20);") .unwrap();
    conn.execute_batch("UPDATE scan_roots SET revision=10; UPDATE scan_runs SET started_at=1")
        .unwrap();
    drop(conn);
    // Real schema + genuine SourceUID writer helper, not shadow-model records.
    insert_source(&f, "one", "/managed/sub/report.txt", 100, "report.txt");
    insert_source(
        &f,
        "two",
        "/managed/sub/notes-report.txt",
        200,
        "notes-report.txt",
    );
    let conn = f.db.conn().unwrap();
    conn.execute_batch("INSERT INTO user_tags(id,display_name,normalized_name,created_at,updated_at) VALUES('tag','Pinned','pinned',1,1);
        INSERT INTO file_user_tags(file_id,tag_id,created_at) VALUES('one','tag',1);") .unwrap();
    drop(conn);
    f
}
fn insert_source(f: &Fixture, id: &str, path: &str, created: i64, name: &str) {
    let req = InsertFileRequest {
        id: id.into(),
        path: path.into(),
        name: name.into(),
        extension: "txt".into(),
        size: 10,
        mtime: 10,
        ctime: 1,
        is_dir: false,
        state_code: 0,
    };
    // Identity fixture is explicitly native-shaped input to the actual writer.
    let physical = PhysicalFileIdentity {
        platform_kind: PhysicalPlatform::Windows,
        platform_volume_id: Some("volume".into()),
        platform_file_id: Some(format!("object-{created}")),
        created_ns: Some(created),
        size: 10,
        modified_ns: Some(10),
        physical_key: Some("proof".into()),
        link_count: Some(1),
    };
    let mut conn = f.db.conn().unwrap();
    let tx = conn.transaction().unwrap();
    let epoch = advance_root_epoch_tx(&tx, "a", "scan", 1, 10).unwrap();
    observe_source_for_root_tx(
        &tx,
        &req,
        Some(&physical),
        SourceObservationContext {
            root_id: "a",
            root_epoch: epoch,
            owner_kind: "scan",
            owner_revision: 1,
            legacy_row_exists: false,
            observed_at: 10,
        },
    )
    .unwrap();
    tx.execute("INSERT INTO files(id,path,name,extension,size,mtime,ctime,is_dir) VALUES(?1,?2,?3,'txt',10,10,1,0)",params![id,path,name]).unwrap();
    tx.execute(
        "INSERT INTO scan_seen(run_id,file_id,observed_path,observed_at) VALUES('run-a',?1,?2,10)",
        params![id, path],
    )
    .unwrap();
    tx.commit().unwrap();
}
fn reader(f: &Fixture) -> SnapshotReader<'_> {
    SnapshotReader {
        database: f.db.path(),
        scheduler: WorkScheduler::new(Default::default()).into(),
    }
}
fn build(f: &Fixture) -> Candidate {
    prepare(reader(f), &f.workspace, budget(), CancellationToken::new()).unwrap_or_else(|e| {
        let conn=f.db.conn().unwrap();
        for sql in ["SELECT json_group_array(json_object('health',health_status,'enabled',enabled,'generation',last_successful_generation,'active',active_run_id,'needs',needs_reconciliation,'watcher',watcher_revision,'applied',watcher_applied_revision)) FROM scan_roots", "SELECT json_group_array(json_object('status',status,'phase',phase,'complete',coverage_complete,'stale',stale_reconciliation_allowed,'watcher_start',watcher_revision_at_start)) FROM scan_runs", "SELECT json_group_array(json_object('state',claim_state,'epoch',root_epoch,'owner',owner_kind,'revision',owner_revision,'observation',observation_revision)) FROM library_source_root_claims"] {
            eprintln!("coverage fixture: {}",conn.query_row(sql,[],|r|r.get::<_,String>(0)).unwrap());
        }
        panic!("builder: {e}")
    })
}
fn spec() -> FileQuerySpecV2 {
    FileQuerySpecV2 {
        scope: FileLibraryScopeV2::AllEnabledRoots,
        text: Some("report".into()),
        filters: Default::default(),
        sort: FileLibrarySortV2 {
            kind: LibrarySortKind::Relevance,
            direction: LibrarySortDirection::Asc,
        },
    }
}
fn fails<T>(r: Result<T, DbError>, reason: &str) {
    match r {
        Ok(_) => panic!("expected {reason}"),
        Err(e) => assert!(e.to_string().contains(reason), "{e}"),
    }
}

#[test]
fn complete_real_sqlite_candidate_and_main_zero_write() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let conn = f.db.conn().unwrap();
    let before: i64 = conn
        .query_row("PRAGMA data_version", [], |r| r.get(0))
        .unwrap();
    let candidate = build(&f);
    let verified = verify(&candidate).unwrap();
    let shadow = open_shadow(&verified).unwrap();
    let page = shadow.query(spec(), 1, None).unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.rows.len(), 1);
    let next = shadow
        .query(spec(), 1, page.next_cursor.as_deref())
        .unwrap();
    assert_eq!(next.rows.len(), 1);
    assert_ne!(page.rows[0].file_id, next.rows[0].file_id);
    assert_eq!(
        conn.query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
    let authority:(String,i64,Option<String>,i64)=conn.query_row("SELECT read_mode,active_catalog_epoch,active_fts_slot_id,revision FROM library_generation_authority",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(authority, ("legacy".into(), 0, None, 1));
    snapshot::qualify_current(&candidate, reader(&f), budget(), CancellationToken::new()).unwrap();
    eprintln!(
        "F1-2 positive: effective_rows=2 index_bytes={} total_private_bytes={} main_writes=0",
        fs::metadata(candidate.directory.join("index.sqlite"))
            .unwrap()
            .len(),
        disk_bytes(&f.workspace).unwrap()
    );
    eprintln!(
        "F1-2 sampled resource maxima [private,sqlite,charged_ram,main_wal,read_us]={:?}",
        candidate.usage.sample()
    );
    drop(shadow);
    retire_candidate(&verified).unwrap();
    assert!(!candidate.directory.exists());
}

#[test]
fn coverage_checks_are_bidirectional_and_do_not_require_equal_batch_epochs() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    for sql in [
        "UPDATE scan_roots SET health_status='degraded'",
        "UPDATE scan_roots SET watcher_revision=1",
        "UPDATE scan_roots SET active_run_id='run-a',active_generation=1",
        "UPDATE scan_runs SET status='running'",
        "UPDATE library_source_root_claims SET root_epoch=999",
        "UPDATE library_source_root_claims SET owner_revision=999",
        "UPDATE scan_seen SET observed_at=999",
        "UPDATE scan_runs SET started_at=19",
        "UPDATE library_source_root_claims SET owner_kind='operation'",
        "UPDATE library_source_observations SET physical_claim_digest='wrong'",
        "UPDATE library_source_observations SET observation_digest='wrong'",
        "UPDATE library_source_root_claims SET claim_state='absent' WHERE observed_path='/managed/sub/report.txt'",
        "UPDATE files SET is_stale=1 WHERE id='one'",
        "INSERT INTO files(id,path,name) VALUES('unassigned','/managed/unassigned.txt','unassigned.txt')",
    ] {
        let f=fixture();f.db.conn().unwrap().execute_batch(sql).unwrap();
        fails(prepare(reader(&f),&f.workspace,budget(),CancellationToken::new()),"coverage_unproven");
    }
    let f = fixture();
    let c = build(&f);
    verify(&c).unwrap(); // claims have epochs 1 and 2; root epoch=2.
}

#[test]
fn multi_root_same_observation_merges_but_multipath_is_unavailable() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    f.db.conn().unwrap().execute_batch("INSERT INTO scan_roots(id,normalized_path,display_name,source_kind,enabled,health_status,current_generation,last_successful_generation,last_full_scan_at,needs_reconciliation,created_at,updated_at)
        VALUES('b','/managed/sub','B','file_library',1,'healthy',1,1,20,0,1,20);
        INSERT INTO scan_runs(id,scan_root_id,generation,lease_token,status,phase,coverage_complete,stale_reconciliation_allowed,finished_at,created_at,updated_at)
        VALUES('run-b','b',1,'lease-b','completed','completed',1,1,20,1,20); UPDATE scan_roots SET revision=10 WHERE id='b'; UPDATE scan_runs SET started_at=1 WHERE id='run-b';
        INSERT INTO library_root_epoch_state SELECT 'b',epoch,last_owner_kind,last_owner_revision,updated_at FROM library_root_epoch_state WHERE root_id='a';
        INSERT INTO library_source_root_claims SELECT 'b',source_uid,observed_path,root_epoch,owner_kind,owner_revision,observation_revision,claim_state,observed_at FROM library_source_root_claims WHERE root_id='a';
        INSERT INTO scan_seen SELECT 'run-b',file_id,observed_path,observed_at FROM scan_seen WHERE run_id='run-a';").unwrap();
    let c = build(&f);
    let v = verify(&c).unwrap();
    let s = open_shadow(&v).unwrap();
    assert_eq!(s.query(spec(), 20, None).unwrap().total, 2);
    // A absent and its completed scan no longer sees that path; B still present.
    f.db.conn().unwrap().execute_batch("UPDATE library_source_root_claims SET claim_state='absent' WHERE root_id='a'; DELETE FROM scan_seen WHERE run_id='run-a';").unwrap();
    let fresh = f.base.join("next");
    fs::create_dir(&fresh).unwrap();
    let c = prepare(reader(&f), &fresh, budget(), CancellationToken::new()).unwrap();
    verify(&c).unwrap();
    f.db.conn().unwrap().execute_batch("UPDATE library_source_root_claims SET observed_path='/managed/hardlink.txt' WHERE root_id='b' AND observed_path='/managed/sub/report.txt'").unwrap();
    let fresh = f.base.join("bad");
    fs::create_dir(&fresh).unwrap();
    fails(
        prepare(reader(&f), &fresh, budget(), CancellationToken::new()),
        "coverage_unproven",
    );
}

#[test]
fn e0_cursor_and_tags_never_observe_e1_replacement() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let c0 = build(&f);
    let v0 = verify(&c0).unwrap();
    let s0 = open_shadow(&v0).unwrap();
    let first = s0.query(spec(), 1, None).unwrap();
    let conn = f.db.conn().unwrap();
    conn.execute_batch("UPDATE files SET path='/archive/old.txt',is_stale=1 WHERE id='one'; DELETE FROM scan_seen WHERE file_id='one'; UPDATE library_source_root_claims SET claim_state='absent' WHERE observed_path='/managed/sub/report.txt';").unwrap();
    drop(conn);
    insert_source(
        &f,
        "replacement",
        "/managed/sub/report.txt",
        300,
        "report.txt",
    );
    let workspace = f.base.join("e1");
    fs::create_dir(&workspace).unwrap();
    let c1 = prepare(reader(&f), &workspace, budget(), CancellationToken::new()).unwrap();
    let v1 = verify(&c1).unwrap();
    let s1 = open_shadow(&v1).unwrap();
    fails(
        s1.query(spec(), 1, first.next_cursor.as_deref()),
        "snapshot_expired",
    );
    let old = s0.query(spec(), 20, None).unwrap();
    assert!(old
        .rows
        .iter()
        .any(|r| r.file_id == "one" && r.tags == vec!["tag"]));
    assert!(!old.rows.iter().any(|r| r.file_id == "replacement"));
    let new = s1.query(spec(), 20, None).unwrap();
    assert!(new
        .rows
        .iter()
        .any(|r| r.file_id == "replacement" && r.tags.is_empty()));
    fails(
        snapshot::qualify_current(&c0, reader(&f), budget(), CancellationToken::new()),
        "stale",
    );
}

#[test]
fn real_manifest_tamper_missing_replay_and_restart_fail_closed() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let c = build(&f);
    verify(&c).unwrap();
    let path = c.directory.join("seal.json");
    let original = fs::read(&path).unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(&original).unwrap();
    json["format"] = 99.into();
    fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    fails(verify(&c), "format_or_slot_mismatch");
    fs::write(&path, &original).unwrap();
    let bytes = fs::read(c.directory.join("index.sqlite")).unwrap();
    fs::write(c.directory.join("index.sqlite"), b"damaged").unwrap();
    fails(verify(&c), "database_digest_mismatch");
    fs::write(c.directory.join("index.sqlite"), &bytes).unwrap();
    let other = f.base.join("other");
    fs::create_dir(&other).unwrap();
    let c2 = prepare(reader(&f), &other, budget(), CancellationToken::new()).unwrap();
    fs::write(c2.directory.join("seal.json"), &original).unwrap();
    fails(verify(&c2), "format_or_slot_mismatch");
    fs::remove_file(c.directory.join("index.sqlite")).unwrap();
    assert!(verify(&c).is_err());
    for state_name in [
        "STAGING",
        "CAPTURED",
        "BUILDING",
        "SEALED",
        "VERIFIED_SHADOW",
        "UNAVAILABLE",
    ] {
        state(&c, state_name);
        fails(
            recover_candidate(c.path()),
            "recovery_requires_fresh_capture",
        );
    }
}
fn state(c: &Candidate, s: &str) {
    slot::state(c, s).unwrap();
}

#[test]
fn query_weights_filters_selection_and_cursor_tampering() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let c = build(&f);
    let v = verify(&c).unwrap();
    let s = open_shadow(&v).unwrap();
    let all = s.query(spec(), 20, None).unwrap();
    for row in &all.rows {
        let expected:f64=s.conn.query_row("SELECT bm25(files_fts,6.0,1.5) FROM files_fts WHERE files_fts MATCH 'report' AND rowid=(SELECT rowid FROM files WHERE id=?1)",[&row.file_id],|r|r.get(0)).unwrap();
        assert_eq!(row.rank.unwrap().to_bits(), expected.to_bits());
    }
    let mut tagged = spec();
    tagged.filters.tags_all_of = vec!["tag".into()];
    let page = s.query(tagged, 1, None).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.rows[0].file_id, "one");
    let mut current = spec();
    current.scope = FileLibraryScopeV2::CurrentScan {
        scan_session_id: "session".into(),
    };
    assert_eq!(s.query(current, 10, None).unwrap().total, 2);
    let selected = s.select_matching(spec(), &["one".into()]).unwrap();
    assert_eq!(selected.file_ids, vec!["two"]);
    assert_eq!(selected.excluded_count, 1);
    assert_eq!(
        s.select_explicit(&["one".into(), "one".into(), "missing".into()])
            .unwrap()
            .file_ids,
        vec!["one"]
    );
    for kind in [
        LibrarySortKind::Relevance,
        LibrarySortKind::Name,
        LibrarySortKind::Size,
        LibrarySortKind::Modified,
        LibrarySortKind::Created,
        LibrarySortKind::Confidence,
    ] {
        for direction in [LibrarySortDirection::Asc, LibrarySortDirection::Desc] {
            let mut spec = spec();
            spec.sort = FileLibrarySortV2 {
                kind: kind.clone(),
                direction,
            };
            let page = s.query(spec.clone(), 1, None).unwrap();
            let second = s
                .query(spec.clone(), 1, page.next_cursor.as_deref())
                .unwrap();
            assert_ne!(page.rows[0].file_id, second.rows[0].file_id);
            let mut forged: serde_json::Value =
                serde_json::from_str(page.next_cursor.as_ref().unwrap()).unwrap();
            forged["tuple"]["total_count"] = 123.into();
            fails(
                s.query(spec, 1, Some(&forged.to_string())),
                "snapshot_expired",
            );
        }
    }
}

#[test]
fn selection_exclusions_preserve_order_counts_and_explicit_deduplication() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let c = build(&f);
    let v = verify(&c).unwrap();
    let s = open_shadow(&v).unwrap();
    for direction in [LibrarySortDirection::Asc, LibrarySortDirection::Desc] {
        let mut query = spec();
        query.sort = FileLibrarySortV2 {
            kind: LibrarySortKind::Name,
            direction,
        };
        let expected = s
            .query(query.clone(), 20, None)
            .unwrap()
            .rows
            .into_iter()
            .map(|r| r.file_id)
            .collect::<Vec<_>>();
        for exclusions in [
            vec![],
            vec!["one".into(), "one".into(), "missing".into()],
            vec!["two".into(), "one".into(), "two".into()],
        ] {
            let selected = s.select_matching(query.clone(), &exclusions).unwrap();
            // Original linear semantics are a small deterministic oracle.
            let retained = expected
                .iter()
                .filter(|id| !exclusions.contains(id))
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(selected.excluded_count, expected.len() - retained.len());
            assert_eq!(selected.file_ids, retained);
            assert_eq!(selected.missing_count, 0);
        }
    }
    let explicit = s
        .select_explicit(&[
            "two".into(),
            "missing".into(),
            "one".into(),
            "two".into(),
            "missing".into(),
        ])
        .unwrap();
    assert_eq!(explicit.file_ids, vec!["one", "two"]);
    assert_eq!(explicit.missing_count, 1);
    assert_eq!(explicit.excluded_count, 0);
    let empty = s.select_explicit(&[]).unwrap();
    assert!(empty.file_ids.is_empty());
    assert_eq!((empty.missing_count, empty.excluded_count), (0, 0));
    let mut no_matches = spec();
    no_matches.text = Some("absent".into());
    let empty = s.select_matching(no_matches, &["one".into()]).unwrap();
    assert!(empty.file_ids.is_empty());
    assert_eq!((empty.missing_count, empty.excluded_count), (0, 0));
}

#[test]
fn selection_input_limits_and_cancelled_or_expired_work_fail_closed() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let c = build(&f);
    let v = verify(&c).unwrap();
    let s = open_shadow(&v).unwrap();
    // Boundary inputs reuse one ID against two rows, not a large corpus or
    // timing benchmark. The raw input cap must apply before deduplication.
    let at_limit = vec!["one".to_string(); 100000];
    let selected = s.select_matching(spec(), &at_limit).unwrap();
    assert_eq!(selected.file_ids, vec!["two"]);
    assert_eq!(selected.excluded_count, 1);
    assert_eq!(s.select_explicit(&at_limit).unwrap().file_ids, vec!["one"]);
    let mut oversized = at_limit;
    oversized.push("one".into());
    fails(s.select_matching(spec(), &oversized), "selection_limit");
    fails(s.select_explicit(&oversized), "selection_limit");

    let token = CancellationToken::new();
    let guard = Guard::new(&f.workspace, f.db.path(), budget(), token.clone());
    token.cancel();
    for ids in [vec![], vec!["one".into(), "one".into()]] {
        fails(s.select_matching_guarded(spec(), &ids, &guard), "cancelled");
        fails(s.select_explicit_guarded(&ids, &guard), "cancelled");
    }
    let mut expired_budget = budget();
    expired_budget.work_age = Duration::ZERO;
    let guard = Guard::new(
        &f.workspace,
        f.db.path(),
        expired_budget,
        CancellationToken::new(),
    );
    fails(
        s.select_matching_guarded(spec(), &["one".into()], &guard),
        "work_deadline",
    );
    fails(
        s.select_explicit_guarded(&["one".into()], &guard),
        "work_deadline",
    );
}

#[test]
fn cancellation_budgets_and_retirement_leave_truthful_retained_state() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let cancel = CancellationToken::new();
    cancel.cancel();
    fails(
        prepare(reader(&f), &f.workspace, budget(), cancel),
        "cancelled",
    );
    let mut small = budget();
    small.staging_bytes = 1;
    fails(
        prepare(reader(&f), &f.workspace, small, CancellationToken::new()),
        "staging_budget",
    );
    let next = f.base.join("tiny");
    fs::create_dir(&next).unwrap();
    let mut small = budget();
    small.private_bytes = 1;
    fails(
        prepare(reader(&f), &next, small, CancellationToken::new()),
        "disk_budget",
    );
    let next = f.base.join("clock");
    fs::create_dir(&next).unwrap();
    let mut small = budget();
    small.main_read_age = Duration::from_nanos(1);
    fails(
        prepare(reader(&f), &next, small, CancellationToken::new()),
        "read_deadline",
    );
    let next = f.base.join("wal");
    fs::create_dir(&next).unwrap();
    let mut small = budget();
    small.main_wal_bytes = 0;
    fails(
        prepare(reader(&f), &next, small, CancellationToken::new()),
        "wal_exposure",
    );
    let next = f.base.join("valid");
    fs::create_dir(&next).unwrap();
    let c = prepare(reader(&f), &next, budget(), CancellationToken::new()).unwrap();
    let v = verify(&c).unwrap();
    let s = open_shadow(&v).unwrap();
    let second_registration = verify(&c).unwrap();
    fails(
        retire_candidate(&second_registration),
        "retained_open_handles",
    );
    fails(open_shadow(&v), "snapshot_expired");
    fails(verify(&c), "snapshot_expired");
    assert!(c.directory.exists());
    assert_eq!(s.query(spec(), 10, None).unwrap().total, 2);
    drop(s);
    retire_candidate(&v).unwrap();
    assert!(!c.directory.exists());
}

#[test]
fn compiled_module_has_no_runtime_entry_callers() {
    let source = include_str!("../mod.rs");
    assert_eq!(source.matches("pub(crate) mod library_shadow;").count(), 1);
    for source in [
        include_str!("../../../scanner.rs"),
        include_str!("../scan.rs"),
        include_str!("../library/mod.rs"),
        include_str!("../../../lib.rs"),
    ] {
        assert!(!source.contains("library_shadow::"));
    }
}

#[test]
fn actual_managed_scan_lifecycle_builds_a_positive_candidate() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    let base = std::env::temp_dir().join(format!("zen-shadow-native-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&base).unwrap();
    let root = base.join("managed");
    fs::create_dir(&root).unwrap();
    let path = root.join("report.txt");
    fs::write(&path, b"real native metadata fixture").unwrap();
    let db = Database::open(base.join("main.sqlite")).unwrap();
    let workspace = base.join("shadow");
    fs::create_dir(&workspace).unwrap();
    let f = Fixture {
        _cleanup: Cleanup(base.clone()),
        base,
        db,
        workspace,
    };
    let root_text = super::super::scan::normalize_scan_root_path(&root.to_string_lossy());
    let admission =
        f.db.admit_managed_scan(&super::super::scan::ScanAdmissionOptions {
            request: super::super::scan::ManagedScanRequest {
                roots: vec![root_text],
                request_key: Some("shadow-real-scan".into()),
                dedupe: false,
            },
            run_id_override: None,
        })
        .unwrap();
    let mut claimed = f.db.claim_queued_scan_run(&admission.runs[0].id).unwrap();
    for name in ["report.txt", "second-report.txt"] {
        let path = root.join(name);
        fs::write(&path, b"real native metadata fixture").unwrap();
        let metadata = fs::metadata(&path).unwrap();
        let physical = crate::fs_safety::capture_source_object_identity(&path).unwrap();
        assert!(
            physical.created_ns.is_some(),
            "test filesystem must expose birth identity"
        );
        let modified = metadata
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let created = metadata
            .created()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let normalized = super::super::scan::normalize_scan_root_path(&path.to_string_lossy());
        let entry = InsertFileRequest {
            id: normalized.clone(),
            path: normalized,
            name: name.into(),
            extension: "txt".into(),
            size: metadata.len() as i64,
            mtime: modified,
            ctime: created,
            is_dir: false,
            state_code: 0,
        };
        claimed =
            f.db.persist_scan_batch(
                &claimed.dto.id,
                claimed.dto.revision,
                claimed.root_revision,
                claimed.session_revision,
                &super::super::scan::ScanBatchInput {
                    entries: &[entry],
                    errors: &[],
                    scanned_files: 1,
                    scanned_directories: 0,
                    processed_bytes: metadata.len() as i64,
                    warnings: 0,
                },
            )
            .unwrap();
    }
    let persisted = claimed;
    let reconciled =
        f.db.reconcile_missing(
            &persisted.dto.id,
            persisted.dto.revision,
            persisted.root_revision,
            persisted.session_revision,
        )
        .unwrap();
    f.db.finalize_scan_run(
        &reconciled.dto.id,
        reconciled.dto.revision,
        reconciled.root_revision,
        reconciled.session_revision,
        &super::super::scan::ScanFinalizeInput {
            terminal_status: "completed".into(),
            error_code: None,
            error_message: None,
            allow_stale_reconciliation: true,
            rule_recovery_succeeded: false,
        },
    )
    .unwrap();
    let c = build(&f);
    let v = verify(&c).unwrap();
    let s = open_shadow(&v).unwrap();
    let page = s.query(spec(), 10, None).unwrap();
    assert_eq!(page.total, 2);
    assert!(page.rows.iter().any(|r| r.name == "report.txt"));
    eprintln!(
        "F1-2 actual Managed Scan: platform={} rows=2 batches=2 index_bytes={}",
        crate::fs_safety::capture_source_object_identity(&path)
            .unwrap()
            .platform_kind
            .as_str(),
        fs::metadata(c.directory.join("index.sqlite"))
            .unwrap()
            .len()
    );
}

#[test]
fn cancellation_at_each_persisted_build_barrier_never_creates_a_reader() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    for barrier in ["STAGING", "CAPTURED", "BUILDING", "SEALED"] {
        let f = fixture();
        let cancel = CancellationToken::new();
        slot::STOP_AT.with(|h| *h.borrow_mut() = Some((barrier.into(), cancel.clone())));
        let result = prepare(reader(&f), &f.workspace, budget(), cancel);
        slot::STOP_AT.with(|h| *h.borrow_mut() = None);
        assert!(result.is_err(), "{barrier} cancellation must fail closed");
        let directories = fs::read_dir(&f.workspace)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(directories.len(), 1);
        assert_eq!(
            fs::read_to_string(directories[0].path().join("state")).unwrap(),
            "UNAVAILABLE"
        );
        fails(
            recover_candidate(&directories[0].path()),
            "recovery_requires_fresh_capture",
        );
    }
}

#[test]
fn crash_worker() {
    let Ok(database) = std::env::var("ZEN_SHADOW_CRASH_DATABASE") else {
        return;
    };
    let workspace = std::env::var("ZEN_SHADOW_CRASH_WORKSPACE").unwrap();
    let result = prepare(
        SnapshotReader {
            database: Path::new(&database),
            scheduler: WorkScheduler::new(Default::default()).into(),
        },
        Path::new(&workspace),
        budget(),
        CancellationToken::new(),
    );
    if let Ok(candidate) = result {
        let _ = verify(&candidate);
    }
    panic!("expected crash barrier");
}
#[test]
fn process_exit_at_each_persisted_barrier_is_unavailable_on_restart() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    for barrier in [
        "STAGING",
        "CAPTURED",
        "BUILDING",
        "SEALED",
        "VERIFIED_SHADOW",
    ] {
        let workspace = f.base.join(format!("crash-{barrier}"));
        fs::create_dir(&workspace).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "db::queries::library_shadow::tests::crash_worker",
                "--nocapture",
            ])
            .env("ZEN_SHADOW_CRASH_DATABASE", f.db.path())
            .env("ZEN_SHADOW_CRASH_WORKSPACE", &workspace)
            .env("ZEN_SHADOW_CRASH_BARRIER", barrier)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(91));
        let directory = fs::read_dir(workspace)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            fs::read_to_string(directory.join("state")).unwrap(),
            barrier
        );
        fails(
            recover_candidate(&directory),
            "recovery_requires_fresh_capture",
        );
    }
}

#[test]
fn consistent_hardlink_observations_are_rejected_instead_of_picking_latest() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    f.db.conn().unwrap().execute_batch("INSERT INTO library_source_observations
        SELECT (SELECT source_uid FROM library_source_root_claims WHERE observed_path='/managed/sub/report.txt'),2,legacy_file_id_snapshot,observed_path,name,extension,size,mtime,ctime,is_dir,
          (SELECT verification_digest FROM library_source_incarnations WHERE source_uid=(SELECT source_uid FROM library_source_root_claims WHERE observed_path='/managed/sub/report.txt')),observation_digest,observed_at
        FROM library_source_observations WHERE observed_path='/managed/sub/notes-report.txt';
        UPDATE library_source_root_claims SET source_uid=(SELECT source_uid FROM library_source_root_claims WHERE observed_path='/managed/sub/report.txt'),observation_revision=2 WHERE observed_path='/managed/sub/notes-report.txt';").unwrap();
    fails(
        prepare(reader(&f), &f.workspace, budget(), CancellationToken::new()),
        "ambiguous_projection",
    );
}

#[test]
fn selected_root_precedes_limit_and_preserves_global_bm25_statistics() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    insert_source(
        &f,
        "three",
        "/managed/other/report-report.txt",
        300,
        "report-report.txt",
    );
    f.db.conn().unwrap().execute_batch("INSERT INTO scan_roots(id,normalized_path,display_name,source_kind,enabled,health_status,current_generation,last_successful_generation,last_full_scan_at,needs_reconciliation,created_at,updated_at)
        VALUES('b','/managed/other','B','file_library',1,'healthy',1,1,20,0,1,20);
        INSERT INTO scan_runs(id,scan_root_id,generation,lease_token,status,phase,coverage_complete,stale_reconciliation_allowed,finished_at,created_at,updated_at)
        VALUES('run-b','b',1,'lease-b','completed','completed',1,1,20,1,20); UPDATE scan_roots SET revision=10 WHERE id='b'; UPDATE scan_runs SET started_at=1 WHERE id='run-b';
        INSERT INTO library_root_epoch_state SELECT 'b',epoch,last_owner_kind,last_owner_revision,updated_at FROM library_root_epoch_state WHERE root_id='a';
        INSERT INTO library_source_root_claims SELECT 'b',source_uid,observed_path,root_epoch,owner_kind,owner_revision,observation_revision,claim_state,observed_at FROM library_source_root_claims WHERE observed_path='/managed/other/report-report.txt';
        INSERT INTO scan_seen SELECT 'run-b',file_id,observed_path,observed_at FROM scan_seen WHERE file_id='three';").unwrap();
    let c = build(&f);
    let v = verify(&c).unwrap();
    let s = open_shadow(&v).unwrap();
    let all = s.query(spec(), 20, None).unwrap();
    let mut scope = spec();
    scope.sort = FileLibrarySortV2 {
        kind: LibrarySortKind::Modified,
        direction: LibrarySortDirection::Asc,
    };
    assert_eq!(
        s.query(scope.clone(), 1, None).unwrap().rows[0].file_id,
        "one"
    );
    scope.scope = FileLibraryScopeV2::Roots {
        scan_root_ids: vec!["b".into()],
    };
    let selected = s.query(scope, 1, None).unwrap();
    assert_eq!(selected.total, 1);
    assert_eq!(selected.rows[0].file_id, "three");
    let global = all.rows.iter().find(|r| r.file_id == "three").unwrap();
    assert_eq!(
        selected.rows[0].rank.unwrap().to_bits(),
        global.rank.unwrap().to_bits()
    );
}

#[test]
fn verifier_checks_actual_fts_postings_beyond_the_seal_hash() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let mut c = build(&f);
    let path = c.directory.join("index.sqlite");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "DELETE FROM files_fts WHERE rowid=(SELECT rowid FROM files WHERE id='one')",
        [],
    )
    .unwrap();
    conn.close().unwrap();
    // Exercise the independent semantic verifier after explicitly replacing the
    // expected physical hash in this test. Production callers cannot mint seals.
    let guard = Guard::new(
        &f.workspace,
        f.db.path(),
        budget(),
        CancellationToken::new(),
    );
    let seal = c.seal.as_mut().unwrap();
    seal.database_digest = slot::file_digest(&path, &guard).unwrap();
    seal.database_bytes = fs::metadata(&path).unwrap().len();
    fs::write(
        c.directory.join("seal.json"),
        serde_json::to_vec(seal).unwrap(),
    )
    .unwrap();
    fails(verify(&c), "fts_corpus_mismatch");
}

#[test]
fn power_policy_defers_before_io_and_charged_ram_budget_fails_closed() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let governor = crate::resource_governor::RuntimeResourceGovernor::from_snapshot(
        crate::resource_governor::ResourcePressureSnapshot::Windows(
            crate::resource_governor::WindowsPowerSnapshot {
                battery_saver: Some(true),
                on_battery: Some(true),
            },
        ),
    );
    let scheduler = WorkScheduler::new(
        crate::scheduler::SchedulerConfig::default().with_policy(Arc::new(governor)),
    );
    fails(
        prepare(
            SnapshotReader {
                database: f.db.path(),
                scheduler: Arc::new(scheduler),
            },
            &f.workspace,
            budget(),
            CancellationToken::new(),
        ),
        "resource_deferred",
    );
    assert!(fs::read_dir(&f.workspace).unwrap().next().is_none());
    f.db.conn()
        .unwrap()
        .execute(
            "UPDATE files SET context=CAST(zeroblob(200000) AS TEXT) WHERE id='one'",
            [],
        )
        .unwrap();
    let mut small = budget();
    small.peak_ram_bytes = 9 * 1024 * 1024;
    small.staging_bytes = 32 * 1024 * 1024;
    fails(
        prepare(reader(&f), &f.workspace, small, CancellationToken::new()),
        "ram_budget",
    );
}

// Observe all persisted main/WAL bytes as well as the captured source facts.
fn main_bytes(f: &Fixture) -> (Vec<u8>, Option<Vec<u8>>) {
    let mut wal = f.db.path().as_os_str().to_os_string();
    wal.push("-wal");
    let wal = PathBuf::from(wal);
    (
        fs::read(f.db.path()).unwrap(),
        wal.exists().then(|| fs::read(wal).unwrap()),
    )
}

#[test]
fn schema41_through_schema45_build_verify_query_and_qualify_without_main_writes() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    for version in [41, 42, 43, 44, 45, 46] {
        let f = fixture();
        let conn = f.db.conn().unwrap();
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            i64::from(crate::db::SUPPORTED_SCHEMA_VERSION)
        );
        if version == 41 {
            crate::db::schema_global_generations::drop_for_test(&conn);
        }
        conn.execute_batch(&format!("PRAGMA user_version={version}"))
            .unwrap();
        let before_version: i64 = conn
            .query_row("PRAGMA data_version", [], |r| r.get(0))
            .unwrap();
        let before_bytes = main_bytes(&f);
        let guard = Guard::new(
            &f.workspace,
            f.db.path(),
            budget(),
            CancellationToken::new(),
        );
        let before = snapshot::capture(f.db.path(), &guard).unwrap();
        let candidate = build(&f);
        let verified = verify(&candidate).unwrap();
        let shadow = open_shadow(&verified).unwrap();
        let page = shadow.query(spec(), 20, None).unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.rows.len(), 2);
        let tagged = page.rows.iter().find(|r| r.file_id == "one").unwrap();
        assert_eq!(tagged.tags, vec!["tag".to_string()]);
        assert!(!tagged.source_uid.is_empty());
        snapshot::qualify_current(&candidate, reader(&f), budget(), CancellationToken::new())
            .unwrap();
        assert_eq!(snapshot::capture(f.db.path(), &guard).unwrap(), before);
        assert_eq!(
            main_bytes(&f),
            before_bytes,
            "Schema{version} main/WAL bytes changed"
        );
        assert_eq!(
            conn.query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            before_version
        );
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
        let authority: (String, i64, Option<String>, i64) = conn.query_row(
            "SELECT read_mode,active_catalog_epoch,active_fts_slot_id,revision FROM library_generation_authority",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        ).unwrap();
        assert_eq!(authority, ("legacy".into(), 0, None, 1));
        drop(shadow);
        retire_candidate(&verified).unwrap();
    }
}

fn assert_capture_rejected_without_main_write(f: &Fixture, reason: &str) {
    let before = main_bytes(f);
    fails(
        prepare(reader(f), &f.workspace, budget(), CancellationToken::new()),
        reason,
    );
    assert_eq!(main_bytes(f), before);
    let directories = fs::read_dir(&f.workspace)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(directories.len(), 1);
    let directory = directories[0].path();
    assert_eq!(
        fs::read_to_string(directory.join("state")).unwrap(),
        "UNAVAILABLE"
    );
    assert!(!directory.join("index.sqlite").exists());
    fails(
        recover_candidate(&directory),
        "recovery_requires_fresh_capture",
    );
}

#[test]
fn schema40_and_schema47_sources_are_rejected_before_shadow_build() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    for version in [40, 47] {
        let f = fixture();
        let conn = f.db.conn().unwrap();
        crate::db::schema_global_generations::drop_for_test(&conn);
        if version == 40 {
            conn.execute_batch("DROP INDEX idx_library_source_physical_lookup")
                .unwrap();
        }
        conn.execute_batch(&format!("PRAGMA user_version={version}"))
            .unwrap();
        assert_capture_rejected_without_main_write(&f, "source_schema_unsupported");
    }
}

#[test]
fn malformed_schema42_namespace_is_rejected_before_shadow_build() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    for sql in [
        "DROP TRIGGER global_index_generation_entries_no_update",
        "DROP TABLE global_index_generation_fts",
        "DROP TRIGGER global_index_generation_entries_no_update; CREATE TRIGGER global_index_generation_entries_no_update BEFORE UPDATE ON global_index_generation_entries BEGIN SELECT 1; END",
        "CREATE TABLE global_index_generation_forged(value TEXT)",
    ] {
        let f = fixture();
        f.db.conn().unwrap().execute_batch(sql).unwrap();
        assert_capture_rejected_without_main_write(&f, "global_index_schema42_generation_signature_conflict");
    }
}

#[test]
fn schema42_current_qualification_rejects_signature_drift_without_main_writes() {
    let _test = TEST.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let candidate = build(&f);
    verify(&candidate).unwrap();
    let conn = f.db.conn().unwrap();
    conn.execute_batch("DROP TRIGGER global_index_generation_entries_no_delete")
        .unwrap();
    let before = main_bytes(&f);
    fails(
        snapshot::qualify_current(&candidate, reader(&f), budget(), CancellationToken::new()),
        "global_index_schema42_generation_signature_conflict",
    );
    assert_eq!(main_bytes(&f), before);
}
