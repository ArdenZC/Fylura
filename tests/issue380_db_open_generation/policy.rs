//! IO-classification shell for executing the frozen old/new fallback functions.
use std::sync::atomic::AtomicBool;
#[derive(Debug)] enum GlobalIndexError { Provider(String),ServiceBusy,ServiceAdmissionMissing(String),ServiceOutcomeUnknown(String),DatabaseBusy(String) }
#[derive(Debug)] enum IndexServiceCallError { MissingAdmission(String),NotSent(String),InvalidRequest(String),SinkFailure(GlobalIndexError),UnknownOutcome(String) }
struct Source{id:String} struct IndexServiceRequest{source:Option<Source>}
trait GlobalIndexSink {fn set_source_state(&mut self,id:&str,state:&str,error:Option<&str>)->Result<(),GlobalIndexError>;}
struct Sink(Vec<String>); impl GlobalIndexSink for Sink{fn set_source_state(&mut self,_:&str,state:&str,_:Option<&str>)->Result<(),GlobalIndexError>{self.0.push(state.into());Ok(())}}
struct DirectWindowsGlobalIndexProvider;
struct Provider{direct:DirectWindowsGlobalIndexProvider,force_direct:bool,fault:String}
impl Provider {
 fn service_failed(&self){}
 fn run_service_stream(&self,_:&IndexServiceRequest,_:&mut dyn GlobalIndexSink)->Result<Result<(),GlobalIndexError>,String>{
  match self.fault.as_str(){"busy"=>Ok(Err(GlobalIndexError::Provider("index_service_busy".into()))),"sqlite_busy"=>Err("sink database is locked".into()),_=>Err(self.fault.clone())}
 }
}
pub fn route(root:std::path::PathBuf,version:&str,fault:&str)->serde_json::Value{
 let provider=Provider{direct:DirectWindowsGlobalIndexProvider,force_direct:false,fault:fault.into()};
 let request=IndexServiceRequest{source:Some(Source{id:"fixture".into()})};
 let mut sink=Sink(Vec::new());let mut opened=false;
 let direct=|_:&DirectWindowsGlobalIndexProvider,_:&mut dyn GlobalIndexSink|{
  let app=crate::AppHandle::<crate::MockRuntime>{root:root.clone(),r:std::marker::PhantomData};
  crate::open_database(&app).map_err(GlobalIndexError::Provider)?;opened=true;Ok(())
 };
 let result=if version=="old"{provider.run_with_fallback(&request,&mut sink,direct)}else{
  provider.run_with_fallback_using(&request,&mut sink,&AtomicBool::new(false),direct,|_,_,_,_|match fault{
   "busy"=>Ok(Err(GlobalIndexError::ServiceBusy)),
   "sqlite_busy"=>Err(IndexServiceCallError::SinkFailure(GlobalIndexError::DatabaseBusy("fixture SQLite BUSY classification at IO seam".into()))),
   "not_sent"=>Err(IndexServiceCallError::NotSent("connect failed before dispatch".into())),
   _=>Err(IndexServiceCallError::UnknownOutcome("post-send transport loss".into()))
  })
 };
 serde_json::json!({"generation":version,"fault":fault,"direct_opened_database":opened,"sink_states":sink.0,"outcome":format!("{result:?}"),"wire_transport":"classification injection; not real named pipe"})
}
