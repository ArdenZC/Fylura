// Synthetic enumerator only: real host admission, cancellation and PipeSink execute unchanged.
impl GlobalIndexProvider for DirectWindowsGlobalIndexProvider {
 fn discover_sources(&self)->Result<Vec<GlobalSourceDescriptor>,GlobalIndexError>{Ok(vec![GlobalSourceDescriptor{volume:crate::volume()}])}
 fn start_initial_index(&self,s:&GlobalSourceDescriptor,k:&mut dyn GlobalIndexSink,c:&AtomicBool)->Result<(),GlobalIndexError>{
  crate::CALLS.fetch_add(1,Ordering::SeqCst);
  if c.load(Ordering::Acquire){return Err(GlobalIndexError::Paused)}
  k.set_source_provider(&s.volume.id,&s.volume.provider)?;
  k.mark_volume_entries_stale(&s.volume.id)?;
  k.checkpoint(&s.volume.id,Some("journal"),Some("next"))?;
  Ok(())
 }
 fn resume_incremental_sync(&self,s:&GlobalSourceDescriptor,k:&mut dyn GlobalIndexSink,c:&AtomicBool)->Result<(),GlobalIndexError>{self.start_initial_index(s,k,c)}
 fn pause(&self)->Result<(),GlobalIndexError>{Ok(())}
 fn status(&self)->Result<String,GlobalIndexError>{Ok("ready".into())}
 fn shutdown(&self)->Result<(),GlobalIndexError>{Ok(())}
}
