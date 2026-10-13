"""Isolated research mechanisms, never a production Repository replacement."""
import copy
import sqlite3
import time
from contextlib import contextmanager

from schema37 import columns, install, seed

TABLES = ('global_entries', 'files', 'managed_entries', 'ai_analysis_state',
          'ai_jobs', 'ai_job_items', 'global_volumes', 'file_fingerprints', 'analysis_findings')
KEYS = {t: 'id' for t in TABLES}
KEYS.update(ai_analysis_state='global_entry_id', file_fingerprints='file_id')
HISTORY = ('classification_history', 'classification_feedback', 'operation_logs',
           'file_user_tags', 'analysis_finding_evidence', 'analysis_finding_decisions', 'managed_scopes')


def connect(path):
    c = sqlite3.connect(path, isolation_level=None, timeout=5)
    c.row_factory = sqlite3.Row
    c.execute('PRAGMA foreign_keys=ON')
    c.execute('PRAGMA journal_mode=WAL')
    c.execute('PRAGMA synchronous=NORMAL')
    c.execute('PRAGMA wal_autocheckpoint=0')
    c.execute('PRAGMA busy_timeout=5000')
    return c


def rows(c, table):
    return [dict(r) for r in c.execute(f'SELECT * FROM {table} ORDER BY 1')]


def oracle(old):
    """Independent row-level expected projection; no SQL or publish helpers."""
    result = copy.deepcopy(old)
    count = len(old['global_entries'])
    missing = f'entry-{count-1}' if count else None
    for r in result['global_entries']:
        r['path'] = r['path'].replace('/oldtoken-', '/newtoken/oldtoken-')
        r['path_normalized'] = r['path'].lower()
        r['size'] += 100
        r['modified_at_fs'] += 100
        r['last_seen_at'] = 2
        r['is_stale'] = int(r['id'] == missing)
    for r in result['files']:
        r['path'] = r['path'].replace('/oldtoken-', '/newtoken/oldtoken-')
        r['size'] += 100
        r['mtime'] += 100
        r['last_seen_at'] = 2
        r['is_stale'] = int(r['id'] == f'stable-file-{count-1}')
    for r in result['managed_entries']:
        if r['global_entry_id'] == missing:
            r.update(enabled=0, updated_at=2)
    for r in result['ai_analysis_state']:
        r.update(status='stale', last_error='input_changed', updated_at=2)
        # Never discard the correction just because metadata changed.
    for t in ('ai_jobs', 'ai_job_items'):
        for r in result[t]:
            if r['status'] in ('running', 'pending'):
                r.update(status='stale', last_error='input_changed')
                r['completed_at' if t == 'ai_jobs' else 'updated_at'] = 2
    for r in result['file_fingerprints']:
        r.update(identity_status='stale', fingerprint_status='stale', revision=r['revision']+1)
        # Retain historical physical identity; require native revalidation.
    for r in result['analysis_findings']:
        r.update(status='stale', stale_at=2, updated_at=2, revision=r['revision']+1)
    result['global_volumes'][0].update(index_status='ready', journal_cursor='new',
                                      entry_count=max(0, count-1), last_full_index_at=2, updated_at=2)
    return result


class Fenced(Exception):
    pass


