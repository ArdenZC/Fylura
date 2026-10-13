#!/usr/bin/env python3
"""Frozen-source, offline harness. Never rewrites production functions."""
import hashlib, json, os, pathlib, shutil, subprocess, sys
ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = ROOT / '.tmp-tests/issue380-mixed-version'
OLD = '3275ac7c72df8ae4139a30f16c7f649971324e97'
NEW = 'd260131f9d6ec354f2361e6c7460c79e62e38391'
D0 = '8ab50cb627712abef8debfa786d9fb2354ef9581'
manifest = []
def source(ref, name):
    path = 'src-tauri/src/global_index/' + name + '.rs'
    data = subprocess.check_output(['git','show',f'{ref}:{path}'], cwd=ROOT).decode()
    manifest.append(dict(ref=ref,path=path,sha256=hashlib.sha256(data.encode()).hexdigest()))
    return data

def function(s, marker):
    start = s.index(marker)
    # Function opening brace follows the signature; skip braces in generic bounds (none here).
    opening = s.index('{', start)
    depth = 1
    pos = opening + 1
    # braces in strings in these selected functions are balanced; fail closed via compile.
    while depth:
        depth += (s[pos] == '{') - (s[pos] == '}')
        pos += 1
    return s[start:pos]

def between(s, start, end):
    return s[s.index(start):s.index(end, s.index(start))]

OUT.mkdir(parents=True, exist_ok=True)
crate = OUT / 'harness'
(crate/'src').mkdir(parents=True,exist_ok=True)
schema_path='src-tauri/src/db/schema.rs'
schema=subprocess.check_output(['git','show',f'{OLD}:{schema_path}'],cwd=ROOT).decode()
start=schema.index('CREATE TABLE IF NOT EXISTS global_volumes')
sql=schema[start:schema.index('"#',start)]
hard_start=schema.index('DROP TRIGGER IF EXISTS global_entries_au;')
sql+=schema[hard_start:schema.index('"#',hard_start)]
(crate/'src/schema.sql').write_text(sql)
(crate/'src/fixture.sql').write_text((ROOT/'tests/issue380_mixed_version/fixture.sql').read_text())
manifest.append(dict(ref=OLD,path=schema_path,sha256=hashlib.sha256(schema.encode()).hexdigest()))
models = source(OLD,'models')
# Shared models are byte-identical in both peers.
assert models == source(NEW,'models')
(crate/'src/models.rs').write_text(models)
for label, ref in [('old',OLD),('new',NEW)]:
    svc = source(ref,'windows/service')
    host = source(ref,'windows/service_host')
    coord = source(ref,'coordinator')
    mod = source(ref,'windows/mod')
    errors = source(ref,'windows/service_errors')
    err = between(coord,'#[derive(Debug, Error)]','pub trait GlobalIndexSink')
    traits = between(coord,'pub trait GlobalIndexSink','    /// Cheap topology-only discovery') + '}\n'
    protocol = between(svc,'pub const IPC_PROTOCOL_VERSION:', 'impl IndexServiceRequest')
    # The old transport is Windows-gated. Execute its verbatim call_stream with a fake connect/IO boundary.
    if label == 'old':
        transport = function(svc,'pub fn call_stream<F>')
        io = (ROOT/'tests/issue380_mixed_version/old_io.rs').read_text()
    else:
        transport = ''
        io = (ROOT/'tests/issue380_mixed_version/new_io.rs').read_text()
    protocol += '\n' + function(svc,'pub fn validate_request')
    protocol += '\n' + function(svc,'fn validate_source_snapshot')
    runtime = between(host,'struct ServiceRuntime','fn service_handler(')
    bridge = (ROOT/'tests/issue380_mixed_version/host_bridge.rs').read_text()
    if label=='old':
        # Constructor selection belongs to test glue, not the extracted runtime.
        bridge = bridge.replace('ServiceRuntime::with_provider(Arc::new(DirectWindowsGlobalIndexProvider))','ServiceRuntime::new()')
    mapping = function(errors,'pub(super) fn classify') + '\n' + function(errors,'pub(super) fn decode')
    head = 'use crate::models::*; use std::sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}}; use thiserror::Error; use std::collections::{HashMap,HashSet}; use crate::DbError;\n'
    service = 'use crate::models::*; use serde::{Serialize,Deserialize}; use super::GlobalIndexError; use thiserror::Error;\n' + protocol + io + transport
    text = head + err + traits + '\npub mod service {\n'+service+'\n}\n'
    text += (ROOT/'tests/issue380_mixed_version/sink.rs').read_text()
    text += 'use service::*;\nmod service_errors { use super::*;\n'+mapping+'\n}\n'
    text += 'struct DirectWindowsGlobalIndexProvider; impl DirectWindowsGlobalIndexProvider {fn new()->Self{Self}}\n'
    text += (ROOT/'tests/issue380_mixed_version/provider.rs').read_text()
    text += '\nmod host { use super::*;\n' + runtime
    text += bridge
    if label=='new':
        text += 'impl IndexServiceServerIo for Wire { fn send_event(&mut self,e:IndexServiceEvent)->Result<(),String>{Wire::send_event(self,e)} fn send_response(&mut self,r:IndexServiceResponse)->Result<(),String>{Wire::send_response(self,r)} fn read_lookup_response(&mut self,id:&str)->Result<IndexServiceLookupResponse,String>{Wire::read_lookup_response(self,id)} }'
    text += '\n} pub use host::*;\n'
    text += '\nstruct WindowsGlobalIndexProvider { direct: DirectWindowsGlobalIndexProvider, force_direct: bool }\n'
    text += 'impl WindowsGlobalIndexProvider { fn service_connected(&self){} fn service_failed(&self){} fn service_response_error(r:&IndexServiceResponse)->GlobalIndexError{service_errors::decode(r)}\n'
    text += function(mod,'fn apply_event')
    if label=='new':
        text += function(mod,'fn apply_validated_index_event')
        text += function(mod,'fn run_service_stream_with_client')
        text += function(mod,'fn finish_service_stream')
        text += function(mod,'fn run_with_fallback_using')
    else:
        text += function(mod,'fn run_with_fallback(')
        text += (ROOT/'tests/issue380_mixed_version/old_stream.rs').read_text()
    text += '}\n'
    if label=='new':
        text += between(mod,'#[derive(Debug, Clone, Copy, PartialEq, Eq)]\nenum IndexEventDisposition','impl WindowsGlobalIndexProvider')
    text += (ROOT/f'tests/issue380_mixed_version/{label}_bridge.rs').read_text()
    repository=source(ref,'repository')
    text += (ROOT/'tests/issue380_mixed_version/rebuild_bridge.rs').read_text()
    text += 'impl Database {\n' + '\n'.join(function(repository,f'pub fn {name}(') for name in ['update_global_volume_provider','update_global_volume_state','mark_global_entries_stale_for_volume']) + '}\n'
    text += 'impl Coordinator {\n' + function(coord,'pub fn rebuild(&self, source_id: Option<String>)') + '}\n'
    (crate/f'src/{label}.rs').write_text(text)
    manifest.append(dict(generated_module=label,sha256=hashlib.sha256(text.encode()).hexdigest()))
