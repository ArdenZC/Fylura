#!/usr/bin/env python3
"""Execute frozen SQL with real SQLite; enumerate source-bound installer schedules."""
import contextlib, hashlib, itertools, json, pathlib, re, sqlite3, subprocess, tempfile, time, unittest
ROOT=pathlib.Path(__file__).resolve().parents[2]
OUT=ROOT/'.tmp-tests/issue380-mixed-version'
OLD='3275ac7c72df8ae4139a30f16c7f649971324e97'
NEW='d260131f9d6ec354f2361e6c7460c79e62e38391'
RESULTS=[]

def read(ref,path):
    return subprocess.check_output(['git','show',f'{ref}:{path}'],cwd=ROOT).decode()

def function(text,name):
    start=text.index('pub fn '+name+'(')
    end=text.index('\n    pub fn ',start+1)
    return text[start:end]

def stale_sql(ref):
    text=function(read(ref,'src-tauri/src/global_index/repository.rs'),'mark_global_entries_stale_for_volume')
    sql=re.findall(r'"(UPDATE [^"\n]+)"',text)
    assert len(sql)==6, 'frozen stale SQL shape changed'
    assert 'transaction(' not in text and 'BEGIN' not in text
    return sql

@contextlib.contextmanager
def fixture(mode='WAL'):
    with tempfile.TemporaryDirectory(prefix='zc380-') as d:
        p=pathlib.Path(d)/'fixture.sqlite'
        c=sqlite3.connect(p,isolation_level=None,timeout=.05)
        c.execute('PRAGMA journal_mode='+mode)
        c.executescript((OUT/'harness/src/schema.sql').read_text())
        c.executescript((ROOT/'tests/issue380_mixed_version/fixture.sql').read_text())
        try: yield c,p
        finally: c.close()

def project(c):
    names=['global_volumes','global_entries','managed_scopes','managed_entries','ai_analysis_state','ai_jobs','ai_job_items']
    p={n:c.execute(f'SELECT * FROM {n} ORDER BY 1').fetchall() for n in names}
    p['fts_raw']=c.execute("SELECT rowid FROM global_entries_fts WHERE global_entries_fts MATCH 'needle'").fetchall()
    p['fts_active']=c.execute("SELECT e.id FROM global_entries_fts f JOIN global_entries e ON e.rowid=f.rowid WHERE global_entries_fts MATCH 'needle' AND e.is_stale=0").fetchall()
    return p

class SQLiteEvidence(unittest.TestCase):
    def test_all_four_pairs_have_pre_admission_rebuild_damage(self):
        for desktop,service in itertools.product([OLD,NEW],repeat=2):
            with fixture() as (c,p):
                before=project(c)
                # Rebuild call order is separately executed from Rust production Coordinator.
                # Here the exact six repository SQL statements run in SQLite autocommit.
                for sql in stale_sql(desktop):c.execute(sql,('v',20) if '?2' in sql else ('v',))
                after=project(c)
                self.assertNotEqual(before,after)
                self.assertEqual(after['fts_raw'],[(1,)])
                self.assertEqual(after['fts_active'],[])
                self.assertEqual(c.execute('SELECT journal_cursor FROM global_volumes').fetchone(),('old',))
                self.assertEqual(c.execute('SELECT content_summary,user_corrected FROM ai_analysis_state').fetchone(),('retained summary',1))
                RESULTS.append(dict(case='pre-admission',desktop=desktop,service=service,stale=1,managed_enabled=0,ai='stale',count=0,checkpoint='old',active_search_hits=0))

    def test_each_sql_failure_retains_prior_autocommit_damage(self):
        tables=['global_entries','managed_entries','ai_jobs','ai_job_items','ai_analysis_state','global_volumes']
        for ref,mode,fail in itertools.product([OLD,NEW],['WAL','DELETE'],range(6)):
            with fixture(mode) as (c,p):
                before=project(c)
                condition=" WHEN NEW.updated_at=20" if fail==5 else ""
                c.execute(f"CREATE TRIGGER failpoint BEFORE UPDATE ON {tables[fail]}{condition} BEGIN SELECT RAISE(ABORT,'fixture failpoint'); END")
                completed=0
                with self.assertRaises(sqlite3.IntegrityError):
                    for sql in stale_sql(ref):
                        c.execute(sql,('v',20) if '?2' in sql else ('v',));completed+=1
                self.assertEqual(completed,fail)
                expected=(fail>0)
                self.assertEqual(project(c)!=before,expected)
                c.close()
                # Reopen validates committed partial writes survive connection loss.
                reopened=sqlite3.connect(p,isolation_level=None)
                self.assertEqual(project(reopened)!=before,expected)
                reopened.close()
                # context finalizer tolerates double close.
                RESULTS.append(dict(case='partial-sql',ref=ref,mode=mode,fail_statement=fail+1,committed_prefix=completed,damage=expected))

    def test_real_writer_lock_is_sqlite_busy_not_service_busy(self):
        for mode in ['WAL','DELETE']:
            with fixture(mode) as (c,p):
                before=project(c)
                locker=sqlite3.connect(p,isolation_level=None)
                locker.execute('BEGIN IMMEDIATE')
                start=time.monotonic()
                try:
                    with self.assertRaises(sqlite3.OperationalError) as e:
                        c.execute(stale_sql(OLD)[0],('v',))
                    self.assertEqual(e.exception.sqlite_errorcode,sqlite3.SQLITE_BUSY)
                    elapsed=time.monotonic()-start
                    self.assertEqual(project(c),before)
                finally: locker.execute('ROLLBACK');locker.close()
                RESULTS.append(dict(case='sqlite-writer-lock',mode=mode,error_code=e.exception.sqlite_errorcode,wait_ms=round(elapsed*1000,3),test_busy_timeout_ms=50))

    def test_child_exit_after_pre_admission_sql_preserves_damage(self):
        for ref,mode in itertools.product([OLD,NEW],['WAL','DELETE']):
            with fixture(mode) as (c,p):
                before=project(c)
                code="import os,sqlite3,sys,json; c=sqlite3.connect(sys.argv[1],isolation_level=None); sql=json.loads(sys.argv[2]); c.execute(sql[0],('v',)); c.execute(sql[1],('v',20)); os._exit(73)"
                r=subprocess.run(['python3','-c',code,str(p),json.dumps(stale_sql(ref))])
                self.assertEqual(r.returncode,73)
                after=project(c)
                self.assertNotEqual(before,after)
                self.assertEqual(c.execute('SELECT is_stale FROM global_entries').fetchone(),(1,))
                self.assertEqual(c.execute('SELECT enabled FROM managed_entries').fetchone(),(0,))
                RESULTS.append(dict(case='child-exit',ref=ref,mode=mode,exit=73,damage_survives=True))

