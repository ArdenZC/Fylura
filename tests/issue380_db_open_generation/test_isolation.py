"""Bounded K1 characterization: expected unsafe witnesses must actually occur."""
import ast,base64,re,contextlib,hashlib,json,os,pathlib,platform,select,shutil,sqlite3,subprocess,sys,tempfile,time,unittest
ROOT=pathlib.Path(__file__).resolve().parents[2];OUT=ROOT/'.tmp-tests/issue380-k1'
BASE='3275ac7c72df8ae4139a30f16c7f649971324e97';TRACK_B='9e144da261b97a421b6a40f61280e149887f6204'
EVENTS=[];CURRENT='';CASE=0

def event(**data):EVENTS.append({'case':CURRENT,'grade':'ISOLATED_EXECUTED_EVIDENCE',**data})
def source(path,ref=BASE):return subprocess.check_output(['git','show',f'{ref}:{path}'],cwd=ROOT).decode()
# Reuse only frozen fixture insertion/seeding, with full production-migrated DB.
seed_text=source('research/generation_publication/schema37.py',TRACK_B)
seed_ast=ast.parse(seed_text);seed_ns={}
for node in seed_ast.body:
    if isinstance(node,ast.FunctionDef) and node.name in ['insert','seed']:
        exec(ast.get_source_segment(seed_text,node),seed_ns)

def probe(root,action='init',qa=False,qa_root=None,expect=0,extra=()):
    assert pathlib.Path(root).resolve().is_relative_to(OUT.resolve()),'probe outside disposable output root'
    if qa_root is not None:assert pathlib.Path(qa_root).resolve().is_relative_to(OUT.resolve())
    cmd=[str(OUT/('probe-qa' if qa else 'probe-normal')),str(root),action,*extra]
    env=os.environ.copy();env.pop('ZC_NATIVE_QA_PROFILE_ROOT',None)
    if qa_root is not None:env['ZC_NATIVE_QA_PROFILE_ROOT']=str(qa_root)
    start=time.monotonic();r=subprocess.run(cmd,capture_output=True,text=True,env=env,timeout=20)
    event(command=cmd,qa_override=str(qa_root) if qa_root else None,stdout=r.stdout,stderr=r.stderr,returncode=r.returncode,elapsed_ms=round((time.monotonic()-start)*1000,3))
    assert r.returncode==expect,(r.returncode,r.stderr)
    return json.loads(r.stdout) if r.stdout else None

class Actor:
    def __init__(self,root,path,role):
        self.p=subprocess.Popen([sys.executable,str(ROOT/'tests/issue380_db_open_generation/worker.py'),str(root),str(path),role],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,bufsize=1)
        self.ready=self.read();assert self.ready['ready'];event(actor_ready=self.ready)
    def read(self):
        ready,_,_=select.select([self.p.stdout],[],[],15)
        assert ready,'worker response timed out'
        line=self.p.stdout.readline();assert line,'worker exited before response'
        event(raw_child_stdout=line)
        return json.loads(line)
    def send(self,op,**params):
        self.p.stdin.write(json.dumps(dict(op=op,**params))+'\n');self.p.stdin.flush()
        r=self.read();event(sync_command=dict(op=op,**params),response=r);return r
    def crash(self,op,**params):
        self.p.stdin.write(json.dumps(dict(op=op,**params))+'\n');self.p.stdin.flush()
        code=self.p.wait(timeout=15);event(sync_command=dict(op=op,**params),process_exit=code,stderr=self.p.stderr.read());return code
    def close(self):
        if self.p.poll() is None:self.send('close');self.p.wait(timeout=5)
        event(worker_exit=self.p.returncode,raw_child_stderr=self.p.stderr.read())
        for stream in [self.p.stdin,self.p.stdout,self.p.stderr]:stream.close()
    def __enter__(self):return self
    def __exit__(self,*args):self.close()

