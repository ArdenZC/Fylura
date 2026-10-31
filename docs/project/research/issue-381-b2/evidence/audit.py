"""Inspect raw output and backed-up fixtures independently of the runner's verdict."""
import hashlib
import json
from pathlib import Path
import sqlite3

p = Path(__file__).resolve().parent
records = [json.loads(line) for line in (p / 'results.jsonl').read_text().splitlines()]
required = {f'B2-T{i:02}' for i in range(1, 17)} | {f'B2-N{i:02}' for i in range(1, 7)}
assert len(records) == 22 and {r['case'] for r in records} == required
assert all(r['failure'] is None for r in records)
assert all(r.get('passed') is True for r in records if '-T' in r['case'])
assert all(r.get('expected_rejection_observed') is True and r['extra']['counterexample']['oracle_diff'] for r in records if '-N' in r['case'])
assert all(all(a['oracle_diff'] is None for a in r['assertions']) for r in records)
environment = json.loads((p / 'environment.json').read_text())
assert environment['fts5']
for name, expected in environment['source_sha256'].items():
    assert hashlib.sha256(Path('tests/issue381_b2', name).read_bytes()).hexdigest() == expected
snapshots = list(p.glob('*.sqlite'))
assert len(snapshots) == 27
for snapshot in snapshots:
    db = sqlite3.connect(snapshot)
    assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
    assert db.execute('PRAGMA foreign_key_check').fetchall() == []
    db.close()
print(json.dumps(dict(positive_passed=16,expected_rejections_observed=6,independent_negative_diffs=6,
                      assertions=sum(len(r['assertions']) for r in records),
                      sqlite_integrity_and_fk_snapshots=len(snapshots),source_hashes_match=True)))