class InstallerSchedules(unittest.TestCase):
    def test_source_stop_close_replace_restart_order_is_not_a_launch_barrier(self):
        # This is a bounded interleaving model, not NSIS/SCM execution.
        path='src-tauri/windows/installer-lifecycle-final.nsh'
        text=read(OLD,path)
        self.assertEqual(text,read(NEW,path))
        prepare=text.split('Function ZCPrepareInstallLifecycleFinal\n',1)[1].split('FunctionEnd',1)[0]
        calls=re.findall(r'^  Call (\w+)',prepare,re.M)
        self.assertEqual(calls,['ZCInitializeInstallLifecycle','ZCStopCapturedServiceForLifecycle','ZCResolveMainAppGateFinal','ZCQuiescePreviewForLifecycle'])
        gate=text.split('Function ZCResolveMainAppGateFinal\n',1)[1].split('FunctionEnd',1)[0]
        self.assertIn('FindProcess',gate);self.assertIn('KillProcess',gate)
        self.assertIn('StrCpy $ZC_LIFECYCLE_GATE_OK 1',gate)
        # Inject launch at all checkpoints. No process mutex/startup barrier is
        # executed by this model; its absence from the observed gate is a source finding.
        # One canonical launch inserted at each checkpoint, then the installer
        # continues. close_desktops is a one-time observation/action; later launches
        # survive it. Simulated connection failure exercises the Rust NotSent path.
        steps=['stop_service','close_desktops','replace_binary','start_service']
        checkpoints=['before_stop','service_stopped','desktop_closed','binary_replaced','service_restarted']
        hazards=[]
        for slot,checkpoint in enumerate(checkpoints):
            binary='old'; service='old'; processes=[]; index_attempt=None
            for position in range(5):
                if position==slot:
                    processes.append(binary)
                    index_attempt=(binary,service)
                if position==4:break
                step=steps[position]
                if step=='stop_service':service=None
                elif step=='close_desktops':processes=[]
                elif step=='replace_binary':binary='new'
                else:service=binary
            mixed=any(client!=service for client in processes)
            direct=index_attempt[1] is None
            if mixed or direct:hazards.append(checkpoint)
            RESULTS.append(dict(case='installer-interleaving-model',checkpoint=checkpoint,index_attempt=index_attempt,post_restart_desktops=processes,post_restart_service=service,NotSent_direct_window=direct,mixed_pair_after_restart=mixed,native_verified=False))
        self.assertEqual(hazards,['service_stopped','desktop_closed','binary_replaced'])
        self.assertTrue(next(r for r in RESULTS if r.get('checkpoint')=='desktop_closed')['mixed_pair_after_restart'])
        RESULTS.append(dict(case='installer-source',path=path,sha256=hashlib.sha256(text.encode()).hexdigest(),calls=calls,native_verified=False))

    def test_cancel_and_partial_failure_recovery_source_and_model(self):
        sync=read(OLD,'src-tauri/windows/installer-lifecycle-synchronous.nsh')
        final=read(OLD,'src-tauri/windows/installer-lifecycle-final.nsh')
        recover=sync.split('Function ZCRecoverInstallReversible\n',1)[1].split('FunctionEnd',1)[0]
        self.assertIn('RestoreZenCanvasPreexistingService',recover)
        failure=final.split('Function ZCHandlePostInstallFailureFinal\n',1)[1].split('FunctionEnd',1)[0]
        self.assertIn('ZCStopCapturedServiceForLifecycle',failure)
        self.assertIn('ZC_LIFECYCLE_STAGE_FILE_MUTATION',failure)
        for state in ['cancel_before_mutation','failure_after_mutation','successful_switch','launch_after_close_check']:
            RESULTS.append(dict(case='installer-recovery-model',state=state,native_verified=False,required_owner_gate='coherent_pair_and_launch_exclusion'))

if __name__=='__main__':
    import sys
    suite=unittest.defaultTestLoader.loadTestsFromModule(sys.modules[__name__])
    with (OUT/'sqlite-installer-results.txt').open('w') as log:
        result=unittest.TextTestRunner(stream=log,verbosity=2).run(suite)
    (OUT/'observations.json').write_text(json.dumps(dict(sqlite_version=sqlite3.sqlite_version,observations=RESULTS),indent=2)+'\n')
    print((OUT/'sqlite-installer-results.txt').read_text())
    sys.exit(not result.wasSuccessful())
