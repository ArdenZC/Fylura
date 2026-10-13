fn run_service_stream(&self,request:&IndexServiceRequest,sink:&mut dyn GlobalIndexSink)->Result<Result<(),GlobalIndexError>,String>{
 let response=service::call_stream(request,|client,event|Self::apply_event(sink,client,event))?;
 if response.ok{Ok(Ok(()))}else{Ok(Err(Self::service_response_error(&response)))}
}