@contextlib.contextmanager
def fixture(mode='WAL'):
    with tempfile.TemporaryDirectory(prefix='zc-k1-',dir=OUT) as temp:
        root=pathlib.Path(temp);probe(root);path=root/'zen-canvas.sqlite3'
        c=sqlite3.connect(path,isolation_level=None,timeout=.1);c.execute('PRAGMA foreign_keys=ON');c.execute('PRAGMA journal_mode='+mode);c.execute('PRAGMA wal_autocheckpoint=0')
        seed_ns['seed'](c,3)
        # Maintain a keeper while workers commit WAL; no implicit close/checkpoint.
        try:yield root,path,c
        finally:c.close()

def context_at(path):
    c=sqlite3.connect(path,isolation_level=None)
    try:return c.execute("SELECT context FROM files WHERE id='stable-file-0'").fetchone()[0]
    finally:c.close()

def backup(c,target):
    b=sqlite3.connect(target,isolation_level=None);c.backup(b);b.close()

def integrity(c):
    tables=['files','global_entries','global_volumes','file_fingerprints','managed_entries','managed_scopes','ai_analysis_state','ai_jobs','ai_job_items','user_tags','file_user_tags','classification_history','classification_feedback','operation_logs','analysis_findings','analysis_finding_evidence','analysis_finding_decisions']
    data={name:c.execute(f'SELECT rowid,* FROM {name} ORDER BY rowid').fetchall() for name in tables}
    data['fk']=c.execute('PRAGMA foreign_key_check').fetchall();data['integrity']=c.execute('PRAGMA integrity_check').fetchall()
    data['global_fts']=c.execute("SELECT rowid FROM global_entries_fts WHERE global_entries_fts MATCH 'oldtoken' ORDER BY rowid").fetchall()
    # Production files FTS is files_fts; rowid and exact match truth are retained.
    data['files_fts']=c.execute("SELECT rowid FROM files_fts WHERE files_fts MATCH 'oldtoken' ORDER BY rowid").fetchall()
    return data

def fixture36(c):
    # Reconstruct the empty automation v36 tables from their frozen DDL; remove
    # only the production v37 additions in this disposable backup. Other rows,
    # IDs and associations are retained. This is fixture setup, never rollback policy.
    c.execute('PRAGMA foreign_keys=OFF')
    for table in ['automation_trigger_state','automation_runs','automation_intents']:
        assert c.execute(f'SELECT count(*) FROM {table}').fetchone()==(0,)
        c.execute(f'DROP TABLE {table}')
    c.execute('ALTER TABLE scan_roots DROP COLUMN library_change_revision')
    c.execute('ALTER TABLE files DROP COLUMN filesystem_observation_key')
    ddl=re.search(r'r#"(.*?)"#',source('src-tauri/src/db/schema_automation.rs'),re.S).group(1)
    c.executescript(ddl);c.execute('PRAGMA user_version=36');c.execute('PRAGMA foreign_keys=ON')
    assert c.execute('PRAGMA foreign_key_check').fetchall()==[]

