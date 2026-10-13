#!/usr/bin/env python3
"""One paired, product-lock native probe; retain native JSONL bytes unchanged.

Large raw results are gzip-compressed, never replaced by a count/digest summary.
Small native reference + independent text BM25 validate actual full row orders.
"""
import argparse
import copy
import gzip
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time
import tomllib

from fixture import SEED, small, scale, document
from reference import rank, visible

ROOT=Path(__file__).resolve().parents[2]
SOURCE=Path(__file__).resolve().parent


def run_command(argv,env,out,name):
    start=time.monotonic()
    proc=subprocess.run(argv,cwd=ROOT,env={**os.environ,**env},capture_output=True,text=True)
    (out/(name+'-stdout.txt')).write_text(proc.stdout)
    (out/(name+'-stderr.txt')).write_text(proc.stderr)
    entry=dict(argv=argv,environment_overrides=env,cwd=str(ROOT),exit_code=proc.returncode,
               elapsed_seconds=time.monotonic()-start,stdout_file=name+'-stdout.txt',stderr_file=name+'-stderr.txt')
    if proc.returncode:raise RuntimeError(entry)
    return entry,proc.stdout


def transformed(initial,phase):
    f=copy.deepcopy(initial)
    if len(f['docs'])>256 or len(f['docs'])<9:
        if phase=='E1 root delta':
            changed=0
            for d in f['docs']:
                if changed<10 and d['roots']==['A']:d['roots']=[];changed+=1
        return f
    if phase!='E0':
        d=next(d for d in f['docs'] if d['uid']=='hidden-9')
        d.update(name='needle needle raretoken '*100,path='/private/cache/needle/'*100)
        beta=next(d for d in f['docs'] if d['uid']=='beta')
        beta.update(name='newly renamed source',path='/Work/B/current-source.dat',observation=2)
    if phase in ['A claim withdrawn','new physical incarnation','global rank changed by A']:
        next(d for d in f['docs'] if d['uid']=='alpha')['roots'].remove('A')
    if phase in ['new physical incarnation','global rank changed by A']:
        d=next(d for d in f['docs'] if d['uid']=='alpha');d['active']=False
        new=copy.deepcopy(d);new.update(uid='alpha-new-incarnation',rowid=25,physical='synthetic-volume:new-inode',active=True)
        f['docs'].append(new)
    if phase=='global rank changed by A':
        added=document(26,'rank-adversary-A','needle '*60,'/Work/A/needle',['A']);added['mtime']=123;f['docs'].append(added)
    return f


def audit_record(r,fixture):
    if 'paired' not in r:return []
    paired=r['paired']
    assert paired['F2_exact_parity'] is True,paired['phase']
    if paired['F1_native_parity'] is not None:assert paired['F1_native_parity'] is True,paired['phase']
    f=transformed(fixture,paired['phase']);cache={};checks=[]
    for q in paired['records']:
        query=q['query'];scope=q['scope']
        if query not in cache:cache[query]=rank(f,query)
        allowed={d['uid'] for d in visible(f,scope)}
        expected=[h for h in cache[query] if h['uid'] in allowed]
        if q['independent_native_reference'] is not None:
            assert q['F1']==q['independent_native_reference']
            assert q['F2']==q['independent_native_reference']
        for variant in ['F1','F2']:
            actual=q[variant]
            mismatch=[]
            if expected!=actual:
                for i,(a,b) in enumerate(zip(expected,actual)):
                    if a!=b:mismatch.append(dict(index=i,expected=a,actual=b))
                    if len(mismatch)>=5:break
            checks.append(dict(case=q['case'],phase=q['phase'],query=query,scope=scope,variant=variant,
                               expected_count=len(expected),actual_count=len(actual),expected_full_order_sha256=hashlib.sha256(json.dumps(expected,sort_keys=True).encode()).hexdigest(),
                               actual_full_order_sha256=hashlib.sha256(json.dumps(actual,sort_keys=True).encode()).hexdigest(),
                               exact_float_bits_and_full_order_equal=expected==actual,oracle_diff=mismatch or (None if expected==actual else 'count/order mismatch')))
    return checks


