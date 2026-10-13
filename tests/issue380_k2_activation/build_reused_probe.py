"""Reuse frozen K1 build/probe shells, not its 20 tests, in task-owned output."""
import hashlib,json,pathlib,subprocess,sys,shutil,time
ROOT=pathlib.Path(__file__).resolve().parents[2];OUT=ROOT/'.tmp-tests/issue380-k2-t0'
REF='bbdd3db254582ae622236237b87adb73b097bc42'
shadow=OUT/'frozen-k1-shell';target=shadow/'tests/issue380_db_open_generation';target.mkdir(parents=True,exist_ok=True)
manifest=[]
for name in ['build_probe.py','probe.rs','policy.rs','Probe.toml','Cargo.lock']:
    path='tests/issue380_db_open_generation/'+name;data=subprocess.check_output(['git','show',f'{REF}:{path}'],cwd=ROOT)
    (target/name).write_bytes(data);manifest.append({'ref':REF,'path':path,'sha256':hashlib.sha256(data).hexdigest()})
# Override only build artifact locations; source functions, settings input and
# IO shell are identical. No K1 test/actor/run script is loaded or executed.
ns={'__name__':'frozen_k1_build','__file__':str(target/'build_probe.py')}
exec(compile((target/'build_probe.py').read_text(),ns['__file__'],'exec'),ns)
ns.update(ROOT=shadow,OUT=OUT,CRATE=OUT/'probe');ns['prepare']()
for features,label in [([], 'normal'),(['--features','native-qa'],'qa')]:
    cmd=['cargo','build','--offline','--locked','--manifest-path',str(OUT/'probe/Cargo.toml')]+features
    start=time.monotonic();r=subprocess.run(cmd,env=ns['env'](),capture_output=True,text=True,cwd=ROOT)
    (OUT/f'build-{label}.stdout.txt').write_text(r.stdout);(OUT/f'build-{label}.stderr.txt').write_text(r.stderr)
    assert r.returncode==0,r.stderr
    shutil.copy2(OUT/'probe/target/debug/k1-frozen-db-probe',OUT/f'probe-{label}')
    manifest.append({'command':cmd,'returncode':r.returncode,'binary_sha256':hashlib.sha256((OUT/f'probe-{label}').read_bytes()).hexdigest(),'elapsed_s':time.monotonic()-start,'rustc':subprocess.check_output(['rustc','--version'],env=ns['env']()).decode().strip()})
(OUT/'reused-shell-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
print('Frozen K1 production-code probes built; K1 test corpus NOT_RUN.')
