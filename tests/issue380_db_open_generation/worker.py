"""Legacy SQLite actor with a retained connection and pipe synchronization.

This is not an old application binary, SCM service, or IPC transport. It uses
independent sqlite3 connections and legacy-table SQL in disposable roots.
"""
import json,os,pathlib,sqlite3,sys,time
root=pathlib.Path(sys.argv[1]).resolve(); path=pathlib.Path(sys.argv[2])
assert root.is_relative_to((pathlib.Path(__file__).resolve().parents[2]/'.tmp-tests/issue380-k1').resolve()),'worker outside disposable output root'
assert path.resolve().is_relative_to(root), 'fixture escaped disposable root'
role=sys.argv[3]
def connect():
    c=sqlite3.connect(path,isolation_level=None,timeout=.1)
    c.execute('PRAGMA foreign_keys=ON');c.execute('PRAGMA wal_autocheckpoint=0')
    return c
c=connect()
def emit(value):print(json.dumps(value),flush=True)
emit({'ready':True,'pid':os.getpid(),'role':role,'db':str(path),'sqlite':sqlite3.sqlite_version})
for line in sys.stdin:
    p=json.loads(line);start=time.monotonic()
    try:
        op=p['op']
        if op=='write':
            c.execute('UPDATE files SET context=? WHERE id=?',(p['value'],'stable-file-0'))
            value=c.execute("SELECT context FROM files WHERE id='stable-file-0'").fetchone()[0]
        elif op=='read':value=c.execute("SELECT context FROM files WHERE id='stable-file-0'").fetchone()[0]
        elif op=='begin_write':c.execute('BEGIN IMMEDIATE');value='locked'
        elif op=='begin_read':c.execute('BEGIN');value=c.execute("SELECT context FROM files WHERE id='stable-file-0'").fetchone()[0]
        elif op=='rollback':c.execute('ROLLBACK');value='released'
        elif op=='reconnect':c.close();c=connect();value=c.execute('PRAGMA user_version').fetchone()[0]
        elif op=='rename':
            # Independent file action witness, confined to the disposable root.
            src=root/'owned-file.txt';dst=root/'old-process-renamed.txt'
            src.rename(dst);value={'renamed':dst.exists(),'content':dst.read_text()}
        elif op=='backup':
            target=root/p['name'];assert target.resolve().is_relative_to(root)
            target_c=sqlite3.connect(target,isolation_level=None);c.backup(target_c);target_c.close();value=str(target)
        elif op=='copy_main':
            import shutil
            target=root/p['name'];assert target.resolve().is_relative_to(root)
            shutil.copyfile(path,target);value=str(target)
        elif op=='pointer':
            target=root/p['name'];assert target.resolve().is_relative_to(root)
            tmp=root/'active.tmp';tmp.write_text(str(target.name));
            with tmp.open('r+') as f:os.fsync(f.fileno())
            if p.get('crash')=='before_replace':os._exit(71)
            os.replace(tmp,root/'active.json')
            if p.get('crash')=='after_replace':os._exit(72)
            # POSIX durability model only; not Windows replace/installer proof.
            fd=os.open(root,os.O_RDONLY);os.fsync(fd);os.close(fd);value=target.name
        elif op=='uncommitted_crash':
            c.execute('BEGIN IMMEDIATE');c.execute("UPDATE files SET context='UNCOMMITTED' WHERE id='stable-file-0'");os._exit(73)
        elif op=='committed_crash':
            c.execute("UPDATE files SET context='COMMITTED' WHERE id='stable-file-0'");os._exit(74)
        elif op=='close':c.close();emit({'closed':True});break
        else:raise ValueError(op)
        emit({'op':op,'value':value,'pid':os.getpid(),'role':role,'elapsed_ms':round((time.monotonic()-start)*1000,3)})
    except Exception as e:
        emit({'op':p.get('op'),'error':str(e),'sqlite_code':getattr(e,'sqlite_errorcode',None),'elapsed_ms':round((time.monotonic()-start)*1000,3)})
