#!/usr/bin/env python3
"""Offline actual-source P1 boundary probe, NOT a replacement native test suite.

Compiles verbatim extracted production definitions (DbError, GlobalIndexError,
IndexServiceResponse and constants), includes production service_errors.rs and
its real test module. No fake error enum or response structs. Three independent
repaired-contract assertions intentionally FAIL on the current public API.
All generated Cargo/target files belong to an ignored task-owned directory.
"""
import argparse
import hashlib
import json
import os
import platform
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


def definition(path, name):
    data = (ROOT / path).read_text()
    start = data.index('pub ' + name)
    start = data.rfind('#[derive(', 0, start)
    brace = data.index('{', start)
    depth = 1
    end = brace + 1
    while depth:
        depth += (data[end] == '{') - (data[end] == '}')
        end += 1
    return data[start:end] + '\n'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--fixture', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    args.fixture.mkdir(parents=True, exist_ok=True)
    fixtures = {
        'db-error.rs': definition('src-tauri/src/db/types.rs', 'enum DbError'),
        'global-error.rs': definition('src-tauri/src/global_index/coordinator.rs', 'enum GlobalIndexError'),
        'response.rs': definition('src-tauri/src/global_index/windows/service.rs', 'struct IndexServiceResponse'),
    }
    protocol = re.search(r'^pub const IPC_PROTOCOL_VERSION:[^\n]+', (ROOT / 'src-tauri/src/global_index/windows/service.rs').read_text(), re.M)
    assert protocol
    fixtures['protocol.rs'] = protocol.group(0) + '\n'
    models = (ROOT / 'src-tauri/src/global_index/models.rs').read_text()
    fixtures['models.rs'] = '\n'.join(re.findall(r'^pub const INDEX_STATUS_(?:ERROR|PAUSED|PERMISSION_REQUIRED|REBUILD_REQUIRED):[^\n]+', models, re.M)) + '\n'
    assert len(fixtures['models.rs'].splitlines()) == 4
    for filename, content in fixtures.items():
        (args.fixture / filename).write_text(content)
    cargo = '''[package]
name = "issue380-p1-actual-source-boundary"
version = "0.0.0"
edition = "2021"
[lib]
path = "lib.rs"
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
rusqlite = { version = "0.39", features = ["bundled"] }
r2d2 = "0.8"
'''
    (args.fixture / 'Cargo.toml').write_text(cargo)
    original = ROOT / 'src-tauri/src/global_index/windows/service_errors.rs'
    lib = '''pub mod db { use thiserror::Error; include!("db-error.rs"); }
pub mod global_index {
 pub mod coordinator { use crate::db::DbError; use thiserror::Error; include!("global-error.rs"); }
 pub mod models { include!("models.rs"); }
 pub mod windows {
  pub mod service { use serde::{Serialize,Deserialize}; include!("protocol.rs"); include!("response.rs"); }
  #[path=SOURCE_PATH] pub mod service_errors;
  #[cfg(test)] mod required_contract {
   use super::service::IndexServiceResponse;
   use super::service_errors::decode;
   use crate::global_index::coordinator::GlobalIndexError;
   use std::mem::discriminant;
   fn require_distinct(code: &str) {
    let response=IndexServiceResponse{protocol_version:3,request_id:"bound-active".into(),ok:false,error_code:Some(code.into()),message:Some("same diagnostic".into()),status:None};
    let actual=decode(&response);
    assert_ne!(discriminant(&actual),discriminant(&GlobalIndexError::Provider("same diagnostic".into())),"P1 typed semantic erased at GlobalIndexProvider boundary: {code}");
   }
   #[test] fn p1_required_service_busy_is_not_provider() {require_distinct("index_service_busy");}
   #[test] fn p1_required_permission_is_not_provider() {require_distinct("index_permission_required");}
   #[test] fn p1_required_unknown_is_not_provider() {require_distinct("index_outcome_unknown");}
  }
 }
}
'''.replace('SOURCE_PATH', json.dumps(str(original)))
    (args.fixture / 'lib.rs').write_text(lib)
    environment = dict(os.environ)
    toolchain = '/workspace/.cache/rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin'
    environment['PATH'] = toolchain + ':' + environment['PATH']
    environment['CARGO_HOME'] = '/workspace/.cache/cargo'
    environment['RUSTUP_HOME'] = '/workspace/.cache/rustup'
    environment['CARGO_TARGET_DIR'] = str(args.fixture.resolve() / 'target')
    runs = []
    for label, selector in [('characterization', 'p1_scope_'), ('required-contract', 'p1_required_')]:
        command = ['cargo', 'test', '--offline', '--manifest-path', str(args.fixture / 'Cargo.toml'), selector, '--', '--nocapture']
        result = subprocess.run(command, env=environment, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        (args.output / (label + '.log')).write_text(result.stdout)
        runs.append({'label':label,'command':command,'exit_code':result.returncode,'log_sha256':hashlib.sha256(result.stdout.encode()).hexdigest()})
    actual_failures = (args.output / 'required-contract.log').read_text()
    expected = runs[0]['exit_code'] == 0 and runs[1]['exit_code'] == 101 and '3 failed' in actual_failures and 'P1 typed semantic erased' in actual_failures
    inputs = ['src-tauri/src/db/types.rs','src-tauri/src/global_index/coordinator.rs','src-tauri/src/global_index/models.rs','src-tauri/src/global_index/windows/service.rs','src-tauri/src/global_index/windows/service_errors.rs','src-tauri/src/global_index/windows/service_errors_p1_scope_tests.rs']
    report = {'terminal':'ISSUE_380_P1_OWNER_ESCALATION_REQUIRED','product':'ISSUE_380_PRODUCT_FIX_NO_GO',
              'platform':platform.platform(),'rustc':subprocess.check_output(['rustc','--version'],env=environment,text=True).strip(),
              'execution_head':subprocess.check_output(['git','-C',str(ROOT),'rev-parse','HEAD'],text=True).strip(),
              'execution_tree':subprocess.check_output(['git','-C',str(ROOT),'rev-parse','HEAD^{tree}'],text=True).strip(),
              'source_sha256':{p:hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in inputs},
              'runner_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'generated_harness_sha256':hashlib.sha256(lib.encode()).hexdigest(),
              'fixture_cargo_sha256':hashlib.sha256(cargo.encode()).hexdigest(),
              'fixture_lock_sha256':hashlib.sha256((args.fixture / 'Cargo.lock').read_bytes()).hexdigest(),
              'extracted_definitions_sha256':{k:hashlib.sha256(v.encode()).hexdigest() for k,v in fixtures.items()},
              'runs':runs,'required_contract_failures_observed':expected,
              'qualification':'Actual Rust production decoder and verbatim definitions; reduced Linux crate, NOT desktop/native runtime qualification'}
    (args.output / 'probe.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))
    return 0 if expected else 1


if __name__ == '__main__':
    raise SystemExit(main())
