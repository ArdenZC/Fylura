use rusqlite::{params,Connection};
struct Database(Mutex<Connection>);
impl Database {
 fn conn(&self)->Result<std::sync::MutexGuard<'_,Connection>,DbError>{self.0.lock().map_err(|_|DbError::Validation("poison".into()))}
 fn list_global_volumes(&self)->Result<Vec<GlobalVolume>,DbError>{let mut v=crate::volume();v.enabled=self.conn()?.query_row("SELECT enabled FROM global_volumes WHERE id='v'",[],|r|r.get(0))?;Ok(vec![v])}
}
struct Coordinator { db:Database,provider:DirectWindowsGlobalIndexProvider }
impl Coordinator {
 fn start(&self)->Result<(),GlobalIndexError>{
  // Worker/admission seam: check effects exist before any Service invocation.
  assert_eq!(self.db.conn()?.query_row("SELECT is_stale FROM global_entries WHERE id='e'",[],|r|r.get::<_,i64>(0)).map_err(DbError::from)?,1);
  Ok(())
 }
}
pub fn rebuild_probe(disabled:bool,all:bool)->Vec<String>{
 let c=Connection::open_in_memory().unwrap();c.execute_batch(include_str!("schema.sql")).unwrap();c.execute_batch(include_str!("fixture.sql")).unwrap();
 if disabled{c.execute("UPDATE global_volumes SET enabled=0",[]).unwrap();}
 let coordinator=Coordinator{db:Database(Mutex::new(c)),provider:DirectWindowsGlobalIndexProvider};
 coordinator.rebuild(if all{None}else{Some("v".into())}).unwrap();
 let c=coordinator.db.conn().unwrap();
 ["SELECT index_status FROM global_volumes","SELECT CAST(is_stale AS TEXT) FROM global_entries","SELECT CAST(enabled AS TEXT) FROM managed_entries","SELECT status FROM ai_jobs","SELECT status FROM ai_job_items","SELECT status FROM ai_analysis_state","SELECT CAST(entry_count AS TEXT) FROM global_volumes","SELECT journal_cursor FROM global_volumes","SELECT content_summary FROM ai_analysis_state","SELECT CAST(user_corrected AS TEXT) FROM ai_analysis_state"].iter().map(|q|c.query_row(q,[],|r|r.get(0)).unwrap()).collect()
}
pub fn sqlite_busy_code()->String {
 let e=GlobalIndexError::Database(DbError::Sqlite(rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(5),Some("database is locked".into()))));
 service_errors::classify(&e).0.to_owned()
}
