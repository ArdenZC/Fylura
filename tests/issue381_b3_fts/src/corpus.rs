use crate::db::*;
use rusqlite::{params, Connection, OpenFlags};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Instant,
};

pub struct Prepared {
    pub epoch: i64,
    pub writer: i64,
    pub text: i64,
    pub roots: i64,
    pub digest: String,
    pub n: i64,
    pub tokens: i64,
    pub record: Value,
}
pub fn slot(dir: &Path, epoch: i64) -> PathBuf {
    dir.join(format!("slot-{epoch}.sqlite"))
}
pub fn readonly(path: &Path) -> Connection {
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    c.execute_batch("PRAGMA query_only=ON; PRAGMA busy_timeout=5000;")
        .unwrap();
    c
}
pub fn slot_ids(c: &Connection, scope: Option<&str>) -> BTreeSet<i64> {
    c.prepare("SELECT DISTINCT f.rowid FROM files f JOIN frozen_claims m ON m.uid=f.uid JOIN frozen_roots r ON r.rid=m.rid WHERE r.enabled=1 AND (? IS NULL OR m.rid=?)").unwrap().query_map(params![scope,scope],|r|r.get(0)).unwrap().map(Result::unwrap).collect()
}
pub fn prepare(main: &Connection, dir: &Path, epoch: i64, incremental: bool) -> Prepared {
    let t = Instant::now();
    main.execute_batch("BEGIN").unwrap();
    let (old, _, writer, text, roots) = authority(main);
    let ids = visible(main, None);
    let docs = documents(main);
    let membership = claims(main);
    let root_data: Vec<Root> = main
        .prepare("SELECT rid,enabled,health FROM roots ORDER BY rid")
        .unwrap()
        .query_map([], |r| {
            Ok(Root {
                rid: r.get(0)?,
                enabled: r.get(1)?,
                health: r.get(2)?,
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let stats_start = Instant::now();
    let (n, tokens) = stats(main, &ids);
    let stats_ms = stats_start.elapsed().as_secs_f64() * 1000.0;
    main.execute_batch("COMMIT").unwrap();
    let projection: Vec<&Doc> = docs.iter().filter(|d| ids.contains(&d.rowid)).collect();
    let digest = hash(&(&projection, &membership, &root_data, writer, text, roots));
    main.execute_batch("CREATE TABLE IF NOT EXISTS epoch_roots(epoch INTEGER,rid TEXT,enabled INTEGER,PRIMARY KEY(epoch,rid)); CREATE TABLE IF NOT EXISTS epoch_members(epoch INTEGER,rid TEXT,rowid INTEGER,uid TEXT,PRIMARY KEY(epoch,rid,rowid));").unwrap();
    main.execute_batch("CREATE TRIGGER IF NOT EXISTS sealed_member_insert BEFORE INSERT ON epoch_members WHEN (SELECT state FROM generations WHERE epoch=NEW.epoch)!='staging' BEGIN SELECT RAISE(ABORT,'sealed corpus membership'); END; CREATE TRIGGER IF NOT EXISTS sealed_member_update BEFORE UPDATE ON epoch_members BEGIN SELECT RAISE(ABORT,'immutable corpus membership'); END; CREATE TRIGGER IF NOT EXISTS sealed_member_delete BEFORE DELETE ON epoch_members WHEN (SELECT state FROM generations WHERE epoch=OLD.epoch) IN ('ready','published') BEGIN SELECT RAISE(ABORT,'retained published corpus membership'); END; CREATE TRIGGER IF NOT EXISTS sealed_root_update BEFORE UPDATE ON epoch_roots BEGIN SELECT RAISE(ABORT,'immutable root snapshot'); END;").unwrap();
    main.execute(
        "INSERT INTO generations VALUES(?,'staging',?,?,?,?,?,?)",
        params![epoch, writer, text, roots, digest, n, tokens],
    )
    .unwrap();
    for r in &root_data {
        main.execute(
            "INSERT INTO epoch_roots VALUES(?,?,?)",
            params![epoch, r.rid, r.enabled],
        )
        .unwrap();
    }
    let projection_by_uid: std::collections::BTreeMap<&str, &&Doc> =
        projection.iter().map(|d| (d.uid.as_str(), d)).collect();
    let member_start = Instant::now();
    let mut member_batches = Vec::new();
    for batch in membership.chunks(1000) {
        let start = Instant::now();
        main.execute_batch("BEGIN IMMEDIATE").unwrap();
        for (rid, uid, observation) in batch {
            if let Some(d) = projection_by_uid
                .get(uid.as_str())
                .filter(|d| d.observation == *observation)
            {
                main.execute(
                    "INSERT INTO epoch_members VALUES(?,?,?,?)",
                    params![epoch, rid, d.rowid, uid],
                )
                .unwrap();
            }
        }
        main.execute_batch("COMMIT").unwrap();
        member_batches.push(
            json!({"claims":batch.len(),"writer_lock_ms":start.elapsed().as_secs_f64()*1000.0}),
        );
    }
    let member_ms = member_start.elapsed().as_secs_f64() * 1000.0;
    let shadow_start = Instant::now();
    let path = slot(dir, epoch);
    assert!(!path.exists(), "immutable slot must be fresh");
    let mut copy_ms = 0.0;
    let shadow = open(&path);
    if incremental && old != 0 {
        // SQLite online backup copies the old index. Copy is O(DB bytes), never
        // represented as O(1); only changed document index operations are delta.
        let previous = readonly(&slot(dir, old));
        let start = Instant::now();
        unsafe {
            let backup = libsqlite3_sys::sqlite3_backup_init(
                shadow.handle(),
                c"main".as_ptr(),
                previous.handle(),
                c"main".as_ptr(),
            );
            assert!(!backup.is_null());
            assert_eq!(
                libsqlite3_sys::sqlite3_backup_step(backup, -1),
                libsqlite3_sys::SQLITE_DONE
            );
            assert_eq!(
                libsqlite3_sys::sqlite3_backup_finish(backup),
                libsqlite3_sys::SQLITE_OK
            );
        }
        drop(shadow);
        copy_ms = start.elapsed().as_secs_f64() * 1000.0;
    } else {
        index_schema(&shadow);
        shadow.execute_batch("CREATE TABLE frozen_meta(epoch INTEGER,text_revision INTEGER); CREATE TABLE frozen_roots(rid TEXT PRIMARY KEY,enabled INTEGER,health TEXT);CREATE TABLE frozen_claims(rid TEXT,uid TEXT,observation INTEGER,PRIMARY KEY(rid,uid));").unwrap();
        drop(shadow);
    }
    let shadow = open(&path);
    let index_before = shadow.total_changes();
    let update = Instant::now();
    let old_docs = documents(&shadow);
    let mut removed = 0;
    let mut added = 0;
    let mut changed = 0;
    for batch in old_docs.chunks(1000) {
        shadow.execute_batch("BEGIN IMMEDIATE").unwrap();
        for d in batch {
            if !ids.contains(&d.rowid) {
                shadow
                    .execute("DELETE FROM files WHERE rowid=?", [d.rowid])
                    .unwrap();
                removed += 1;
            }
        }
        shadow.execute_batch("COMMIT").unwrap();
    }
    let old_map: std::collections::BTreeMap<i64, Doc> =
        old_docs.into_iter().map(|d| (d.rowid, d)).collect();
    let mut batches = Vec::new();
    for batch in projection.chunks(1000) {
        let start = Instant::now();
        shadow.execute_batch("BEGIN IMMEDIATE").unwrap();
        for d in batch {
            if let Some(previous) = old_map.get(&d.rowid) {
                if previous.name != d.name
                    || previous.path != d.path
                    || previous.observation != d.observation
                    || previous.mtime != d.mtime
                {
                    shadow
                        .execute(
                            "UPDATE files SET name=?,path=?,observation=?,physical=?,mtime=? WHERE rowid=?",
                            params![d.name, d.path, d.observation, d.physical, d.mtime,d.rowid],
                        )
                        .unwrap();
                    changed += 1;
                }
            } else {
                insert(&shadow, d);
                added += 1;
            }
        }
        shadow.execute_batch("COMMIT").unwrap();
        batches.push(json!({"rows":batch.len(),"ms":start.elapsed().as_secs_f64()*1000.0}));
    }
    shadow.execute_batch("BEGIN IMMEDIATE; DELETE FROM frozen_claims; DELETE FROM frozen_roots; DELETE FROM frozen_meta;").unwrap();
    shadow
        .execute("INSERT INTO frozen_meta VALUES(?,?)", params![epoch, text])
        .unwrap();
    for r in &root_data {
        shadow
            .execute(
                "INSERT INTO frozen_roots VALUES(?,?,?)",
                params![r.rid, r.enabled, r.health],
            )
            .unwrap();
    }
    for batch in membership.chunks(1000) {
        for (rid, uid, observation) in batch {
            shadow
                .execute(
                    "INSERT INTO frozen_claims VALUES(?,?,?)",
                    params![rid, uid, observation],
                )
                .unwrap();
        }
    }
    shadow.execute_batch("COMMIT").unwrap();
    let delta_ms = update.elapsed().as_secs_f64() * 1000.0;
    shadow
        .execute(
            "INSERT INTO files_fts(files_fts,rank) VALUES('integrity-check',1)",
            [],
        )
        .unwrap();
    assert_eq!(documents(&shadow).len() as i64, n);
    assert_eq!(stats(&shadow, &slot_ids(&shadow, None)), (n, tokens));
    let storage_data = storage(&shadow);
    let changes = shadow.total_changes() - index_before;
    shadow
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    drop(shadow);
    // Normal query operations only open sealed slots read-only. Logical immutability
    // is exercised by an actual attempted SQL update, not filesystem permission lore.
    let ro = readonly(&path);
    assert!(ro.execute("UPDATE files SET name='forged'", []).is_err());
    drop(ro);
    main.execute(
        "UPDATE generations SET state='ready' WHERE epoch=? AND state='staging'",
        [epoch],
    )
    .unwrap();
    let record = json!({"phase":"F1_prepare","epoch":epoch,"n":n,"tokens":tokens,"digest":digest,"source_writer":writer,"text_revision":text,"root_revision":roots,"F1_shadow_prepare_ms":shadow_start.elapsed().as_secs_f64()*1000.0,"F2_stats_prepare_ms":stats_ms,"shared_membership_prepare_ms":member_ms,"shared_membership_batches":member_batches,"total_ms":t.elapsed().as_secs_f64()*1000.0,"copy_ms":copy_ms,"delta_ms":delta_ms,"delta_documents":{"added":added,"removed":removed,"changed":changed},"shadow_sqlite_changes":changes,"batches":batches,"storage":storage_data,"sealed_readonly_update_rejected":true,"cost":"full build O(N); incremental online backup copy O(DB bytes), document index delta O(changed docs); root claim projection O(claims)"});
    Prepared {
        epoch,
        writer,
        text,
        roots,
        digest,
        n,
        tokens,
        record,
    }
}
pub fn publish(
    main: &Connection,
    p: &Prepared,
    barrier: impl FnOnce(),
    crash_before: bool,
    crash_after: bool,
) -> Result<Value, String> {
    let start = Instant::now();
    main.execute_batch("BEGIN IMMEDIATE").unwrap();
    let acquired = Instant::now();
    let rc=main.execute("UPDATE authority SET epoch=?,revision=revision+1 WHERE id=1 AND writer=? AND text_revision=? AND root_revision=? AND EXISTS(SELECT 1 FROM generations g WHERE g.epoch=? AND g.state='ready' AND g.writer=? AND g.text_revision=? AND g.root_revision=? AND g.payload_digest=? AND g.n=? AND g.tokens=?)",params![p.epoch,p.writer,p.text,p.roots,p.epoch,p.writer,p.text,p.roots,p.digest,p.n,p.tokens]).unwrap();
    if rc != 1 {
        main.execute_batch("ROLLBACK").unwrap();
        return Err("CAS rejected stale/cancelled/forged FTS authority".into());
    }
    main.execute(
        "UPDATE generations SET state='published' WHERE epoch=?",
        [p.epoch],
    )
    .unwrap();
    barrier();
    if crash_before {
        crate::checkpoint("before-commit");
    }
    main.execute_batch("COMMIT").unwrap();
    if crash_after {
        crate::checkpoint("after-commit");
    }
    Ok(
        json!({"phase":"publication","epoch":p.epoch,"begin_wait_ms":acquired.duration_since(start).as_secs_f64()*1000.0,"writer_lock_ms":acquired.elapsed().as_secs_f64()*1000.0,"statement_count":4,"statements":["BEGIN IMMEDIATE","constant-size UPDATE authority CAS + epoch/revision","UPDATE generations WHERE epoch=?","COMMIT"],"no_index_or_row_walk":true}),
    )
}
pub fn pointer(main: &Connection) -> i64 {
    authority(main).0
}
pub fn recover(main: &Connection, dir: &Path) -> Value {
    let epoch = pointer(main);
    assert!(slot(dir, epoch).exists());
    let c = readonly(&slot(dir, epoch));
    main.execute(
        "UPDATE generations SET state='abandoned' WHERE state='ready' AND epoch!=?",
        [epoch],
    )
    .unwrap();
    json!({"epoch":epoch,"visible_ids":documents(&c).iter().map(|d|d.uid.clone()).collect::<Vec<_>>(),"sqlite_integrity":c.query_row("PRAGMA integrity_check",[],|r|r.get::<_,String>(0)).unwrap(),"state":"published pointer only; abandoned stage never READY"})
}
