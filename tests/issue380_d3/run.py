#!/usr/bin/env python3
"""Bounded offline conformance experiment; run from any cwd with Python 3.10+.

Outputs raw JSONL, compact summary, environment and source provenance. All DBs
are disposable TemporaryDirectory fixtures. No third-party dependencies,
network, sleep, native product execution or production code modification.
"""
import argparse
import hashlib
import json
import os
import platform
import sqlite3
import subprocess
import sys
import tempfile
import traceback
from pathlib import Path
from scenarios import Scenario, execute, interrupt_case, fault_case, negative, response, before_intent_commit, legal_event_routes, source_fault_with_ok_terminal, same_source_competition, b_handshake_loss, no_inventory_checkpoint, old_intent_cancel, different_source_fault, live_owner_wake, b_expiry_release

ROOT = Path(__file__).resolve().parents[2]
TEST = Path(__file__).resolve().parent


def git(*args):
    return subprocess.check_output(['git', '-C', str(ROOT), *args], text=True).strip()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    test_hashes = {str(p.relative_to(ROOT)): sha(p.read_bytes()) for p in sorted(TEST.iterdir()) if p.suffix in ('.py', '.sql')}
    files = ['coordinator.rs', 'repository.rs', 'windows/service.rs', 'windows/service_host.rs', 'windows/mod.rs', 'windows/service_errors.rs']
    production = {}
    for filename in files:
        rel = 'src-tauri/src/global_index/' + filename
        data = subprocess.check_output(['git', '-C', str(ROOT), 'show', 'origin/master:' + rel])
        production[rel] = {'sha256': sha(data), 'git_blob': git('rev-parse', 'origin/master:' + rel)}
    db = sqlite3.connect(':memory:')
    environment = {'python': sys.version, 'executable': sys.executable, 'platform': platform.platform(),
                   'machine': platform.machine(), 'sqlite_version': sqlite3.sqlite_version,
                   'sqlite_source_id': db.execute('SELECT sqlite_source_id()').fetchone()[0],
                   'sqlite_compile_options': [r[0] for r in db.execute('PRAGMA compile_options')],
                   'cwd': str(ROOT), 'command': [sys.executable, str(TEST / 'run.py'), '--output', str(args.output)],
                   'base_head': git('rev-parse', 'origin/master'), 'base_tree': git('rev-parse', 'origin/master^{tree}'),
                   'execution_head': git('rev-parse', 'HEAD'), 'execution_tree': git('rev-parse', 'HEAD^{tree}'),
                   'working_diff': git('status', '--short'), 'test_source_sha256': test_hashes,
                   'production_read_only_provenance': production,
                   'owner_input_sha256': sha((ROOT / 'docs/project/research/issue-380-d3/owner-inputs.json').read_bytes()),
                   'qualification': 'Linux isolated SQLite research model; NOT native Windows/macOS Service acceptance',
                   'ordering': 'JSON pipe readiness / SQLite transactions / process exit barriers; no sleeps or clock-based ordering'}
    db.close()
    results, failures = [], []
    raw_path = args.output / 'raw.jsonl'
    with raw_path.open('w') as raw, tempfile.TemporaryDirectory(prefix='zen380-d3-') as temp:
        root = Path(temp)

        def run(approach, label, body):
            scenario = Scenario(root, approach, label)
            try:
                extra = body(scenario) or {}
                outcome = scenario.finish()
                outcome.update(extra)
                raw.write(json.dumps(outcome, sort_keys=True) + '\n')
                results.append({k: v for k, v in outcome.items() if k not in ('trace', 'final_snapshot', 'snapshot_catalog')})
            except BaseException as error:
                failures.append({'approach': approach, 'case': label, 'error': str(error), 'traceback': traceback.format_exc()})
                raw.write(json.dumps({'approach': approach, 'case': label, 'status': 'FAIL', 'error': str(error), 'trace': scenario.trace, 'snapshot_catalog': scenario.catalog}) + '\n')
                for worker in [scenario.worker] + scenario.extra_workers:
                    worker.close(abrupt=True)
            raw.flush()

        for approach in ('A', 'B', 'C'):
            for number in range(1, 15):
                if number != 11:
                    run(approach, 'D3-T%02d' % number, lambda s, n=number: execute(s, n))
                else:
                    for cut in ('before_offer', 'after_offer', 'after_grant', 'after_first_effect'):
                        for action in ('pause', 'cancel', 'shutdown', 'restart'):
                            run(approach, 'D3-T11-' + cut + '-' + action, lambda s, c=cut, a=action: interrupt_case(s, c, a))
            for status in ('PERMISSION_REQUIRED', 'REBUILD_REQUIRED', 'ERROR', 'PAUSED', 'UNAVAILABLE'):
                def preexisting(s, state=status):
                    s.worker.ok('external_owner', status=state, cause='true-' + state)
                    baseline = s.worker.snapshot()
                    s.request(command='Rebuild')
                    s.worker.ok('frame', request='r1', frame=response(code='index_service_busy'))
                    s.oracle.unchanged(s.worker.snapshot(), baseline=baseline)
                    s.oracle.pending(s.worker.snapshot())
                run(approach, 'D3-T03-prior-' + status, preexisting)
            for code, expected in [('index_paused', 'Paused'), ('index_permission_required', 'PermissionRequired'),
                                   ('index_rebuild_required', 'RebuildRequired'), ('index_failed', 'Provider'),
                                   ('research_windows_io', 'WindowsIo')]:
                run(approach, 'D3-T14-fault-' + expected, lambda s, c=code, e=expected: fault_case(s, c, e))
            for label, method in [('D3-T11-live-owner-wake', live_owner_wake), ('D3-T04-different-source-fault', different_source_fault), ('D3-T06-old-intent-cancel', old_intent_cancel), ('D3-T06-before-intent-commit', before_intent_commit), ('D3-T09-legal-event-routes', legal_event_routes), ('D3-T10-checkpoint-only', no_inventory_checkpoint), ('D3-T14-staged-source-fault', source_fault_with_ok_terminal), ('D3-T05-same-source-competition', same_source_competition)]:
                run(approach, label, method)
            if approach == 'B':
                run(approach, 'D3-T09-expiry-release', b_expiry_release)
                run(approach, 'D3-T11-offer-lost-service-restart', lambda s: b_handshake_loss(s, False))
                run(approach, 'D3-T11-grant-lost-service-restart', lambda s: b_handshake_loss(s, True))
            # Run ALL six negative algorithms against each candidate fixture;
            # unsafe SQL is deliberately outside the candidate's guarded API.
            for number in range(1, 7):
                run(approach, 'N%02d' % number, lambda s, n=number: negative(s, n))
        def coverage_gap(s):
            s.request(command='Rebuild')
            s.prepare()
            before = s.worker.snapshot()
            other = s.independent()
            actual = other.ok('raw_write')
            result = s.oracle.expected_rejection(actual, 'C_unregistered_database_writer_bypasses_arbiter', before)
            result['status'] = 'BLOCKED_EXPECTED_REJECTION'
            result['platform_authority'] = 'BLOCKED'
            result['scope'] = 'persistent cooperative arbiter is not mandatory authority for independent existing SQL connections'
            return result
        run('C', 'C-unified-authority-coverage', coverage_gap)
    matrix = {}
    for approach in ('A', 'B', 'C'):
        suite = [r for r in results if r['approach'] == approach]
        failed = [f for f in failures if f['approach'] == approach]
        matrix[approach] = {'passing_cases': len(suite), 'failures': len(failed),
                            'T01_T14': all(any(r['case'].startswith('D3-T%02d' % n) for r in suite) for n in range(1, 15)) and not failed,
                            'negative_controls_rejected': sum(bool(r.get('expected_rejection_observed')) for r in suite if r['case'].startswith('N')),
                            'full_scope': 'BLOCKED_unregistered_SQL_writer' if approach == 'C' else ('PASS_research_scope' if not failed else 'FAIL')}
    ready = not failures and all(matrix[a]['T01_T14'] and matrix[a]['negative_controls_rejected'] == 6 for a in ('A', 'B'))
    summary = {'terminal': 'ISSUE_380_D3_COMPARATIVE_TEST_ONLY_READY_FOR_OWNER_REVIEW' if ready else 'D3_CONTRACT_BLOCKED',
               'recommended': 'A' if ready else None, 'production': 'NO_GO_pending_Owner_schema_writer_coverage_and_native_acceptance',
               'matrix': matrix, 'cases': results, 'failures': failures,
               'raw_sha256': sha(raw_path.read_bytes()), 'test_source_sha256': test_hashes,
               'exit_code': 0 if ready else 1}
    (args.output / 'summary.json').write_text(json.dumps(summary, indent=2, sort_keys=True) + '\n')
    environment['exit_code'] = summary['exit_code']
    (args.output / 'environment.json').write_text(json.dumps(environment, indent=2, sort_keys=True) + '\n')
    print(json.dumps({k: summary[k] for k in ('terminal', 'matrix', 'failures', 'exit_code')}, indent=2))
    return summary['exit_code']


if __name__ == '__main__':
    raise SystemExit(main())
