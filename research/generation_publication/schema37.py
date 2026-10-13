"""Test-only Schema 37 domain slice, extracted from the checked-out authority.

No production migration is invoked. Unrelated automation/content/rule domains
are excluded. Every included table retains the production columns/constraints.
"""
import hashlib
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'src-tauri/src/db/schema.rs'
TEXT = SOURCE.read_text()
SOURCE_SHA256 = hashlib.sha256(SOURCE.read_bytes()).hexdigest()


def raw_sql(function):
    start = TEXT.index('fn ' + function + '(')
    end = TEXT.find('\nfn ', start + 1)
    section = TEXT[start:end if end != -1 else len(TEXT)]
    return re.findall(r'r#"(.*?)"#', section, re.S)


def install(c):
    assert 'CURRENT_SCHEMA_VERSION: i32 = 37' in TEXT
    blocks = re.findall(r'r#"(.*?)"#', TEXT, re.S)
    for table in ('files', 'operation_batches', 'cleanup_trash_batches',
                  'classification_history'):
        c.executescript(next(b for b in blocks if
                            b.lstrip().startswith('CREATE TABLE IF NOT EXISTS ' + table + ' (')))
    # Apply literal production additive columns, including multiline raw SQL.
    alterations = re.findall(r'ALTER TABLE (?:files|operation_logs|operation_batches|cleanup_trash_items)\b[^;]+;', TEXT)
    for sql in alterations:
        try:
            c.execute(sql)
        except Exception as exc:
            if 'duplicate column name' not in str(exc):
                raise
    for function in ('ensure_global_index_schema', 'ensure_global_index_hardening',
                     'ensure_trigram_fts', 'ensure_fts_triggers',
                     'ensure_file_library_schema', 'ensure_organization_plan_schema',
                     'ensure_scan_ledger_schema', 'ensure_dedupe_schema', 'ensure_analysis_schema',
                     'ensure_journal_state_triggers'):
        for sql in raw_sql(function):
            c.executescript(sql)
    c.executescript("CREATE TABLE app_settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);")
    # This identifies the source slice only; it is NOT a v37 migration artifact.
    c.execute('PRAGMA user_version=37')


def columns(c, table):
    return [r[1] for r in c.execute(f'PRAGMA table_info({table})')]


def insert(c, table, **values):
    names = ','.join(values)
    marks = ','.join('?' for _ in values)
    c.execute(f'INSERT INTO {table}({names}) VALUES({marks})', tuple(values.values()))


def seed(c, n):
    insert(c, 'global_volumes', id='v', platform='windows', stable_volume_id='volume-1',
           display_name='Fixture', mount_path='C:/fixture', filesystem_type='NTFS',
           drive_kind='fixed', provider='ntfs', index_status='ready', journal_id='j1',
           journal_cursor='old', created_at=1, updated_at=1)
    insert(c, 'managed_scopes', id='scope', path='C:/fixture', created_at=1, updated_at=1)
    insert(c, 'user_tags', id='tag', display_name='Preserve', normalized_name='preserve',
           created_at=1, updated_at=1)
    insert(c, 'operation_batches', id='history-batch', created_at=1, status='completed')
    insert(c, 'analysis_runs', id='run', request_key='rk', scope_json='{}', scope_hash='scope',
           source_snapshot_json='{}', source_snapshot_hash='snapshot', detector_set_json='[]',
           detector_set_hash='detectors', status='completed', phase='completed', created_at=1, updated_at=1)
    c.execute('BEGIN IMMEDIATE')
    for i in range(n):
        eid, fid, path = f'entry-{i}', f'stable-file-{i}', f'C:/fixture/oldtoken-{i}.txt'
        # Two namespace entries may identify the same physical object (hardlinks).
        physical = f'physical-{i // 2}'
        insert(c, 'global_entries', id=eid, volume_id='v', platform_file_id=physical,
               parent_platform_file_id='parent', name=f'oldtoken-{i}.txt',
               name_normalized=f'oldtoken-{i}.txt', path=path, path_normalized=path.lower(),
               extension='txt', size=4096+i, created_at_fs=1, modified_at_fs=10+i,
               file_attributes=32, is_hidden=i % 2, source_provider='ntfs', last_seen_at=1)
        insert(c, 'files', id=fid, path=path, name=f'oldtoken-{i}.txt', extension='txt',
               size=4096+i, mtime=10+i, ctime=1, purpose='User purpose',
               classification_status='classified', context='user correction', last_seen_at=1)
        insert(c, 'file_fingerprints', file_id=fid, path_snapshot=path, identity_status='verified',
               platform_kind='windows', platform_volume_id='volume-1', platform_file_id=physical,
               physical_key='windows:v1:volume-1:'+physical, link_count=2, size=4096+i,
               modified_ns=10+i, fingerprint_status='complete', full_hash='hash',
               captured_at=1, last_verified_at=1)
        insert(c, 'managed_entries', id=f'managed-{i}', global_entry_id=eid,
               managed_scope_id='scope', created_at=1, updated_at=1)
        insert(c, 'ai_analysis_state', global_entry_id=eid, status='completed',
               input_fingerprint='old', model='fixture', content_summary='Summary '*32,
               classification_json='{"purpose":"user correction"}', user_corrected=1, updated_at=1)
        insert(c, 'ai_jobs', id=f'job-{i}', global_entry_id=eid, managed_scope_id='scope',
               input_fingerprint='old', provider='local', status='running' if i % 2 else 'canceled', created_at=1)
        insert(c, 'ai_job_items', id=f'item-{i}', job_id=f'job-{i}', global_entry_id=eid,
               status='running' if i % 2 else 'canceled', created_at=1, updated_at=1)
        insert(c, 'file_user_tags', file_id=fid, tag_id='tag', created_at=1)
        insert(c, 'classification_history', id=f'hist-{i}', file_id=fid,
               file_name=f'oldtoken-{i}.txt', file_path=path, source='user', file_type='Document',
               purpose='User purpose', lifecycle='Inbox', risk_level='Normal', suggested_action='Keep',
               user_confirmed=1, created_at=1)
        insert(c, 'classification_feedback', id=f'feedback-{i}', file_id=fid,
               file_name=f'oldtoken-{i}.txt', original_json='{}', corrected_json='{"user":true}', created_at=1)
        insert(c, 'operation_logs', id=f'log-{i}', batch_id='history-batch', operation_type='rename',
               source_path=path, target_path=path, old_name=f'oldtoken-{i}.txt', new_name=f'oldtoken-{i}.txt',
               status='success', created_at=1, path_before=path, path_after=path,
               name_before=f'oldtoken-{i}.txt', name_after=f'oldtoken-{i}.txt')
        insert(c, 'analysis_findings', id=f'finding-{i}', finding_key=f'key-{i}', run_id='run',
               detector_id='fixture', detector_version=1, scope_hash='scope', status='active',
               tier='review', category='fixture', action_kind='none', title='Historical finding',
               reason='fixture', confidence='exact', primary_subject_kind='file', primary_subject_id=fid,
               path_snapshot=path, identity_snapshot_json='{"physical":"'+physical+'"}', created_at=1, updated_at=1)
        insert(c, 'analysis_finding_evidence', id=f'evidence-{i}', finding_id=f'finding-{i}',
               evidence_kind='fixture', subject_kind='file', subject_id=fid, path_snapshot=path,
               value_json='{}', created_at=1)
        insert(c, 'analysis_finding_decisions', finding_key=f'key-{i}', decision='dismissed',
               note='user decision', created_at=1, updated_at=1)
    c.execute('COMMIT')
