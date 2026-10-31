#!/usr/bin/env python3
"""Run all B2 cases; exits nonzero on a missing assertion or oracle mismatch.

Replay: python tests/issue381_b2/run.py --output /absolute/new/evidence/directory
No repository dependencies, manifest changes, production imports, or OS claims.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import select
import signal
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import traceback

from model import Model, Rejected, connect, digest
from oracle import Reference

SEED = 38120261013


class Case:
    def __init__(self, number, output):
        self.number = number
        self.output = output
        self.tmp = tempfile.TemporaryDirectory(prefix='zc-b2-')
        self.path = Path(self.tmp.name) / 'fixture.sqlite'
        self.m = Model(self.path)
        self.o = Reference()
        self.events = []
        self.checks = []
        self.fixture = {'roots': [], 'sources': [], 'seed': SEED}
        self.extra = {}

    def event(self, name, **fields):
        self.events.append(dict(step=len(self.events), event=name, **fields))

    def root(self, rid, path):
        self.m.root(rid, path)
        self.o.root(rid, path)
        self.fixture['roots'].append([rid, path])
        self.event('register root', rid=rid, path=path)

    def source(self, uid, path=None, physical=None, text='needle'):
        path = path or '/selected/' + uid
        physical = physical or 'synthetic-volume:inode-' + uid
        rev = self.o.sources.get(uid, {}).get('observation', 0) + 1
        self.m.source(uid, path, physical, text)
        self.o.source(uid, path, physical, text, rev)
        self.o.revision += 1
        self.fixture['sources'].append([uid, path, physical, text, rev])
        self.event('source observation', uid=uid, observation=rev, physical=physical)

    def publish(self, rid, epoch, ids):
        self.m.ready(rid, epoch, ids)
        self.event('candidate sealed', rid=rid, epoch=epoch, ids=ids)
        self.m.publish(rid, epoch)
        self.o.publish(rid, epoch, ids)
        self.event('publication committed', rid=rid, epoch=epoch)

    def watcher(self, rid, applied=True):
        self.m.watcher(rid, applied)
        self.o.roots[rid]['watcher'] += 1
        self.o.roots[rid]['revision'] += 1
        self.o.revision += 1
        self.event('watcher revision', rid=rid, applied=applied)

    def eq(self, label, expected, actual):
        diff = None if expected == actual else {'expected': expected, 'actual': actual}
        self.checks.append(dict(label=label, expected=expected, actual=actual, oracle_diff=diff))
        assert diff is None, (label, diff)

    def snapshot(self, label, scope=None):
        expected = self.o.view(scope)
        actual = self.m.request(scope)
        self.eq(label, expected, {key: actual[key] for key in expected})
        self.eq(label+' selection/count', expected['ids'], actual['selection'])
        self.eq(label+' detail/preview IDs', expected['ids'], sorted(actual['detail']))
        for uid, row in actual['detail'].items():
            s = self.o.sources[uid]
            self.eq(label+' source '+uid, [s['path'],s['physical'],s['observation']], row)
        return actual

    def rank(self, label, query='needle', scope=None, limit=None):
        expected = self.o.rank(query, scope, limit)
        actual = self.m.request(scope, query, limit)['ranked']
        self.eq(label+' IDs/order', [r[0] for r in expected], [r[0] for r in actual])
        self.eq(label+' count', len(expected), len(actual))
        errors = [abs(e[1]-a[1]) for e,a in zip(expected,actual)]
        self.eq(label+' independent BM25 error <=1e-12', True, all(e<=1e-12 for e in errors))
        self.extra.setdefault('rank_records', []).append(dict(label=label, expected=expected, actual=actual, errors=errors))
        return actual

    def reject(self, label, action):
        try:
            action()
        except (Rejected, sqlite3.IntegrityError) as exc:
            self.eq(label, True, True)
            self.event('rejected', reason=str(exc), label=label)
            return
        self.eq(label, True, False)

    def metadata(self):
        return dict(annotations=[list(r) for r in self.m.db.execute('SELECT * FROM annotations ORDER BY uid')],
                    history=[list(r) for r in self.m.db.execute('SELECT * FROM history ORDER BY seq')],
                    sources=[list(r) for r in self.m.db.execute('SELECT uid,path,physical,observation FROM sources ORDER BY uid')])

    def finish(self, failure=None, negative=False):
        if self.m.db.in_transaction:
            self.m.db.execute('ROLLBACK')
        stats = dict(page_size=self.m.db.execute('PRAGMA page_size').fetchone()[0],
                     page_count=self.m.db.execute('PRAGMA page_count').fetchone()[0],
                     freelist_count=self.m.db.execute('PRAGMA freelist_count').fetchone()[0],
                     journal_mode=self.m.db.execute('PRAGMA journal_mode').fetchone()[0],
                     synchronous=self.m.db.execute('PRAGMA synchronous').fetchone()[0],
                     busy_timeout=self.m.db.execute('PRAGMA busy_timeout').fetchone()[0],
                     wal_bytes=Path(str(self.path)+'-wal').stat().st_size if Path(str(self.path)+'-wal').exists() else 0)
        # WAL sizes are captured before backup/checkpoint/close.
        backup = sqlite3.connect(self.output / (self.number + '.sqlite'))
        self.m.db.backup(backup)
        backup.close()
        record = dict(case=self.number, seed=SEED, fixture=self.fixture, event_schedule=self.events,
                      assertions=self.checks, publication_records=self.m.publication_records,
                      sqlite_storage=stats, sqlite_schema=[list(r) for r in self.m.db.execute("SELECT type,name,sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type,name")], extra=self.extra, failure=failure)
        record['expected_rejection_observed' if negative else 'passed'] = failure is None and bool(self.checks)
        self.m.close()
        self.tmp.cleanup()
        return record


def basic(c):
    c.root('A', '/selected')
    for uid in ['old','shared','new']:
        c.source(uid)
    c.publish('A', 1, ['old','shared'])


def t01(c):
    c.root('A','/selected')
    c.publish('A',1,[])
    c.snapshot('empty published corpus')
    c.source('only')
    c.publish('A',2,['only'])
    c.snapshot('one file')
    c.rank('one file rank')


def t02(c):
    basic(c)
    c.m.begin('A',2,['new','shared'])
    c.m.batch('A',2,['new'])
    c.event('first staging batch committed')
    c.snapshot('E0 during batch 1')
    c.m.batch('A',2,['shared'])
    c.m.seal('A',2)
    c.snapshot('E0 after complete sealed E1')
    c.eq('cleanup skips ready immutable candidate',0,c.m.cleanup(100))
    c.reject('sealed member deletion',lambda:c.m.db.execute("DELETE FROM members WHERE rid='A' AND epoch=2"))
    c.reject('sealed epoch proof modification',lambda:c.m.db.execute("UPDATE epochs SET writer=writer+1 WHERE rid='A' AND epoch=2"))
    c.reject('sealed member append',lambda:c.m.batch('A',2,['old']))
    if c.m.db.in_transaction:
        c.m.db.execute('ROLLBACK')
    c.reject('sealed member update',lambda:c.m.db.execute("UPDATE members SET observation=99 WHERE rid='A' AND epoch=2"))
    c.m.publish('A',2)
    c.o.publish('A',2,['new','shared'])
    c.snapshot('E1 after commit')


def t03(c):
    basic(c)
    c.m.ready('A',2,['new'])
    reader=Model(c.path,False)
    reader.db.execute('BEGIN')
    before=reader.view()
    c.event('reader snapshot pinned E0')
    other=Model(c.path,False)
    c.m.publish('A',2)
    c.o.publish('A',2,['new'])
    c.eq('pinned same-transaction list/count',before,reader.view())
    reader.db.execute('COMMIT')
    c.eq('new connection same-transaction E1',c.o.view(),other.view())
    c.snapshot('new request consistent E1')
    reader.close(); other.close()
    for field in ['run','session','revision','watcher']:
        c.m.ready('A',3,['new']) if field=='run' else None
        old=c.m.db.execute('SELECT '+field+' FROM roots WHERE rid=?',('A',)).fetchone()[0]
        c.m.db.execute('UPDATE roots SET '+field+'=? WHERE rid=?',(str(old)+'-ABA' if field in ['run','session'] else old+1,'A'))
        c.reject('wrong '+field+' ownership',lambda:c.m.publish('A',3))
        # Test-only fixture restores CAS input; publisher itself NEVER restores it.
        c.m.db.execute('UPDATE roots SET '+field+'=? WHERE rid=?',(old,'A'))
    c.eq('CAS rejects never switch pointer',2,c.m.db.execute("SELECT epoch FROM roots WHERE rid='A'").fetchone()[0])


def t04(c):
    c.root('A','/selected'); c.root('B','/selected/nested')
    for uid in ['a','shared','b']: c.source(uid,'/selected/'+uid if uid=='a' else '/selected/nested/'+uid)
    c.publish('A',1,['a','shared']); c.publish('B',1,['shared','b'])
    c.snapshot('overlap deduplicates UID')
    c.publish('A',2,[])
    c.snapshot('root A cannot erase B')
    c.snapshot('explicit root A scope',['A'])
    c.snapshot('explicit root B scope',['B'])
    c.snapshot('empty allowed root scope',[])
    c.m.db.execute("UPDATE roots SET enabled=0,revision=revision+1 WHERE rid='B'")
    c.o.roots['B']['enabled']=False; c.o.roots['B']['revision']+=1
    c.snapshot('disabled root supplies no claim')


def t05(c):
    # B2 tests root ownership, NOT a new path inclusion/exclusion policy.
    c.root('chosen-build','/Design/build'); c.root('other-build','/Cache/build')
    c.source('valuable','/Design/build/README.md')
    c.source('unrelated','/Cache/build/README.md')
    c.publish('chosen-build',1,['valuable']); c.publish('other-build',1,['unrelated'])
    c.snapshot('selected build explicit scope',['chosen-build'])
    c.eq('basename equality grants no scope', ['valuable'], c.m.view(['chosen-build'])['ids'])


def t06(c):
    c.root('A','/selected'); c.source('old','/selected/same','vol:inode-1')
    c.publish('A',1,['old'])
    c.root('B','/selected'); c.publish('B',1,['old'])
    b_token=c.m.token(['B'])
    c.m.annotate('old',0,['personal'],'old note')
    c.m.ready('A',2,[])
    c.source('replacement','/selected/same','vol:inode-2')
    c.reject('source watermark rejects same-path ABA stage',lambda:c.m.publish('A',2))
    c.publish('A',3,['replacement'])
    c.snapshot('new incarnation only')
    c.snapshot('B cannot resurrect superseded physical incarnation',['B'])
    c.reject('shared path ABA invalidates B token',lambda:c.m.validate(b_token))
    c.reject('old incarnation cannot be republished',lambda:c.m.ready('B',2,['old']))
    c.eq('replacement inherits no annotation', [], list(c.m.db.execute("SELECT uid FROM annotations WHERE uid='replacement'")))
    c.eq('old annotations retained',1,c.m.db.execute("SELECT count(*) FROM annotations WHERE uid='old'").fetchone()[0])
    c.reject('same UID new physical identity forbidden',lambda:c.m.source('old','/selected/same','vol:inode-2','needle'))


def t07(c):
    basic(c)
    for change in ['watcher','upsert','root','run','session','writer-ABA']:
        e=2+['watcher','upsert','root','run','session','writer-ABA'].index(change)
        c.m.ready('A',e,['new'])
        other=Model(c.path,False)
        if change=='watcher':
            other.watcher('A'); c.o.roots['A']['watcher']+=1; c.o.roots['A']['revision']+=1; c.o.revision+=1
        elif change=='upsert':
            other.source('new','/selected/new','synthetic-volume:inode-new','needle changed')
            c.o.source('new','/selected/new','synthetic-volume:inode-new','needle changed',2); c.o.revision+=1
        elif change=='root':
            other.db.execute("UPDATE roots SET revision=revision+1 WHERE rid='A'"); c.o.roots['A']['revision']+=1
        elif change in ['run','session']:
            other.db.execute('UPDATE roots SET '+change+"='other-owner' WHERE rid='A'")
        else:
            # Text A -> B -> A, unchanged path/UID; monotonic watermark still differs.
            for text in ['temporary','needle changed']:
                other.source('new','/selected/new','synthetic-volume:inode-new',text)
                rev=c.o.sources['new']['observation']+1
                c.o.source('new','/selected/new','synthetic-volume:inode-new',text,rev); c.o.revision+=1
        c.event('second connection commit before CAS',change=change)
        c.reject('concurrent '+change,lambda:c.m.publish('A',e))
        other.close()
        c.snapshot('E0 retained after '+change)
    c.m.ready('A',8,['new','shared'])
    old_ownership=c.m.db.execute("SELECT run,session FROM roots WHERE rid='A'").fetchone()
    c.m.begin('A',10,['new'],run=old_ownership[0],session=old_ownership[1])
    c.reject('run/session IDs reused with newer ownership revision',lambda:c.m.publish('A',8))
    c.publish('A',9,['new','shared'])
    c.snapshot('fresh source watermark can publish')


def t08(c):
    basic(c)
    for e,reason in enumerate(['incomplete','cancelled','gap','UNKNOWN','UNCERTAIN','failed'],2):
        c.m.begin('A',e,['new','shared'])
        c.m.batch('A',e,['new'])
        c.reject('coverage '+reason,lambda:c.m.seal('A',e,reason))
        c.reject('no publish '+reason,lambda:c.m.publish('A',e))
        c.eq('truthful '+reason,'reconciliation_required',c.m.db.execute("SELECT health FROM roots WHERE rid='A'").fetchone()[0])
        c.snapshot('retained E0 '+reason)
    c.m.begin('A',8,['new','shared']); c.m.batch('A',8,['new'])
    c.reject('claimed complete with missing member',lambda:c.m.seal('A',8))
    c.m.ready('A',9,['new']); c.watcher('A',False)
    c.reject('unapplied watcher gap',lambda:c.m.publish('A',9))
    c.snapshot('E0 after Watcher gap')
    c.m.begin('A',10,['new'])
    c.m.batch('A',10,['new'])
    other=Model(c.path,False)
    c.reject('member appended between complete proof and seal',lambda:c.m.seal('A',10,before_seal=lambda:other.batch('A',10,['shared'])))
    other.close()
    c.reject('seal race cannot publish',lambda:c.m.publish('A',10))
    c.snapshot('E0 after immutable seal race')


def t09(c):
    basic(c)
    c.m.annotate('old',0,['work'],'retained note','corrected','positive')
    c.m.ready('A',2,['new'])
    other=Model(c.path,False)
    other.annotate('old',1,['user-new'],'newer note','manual','negative'); other.close()
    c.event('new user correction committed while staging')
    c.reject('old user feedback CAS',lambda:c.m.annotate('old',1,['lost'],'lost'))
    expected=c.metadata()
    c.m.publish('A',2); c.o.publish('A',2,['new'])
    c.m.cleanup(1); c.m.cleanup(1); c.m.cleanup(1)
    c.eq('bounded idempotent cleanup preserves all user truth',expected,c.metadata())
    c.snapshot('hidden historical source not actionable')
    c.eq('new source no path/annotation inheritance',[],list(c.m.db.execute("SELECT uid FROM annotations WHERE uid='new'")))


def t10(c):
    basic(c)
    for kind in ['AI','analysis','duplicate','content']:
        c.m.launch(kind+'-early','old',kind)
        c.eq(kind+' current completion',True,c.m.complete(kind+'-early','kept history'))
        c.m.launch(kind+'-late','old',kind)
    c.publish('A',2,['new'])
    c.eq('pointer logically invalidates existing derived authority',[],c.m.actionable_jobs())
    for kind in ['AI','analysis','duplicate','content']:
        c.eq(kind+' stale completion rejected',False,c.m.complete(kind+'-late','late retained result'))
    c.eq('all 8 result histories retained',8,c.m.db.execute('SELECT count(*) FROM jobs').fetchone()[0])
    c.m.launch('fresh','new','AI')
    c.eq('fresh completion current',True,c.m.complete('fresh','fresh result'))
    c.eq('only fresh actionable',['fresh'],c.m.actionable_jobs())
    c.snapshot('derived source visibility')


def t11(c):
    basic(c)
    token=c.m.token(['A'])
    c.eq('valid execution source','old',c.m.execute(token,'old','/selected/old','synthetic-volume:inode-old',True))
    for path,physical,permission in [('/selected/old','new-inode',True),('/elsewhere','synthetic-volume:inode-old',True),('/selected/old','synthetic-volume:inode-old',False)]:
        c.reject('action-time independent physical/path/permission',lambda:c.m.execute(token,'old',path,physical,permission))
    c.reject('hidden ID cannot execute',lambda:c.m.execute(token,'new','/selected/new','synthetic-volume:inode-new',True))
    c.publish('A',2,['new'])
    c.reject('old token action',lambda:c.m.execute(token,'old','/selected/old','synthetic-volume:inode-old',True))
    c.snapshot('execution candidate view')


def fts_fixture(c):
    c.root('A','/selected'); c.root('B','/other')
    c.source('v1',text='needle x x x x')
    c.source('v2',text='needle needle x')
    c.source('v3',text='other other')
    for i in range(12): c.source(f'h{i:02}',text='needle '*30)
    c.publish('A',1,['v1','v2','v3'])
    c.publish('B',1,[])


def t12(c):
    fts_fixture(c)
    c.snapshot('effective FTS corpus')
    c.rank('visibility before LIMIT',limit=2)
    raw=c.m.search('needle',limit=2,raw=True)
    underfill=[r for r in raw if r[0] in c.o.visible()]
    c.eq('adversarial post-LIMIT underfill',[],underfill)
    c.extra['bad_raw_top_two']=raw
    c.rank('root scope before LIMIT',scope=['A'],limit=2)


def t13(c):
    fts_fixture(c)
    before=c.rank('visible-only baseline')
    raw_before=c.m.search('needle',raw=True)
    c.source('h00',text='hidden unrelated '*100)
    after=c.rank('hidden change no effective rank impact')
    c.eq('effective score/order invariant',before,after)
    raw_after=c.m.search('needle',raw=True)
    global_before=[r for r in raw_before if r[0] in c.o.visible()]
    global_after=[r for r in raw_after if r[0] in c.o.visible()]
    c.eq('global retained corpus scores adversary detected',True,global_before!=global_after)
    c.eq('global corpus differs from effective reference',True,global_after!=after)
    c.extra.update(global_before=global_before,global_after=global_after)
    c.m.close(); c.m=Model(c.path,False)
    c.rank('reopen visible-only FTS rebuilt equivalence')
    c.snapshot('FTS snapshot after reopen')


def t14(c):
    c.root('A','/selected'); c.root('B','/selected/nested')
    for uid,text in [('a','needle'),('b','needle x x'),('shared','other')]: c.source(uid,'/selected/'+uid if uid=='a' else '/selected/nested/'+uid,text=text)
    c.publish('A',1,['a']); c.publish('B',1,['b','shared'])
    global_token=c.m.token(query='needle'); b_token=c.m.token(['B'],'needle')
    c.watcher('A')
    c.reject('global root change invalidates',lambda:c.m.validate(global_token))
    c.m.validate(b_token); c.eq('unchanged B seals and exact rank survive A Watcher',True,True)
    c.publish('A',2,['a'])
    c.m.validate(b_token); c.eq('unchanged B rank survives unrelated pointer',True,True)
    # Add a VISIBLE A doc: B IDs/anchor unchanged but global effective BM25 changes.
    old_b_rank=c.m.search('needle',['B'])
    c.source('a2',text='needle '*50)
    c.publish('A',3,['a','a2'])
    new_b_rank=c.m.search('needle',['B'])
    c.eq('true rank changes with unchanged B result IDs',True,old_b_rank!=new_b_rank)
    c.reject('true rank invalidation',lambda:c.m.validate(b_token))
    c.extra.update(old_b_rank=old_b_rank,new_b_rank=new_b_rank)
    shared_token=c.m.token(['B'],'needle')
    c.source('shared','/selected/nested/shared',text='changed observation')
    c.reject('shared source change invalidates root B',lambda:c.m.validate(shared_token))
    scope_token=c.m.token(['B'],'needle')
    tampered=json.loads(json.dumps(scope_token)); tampered['seals']['scope']=['A']
    c.reject('scope cannot be rebound',lambda:c.m.validate(tampered))
    c.rank('root B current rank oracle',scope=['B'])
    c.snapshot('shared observation seal invalidated')


def checkpoint(name):
    print(json.dumps({'checkpoint':name}),flush=True)
    sys.stdin.readline()  # Parent kills the process while blocked at deterministic barrier.
    raise AssertionError('parent must crash child')


def crash_child(path, point):
    m=Model(path,False)
    if point=='P1':
        m.begin('A',2,['new']); m.batch('A',2,['new'],commit=False); checkpoint(point)
    elif point=='P2':
        m.ready('A',2,['new']); checkpoint(point)
    elif point=='P3-before-COMMIT':
        m.ready('A',2,['new']); m.publish('A',2,before_commit=lambda:checkpoint(point))
    elif point=='P3-after-COMMIT':
        m.ready('A',2,['new']); m.publish('A',2,after_commit=lambda:checkpoint(point))
    elif point=='P4':
        m.ready('A',2,['new']); m.publish('A',2); m.cleanup(1,commit=False); checkpoint(point)


def t15(c):
    basic(c)
    c.m.annotate('old',0,['retained'],'survive crashes')
    before_metadata=c.metadata()
    baseline=c.o.view()
    c.m.close()
    runs=[]
    for point in ['P1','P2','P3-before-COMMIT','P3-after-COMMIT','P4']:
        path=Path(c.tmp.name)/(point+'.sqlite')
        source=connect(c.path); target=sqlite3.connect(path); source.backup(target); source.close(); target.close()
        proc=subprocess.Popen([sys.executable,__file__,'--crash-child',str(path),point],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        readable,_,_=select.select([proc.stdout],[],[],30)
        assert readable,'crash barrier timeout'
        barrier=proc.stdout.readline().strip()
        assert json.loads(barrier)=={'checkpoint':point},barrier
        proc.kill(); stdout,stderr=proc.communicate(timeout=10)
        c.eq(point+' actual killed subprocess',-signal.SIGKILL,proc.returncode)
        m=Model(path,False)
        expected=json.loads(json.dumps(baseline))
        if point in ['P3-after-COMMIT','P4']:
            expected['ids']=['new']; expected['count']=1; expected['revision']+=1
            expected['roots']['A']=[2,2,0,True]
        actual=m.recover()
        c.eq(point+' recover exact old-or-new',expected,actual)
        c.eq(point+' user truth survives',before_metadata,dict(annotations=[list(r) for r in m.db.execute('SELECT * FROM annotations ORDER BY uid')],history=[list(r) for r in m.db.execute('SELECT * FROM history ORDER BY seq')],sources=[list(r) for r in m.db.execute('SELECT uid,path,physical,observation FROM sources ORDER BY uid')]))
        ref=Reference(sources=c.o.sources.copy(),roots={k:v.copy() for k,v in c.o.roots.items()},members={k:v.copy() for k,v in c.o.members.items()},revision=c.o.revision)
        if point in ['P3-after-COMMIT','P4']: ref.publish('A',2,['new'])
        c.eq(point+' FTS after restart IDs/order',[r[0] for r in ref.rank('needle')],[r[0] for r in m.search('needle')])
        c.eq(point+' independent FTS rank',True,all(abs(a[1]-b[1])<1e-12 for a,b in zip(ref.rank('needle'),m.search('needle'))))
        c.eq(point+' integrity','ok',m.db.execute('PRAGMA integrity_check').fetchone()[0])
        for _ in range(4): m.cleanup(1)
        c.eq(point+' cleanup restart idempotent',0,m.cleanup(1))
        c.eq(point+' cleanup retains published IDs',expected['ids'],m.view()['ids'])
        health=m.db.execute("SELECT health FROM roots WHERE rid='A'").fetchone()[0]
        c.eq(point+' health','healthy' if point in ['P3-after-COMMIT','P4'] else 'reconciliation_required',health)
        backup=sqlite3.connect(c.output/('T15-'+point+'.sqlite')); m.db.backup(backup); backup.close(); m.close()
        runs.append(dict(point=point,command=[sys.executable,__file__,'--crash-child',str(path),point],barrier=barrier,exit_code=proc.returncode,stdout=stdout,stderr=stderr,expected=expected,actual=actual,health=health))
    c.m=Model(c.path,False)
    c.extra['crash_records']=runs
    c.event('five subprocess SIGKILL barriers and recovery')
    c.snapshot('parent fixture unchanged')


def contend(m, publish, records):
    attempted=[threading.Event(),threading.Event()]
    completed=[]
    errors=[]
    def writer(i):
        db=connect(m.path)
        start=time.perf_counter_ns()
        # Signal from SQLite's actual statement execution, not thread intent.
        def trace(statement):
            if statement == 'BEGIN IMMEDIATE':
                attempted[i].set()
        db.set_trace_callback(trace)
        try:
            db.execute('BEGIN IMMEDIATE')
            acquired=time.perf_counter_ns()
            if i==0:
                db.execute("UPDATE sources SET text='writer correction',observation=observation+1 WHERE uid='anchor'")
                db.execute('UPDATE authority SET writer=writer+1,revision=revision+1 WHERE id=1')
            else:
                db.execute("INSERT INTO annotations VALUES('anchor',1,'[\"writer\"]','live note','manual','positive') ON CONFLICT(uid) DO UPDATE SET revision=revision+1,notes='live note'")
                db.execute("INSERT INTO history(uid,body) VALUES('anchor','writer preserved')")
            db.execute('COMMIT')
            end=time.perf_counter_ns()
            completed.append(dict(writer=['source_metadata','user_tag_note'][i],begin_wait_ms=(acquired-start)/1e6,elapsed_ms=(end-start)/1e6,exit='committed',busy_timeout_ms=5000,attempt_barrier='sqlite trace BEGIN IMMEDIATE'))
        except BaseException as exc:
            errors.append(repr(exc))
        finally: db.close()
    threads=[threading.Thread(target=writer,args=(i,)) for i in range(2)]
    def barrier():
        for t in threads: t.start()
        for event in attempted: assert event.wait(5),'writer attempt barrier timeout'
    publish(barrier)
    for t in threads:
        t.join(6); assert not t.is_alive(),'writer failed liveness'
    records.extend(completed)
    assert not errors,errors
    assert len(completed)==2
    assert all(r['elapsed_ms']<5000 for r in completed),completed


def t16(c):
    # Independent disposable instances per size; NO native/schema37 claims.
    samples=[]
    for size in [0,10000,100000]:
        path=Path(c.tmp.name)/f'contention-{size}.sqlite'
        m=Model(path); m.root('A','/selected')
        ids=[f'f{i:06}' for i in range(size)]
        stage_records=[]
        m.source('anchor','/selected/anchor','synthetic-anchor','needle')
        for start in range(0,size,1000):
            t0=time.perf_counter_ns(); m.db.execute('BEGIN IMMEDIATE')
            m.db.executemany('INSERT INTO sources(uid,path,physical,observation,text) VALUES(?,?,?,1,?)',[(uid,'/selected/'+uid,'synthetic-'+uid,'needle') for uid in ids[start:start+1000]])
            m.db.execute('UPDATE authority SET writer=writer+1,revision=revision+1 WHERE id=1'); m.db.execute('COMMIT')
            stage_records.append(dict(phase='source_fixture_batch',start=start,count=min(1000,size-start),elapsed_ms=(time.perf_counter_ns()-t0)/1e6))
        m.ready('A',1,['anchor']); m.publish('A',1)
        m.begin('A',2,ids)
        for start in range(0,size,1000):
            t0=time.perf_counter_ns(); m.batch('A',2,ids[start:start+1000])
            stage_records.append(dict(phase='membership_batch',start=start,count=min(1000,size-start),elapsed_ms=(time.perf_counter_ns()-t0)/1e6))
            c.eq(f'{size} batch {start} E0 IDs',['anchor'],m.view()['ids'])
        m.seal('A',2)
        reader=Model(path,False); reader.db.execute('BEGIN'); old=reader.view()
        writers=[]
        c.event('publication acquired -> two writer attempt barriers -> commit -> writer joins',size=size)
        contend(m,lambda barrier:m.publish('A',2,before_commit=barrier),writers)
        c.eq(f'{size} pinned reader old visibility',old,reader.view()); reader.db.execute('COMMIT'); reader.close()
        actual=m.view()
        c.eq(f'{size} exact complete new IDs',ids,actual['ids'])
        c.eq(f'{size} exact count',size,actual['count'])
        c.eq(f'{size} exact publication plus writer revision',old['revision']+2,actual['revision'])
        c.eq(f'{size} pointer epoch',2,actual['roots']['A'][0])
        c.eq(f'{size} two writer liveness',2,len(writers))
        c.eq(f'{size} correction survives', ['writer correction',2],list(m.db.execute("SELECT text,observation FROM sources WHERE uid='anchor'").fetchone()))
        c.eq(f'{size} tags/notes retained', ['["writer"]','live note'],list(m.db.execute("SELECT tags,notes FROM annotations WHERE uid='anchor'").fetchone()))
        publication=m.publication_records[-1]
        c.eq(f'{size} publish has no members/source/FTS walk',False,any('members' in s.lower() or 'update sources' in s.lower() or 'fts' in s.lower() for s in publication['statements']))
        c.eq(f'{size} fixed publication statement count',5,len(publication['statements']))
        c.eq(f'{size} writer lock under unchanged budget',True,publication['lock_ms']<5000)
        m.cleanup(1000); m.cleanup(1000)
        c.eq(f'{size} cleanup retains later source correction','writer correction',m.db.execute("SELECT text FROM sources WHERE uid='anchor'").fetchone()[0])
        samples.append(dict(size=size,ids=ids,expected_ids_sha256=digest(ids),actual_ids_sha256=digest(actual['ids']),view=actual,stage_records=stage_records,publication=publication,writers=writers,page_size=m.db.execute('PRAGMA page_size').fetchone()[0],page_count=m.db.execute('PRAGMA page_count').fetchone()[0],wal_bytes=Path(str(path)+'-wal').stat().st_size))
        m.close()
    c.fixture['contention_population']={'sizes':[0,10000,100000],'uid_format':'f + six-digit zero-padded integer in range(size)','batch_size':1000,'root':['A','/selected'],'anchor':['anchor','/selected/anchor','synthetic-anchor','needle'],'source_path':'/selected/ + UID','physical':'synthetic- + UID','document':'needle'}
    c.extra['size_records']=samples
    c.extra['provenance']='single samples; Python stdlib SQLite/Linux disposable reduced model; no percentiles, native, product 3.51.3, or end-to-end qualification'


def negative(c, label, expected, actual):
    # A broken design succeeds internally, but MUST disagree with independent truth.
    diff={'expected':expected,'actual':actual} if expected!=actual else None
    def ids(value):
        return [row if isinstance(row,str) else row[0] for row in value]
    actual_view=c.m.view()
    c.extra['counterexample']=dict(label=label,expected=expected,actual=actual,oracle_diff=diff,
        expected_state=dict(ids=ids(expected),count=len(expected),revision=c.o.revision,roots=c.o.view()['roots']),
        actual_state=dict(ids=ids(actual),count=len(actual),revision=actual_view['revision'],roots=actual_view['roots']))
    c.eq(label+' expected rejection observed',True,diff is not None)


def n01(c):
    basic(c)
    db=c.m.db
    db.execute('CREATE TABLE naive(uid TEXT PRIMARY KEY,live INTEGER)')
    db.executemany('INSERT INTO naive VALUES(?,1)',[(uid,) for uid in ['old','shared']])
    db.execute("UPDATE naive SET live=0 WHERE uid='old'")
    c.event('bad batch 1 committed; reader before batch 2')
    actual=[r[0] for r in db.execute('SELECT uid FROM naive WHERE live=1 ORDER BY uid')]
    negative(c,'mixed old/new generation',c.o.visible(),actual)
    c.eq('mixed is neither complete generation',False,actual in [['old','shared'],['new','shared']])
    c.snapshot('authoritative model still E0')


def n02(c):
    basic(c); token=c.m.token()
    c.publish('A',2,['new'])
    c.reject('correct model rejects stale cursor',lambda:c.m.validate(token))
    # Broken continuation silently replays captured IDs without current seals.
    actual=[r[0] for r in token['seals']['sources']]
    negative(c,'stale cursor accepted by broken continuation',c.o.visible(),actual)
    c.snapshot('current cursor authority')


def n03(c):
    c.root('A','/selected'); c.source('old','/selected/same','inode-old')
    c.m.annotate('old',0,['private'],'old note'); c.publish('A',1,['old'])
    c.source('new','/selected/same','inode-new'); c.publish('A',2,['new'])
    # Deliberately broken path join copies old annotations to a replacement.
    actual=[list(r) for r in c.m.db.execute('SELECT n.uid,a.tags,a.notes FROM sources n JOIN sources old ON old.path=n.path JOIN annotations a ON a.uid=old.uid WHERE n.uid=?',('new',))]
    negative(c,'path-based tag inheritance',[],actual)
    c.snapshot('incarnation oracle')


def n04(c):
    c.root('A','/selected'); c.root('B','/selected/nested'); c.source('shared','/selected/nested/shared')
    c.publish('A',1,['shared']); c.publish('B',1,['shared']); c.publish('A',2,[])
    c.m.db.execute('CREATE TABLE bad_tombstone(uid TEXT PRIMARY KEY)')
    c.m.db.execute("INSERT INTO bad_tombstone VALUES('shared')")
    actual=[uid for uid in c.m.view()['ids'] if not c.m.db.execute('SELECT 1 FROM bad_tombstone WHERE uid=?',(uid,)).fetchone()]
    negative(c,'A global tombstone destroys B claim',c.o.visible(),actual)
    c.snapshot('independent B claim')


def n05(c):
    fts_fixture(c)
    good=c.o.rank('needle',limit=2)
    raw=c.m.search('needle',raw=True)
    bad=[r for r in raw[:2] if r[0] in c.o.visible()]
    negative(c,'post-LIMIT underfill',good,bad)
    global_filtered=[r for r in raw if r[0] in c.o.visible()]
    c.eq('hidden global BM25 affects visible scores',True,global_filtered!=c.o.rank('needle'))
    c.extra['global_filtered']=global_filtered
    c.rank('correct visible corpus independently verified',limit=2)


def n06(c):
    basic(c); c.m.ready('A',2,['new'])
    c.m.launch('stale-worker','old','AI'); c.watcher('A',False)
    c.reject('correct model Watcher fence',lambda:c.m.publish('A',2))
    c.eq('correct stale worker rejected',False,c.m.complete('stale-worker','result'))
    # Deliberately broken publisher ignores run/source/watch fences.
    c.m.db.execute("UPDATE roots SET epoch=2,revision=revision+1,health='healthy' WHERE rid='A'")
    c.m.db.execute("UPDATE jobs SET status='current' WHERE jid='stale-worker'")
    c.event('bad unconditional pointer and worker update')
    negative(c,'ignored Watcher and worker revision',c.o.visible(),c.m.view()['ids'])
    c.eq('false healthy exposed by bad publisher','healthy',c.m.db.execute("SELECT health FROM roots WHERE rid='A'").fetchone()[0])
    c.eq('stale worker false current exposed','current',c.m.db.execute("SELECT status FROM jobs WHERE jid='stale-worker'").fetchone()[0])


CASES={**{f'B2-T{i:02}':globals()[f't{i:02}'] for i in range(1,17)},
       **{f'B2-N{i:02}':globals()[f'n{i:02}'] for i in range(1,7)}}


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--output',type=Path)
    parser.add_argument('--case',choices=CASES)
    parser.add_argument('--crash-child',nargs=2,metavar=('DB','POINT'))
    args=parser.parse_args()
    if args.crash_child:
        crash_child(*args.crash_child); return 0
    if args.output is None: parser.error('--output required')
    out=args.output.resolve(); out.mkdir(parents=True,exist_ok=True)
    if (out/'results.jsonl').exists(): parser.error('refusing to overwrite raw evidence; use a fresh output directory')
    if sys.flags.optimize:
        parser.error('assertions must remain enabled; do not run with -O')
    started=time.time()
    db=sqlite3.connect(':memory:')
    environment=dict(python=sys.version,executable=sys.executable,sqlite_version=sqlite3.sqlite_version,
                     sqlite_source_id=db.execute('SELECT sqlite_source_id()').fetchone()[0],
                     compile_options=[r[0] for r in db.execute('PRAGMA compile_options')],
                     os=platform.platform(),machine=platform.machine(),command=sys.argv,
                     started_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime(started)),
                     git={key:subprocess.check_output(['git',*command],text=True).strip() for key,command in {'head':['rev-parse','HEAD'],'tree':['rev-parse','HEAD^{tree}'],'branch':['branch','--show-current'],'base':['rev-parse','origin/master'],'base_tree':['rev-parse','origin/master^{tree}']}.items()},
                     seed=SEED,fts5=False,source_sha256={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in Path(__file__).parent.glob('*.py')})
    try:
        db.execute('CREATE VIRTUAL TABLE probe USING fts5(text)'); environment['fts5']=True
    except sqlite3.OperationalError:
        (out/'environment.json').write_text(json.dumps(environment,indent=2)+'\n')
        (out/'summary.json').write_text(json.dumps({'status':'B2_ENVIRONMENT_UNAVAILABLE','reason':'FTS5 missing; no LIKE substitution'})+'\n')
        return 2
    finally: db.close()
    (out/'environment.json').write_text(json.dumps(environment,indent=2)+'\n')
    chosen={args.case:CASES[args.case]} if args.case else CASES
    results=[]
    with (out/'results.jsonl').open('w') as raw:
        for number,fn in chosen.items():
            c=Case(number,out); failure=None
            try: fn(c)
            except BaseException: failure=traceback.format_exc()
            record=c.finish(failure,negative='-N' in number)
            raw.write(json.dumps(record,sort_keys=True)+'\n'); raw.flush()
            results.append(record)
            print(number, 'EXPECTED_REJECTION_OBSERVED' if '-N' in number and failure is None else 'PASS' if failure is None else 'FAIL',flush=True)
            if failure: print(failure,flush=True)
    passed=sum(r.get('passed',False) for r in results)
    rejected=sum(r.get('expected_rejection_observed',False) for r in results)
    status='B2_TEST_ONLY_ORACLE_READY_FOR_OWNER_REVIEW' if passed==16 and rejected==6 and len(results)==22 else 'B2_CONTRACT_BLOCKED'
    summary=dict(status=status,positive_passed=passed,expected_rejections_observed=rejected,
                 case_count=len(results),elapsed_seconds=time.time()-started,exit_code=0 if status.endswith('OWNER_REVIEW') else 1)
    (out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    print(json.dumps(summary),flush=True)
    return summary['exit_code']


if __name__=='__main__': sys.exit(main())
