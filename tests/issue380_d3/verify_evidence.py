#!/usr/bin/env python3
"""Offline integrity/coverage check of the published evidence, not another run."""
import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def verify(directory):
    summary = json.loads((directory / 'summary.json').read_text())
    environment = json.loads((directory / 'environment.json').read_text())
    raw = (directory / 'raw.jsonl').read_bytes()
    assert hashlib.sha256(raw).hexdigest() == summary['raw_sha256'], 'raw bytes differ'
    assert summary['test_source_sha256'] == environment['test_source_sha256']
    for path, digest in summary['test_source_sha256'].items():
        assert hashlib.sha256((ROOT / path).read_bytes()).hexdigest() == digest, 'test source differs: ' + path
    records = [json.loads(line) for line in raw.splitlines()]
    assert len(records) == len(summary['cases'])
    for record in records:
        for digest, snapshot in record['snapshot_catalog'].items():
            assert hashlib.sha256(json.dumps(snapshot, sort_keys=True).encode()).hexdigest() == digest
        for message in record['trace']:
            if message.get('input', {}).get('method') == 'snapshot':
                assert message['output']['snapshot_sha256'] in record['snapshot_catalog']
    required = {'D3-T%02d' % n for n in range(1, 15) if n != 11}
    required |= {'D3-T11-' + cut + '-' + action for cut in ('before_offer', 'after_offer', 'after_grant', 'after_first_effect') for action in ('pause', 'cancel', 'shutdown', 'restart')}
    for approach in ('A', 'B', 'C'):
        cases = {r['case']: r for r in records if r['approach'] == approach}
        assert required <= set(cases), 'incomplete T01-T14'
        assert all(cases[name]['status'] == 'PASS' for name in required)
        for n in range(1, 7):
            control = cases['N%02d' % n]
            assert control['expected_rejection_observed'] and control['oracle_failure']
    c = next(r for r in records if r['case'] == 'C-unified-authority-coverage')
    assert c['status'] == 'BLOCKED_EXPECTED_REJECTION' and c['expected_rejection_observed']
    assert summary['exit_code'] == environment['exit_code'] == 0 and not summary['failures']
    print(json.dumps({'verified_records': len(records), 'raw_sha256': summary['raw_sha256'],
                      'negative_controls_verified': 18, 'test_source_files': len(summary['test_source_sha256'])}, indent=2))


if __name__ == '__main__':
    verify(Path(sys.argv[1]))
