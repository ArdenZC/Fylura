pub fn receive(frames:&[String],cancel:bool)->(String,Vec<String>,usize){receive_fault(frames,cancel,false,false)}
pub fn receive_fault(frames:&[String],cancel:bool,partial:bool,not_sent:bool)->(String,Vec<String>,usize){
 let mut sink=Sink::default();
 let provider=WindowsGlobalIndexProvider{direct:DirectWindowsGlobalIndexProvider,force_direct:false};
 let mut direct=0;
 let r=provider.run_with_fallback_using(&request(),&mut sink,&AtomicBool::new(cancel),|_,_|{direct+=1;Ok(())},|p,r,s,c|{
  if not_sent{return Err(IndexServiceCallError::NotSent("connect_failed".into()))}
  let mut client=service::Client{frames:frames.iter().map(|s|serde_json::from_str(s).unwrap()).collect(),partial};
  p.run_service_stream_with_client(r,s,c,&mut client)
 });
 (format!("{r:?}"),sink.0,direct)
}
pub fn busy_decode()->String {format!("{:?}",service_errors::decode(&IndexServiceResponse{protocol_version:3,request_id:"r".into(),ok:false,error_code:Some("index_service_busy".into()),message:None,status:None}))}