(crate/'Cargo.toml').write_text('''[package]
name="issue380-mixed-version-frozen-source"
version="0.0.0"
edition="2021"
[dependencies]
serde={version="=1.0.229",features=["derive"]}
serde_json="1"
thiserror="=2.0.20"
uuid={version="=1.25.0",features=["v4"]}
rusqlite={version="=0.39.0",features=["bundled"]}
blake3="1"
''')
(crate/'src/lib.rs').write_text((ROOT/'tests/issue380_mixed_version/harness.rs').read_text())
shutil.copyfile(ROOT/'tests/issue380_mixed_version/Cargo.lock',crate/'Cargo.lock')
for ref,path in [(D0,'docs/research/ISSUE-380-D0-TEST-CONTRACT.md'),(NEW,'docs/research/issue-380-d1a-mixed-version-contract.md')]:
    data=subprocess.check_output(['git','show',f'{ref}:{path}'],cwd=ROOT)
    manifest.append(dict(ref=ref,path=path,sha256=hashlib.sha256(data).hexdigest(),reference_only=True))
for p in sorted((ROOT/'tests/issue380_mixed_version').iterdir()):
    if p.is_file():
        manifest.append(dict(test_input=str(p.relative_to(ROOT)),sha256=hashlib.sha256(p.read_bytes()).hexdigest()))
(OUT/'sources.json').write_text(json.dumps(manifest,indent=2)+'\n')
identity={key:subprocess.check_output(['git','rev-parse',value],cwd=ROOT).decode().strip() for key,value in [('head','HEAD'),('tree','HEAD^{tree}'),('old_tree',OLD+'^{tree}'),('new_tree',NEW+'^{tree}'),('d0_tree',D0+'^{tree}')]}
identity['dirty']=subprocess.check_output(['git','status','--porcelain'],cwd=ROOT).decode()
(OUT/'run-identity.json').write_text(json.dumps(identity,indent=2)+'\n')
env = os.environ.copy()
if not shutil.which('cargo'):
    env.setdefault('CARGO_HOME','/workspace/.cache/cargo')
    env.setdefault('RUSTUP_HOME','/workspace/.cache/rustup')
    env['PATH'] = '/workspace/.cache/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin:' + env['PATH']
command = ['cargo','test','--offline','--locked','--manifest-path',str(crate/'Cargo.toml'),'--','--nocapture','--test-threads=1']
if '--prepare' not in sys.argv:
    result = subprocess.run(command,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
    (OUT/'rust-results.txt').write_text(result.stdout)
    print(result.stdout)
    if result.returncode: sys.exit(result.returncode)
    subprocess.run([sys.executable,str(ROOT/'tests/issue380_mixed_version/sqlite_installer.py')],check=True)
