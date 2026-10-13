"""Shared adversarial inputs; A/B/C results are judged by oracle.py only."""
import copy
import hashlib
import json
import subprocess
import sys
from pathlib import Path
from oracle import Oracle

INVENTORY = [('kept', 'new-kept'), ('added', 'new-added')]


def event(request='r1', source='s1', kind='Entries', **payload):
    names = {'Entries': 'entries', 'EntryStale': 'entry_stale', 'VolumeEntriesStale': 'volume_entries_stale',
             'Checkpoint': 'checkpoint', 'FindEntryByIdentity': 'find_entry_by_identity', 'ResolveParentPath': 'resolve_parent_path',
             'SourceProvider': 'source_provider', 'SourceState': 'source_state', 'Sources': 'sources'}
    data = {'event': names.get(kind, kind), 'volume_id': source}
    if kind == 'Entries':
        data.pop('volume_id')
        rows = payload.pop('rows', INVENTORY)
        data['entries'] = [{'volume_id': source, 'name': e, 'value': v} for e, v in rows]
    if kind == 'EntryStale':
        data.pop('volume_id')
    if kind == 'Checkpoint':
        data.update(journal_id='journal-fixture', journal_cursor=str(payload.pop('cursor', 90)))
    data.update(payload)
    return {'kind': 'event', 'request_id': request, 'event': data}


def response(request='r1', code=None, **overrides):
    result = {'protocol_version': 3, 'request_id': request,
              'ok': code is None, 'error_code': code, 'message': 'index_service_busy text is not a classifier', 'status': None}
    result.update(overrides)
    return {'kind': 'response', 'response': result}