class Prototype:
    def __init__(self, path, strategy, n=None):
        self.path, self.strategy = str(path), strategy
        self.c = connect(self.path)
        self.locks = []
        if n is not None:
            install(self.c)
            seed(self.c, n)
            self.c.executescript('''
                CREATE TABLE rp_control(id INTEGER PRIMARY KEY CHECK(id=1),
                  active INTEGER NOT NULL, revision INTEGER NOT NULL,
                  metadata_revision INTEGER NOT NULL, phase TEXT NOT NULL,
                  expected_identity TEXT NOT NULL);
                INSERT INTO rp_control VALUES(1,1,1,0,'staging','volume-1');
                CREATE TABLE rp_manifest(generation INTEGER PRIMARY KEY, complete INTEGER NOT NULL);
                INSERT INTO rp_manifest VALUES(1,1);
                CREATE TABLE rp_leases(token TEXT PRIMARY KEY,generation INTEGER NOT NULL);
            ''')
            # Copies carry every source column. They are experimental projections,
            # not migrated FK authorities; durable FK anchors remain in v37 tables.
            for t in TABLES:
                self.c.execute(f'CREATE TABLE rp_{t} AS SELECT 1 AS generation,* FROM {t}')
                self.c.execute(f'CREATE UNIQUE INDEX rp_{t}_key ON rp_{t}(generation,{KEYS[t]})')
                self.c.execute(f'CREATE INDEX rp_{t}_versions ON rp_{t}({KEYS[t]},generation)')
            self.c.executescript('''
                CREATE VIRTUAL TABLE rp_fts USING fts5(name,path,extension,tokenize='trigram');
                CREATE TABLE rp_fts_keys(rowid INTEGER PRIMARY KEY,generation INTEGER NOT NULL,
                                         entry_id TEXT NOT NULL,UNIQUE(generation,entry_id));
                CREATE INDEX rp_fts_generation ON rp_fts_keys(generation,entry_id);
            ''')
            with self.transaction('fixture:fts') as c:
                self.index_fts(c, 1, rows(c, 'global_entries'))
        self.old = self.read()
        self.expected = oracle(self.old)
        self.history = self.history_rows()
        self.metadata_revision = self.c.execute('SELECT metadata_revision FROM rp_control').fetchone()[0]

    def close(self):
        self.c.close()

    @contextmanager
    def transaction(self, label, c=None):
        c = c or self.c
        attempt = time.perf_counter()
        c.execute('BEGIN IMMEDIATE')
        acquired = time.perf_counter()
        outcome = 'commit'
        rollback_ms = 0
        try:
            yield c
            c.execute('COMMIT')
        except BaseException:
            outcome = 'rollback'
            rollback_start = time.perf_counter()
            c.execute('ROLLBACK')
            rollback_ms = (time.perf_counter()-rollback_start)*1000
            raise
        finally:
            self.locks.append(dict(label=label, wait_ms=(acquired-attempt)*1000,
                                   held_ms=(time.perf_counter()-acquired)*1000,
                                   rollback_ms=rollback_ms, outcome=outcome))

    def fence(self, c, revision=1):
        control = c.execute('SELECT * FROM rp_control').fetchone()
        source = c.execute('SELECT * FROM global_volumes WHERE id="v"').fetchone()
        if (control['revision'] != revision or control['phase'] != 'staging'
                or control['metadata_revision'] != self.metadata_revision
                or not source['enabled'] or source['stable_volume_id'] != control['expected_identity']
                or source['mount_path'] != 'C:/fixture' or source['provider'] != 'ntfs'):
            raise Fenced('operation/source/metadata changed')

    def index_fts(self, c, generation, entries):
        for r in entries:
            cursor = c.execute('INSERT INTO rp_fts_keys(generation,entry_id) VALUES(?,?)',
                               (generation, r['id']))
            c.execute('INSERT INTO rp_fts(rowid,name,path,extension) VALUES(?,?,?,?)',
                      (cursor.lastrowid, r['name'], r['path'], r['extension']))

    def stage(self, batch=250, hook=None):
        # C1 also stages full rows outside active tables, then merges in one tx.
        # C2: complete generation snapshots. C3: append changed row versions only.
        self.c.execute('INSERT INTO rp_manifest VALUES(2,0)')
        for t in TABLES:
            old_by_key = {r[KEYS[t]]: r for r in self.old[t]}
            data = self.expected[t]
            if self.strategy == 'C3':
                data = [r for r in data if r != old_by_key.get(r[KEYS[t]])]
            for start in range(0, len(data), batch):
                part = data[start:start+batch]
                with self.transaction('stage:'+t) as c:
                    self.fence(c)
                    names = ','.join(columns(c, t))
                    marks = ','.join('?' for _ in columns(c, t))
                    c.executemany(f'INSERT INTO rp_{t}(generation,{names}) VALUES(2,{marks})',
                                  [tuple(r.values()) for r in part])
                    if t == 'global_entries' and self.strategy != 'C1':
                        self.index_fts(c, 2, part)
                if hook:
                    hook(t, start)
        with self.transaction('stage:seal') as c:
            self.fence(c)
            c.execute('UPDATE rp_manifest SET complete=1 WHERE generation=2')

    def publish(self, fail_after=None, revision=1, hook=None, step_hook=None):
        with self.transaction('publish') as c:
            self.fence(c, revision)
            if c.execute('SELECT complete FROM rp_manifest WHERE generation=2').fetchone()[0] != 1:
                raise Fenced('incomplete generation')
            if hook:
                hook(c)
            if self.strategy == 'C1':
                for t in TABLES:
                    names = columns(c, t)
                    mutable = [name for name in names if name != KEYS[t]]
                    assignments = ','.join(name+'=?' for name in mutable)
                    c.executemany(f'UPDATE {t} SET {assignments} WHERE {KEYS[t]}=?',
                                  [tuple(r[name] for name in mutable)+(r[KEYS[t]],)
                                   for r in self.expected[t]])
                    if fail_after == t:
                        # Actual SQL error, after prior table updates/triggers.
                        c.execute('INSERT INTO rp_missing_table VALUES(1)')
                    if step_hook:
                        step_hook(t)
            c.execute("UPDATE rp_control SET active=2,phase='completed' WHERE id=1 AND revision=?", (revision,))
            if step_hook:
                step_hook('pointer')
            if fail_after == 'pointer':
                c.execute('INSERT INTO rp_missing_table VALUES(1)')

    def read(self, c=None, generation=None):
        c = c or self.c
        own = not c.in_transaction
        if own:
            c.execute('BEGIN')
        try:
            generation = generation or c.execute('SELECT active FROM rp_control').fetchone()[0]
            if self.strategy == 'C1':
                return {t: rows(c, t) for t in TABLES}
            result = {}
            for t in TABLES:
                names = ','.join('r.'+col for col in columns(c, t))
                condition = 'r.generation=?'
                if self.strategy == 'C3':
                    condition = (f'r.generation<=? AND NOT EXISTS(SELECT 1 FROM rp_{t} newer '
                                 f'WHERE newer.{KEYS[t]}=r.{KEYS[t]} AND newer.generation<=? '
                                 'AND newer.generation>r.generation)')
                params = (generation, generation) if self.strategy == 'C3' else (generation,)
                result[t] = [dict(r) for r in c.execute(f'SELECT {names} FROM rp_{t} r '
                                                        f'WHERE {condition} ORDER BY r.{KEYS[t]}', params)]
            return result
        finally:
            if own:
                c.execute('COMMIT')

    def search(self, token, c=None):
        c = c or self.c
        own = not c.in_transaction
        if own:
            c.execute('BEGIN')
        try:
            if self.strategy == 'C1':
                return [r[0] for r in c.execute('''SELECT ge.id FROM global_entries_fts
                    JOIN global_entries ge ON ge.rowid=global_entries_fts.rowid
                    JOIN global_volumes gv ON gv.id=ge.volume_id
                    WHERE global_entries_fts MATCH ? AND gv.enabled=1 AND ge.is_stale=0
                    ORDER BY ge.modified_at_fs DESC,ge.id ASC''', (token,))]
            gen = c.execute('SELECT active FROM rp_control').fetchone()[0]
            predicate = 'k.generation=?'
            params = (token, gen)
            if self.strategy == 'C3':
                predicate = '''k.generation<=? AND NOT EXISTS(SELECT 1 FROM rp_global_entries newer
                    WHERE newer.id=e.id AND newer.generation<=? AND newer.generation>e.generation)'''
                params = (token, gen, gen)
            return [r[0] for r in c.execute(f'''SELECT e.id FROM rp_fts
                JOIN rp_fts_keys k ON k.rowid=rp_fts.rowid
                JOIN rp_global_entries e ON e.id=k.entry_id AND e.generation=k.generation
                WHERE rp_fts MATCH ? AND {predicate} AND e.is_stale=0
                ORDER BY e.modified_at_fs DESC,e.id ASC''', params)]
        finally:
            if own:
                c.execute('COMMIT')

    def history_rows(self):
        return {t: rows(self.c, t) for t in HISTORY}

    def cancel(self):
        with self.transaction('cancel') as c:
            c.execute("UPDATE rp_control SET revision=revision+1,phase='cancelled'")

    def gc(self):
        with self.transaction('gc') as c:
            active = c.execute('SELECT active FROM rp_control').fetchone()[0]
            if c.execute('SELECT 1 FROM rp_leases WHERE generation<?', (active,)).fetchone():
                return False
            for t in TABLES:
                predicate = 'generation<?'
                if self.strategy == 'C3':
                    predicate += (f' AND EXISTS(SELECT 1 FROM rp_{t} newer WHERE '
                                  f'newer.{KEYS[t]}=rp_{t}.{KEYS[t]} AND newer.generation={active})')
                c.execute(f'DELETE FROM rp_{t} WHERE {predicate}', (active,))
            # Delete only FTS versions whose corresponding namespace row was reclaimed.
            keys = c.execute('''SELECT k.rowid FROM rp_fts_keys k WHERE NOT EXISTS
                (SELECT 1 FROM rp_global_entries e WHERE e.id=k.entry_id AND e.generation=k.generation)''').fetchall()
            c.executemany('DELETE FROM rp_fts WHERE rowid=?', keys)
            c.executemany('DELETE FROM rp_fts_keys WHERE rowid=?', keys)
            # Retain v37 identity anchors: GC must never cascade user/history state.
            return True
