use crate::{corpus::*, db::*};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Instant,
};

pub fn setup_leases(main: &Connection) {
    main.execute_batch("CREATE TABLE IF NOT EXISTS leases(reader TEXT PRIMARY KEY,epoch INTEGER,expires INTEGER);CREATE TABLE IF NOT EXISTS lease_clock(id INTEGER PRIMARY KEY,tick INTEGER);INSERT OR IGNORE INTO lease_clock VALUES(1,0);CREATE TABLE IF NOT EXISTS user_state(k TEXT PRIMARY KEY,v TEXT);INSERT OR IGNORE INTO user_state VALUES('note','empty'),('tag','empty');").unwrap();
}
pub struct Pin {
    pub epoch: i64,
    pub reader: String,
    pub c: Connection,
    pub started: Instant,
}
impl Pin {
    pub fn new(main: &Connection, dir: &Path, reader: &str, expires: i64) -> Self {
        setup_leases(main);
        main.execute_batch("BEGIN IMMEDIATE").unwrap();
        let epoch = pointer(main);
        main.execute(
            "INSERT INTO leases VALUES(?,?,?)",
            params![reader, epoch, expires],
        )
        .unwrap();
        main.execute_batch("COMMIT").unwrap();
        let c = readonly(&slot(dir, epoch));
        c.execute_batch("BEGIN").unwrap();
        // Force an actual SQLite snapshot before staging/GC starts.
        let _: i64 = c
            .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
            .unwrap();
        Self {
            epoch,
            reader: reader.into(),
            c,
            started: Instant::now(),
        }
    }
    pub fn ids(&self, main: &Connection, tick: i64) -> Result<BTreeSet<i64>, String> {
        let valid: i64 = main
            .query_row(
                "SELECT count(*) FROM leases WHERE reader=? AND epoch=? AND expires>max(?,(SELECT tick FROM lease_clock WHERE id=1))",
                params![self.reader, self.epoch, tick],
                |r| r.get(0),
            )
            .unwrap();
        if valid != 1 {
            Err("reader_snapshot_expired; no replay of retired FTS authority".into())
        } else {
            Ok(slot_ids(&self.c, None))
        }
    }
}
pub fn gc(main: &Connection, dir: &Path, tick: i64, checkpoint: Option<&str>) -> Value {
    setup_leases(main);
    main.execute("UPDATE lease_clock SET tick=max(tick,?) WHERE id=1", [tick])
        .unwrap();
    let epoch = pointer(main);
    let t = Instant::now();
    let mut deleted = Vec::new();
    let mut held = Vec::new();
    let old: Vec<i64> = main
        .prepare("SELECT epoch FROM generations WHERE epoch!=? ORDER BY epoch LIMIT 1")
        .unwrap()
        .query_map([epoch], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for e in old {
        let live: i64 = main
            .query_row(
                "SELECT count(*) FROM leases WHERE epoch=? AND expires>max(?,(SELECT tick FROM lease_clock WHERE id=1))",
                params![e, tick],
                |r| r.get(0),
            )
            .unwrap();
        if live != 0 {
            held.push(e);
            continue;
        }
        main.execute("UPDATE generations SET state='gc_pending' WHERE epoch=? AND epoch!=(SELECT epoch FROM authority)",[e]).unwrap();
        if checkpoint == Some("gc-before-unlink") {
            crate::checkpoint("gc-before-unlink");
        }
        let path = slot(dir, e);
        if path.exists() {
            std::fs::remove_file(path).unwrap();
        }
        if checkpoint == Some("gc-after-unlink") {
            crate::checkpoint("gc-after-unlink");
        }
        let row_start = Instant::now();
        let removed=main.execute("DELETE FROM epoch_members WHERE epoch=? AND _rowid_ IN (SELECT _rowid_ FROM epoch_members WHERE epoch=? LIMIT 1000)",rusqlite::params![e,e]).unwrap();
        let row_lock_ms = row_start.elapsed().as_secs_f64() * 1000.0;
        let remaining: bool = main
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM epoch_members WHERE epoch=? LIMIT 1)",
                [e],
                |r| r.get(0),
            )
            .unwrap();
        if remaining {
            deleted.push(json!({"epoch":e,"members_removed":removed,"ledger_writer_ms":row_lock_ms,"remaining":true}));
            continue;
        }
        main.execute("DELETE FROM epoch_roots WHERE epoch=?", [e])
            .unwrap();
        main.execute(
            "DELETE FROM generations WHERE epoch=? AND state='gc_pending'",
            [e],
        )
        .unwrap();
        deleted.push(json!({"epoch":e,"members_removed":removed,"ledger_writer_ms":row_lock_ms,"remaining":false}));
    }
    json!({"phase":"bounded_gc_one_slot","held_by_live_lease":held,"deleted":deleted,"ms":t.elapsed().as_secs_f64()*1000.0,"scope":"private index slot only; no physical file/user rows removed"})
}

