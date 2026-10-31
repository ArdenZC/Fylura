"""Bounded genuine-platform experiments. Passing execution never means production GO."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import queue
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import traceback

HERE = Path(__file__).resolve().parent
BASE = '3275ac7c72df8ae4139a30f16c7f649971324e97'
EVENTS = []
ACTORS = []
ROOT = None


def event(kind, **fields):
    row = dict(sequence=len(EVENTS), monotonic_ns=time.monotonic_ns(), kind=kind, **fields)
    EVENTS.append(row)
    return row


class Actor:
    def __init__(self, name, copied=False):
        path = HERE / 'actor.py'
        if copied:
            target = ROOT / 'copied-old-model'
            target.mkdir(exist_ok=True)
            for file in ('actor.py', 'native.py'):
                shutil.copyfile(HERE / file, target / file)
            path = target / 'actor.py'
        self.name = name
        self.stderr_path = ROOT / (name + '.stderr')
        self.stderr = self.stderr_path.open('w', encoding='utf-8')
        self.p = subprocess.Popen([sys.executable, '-u', str(path), str(ROOT)], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=self.stderr, text=True, encoding='utf-8')
        self.q = queue.Queue()
        def drain():
            for line in self.p.stdout:
                self.q.put(line)
            self.q.put(None)
        threading.Thread(target=drain, daemon=True).start()
        ACTORS.append(self)
        event('launch', actor=name, pid=self.p.pid, command=[sys.executable, '-u', str(path), str(ROOT)],
              actor_sha256=hashlib.sha256(path.read_bytes()).hexdigest(), evidence_level='TEST_MODEL')
        self.ask('hello')

    def ask(self, op, **fields):
        req = dict(op=op, **fields)
        if self.p.poll() is not None:
            answer = dict(ok=False, error='PROCESS_ALREADY_EXITED', returncode=self.p.returncode)
            event('exchange', actor=self.name, pid=self.p.pid, request=req, response=answer)
            return answer
        self.p.stdin.write(json.dumps(req) + '\n')
        self.p.stdin.flush()
        try:
            line = self.q.get(timeout=8)
        except queue.Empty:
            self.stop(abnormal=True)
            answer = dict(ok=False, error='BOUNDED_ACTOR_TIMEOUT', timeout_seconds=8,
                          sqlite_errorcode=None, note='No SQLite result returned; not a SQLITE_BUSY claim')
            event('exchange', actor=self.name, pid=self.p.pid, request=req, response=answer)
            return answer
        if line is None:
            raise RuntimeError('actor exited unexpectedly: ' + self.name)
        answer = json.loads(line)
        event('exchange', actor=self.name, pid=self.p.pid, request=req, response=answer)
        return answer

    def stop(self, abnormal=False):
        if self.p.poll() is None:
            if abnormal:
                self.p.kill()
            else:
                self.ask('close')
                self.ask('release')
                self.ask('exit')
            try:
                code = self.p.wait(timeout=4)
            except subprocess.TimeoutExpired:
                self.p.kill()
                code = self.p.wait(timeout=4)
            event('process_exit', actor=self.name, pid=self.p.pid, abnormal=abnormal, returncode=code)
        self.stderr.close()


def require(result):
    if not result['ok']:
        raise RuntimeError('setup/control failed: ' + json.dumps(result))
    return result['result']


def fixture(name):
    folder = ROOT / name
    folder.mkdir()
    p = folder / 'old.sqlite3'
    old = Actor(name + '-old')
    require(old.ask('open', path=str(p)))
    require(old.ask('init'))
    require(old.ask('write', id='old-final', value='confirmed correction before barrier'))
    require(old.ask('identity', path=str(p)))
    return folder, p, old


def gate(actor, path, spec):
    if platform.system() == 'Windows':
        return actor.ask('win_gate', path=str(path), suffixes=['', '-wal', '-shm'], **spec)
    return actor.ask('posix_gate', path=str(path), suffixes=['', '-wal', '-shm'], **spec)


def alias_paths(folder, path):
    from native import identities
    result = {}
    for kind in ('hardlink', 'symlink'):
        p = folder / (kind + '.sqlite3')
        try:
            for suffix in ('', '-wal', '-shm'):
                if kind == 'hardlink':
                    os.link(str(path) + suffix, str(p) + suffix)
                else:
                    os.symlink(str(path) + suffix, str(p) + suffix)
            event('alias', alias_type=kind, path=str(p), identities=identities(p), canonical=identities(path))
            result[kind] = p
        except OSError as e:
            event('alias_not_run', alias_type=kind, errno=e.errno, winerror=getattr(e, 'winerror', None), message=str(e))
    aliasdir = folder / 'directory-alias'
    try:
        if platform.system() == 'Windows':
            cmd = ['cmd.exe', '/c', 'mklink', '/J', str(aliasdir), str(folder)]
            done = subprocess.run(cmd, capture_output=True, text=True, timeout=5)
            event('junction', command=cmd, returncode=done.returncode, stdout=done.stdout, stderr=done.stderr)
            if done.returncode:
                return result
            kind = 'junction_reparse'
        else:
            os.symlink(folder, aliasdir, target_is_directory=True)
            kind = 'directory_symlink'
        p = aliasdir / path.name
        result[kind] = p
        event('alias', alias_type=kind, path=str(p), identities=identities(p), canonical=identities(path))
    except OSError as e:
        event('alias_not_run', alias_type='directory', errno=e.errno, message=str(e))
    return result


def open_write(actor, path, correction):
    opened = actor.ask('open', path=str(path))
    written = actor.ask('write', id=correction, value='confirmed old-model correction') if opened['ok'] else None
    actor.ask('close')
    return dict(open=opened, write=written, acknowledged=bool(written and written['ok']))


def experiment(name, spec):
    folder, path, old = fixture(name)
    guard = Actor(name + '-guard')
    # Existing connection: actual acquire failures/retained commits, not process enumeration.
    existing = gate(guard, path, spec)
    retained = old.ask('write', id='retained-before-stop', value='confirmed retained-handle correction')
    guard.ask('release')
    # Abrupt exit preserves WAL/SHM and tests recovery, without clean checkpoint.
    old.stop(abnormal=True)
    aliases = alias_paths(folder, path)
    acquired = gate(guard, path, spec)
    complete_gate = acquired['ok'] and acquired['result']['all_acquired']
    late = Actor(name + '-late')
    late_result = open_write(late, path, 'late-held')
    late.stop()
    alias_results = {}
    for alias, alternate in aliases.items():
        actor = Actor(name + '-' + alias)
        alias_results[alias] = open_write(actor, alternate, 'alias-' + alias)
        actor.stop()
    probe = Actor(name + '-raw-fd-probe')
    raw_fd = probe.ask('raw_same_bytes', path=str(path))
    probe.stop()
    reader = Actor(name + '-backup')
    opened = reader.ask('open', path=str(path), readonly=True)
    target = folder / 'new.sqlite3'
    backup = reader.ask('backup', target=str(target)) if opened['ok'] else None
    reader.stop()
    new_result = None
    if backup and backup['ok']:
        newer = Actor(name + '-new')
        require(newer.ask('open', path=str(target)))
        prepared = newer.ask('prepare')
        first = newer.ask('write', id='new-first', value='confirmed new-generation user correction')
        rows = newer.ask('read')
        new_result = dict(prepare=prepared, first=first, read=rows)
        newer.stop(abnormal=True)
        recovery = Actor(name + '-new-recovery')
        require(recovery.ask('open', path=str(target)))
        new_result['recovery'] = recovery.ask('read')
        recovery.stop()
    # Barrier holder failure is a separate lifecycle boundary, not a held-barrier result.
    guard.stop(abnormal=True)
    copied = Actor(name + '-copied-relaunch', copied=True)
    after_exit = open_write(copied, path, 'old-after-holder-exit')
    final_old = copied.ask('open', path=str(path))
    old_rows = copied.ask('read') if final_old['ok'] else None
    copied.stop()
    last_known = {'old-final'}
    if retained['ok']:
        last_known.add('retained-before-stop')
    if late_result['acknowledged']:
        last_known.add('late-held')
    for alias, result in alias_results.items():
        if result['acknowledged']:
            last_known.add('alias-' + alias)
    snapshot_ids = {r[0] for r in backup['result']['rows']} if backup and backup['ok'] else set()
    summary = dict(candidate=name, spec=spec, existing_acquisition=existing,
                   existing_retained_write=retained, acquired_after_exit=acquired, complete_gate=complete_gate,
                   late=late_result, aliases=alias_results, raw_fd_probe=raw_fd, snapshot_open=opened, snapshot=backup,
                   snapshot_includes_all_known_confirmed=bool(backup and backup['ok'] and last_known <= snapshot_ids),
                   confirmed_before_snapshot=sorted(last_known), snapshot_ids=sorted(snapshot_ids),
                   separate_generation=new_result, holder_abnormal_exit_old_relaunch=after_exit,
                   final_old=old_rows, real_product='NOT_RUN', production='NO_GO')
    summary['bounded_held_window_candidate'] = bool(complete_gate and not late_result['acknowledged']
          and all(not r['acknowledged'] for r in alias_results.values())
          and summary['snapshot_includes_all_known_confirmed'] and new_result
          and new_result['first']['ok'])
    summary['two_generations_acknowledged_after_holder_exit'] = bool(new_result and new_result['first']['ok'] and after_exit['acknowledged'])
    event('candidate_summary', **summary)
    return summary


def lifecycle_control():
    folder, path, old = fixture('lifecycle')
    old.ask('close')
    old.stop()
    restart = Actor('lifecycle-restart')
    result = open_write(restart, path, 'restart-after-clean-stop')
    restart.stop()
    # Independent SQLite BEGIN IMMEDIATE gate: compatible WAL snapshot, temporary writer admission only.
    writer = sqlite3.connect(path, timeout=.15)
    writer.execute('BEGIN IMMEDIATE')
    blocked = Actor('lifecycle-sqlite-busy')
    response = open_write(blocked, path, 'during-sqlite-transaction')
    blocked.stop()
    reader = Actor('lifecycle-snapshot')
    require(reader.ask('open', path=str(path), readonly=True))
    backup = reader.ask('backup', target=str(folder / 'busy-snapshot.sqlite3'))
    reader.stop()
    newer = Actor('lifecycle-new-generation')
    require(newer.ask('open', path=str(folder / 'busy-snapshot.sqlite3')))
    prepared = require(newer.ask('prepare'))
    new_first = require(newer.ask('write', id='new-first', value='confirmed independent new-generation correction'))
    newer.stop(abnormal=True)
    recover = Actor('lifecycle-new-recovery')
    require(recover.ask('open', path=str(folder / 'busy-snapshot.sqlite3')))
    recovered = require(recover.ask('read'))
    recover.stop()
    writer.rollback()
    writer.close()
    late = Actor('lifecycle-after-sqlite-release')
    unblocked = open_write(late, path, 'after-sqlite-release')
    late.stop()
    event('lifecycle_control', restart=result, sqlite_held=response, snapshot=backup,
          prepare=prepared, new_first=new_first, new_recovery=recovered, sqlite_released=unblocked,
          two_generations_acknowledged=bool(new_first['acknowledged'] and unblocked['acknowledged']),
          conclusion='SQLite transaction lock is not a durable generation fence')


def main():
    global ROOT
    args = argparse.ArgumentParser()
    args.add_argument('--output', required=True)
    args.add_argument('--allow-linux-smoke', action='store_true')
    opt = args.parse_args()
    system = platform.system()
    if system not in ('Windows', 'Darwin') and not opt.allow_linux_smoke:
        raise SystemExit('Native Windows/macOS required; Linux cannot qualify G1')
    output = Path(opt.output).resolve()
    # Only user-provided evidence output plus unique owned fixture directory are written.
    output.mkdir(parents=True, exist_ok=True)
    ROOT = Path(tempfile.mkdtemp(prefix='issue380-k2-g1-', dir=output))
    source_hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(HERE.glob('*.py'))}
    git = lambda expr: subprocess.check_output(['git', 'rev-parse', expr], text=True).strip()
    report = dict(base=BASE, head=git('HEAD'), tree=git('HEAD^{tree}'), source_hashes=source_hashes,
                  platform=platform.platform(), system=system, machine=platform.machine(),
                  python=sys.version, sqlite=sqlite3.sqlite_version, pid=os.getpid(),
                  uid=getattr(os, 'getuid', lambda: None)(), fixture_root=str(ROOT),
                  runner_os=os.environ.get('RUNNER_OS'), image_os=os.environ.get('ImageOS'),
                  image_version=os.environ.get('ImageVersion'), github_run_id=os.environ.get('GITHUB_RUN_ID'),
                  github_run_attempt=os.environ.get('GITHUB_RUN_ATTEMPT'),
                  evidence_level='HOSTED_NATIVE_API' if system in ('Windows', 'Darwin') and os.environ.get('GITHUB_ACTIONS') else 'LOCAL_SMOKE_ONLY',
                  shipped_old_product='NOT_RUN', reconstructed_product='NOT_RUN', controlled_actors='TEST_MODEL',
                  owner_native='NOT_RUN', production_migration='NOT_AUTHORIZED', production='NO_GO')
    info = sqlite3.connect(':memory:')
    report['sqlite_compile_options'] = [r[0] for r in info.execute('PRAGMA compile_options')]
    info.close()
    # Resolve the real Python SQLite default VFS when its linked symbol is exposed.
    # Never infer the product's bundled rusqlite VFS from this interpreter.
    try:
        import ctypes
        import _sqlite3
        class VFS(ctypes.Structure):
            pass
        VFS._fields_ = [('version', ctypes.c_int), ('szOsFile', ctypes.c_int),
                       ('mxPathname', ctypes.c_int), ('next', ctypes.c_void_p), ('name', ctypes.c_char_p)]
        library = ctypes.CDLL(_sqlite3.__file__)
        find = library.sqlite3_vfs_find
        find.argtypes = [ctypes.c_char_p]
        find.restype = ctypes.POINTER(VFS)
        report['sqlite_default_vfs'] = find(None).contents.name.decode()
    except Exception as error:
        report['sqlite_default_vfs'] = dict(unavailable=str(error))
    try:
        if system == 'Windows':
            specs = [('win-exclusive', dict(access=0x80000000, share=0)),
                     ('win-read-share', dict(access=0x80000000, share=1)),
                     ('win-readwrite-share-negative', dict(access=0x80000000, share=3))]
        else:
            specs = [('posix-flock', dict(kind='flock')), ('posix-fcntl-whole-file', dict(kind='fcntl'))]
        report['candidates'] = [experiment(name, spec) for name, spec in specs]
        lifecycle_control()
        report['execution'] = 'COMPLETE'
    except Exception:
        report['execution'] = 'ERROR'
        report['traceback'] = traceback.format_exc()
    finally:
        for actor in ACTORS:
            if actor.p.poll() is None:
                actor.stop(abnormal=True)
        report['events'] = EVENTS
        report['actor_stderr'] = {a.name: a.stderr_path.read_text(encoding='utf-8') for a in ACTORS}
        (output / 'results.json').write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
        print(json.dumps({k: v for k, v in report.items() if k not in ('events', 'actor_stderr', 'candidates')}, indent=2))
        print('Raw results: ' + str(output / 'results.json'))
        # Remove only this run's owned fixture. Keep evidence JSON, not databases or child binaries.
        # Junction/symlink aliases must be unlinked before recursive cleanup.
        for folder in ROOT.iterdir():
            if folder.is_dir() and not folder.is_symlink():
                alias = folder / 'directory-alias'
                if os.path.lexists(alias):
                    if system == 'Windows':
                        os.rmdir(alias)
                    else:
                        alias.unlink()
        shutil.rmtree(ROOT)
    return 0 if report['execution'] == 'COMPLETE' else 1


if __name__ == '__main__':
    sys.exit(main())
