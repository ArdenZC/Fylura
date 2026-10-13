pub fn receive(frames:&[String],cancel:bool)->(String,Vec<String>,usize){
 let mut sink=Sink::default();
 let _cancel=cancel; // Old stream has no per-event cancellation guard.
 service::INPUT.with(|i|*i.borrow_mut()=Some(service::IndexServiceClient{frames:frames.iter().map(|s|serde_json::from_str(s).unwrap()).collect()}));
 let provider=WindowsGlobalIndexProvider{direct:DirectWindowsGlobalIndexProvider,force_direct:false};
 let mut direct=0;
 let r=provider.run_with_fallback(&request(),&mut sink,|_,_|{direct+=1;Ok(())});
 (format!("{r:?}"),sink.0,direct)
}
pub fn busy_decode()->String {format!("{:?}",service_errors::decode(&IndexServiceResponse{protocol_version:3,request_id:"r".into(),ok:false,error_code:Some("index_service_busy".into()),message:None,status:None}))}
