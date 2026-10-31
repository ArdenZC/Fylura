"""Noncooperative legacy behavior model; never reads activation/mutex/generation.

The nonce only confines the harness to an owned disposable directory. This is
not an old Zen Canvas binary, SCM service or installer implementation.
"""
import json,os,pathlib,shutil,sqlite3,sys,time
namespace=pathlib.Path(sys.argv[1]).resolve();root=pathlib.Path(sys.argv[2]).resolve()
assert namespace.name=='issue380-k2-t0' and namespace.parent.name=='.tmp-tests'
assert root.is_relative_to(namespace) and root!=namespace
assert json.loads((root/'fixture-owner.json').read_text())['nonce']==sys.argv[3]
role=sys.argv[4]
def owned(name):
    p=root/name;assert p.resolve().is_relative_to(root),'path escapes fixture';return p
path=owned(sys.argv[5])
def identity(p):
    try:
        s=p.stat();return {'path':str(p),'device':s.st_dev,'inode':s.st_ino,'links':s.st_nlink,'size':s.st_size,'mode':oct(s.st_mode&0o777)}
    except FileNotFoundError:return {'path':str(p),'exists':False}
def connect():
    c=sqlite3.connect(path,isolation_level=None,timeout=.1)
    c.execute('PRAGMA foreign_keys=ON');c.execute('PRAGMA wal_autocheckpoint=0');return c
c=connect();main_handle=os.open(path,os.O_RDONLY);file_handle=None
def emit(v):print(json.dumps(v),flush=True)
def physical():
    fd=os.fstat(main_handle)
    return {'main_path':identity(path),'opened_main_handle':{'kind':'duplicate O_RDONLY identity probe; not SQLite internal FD','device':fd.st_dev,'inode':fd.st_ino,'links':fd.st_nlink},'wal':identity(pathlib.Path(str(path)+'-wal')),'shm':identity(pathlib.Path(str(path)+'-shm'))}
emit({'ready':True,'pid':os.getpid(),'uid':os.getuid(),'euid':os.geteuid(),'role':role,'program':str(pathlib.Path(__file__).resolve()),'sqlite':sqlite3.sqlite_version,'identity':physical()})
for raw in sys.stdin:
    p=json.loads(raw);op=p['op'];start=time.monotonic()
    try:
        if op=='commit':
            c.execute('BEGIN IMMEDIATE')
            c.execute("UPDATE files SET context=? WHERE id='stable-id'",(p['value'],))
            c.execute('INSERT INTO t0_commits(actor,phase,value)VALUES(?,?,?)',(role,p['phase'],p['value']))
            c.execute('COMMIT');value=c.execute('SELECT seq,actor,phase,value FROM t0_commits ORDER BY seq DESC LIMIT 1').fetchone()
        elif op=='read':value=c.execute("SELECT context FROM files WHERE id='stable-id'").fetchone()[0]
        elif op=='reconnect':
            c.close();os.close(main_handle);c=connect();main_handle=os.open(path,os.O_RDONLY);value='reconnected'
        elif op=='snapshot':
            target=owned(p['target']);b=sqlite3.connect(target,isolation_level=None);c.backup(b);b.close();value={'target':identity(target),'copied_commits':c.execute('SELECT count(*) FROM t0_commits').fetchone()[0]}
        elif op=='migrate_model':
            c.execute('BEGIN IMMEDIATE');c.execute('CREATE TABLE t0_offline_migration(seal TEXT NOT NULL)');c.execute("INSERT INTO t0_offline_migration VALUES('fixture-only')")
            if p.get('crash'):os._exit(73)
            if p.get('fail'):c.execute('INSERT INTO t0_offline_migration VALUES(NULL)')
            c.execute('COMMIT');value='test-only offline transform committed; not production migration'
        elif op=='switch':
            target=owned(p['target']);temp=owned('active.tmp');temp.write_text(json.dumps({'db':target.name,'stage':'ACTIVE_NEW'}))
            with temp.open('r+') as f:os.fsync(f.fileno())
            if p.get('crash')=='before':os._exit(71)
            os.replace(temp,owned('activation.json'))
            if p.get('crash')=='after':os._exit(72)
            fd=os.open(root,os.O_RDONLY);os.fsync(fd);os.close(fd);value=target.name
        elif op=='chmod':os.chmod(path,0o400);value=identity(path)
        elif op=='restore_mode':os.chmod(path,0o600);value=identity(path)
        elif op=='open_file':
            file_handle=os.open(owned(p['name']),os.O_RDWR);s=os.fstat(file_handle);value={'device':s.st_dev,'inode':s.st_ino,'links':s.st_nlink}
        elif op=='rename':os.rename(owned(p['source']),owned(p['target']));value=identity(owned(p['target']))
        elif op=='move':shutil.move(owned(p['source']),owned(p['target']));value=identity(owned(p['target']))
        elif op=='delete':owned(p['name']).unlink();value={'path_absent':not owned(p['name']).exists()}
        elif op=='write_open_handle':
            os.lseek(file_handle,0,os.SEEK_SET);os.write(file_handle,p['value'].encode());os.fsync(file_handle);os.lseek(file_handle,0,os.SEEK_SET)
            s=os.fstat(file_handle);value={'content':os.read(file_handle,1024).decode(),'device':s.st_dev,'inode':s.st_ino,'links':s.st_nlink}
        elif op=='close':c.close();os.close(main_handle);emit({'closed':True});break
        else:raise ValueError(op)
        emit({'op':op,'success':True,'value':value,'pid':os.getpid(),'identity':physical(),'elapsed_ms':round((time.monotonic()-start)*1000,3)})
    except Exception as e:
        if c.in_transaction:c.rollback()
        emit({'op':op,'success':False,'error':str(e),'sqlite_code':getattr(e,'sqlite_errorcode',None),'sqlite_name':getattr(e,'sqlite_errorname',None),'pid':os.getpid(),'identity':physical(),'elapsed_ms':round((time.monotonic()-start)*1000,3)})
if file_handle is not None:os.close(file_handle)
