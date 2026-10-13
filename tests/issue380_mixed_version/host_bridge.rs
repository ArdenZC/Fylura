#[derive(Default)]
pub struct Wire {pub frames:Vec<String>,pub request_id:String,pub cancel:Option<Arc<AtomicBool>>}
impl Wire {
 fn send_event(&mut self,event:IndexServiceEvent)->Result<(),String>{
  self.frames.push(serde_json::to_string(&IndexServiceFrame::Event{request_id:self.request_id.clone(),event}).unwrap());
  if let Some(c)=&self.cancel{c.store(true,Ordering::Release)}
  Ok(())
 }
 fn send_response(&mut self,response:IndexServiceResponse)->Result<(),String>{self.frames.push(serde_json::to_string(&IndexServiceFrame::Response{response}).unwrap());Ok(())}
 fn read_lookup_response(&mut self,_:&str)->Result<IndexServiceLookupResponse,String>{Err("unused lookup".into())}
}
// New host uses its production IO trait; the old host uses the concrete connection shim.
pub fn serve(busy:bool,cancel:bool,replay:bool)->Vec<String>{
 let runtime=ServiceRuntime::with_provider(Arc::new(DirectWindowsGlobalIndexProvider));
 runtime.busy.store(busy,Ordering::Release);
 let mut wire=Wire{request_id:"r".into(),..Default::default()};
 if cancel{wire.cancel=Some(runtime.cancel.clone())}
 runtime.handle_request(request(),&mut wire).unwrap();
 if replay{runtime.handle_request(request(),&mut wire).unwrap();}
 wire.frames
}
pub fn restart_replay()->usize {
 let start=crate::CALLS.load(Ordering::SeqCst);
 serve(false,false,false);
 serve(false,false,false); // Fresh runtime has no durable record of request r.
 crate::CALLS.load(Ordering::SeqCst)-start
}
pub fn race_admission()->(usize,usize){
 let runtime=Arc::new(ServiceRuntime::new());
 let barrier=Arc::new(std::sync::Barrier::new(8));
 let threads=(0..8).map(|_|{let r=runtime.clone();let b=barrier.clone();std::thread::spawn(move||{b.wait();r.try_begin_index_request(&request()).is_ok()})}).collect::<Vec<_>>();
 let accepted=threads.into_iter().map(|t|usize::from(t.join().unwrap())).sum::<usize>();
 (accepted,8-accepted)
}