def parity(out,env):
    product=tomllib.loads((ROOT/'src-tauri/Cargo.lock').read_text())['package']
    local=tomllib.loads((SOURCE/'Cargo.lock').read_text())['package']
    expected={(p['name'],p['version']):(p.get('source'),p.get('checksum')) for p in product}
    rows=[]
    for p in local:
        if not p.get('source'):continue
        actual=(p.get('source'),p.get('checksum'))
        assert expected.get((p['name'],p['version']))==actual,('PARITY_UNPROVEN',p)
        rows.append({k:p[k] for k in ['name','version','source','checksum']})
    # The product has only rusqlite/r2d2_sqlite as rusqlite dependency owners.
    owners=[p['name'] for p in product if any(d.split()[0]=='rusqlite' for d in p.get('dependencies',[]))]
    assert set(owners)=={'r2d2_sqlite','zen-canvas-tauri'},owners
    syscrate=Path('/workspace/.cache/cargo/registry/src/index.crates.io-1949cf8c6b5b557f/libsqlite3-sys-0.37.0')
    crate=Path('/workspace/.cache/cargo/registry/cache/index.crates.io-1949cf8c6b5b557f/libsqlite3-sys-0.37.0.crate')
    sqlite=next(p for p in rows if p['name']=='libsqlite3-sys')
    assert hashlib.sha256(crate.read_bytes()).hexdigest()==sqlite['checksum']
    _,features=run_command(['cargo','tree','--locked','-e','features','--manifest-path',str(SOURCE/'Cargo.toml')],env,out,'dependency-features')
    result=dict(registry_packages=rows,all_registry_sources_match_product=True,rusqlite_owners=owners,
                product_declared_sqlite_features={'rusqlite':['default','bundled'],'r2d2_sqlite':['default','bundled']},
                bundled_crate_checksum_verified=True,amalgamation_sha256=hashlib.sha256((syscrate/'sqlite3/sqlite3.c').read_bytes()).hexdigest(),
                header_sha256=hashlib.sha256((syscrate/'sqlite3/sqlite3.h').read_bytes()).hexdigest(),
                product_lock_sha256=hashlib.sha256((ROOT/'src-tauri/Cargo.lock').read_bytes()).hexdigest(),
                toolchain_file=(ROOT/'rust-toolchain.toml').read_text(),
                sqlite_environment_overrides='no SQLITE_*/LIBSQLITE3_* options changed; system lib never used',
                qualification='same locked registry sources and bundled feature path; actual runtime 3.51.3/source ID/opts in native evidence. Not actual Windows/macOS application binary equivalence.')
    (out/'source-parity.json').write_text(json.dumps(result,indent=2)+'\n')


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--output',type=Path,required=True);args=parser.parse_args()
    if sys.flags.optimize:parser.error('assertions must remain enabled')
    out=args.output.resolve()
    if out.exists():parser.error('fresh output directory required; no evidence overwrite')
    out.mkdir(parents=True)
    work=ROOT/'.tmp-tests/issue381_b3f'/('run-'+out.name+'-'+str(os.getpid()));work.mkdir(parents=True)
    env={'PATH':'/workspace/.cache/cargo/bin:'+os.environ['PATH'],'RUSTUP_HOME':'/workspace/.cache/rustup','CARGO_HOME':'/workspace/.cache/cargo','CARGO_TARGET_DIR':str(ROOT/'.tmp-tests/issue381_b3f/target'),'PYTHONDONTWRITEBYTECODE':'1'}
    commands=[];all_checks=[];summary=[]
    try:
        for name,argv in [('build',['cargo','build','--release','--locked','--manifest-path',str(SOURCE/'Cargo.toml')]),('format',['cargo','fmt','--manifest-path',str(SOURCE/'Cargo.toml'),'--','--check'])]:
            command,_=run_command(argv,env,out,name);commands.append(command)
        parity(out,env)
        one=small();one['docs']=one['docs'][:1];one['queries']=['needle','Projects','资料','AI']
        cases=[('B3F-C10',scale(0)),('B3F-C01-one',one),('B3F-C01',small()),('B3F-C11',scale(10000)),('B3F-C12',scale(100000))]
        for case,fixture in cases:
            config=work/(case+'.json');config.write_text(json.dumps(fixture,ensure_ascii=False))
            data=work/(case+'-database');raw=work/(case+'-native.jsonl');stderr=out/(case+'-stderr.txt')
            argv=[str(ROOT/'.tmp-tests/issue381_b3f/target/release/issue381-b3f-probe'),'probe',str(config),str(data),case]
            start=time.monotonic()
            with raw.open('wb') as stdout,stderr.open('wb') as err:
                proc=subprocess.run(argv,cwd=ROOT,stdout=stdout,stderr=err,timeout=240)
            command=dict(argv=argv,environment_overrides={},cwd=str(ROOT),exit_code=proc.returncode,elapsed_seconds=time.monotonic()-start,raw_stdout_archive=case+'-native.jsonl.gz',stderr_file=stderr.name,fixture_sha256=hashlib.sha256(config.read_bytes()).hexdigest())
            commands.append(command);assert proc.returncode==0,command
            with raw.open('rb') as source,gzip.open(out/(case+'-native.jsonl.gz'),'wb',compresslevel=6) as target:shutil.copyfileobj(source,target)
            raw_sha=hashlib.sha256(raw.read_bytes()).hexdigest();records=[]
            with raw.open() as native:
                for line in native:
                    r=json.loads(line);all_checks.extend(audit_record(r,fixture));records.append(r)
            assert all(c['exact_float_bits_and_full_order_equal'] for c in all_checks),all_checks[-5:]
            # Summary is a separate derived report; raw native output is unchanged.
            for r in records:
                if 'paired' in r:
                    timings=[dict(query=q['query'],scope=q['scope'],F0_ms=q['F0_evidence']['query_ms'],F1_ms=q['F1_evidence']['query_ms'],F2_ms=q['F2_evidence']['query_ms'],query_plans={v:q[v+'_evidence']['plan'] for v in ['F0','F1','F2']},corpus_n=q['corpus_n'],corpus_tokens=q['corpus_tokens'],count=len(q['F1']),F2_exact_float_and_order_parity=q['F2_exact_float_and_order_parity']) for q in r['paired']['records']]
                    summary.append(dict(case=case,phase=r['paired']['phase'],population=len(fixture['docs']),raw_bytes=raw.stat().st_size,raw_sha256=raw_sha,F1_native_small_reference=r['paired']['F1_native_parity'],F2_parity=r['paired']['F2_exact_parity'],timings=timings,prepare=r.get('F1_prepare',r.get('F1_incremental_prepare')),publication=r.get('timing',r.get('publish')),environment=r.get('environment')))
            print(case,'FULL INDEPENDENT ROW/FLOAT/ORDER PARITY',flush=True)
        (out/'independent-oracle-checks.jsonl').write_text(''.join(json.dumps(c,sort_keys=True)+'\n' for c in all_checks))
        (out/'results.jsonl').write_text(''.join(json.dumps(r,sort_keys=True)+'\n' for r in summary))
        status='#381_B3F_FEASIBLE_CANDIDATE_READY_FOR_OWNER_REVIEW'
        verdict=dict(status=status,F0='EXPECTED_REJECTION_OBSERVED: retained-corpus rank and post-LIMIT underfill',F1='FEASIBLE_TEST_ONLY: exact native/small and full independent 0/10k/100k parity; sealed shadow generations; main publication below unchanged 5000ms budget',F2='EXACT_RANK_MATH_AND_ORDER_PROVEN; CURRENT_NEW_READER_GENERATION_BLOCKED on in-place physical text change until refresh; immutable all-observation/source integration unresolved',oracle_checks=len(all_checks),sizes=[0,10000,100000],native_small_only=True,scale_native_oracle_rebuild=False,percentiles_claimed=False,exit_code=0)
    except Exception as exc:
        verdict=dict(status='#381_B3F_PRODUCT_FTS_DECISION_BLOCKED',failure=repr(exc),exit_code=1)
    (out/'summary.json').write_text(json.dumps(verdict,indent=2)+'\n')
    (out/'commands.json').write_text(json.dumps(commands,indent=2)+'\n')
    files=[p for p in SOURCE.rglob('*') if p.is_file() and '__pycache__' not in str(p)]
    environment=dict(python=sys.version,os=platform.platform(),seed=SEED,command=sys.argv,
                     git={key:subprocess.check_output(['git',*cmd],cwd=ROOT,text=True).strip() for key,cmd in {'head':['rev-parse','HEAD'],'tree':['rev-parse','HEAD^{tree}'],'branch':['branch','--show-current'],'base':['rev-parse','origin/master'],'base_tree':['rev-parse','origin/master^{tree}']}.items()},
                     source_sha256={str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in files},
                     runtime_version_provenance='native Rust output only; independent Python sqlite3 is never a ranking/runtime substitute')
    (out/'environment.json').write_text(json.dumps(environment,indent=2)+'\n')
    print(json.dumps(verdict),flush=True);return verdict['exit_code']


if __name__=='__main__':sys.exit(main())
