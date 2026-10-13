use std::collections::VecDeque;
pub struct Client {pub frames:VecDeque<IndexServiceFrame>,pub partial:bool}
impl IndexServiceClientIo for Client {
 fn send_request(&mut self,_:&IndexServiceRequest)->Result<(),String>{if self.partial {Err("partial_write".into())}else{Ok(())}}
 fn read_frame(&mut self)->Result<IndexServiceFrame,String>{self.frames.pop_front().ok_or("disconnect".into())}
 fn send_lookup_response(&mut self,_:IndexServiceLookupResponse)->Result<(),String>{Ok(())}
}