pub fn crashes(parent: &Path, f: &Fixture) -> Value {
    use std::os::unix::process::ExitStatusExt;
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        sync::mpsc,
    };
    let mut results = Vec::new();
    for point in [
        "during-build",
        "before-publish",
        "before-commit",
        "after-commit",
        "gc-before-unlink",
        "gc-after-unlink",
    ] {
        let dir = parent.join(point);
        std::fs::create_dir_all(&dir).unwrap();
        let main = open(&dir.join("main.sqlite"));
        create(&main, f);
        setup_leases(&main);
        let p = prepare(&main, &dir, 1, false);
        publish(&main, &p, || {}, false, false).unwrap();
        drop(main);
        let command = vec![
            std::env::current_exe().unwrap().display().to_string(),
            "--crash".into(),
            dir.display().to_string(),
            point.into(),
        ];
        let mut child = Command::new(&command[0])
            .args(&command[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, recv) = mpsc::channel();
        std::thread::spawn(move || {
            let mut s = String::new();
            BufReader::new(stdout).read_line(&mut s).unwrap();
            let _ = send.send(s);
        });
        let barrier = recv
            .recv_timeout(std::time::Duration::from_secs(30))
            .unwrap();
        assert!(barrier.contains(point));
        child.kill().unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.signal(), Some(9));
        let main = open(&dir.join("main.sqlite"));
        let recovered = recover(&main, &dir);
        let expected = if matches!(
            point,
            "after-commit" | "gc-before-unlink" | "gc-after-unlink"
        ) {
            2
        } else {
            1
        };
        assert_eq!(pointer(&main), expected);
        let c = readonly(&slot(&dir, expected));
        let ids = slot_ids(&c, None);
        let mut expected_ids: BTreeSet<i64> = f
            .docs
            .iter()
            .filter(|d| {
                d.active
                    && d.roots
                        .iter()
                        .any(|rid| f.roots.iter().any(|r| r.rid == *rid && r.enabled))
            })
            .map(|d| d.rowid)
            .collect();
        if expected == 2 {
            expected_ids.remove(&2);
        }
        assert_eq!(ids, expected_ids);
        assert_eq!(documents(&main).len(), f.docs.len());
        let cleanup = gc(&main, &dir, 100, None);
        let again = gc(&main, &dir, 100, None);
        let (n, tokens) = stats(&c, &ids);
        crate::query::register(&main, &ids, n, tokens);
        let (a, _) = crate::query::run(&c, "F1", "needle", &ids, None, false);
        let (b, _) = crate::query::run(&main, "F2", "needle", &ids, None, false);
        assert_eq!(a, b, "both candidates recover the same published rank");
        let mut ref_fixture = f.clone();
        if expected == 2 {
            ref_fixture
                .docs
                .iter_mut()
                .find(|d| d.uid == "beta")
                .unwrap()
                .roots
                .clear();
        }
        assert_eq!(
            a,
            crate::query::reference(&ref_fixture, None, "needle").0,
            "independent native crash recovery rank"
        );
        results.push(json!({"checkpoint":point,"command":command,"barrier":barrier,"signal":9,"stderr":String::from_utf8_lossy(&output.stderr),"expected_epoch":expected,"actual_epoch":pointer(&main),"expected_ids":expected_ids,"actual_ids":ids,"recovery":recovered,"cleanup":cleanup,"repeat_gc":again,"F1":a,"F2":b,"physical_source_count_retained":f.docs.len()}));
    }
    json!(results)
}
pub fn child(dir: PathBuf, point: &str) {
    let main = open(&dir.join("main.sqlite"));
    main.execute_batch("BEGIN IMMEDIATE; DELETE FROM claims WHERE uid='beta'; UPDATE authority SET root_revision=root_revision+1; COMMIT;").unwrap();
    if point == "during-build" {
        let c = open(&slot(&dir, 2));
        index_schema(&c);
        c.execute_batch("BEGIN IMMEDIATE; INSERT INTO files(uid,name,path,physical,observation,active) VALUES('partial','needle','/partial','synthetic',1,1);").unwrap();
        crate::checkpoint(point);
    }
    let p = prepare(&main, &dir, 2, true);
    if point == "before-publish" {
        crate::checkpoint(point);
    }
    publish(
        &main,
        &p,
        || {},
        point == "before-commit",
        point == "after-commit",
    )
    .unwrap();
    if point.starts_with("gc-") {
        gc(&main, &dir, 100, Some(point));
    }
    panic!("parent must kill subprocess at checkpoint");
}
