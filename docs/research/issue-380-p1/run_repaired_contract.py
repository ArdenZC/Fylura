#!/usr/bin/env python3
"""Offline production client/admission/decoder tests, not Windows native acceptance.

Imports complete service.rs/service_errors.rs; extracts unchanged model/error/sink
contracts verbatim. Historical failure evidence is never overwritten.
"""
import argparse
import hashlib
import json
import os
import platform
import re
import subprocess
import sys
from pathlib import Path
sys.dont_write_bytecode = True
from run_error_boundary_probe import definition, ROOT


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--fixture', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.fixture.mkdir(parents=True, exist_ok=True)
    args.output.mkdir(parents=True, exist_ok=True)
    paths = ['src-tauri/src/db/types.rs', 'src-tauri/src/global_index/coordinator.rs',
             'src-tauri/src/global_index/models.rs', 'src-tauri/src/global_index/windows/service.rs',
             'src-tauri/src/global_index/windows/service_errors.rs',
             'src-tauri/src/global_index/windows/service_errors_p1_scope_tests.rs',
             'src-tauri/src/global_index/windows/service_p1_tests.rs',
             'src-tauri/src/global_index/windows/service_host.rs',
             'src-tauri/src/global_index/windows/service_host_p1_tests.rs']
    fixtures = {'db.rs': definition(paths[0], 'enum DbError'),
                'error.rs': definition(paths[1], 'enum GlobalIndexError')}
    models = (ROOT / paths[2]).read_text()
    fixtures['models.rs'] = '\n'.join(re.findall(r'^pub const [^\n]+', models, re.M)) + '\n'
    for name in ['GlobalVolume', 'GlobalEntry', 'GlobalEntryInput', 'GlobalSourceDescriptor']:
        fixtures['models.rs'] += definition(paths[2], 'struct ' + name)
    coordinator = (ROOT / paths[1]).read_text()
    start = coordinator.index('pub trait GlobalIndexSink:')
    end = coordinator.index('\npub trait GlobalIndexProvider:', start)
    fixtures['sink.rs'] = coordinator[start:end]
    provider_end = coordinator.index('\n#[derive(Debug, Default)]', end)
    fixtures['provider.rs'] = coordinator[end:provider_end]
    host = (ROOT / paths[7]).read_text()
    host_start = host.index('struct ServiceRuntime {')
    host_end = host.index('\nfn service_handler(', host_start)
    # Only the unused constructor default is stubbed on Linux. Each runtime
    # experiment immediately replaces it with its explicit controlled provider.
    # Windows CI compiles the original complete service_host.rs without this stub.
    constructor_stub = r"""
struct DirectWindowsGlobalIndexProvider;
impl DirectWindowsGlobalIndexProvider {fn new()->Self{Self}}
impl GlobalIndexProvider for DirectWindowsGlobalIndexProvider {
 fn discover_sources(&self)->Result<Vec<GlobalSourceDescriptor>,GlobalIndexError>{unreachable!()}
 fn start_initial_index(&self,_:&GlobalSourceDescriptor,_:&mut dyn GlobalIndexSink,_:&AtomicBool)->Result<(),GlobalIndexError>{unreachable!()}
 fn resume_incremental_sync(&self,_:&GlobalSourceDescriptor,_:&mut dyn GlobalIndexSink,_:&AtomicBool)->Result<(),GlobalIndexError>{unreachable!()}
 fn pause(&self)->Result<(),GlobalIndexError>{unreachable!()}
 fn status(&self)->Result<String,GlobalIndexError>{unreachable!()}
 fn shutdown(&self)->Result<(),GlobalIndexError>{unreachable!()}
}
"""
    fixtures['runtime-core.rs'] = ('use super::service::*;\nuse crate::global_index::coordinator::*;\n'
        'use crate::global_index::models::*;\nuse std::sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}};\n'
        + constructor_stub + host[host_start:host_end]
        + '\n#[cfg(test)] #[path=' + json.dumps(str(ROOT / paths[8])) + '] mod p1_tests;\n')
    for filename, content in fixtures.items():
        (args.fixture / filename).write_text(content)
    cargo = '''[package]
name="issue380-p1-repaired-production-client"
version="0.0.0"
edition="2021"
[lib]
path="lib.rs"
[dependencies]
serde={version="1",features=["derive"]}
serde_json="1"
thiserror="2"
rusqlite={version="0.39",features=["bundled"]}
r2d2="0.8"
uuid={version="1",features=["v4"]}
'''
    (args.fixture / 'Cargo.toml').write_text(cargo)
    lib = '''pub mod db {use thiserror::Error;include!("db.rs");}
pub mod global_index {
 pub mod models {use serde::{Serialize,Deserialize};include!("models.rs");}
 pub mod coordinator {use crate::db::DbError;use thiserror::Error;use super::models::{GlobalEntry,GlobalEntryInput};include!("error.rs");include!("sink.rs");use super::models::GlobalSourceDescriptor;use std::sync::atomic::AtomicBool;use std::time::Duration;include!("provider.rs");}
 pub mod windows {
  pub mod service_host {include!("runtime-core.rs");}
  #[path=SERVICE] pub mod service;
  #[path=ERRORS] pub mod service_errors;
 }
}
'''.replace('SERVICE', json.dumps(str(ROOT / paths[3]))).replace('ERRORS', json.dumps(str(ROOT / paths[4])))
    (args.fixture / 'lib.rs').write_text(lib)
    env = dict(os.environ)
    env['PATH'] = '/workspace/.cache/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin:' + env['PATH']
    env['CARGO_HOME'] = '/workspace/.cache/cargo'
    env['RUSTUP_HOME'] = '/workspace/.cache/rustup'
    env['CARGO_TARGET_DIR'] = str(args.fixture.resolve() / 'target')
    command = ['cargo', 'test', '--offline', '--manifest-path', str(args.fixture / 'Cargo.toml'), '--', '--nocapture']
    result = subprocess.run(command, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    record = {'command': command, 'exit_code': result.returncode, 'content': result.stdout,
              'stdout_sha256': hashlib.sha256(result.stdout.encode()).hexdigest(),
              'platform': platform.platform(),
              'rustc': subprocess.check_output(['rustc', '--version'], env=env, text=True).strip(),
              'execution_head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
              'execution_tree': subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], cwd=ROOT, text=True).strip(),
              'source_sha256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in paths},
              'runner_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'extracted_sha256': {k: hashlib.sha256(v.encode()).hexdigest() for k, v in fixtures.items()},
              'harness_sha256': hashlib.sha256(lib.encode()).hexdigest(),
              'qualification': 'Complete production client/decoder imported; ServiceRuntime/PipeSink core verbatim with unused Direct constructor stub and controlled test providers. Linux core tests are not Windows native API acceptance. Normal Windows CI compiles complete real runtime.'}
    (args.output / 'repaired-client.json').write_text(json.dumps(record, indent=2) + '\n')
    print(result.stdout)
    return result.returncode


if __name__ == '__main__':
    raise SystemExit(main())
