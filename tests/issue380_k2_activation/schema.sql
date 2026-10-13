-- Test-only representative user state; NOT Schema 37 or a migration proposal.
PRAGMA foreign_keys=ON;
CREATE TABLE files(id TEXT PRIMARY KEY,path TEXT NOT NULL,context TEXT NOT NULL,consent INTEGER NOT NULL);
CREATE TABLE tags(id TEXT PRIMARY KEY,name TEXT NOT NULL);
CREATE TABLE file_tags(file_id TEXT REFERENCES files(id),tag_id TEXT REFERENCES tags(id),PRIMARY KEY(file_id,tag_id));
CREATE TABLE history(id INTEGER PRIMARY KEY,file_id TEXT REFERENCES files(id),decision TEXT NOT NULL);
CREATE TABLE t0_commits(seq INTEGER PRIMARY KEY,actor TEXT NOT NULL,phase TEXT NOT NULL,value TEXT NOT NULL);
CREATE VIRTUAL TABLE files_fts USING fts5(context,content='files',content_rowid='rowid');
CREATE TRIGGER files_ai AFTER INSERT ON files BEGIN INSERT INTO files_fts(rowid,context)VALUES(new.rowid,new.context); END;
CREATE TRIGGER files_au AFTER UPDATE OF context ON files BEGIN INSERT INTO files_fts(files_fts,rowid,context)VALUES('delete',old.rowid,old.context);INSERT INTO files_fts(rowid,context)VALUES(new.rowid,new.context); END;
INSERT INTO files VALUES('stable-id','owned-fixture.txt','USER_SEED',1);
INSERT INTO tags VALUES('tag-id','user tag');INSERT INTO file_tags VALUES('stable-id','tag-id');INSERT INTO history VALUES(1,'stable-id','Keep');
