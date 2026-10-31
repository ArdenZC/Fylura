"""Eight bounded K2-T0 cases; unsafe witnesses, not isolation qualification."""
import contextlib,fcntl,hashlib,json,os,pathlib,platform,select,shutil,sqlite3,subprocess,sys,tempfile,time,unittest,uuid
ROOT=pathlib.Path(__file__).resolve().parents[2];OUT=ROOT/'.tmp-tests/issue380-k2-t0'
BASE='3275ac7c72df8ae4139a30f16c7f649971324e97';K1='bbdd3db254582ae622236237b87adb73b097bc42'
EVENTS=[];CURRENT='';SOURCES=[]
def record(**data):EVENTS.append({'case':CURRENT,'grade':'ISOLATED_EXECUTED_EVIDENCE',**data})
def source(path,ref=BASE):
    data=subprocess.check_output(['git','show',f'{ref}:{path}'],cwd=ROOT)
    SOURCES.append({'ref':ref,'path':path,'sha256':hashlib.sha256(data).hexdigest()});return data.decode()
def identity(path):
    try:
        s=path.stat();return {'path':str(path),'device':s.st_dev,'inode':s.st_ino,'links':s.st_nlink,'size':s.st_size}
    except FileNotFoundError:return {'path':str(path),'exists':False}
def sql(path):
    assert path.resolve().is_relative_to(OUT.resolve())
    c=sqlite3.connect(path,isolation_level=None,timeout=.1);c.execute('PRAGMA foreign_keys=ON');return c

def value(path):
    c=sql(path)
    try:return c.execute("SELECT context FROM files WHERE id='stable-id'").fetchone()[0]
    finally:c.close()
def commit_count(path):
    c=sql(path)
    try:return c.execute('SELECT count(*) FROM t0_commits').fetchone()[0]
    finally:c.close()
def projection(c):
    return {t:c.execute(f'SELECT rowid,* FROM {t} ORDER BY rowid').fetchall() for t in ['files','tags','file_tags','history']}|{'fk':c.execute('PRAGMA foreign_key_check').fetchall(),'integrity':c.execute('PRAGMA integrity_check').fetchall()}

@contextlib.contextmanager
def fixture(mode='WAL'):
    OUT.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='owned-t0-',dir=OUT) as temp:
        root=pathlib.Path(temp);nonce=uuid.uuid4().hex;(root/'fixture-owner.json').write_text(json.dumps({'nonce':nonce}))
        path=root/'zen-canvas.sqlite3';c=sql(path);c.execute('PRAGMA journal_mode='+mode);c.execute('PRAGMA wal_autocheckpoint=0')
        c.executescript((ROOT/'tests/issue380_k2_activation/schema.sql').read_text())
        try:yield root,path,c,nonce
        finally:c.close()

class Actor:
    def __init__(self,root,nonce,role,name='zen-canvas.sqlite3',program=None):
        assert root.resolve().is_relative_to(OUT.resolve());self.role=role
        command=[sys.executable,str(program or ROOT/'tests/issue380_k2_activation/actor.py'),str(OUT),str(root),nonce,role,name]
        self.p=subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,bufsize=1)
        self.ready=self.read();assert self.ready['ready'];record(command=command,actor=self.ready)
    def read(self):
        available,_,_=select.select([self.p.stdout],[],[],15);assert available,'actor timeout'
        line=self.p.stdout.readline();assert line,'actor exited without response';record(raw_stdout=line)
        return json.loads(line)
    def send(self,op,**params):
        self.p.stdin.write(json.dumps({'op':op,**params})+'\n');self.p.stdin.flush();r=self.read();record(input={'op':op,**params},output=r);return r
    def crash(self,op,**params):
        self.p.stdin.write(json.dumps({'op':op,**params})+'\n');self.p.stdin.flush();code=self.p.wait(timeout=15);record(input={'op':op,**params},exit=code);return code
    def close(self):
        if self.p.poll() is None:self.send('close');self.p.wait(timeout=5)
        record(role=self.role,process_exit=self.p.returncode,raw_stderr=self.p.stderr.read())
        for s in [self.p.stdin,self.p.stdout,self.p.stderr]:s.close()
    def __enter__(self):return self
    def __exit__(self,*args):self.close()

