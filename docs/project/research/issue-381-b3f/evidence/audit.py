"""Inspect preserved native outputs, reference checks and exact source bytes."""
import gzip
import hashlib
import json
from pathlib import Path
import sys

p=Path(__file__).resolve().parent
root=Path.cwd()
summary=json.loads((p/'summary.json').read_text())
assert summary['status']=='#381_B3F_FEASIBLE_CANDIDATE_READY_FOR_OWNER_REVIEW'
assert summary['oracle_checks']==760
checks=[json.loads(l) for l in (p/'independent-oracle-checks.jsonl').read_text().splitlines()]
assert len(checks)==760 and all(c['exact_float_bits_and_full_order_equal'] and c['oracle_diff'] is None for c in checks)
e=json.loads((p/'environment.json').read_text())
for name,expected in e['source_sha256'].items():assert hashlib.sha256((root/name).read_bytes()).hexdigest()==expected
measurements=[json.loads(l) for l in (p/'results.jsonl').read_text().splitlines()]
by_case={}
for r in measurements:by_case.setdefault(r['case'],r)
seen=set();crashes=0;writer_sets=0;gc_rows=[]
for case,record in by_case.items():
 archive=p/(case+'-native.jsonl.gz');digest=hashlib.sha256();byte_count=0
 with gzip.open(archive,'rb') as raw:
  for line in raw:
   digest.update(line);byte_count+=len(line);r=json.loads(line);seen.add(r['case'])
   if r['case']=='B3F-C05':
    assert r['before_user_data_sha256']==r['after_user_data_sha256']
    assert r['physical_sqlite_changes']>1 and r['shadow_sqlite_changes']==0
   if r['case']=='B3F-C09':
    for crash in r['actual_SIGKILL_records']:
     assert crash['signal']==9 and crash['expected_epoch']==crash['actual_epoch']
     assert crash['expected_ids']==crash['actual_ids'] and crash['F1']==crash['F2']
     crashes+=1
   if 'timing' in r:
    pub=r['timing']['publication'];assert pub['writer_lock_ms']<5000 and pub['statement_count']==4
    assert len(r['timing']['competing_writers'])==2
    assert all(w['committed'] and w['timeout_ms']==5000 and w['elapsed_ms']<5000 and w['attempt_barrier']=='actual SQLITE_TRACE_STMT BEGIN IMMEDIATE' for w in r['timing']['competing_writers'])
    writer_sets+=1
    for batch in r['expired_pin_cleanup_batches']:
     for deletion in batch['deleted']:assert deletion['members_removed']<=1000;gc_rows.append(deletion['members_removed'])
    assert r['all_physical_sources_retained']==record['population']
   if 'paired' in r:
    for q in r['paired']['records']:
     assert q['F1']==q['F2']
     if q['independent_native_reference'] is not None:assert q['F1']==q['independent_native_reference']
     assert [uid for page in q['pages_of_3'] for uid in page]==[h['uid'] for h in q['F1']]
     if q['legacy_short_like_ids'] is not None:assert q['legacy_short_like_ids']['exact_like_ids_equal']
 assert digest.hexdigest()==record['raw_sha256'] and byte_count==record['raw_bytes']
assert {f'B3F-C{i:02}' for i in range(1,13)}<=seen
assert crashes==6 and writer_sets==4
print(json.dumps(dict(native_cases=sorted(seen),independent_checks=len(checks),exact_source_hashes=True,unmodified_native_archives_verified=True,sigkill_barriers=crashes,competing_writer_pairs=writer_sets,bounded_gc_batches=len(gc_rows),max_gc_members_per_batch=max(gc_rows))))
