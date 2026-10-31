"""Supplemental timed WAL recovery, online-backup and busy-contract evidence."""
import argparse
import hashlib
import json
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

from prototype import Prototype, connect


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error('output exists')
    evidence = dict(source_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                    sqlite=sqlite3.sqlite_version, recovery=[], writer_controls=[])
    with tempfile.TemporaryDirectory(prefix='issue-380-recovery-') as root:
        root = Path(root)
        for strategy in ('C1', 'C2', 'C3'):
            p = Prototype(root/(strategy+'.db'), strategy, 1000)
            p.stage()
            # Close the parent BEFORE spawning: the crashing child must be the
            # last SQLite connection, so no orderly keeper close checkpoints WAL.
            p.c.close()
            child = subprocess.run([sys.executable, '-c', '''
import os,sys
from prototype import Prototype
p=Prototype(sys.argv[1],sys.argv[2])
p.publish(step_hook=lambda step: os._exit(73) if step in ('ai_job_items','pointer') else None)
''', p.path, strategy], cwd=Path(__file__).parent, timeout=30)
            assert child.returncode == 73
            # No reader connection survives recovery; force reopen from WAL files.
            start = time.perf_counter()
            p.c = connect(p.path)
            assert p.read() == p.old
            recovery_ms = (time.perf_counter()-start)*1000
            p.publish()
            backup = root/(strategy+'-backup.db')
            target = sqlite3.connect(backup)
            start = time.perf_counter()
            p.c.backup(target)
            backup_ms = (time.perf_counter()-start)*1000
            target.close()
            restored = connect(backup)
            assert p.read(restored) == p.expected
            for table, expected in p.history.items():
                assert [dict(r) for r in restored.execute(f'SELECT * FROM {table} ORDER BY 1')] == expected
            assert restored.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
            restored.close()
            evidence['recovery'].append(dict(strategy=strategy, rows=1000,
                  recovery_open_and_full_projection_ms=recovery_ms,
                  backup_ms=backup_ms, backup_bytes=backup.stat().st_size, restored_projection_equal=True))
            p.close()
        p = Prototype(root/'writer.db', 'C2', 1)
        for hold in (.05, 5.25):
            ready, attempt = threading.Event(), threading.Event()
            go = threading.Event()
            result = dict(hold_requested_ms=hold*1000)
            def writer():
                c = connect(p.path)
                ready.set()
                assert go.wait(2)
                start = time.perf_counter()
                attempt.set()
                try:
                    c.execute('BEGIN IMMEDIATE')
                    c.execute("INSERT INTO app_settings VALUES('control','ok') ON CONFLICT(key) DO UPDATE SET value='ok'")
                    c.execute('COMMIT')
                    result['outcome'] = 'committed'
                except sqlite3.OperationalError as e:
                    result.update(outcome='sqlite_busy', code=e.sqlite_errorcode)
                finally:
                    result['elapsed_ms'] = (time.perf_counter()-start)*1000
                    c.close()
            t = threading.Thread(target=writer)
            t.start()
            assert ready.wait(2)
            p.c.execute('BEGIN IMMEDIATE')
            acquired = time.perf_counter()
            go.set()
            assert attempt.wait(1)
            time.sleep(hold)
            p.c.execute('COMMIT')
            result['holder_ms'] = (time.perf_counter()-acquired)*1000
            t.join(2)
            assert not t.is_alive()
            assert result['outcome'] == ('committed' if hold < 5 else 'sqlite_busy')
            if hold > 5:
                assert result['code'] == sqlite3.SQLITE_BUSY and result['elapsed_ms'] >= 4900
            evidence['writer_controls'].append(result)
        p.close()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence,indent=2)+'\n')
    print(json.dumps(evidence,indent=2))


if __name__ == '__main__':
    main()
