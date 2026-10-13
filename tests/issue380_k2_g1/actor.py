"""Controlled SQLite/native API actor. TEST_MODEL; never a shipped Zen Canvas binary."""
import json
import os
from pathlib import Path
import sqlite3
import sys
import time
import traceback
from native import WINDOWS, identities, win_open
import native

root = Path(sys.argv[1]).resolve(strict=True)
conn = None
handles = []
fds = []


def owned(value):
    p = Path(value).absolute()
    # Resolve aliases, too. All alias targets must remain inside this fixture.
    p.resolve().relative_to(root)
    return p


def execute(r):
    global conn
    op = r['op']
    if op == 'hello':
        return dict(pid=os.getpid(), parent=os.getppid(), executable=sys.executable,
                    actor=__file__, sqlite=sqlite3.sqlite_version, uid=getattr(os, 'getuid', lambda: None)())
    if op == 'open':
        p = owned(r['path'])
        if r.get('readonly'):
            conn = sqlite3.connect(p.as_uri() + '?mode=ro', uri=True, timeout=.15)
        else:
            conn = sqlite3.connect(p, timeout=.15)
        conn.execute('PRAGMA busy_timeout=150')
        return dict(journal=conn.execute('PRAGMA journal_mode').fetchone()[0], identities=identities(p))
    if op == 'init':
        conn.execute('PRAGMA journal_mode=WAL')
        conn.execute('PRAGMA wal_autocheckpoint=0')
        conn.execute('CREATE TABLE IF NOT EXISTS corrections(id TEXT PRIMARY KEY, value TEXT)')
        conn.commit()
        return dict(journal=conn.execute('PRAGMA journal_mode').fetchone()[0])
    if op == 'write':
        conn.execute('INSERT INTO corrections VALUES (?,?)', (r['id'], r['value']))
        conn.commit()
        return dict(acknowledged=True, id=r['id'], rows=conn.execute('SELECT * FROM corrections ORDER BY id').fetchall())
    if op == 'read':
        return dict(rows=conn.execute('SELECT * FROM corrections ORDER BY id').fetchall(),
                    integrity=conn.execute('PRAGMA integrity_check').fetchone()[0])
    if op == 'backup':
        p = owned(r['target'])
        deadline = time.monotonic() + 3
        def progress(status, remaining, total):
            if time.monotonic() > deadline:
                raise TimeoutError('bounded SQLite backup did not finish')
        dest = sqlite3.connect(p, timeout=.15)
        try:
            conn.backup(dest, pages=4, progress=progress, sleep=.01)
            rows = dest.execute('SELECT * FROM corrections ORDER BY id').fetchall()
            integrity = dest.execute('PRAGMA integrity_check').fetchone()[0]
            return dict(api='sqlite3_backup', rows=rows, integrity=integrity, target=identities(p))
        finally:
            dest.close()
    if op == 'prepare':
        # Separate-generation toy migration only, never the production schema.
        conn.execute('CREATE TABLE fixture_generation(generation INTEGER)')
        conn.execute('INSERT INTO fixture_generation VALUES (2)')
        conn.commit()
        return dict(test_model=True, generation=2)
    if op == 'close':
        if conn is not None:
            conn.close()
            conn = None
        return dict(closed=True)
    if op == 'raw_same_bytes':
        p = owned(r['path'])
        fd = os.open(p, os.O_RDWR)
        try:
            before = os.read(fd, 16)
            os.lseek(fd, 0, os.SEEK_SET)
            count = os.write(fd, before)
            os.fsync(fd)
            os.lseek(fd, 0, os.SEEK_SET)
            after = os.read(fd, 16)
            return dict(api='open(O_RDWR)/read/write/fsync', fd=fd, written=count,
                        bytes_unchanged=before == after, sqlite_commit=False,
                        identity=identities(p))
        finally:
            os.close(fd)
    if op == 'identity':
        return identities(owned(r['path']))
    if op == 'win_gate':
        results = []
        for s in r['suffixes']:
            h, result = win_open(owned(r['path'] + s), r['access'], r['share'])
            results.append(result)
            if h is not None:
                handles.append(h)
        return dict(handles=results, all_acquired=all(x['ok'] for x in results))
    if op == 'posix_gate':
        import fcntl
        results = []
        for s in r['suffixes']:
            p = owned(r['path'] + s)
            fd = os.open(p, os.O_RDWR)
            fds.append(fd)
            try:
                if r['kind'] == 'flock':
                    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                else:
                    fcntl.lockf(fd, fcntl.LOCK_EX | fcntl.LOCK_NB, 0, 0)
                results.append(dict(api=r['kind'], path=str(p), fd=fd, acquired=True,
                                    device=os.fstat(fd).st_dev, inode=os.fstat(fd).st_ino))
            except OSError as e:
                results.append(dict(api=r['kind'], path=str(p), fd=fd, acquired=False, errno=e.errno))
        return dict(handles=results, all_acquired=all(x['acquired'] for x in results))
    if op == 'release':
        results = []
        for h in handles:
            results.append(dict(handle=h, CloseHandle=bool(native.k.CloseHandle(h))))
        handles.clear()
        for fd in fds:
            os.close(fd)
            results.append(dict(fd=fd, closed=True))
        fds.clear()
        return dict(released=results)
    if op == 'exit':
        return dict(exit=True)
    raise ValueError(op)


for line in sys.stdin:
    try:
        request = json.loads(line)
        result = execute(request)
        response = dict(ok=True, result=result)
    except Exception as e:
        response = dict(ok=False, error=type(e).__name__, message=str(e),
                        sqlite_errorcode=getattr(e, 'sqlite_errorcode', None),
                        sqlite_errorname=getattr(e, 'sqlite_errorname', None),
                        errno=getattr(e, 'errno', None), winerror=getattr(e, 'winerror', None))
    print(json.dumps(response), flush=True)
    if request.get('op') == 'exit':
        break