class Worker:
    def __init__(self, path, approach, trace, catalog):
        self.path, self.approach, self.trace, self.catalog = path, approach, trace, catalog
        self.process = subprocess.Popen([sys.executable, str(Path(__file__).with_name('process.py')), str(path), approach],
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        ready = json.loads(self.process.stdout.readline())
        assert ready['barrier'] == 'process_ready'
        self.trace.append({'process': ready, 'approach': approach, 'database': path.name})

    def call(self, method, **args):
        command = {'method': method, 'args': args}
        self.process.stdin.write(json.dumps(command) + '\n')
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        assert line, 'controlled child exited unexpectedly: ' + self.process.stderr.read()
        result = json.loads(line)
        recorded = result
        if method == 'snapshot' and result['ok']:
            digest = hashlib.sha256(json.dumps(result['result'], sort_keys=True).encode()).hexdigest()
            self.catalog[digest] = result['result']
            recorded = {'ok': True, 'snapshot_sha256': digest}
        self.trace.append({'pid': self.process.pid, 'input': command, 'output': recorded})
        return result

    def ok(self, method, **args):
        result = self.call(method, **args)
        assert result['ok'], result
        return result['result']

    def snapshot(self):
        return self.ok('snapshot')

    def reject(self, method, **args):
        before = self.snapshot()
        result = self.call(method, **args)
        assert result.get('rejected'), result
        assert self.snapshot() == before, 'rejected input changed database'
        return result['rejected']

    def close(self, abrupt=False):
        if self.process.poll() is None:
            if abrupt:
                self.process.kill()
            else:
                self.process.stdin.write('{"method":"exit"}\n')
                self.process.stdin.flush()
        self.process.wait(timeout=15)
        self.trace.append({'pid': self.process.pid, 'exit_code': self.process.returncode, 'abrupt': abrupt})
        self.process.stdin.close()
        self.process.stdout.close()
        self.process.stderr.close()


class Scenario:
    def __init__(self, root, approach, case):
        self.approach, self.case = approach, case
        self.path = root / (approach + '-' + case + '.sqlite')
        self.trace, self.catalog = [], {}
        self.worker = Worker(self.path, approach, self.trace, self.catalog)
        self.initial = self.worker.ok('initialize')
        self.oracle = Oracle(self.initial)
        self.extra_workers = []

    def request(self, **args):
        return self.worker.ok('request', **args)

    def prepare(self, request='r1'):
        if self.approach == 'B':
            token = self.worker.ok('offer', request=request)
            self.worker.ok('grant', request=request, token=token['token'])
        if self.approach == 'C':
            self.worker.ok('arbitrate', request=request)

    def stream(self, request='r1', source='s1'):
        self.worker.ok('frame', request=request, frame=event(request, source))
        self.worker.ok('frame', request=request, frame=event(request, source, 'Checkpoint', cursor=90))

    def restart(self):
        self.worker.close(abrupt=True)
        self.worker = Worker(self.path, self.approach, self.trace, self.catalog)
        return self.worker.ok('recover')

    def independent(self):
        worker = Worker(self.path, self.approach, self.trace, self.catalog)
        self.extra_workers.append(worker)
        return worker

    def finish(self):
        final = self.worker.snapshot()
        for worker in self.extra_workers:
            worker.close()
        self.worker.close()
        return {'case': self.case, 'approach': self.approach, 'status': 'PASS',
                'oracle_checks': self.oracle.observations, 'final_snapshot': final, 'trace': self.trace, 'snapshot_catalog': self.catalog}


def execute(s, number):
    w, o = s.worker, s.oracle
    if number in (1, 2, 3):
        s.request(command={1: 'Start', 2: 'Resume', 3: 'Rebuild'}[number])
        # C reserves DB authority, independently from Service resource admission.
        if s.approach == 'C':
            w.ok('arbitrate', request='r1')
        assert w.ok('frame', request='r1', frame=response(code='index_service_busy')) == 'ServiceBusy'
        o.unchanged(w.snapshot())
        if number == 3:
            o.pending(w.snapshot())
        # Recovery uses an explicit existing-wake input, not a new retry loop.
        s.request(request='wake-r2', command={1: 'Start', 2: 'Resume', 3: 'Rebuild'}[number])
        s.prepare('wake-r2')
        s.stream('wake-r2')
        w.ok('frame', request='wake-r2', frame=response('wake-r2'))
        o.published(w.snapshot(), INVENTORY, full=number != 2)
    elif number == 4:
        s.request(command='Rebuild')
        s.request(source='s2', request='r2', command='Rebuild')
        w.ok('frame', request='r1', frame=response(code='index_service_busy'))
        o.unchanged(w.snapshot())
        o.pending(w.snapshot())
        s.prepare('r2')
        s.stream('r2', 's2')
        w.ok('frame', request='r2', frame=response('r2'))
        o.unchanged(w.snapshot())
        o.published(w.snapshot(), INVENTORY, source='s2')
    elif number == 5:
        s.request(command='Rebuild')
        if s.approach == 'B':
            offer = w.ok('offer', request='r1')
            w.reject('frame', request='r1', frame=event())  # no ACK grant
        producer = subprocess.run([sys.executable, str(Path(__file__).with_name('producer.py'))],
                                  input=json.dumps({'protocol_version': 3, 'request_id': 'r1', 'source': 's1'}),
                                  text=True, capture_output=True, check=True, timeout=15)
        produced = json.loads(producer.stdout)
        s.trace.append({'independent_service_producer': produced, 'exit_code': producer.returncode})
        assert produced['computed_before_client_owner'] and produced['database_handle'] is None
        o.unchanged(w.snapshot())
        peer = s.independent()
        peer.ok('external_owner', status='PERMISSION_REQUIRED', cause='trusted-access-denied')
        baseline = peer.snapshot()
        if s.approach == 'A':
            w.reject('frame', request='r1', frame=produced['frame'])
        elif s.approach == 'B':
            w.reject('grant', request='r1', token=offer['token'])
        else:
            w.reject('arbitrate', request='r1')
        o.unchanged(w.snapshot(), baseline=baseline)
        o.pending(w.snapshot())
    elif number == 6:
        s.request(command='Rebuild')
        s.request(request='coalesced-r2', command='Rebuild')
        o.pending(w.snapshot())
        replay = s.restart()
        assert len(replay) == 1 and replay[0]['key'] == 'rebuild-s1'
        w = s.worker
        s.request(request='after-restart', command='Rebuild')
        w.ok('interrupt', request='after-restart', action='cancel')
        o.pending(w.snapshot(), state='cancelled')
        assert s.restart() == []
        o.pending(s.worker.snapshot(), state='cancelled')
        o.unchanged(s.worker.snapshot())
    elif number in (7, 8):
        s.request(command='Rebuild')
        peer = s.independent()
        states = ('READY', 'PERMISSION_REQUIRED', 'REBUILD_REQUIRED', 'ERROR') if number == 7 else ('READY', 'READY')
        for i, state in enumerate(states):
            before = peer.snapshot()
            peer.ok('external_owner', status=state, cause=None if state == 'READY' else 'true-' + state, metadata='generation-' + str(i))
            baseline = peer.snapshot()
            o.revision_advanced(before, baseline)
            w.reject('frame', request='r1', frame=response(code='index_service_busy'))
            w.reject('frame', request='r1', frame=event())
            w.reject('interrupt', request='r1', action='transport')
            o.unchanged(w.snapshot(), baseline=baseline)
    elif number == 9:
        s.request()
        s.prepare()
        bad = [response('forged-id'), response(protocol_version=2), response(protocol_version=4),
               response(protocol_version=None), response(protocol_version=3.0), response(code='index_service_busy', ok=True), response(ok='true'), response(code='unknown_code'),
               response(code='', ok=False), response(code=None, ok=False),
               event('forged-id'), event(source='forged-source'), {'kind': 'lookup_response', 'response': {'lookup_id': 'r1-lookup'}},
               event(kind='EntryStale', entry_id='s2:removed'), event(kind='Sources', sources=[{'id': 's2'}]),
               event(kind='FindEntryByIdentity', lookup_id='old-lookup'), event(kind='Unknown')]
        for frame in bad:
            w.reject('frame', request='r1', frame=frame)
        o.unchanged(w.snapshot())
        if s.approach == 'B':
            # Explicit research-only offer token replay/expiry/instance mismatch.
            w.ok('interrupt', request='r1', action='cancel')
            s.request(request='offer-expired')
            offered = w.ok('offer', request='offer-expired', expiry=2)
            w.reject('grant', request='offer-expired', token=offered['token'], now=2)
            w.reject('grant', request='offer-expired', token='forged')
            w.reject('grant', request='offer-expired', token=offered['token'], instance='service-restarted')
            w.reject('offer', request='offer-expired')
    elif number == 10:
        s.request(command='Rebuild')
        s.prepare()
        assert w.ok('frame', request='r1', frame=response()) == 'no-effect-success-preserved'
        o.unchanged(w.snapshot())
        o.pending(w.snapshot())
        w.reject('frame', request='r1', frame=response())
        w.reject('frame', request='r1', frame=event())
    elif number == 11:
        # A/B/C all see these same interruption cuts; the preparation handshake
        # differs. Actual fresh-process restart; no wall clock ordering.
        raise AssertionError('T11 is expanded by run.py')
    elif number == 12:
        s.request(command='Rebuild')
        s.prepare()
        s.stream()
        o.unchanged(w.snapshot())  # staging has no published side effects
        assert w.ok('interrupt', request='r1', action='transport') == 'unknown'
        o.incomplete(w.snapshot())
        w.reject('transfer', request='r1', proof='released_no_effect')
        w.reject('frame', request='r1', frame=response())
        s.restart()
        o.incomplete(s.worker.snapshot())
        o.pending(s.worker.snapshot())
    elif number == 13:
        s.request(command='Rebuild')
        # NotSent is an explicit pre-send transport classification, not inferred
        # from no observed events. The harness supplies proven fixture input.
        w.ok('transfer', request='r1', proof='NotSent')
        peer = s.independent()
        peer.ok('request', request='competitor')
        if s.approach == 'C':
            peer.reject('arbitrate', request='competitor')
        if s.approach == 'B':
            offer = peer.ok('offer', request='competitor')
            peer.reject('grant', request='competitor', token=offer['token'])
        peer.reject('frame', request='competitor', frame=event('competitor'))
        s.stream()
        w.ok('frame', request='r1', frame=response())
        o.published(w.snapshot(), INVENTORY)
        routes = [r for r in w.snapshot()['operations'] if r['state'] == 'completed']
        assert len(routes) == 1 and routes[0]['route'] == 'Direct'
    elif number == 14:
        s.request(command='Rebuild')
        s.prepare()
        s.stream()
        o.unchanged(w.snapshot())
        locker = s.independent()
        assert locker.ok('lock')['barrier'] == 'write_lock_held'
        result = w.call('frame', request='r1', frame=response())
        assert result['sqlite_errorcode'] == 5 and result['sqlite_errorname'] == 'SQLITE_BUSY'
        assert result['error'] == 'OperationalError' and not result.get('rejected')
        o.unchanged(locker.snapshot())
        locker.ok('unlock')
        # Actual abrupt crash after first publication SQL before transaction COMMIT.
        assert w.ok('crash_publication', request='r1')['barrier'] == 'after_first_SQL_before_commit'
        o.unchanged(locker.snapshot())  # other connection cannot see half-invalidation
        s.restart()
        w = s.worker
        o.incomplete(w.snapshot())
        s.request(request='after-crash', command='Rebuild')
        s.prepare('after-crash')
        s.stream('after-crash')
        w.ok('frame', request='after-crash', frame=response('after-crash'))
        o.published(w.snapshot(), INVENTORY)
        o.unchanged(w.snapshot(), source='s2')
    else:
        raise AssertionError('unknown case')


def interrupt_case(s, cut, action):
    s.request(command='Rebuild')
    w, o = s.worker, s.oracle
    if cut == 'after_offer':
        if s.approach == 'B':
            w.ok('offer', request='r1')
        else:
            computed = subprocess.run([sys.executable, str(Path(__file__).with_name('producer.py'))], input=json.dumps({'protocol_version': 3, 'request_id': 'r1', 'source': 's1'}), text=True, capture_output=True, check=True, timeout=15)
            s.trace.append({'pure_service_computation_before_client_claim': json.loads(computed.stdout), 'exit_code': computed.returncode})
    elif cut in ('after_grant', 'after_first_effect'):
        s.prepare()
        if s.approach == 'A' and cut == 'after_grant':
            # A has NO Offer/Grant wire phase. Equivalent durable ownership cut:
            # a validated read-only lookup event claimed owner, no data effect.
            w.ok('frame', request='r1', frame=event(kind='FindEntryByIdentity', lookup_id='first-lawful-lookup', platform_file_id='file', parent_platform_file_id='parent', name='kept'))
    if cut == 'after_first_effect':
        s.stream()
    if action == 'restart':
        s.restart()
        w = s.worker
    else:
        w.ok('interrupt', request='r1', action=action)
    if cut == 'after_first_effect':
        o.unchanged(w.snapshot(), allow_reconciliation=True)
        assert next(r for r in w.snapshot()['health'] if r['source'] == 's1')['reconciliation'] == 1
    else:
        o.unchanged(w.snapshot())
    o.pending(w.snapshot(), state='cancelled' if action == 'cancel' else 'pending')
    w.reject('frame', request='r1', frame=event())
    w.reject('frame', request='r1', frame=response())


def fault_case(s, code, expected):
    s.request()
    s.prepare()
    if code == 'research_windows_io':
        s.worker.ok('transfer', request='r1', proof='released_no_effect')
    assert s.worker.ok('frame', request='r1', frame=response(code=code, message='genuine-' + expected)) == expected
    health = s.worker.snapshot()['health'][0]
    assert health['cause'] == 'genuine-' + expected
    assert health['checkpoint'] == 42 and health['full_time'] == 7 and health['incremental_time'] == 8
    s.oracle.unchanged(s.worker.snapshot(), source='s2')


def negative(s, number):
    """Execute unsafe behaviors as actual SQL, retaining their rejected results.

These deliberately bad algorithms have separate mutations; they do not invoke
safe candidate transitions and then merely assert that an enum exists.
"""
    s.request(command='Rebuild')
    w, o = s.worker, s.oracle
    if number == 1:
        mutation = "UPDATE health SET status='INDEXING',cause=NULL WHERE source='s1'"
        invariant = 'skip_busy_leaves_speculative_health'
    elif number == 2:
        w.ok('external_owner', status='PERMISSION_REQUIRED', cause='new-access-denial')
        baseline = w.snapshot()
        mutation = "UPDATE health SET status='READY',cause=NULL WHERE source='s1'"
        invariant = 'blind_restore_overwrites_new_owner'
    elif number == 3:
        mutation = "UPDATE global_rows SET stale=1 WHERE source='s1'; UPDATE managed SET enabled=0 WHERE source='s1'; UPDATE health SET count=0 WHERE source='s1'; UPDATE ai_jobs SET state='stale' WHERE source='s1'; UPDATE ai_items SET state='stale' WHERE source='s1'; UPDATE ai_analysis SET state='stale' WHERE source='s1'"
        invariant = 'pre_admission_destructive_stale'
    elif number == 4:
        w.ok('external_owner', status='READY', metadata='ABA-new-owner')
        baseline = w.snapshot()
        mutation = "UPDATE health SET status='ERROR',cause='late-old-error' WHERE source='s1'; UPDATE global_rows SET stale=1 WHERE source='s1'"
        invariant = 'old_event_same_status_ABA'
    elif number == 5:
        s.prepare()
        s.stream()
        w.ok('interrupt', request='r1', action='transport')
        mutation = "UPDATE operations SET route='Direct' WHERE request='r1'; UPDATE health SET reconciliation=0,status='READY' WHERE source='s1'; INSERT INTO global_rows VALUES ('s1','direct-replayed','unsafe-replay',0)"
        invariant = 'unknown_no_replay'
    else:
        # A real resource lease artifact (a separate file) does not reserve the
        # independent Service slot or authorize SQL writes.
        lease = s.path.with_suffix('.scheduler-lease')
        lease.write_text('CPU_IO=1 granted; service reservation absent\n')
        mutation = "UPDATE health SET status='INDEXING' WHERE source='s1'; UPDATE global_rows SET stale=1 WHERE source='s1'"
        invariant = 'scheduler_lease_is_not_service_admission'
    # Distinct independent SQLite connection, explicit unsafe SQL, no mocked DB.
    import sqlite3
    db = sqlite3.connect(s.path, timeout=0)
    db.executescript(mutation)
    db.close()
    s.trace.append({'negative_algorithm': number, 'sql_executed': mutation, 'service_busy_response': response(code='index_service_busy')})
    result = o.expected_rejection(w.snapshot(), invariant, locals().get('baseline'))
    return result


def before_intent_commit(s):
    w = s.worker
    observer = s.independent()
    assert w.ok('crash_intent')['barrier'] == 'intent_before_commit'
    assert observer.snapshot()['intent'] == []
    s.restart()
    assert s.worker.snapshot()['intent'] == []
    s.oracle.unchanged(s.worker.snapshot())


def legal_event_routes(s):
    s.request(command='Rebuild')
    s.prepare()
    w = s.worker
    for frame in [event(kind='Sources', sources=[{'id': 's1'}]),
                  event(kind='SourceProvider', provider='validated-provider'),
                  event(kind='SourceState', status='INDEXING', error=None),
                  event(kind='ResolveParentPath', lookup_id='fresh-lookup', parent_platform_file_id='parent'),
                  event(kind='FindEntryByIdentity', lookup_id='second-lookup', platform_file_id='file', parent_platform_file_id='parent', name='kept'),
                  event(kind='EntryStale', entry_id='s1:removed'),
                  event(kind='VolumeEntriesStale')]:
        w.ok('frame', request='r1', frame=frame)
        s.oracle.unchanged(w.snapshot())
    s.stream()
    w.ok('frame', request='r1', frame=response())
    s.oracle.published(w.snapshot(), INVENTORY, provider='validated-provider')


def source_fault_with_ok_terminal(s):
    s.request(command='Rebuild')
    s.prepare()
    s.stream()
    w = s.worker
    w.ok('frame', request='r1', frame=event(kind='SourceState', status='REBUILD_REQUIRED', error='real-journal-gap'))
    assert w.ok('frame', request='r1', frame=response()) == 'staged-fault-preserved'
    snapshot = w.snapshot()
    assert snapshot['health'][0]['status'] == 'REBUILD_REQUIRED' and snapshot['health'][0]['cause'] == 'real-journal-gap'
    # The trusted entry/Managed/AI view remains unchanged; no completion clocks.
    baseline = copy.deepcopy(s.initial)
    baseline['health'][0].update(status='REBUILD_REQUIRED', cause='real-journal-gap', reconciliation=1)
    s.oracle.unchanged(snapshot, baseline=baseline)
    s.oracle.pending(snapshot)


def same_source_competition(s):
    s.request(command='Rebuild')
    s.prepare()
    if s.approach == 'A':
        s.worker.ok('frame', request='r1', frame=event())
    peer = s.independent()
    peer.ok('request', request='duplicate', command='Rebuild')
    if s.approach == 'B':
        peer.reject('offer', request='duplicate')
    elif s.approach == 'C':
        peer.reject('arbitrate', request='duplicate')
    peer.reject('frame', request='duplicate', frame=event('duplicate'))
    peer.reject('frame', request='duplicate', frame=response('duplicate', code='index_service_busy'))
    s.oracle.unchanged(peer.snapshot())
    s.stream()
    s.worker.ok('frame', request='r1', frame=response())
    s.oracle.published(s.worker.snapshot(), INVENTORY)


def b_handshake_loss(s, granted=False):
    s.request(command='Rebuild')
    offer = s.worker.ok('offer', request='r1')
    if granted:
        s.worker.ok('grant', request='r1', token=offer['token'])
        s.worker.reject('grant', request='r1', token=offer['token'])
    s.worker.ok('service_restart', instance='service-2')
    s.worker.reject('grant', request='r1', token=offer['token'], instance='service-1')
    s.worker.reject('frame', request='r1', frame=event())
    s.worker.reject('transfer', request='r1', proof='released_no_effect')
    s.oracle.incomplete(s.worker.snapshot())
    s.oracle.pending(s.worker.snapshot())


def no_inventory_checkpoint(s):
    s.request(command='Rebuild')
    s.prepare()
    s.worker.ok('frame', request='r1', frame=event(kind='Checkpoint', cursor=999))
    assert s.worker.ok('frame', request='r1', frame=response()) == 'no-effect-success-preserved'
    s.oracle.unchanged(s.worker.snapshot(), allow_reconciliation=True)
    assert s.worker.snapshot()['health'][0]['reconciliation'] == 1
    s.oracle.pending(s.worker.snapshot())


def old_intent_cancel(s):
    s.request(command='Rebuild')
    s.request(request='coalesced-old', command='Rebuild')
    s.worker.ok('interrupt', request='r1', action='cancel')
    s.request(request='new-intent', command='Rebuild')
    baseline = s.worker.snapshot()
    assert baseline['intent'][0]['generation'] == 2
    s.worker.reject('interrupt', request='coalesced-old', action='cancel')
    s.worker.reject('frame', request='coalesced-old', frame=event('coalesced-old'))
    s.oracle.pending(s.worker.snapshot())
    assert s.worker.snapshot() == baseline
    s.oracle.unchanged(s.worker.snapshot())


def different_source_fault(s):
    s.request(command='Rebuild')
    s.request(source='s2', request='r2', command='Rebuild')
    s.prepare('r2')
    s.worker.ok('frame', request='r2', frame=response('r2', code='index_permission_required', message='s2-permission-fault'))
    baseline = s.worker.snapshot()
    assert baseline['health'][1]['status'] == 'PERMISSION_REQUIRED'
    s.worker.ok('frame', request='r1', frame=response(code='index_service_busy'))
    s.oracle.unchanged(s.worker.snapshot())
    s.oracle.unchanged(s.worker.snapshot(), source='s2', baseline=baseline)
    errors = lambda snapshot: [r['source'] for r in snapshot['health'] if r['status'] in ('ERROR','PERMISSION_REQUIRED','REBUILD_REQUIRED') or r['reconciliation']]
    assert errors(baseline) == errors(s.worker.snapshot()) == ['s2']
    s.oracle.observations.append('multi-source degraded aggregate contains only actual s2 fault')


def live_owner_wake(s):
    s.request(command='Rebuild')
    s.prepare()
    s.stream()
    before = s.worker.snapshot()
    peer = s.independent()
    assert peer.ok('recover') == []
    assert peer.snapshot() == before, 'wake stole a live process owner'
    s.oracle.unchanged(peer.snapshot())
    s.worker.ok('frame', request='r1', frame=response())
    s.oracle.published(s.worker.snapshot(), INVENTORY)


def b_expiry_release(s):
    s.request(command='Rebuild')
    offered = s.worker.ok('offer', request='r1', expiry=2)
    s.worker.reject('grant', request='r1', token=offered['token'], source='s2')
    s.worker.reject('grant', request='r1', token=offered['token'], revision=99)
    s.worker.ok('expire_offers', now=2)
    assert s.worker.snapshot()['service_slot'][0]['reserved'] is None
    s.worker.reject('grant', request='r1', token=offered['token'], now=2)
    s.worker.reject('frame', request='r1', frame=event())
    s.request(request='after-expiry', command='Rebuild')
    offered = s.worker.ok('offer', request='after-expiry', now=2)
    s.worker.ok('grant', request='after-expiry', token=offered['token'], source='s1', revision=offered['revision'], now=2)
    s.stream('after-expiry')
    s.worker.ok('frame', request='after-expiry', frame=response('after-expiry'))
    s.oracle.published(s.worker.snapshot(), INVENTORY)
