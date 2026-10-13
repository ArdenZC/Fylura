PRAGMA foreign_keys=ON;
INSERT INTO global_volumes(id,platform,stable_volume_id,display_name,mount_path,filesystem_type,drive_kind,provider,index_status,last_error,journal_id,journal_cursor,last_full_index_at,last_incremental_sync_at,created_at,updated_at)
VALUES('v','windows','stable','volume','C:\','NTFS','fixed','windows_mft_usn','ready','prior diagnostic','journal','old',10,11,1,2);
INSERT INTO global_entries(id,volume_id,platform_file_id,name,name_normalized,path,path_normalized,extension,size,created_at_fs,modified_at_fs,file_attributes,is_hidden,source_provider,last_seen_at)
VALUES('e','v','identity','needle.txt','needle.txt','C:\needle.txt','c:\needle.txt','txt',123,10,11,32,1,'windows_mft_usn',12);
INSERT INTO managed_scopes(id,path,global_entry_id,created_at,updated_at)VALUES('scope','C:\','e',1,2);
INSERT INTO managed_entries(id,global_entry_id,managed_scope_id,created_at,updated_at)VALUES('managed','e','scope',1,2);
INSERT INTO ai_analysis_state(global_entry_id,status,input_fingerprint,content_summary,classification_json,user_corrected,updated_at)VALUES('e','completed','fp','retained summary','{"user":"retained"}',1,2);
INSERT INTO ai_jobs(id,global_entry_id,managed_scope_id,input_fingerprint,provider,status,created_at)VALUES('job','e','scope','fp','local','running',1);
INSERT INTO ai_job_items(id,job_id,global_entry_id,status,created_at,updated_at)VALUES('item','job','e','running',1,2);
