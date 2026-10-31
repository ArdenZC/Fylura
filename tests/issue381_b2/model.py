"""Disposable SQLite research model, never imported by the application.

Source UID, synthetic physical observation, and path are three distinct values.
Expensive completeness proof and derived FTS construction happen outside publish.
A global source watermark is deliberately conservative, not a proposed production
cost/availability decision. All source writes invalidate an older staging seal.
"""
import hashlib
import json
import sqlite3
import time


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


class Rejected(Exception):
    pass


def connect(path):
    c = sqlite3.connect(path, timeout=5, isolation_level=None)
    c.execute('PRAGMA foreign_keys=ON')
    c.execute('PRAGMA busy_timeout=5000')
    return c


class Model:
    def __init__(self, path, create=True):
        self.path = str(path)
        self.db = connect(self.path)
        self.publication_records = []
        if create:
            self.db.execute('PRAGMA journal_mode=WAL')
            self.db.execute('PRAGMA synchronous=FULL')
            self.db.executescript('''
            CREATE TABLE authority(id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL, writer INTEGER NOT NULL);
            INSERT INTO authority VALUES(1,0,0);
            CREATE TABLE sources(uid TEXT PRIMARY KEY, path TEXT NOT NULL, physical TEXT NOT NULL,
              observation INTEGER NOT NULL, text TEXT NOT NULL, active INTEGER NOT NULL DEFAULT 1);
            CREATE TABLE annotations(uid TEXT PRIMARY KEY REFERENCES sources(uid), revision INTEGER NOT NULL,
              tags TEXT NOT NULL, notes TEXT NOT NULL, classification TEXT NOT NULL, feedback TEXT NOT NULL);
            CREATE TABLE history(seq INTEGER PRIMARY KEY, uid TEXT NOT NULL REFERENCES sources(uid), body TEXT NOT NULL);
            CREATE TABLE roots(rid TEXT PRIMARY KEY, path TEXT NOT NULL, enabled INTEGER NOT NULL,
              epoch INTEGER NOT NULL, revision INTEGER NOT NULL, watcher INTEGER NOT NULL,
              applied INTEGER NOT NULL, health TEXT NOT NULL, run TEXT, session TEXT,
              run_rev INTEGER NOT NULL, session_rev INTEGER NOT NULL);
            CREATE TABLE epochs(rid TEXT NOT NULL REFERENCES roots(rid), epoch INTEGER NOT NULL,
              state TEXT NOT NULL, run TEXT NOT NULL, session TEXT NOT NULL,
              root_rev INTEGER NOT NULL, watcher INTEGER NOT NULL, writer INTEGER NOT NULL,
              expected_count INTEGER NOT NULL, expected_digest TEXT NOT NULL,
              member_rev INTEGER NOT NULL, run_rev INTEGER NOT NULL, session_rev INTEGER NOT NULL,
              PRIMARY KEY(rid,epoch));
            CREATE TABLE members(rid TEXT NOT NULL, epoch INTEGER NOT NULL, uid TEXT NOT NULL REFERENCES sources(uid),
              observation INTEGER NOT NULL, PRIMARY KEY(rid,epoch,uid),
              FOREIGN KEY(rid,epoch) REFERENCES epochs(rid,epoch));
            CREATE TRIGGER immutable_member_insert BEFORE INSERT ON members
              WHEN (SELECT state FROM epochs WHERE rid=NEW.rid AND epoch=NEW.epoch)!='staging'
              BEGIN SELECT RAISE(ABORT,'sealed'); END;
            CREATE TRIGGER member_insert_revision AFTER INSERT ON members
              BEGIN UPDATE epochs SET member_rev=member_rev+1 WHERE rid=NEW.rid AND epoch=NEW.epoch; END;
            CREATE TRIGGER immutable_epoch_proof BEFORE UPDATE OF run,session,root_rev,watcher,writer,expected_count,expected_digest,run_rev,session_rev ON epochs
              WHEN OLD.state!='staging' BEGIN SELECT RAISE(ABORT,'immutable epoch proof'); END;
            CREATE TRIGGER immutable_member_update BEFORE UPDATE ON members
              BEGIN SELECT RAISE(ABORT,'immutable'); END;
            CREATE TRIGGER immutable_member_delete BEFORE DELETE ON members
              WHEN EXISTS(SELECT 1 FROM roots WHERE rid=OLD.rid AND epoch=OLD.epoch)
                OR (SELECT state FROM epochs WHERE rid=OLD.rid AND epoch=OLD.epoch) IN ('staging','ready')
              BEGIN SELECT RAISE(ABORT,'published'); END;
            CREATE TABLE jobs(jid TEXT PRIMARY KEY, uid TEXT NOT NULL REFERENCES sources(uid),
              kind TEXT NOT NULL, token TEXT NOT NULL, status TEXT NOT NULL, body TEXT NOT NULL);
            ''')

    def close(self):
        self.db.close()

    def root(self, rid, path):
        self.db.execute("INSERT INTO roots VALUES(?,?,1,0,0,0,0,'reconciliation_required',NULL,NULL,0,0)", (rid, path))

    def source(self, uid, path, physical, text, observation=1):
        self.db.execute('BEGIN IMMEDIATE')
        try:
            old = self.db.execute('SELECT path,physical,active FROM sources WHERE uid=?', (uid,)).fetchone()
            if old and (old[1] != physical or not old[2]):
                raise Rejected('incarnation UID cannot change physical identity')
            # Replacement may reuse path only with a NEW UID; old rows/history are retained.
            self.db.execute('UPDATE sources SET active=0 WHERE path=? AND uid!=?', (path, uid))
            self.db.execute('INSERT INTO sources(uid,path,physical,observation,text) VALUES(?,?,?,?,?) ON CONFLICT(uid) DO UPDATE SET path=excluded.path, observation=sources.observation+1,text=excluded.text',
                            (uid, path, physical, observation, text))
            self.db.execute('UPDATE authority SET writer=writer+1,revision=revision+1 WHERE id=1')
            self.db.execute('COMMIT')
        except BaseException:
            self.db.execute('ROLLBACK')
            raise

    def watcher(self, rid, applied=True):
        self.db.execute('BEGIN IMMEDIATE')
        self.db.execute('UPDATE roots SET watcher=watcher+1,applied=applied+?,revision=revision+1,health=? WHERE rid=?',
                        (int(applied), 'reconciliation_required', rid))
        self.db.execute('UPDATE authority SET revision=revision+1 WHERE id=1')
        self.db.execute('COMMIT')

    def begin(self, rid, epoch, expected, run='run-1', session='session-1'):
        root_path = self.db.execute('SELECT path FROM roots WHERE rid=?', (rid,)).fetchone()[0].rstrip('/')
        for uid in expected:
            path, active = self.db.execute('SELECT path,active FROM sources WHERE uid=?', (uid,)).fetchone()
            if not active:
                raise Rejected('superseded source incarnation')
            if not path.startswith(root_path + '/'):
                raise Rejected('fixture source outside selected root')
        rows = [[uid, self.db.execute('SELECT observation FROM sources WHERE uid=?', (uid,)).fetchone()[0]]
                for uid in sorted(expected)]
        self.db.execute('BEGIN IMMEDIATE')
        self.db.execute("UPDATE roots SET run=?,session=?,run_rev=run_rev+1,session_rev=session_rev+1,health='reconciliation_required' WHERE rid=?", (run, session, rid))
        r = self.db.execute('SELECT revision,watcher,run_rev,session_rev FROM roots WHERE rid=?', (rid,)).fetchone()
        w = self.db.execute('SELECT writer FROM authority').fetchone()[0]
        self.db.execute("INSERT INTO epochs VALUES(?,?,'staging',?,?,?,?,?,?,?,0,?,?)",
                        (rid, epoch, run, session, r[0], r[1], w, len(rows), digest(rows), r[2], r[3]))
        self.db.execute('COMMIT')

    def batch(self, rid, epoch, uids, commit=True):
        self.db.execute('BEGIN IMMEDIATE')
        self.db.executemany('INSERT INTO members VALUES(?,?,?,(SELECT observation FROM sources WHERE uid=?))',
                            [(rid, epoch, uid, uid) for uid in uids])
        if commit:
            self.db.execute('COMMIT')

    def seal(self, rid, epoch, coverage='complete', before_seal=None):
        # Exact complete-set scan/digest outside the publication writer lock.
        self.db.execute('BEGIN')
        rows = [list(r) for r in self.db.execute('SELECT uid,observation FROM members WHERE rid=? AND epoch=? ORDER BY uid', (rid, epoch))]
        e = self.db.execute('SELECT expected_count,expected_digest,writer,member_rev FROM epochs WHERE rid=? AND epoch=?', (rid, epoch)).fetchone()
        w = self.db.execute('SELECT writer FROM authority').fetchone()[0]
        self.db.execute('COMMIT')
        if coverage != 'complete' or (len(rows), digest(rows), w) != (e[0], e[1], e[2]):
            self.fail(rid, epoch, coverage)
            raise Rejected('incomplete/uncertain/source seal')
        if before_seal:
            before_seal()
        self.db.execute('BEGIN IMMEDIATE')
        sealed = self.db.execute("UPDATE epochs SET state='ready' WHERE rid=? AND epoch=? AND state='staging' AND member_rev=? AND writer=(SELECT writer FROM authority WHERE id=1)", (rid, epoch, e[3])).rowcount
        self.db.execute('COMMIT')
        if sealed != 1:
            self.fail(rid, epoch, 'seal_race')
            raise Rejected('candidate changed between proof and immutable seal')

    def fail(self, rid, epoch, reason):
        self.db.execute('BEGIN IMMEDIATE')
        self.db.execute("UPDATE epochs SET state=? WHERE rid=? AND epoch=? AND state!='published'", (reason, rid, epoch))
        self.db.execute("UPDATE roots SET health='reconciliation_required' WHERE rid=?", (rid,))
        self.db.execute('COMMIT')

    def publish(self, rid, epoch, before_commit=None, after_commit=None):
        trace = []
        self.db.set_trace_callback(trace.append)
        t0 = time.perf_counter_ns()
        self.db.execute('BEGIN IMMEDIATE')
        acquired = time.perf_counter_ns()
        try:
            # Fixed indexed root/epoch/authority lookups: never read members here.
            n = self.db.execute('''UPDATE roots SET epoch=?,revision=revision+1,health='healthy'
                WHERE rid=? AND enabled=1 AND watcher=applied AND EXISTS(
                SELECT 1 FROM epochs e, authority a WHERE e.rid=roots.rid AND e.epoch=? AND a.id=1
                  AND e.state='ready' AND e.root_rev=roots.revision AND e.watcher=roots.watcher
                  AND e.writer=a.writer AND e.run=roots.run AND e.session=roots.session
                  AND e.run_rev=roots.run_rev AND e.session_rev=roots.session_rev)''',
                                (epoch, rid, epoch)).rowcount
            if n != 1:
                raise Rejected('publication ownership/source/root/watcher CAS')
            self.db.execute("UPDATE epochs SET state='published' WHERE rid=? AND epoch=?", (rid, epoch))
            self.db.execute('UPDATE authority SET revision=revision+1 WHERE id=1')
            if before_commit:
                before_commit()
            self.db.execute('COMMIT')
            if after_commit:
                after_commit()
        except BaseException:
            if self.db.in_transaction:
                self.db.execute('ROLLBACK')
            raise
        finally:
            end = time.perf_counter_ns()
            self.db.set_trace_callback(None)
            self.publication_records.append(dict(begin_wait_ms=(acquired-t0)/1e6,
                lock_ms=(end-acquired)/1e6, statements=trace))

    def ready(self, rid, epoch, uids, batch_size=1000):
        self.begin(rid, epoch, uids, run=f'run-{rid}-{epoch}', session=f'session-{rid}-{epoch}')
        for start in range(0, len(uids), batch_size):
            self.batch(rid, epoch, uids[start:start+batch_size])
        self.seal(rid, epoch)

    def visible_rows(self, scope=None):
        clause = '' if scope is None else ' AND r.rid IN (' + ','.join('?' for _ in scope) + ')'
        if scope == []:
            return []
        return [list(r) for r in self.db.execute('''SELECT DISTINCT s.uid,s.path,s.physical,s.observation,s.text
            FROM roots r JOIN members m ON m.rid=r.rid AND m.epoch=r.epoch
            JOIN sources s ON s.uid=m.uid AND s.observation=m.observation
            WHERE r.enabled=1 AND s.active=1''' + clause + ' ORDER BY s.uid', tuple(scope or []))]

    def view(self, scope=None):
        rows = self.visible_rows(scope)
        roots = {r[0]: [r[1], r[2], r[3], bool(r[4])] for r in self.db.execute('SELECT rid,epoch,revision,watcher,enabled FROM roots ORDER BY rid')
                 if scope is None or r[0] in scope}
        clause = '' if scope is None else ' AND r.rid IN (' + ','.join('?' for _ in scope) + ')'
        count = 0 if scope == [] else self.db.execute('''SELECT count(DISTINCT s.uid)
            FROM roots r JOIN members m ON m.rid=r.rid AND m.epoch=r.epoch
            JOIN sources s ON s.uid=m.uid AND s.observation=m.observation
            WHERE r.enabled=1 AND s.active=1''' + clause, tuple(scope or [])).fetchone()[0]
        return dict(ids=[r[0] for r in rows], count=count,
                    revision=self.db.execute('SELECT revision FROM authority').fetchone()[0], roots=roots)

    def request(self, scope=None, query=None, limit=None):
        self.db.execute('BEGIN')
        try:
            v = self.view(scope)
            # Selection/detail/preview eligibility come from this same snapshot.
            v['selection'] = v['ids'][:]
            v['detail'] = {r[0]: r[1:4] for r in self.visible_rows(scope)}
            if query:
                v['ranked'] = self.search(query, scope, limit)
            return v
        finally:
            self.db.execute('COMMIT')

    def search(self, query, scope=None, limit=None, raw=False):
        # Small-set research materialization only; not a production cost proposal.
        rows = ([list(r) for r in self.db.execute('SELECT uid,path,physical,observation,text FROM sources ORDER BY uid')]
                if raw else self.visible_rows())
        f = sqlite3.connect(':memory:')
        try:
            f.execute("CREATE VIRTUAL TABLE docs USING fts5(uid UNINDEXED, text, tokenize='unicode61')")
            f.executemany('INSERT INTO docs VALUES(?,?)', [(r[0], r[4]) for r in rows])
            allowed = {r[0] for r in self.visible_rows(scope)}
            # MATCH + rank over effective corpus, scope before LIMIT.
            result = [list(r) for r in f.execute('SELECT uid,bm25(docs) FROM docs WHERE docs MATCH ? ORDER BY bm25(docs),uid', (query,))
                      if raw or r[0] in allowed]
            return result if limit is None else result[:limit]
        finally:
            f.close()

    def token(self, scope=None, query=None):
        self.db.execute('BEGIN')
        try:
            view = self.view(scope)
            rows = self.visible_rows(scope)
            ranks = self.search(query, scope) if query else []
            seals = dict(scope=scope, query=query, roots=view['roots'], sources=[r[:4] for r in rows], ranks=ranks)
            if scope is None:
                seals['revision'] = view['revision']
            return dict(seals=seals, digest=digest(seals))
        finally:
            self.db.execute('COMMIT')

    def validate(self, token):
        seals = token['seals']
        if token != self.token(seals['scope'], seals['query']):
            raise Rejected('cursor/source/scope/rank authority changed')

    def annotate(self, uid, expected, tags, notes, classification='manual', feedback='accepted'):
        self.db.execute('BEGIN IMMEDIATE')
        try:
            current = self.db.execute('SELECT revision FROM annotations WHERE uid=?', (uid,)).fetchone()
            if (current[0] if current else 0) != expected:
                raise Rejected('annotation CAS')
            self.db.execute('INSERT INTO annotations VALUES(?,?,?,?,?,?) ON CONFLICT(uid) DO UPDATE SET revision=excluded.revision,tags=excluded.tags,notes=excluded.notes,classification=excluded.classification,feedback=excluded.feedback',
                            (uid, expected+1, json.dumps(tags), notes, classification, feedback))
            self.db.execute('INSERT INTO history(uid,body) VALUES(?,?)', (uid, json.dumps([tags,notes,classification,feedback])))
            self.db.execute('COMMIT')
        except BaseException:
            self.db.execute('ROLLBACK')
            raise

    def launch(self, jid, uid, kind):
        token = self.token()
        if uid not in [r[0] for r in token['seals']['sources']]:
            raise Rejected('hidden job input')
        self.db.execute('INSERT INTO jobs VALUES(?,?,?,?,?,?)', (jid, uid, kind, json.dumps(token), 'pending', ''))

    def complete(self, jid, body):
        self.db.execute('BEGIN IMMEDIATE')
        try:
            uid, token = self.db.execute('SELECT uid,token FROM jobs WHERE jid=?', (jid,)).fetchone()
            # Same transaction revalidation; history survives rejection.
            saved = json.loads(token)
            v = self.view()
            seals = dict(scope=None, query=None, roots=v['roots'], sources=[r[:4] for r in self.visible_rows()], ranks=[], revision=v['revision'])
            valid = saved == dict(seals=seals, digest=digest(seals)) and uid in v['ids']
            self.db.execute('UPDATE jobs SET status=?,body=? WHERE jid=?', ('current' if valid else 'historical', body, jid))
            self.db.execute('COMMIT')
            return valid
        except BaseException:
            self.db.execute('ROLLBACK')
            raise

    def actionable_jobs(self):
        current = self.token()
        return [r[0] for r in self.db.execute("SELECT jid,token FROM jobs WHERE status='current' ORDER BY jid") if json.loads(r[1]) == current]

    def execute(self, token, uid, path, physical, permitted):
        self.validate(token)
        rows = {r[0]: r for r in self.visible_rows(token['seals']['scope'])}
        if uid not in rows or rows[uid][1:3] != [path, physical] or not permitted:
            raise Rejected('action-time source/physical/permissions')
        return uid

    def cleanup(self, limit=1000, commit=True):
        # Only obsolete membership; never DELETE sources/annotations/history/jobs.
        self.db.execute('BEGIN IMMEDIATE')
        n = self.db.execute('''DELETE FROM members WHERE rowid IN (
            SELECT m.rowid FROM members m JOIN roots r ON r.rid=m.rid
            JOIN epochs e ON e.rid=m.rid AND e.epoch=m.epoch
            WHERE m.epoch!=r.epoch AND e.state NOT IN ('staging','ready') ORDER BY m.rowid LIMIT ?)''', (limit,)).rowcount
        if commit:
            self.db.execute('COMMIT')
        return n

    def recover(self):
        self.db.execute('BEGIN IMMEDIATE')
        self.db.execute("UPDATE epochs SET state='abandoned' WHERE state IN ('staging','ready')")
        self.db.execute("UPDATE roots SET health='reconciliation_required' WHERE EXISTS(SELECT 1 FROM epochs e WHERE e.rid=roots.rid AND e.state='abandoned')")
        self.db.execute('COMMIT')
        return self.view()