def probe(root,action='init',qa=False,profile=None,extra=(),expected=0):
    assert root.resolve().is_relative_to(OUT.resolve())
    e=os.environ.copy();e.pop('ZC_NATIVE_QA_PROFILE_ROOT',None)
    if profile is not None:
        assert profile.resolve().is_relative_to(OUT.resolve());e['ZC_NATIVE_QA_PROFILE_ROOT']=str(profile)
    command=[str(OUT/('probe-qa' if qa else 'probe-normal')),str(root),action,*extra]
    r=subprocess.run(command,env=e,capture_output=True,text=True,timeout=20);record(command=command,profile=str(profile) if profile else None,stdout=r.stdout,stderr=r.stderr,exit=r.returncode)
    assert r.returncode==expected,(r.returncode,r.stderr)
    return json.loads(r.stdout) if r.stdout else None

class T0(unittest.TestCase):
    def setUp(self):
        global CURRENT
        CURRENT=self._testMethodName

    def test_t0_01_noncooperative_commits_across_entire_activation_window(self):
        with fixture() as (root,path,c,nonce),Actor(root,nonce,'legacy-writer') as old:
            (root/'new-only-mutex').write_text('upgrade authority candidate')
            with (root/'new-only-mutex').open('r+') as guard:
                fcntl.flock(guard,fcntl.LOCK_EX|fcntl.LOCK_NB)
                phases=['before_admission','isolation_started','snapshot_completed','new_activation','reconnect_after_activation']
                new=root/'generation-new.sqlite3'
                for i,phase in enumerate(phases):
                    if phase=='isolation_started':(root/'activation.json').write_text(json.dumps({'stage':'UPGRADING','generation':'new'}))
                    if phase=='snapshot_completed':
                        with Actor(root,nonce,'snapshot-worker') as snap:self.assertTrue(snap.send('snapshot',target=new.name)['success'])
                    if phase=='new_activation':
                        with Actor(root,nonce,'switch-worker') as switch:self.assertTrue(switch.send('switch',target=new.name)['success'])
                    if phase=='reconnect_after_activation':self.assertTrue(old.send('reconnect')['success'])
                    r=old.send('commit',phase=phase,value='USER_'+phase);self.assertTrue(r['success']);self.assertEqual(commit_count(path),i+1)
                    self.assertEqual(value(path),'USER_'+phase)
                self.assertNotEqual(value(new),value(path))
                record(isolation=False,risk='five real commits across marker/lock/snapshot/activation; actor never reads new protocol',old_commits=commit_count(path),new_commits=commit_count(new),old_pid=old.ready['pid'],old_identity=identity(path),new_identity=identity(new))

    def test_t0_02_service_desktop_connection_worker_conditions_are_distinct(self):
        nsis=source('src-tauri/windows/installer-lifecycle-final.nsh');main=source('src-tauri/src/main.rs');host=source('src-tauri/src/global_index/windows/service_host.rs')
        prep=nsis.split('Function ZCPrepareInstallLifecycleFinal\n')[1].split('FunctionEnd')[0]
        self.assertLess(prep.index('ZCStopCapturedServiceForLifecycle'),prep.index('ZCResolveMainAppGateFinal'))
        self.assertIn('FindProcess "${MAINBINARYNAME}.exe"',nsis);self.assertIn('Sleep 500',nsis)
        self.assertIn('tauri_plugin_single_instance::init',main);self.assertNotIn('Database::open',host)
        with fixture() as (root,path,c,nonce),Actor(root,nonce,'background-worker') as worker:
            desktop=Actor(root,nonce,'desktop');desktop.close()
            states={'Service_STOPPED':'MODELED','Desktop_exited':True,'worker_SQLite_closed':False,'worker_stopped':False,'late_start_excluded':False,'old_copy_excluded':False}
            r=worker.send('commit',phase='after_desktop_exit_and_mock_service_stop',value='WORKER_STILL_COMMITS');self.assertTrue(r['success'])
            self.assertEqual(value(path),'WORKER_STILL_COMMITS');record(conditions=states,worker_pid=worker.ready['pid'],native_windows='NOT_RUN',isolation=False)

    def test_t0_03_main_wal_shm_alias_hardlink_and_replaced_handle_identity(self):
        with fixture() as (root,path,c,nonce),Actor(root,nonce,'wal-owner') as old:
            c.execute('PRAGMA wal_checkpoint(TRUNCATE)');old.send('commit',phase='wal_only',value='WAL_COMMIT')
            alias=root/'alias';alias.symlink_to(root,target_is_directory=True);(root/'sub').mkdir()
            for name in ['alias/zen-canvas.sqlite3','sub/../zen-canvas.sqlite3']:
                with Actor(root,nonce,'lexical-alias',name) as actor:
                    self.assertEqual(actor.send('read')['value'],'WAL_COMMIT')
                    self.assertEqual(actor.ready['identity']['main_path']['inode'],path.stat().st_ino)
            hard=root/'hardlink.sqlite3';os.link(path,hard)
            with Actor(root,nonce,'hardlink-reader',hard.name) as actor:
                r=actor.send('read');self.assertTrue(r['success']);self.assertEqual(hard.stat().st_ino,path.stat().st_ino)
                record(hardlink_read=r['value'],canonical_read=value(path),hardlink_identity=actor.ready['identity'],canonical_identity=old.send('read')['identity'],risk='same main inode does not prove shared WAL/SHM namespace; no writes through hardlink alias')
            record(junction_reparse='NOT_RUN',windows_handle_identity='NOT_RUN')
        # DELETE-journal replacement avoids mixing live canonical WAL sidecars.
        with fixture('DELETE') as (root,path,c,nonce),Actor(root,nonce,'retained-inode') as old:
            before=old.ready['identity']['opened_main_handle'];replacement=root/'replacement.sqlite3'
            b=sql(replacement);b.executescript((ROOT/'tests/issue380_k2_activation/schema.sql').read_text());b.execute("UPDATE files SET context='NEW_INODE' WHERE id='stable-id'");b.close()
            retired=root/'retired.sqlite3';path.rename(retired);os.replace(replacement,path)
            self.assertEqual(old.send('read')['value'],'USER_SEED');self.assertEqual(value(path),'NEW_INODE')
            result=old.send('commit',phase='after_path_replace',value='OLD_HANDLE_TRY')
            self.assertNotEqual(path.stat().st_ino,before['inode']);self.assertEqual(retired.stat().st_ino,before['inode'])
            record(replacement_commit=result,retained_read=value(retired),new_path_read=value(path),risk='replacement is not evidence that old connection/handle exited',platform='POSIX DELETE only; Windows/WAL replacement NOT_RUN')
            self.assertTrue(old.send('reconnect')['success'])
            self.assertTrue(old.send('commit',phase='reopen_replaced_fixed_path',value='REOPENED_WRITES_NEW_IDENTITY')['success'])
            self.assertEqual(value(path),'REOPENED_WRITES_NEW_IDENTITY');self.assertEqual(value(retired),'USER_SEED')
            record(reopened_old_actor_changes_replacement=True,continuing_identity_authority=False)

    def test_t0_04_late_login_second_instance_old_copy_and_same_owner_mode(self):
        with fixture() as (root,path,c,nonce),Actor(root,nonce,'retained-old') as held:
            checked=Actor(root,nonce,'checked-old-desktop');checked.close()
            copy=root/'old-copy.py';shutil.copyfile(ROOT/'tests/issue380_k2_activation/actor.py',copy)
            self.assertEqual(hashlib.sha256(copy.read_bytes()).hexdigest(),hashlib.sha256((ROOT/'tests/issue380_k2_activation/actor.py').read_bytes()).hexdigest())
            for role,program in [('late-relaunch',None),('login-item-model',None),('second-instance-model',None),('different-old-copy',copy)]:
                with Actor(root,nonce,role,program=program) as late:
                    self.assertTrue(late.send('commit',phase=role,value=role)['success']);self.assertEqual(value(path),role)
            held.send('chmod')
            r=held.send('commit',phase='main_mode_readonly_with_live_wal',value='HELD_AFTER_CHMOD')
            # Record the real VFS result; always test same owner can restore mode.
            self.assertTrue(held.send('restore_mode')['success']);self.assertTrue(held.send('reconnect')['success'])
            self.assertTrue(held.send('commit',phase='same_owner_restores_mode',value='OWNER_REOPENS')['success'])
            record(readonly_mode_attempt=r,isolation=False,risk='one-time close, new-only lock and owner-controlled file mode are not continuous exclusion',windows_ACL_UAC_sessions='NOT_RUN',macos_LaunchAgent_bundle='NOT_RUN')

    def test_t0_05_last_commit_snapshot_migration_switch_crash_and_restore(self):
        for fault in ['none','migration_exit','migration_fail','before_switch_exit','after_switch_exit']:
            with fixture() as (root,path,c,nonce),Actor(root,nonce,'old-correction-writer') as old:
                old.send('commit',phase='last_observed_before_snapshot',value='USER_AT_SNAPSHOT')
                before=projection(c);new=root/'new.sqlite3';(root/'activation.json').write_text(json.dumps({'db':path.name,'stage':'OLD_ACTIVE'}))
                with Actor(root,nonce,'snapshot-process') as snap:self.assertTrue(snap.send('snapshot',target=new.name)['success'])
                n=sql(new);self.assertEqual(projection(n),before);n.close()
                self.assertTrue(old.send('commit',phase='late_after_snapshot',value='LATE_USER_CORRECTION')['success'])
                with Actor(root,nonce,'offline-transform',new.name) as transform:
                    if fault=='migration_exit':self.assertEqual(transform.crash('migrate_model',crash=True),73)
                    else:
                        r=transform.send('migrate_model',fail=fault=='migration_fail');self.assertEqual(r['success'],fault!='migration_fail')
                if fault not in ['migration_exit','migration_fail']:
                    with Actor(root,nonce,'activation-switch',new.name) as switch:
                        if fault in ['before_switch_exit','after_switch_exit']:self.assertEqual(switch.crash('switch',target=new.name,crash='before' if fault=='before_switch_exit' else 'after'),71 if fault=='before_switch_exit' else 72)
                        else:self.assertTrue(switch.send('switch',target=new.name)['success'])
                selected=json.loads((root/'activation.json').read_text())['db']
                if fault in ['none','after_switch_exit']:
                    with Actor(root,nonce,'new-first-writer',new.name) as active:self.assertTrue(active.send('commit',phase='new_epoch_first_write',value='NEW_AFTER_ACTIVATION')['success'])
                    self.assertTrue(old.send('commit',phase='old_writes_after_new_activation',value='OLD_AFTER_ACTIVATION')['success'])
                    self.assertNotEqual(value(new),value(path))
                    restored=root/'old-snapshot-restore.sqlite3'
                    with Actor(root,nonce,'restore-copy',new.name) as active:self.assertTrue(active.send('snapshot',target=restored.name)['success'])
                    # Choosing old generation loses the acknowledged new first write.
                    self.assertNotEqual(value(path),value(restored))
                elif fault in ['migration_exit','migration_fail']:
                    n=sql(new);self.assertEqual(projection(n),before);self.assertEqual(n.execute("SELECT count(*) FROM sqlite_master WHERE name='t0_offline_migration'").fetchone(),(0,));n.close()
                self.assertEqual(selected,new.name if fault in ['none','after_switch_exit'] else path.name)
                self.assertEqual(c.execute('PRAGMA foreign_key_check').fetchall(),[])
                record(fault=fault,snapshot_context='USER_AT_SNAPSHOT',old_context=value(path),new_context=value(new),selected_db=selected,old_commits=commit_count(path),new_commits=commit_count(new),two_active_writers=fault in ['none','after_switch_exit'],no_silent_fork=fault not in ['none','after_switch_exit'],snapshot_missing_late_write=True,protocol_decision='BLOCK activation without final-writer/lifetime proof',power_loss='NOT_RUN',offline_migration='fixture-only model; no production SQL migration rerun')

    def test_t0_06_database_write_block_does_not_revoke_file_operations_or_fd(self):
        with fixture() as (root,path,c,nonce),Actor(root,nonce,'same-user-file-actor') as old:
            ordinary=root/'ordinary.txt';ordinary.write_text('ORIGINAL');(root/'moved').mkdir()
            opened=old.send('open_file',name=ordinary.name);self.assertTrue(opened['success'])
            c.execute('BEGIN IMMEDIATE')
            try:
                busy=old.send('commit',phase='db_write_excluded',value='BLOCKED_DB_WRITE');self.assertFalse(busy['success']);self.assertEqual(busy['sqlite_code'],sqlite3.SQLITE_BUSY)
                self.assertTrue(old.send('rename',source='ordinary.txt',target='renamed.txt')['success'])
                self.assertTrue(old.send('move',source='renamed.txt',target='moved/moved.txt')['success'])
                self.assertTrue(old.send('delete',name='moved/moved.txt')['success'])
                held=old.send('write_open_handle',value='AFTER_UNLINK');self.assertTrue(held['success']);self.assertEqual(held['value']['content'],'AFTER_UNLINK');self.assertEqual(held['value']['links'],0)
                record(db_write_blocked=True,file_authority_isolated=False,opened_file=opened['value'],unlinked_file=held['value'],operations=['rename','move','delete','write retained fd'],native_product_pipeline='NOT_RUN',Windows_delete_share_handles='NOT_RUN')
            finally:c.execute('ROLLBACK')

    def test_t0_07_preopen_records_direct_qa_recovery_and_busy_boundaries(self):
        for state in ['missing','corrupt','generation_mismatch']:
            with tempfile.TemporaryDirectory(prefix='owned-t0-probe-',dir=OUT) as temp:
                root=pathlib.Path(temp)
                if state!='missing':(root/'activation.json').write_text('broken json' if state=='corrupt' else json.dumps({'db':'other-generation.sqlite3','generation':'incompatible'}))
                result=probe(root);self.assertEqual(result['user_version'],37);self.assertTrue((root/'zen-canvas.sqlite3').exists())
                record(activation_record=state,legacy_bypasses_target_gate=True,first_block_required='before manager/pool opens SQLite; no gate implemented')
        for version,fault in [('old','unknown'),('new','not_sent')]:
            with tempfile.TemporaryDirectory(prefix='owned-t0-probe-',dir=OUT) as temp:
                root=pathlib.Path(temp);(root/'activation.json').write_text('broken json')
                r=probe(root,'route',extra=(version,fault));self.assertTrue(r['direct_opened_database']);record(activation_bypass=r)
        with tempfile.TemporaryDirectory(prefix='owned-t0-probe-',dir=OUT) as temp:
            root=pathlib.Path(temp);profile=root/'legacy-profile';profile.mkdir()
            r=probe(root,'path',qa=True,profile=profile);self.assertEqual(pathlib.Path(r['path']),profile/'zen-canvas.sqlite3')
            (root/'activation.json').write_text(json.dumps({'db':'new.sqlite3'}));self.assertEqual(pathlib.Path(probe(root,'path')['path']),root/'zen-canvas.sqlite3')
            record(reinstall_restore_uninstall='STATIC/MODELED; native NOT_RUN',path_selection='frozen normal resolver ignores test activation record; QA picks legacy profile')
        lib=source('src-tauri/src/lib.rs');connection=source('src-tauri/src/db/connection.rs');schema=source('src-tauri/src/db/schema.rs');recovery=source('src-tauri/src/global_index/windows/recovery_qa.rs')
        self.assertIn('Database::open(database_path(app)?)',lib);self.assertLess(connection.index('.build(manager)?'),connection.index('migrate(&conn)?'));self.assertIn('root.join("zen-canvas.sqlite3")',recovery)
        prior=json.loads(source('tests/issue380_db_open_generation/evidence/results.json',K1));self.assertTrue(prior['environment']['success']);self.assertEqual(prior['environment']['methods_run'],20)
        record(grade='STATIC_SOURCE_FINDING',prior_K1_busy_proof='accepted frozen 5s production timeout; NOT_RERUN',K2_busy_proof='T0-06 real SQLITE_BUSY(5) at 100ms actor timeout',new_unknown_direct='forbidden by frozen new fallback / accepted K1; no wire requalification',production_migrate_source='unchanged frozen full Schema37 via reused probe')

    def test_t0_08_bounded_dependency_evidence_not_new_inventory(self):
        refs=[('a48f8849b1ba1baadcf84e217d505e2298cf35a1','docs/project/research/issue-381-d4-s8r/README.md'),('9e144da261b97a421b6a40f61280e149887f6204','research/generation_publication/schema37.py'),('9e66433fd833e2f52a428976303af10d8a252de9','docs/research/ISSUE-380-MIXED-VERSION-TEST-ONLY-RESULT.md')]
        for ref,path in refs:self.assertTrue(source(path,ref))
        deps=[{'domain':d,'future_migration_contract':need,'frozen_evidence':evidence,'qualified':False} for d,need,evidence in [
            ('installation activation','OS lifetime exclusion + authenticated activation/forward recovery','K1/K2 negative witnesses'),
            ('DB generation','last old commit, full WAL snapshot, distinct physical DB/WAL/SHM, first new writer','K1 representative backup and K2 race traces'),
            ('C2 Global Index','complete generation seal, active pointer CAS, all reader/writer participation','Track B isolated prototype; CTAS not relational proof'),
            ('#381 Root/Source Manifest','same-read-transaction compatible physical/source/path epochs + visibility','frozen S8R inventory, not new audit'),
            ('tags/corrections/Consent','independent user metadata authority, preserve latest correction, consent predicates','K1 17-table three-row projection; K2 late correction witness'),
            ('IDs/rowid/FK/history','stable identity and history anchors through migration/add/delete/path reuse','K1 limited equality; K2 representative preservation'),
            ('Global FTS/BM25/Cursor','full visible generation corpus and generation-bound cursor','Track B limited FTS; product rank/pagination NOT_PROVEN'),
            ('GC/Recovery','leases, crash reclamation, budgets, one active generation, no silent rollback/fork','Track B isolated lease/backup controls; K2 process-exit model')]]
        record(grade='STATIC_SOURCE_FINDING',dependencies=deps,three_authorities=['installation_activation','DB_write_generation','domain_publication'],D4_S9='NOT_STARTED',inventory_rescan='NOT_RUN')
        (OUT/'dependencies.json').write_text(json.dumps(deps,indent=2)+'\n')

if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    with (OUT/'unittest.stdout.txt').open('w') as log:r=unittest.TextTestRunner(stream=log,verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(T0))
    environment={'os':platform.platform(),'python':sys.version,'sqlite':sqlite3.sqlite_version,'uid':os.getuid(),'base':BASE,'head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT).decode().strip(),'tree':subprocess.check_output(['git','rev-parse','HEAD^{tree}'],cwd=ROOT).decode().strip(),'command':'python3 tests/issue380_k2_activation/test_feasibility.py','methods_run':r.testsRun,'success':r.wasSuccessful(),'test_inputs':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted((ROOT/'tests/issue380_k2_activation').iterdir()) if p.is_file()},'HOSTED_WINDOWS_VERIFIED':'NOT_RUN','OWNER_NATIVE_VERIFIED':'NOT_RUN','macos_native':'NOT_RUN','K1_20_case_corpus':'NOT_RERUN'}
    (OUT/'results.json').write_text(json.dumps({'environment':environment,'sources':SOURCES,'events':EVENTS},indent=2)+'\n')
    print((OUT/'unittest.stdout.txt').read_text());sys.exit(not r.wasSuccessful())
