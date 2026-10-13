#![allow(dead_code,unused_imports)]
mod models;
mod old;
mod new;
mod global_index {pub mod models {pub use crate::models::*;}}
#[derive(Debug,thiserror::Error)]
pub enum DbError {
 #[error("sqlite: {0}")] Sqlite(#[from] rusqlite::Error),
 #[error("validation: {0}")] Validation(String),
}
mod qa_trace {pub fn record(_: &str){}}
static CALLS:std::sync::atomic::AtomicUsize=std::sync::atomic::AtomicUsize::new(0);
fn volume()->models::GlobalVolume {models::GlobalVolume{
 id:"v".into(),platform:"windows".into(),stable_volume_id:"stable".into(),display_name:"volume".into(),mount_path:"C:\\".into(),filesystem_type:"NTFS".into(),drive_kind:"fixed".into(),enabled:true,provider:models::PROVIDER_WINDOWS_MFT_USN.into(),index_status:"ready".into(),last_error:None,journal_id:Some("journal".into()),journal_cursor:Some("old".into()),last_full_index_at:Some(10),last_incremental_sync_at:Some(11),entry_count:1,created_at:1,updated_at:2}}
#[test]
fn four_pairs_execute_real_hosts_and_clients(){
 for (service_name,frames) in [("old",old::serve(false,false,false)),("new",new::serve(false,false,false))]{
  let o=old::receive(&frames,false);let n=new::receive(&frames,false);
  println!("MATRIX old/{service_name} {o:?}"); println!("MATRIX new/{service_name} {n:?}");
  assert_eq!(o.0,"Ok(())");assert!(o.1.iter().any(|s|s=="volume_stale:v"));
  if service_name=="old"{assert!(n.0.contains("ServiceAdmissionMissing"));assert!(n.1.is_empty());assert_eq!(n.2,0)}
  else{assert_eq!(n.0,"Ok(())");assert!(n.1.contains(&"state:syncing".into()));}
 }
}
#[test]
fn busy_matrix_and_typed_decode(){
 for frames in [old::serve(true,false,false),new::serve(true,false,false)]{
  let o=old::receive(&frames,false);let n=new::receive(&frames,false);
  assert!(o.0.contains("Provider"));assert!(n.0.contains("ServiceBusy"));assert!(o.1.is_empty()&&n.1.is_empty());assert_eq!((o.2,n.2),(0,0));
 }
 assert!(old::busy_decode().contains("Provider"));assert_eq!(new::busy_decode(),"ServiceBusy");
}
#[test]
fn disconnect_partial_send_and_not_sent(){
 let frames=new::serve(false,false,false);
 let partial=&frames[..3]; // Admission, provider, stale have already applied.
 let o=old::receive(partial,false);let n=new::receive(partial,false);
 println!("AFTER_STALE disconnect old={o:?} new={n:?}");
 assert_eq!(o.2,1);assert!(o.1.contains(&"state:unavailable".into()));
 assert_eq!(n.2,0);assert!(n.0.contains("ServiceOutcomeUnknown"));assert!(n.1.contains(&"volume_stale:v".into()));
 let p=new::receive_fault(&frames,false,true,false);assert!(p.0.contains("ServiceOutcomeUnknown"));assert_eq!(p.2,0);assert!(p.1.is_empty());
 let pre=new::receive(&[],false);assert!(pre.0.contains("ServiceOutcomeUnknown"));assert_eq!(pre.2,0);
 let ns=new::receive_fault(&[],false,false,true);println!("NOT_SENT {ns:?}");assert_eq!(ns.2,1);
}
#[test]
fn old_request_identity_is_ignored_new_identity_is_fenced(){
 let mut frames=new::serve(false,false,false);
 for f in &mut frames {let mut j:serde_json::Value=serde_json::from_str(f).unwrap();if j["kind"]=="event"{j["request_id"]="old-request".into()}else{j["response"]["request_id"]="old-request".into()}*f=j.to_string();}
 let o=old::receive(&frames,false);let n=new::receive(&frames,false);
 assert_eq!(o.0,"Ok(())");assert!(!o.1.is_empty());assert!(n.0.contains("ServiceOutcomeUnknown"));assert!(n.1.is_empty());
}
#[test]
fn service_restart_and_same_request_replay_are_not_durable_fenced(){
 if std::env::var_os("ZC380_RESTART_CHILD").is_some(){
  let old=old::serve(false,false,false);let new=new::serve(false,false,false);
  println!("CHILD_REQUEST_r_OLD {}",serde_json::to_string(&old).unwrap());
  println!("CHILD_REQUEST_r_NEW {}",serde_json::to_string(&new).unwrap());
  assert_eq!(CALLS.load(std::sync::atomic::Ordering::SeqCst),2);return
 }
 // Real process restart with identical request ID, using frozen host logic.
 let mut outputs=Vec::new();
 for _ in 0..2{
  let result=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","service_restart_and_same_request_replay_are_not_durable_fenced","--nocapture"]).env("ZC380_RESTART_CHILD","1").output().unwrap();
  assert!(result.status.success());
  let output=String::from_utf8(result.stdout).unwrap();
  let lines=output.lines().filter(|s|s.starts_with("CHILD_REQUEST_r_")).map(str::to_owned).collect::<Vec<_>>();
  assert_eq!(lines.len(),2);outputs.push(lines);
 }
 assert_eq!(outputs[0],outputs[1]);println!("PROCESS_RESTART identical request r executed by old and new hosts in two separate processes");
 assert_eq!(old::restart_replay(),2);assert_eq!(new::restart_replay(),2);
 for serve in [old::serve,new::serve]{let start=CALLS.load(std::sync::atomic::Ordering::SeqCst);serve(false,false,true);assert_eq!(CALLS.load(std::sync::atomic::Ordering::SeqCst)-start,2);}
}
#[test]
fn cancellation_and_resume_replay(){
 let frames=new::serve(false,false,false);
 let o=old::receive(&frames,true);let n=new::receive(&frames,true);
 assert_eq!(o.0,"Ok(())");assert!(!o.1.is_empty());assert!(n.0.contains("Paused"));assert!(n.1.is_empty());
 for serve in [old::serve,new::serve]{let cancelled=serve(false,true,false);assert!(cancelled.last().unwrap().contains("index_paused"));let resumed=serve(false,false,false);assert!(resumed.last().unwrap().contains("\"ok\":true"));}
}
#[test]
fn wire_has_no_client_generation_or_required_capabilities(){
 let r=old::service::IndexServiceRequest{protocol_version:3,request_id:"r".into(),command:old::service::IndexServiceCommand::Rebuild{source_id:"v".into()},source:Some(volume())};
 let j=serde_json::to_value(r).unwrap();assert_eq!(j.as_object().unwrap().len(),4);
 let n:new::service::IndexServiceRequest=serde_json::from_value(j).unwrap();assert!(new::service::validate_request(&n).is_ok());
 let event=new::service::IndexServiceEvent::EntryStale{volume_id:"v".into(),entry_id:"e".into()};
 let old:old::service::IndexServiceEvent=serde_json::from_value(serde_json::to_value(event).unwrap()).unwrap();assert!(matches!(old,old::service::IndexServiceEvent::EntryStale{..}));
}

