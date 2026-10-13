use std::collections::VecDeque;
use std::cell::RefCell;
pub struct IndexServiceClient { pub frames: VecDeque<IndexServiceFrame> }
pub type IndexServiceServerConnection = super::Wire;
thread_local! {pub static INPUT: RefCell<Option<IndexServiceClient>> = const {RefCell::new(None)};}
fn connect()->Result<IndexServiceClient,String>{ INPUT.with(|i|i.borrow_mut().take().ok_or("not_sent".into())) }
impl IndexServiceClient {
 fn send_request(&mut self,_:&IndexServiceRequest)->Result<(),String>{Ok(())}
 fn read_frame(&mut self)->Result<IndexServiceFrame,String>{self.frames.pop_front().ok_or("disconnect".into())}
 pub fn send_lookup_response(&mut self,_:IndexServiceLookupResponse)->Result<(),String>{Ok(())}
}
