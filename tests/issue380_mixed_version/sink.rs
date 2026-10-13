#[derive(Default)]
struct Sink(Vec<String>);
impl GlobalIndexSink for Sink {
 fn write_batch(&mut self,e:&[GlobalEntryInput])->Result<usize,GlobalIndexError>{self.0.push(format!("entries:{}",e.len()));Ok(e.len())}
 fn mark_entry_stale(&mut self,id:&str)->Result<(),GlobalIndexError>{self.0.push(format!("entry_stale:{id}"));Ok(())}
 fn checkpoint(&mut self,_:&str,_:Option<&str>,c:Option<&str>)->Result<(),GlobalIndexError>{self.0.push(format!("checkpoint:{c:?}"));Ok(())}
 fn set_source_state(&mut self,_:&str,s:&str,_:Option<&str>)->Result<(),GlobalIndexError>{self.0.push(format!("state:{s}"));Ok(())}
 fn set_source_provider(&mut self,_:&str,p:&str)->Result<(),GlobalIndexError>{self.0.push(format!("provider:{p}"));Ok(())}
 fn resolve_parent_path(&mut self,_:&str,_:&str)->Result<Option<String>,GlobalIndexError>{Ok(None)}
 fn find_entry_by_identity(&mut self,_:&str,_:&str,_:&str,_:&str)->Result<Option<GlobalEntry>,GlobalIndexError>{Ok(None)}
 fn mark_volume_entries_stale(&mut self,id:&str)->Result<(),GlobalIndexError>{if let Some(e)=crate::SQLITE_ERROR.with(|v|v.borrow_mut().take()){return Err(GlobalIndexError::Database(DbError::Sqlite(e)))}self.0.push(format!("volume_stale:{id}"));Ok(())}
}
fn request()->IndexServiceRequest {IndexServiceRequest{protocol_version:3,request_id:"r".into(),command:IndexServiceCommand::Rebuild{source_id:"v".into()},source:Some(crate::volume())}}
