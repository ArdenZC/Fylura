"""Compile frozen Database::open/migrate and path functions in a minimal IO shell."""
import hashlib,json,os,pathlib,shutil,subprocess,sys,time
ROOT=pathlib.Path(__file__).resolve().parents[2]
BASE='3275ac7c72df8ae4139a30f16c7f649971324e97'
OUT=ROOT/'.tmp-tests/issue380-k1'
CRATE=OUT/'probe'
MANIFEST=[]
def read(path,ref=BASE):
    raw=subprocess.check_output(['git','show',f'{ref}:{path}'],cwd=ROOT)
    MANIFEST.append({'ref':ref,'path':path,'sha256':hashlib.sha256(raw).hexdigest()})
    return raw.decode()
def fn(text,marker):
    start=text.index(marker);pos=text.index('{',start)+1;depth=1
    while depth:
        depth+=(text[pos]=='{')-(text[pos]=='}');pos+=1
    block=text[start:pos]
    MANIFEST.append({'symbol':marker,'slice_sha256':hashlib.sha256(block.encode()).hexdigest(),'first_line':text[:start].count('\n')+1})
    return block

def prepare():
    (CRATE/'src/db').mkdir(parents=True,exist_ok=True)
    for name in ['schema','schema_automation','schema_automation_triggers']:
        (CRATE/f'src/db/{name}.rs').write_text(read(f'src-tauri/src/db/{name}.rs'))
    text=read('src-tauri/src/db/connection.rs')
    fn(text,'pub fn open(') # Record exact open slice separately from module assembly.
    prefix=text[:text.index('    pub(crate) fn set_managed_ai_waker')]
    # No production function body is changed. Only unrelated connection methods
    # are excluded; Database fields and real pool/open/configure code remain.
    connection=prefix+'\n'+fn(text,'pub fn path(')+'\n'+fn(text,'pub(crate) fn conn(')+'\n}\n'+fn(text,'fn configure_connection(')
    (CRATE/'src/db/connection.rs').write_text(connection)
    types=read('src-tauri/src/db/types.rs')
    queries=read('src-tauri/src/db/queries/mod.rs')
    db='use thiserror::Error; use std::time::{SystemTime,UNIX_EPOCH};\n'+types[types.index('#[derive(Debug, Error)]'):types.index('#[derive(Debug, Clone)]')]
    db+='\n'+fn(queries,'pub(crate) fn trim_trailing_path_separators(')+'\n'+fn(queries,'pub(crate) fn normalize_path_text(')+'\n'+fn(queries,'pub(crate) fn current_unix_seconds(')
    db+='\nstruct AutomationWake; mod schema; mod schema_automation; mod schema_automation_triggers; mod connection; pub use connection::Database;\n'
    (CRATE/'src/db/mod.rs').write_text(db)
    policy=(ROOT/'tests/issue380_db_open_generation/policy.rs').read_text()
    old=read('src-tauri/src/global_index/windows/mod.rs')
    new=read('src-tauri/src/global_index/windows/mod.rs','d260131f9d6ec354f2361e6c7460c79e62e38391')
    policy+='\nimpl Provider {\n'+fn(old,'fn run_with_fallback(')+'\n'+fn(new,'fn run_with_fallback_using<F>')+'\n}\n'
    (CRATE/'src/policy.rs').write_text(policy)
    assert 'pub const APP_SETTINGS_KEY: &str = "app_settings_v1";' in read('src-tauri/src/settings.rs')
    lib=read('src-tauri/src/lib.rs')
    main=(ROOT/'tests/issue380_db_open_generation/probe.rs').read_text()
    main+='\n'+fn(lib,'pub fn database_path<')+'\n'+fn(lib,'pub fn open_database<')
    (CRATE/'src/main.rs').write_text(main)
    (CRATE/'Cargo.toml').write_text((ROOT/'tests/issue380_db_open_generation/Probe.toml').read_text())
    lock=ROOT/'tests/issue380_db_open_generation/Cargo.lock'
    if lock.exists():shutil.copyfile(lock,CRATE/'Cargo.lock')
    # Targeted reference inspection, not an inventory rescan or rerun.
    for ref,path in [('9e66433fd833e2f52a428976303af10d8a252de9','tests/issue380_mixed_version/run.py'),('9e144da261b97a421b6a40f61280e149887f6204','research/generation_publication/schema37.py'),('9e144da261b97a421b6a40f61280e149887f6204','research/generation_publication/verify_recovery.py'),('a48f8849b1ba1baadcf84e217d505e2298cf35a1','docs/project/research/issue-381-d4-s8r/README.md')]:read(path,ref)
    for path in ['src-tauri/src/main.rs','src-tauri/src/global_index/windows/service_host.rs','src-tauri/src/global_index/windows/recovery_qa.rs','src-tauri/windows/installer-lifecycle-final.nsh','src-tauri/windows/installer-lifecycle-synchronous.nsh','src-tauri/src/global_index/coordinator.rs','src-tauri/src/global_index/windows/mod.rs']:read(path)
    for path in ['src-tauri/src/global_index/windows/mod.rs','src-tauri/src/global_index/windows/service.rs']:
        read(path,'d260131f9d6ec354f2361e6c7460c79e62e38391')
    (OUT/'source-manifest.json').write_text(json.dumps(MANIFEST,indent=2)+'\n')

def env():
    e=os.environ.copy();e.pop('ZC_NATIVE_QA_PROFILE_ROOT',None)
    if not shutil.which('cargo'):
        e.setdefault('CARGO_HOME','/workspace/.cache/cargo');e.setdefault('RUSTUP_HOME','/workspace/.cache/rustup')
        e['PATH']='/workspace/.cache/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin:'+e['PATH']
    return e
if __name__=='__main__':
    prepare()
    if '--prepare' in sys.argv:sys.exit(0)
    for features,label in [([], 'normal'),(['--features','native-qa'],'qa')]:
        cmd=['cargo','build','--offline','--locked','--manifest-path',str(CRATE/'Cargo.toml')]+features
        start=time.monotonic()
        result=subprocess.run(cmd,cwd=ROOT,env=env(),capture_output=True,text=True)
        (OUT/f'build-{label}.stdout.txt').write_text(result.stdout)
        (OUT/f'build-{label}.stderr.txt').write_text(result.stderr)
        if result.returncode:print(result.stderr);sys.exit(result.returncode)
        shutil.copy2(CRATE/'target/debug/k1-frozen-db-probe',OUT/f'probe-{label}')
        record={'command':cmd,'rustc':subprocess.check_output(['rustc','--version'],env=env()).decode().strip(),'returncode':result.returncode,'elapsed_seconds':time.monotonic()-start,'binary_sha256':hashlib.sha256((OUT/f'probe-{label}').read_bytes()).hexdigest(),'native_windows':False}
        (OUT/f'build-{label}.json').write_text(json.dumps(record,indent=2)+'\n')
    print('Built normal and native-qa path probes from frozen Database::open and full migrate source.')