class K1(unittest.TestCase):
    def setUp(self):
        global CURRENT,CASE
        CURRENT=self._testMethodName;CASE+=1

    def test_k1_01_frozen_open_and_migration_precede_any_post_open_gate(self):
        with tempfile.TemporaryDirectory(dir=OUT) as temp:
            root=pathlib.Path(temp);r=probe(root)
            self.assertEqual(r['user_version'],37);self.assertGreater(r['persistent_tables'],30);self.assertTrue((root/'zen-canvas.sqlite3').exists())
            self.assertEqual(r['busy_timeout_ms'],5000)
        lib=source('src-tauri/src/lib.rs');conn=source('src-tauri/src/db/connection.rs');main=source('src-tauri/src/main.rs')
        self.assertIn('Database::open(database_path(app)?)',lib)
        self.assertLess(conn.index('.build(manager)?'),conn.index('migrate(&conn)?'))
        self.assertLess(main.index('let db = open_database('),main.index('db.recover_dedupe_runs()'))
        event(grade='STATIC_SOURCE_FINDING',chain=['database_path','Database::open','configure_connection/WAL','pool build opens SQLite','migrate/DDL/user_version','main recovery/prune','later IPC/domain admission'])

    def test_k1_01_same_version_open_can_still_recreate_persistent_schema(self):
        with fixture() as (root,path,c):
            c.execute('DROP INDEX idx_global_volumes_mount_path')
            r=probe(root);self.assertEqual(r['user_version'],37)
            self.assertEqual(c.execute("SELECT count(*) FROM sqlite_master WHERE name='idx_global_volumes_mount_path'").fetchone(),(1,))
            event(witness='schema37 ensure DDL ran before post-open reject')

    def test_k1_02_retained_old_desktop_writes_before_after_open_and_reconnect(self):
        with fixture() as (root,path,c),Actor(root,path,'old-desktop') as old:
            self.assertEqual(old.send('write',value='BEFORE_OPEN')['value'],'BEFORE_OPEN')
            probe(root)
            self.assertEqual(old.send('write',value='AFTER_NEW_OPEN')['value'],'AFTER_NEW_OPEN')
            self.assertEqual(old.send('reconnect')['value'],37)
            self.assertEqual(old.send('write',value='AFTER_RECONNECT')['value'],context_at(path))
            event(strategy='S',safe=False,witness='old connection and re-open still write shared file')

    def test_k1_02_future_version_refusal_does_not_revoke_existing_connection(self):
        with fixture() as (root,path,c),Actor(root,path,'old-desktop-held') as old:
            c.execute('PRAGMA user_version=1000') # Adversarial future marker, no schema allocation.
            probe(root,expect=23)
            self.assertEqual(old.send('write',value='HELD_IGNORES_FUTURE')['value'],'HELD_IGNORES_FUTURE')
            event(strategy='S',safe=False,witness='migrator future refusal is not SQLite authorization')

    def test_k1_02_held_read_snapshot_survives_migration_then_observes_new_commits(self):
        with fixture() as (root,path,c),Actor(root,path,'old-held-reader') as old:
            self.assertEqual(old.send('begin_read')['value'],'user correction')
            probe(root)
            c.execute("UPDATE files SET context='NEW_COMMIT' WHERE id='stable-file-0'")
            self.assertEqual(old.send('read')['value'],'user correction')
            old.send('rollback');self.assertEqual(old.send('read')['value'],'NEW_COMMIT')
            event(strategy='S',safe_property='SQLite transaction snapshot is consistent',safe=False,witness='held read transaction is not generation revocation')

    def test_k1_03_worker_replay_after_mock_service_stop_still_writes(self):
        with fixture() as (root,path,c),Actor(root,path,'legacy-desktop-worker') as worker:
            pid=worker.ready['pid'];event(service_control='modeled stopped; worker kept alive',native='NOT_RUN')
            for value in ['REPLAY_BEFORE_SWITCH','REPLAY_AFTER_SWITCH']:
                probe(root);r=worker.send('write',value=value);self.assertEqual(r['pid'],pid);self.assertEqual(context_at(path),value)
            host=source('src-tauri/src/global_index/windows/service_host.rs')
            self.assertNotIn('Database::open',host)
            event(grade='STATIC_SOURCE_FINDING',service='frozen Service uses PipeSink; SQLite belongs to Desktop sink/worker, not host-owned connection')

    def test_k1_04_one_time_close_allows_real_late_child_to_open_db(self):
        for checkpoint in ['service_stopped','desktop_closed','binary_replaced','service_restarted']:
            with fixture() as (root,path,c):
                # Model control boundaries, then launch a real independent child.
                event(installer_checkpoint=checkpoint,grade='STATIC_SOURCE_FINDING',native='NOT_RUN')
                with Actor(root,path,'late-old-desktop') as late:
                    self.assertEqual(late.send('write',value=checkpoint)['value'],context_at(path))
                    event(strategy='S',safe=False,witness='late process opened fixed path after modeled close')

    def test_k1_04_new_only_advisory_lock_does_not_stop_old_sqlite_actor(self):
        import fcntl
        with fixture() as (root,path,c),Actor(root,path,'old-ignores-new-lock') as old:
            with (root/'new-version-only.lock').open('w') as lock:
                fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
                self.assertEqual(old.send('write',value='IGNORES_NEW_LOCK')['value'],'IGNORES_NEW_LOCK')
                event(strategy='S',safe=False,witness='new-only advisory lock has no legacy cooperation',windows_mutex='NOT_RUN; POSIX flock witness only')

    def test_k1_05_normalized_alias_and_symlink_are_same_database(self):
        with fixture() as (root,path,c):
            alias=root/'alias';alias.symlink_to(root,target_is_directory=True)
            normalized=root/'sub'/'..'/'zen-canvas.sqlite3';(root/'sub').mkdir()
            for p in [alias/'zen-canvas.sqlite3',normalized]:
                with Actor(root,p,'alias-old') as old:
                    old.send('write',value=str(p.name));self.assertEqual(context_at(path),'zen-canvas.sqlite3')
            self.assertEqual(path.stat().st_ino,(alias/'zen-canvas.sqlite3').stat().st_ino)
            event(strategy='S/N',safe=False,witness='different lexical names may alias the same SQLite/WAL identity',platform='POSIX symlink/inode; Windows NOT_RUN')

    def test_k1_05_generation_symlink_pointing_to_old_file_is_not_isolation(self):
        with fixture() as (root,path,c):
            newdir=root/'generation-2';newdir.mkdir();newpath=newdir/'zen-canvas.sqlite3';newpath.symlink_to(path)
            with Actor(root,newpath,'new-namespace-alias') as new:
                new.send('write',value='NAMESPACE_ALIAS');self.assertEqual(context_at(path),'NAMESPACE_ALIAS')
            event(strategy='N',safe=False,witness='path generation label without physical separation aliases old DB')

    def test_k1_05_main_file_copy_omits_committed_wal_backup_preserves_it(self):
        with fixture() as (root,path,c):
            c.execute('PRAGMA wal_checkpoint(TRUNCATE)')
            with Actor(root,path,'old-wal-writer') as old:
                old.send('write',value='COMMITTED_WAL_ONLY')
                self.assertGreater(pathlib.Path(str(path)+'-wal').stat().st_size,0)
                main=root/'main-only.sqlite3';old.send('copy_main',name=main.name)
                snap=root/'online-backup.sqlite3';old.send('backup',name=snap.name)
                self.assertNotEqual(context_at(main),'COMMITTED_WAL_ONLY');self.assertEqual(context_at(snap),'COMMITTED_WAL_ONLY')
                event(strategy='N',main_only=context_at(main),online_backup=context_at(snap),safe=False,witness='main-file copy loses committed WAL data')

    def test_k1_06_online_snapshot_loses_writes_committed_after_snapshot(self):
        with fixture() as (root,path,c),Actor(root,path,'old-desktop') as old:
            new=root/'generation-2.sqlite3';old.send('write',value='AT_SNAPSHOT');backup(c,new)
            old.send('write',value='LATE_OLD_WRITE')
            old.send('pointer',name=new.name)
            self.assertEqual(context_at(new),'AT_SNAPSHOT');self.assertEqual(context_at(path),'LATE_OLD_WRITE')
            event(strategy='N',safe=False,witness='backup is consistent snapshot, not cutover quiescence')

    def test_k1_06_post_switch_old_and_new_connections_fork_and_old_renames_file(self):
        with fixture() as (root,path,c),Actor(root,path,'old-desktop') as old:
            newpath=root/'generation-2.sqlite3';backup(c,newpath);(root/'owned-file.txt').write_text('user-fixture')
            old.send('pointer',name=newpath.name)
            with Actor(root,newpath,'new-desktop') as new:
                old.send('write',value='OLD_BRANCH');new.send('write',value='NEW_BRANCH')
                self.assertEqual(context_at(path),'OLD_BRANCH');self.assertEqual(context_at(newpath),'NEW_BRANCH')
                self.assertTrue(old.send('rename')['value']['renamed'])
                event(strategy='N',new_file_isolated=True,safe=False,witness='old/new branches diverge and old process still has native file capability',native_product_action='NOT_RUN; owned POSIX rename executed')

    def test_k1_06_pointer_crash_before_after_replace_selects_atomic_file_not_coherent_writers(self):
        for phase,expected,code in [('before_replace','zen-canvas.sqlite3',71),('after_replace','generation-2.sqlite3',72)]:
            with fixture() as (root,path,c):
                new=root/'generation-2.sqlite3';backup(c,new);(root/'active.json').write_text(path.name)
                with Actor(root,path,'pointer-writer') as worker:self.assertEqual(worker.crash('pointer',name=new.name,crash=phase),code)
                self.assertEqual((root/'active.json').read_text(),expected)
                self.assertEqual(context_at(root/expected),'user correction')
                event(strategy='N',phase=phase,pointer=expected,claim='POSIX process-exit atomicity only; not power-loss/Windows durability')

    def test_k1_06_child_crash_sqlite_rolls_back_uncommitted_and_keeps_committed(self):
        for mode in ['WAL','DELETE']:
            for op,expected,code in [('uncommitted_crash','user correction',73),('committed_crash','COMMITTED',74)]:
                with fixture(mode) as (root,path,c),Actor(root,path,'crashing-worker') as worker:
                    self.assertEqual(worker.crash(op),code);self.assertEqual(context_at(path),expected)
                    self.assertEqual(c.execute('PRAGMA integrity_check').fetchone(),('ok',))
                    event(journal_mode=mode,crash=op,recovered_context=expected)

    def test_k1_06_partial_real_migration_failure_does_not_publish_new_copy(self):
        for fail in [False,True]:
            with fixture() as (root,path,c):
                newdir=root/'generation-2';newdir.mkdir();newpath=newdir/'zen-canvas.sqlite3';backup(c,newpath)
                n=sqlite3.connect(newpath,isolation_level=None);fixture36(n)
                if fail:
                    n.execute('DROP INDEX idx_automation_intents_active')
                    n.execute('CREATE VIEW idx_automation_intents_active AS SELECT 1 AS broken')
                new_before=integrity(n);n.close()
                before=integrity(c);(root/'active.json').write_text(path.name)
                probe(newdir,expect=23 if fail else 0)
                n=sqlite3.connect(newpath)
                if fail:
                    self.assertEqual(n.execute('PRAGMA user_version').fetchone(),(36,));self.assertEqual(integrity(n),new_before)
                else:
                    self.assertEqual(n.execute('PRAGMA user_version').fetchone(),(37,));self.assertEqual(integrity(n),before)
                n.close()
                self.assertEqual(integrity(c),before);self.assertEqual((root/'active.json').read_text(),path.name)
                event(strategy='N',safe_property='real 36->37 successful control preserves rows; injected failure rolls back copied DB and no pointer promotion occurs',fault='index name occupied by view' if fail else 'none')

    def test_k1_07_real_busy_is_not_admission_or_generation_fencing(self):
        with fixture() as (root,path,c),Actor(root,path,'held-writer') as old:
            old.send('begin_write');start=time.monotonic();probe(root,expect=23);elapsed=time.monotonic()-start
            self.assertGreater(elapsed,4.5);old.send('rollback');probe(root)
            old.send('write',value='AFTER_BUSY_RELEASE');self.assertEqual(context_at(path),'AFTER_BUSY_RELEASE')
            event(strategy='S',safe=False,witness='actual production 5s timeout fails temporarily, then legacy writer proceeds',elapsed_seconds=elapsed)

    def test_k1_07_frozen_busy_unknown_notsent_routes_and_actual_direct_db_open(self):
        for version in ['old','new']:
            for fault in ['busy','sqlite_busy','unknown','not_sent']:
                with tempfile.TemporaryDirectory(dir=OUT) as temp:
                    root=pathlib.Path(temp)
                    r=probe(root,'route',extra=(version,fault))
                    expected=fault!='busy' if version=='old' else fault=='not_sent'
                    self.assertEqual(r['direct_opened_database'],expected)
                    self.assertEqual((root/'zen-canvas.sqlite3').exists(),expected)
                    event(strategy='S/N',safe=False,route=r,witness='Direct actually executes frozen Database::open/migrate, outside a hypothetical generation gate' if expected else 'request-local rejection does not certify DB generation')

    def test_k1_08_normal_qa_profile_reinstall_and_restore_choose_unfenced_paths(self):
        with fixture() as (root,path,c):
            qa=root/'qa';qa.mkdir()
            normal=probe(root,'path',qa_root=qa);self.assertEqual(pathlib.Path(normal['path']),path)
            native=probe(root,'path',qa=True,qa_root=qa);self.assertEqual(pathlib.Path(native['path']),qa/'zen-canvas.sqlite3')
            qa_alias=root/'qa-alias';qa_alias.symlink_to(root,target_is_directory=True)
            alias=probe(root,'path',qa=True,qa_root=qa_alias);self.assertEqual(pathlib.Path(alias['path']).resolve(),path.resolve())
            newpath=root/'generation-2.sqlite3';backup(c,newpath);(root/'active.json').write_text(newpath.name)
            # Old normal startup ignores this experimental pointer on reinstall/relaunch.
            self.assertEqual(pathlib.Path(probe(root,'path')['path']),path)
            # Restoring an old backup in an active namespace rolls back newer data.
            restored=root/'restored.sqlite3';backup(c,restored)
            with Actor(root,newpath,'new-active') as new:new.send('write',value='NEW_AFTER_BACKUP')
            restore=root/'restored-active.sqlite3';shutil.copyfile(restored,restore)
            self.assertNotEqual(context_at(restore),context_at(newpath))
            event(strategy='N',safe=False,witness='QA alias, old fixed-path relaunch and restore selection are not generation authority',nsis_reinstall_uninstall='NATIVE_UNVERIFIED/NOT_RUN')

    def test_k1_09_backup_preserves_ids_rowids_fk_tags_corrections_history_and_fts(self):
        with fixture() as (root,path,c):
            before=integrity(c);self.assertEqual(before['fk'],[]);self.assertEqual(before['integrity'],[('ok',)])
            new=root/'generation-2.sqlite3';backup(c,new)
            n=sqlite3.connect(new,isolation_level=None);self.assertEqual(integrity(n),before)
            n.execute("UPDATE files SET context='LATE_USER_CORRECTION' WHERE id='stable-file-0'")
            self.assertEqual(c.execute("SELECT context FROM files WHERE id='stable-file-0'").fetchone(),('user correction',))
            self.assertEqual(n.execute('PRAGMA foreign_key_check').fetchall(),[]);n.close()
            event(strategy='N',safe_property='online backup retains representative complete row/association projections',table_count=17,rows=3,projection=before,projection_sha256=hashlib.sha256(json.dumps(before,sort_keys=True).encode()).hexdigest(),witness='separate old/new metadata still forks')

if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    suite=unittest.defaultTestLoader.loadTestsFromTestCase(K1)
    with (OUT/'unittest.stdout.txt').open('w') as log:result=unittest.TextTestRunner(stream=log,verbosity=2).run(suite)
    environment={'os':platform.platform(),'machine':platform.machine(),'python':sys.version,'python_sqlite':sqlite3.sqlite_version,'base':BASE,'head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT).decode().strip(),'tree':subprocess.check_output(['git','rev-parse','HEAD^{tree}'],cwd=ROOT).decode().strip(),'command':'python3 tests/issue380_db_open_generation/test_isolation.py','test_inputs':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted((ROOT/'tests/issue380_db_open_generation').iterdir()) if p.is_file()},'source_manifest_sha256':hashlib.sha256((OUT/'source-manifest.json').read_bytes()).hexdigest(),'methods_run':result.testsRun,'success':result.wasSuccessful(),'HOSTED_WINDOWS_VERIFIED':'NOT_RUN','OWNER_NATIVE_VERIFIED':'NOT_RUN'}
    (OUT/'results.json').write_text(json.dumps({'environment':environment,'events':EVENTS},indent=2)+'\n')
    print((OUT/'unittest.stdout.txt').read_text());sys.exit(not result.wasSuccessful())
