mod corpus;
mod db;
mod lifecycle;
mod query;
use corpus::*;
use db::*;
use query::*;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::Path, time::Instant};

pub fn checkpoint(point: &str) {
    use std::io::Write;
    println!("{}", json!({"checkpoint":point}));
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    panic!("parent must kill child");
}
fn environment(c: &Connection) -> Value {
    let opts: Vec<String> = c
        .prepare("PRAGMA compile_options")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let sqlite: String = c
        .query_row("SELECT sqlite_version()", [], |r| r.get(0))
        .unwrap();
    let source: String = c
        .query_row("SELECT sqlite_source_id()", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sqlite, "3.51.3", "product bundled library parity required");
    json!({"sqlite_version":sqlite,"source_id":source,"compile_options":opts,"fts5":true,"trigram":true,"peak_memory_proc_status":std::fs::read_to_string("/proc/self/status").unwrap_or_default().lines().filter(|l|l.starts_with("VmHWM:")).collect::<Vec<_>>(),"qualification":"standalone Codex Linux, product-lock/SQLite source feature parity; no Windows/macOS native or application integration qualification"})
}
fn paired(
    main: &Connection,
    shadow: &Connection,
    f: &Fixture,
    case: &str,
    phase: &str,
    small: bool,
) -> Value {
    f2_ready(main).unwrap();
    let ids = f2_ids(main, None);
    let (n, tokens) = f2_stats(main);
    assert_eq!(ids, slot_ids(shadow, None));
    register(main, &ids, n, tokens);
    let mut records = Vec::new();
    let mut f1_ok = true;
    let mut f2_ok = true;
    for q in &f.queries {
        for scope in [None, Some("A"), Some("B"), Some("D")] {
            let allowed = slot_ids(shadow, scope);
            let (one, one_record) = run(shadow, "F1", q, &allowed, None, false);
            let f2_allowed = f2_ids(main, scope);
            assert_eq!(allowed, f2_allowed);
            let (two, two_record) = run(main, "F2", q, &f2_allowed, None, small);
            let (zero, zero_record) = run(main, "F0", q, &allowed, None, false);
            f2_ok &= one == two;
            let (oracle, oracle_record) = if small {
                let (r, m) = reference(f, scope, q);
                f1_ok &= r == one;
                f2_ok &= r == two;
                (Some(r), m)
            } else {
                (
                    None,
                    json!({"oracle":"independent Python whole-fixture arithmetic checked after native run; no large rebuilt native oracle"}),
                )
            };
            let pages: Vec<Vec<String>> = one
                .chunks(3)
                .map(|p| p.iter().map(|h| h.uid.clone()).collect())
                .collect();
            assert_eq!(pages.iter().flatten().count(), one.len());
            let like = if q.chars().filter(|ch| !ch.is_whitespace()).count() < 3 {
                let f1_like = legacy_like(shadow, q, &allowed);
                let f2_like = legacy_like(main, q, &f2_allowed);
                assert_eq!(f1_like, f2_like);
                Some(json!({"F1":f1_like,"F2":f2_like,"exact_like_ids_equal":true}))
            } else {
                None
            };
            let zero_ids: BTreeSet<String> = zero.iter().map(|h| h.uid.clone()).collect();
            let correct_ids: BTreeSet<String> = one.iter().map(|h| h.uid.clone()).collect();
            assert_eq!(
                zero_ids, correct_ids,
                "F0 pre-LIMIT filtering must retain correct membership"
            );
            records.push(json!({"case":case,"phase":phase,"query":q,"scope":scope,"corpus_n":n,"corpus_tokens":tokens,"scope_ids":allowed,"F0":zero,"F0_evidence":zero_record,"F1":one,"F1_evidence":one_record,"F2":two,"F2_evidence":two_record,"independent_native_reference":oracle,"reference_evidence":oracle_record,"F0_rank_rejected":zero!=one,"F2_exact_float_and_order_parity":one==two,"pages_of_3":pages,"legacy_short_like_ids":like,"legacy_like_rank":1000000.0,"library_v2_has_like_fallback":false}));
        }
    }
    json!({"case":case,"phase":phase,"F1_native_parity":if small {json!(f1_ok)}else{Value::Null},"F2_exact_parity":f2_ok,"records":records})
}
fn contention(main: &Connection, p: &Prepared, path: &Path, has_files: bool) -> Value {
    use std::{
        ffi::{c_void, CStr},
        sync::mpsc,
        thread,
        time::Duration,
    };
    unsafe extern "C" fn trace(
        _kind: u32,
        data: *mut c_void,
        _stmt: *mut c_void,
        sql: *mut c_void,
    ) -> i32 {
        if unsafe { CStr::from_ptr(sql as *const i8) }.to_bytes() == b"BEGIN IMMEDIATE" {
            let sender = unsafe { &*(data as *const mpsc::Sender<Instant>) };
            let _ = sender.send(Instant::now());
        }
        0
    }
    let mut starts = Vec::new();
    let mut joins = Vec::new();
    let (ready, ready_rx) = mpsc::channel();
    let (attempt, attempt_rx) = mpsc::channel::<Instant>();
    for kind in ["source_metadata", "user_tag"] {
        let dbpath = path.to_path_buf();
        let (rtx, rx) = mpsc::channel();
        starts.push(rtx);
        let ready = ready.clone();
        let attempt = attempt.clone();
        joins.push(thread::spawn(move||{
            let c=Connection::open(dbpath).unwrap();c.busy_timeout(Duration::from_millis(5000)).unwrap();c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
            unsafe{libsqlite3_sys::sqlite3_trace_v2(c.handle(),libsqlite3_sys::SQLITE_TRACE_STMT as u32,Some(trace),&attempt as *const _ as *mut c_void);}
            ready.send(()).unwrap();rx.recv_timeout(Duration::from_secs(10)).unwrap();let t=Instant::now();c.execute_batch("BEGIN IMMEDIATE").unwrap();let acquired=Instant::now();
            if has_files {
                if kind=="source_metadata" {c.execute("UPDATE files SET user_note='committed user correction' WHERE rowid=1",[]).unwrap();c.execute_batch("UPDATE authority SET writer=writer+1").unwrap();}
                else {c.execute("INSERT OR REPLACE INTO tags VALUES((SELECT uid FROM files WHERE rowid=1),'writer-tag')",[]).unwrap();}
            }else{c.execute("UPDATE user_state SET v='committed empty-corpus writer' WHERE k=?",[if kind=="source_metadata"{"note"}else{"tag"}]).unwrap();}
            c.execute_batch("COMMIT").unwrap();let elapsed=t.elapsed().as_secs_f64()*1000.0;assert!(elapsed<5000.0,"unchanged competing writer budget");
            json!({"writer":kind,"begin_wait_ms":acquired.duration_since(t).as_secs_f64()*1000.0,"elapsed_ms":elapsed,"committed":true,"timeout_ms":5000,"attempt_barrier":"actual SQLITE_TRACE_STMT BEGIN IMMEDIATE","rows":"real file note/tag".to_owned()+if has_files{""}else{" unavailable with 0 files; independent persisted empty-corpus user state updated"}})
        }));
    }
    for _ in 0..2 {
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    let publication = publish(
        main,
        p,
        || {
            for start in starts {
                start.send(()).unwrap();
            }
            for _ in 0..2 {
                attempt_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        },
        false,
        false,
    )
    .unwrap();
    assert!(publication["writer_lock_ms"].as_f64().unwrap() < 5000.0);
    json!({"publication":publication,"competing_writers":joins.into_iter().map(|h|h.join().unwrap()).collect::<Vec<_>>()})
}
fn suite(f: &mut Fixture, dir: &Path, case: &str, small: bool) {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("main.sqlite");
    let main = open(&path);
    let physical = create(&main, f);
    lifecycle::setup_leases(&main);
    let first = prepare(&main, dir, 1, false);
    let initial_publish = publish(&main, &first, || {}, false, false).unwrap();
    let pin = lifecycle::Pin::new(&main, dir, "reader-E0", 10);
    let initial = paired(&main, &pin.c, f, case, "E0", small);
    println!(
        "{}",
        json!({"case":case,"seed":f.seed,"event":"initial paired matrix","fixture_population":f.docs.len(),"effective_ids":slot_ids(&pin.c,None),"physical_prepare":physical,"F1_prepare":first.record,"publish":initial_publish,"paired":initial,"environment":environment(&main)})
    );
    if small && f.docs.len() > 8 {
        let native_pin = open(&path);
        native_pin.execute_batch("BEGIN").unwrap();
        let old_authority = authority(&native_pin);
        let old_ids = f2_ids(&native_pin, None);
        let (old_n, old_tokens) = f2_stats(&native_pin);
        register(&native_pin, &old_ids, old_n, old_tokens);
        assert!(main
            .execute("UPDATE epoch_members SET rowid=9999 WHERE epoch=1", [])
            .is_err());
        let ids = slot_ids(&pin.c, None);
        let before = run(&pin.c, "F1", "needle", &ids, None, false).0;
        // Hidden data changes both length and phrase df, including raretoken.
        let hidden = f.docs.iter_mut().find(|d| d.uid == "hidden-9").unwrap();
        hidden.name = "needle needle raretoken ".repeat(100);
        hidden.path = "/private/cache/needle/".repeat(100);
        let changes = main.total_changes();
        main.execute(
            "UPDATE files SET name=?,path=? WHERE uid=?",
            params![hidden.name, hidden.path, hidden.uid],
        )
        .unwrap();
        main.execute_batch("UPDATE authority SET writer=writer+1,text_revision=text_revision+1")
            .unwrap();
        let after = run(&pin.c, "F1", "needle", &ids, None, false).0;
        let new_oracle = reference(f, None, "needle").0;
        assert_eq!(before, after);
        assert_eq!(after, new_oracle);
        let zero = run(&main, "F0", "needle", &ids, None, false).0;
        assert_ne!(zero, after, "hidden corpus negative must be observed");
        let all_ids: BTreeSet<i64> = documents(&main).iter().map(|d| d.rowid).collect();
        let raw = run(&main, "F0", "needle", &all_ids, Some(2), false).0;
        let underfill: Vec<_> = raw.iter().filter(|h| ids.contains(&h.rowid)).collect();
        assert!(underfill.len() < 2);
        println!(
            "{}",
            json!({"case":"B3F-C03","event":"hidden corpus adversary","old_F1":before,"new_F1":after,"visible_native_oracle":new_oracle,"F0_changed_rank":zero,"post_LIMIT_raw":raw,"post_LIMIT_filtered":underfill,"expected_rejection_observed":true,"real_trigger_changes":main.total_changes()-changes,"F2_current_query_failure":f2_ready(&main).unwrap_err(),"F2_pinned_old_transaction":run(&native_pin,"F2","needle",&old_ids,None,false).0})
        );
        let unchanged_user_data = user_data(&main);
        let physical_storage_before = storage(&main);
        let physical_changes = main.total_changes();
        let shadow_changes = pin.c.total_changes();
        let t = Instant::now();
        main.execute(
            "UPDATE files SET is_stale=1-is_stale WHERE uid='hidden-10'",
            [],
        )
        .unwrap();
        assert_eq!(unchanged_user_data, user_data(&main));
        let write_amplification = main.total_changes() - physical_changes;
        assert!(write_amplification > 1);
        assert_eq!(pin.c.total_changes(), shadow_changes);
        assert_eq!(run(&pin.c, "F1", "needle", &ids, None, false).0, before);
        println!(
            "{}",
            json!({"case":"B3F-C05","event":"real AFTER ANY UPDATE trigger; unchanged name/path stale bit toggle","physical_sqlite_changes":write_amplification,"shadow_sqlite_changes":pin.c.total_changes()-shadow_changes,"elapsed_ms":t.elapsed().as_secs_f64()*1000.0,"before_storage":physical_storage_before,"after_storage":storage(&main),"source_count_and_user_metadata_unchanged":true,"before_user_data_sha256":hash(&unchanged_user_data),"after_user_data_sha256":hash(&user_data(&main))})
        );
        // Visible in-place name/path mutation demonstrates a concrete candidate
        // omission if an F2 caller bypasses the source/term revision guard.
        let beta = f.docs.iter_mut().find(|d| d.uid == "beta").unwrap();
        beta.name = "newly renamed source".into();
        beta.path = "/Work/B/current-source.dat".into();
        beta.observation = 2;
        main.execute(
            "UPDATE files SET name=?,path=?,observation=2 WHERE uid='beta'",
            params![beta.name, beta.path],
        )
        .unwrap();
        main.execute("UPDATE claims SET observation=2 WHERE uid='beta'", [])
            .unwrap();
        main.execute_batch("UPDATE authority SET writer=writer+1,text_revision=text_revision+1,root_revision=root_revision+1").unwrap();
        let unsafe_f2 = run(&main, "F2", "needle", &old_ids, None, false).0;
        let expected_e0 = run(&pin.c, "F1", "needle", &old_ids, None, false).0;
        assert_ne!(unsafe_f2, expected_e0);
        assert_eq!(
            run(&native_pin, "F2", "needle", &old_ids, None, false).0,
            expected_e0
        );
        println!(
            "{}",
            json!({"case":"B3F-C04","event":"visible physical name/path rewrite before pointer publication","expected_E0":expected_e0,"unsafe_F2_ignoring_revision_guard":unsafe_f2,"actual_F2_guard_failure":f2_ready(&main).unwrap_err(),"old_pinned_F2_snapshot_preserved":true,"expected_rejection_observed":true})
        );
        // Correct F2 refresh from a fresh epoch, not silent reuse of old statistics.
        let refresh = prepare(&main, dir, 2, true);
        publish(&main, &refresh, || {}, false, false).unwrap();
        let shadow = readonly(&slot(dir, 2));
        let paired_hidden = paired(
            &main,
            &shadow,
            f,
            "B3F-C03",
            "hidden change refreshed",
            true,
        );
        println!(
            "{}",
            json!({"case":"B3F-C03","F1_incremental_prepare":refresh.record,"paired":paired_hidden})
        );
        main.execute("UPDATE authority SET root_revision=root_revision+1", [])
            .unwrap();
        let cancelled = prepare(&main, dir, 3, true);
        main.execute("UPDATE generations SET state='cancelled' WHERE epoch=3", [])
            .unwrap();
        let rejected = publish(&main, &cancelled, || {}, false, false).unwrap_err();
        assert_eq!(pointer(&main), 2);
        let forged = prepare(&main, dir, 4, true);
        main.execute(
            "UPDATE generations SET payload_digest='forged READY' WHERE epoch=4",
            [],
        )
        .unwrap();
        let forged_err = publish(&main, &forged, || {}, false, false).unwrap_err();
        let stale = prepare(&main, dir, 5, true);
        main.execute("UPDATE authority SET writer=writer+1", [])
            .unwrap();
        let stale_err = publish(&main, &stale, || {}, false, false).unwrap_err();
        println!(
            "{}",
            json!({"case":"B3F-C07","cancel_failure":rejected,"forged_READY_failure":forged_err,"source_writer_failure":stale_err,"pointer_retained":pointer(&main),"expected_rejection_observed":true})
        );
        // Root A removes shared source: B's explicit claim remains, never path inferred.
        main.execute("DELETE FROM claims WHERE rid='A' AND uid='alpha'", [])
            .unwrap();
        main.execute_batch("UPDATE authority SET root_revision=root_revision+1")
            .unwrap();
        f.docs
            .iter_mut()
            .find(|d| d.uid == "alpha")
            .unwrap()
            .roots
            .retain(|r| r != "A");
        let overlap = prepare(&main, dir, 6, true);
        publish(&main, &overlap, || {}, false, false).unwrap();
        let s = readonly(&slot(dir, 6));
        assert!(slot_ids(&s, Some("B")).contains(&1));
        assert!(!slot_ids(&s, Some("A")).contains(&1));
        println!(
            "{}",
            json!({"case":"B3F-C02","event":"overlap A withdrawal and B reconciliation_required readability","prepare":overlap.record,"paired":paired(&main,&s,f,"B3F-C02","A claim withdrawn",true),"action_authority":"not granted by this search fixture"})
        );
        // Same-path physical incarnation replacement; source rows/tags/notes retained.
        main.execute(
            "UPDATE files SET user_note='private old note' WHERE uid='alpha'",
            [],
        )
        .unwrap();
        main.execute("INSERT INTO tags VALUES('alpha','private-old-tag')", [])
            .unwrap();
        let old = f.docs.iter_mut().find(|d| d.uid == "alpha").unwrap();
        old.active = false;
        let mut replacement = old.clone();
        replacement.uid = "alpha-new-incarnation".into();
        replacement.rowid = 25;
        replacement.physical = "synthetic-volume:new-inode".into();
        replacement.active = true;
        main.execute("UPDATE files SET active=0 WHERE uid='alpha'", [])
            .unwrap();
        insert(&main, &replacement);
        for rid in &replacement.roots {
            main.execute(
                "INSERT INTO claims VALUES(?,?,?)",
                params![rid, replacement.uid, replacement.observation],
            )
            .unwrap();
        }
        f.docs.push(replacement);
        main.execute_batch("UPDATE authority SET writer=writer+1,text_revision=text_revision+1,root_revision=root_revision+1").unwrap();
        let source_drift = authority(&main).3
            != main
                .query_row(
                    "SELECT text_revision FROM generations WHERE epoch=6",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap();
        assert!(source_drift);
        let actual_f2_failure = f2_ready(&main).unwrap_err();
        let incarnated = prepare(&main, dir, 7, true);
        publish(&main, &incarnated, || {}, false, false).unwrap();
        let s = readonly(&slot(dir, 7));
        assert!(!slot_ids(&s, None).contains(&1));
        assert!(slot_ids(&s, None).contains(&25));
        let old_note: String = main
            .query_row("SELECT user_note FROM files WHERE uid='alpha'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let new_tags: i64 = main
            .query_row(
                "SELECT count(*) FROM tags WHERE uid='alpha-new-incarnation'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(old_note, "private old note");
        assert_eq!(new_tags, 0);
        let new_note: String = main
            .query_row(
                "SELECT user_note FROM files WHERE uid='alpha-new-incarnation'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_ne!(new_note, old_note);
        println!(
            "{}",
            json!({"case":"B3F-C04","event":"identity ABA + mutable physical-index F2 generation gap","F2_drift_guard_rejection":actual_f2_failure,"old_note_retained":old_note,"new_tags":new_tags,"new_note":new_note,"prepare":incarnated.record,"paired":paired(&main,&s,f,"B3F-C04","new physical incarnation",true),"pinned_E0_ids":pin.ids(&main,0).unwrap(),"new_pointer":pointer(&main),"old_cursor_rejected":pin.epoch!=pointer(&main)})
        );
        let b_cursor = cursor(&main, &s, "needle", Some("B"));
        let old_b = run(&s, "F1", "needle", &slot_ids(&s, Some("B")), None, false).0;
        let added = Doc {
            rowid: 26,
            uid: "rank-adversary-A".into(),
            name: "needle ".repeat(60),
            path: "/Work/A/needle".into(),
            physical: "synthetic:rank-adversary".into(),
            observation: 1,
            mtime: 123,
            roots: vec!["A".into()],
            active: true,
        };
        insert(&main, &added);
        main.execute("INSERT INTO claims VALUES('A',?,1)", [&added.uid])
            .unwrap();
        f.docs.push(added);
        main.execute_batch("UPDATE authority SET writer=writer+1,text_revision=text_revision+1,root_revision=root_revision+1").unwrap();
        let rank_stage = prepare(&main, dir, 8, true);
        publish(&main, &rank_stage, || {}, false, false).unwrap();
        let rank_slot = readonly(&slot(dir, 8));
        let new_b = run(
            &rank_slot,
            "F1",
            "needle",
            &slot_ids(&rank_slot, Some("B")),
            None,
            false,
        )
        .0;
        assert_eq!(
            old_b.iter().map(|h| &h.uid).collect::<BTreeSet<_>>(),
            new_b.iter().map(|h| &h.uid).collect::<BTreeSet<_>>()
        );
        assert_ne!(old_b, new_b);
        let invalid = validate(&main, &rank_slot, &b_cursor, "needle", Some("B")).unwrap_err();
        let mut legacy = cursor(&main, &rank_slot, "needle", Some("B"));
        legacy.version = 1;
        let legacy_invalid = validate(&main, &rank_slot, &legacy, "needle", Some("B")).unwrap_err();
        let tamper = validate(
            &main,
            &rank_slot,
            &cursor(&main, &rank_slot, "needle", Some("B")),
            "needle",
            Some("A"),
        )
        .unwrap_err();
        println!(
            "{}",
            json!({"case":"B3F-C06","unchanged_B_ids_changed_global_rank":{"old":old_b,"new":new_b},"old_cursor_failure":invalid,"legacy_cursor_failure":legacy_invalid,"scope_tampering_failure":tamper,"prepare":rank_stage.record,"paired":paired(&main,&rank_slot,f,"B3F-C06","global rank changed by A",true)})
        );
        assert_eq!(authority(&native_pin), old_authority);
        f2_ready(&native_pin).unwrap();
        assert_eq!(
            run(&native_pin, "F2", "needle", &old_ids, None, false).0,
            run(&pin.c, "F1", "needle", &old_ids, None, false).0
        );
        native_pin.execute_batch("COMMIT").unwrap();
        let source_count = documents(&main).len();
        let held = lifecycle::gc(&main, dir, 0, None);
        assert!(slot(dir, 1).exists());
        assert!(pin.ids(&main, 11).is_err());
        let released = lifecycle::gc(&main, dir, 11, None);
        assert!(!slot(dir, 1).exists());
        assert!(
            pin.ids(&main, 0).is_err(),
            "expired reader cannot be resurrected with an older clock"
        );
        assert_eq!(documents(&main).len(), source_count);
        println!(
            "{}",
            json!({"case":"B3F-C08","live_lease_gc":held,"expired_reader_rejected":pin.ids(&main,11).unwrap_err(),"past_clock_replay_rejected":pin.ids(&main,0).unwrap_err(),"expired_lease_gc":released,"pinned_reader_lifetime_ms":pin.started.elapsed().as_secs_f64()*1000.0,"lease_clock":"deterministic logical ticks, not production timeout policy","physical_source_count_retained":source_count})
        );
        let raw: Fixture =
            serde_json::from_slice(&std::fs::read(std::env::args().nth(2).unwrap()).unwrap())
                .unwrap();
        println!(
            "{}",
            json!({"case":"B3F-C09","actual_SIGKILL_records":lifecycle::crashes(&dir.join("crashes"),&raw)})
        );
    } else {
        // Controlled root churn: withdraw 10 A-only claims; shared B claims survive.
        let mut changed = 0;
        for d in &mut f.docs {
            if changed < 10 && d.roots == ["A"] {
                main.execute("DELETE FROM claims WHERE rid='A' AND uid=?", [&d.uid])
                    .unwrap();
                d.roots.clear();
                changed += 1;
            }
        }
        main.execute_batch("UPDATE authority SET root_revision=root_revision+1")
            .unwrap();
        let second = prepare(&main, dir, 2, true);
        assert_eq!(pin.ids(&main, 0).unwrap(), slot_ids(&pin.c, None));
        let timing = contention(&main, &second, &path, !f.docs.is_empty());
        let new = readonly(&slot(dir, 2));
        let ids = slot_ids(&new, None);
        let (n, tokens) = stats(&new, &ids);
        register(&main, &ids, n, tokens);
        let after = paired(&main, &new, f, case, "E1 root delta", small);
        let pinned = slot_ids(&pin.c, None);
        let gc = lifecycle::gc(&main, dir, 0, None);
        assert!(slot(dir, 1).exists());
        let current_note: Option<String> = if f.docs.is_empty() {
            None
        } else {
            Some(
                main.query_row("SELECT user_note FROM files WHERE rowid=1", [], |r| {
                    r.get(0)
                })
                .unwrap(),
            )
        };
        if let Some(ref note) = current_note {
            assert_eq!(note, "committed user correction");
        }
        let before_gc_count = documents(&main).len();
        let mut gc_batches = Vec::new();
        for _ in 0..300 {
            let remaining: i64 = main
                .query_row("SELECT count(*) FROM generations WHERE epoch!=2", [], |r| {
                    r.get(0)
                })
                .unwrap();
            if remaining == 0 {
                break;
            }
            gc_batches.push(lifecycle::gc(&main, dir, 11, None));
        }
        assert_eq!(
            main.query_row("SELECT count(*) FROM generations WHERE epoch!=2", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(pin.ids(&main, 0).is_err());
        assert_eq!(f2_ids(&main, None), slot_ids(&new, None));
        assert_eq!(f2_stats(&main), (n, tokens));
        assert_eq!(documents(&main).len(), before_gc_count);
        println!(
            "{}",
            json!({"case":case,"event":"root delta paired E1 and publication contention","changed_root_claims":changed,"F1_incremental_prepare":second.record,"timing":timing,"paired":after,"pinned_E0_ids":pinned,"pinned_reader_lifetime_ms":pin.started.elapsed().as_secs_f64()*1000.0,"gc":gc,"expired_pin_cleanup_batches":gc_batches,"all_physical_sources_retained":before_gc_count,"past_clock_replay_rejected":pin.ids(&main,0).unwrap_err(),"current_note":current_note,"storage":storage(&main),"old_cursor_rejected":pin.epoch!=pointer(&main),"environment":environment(&main)})
        );
    }
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|s| s == "--crash") {
        lifecycle::child(args[2].clone().into(), &args[3]);
        return;
    }
    assert_eq!(args.len(), 5, "probe FIXTURE_JSON OUTPUT_DIR CASE_ID");
    let mut f: Fixture = serde_json::from_slice(&std::fs::read(&args[2]).unwrap()).unwrap();
    let small = f.docs.len() < 256;
    suite(&mut f, Path::new(&args[3]), &args[4], small);
}
