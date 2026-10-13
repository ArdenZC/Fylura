use crate::db::*;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

unsafe extern "C" {
    fn b3f_register(
        db: *mut libsqlite3_sys::sqlite3,
        n: i64,
        tokens: i64,
        ids: *const i64,
        length: i32,
    ) -> i32;
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Hit {
    pub uid: String,
    pub rowid: i64,
    pub score: f64,
    pub bits: String,
}
pub fn sort(hits: &mut [Hit], data: &BTreeMap<i64, Doc>) {
    hits.sort_by(|a, b| {
        a.score
            .total_cmp(&b.score)
            .then(data[&b.rowid].mtime.cmp(&data[&a.rowid].mtime))
            .then(
                data[&a.rowid]
                    .name
                    .to_ascii_lowercase()
                    .cmp(&data[&b.rowid].name.to_ascii_lowercase()),
            )
            .then(a.uid.cmp(&b.uid))
    });
}
pub fn register(c: &Connection, ids: &BTreeSet<i64>, n: i64, tokens: i64) {
    let rows: Vec<i64> = ids.iter().copied().collect();
    let rc = unsafe { b3f_register(c.handle(), n, tokens, rows.as_ptr(), rows.len() as i32) };
    assert_eq!(rc, 0, "FTS5 API registration");
}
pub fn run(
    c: &Connection,
    variant: &str,
    raw: &str,
    allowed: &BTreeSet<i64>,
    limit: Option<usize>,
    metrics: bool,
) -> (Vec<Hit>, Value) {
    let expression = qform(raw);
    if expression.is_empty() {
        return (Vec::new(), json!({"empty_query":true}));
    }
    let score = if variant == "F2" {
        "effective_bm25(files_fts)"
    } else {
        "bm25(files_fts,6.0,1.5)"
    };
    let sql = format!("SELECT rowid,{score} FROM files_fts WHERE files_fts MATCH ?");
    let t = Instant::now();
    let mut stmt = c.prepare(&sql).unwrap();
    let rows: Vec<(i64, Option<f64>)> = stmt
        .query_map([&expression], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let candidate_count = rows.len();
    let names: BTreeMap<i64, String> = c
        .prepare("SELECT rowid,uid FROM files")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut hits: Vec<Hit> = rows
        .into_iter()
        .filter(|r| allowed.contains(&r.0))
        .filter_map(|(rowid, score)| {
            score.map(|score| Hit {
                rowid,
                uid: names[&rowid].clone(),
                score,
                bits: format!("{:016x}", score.to_bits()),
            })
        })
        .collect();
    let data: BTreeMap<i64, Doc> = documents(c).into_iter().map(|d| (d.rowid, d)).collect();
    sort(&mut hits, &data);
    let total = hits.len();
    if let Some(l) = limit {
        hits.truncate(l);
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    let info = if metrics {
        let sql =
            "SELECT rowid,effective_metrics(files_fts) FROM files_fts WHERE files_fts MATCH ?";
        let data: Vec<(i64, String)> = c
            .prepare(sql)
            .unwrap()
            .query_map([&expression], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        json!(data
            .into_iter()
            .filter(|r| allowed.contains(&r.0))
            .map(|(id, s)| (id, serde_json::from_str::<Value>(&s).unwrap()))
            .collect::<Vec<_>>())
    } else {
        json!(null)
    };
    (
        hits,
        json!({"variant":variant,"sql":sql,"bindings":[expression],"query_ms":ms,"unlimited_physical_candidates":candidate_count,"scoped_count_before_limit":total,"limit":limit,"plan":plan(c,&sql,&expression),"metrics":info}),
    )
}
pub fn reference(f: &Fixture, scope: Option<&str>, query: &str) -> (Vec<Hit>, Value) {
    // Fresh native oracle owns a separate minimal index and direct rank query.
    // Visibility/scope is derived from fixture labels, never the model SQL.
    let roots: BTreeSet<String> = f
        .roots
        .iter()
        .filter(|r| r.enabled)
        .map(|r| r.rid.clone())
        .collect();
    let global: Vec<&Doc> = f
        .docs
        .iter()
        .filter(|d| d.active && d.roots.iter().any(|r| roots.contains(r)))
        .collect();
    let oracle = Connection::open_in_memory().unwrap();
    oracle
        .execute_batch(
            "CREATE VIRTUAL TABLE expected USING fts5(uid UNINDEXED,name,path,mtime UNINDEXED,tokenize='trigram');",
        )
        .unwrap();
    for d in &global {
        oracle
            .execute(
                "INSERT INTO expected(rowid,uid,name,path,mtime) VALUES(?,?,?,?,?)",
                rusqlite::params![d.rowid, d.uid, d.name, d.path, d.mtime],
            )
            .unwrap();
    }
    let expression = qform(query);
    if expression.is_empty() {
        return (Vec::new(), json!({"oracle_empty_query":true}));
    }
    let hits:Vec<Hit>=oracle.prepare("SELECT rowid,uid,bm25(expected,0.0,6.0,1.5,0.0) FROM expected WHERE expected MATCH ? ORDER BY bm25(expected,0.0,6.0,1.5,0.0),mtime DESC,name COLLATE NOCASE,uid").unwrap().query_map([&expression],|r|{let score:f64=r.get(2)?;Ok(Hit{rowid:r.get(0)?,uid:r.get(1)?,score,bits:format!("{:016x}",score.to_bits())})}).unwrap().map(Result::unwrap).filter(|h| scope.is_none_or(|s|roots.contains(s) && f.docs.iter().any(|d|d.rowid==h.rowid && d.roots.iter().any(|r|r==s)))).collect();
    // UNINDEXED UID contributes zero tokens; two name/path weights retain the
    // exact production BM25 corpus semantics while owning independent row data.
    (
        hits,
        json!({"oracle":"fresh visible-only native trigram; independent fixture-label union","n":global.len(),"query":expression}),
    )
}
pub fn legacy_like(c: &Connection, query: &str, allowed: &BTreeSet<i64>) -> Vec<String> {
    let pattern = format!(
        "%{}%",
        query
            .trim()
            .replace('~', "~~")
            .replace('%', "~%")
            .replace('_', "~_")
    );
    c.prepare("SELECT rowid,uid FROM files WHERE name LIKE ? ESCAPE '~' OR path LIKE ? ESCAPE '~' ORDER BY uid").unwrap().query_map(rusqlite::params![pattern,pattern],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).unwrap().map(Result::unwrap).filter(|r|allowed.contains(&r.0)).map(|r|r.1).collect()
}

pub fn f2_ready(main: &Connection) -> Result<(), String> {
    let (epoch, _, _, text, _) = authority(main);
    let (sealed_text, state): (i64, String) = main
        .query_row(
            "SELECT text_revision,state FROM generations WHERE epoch=?",
            [epoch],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    if text != sealed_text || state != "published" {
        Err("F2_BLOCKED_MUTABLE_PHYSICAL_INDEX: current terms no longer match published corpus proof; pinned read transaction required or immutable all-observation index integration".into())
    } else {
        Ok(())
    }
}
pub fn f2_ids(main: &Connection, scope: Option<&str>) -> BTreeSet<i64> {
    main.prepare("SELECT DISTINCT m.rowid FROM epoch_members m JOIN epoch_roots r ON r.epoch=m.epoch AND r.rid=m.rid WHERE m.epoch=(SELECT epoch FROM authority) AND r.enabled=1 AND (? IS NULL OR m.rid=?)").unwrap().query_map(rusqlite::params![scope,scope],|r|r.get(0)).unwrap().map(Result::unwrap).collect()
}
pub fn f2_stats(main: &Connection) -> (i64, i64) {
    main.query_row(
        "SELECT n,tokens FROM generations WHERE epoch=(SELECT epoch FROM authority)",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Cursor {
    pub version: u32,
    pub epoch: i64,
    pub revision: i64,
    pub query: String,
    pub scope: Option<String>,
    pub ranked_digest: String,
}
pub fn cursor(main: &Connection, shadow: &Connection, query: &str, scope: Option<&str>) -> Cursor {
    let (epoch, revision, _, _, _) = authority(main);
    let rows = run(
        shadow,
        "F1",
        query,
        &crate::corpus::slot_ids(shadow, scope),
        None,
        false,
    )
    .0;
    Cursor {
        version: 2,
        epoch,
        revision,
        query: query.into(),
        scope: scope.map(str::to_owned),
        ranked_digest: hash(&rows),
    }
}
pub fn validate(
    main: &Connection,
    shadow: &Connection,
    saved: &Cursor,
    query: &str,
    scope: Option<&str>,
) -> Result<(), String> {
    if saved.version != 2 {
        return Err("legacy snapshot cursor unsupported; restart current query".into());
    }
    let current = cursor(main, shadow, query, scope);
    if saved.epoch != current.epoch
        || saved.revision != current.revision
        || saved.query != current.query
        || saved.scope != current.scope
        || saved.ranked_digest != current.ranked_digest
    {
        Err("cursor/scope/global rank authority changed; no old index replay".into())
    } else {
        Ok(())
    }
}