#[test]
fn real_coordinator_rebuild_mutates_before_admission_in_both_versions(){
 for probe in [old::rebuild_probe,new::rebuild_probe]{
  for disabled in [false,true]{for all in [false,true]{
   let values=probe(disabled,all);println!("PRE_ADMISSION disabled={disabled} all={all} {values:?}");
   assert_eq!(values,vec!["rebuild_required","1","0","stale","stale","stale","0","old","retained summary","1"]);
  }}
 }
}
#[test]
fn sqlite_busy_is_distinct_from_service_busy(){println!("BUNDLED_SQLITE {}",rusqlite::version());assert_eq!(old::sqlite_busy_code(),"index_failed");assert_eq!(new::sqlite_busy_code(),"index_database_busy");assert_eq!(new::busy_decode(),"ServiceBusy");}
#[test]
fn real_service_admission_cas_allows_one_contender(){assert_eq!(old::race_admission(),(1,7));assert_eq!(new::race_admission(),(1,7));}
#[test]
fn response_protocol_and_source_mismatch_matrix(){
 let original=new::serve(false,false,false);
 for mismatch in ["protocol","source","request"]{
  let mut frames=original.clone();
  for f in &mut frames{
   let mut j:serde_json::Value=serde_json::from_str(f).unwrap();
   if mismatch=="protocol" && j["kind"]=="response"{j["response"]["protocol_version"]=99.into();}
   if mismatch=="source" && j["kind"]=="event"{if j["event"]["event"]=="sources"{j["event"]["sources"][0]["id"]="other".into();}else if j["event"].get("volume_id").is_some(){j["event"]["volume_id"]="other".into();}}
   if mismatch=="request" && j["kind"]=="response"{j["response"]["request_id"]="replayed".into();}
   *f=j.to_string();
  }
  let o=old::receive(&frames,false);let n=new::receive(&frames,false);
  println!("MISMATCH {mismatch} old={o:?} new={n:?}");
  assert_eq!(o.0,"Ok(())");assert!(n.0.contains("ServiceOutcomeUnknown"));assert_eq!(n.2,0);
  if mismatch=="source"{assert!(n.1.is_empty())}else{assert!(n.1.contains(&"volume_stale:v".into()))}
 }
}
#[test]
fn valid_old_request_replay_is_accepted_again_by_new_service_and_new_client(){
 let frames=new::serve(false,false,false);
 assert_eq!(new::receive(&frames,false).0,"Ok(())");
 // A fresh admission validator is scoped to this call, not a durable generation.
 assert_eq!(new::receive(&frames,false).0,"Ok(())");
}
thread_local! {static SQLITE_ERROR:std::cell::RefCell<Option<rusqlite::Error>>=const {std::cell::RefCell::new(None)};}
fn actual_busy_error()->rusqlite::Error {
 let path=std::env::temp_dir().join(format!("zc380-lock-{}.sqlite",uuid::Uuid::new_v4()));
 let c=rusqlite::Connection::open(&path).unwrap();
 c.execute("CREATE TABLE witness(x)",[]).unwrap();
 c.busy_timeout(std::time::Duration::from_millis(20)).unwrap();
 let locker=rusqlite::Connection::open(&path).unwrap();
 locker.execute_batch("BEGIN IMMEDIATE").unwrap();
 let error=c.execute("INSERT INTO witness VALUES(1)",[]).unwrap_err();
 assert!(matches!(&error,rusqlite::Error::SqliteFailure(e,_) if e.code==rusqlite::ErrorCode::DatabaseBusy));
 locker.execute_batch("ROLLBACK").unwrap();drop(locker);drop(c);std::fs::remove_file(path).unwrap();error
}
#[test]
fn actual_sqlite_sink_failure_causes_old_replay_but_new_preserves_database_error(){
 let frames=new::serve(false,false,false);
 SQLITE_ERROR.with(|v|*v.borrow_mut()=Some(actual_busy_error()));
 let o=old::receive(&frames,false);assert_eq!(o.2,1);
 SQLITE_ERROR.with(|v|*v.borrow_mut()=Some(actual_busy_error()));
 let n=new::receive(&frames,false);println!("SQLITE_BUSY_SINK old={o:?} new={n:?}");
 assert!(n.0.contains("Database(Sqlite(SqliteFailure"));assert_eq!(n.2,0);assert!(!n.0.contains("ServiceBusy"));
}
