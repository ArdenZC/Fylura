"""Explicit bounded runner. Not connected to CI or any product database."""
import argparse
import hashlib
import json
import os
import platform
import resource
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

from prototype import Prototype, connect
from schema37 import ROOT, SOURCE_SHA256

MAX_DISK = 1024 * 1024 * 1024


def sizes(path):
    values = {suffix or 'db': Path(str(path)+suffix).stat().st_size
              if Path(str(path)+suffix).exists() else 0 for suffix in ('', '-wal', '-shm')}
    values['total'] = sum(values.values())
    return values


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def experiment(strategy, n, folder):
    path = folder / f'{strategy}-{n}.db'
    started = time.perf_counter()
    p = Prototype(path, strategy, n)
    fixture_ms = (time.perf_counter()-started)*1000
    baseline = sizes(path)
    stop = threading.Event()
    peaks = baseline.copy()
    sample_count = 0
    errors = []
    writer_results = []
    def sample():
        nonlocal sample_count
        while not stop.is_set():
            current = sizes(path)
            sample_count += 1
            for key, value in current.items():
                peaks[key] = max(peaks[key], value)
            if current['total'] > MAX_DISK:
                errors.append('1GiB per-fixture disk cap exceeded')
                stop.set()
            stop.wait(.01)
    sampler = threading.Thread(target=sample)
    sampler.start()
    writer_ready = threading.Event()
    def metadata_writer():
        c = connect(path)
        writer_ready.set()
        i = 0
        while not stop.is_set():
            attempt = time.perf_counter()
            try:
                c.execute('BEGIN IMMEDIATE')
                c.execute("INSERT INTO app_settings VALUES('concurrent-writer',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", (str(i),))
                c.execute('COMMIT')
                writer_results.append(dict(outcome='committed', elapsed_ms=(time.perf_counter()-attempt)*1000))
            except sqlite3.OperationalError as e:
                if c.in_transaction:
                    c.execute('ROLLBACK')
                writer_results.append(dict(outcome='sqlite_busy' if e.sqlite_errorcode == sqlite3.SQLITE_BUSY else 'error',
                                           code=e.sqlite_errorcode, elapsed_ms=(time.perf_counter()-attempt)*1000))
            i += 1
            stop.wait(.01)
        c.close()
    writer = threading.Thread(target=metadata_writer)
    writer.start()
    if not writer_ready.wait(5):
        raise RuntimeError('writer failed to initialize')
    deadline = time.perf_counter()+120
    p.c.set_progress_handler(lambda: int(bool(errors) or time.perf_counter()>deadline), 10000)
    try:
        stage_start = time.perf_counter()
        p.stage()
        stage_ms = (time.perf_counter()-stage_start)*1000
        assert p.read() == p.old
        after_stage = sizes(path)
        failure_start = time.perf_counter()
        try:
            p.publish(fail_after='ai_job_items' if strategy == 'C1' else 'pointer')
            raise AssertionError('failpoint did not fail')
        except sqlite3.OperationalError as e:
            assert 'rp_missing_table' in str(e)
        failure_ms = (time.perf_counter()-failure_start)*1000
        assert p.read() == p.old
        after_failure = sizes(path)
        # Dedicated already-connected writer attempts while publish owns the lock.
        go, attempting = threading.Event(), threading.Event()
        dedicated_ready = threading.Event()
        dedicated = {}
        def publish_contender():
            c = connect(path)
            dedicated_ready.set()
            if not go.wait(10):
                c.close()
                return
            attempt = time.perf_counter()
            attempting.set()
            try:
                c.execute('BEGIN IMMEDIATE')
                c.execute("INSERT INTO app_settings VALUES('publish-contender','ok')")
                c.execute('COMMIT')
                dedicated['outcome'] = 'committed'
            except sqlite3.OperationalError as e:
                dedicated.update(outcome='sqlite_busy', code=e.sqlite_errorcode)
            finally:
                dedicated['elapsed_ms'] = (time.perf_counter()-attempt)*1000
                c.close()
        contender = threading.Thread(target=publish_contender)
        contender.start()
        assert dedicated_ready.wait(5)
        def probe_hook(c):
            go.set()
            assert attempting.wait(1)
        publish_start = time.perf_counter()
        p.publish(hook=probe_hook)
        publish_ms = (time.perf_counter()-publish_start)*1000
        contender.join(6)
        assert not contender.is_alive()
        assert p.read() == p.expected
        assert p.history_rows() == p.history
        search_start = time.perf_counter()
        hits = p.search('newtoken')
        search_ms = (time.perf_counter()-search_start)*1000
        assert len(hits) == max(0,n-1)
        after_publish = sizes(path)
        p.c.execute("INSERT INTO rp_leases VALUES('pinned',1)")
        assert not p.gc()
        p.c.execute("DELETE FROM rp_leases WHERE token='pinned'")
        gc_start = time.perf_counter()
        assert p.gc()
        gc_ms = (time.perf_counter()-gc_start)*1000
        assert p.read() == p.expected
        after_gc = sizes(path)
    finally:
        stop.set()
        writer.join(6)
        sampler.join(1)
        p.c.set_progress_handler(None, 0)
    checkpoint_start = time.perf_counter()
    checkpoint = tuple(p.c.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone())
    checkpoint_ms = (time.perf_counter()-checkpoint_start)*1000
    assert checkpoint[0] == 0
    after_checkpoint = sizes(path)
    labels = ('stage', 'publish', 'gc')
    lock_summary = {}
    for label in labels:
        records = [r for r in p.locks if r['label'].split(':')[0] == label]
        lock_summary[label] = dict(transactions=len(records), max_held_ms=max(r['held_ms'] for r in records),
                                   total_held_ms=sum(r['held_ms'] for r in records),
                                   total_wait_ms=sum(r['wait_ms'] for r in records))
    result = dict(strategy=strategy, rows=n, fixture_ms=fixture_ms, stage_ms=stage_ms,
                  failure_ms=failure_ms, publish_ms=publish_ms, search_ms=search_ms, gc_ms=gc_ms,
                  checkpoint_ms=checkpoint_ms, checkpoint=checkpoint, sizes=dict(baseline=baseline,
                  staged=after_stage, failed=after_failure, published=after_publish, collected=after_gc,
                  checkpointed=after_checkpoint, sampled_peak_lower_bound=peaks), samples=sample_count,
                  locks=lock_summary, transactions=p.locks, concurrent_writer=writer_results,
                  publish_contender=dedicated, resource_errors=errors)
    p.close()
    for suffix in ('', '-wal', '-shm'):
        Path(str(path)+suffix).unlink(missing_ok=True)
    if errors:
        raise RuntimeError(errors)
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--bounded-100k', action='store_true',
                        help='one explicit 100k experiment per strategy AFTER small correctness gate')
    args = parser.parse_args()
    if args.output.exists():
        parser.error('output exists; preserve evidence by choosing a new filename')
    # Hard resource guard for this Linux research host. No automatic large sizes.
    resource.setrlimit(resource.RLIMIT_AS, (1536*1024**2,1536*1024**2))
    if shutil.disk_usage(ROOT).free < 3*1024**3:
        raise RuntimeError('need 3GiB free disk')
    gate = subprocess.run([sys.executable, '-m', 'unittest', 'discover', '-s',
                           str(Path(__file__).parent), '-p', 'test_prototype.py', '-v'],
                          capture_output=True, text=True, timeout=180)
    print(gate.stderr, flush=True)
    if gate.returncode:
        raise RuntimeError('small correctness gate failed; no measurements authorized')
    suite_files = sorted(Path(__file__).parent.glob('*.py'))
    output = dict(base_sha=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
                  base_tree=subprocess.check_output(['git','rev-parse','HEAD^{tree}'],cwd=ROOT,text=True).strip(),
                  schema_sha256=SOURCE_SHA256, source_sha256={p.name:digest(p) for p in suite_files},
                  environment=dict(python=sys.version,sqlite=sqlite3.sqlite_version,platform=platform.platform(),
                                   cpu_count=os.cpu_count(), journal_mode='WAL',synchronous='NORMAL',
                                   busy_timeout_ms=5000, wal_autocheckpoint=0, batch=250),
                  correctness=dict(exit_code=gate.returncode,stdout=gate.stdout,stderr=gate.stderr),
                  bounded_100k=args.bounded_100k, measurements=[])
    with tempfile.TemporaryDirectory(prefix='issue-380-measure-') as folder:
        for n in (0,1,1000,10000) + ((100000,) if args.bounded_100k else ()):
            for strategy in ('C1','C2','C3'):
                result = experiment(strategy,n,Path(folder))
                output['measurements'].append(result)
                print(json.dumps({k:result[k] for k in ('strategy','rows','stage_ms','publish_ms','failure_ms','locks','publish_contender')}), flush=True)
                args.output.parent.mkdir(parents=True,exist_ok=True)
                args.output.write_text(json.dumps(output,ensure_ascii=False,indent=2)+'\n')
    output['peak_rss_kib'] = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    args.output.write_text(json.dumps(output,ensure_ascii=False,indent=2)+'\n')


if __name__ == '__main__':
    main()
