#![allow(dead_code,private_interfaces)]
use std::path::PathBuf;
mod db; mod policy;
mod global_index {pub mod models {pub const INDEX_STATUS_UNAVAILABLE:&str="unavailable";}}
use db::{Database,DbError};
// Only Tauri path acquisition is replaced. All supplied roots are disposable.
trait Runtime {}
struct MockRuntime; impl Runtime for MockRuntime {}
struct AppHandle<R:Runtime>{root:PathBuf,r:std::marker::PhantomData<R>}
struct PathResolver(PathBuf);
impl PathResolver{fn app_data_dir(&self)->Result<PathBuf,String>{Ok(self.0.clone())}}
impl<R:Runtime> AppHandle<R>{fn path(&self)->PathResolver{PathResolver(self.root.clone())}}
mod settings {
 pub const APP_SETTINGS_KEY:&str="app_settings_v1";
 // Empty scan-folder policy keeps a fresh isolated migration off user paths.
 // This is the only migration input shim; no migration SQL/guard is replaced.
 pub fn default_settings_json()->Result<String,crate::DbError>{Ok("{\"defaultScanFolders\":[]}".into())}
}
fn main(){
 let args=std::env::args().collect::<Vec<_>>();
 let app=AppHandle::<MockRuntime>{root:PathBuf::from(&args[1]),r:std::marker::PhantomData};
 if args.get(2).map(String::as_str)==Some("route"){
  println!("{}",policy::route(app.root,&args[3],&args[4]));return
 }
 let path=database_path(&app).unwrap();
 if args.get(2).map(String::as_str)==Some("path"){
  println!("{}",serde_json::json!({"path":path,"native_qa":cfg!(feature="native-qa")}));return
 }
 // Hypothetical post-open admission rejection cannot undo the call below.
 match open_database(&app){
  Ok(db)=>{
   let c=db.conn().unwrap();
   let version:i64=c.query_row("PRAGMA user_version",[],|r|r.get(0)).unwrap();
   let tables:i64=c.query_row("SELECT count(*) FROM sqlite_master WHERE type='table'",[],|r|r.get(0)).unwrap();
   let timeout:i64=c.query_row("PRAGMA busy_timeout",[],|r|r.get(0)).unwrap();
   println!("{}",serde_json::json!({"path":db.path(),"user_version":version,"persistent_tables":tables,"busy_timeout_ms":timeout,"sqlite":rusqlite::version(),"post_open_admission":"hypothetical_reject_after_migration"}));
  },
  Err(e)=>{eprintln!("{e}");std::process::exit(23)}
 }
}
