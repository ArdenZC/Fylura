"""Executable research candidates. No production imports or protocol changes.

A's only incoming wire objects are existing-v3 shaped Event / Response. Local
context and SQL tokens are not fictional fields added to IPC v3. B alone uses
research Offer/Grant. C attempts one persistent writer arbiter; qualification
also tests a connection outside that arbiter, rather than assuming coverage.
"""
import contextlib
import json
import os
import sqlite3
from wire import project
from pathlib import Path


class Rejected(Exception):
    pass


class Candidate:
    def __init__(self, path, approach):
        self.path, self.approach = str(path), approach
        self.db = sqlite3.connect(self.path, timeout=0, isolation_level=None)
        self.db.row_factory = sqlite3.Row
        self.db.execute('PRAGMA foreign_keys=ON')

    @contextlib.contextmanager
    def transaction(self):
        self.db.execute('BEGIN IMMEDIATE')
        try:
            yield
            self.db.execute('COMMIT')
        except BaseException:
            self.db.execute('ROLLBACK')
            raise

    def initialize(self):
        self.db.executescript(Path(__file__).with_name('fixture.sql').read_text())
        with self.transaction():
            self.db.execute("INSERT INTO service_slot VALUES (1,'service-1',NULL)")
            self.db.execute("INSERT INTO lookup_registry VALUES ('old-lookup','old-request')")
            for source in ('s1', 's2'):
                self.db.execute('INSERT INTO control VALUES (?,0,NULL,NULL)', (source,))
                self.db.execute('INSERT INTO health(source,status,cause,checkpoint,full_time,incremental_time,count) VALUES (?,?,?,?,?,?,?)',
                                (source, 'READY', None, 42, 7, 8, 2))
                for entry in ('kept', 'removed'):
                    self.db.execute('INSERT INTO global_rows VALUES (?,?,?,0)', (source, entry, 'old-' + entry))
                    self.db.execute('INSERT INTO managed VALUES (?,?,1,?)', (source, entry, 'user-correction'))
                    for table in ('ai_jobs', 'ai_items', 'ai_analysis'):
                        self.db.execute(f'INSERT INTO {table} VALUES (?,?,?)', (source, entry, 'pending'))
        return self.snapshot()

    def snapshot(self):
        tables = ('control', 'health', 'intent', 'operations', 'offers', 'stage',
                  'global_rows', 'managed', 'ai_jobs', 'ai_items', 'ai_analysis', 'service_slot', 'lookup_registry')
        return {t: [dict(r) for r in self.db.execute(f'SELECT * FROM {t} ORDER BY rowid')] for t in tables}

    def request(self, source='s1', command='Start', request='r1', instance='service-1'):
        with self.transaction():
            c = self.db.execute('SELECT * FROM control WHERE source=?', (source,)).fetchone()
            if c is None:
                raise Rejected('source identity')
            if command == 'Rebuild':
                self.db.execute("INSERT INTO intent VALUES (?,?,'pending',0,1) ON CONFLICT(source) DO UPDATE SET generation=generation+CASE WHEN state IN ('completed','cancelled') THEN 1 ELSE 0 END,state=CASE WHEN state='completed' OR state='cancelled' THEN 'pending' ELSE state END", (source, 'rebuild-' + source))
            intent = self.db.execute('SELECT generation FROM intent WHERE source=?', (source,)).fetchone()
            generation = intent['generation'] if command == 'Rebuild' else None
            self.db.execute('INSERT INTO operations VALUES (?,?,?,?,?,?,0,?,?,?,?)',
                            (request, source, c['revision'], request + '-owner', 'Service', 'sent', instance, command, generation, os.getpid()))
        return dict(self.db.execute('SELECT * FROM operations WHERE request=?', (request,)).fetchone())

    def op(self, request):
        row = self.db.execute('SELECT * FROM operations WHERE request=?', (request,)).fetchone()
        if row is None:
            raise Rejected('request identity')
        return dict(row)

    def _intent_fence(self, op):
        if op['intent_generation'] is not None:
            current = self.db.execute('SELECT generation,state FROM intent WHERE source=?', (op['source'],)).fetchone()
            if current is None or current['generation'] != op['intent_generation'] or current['state'] in ('completed','cancelled'):
                raise Rejected('intent generation fence')

    def _claim(self, op):
        self._intent_fence(op)
        changed = self.db.execute('UPDATE control SET owner=?,route=? WHERE source=? AND revision=? AND owner IS NULL',
                                  (op['token'], op['route'], op['source'], op['revision'])).rowcount
        if changed != 1:
            raise Rejected('owner/revision CAS')
        self.db.execute("UPDATE operations SET state='owned' WHERE request=?", (op['request'],))
        self.db.execute("UPDATE intent SET state='claimed',attempts=attempts+1 WHERE source=? AND generation=? AND state='pending'", (op['source'], op['intent_generation']))

    def _fence(self, op):
        row = self.db.execute('SELECT * FROM control WHERE source=?', (op['source'],)).fetchone()
        if row is None or row['owner'] != op['token'] or row['revision'] != op['revision'] or row['route'] != op['route']:
            raise Rejected('stale owner')
        return row

    def offer(self, request, now=0, expiry=10):
        if self.approach != 'B':
            raise Rejected('not a v3 frame')
        self.expire_offers(now)
        op = self.op(request)
        if op['state'] != 'sent':
            raise Rejected('offer order')
        # Research Service reservation: it owns no production database handle.
        with self.transaction():
            reserved = self.db.execute('UPDATE service_slot SET reserved=? WHERE singleton=1 AND reserved IS NULL AND instance=?', (request, op['instance'])).rowcount
            if reserved != 1:
                raise Rejected('Service reservation busy/instance')
            self.db.execute('INSERT INTO offers VALUES (?,?,?,?,?,?,0)',
                            (request, op['instance'], op['token'], op['source'], op['revision'], now + expiry))
            self.db.execute("UPDATE operations SET state='offered' WHERE request=?", (request,))
        return dict(self.db.execute('SELECT * FROM offers WHERE request=?', (request,)).fetchone())

    def expire_offers(self, now):
        if self.approach != 'B':
            raise Rejected('not offer protocol')
        with self.transaction():
            for offer in self.db.execute('SELECT * FROM offers WHERE expires<=? AND granted=0', (now,)).fetchall():
                self.db.execute("UPDATE operations SET state='expired' WHERE request=? AND state='offered'", (offer['request'],))
                self.db.execute('UPDATE service_slot SET reserved=NULL WHERE reserved=?', (offer['request'],))
        return True

    def grant(self, request, token, instance='service-1', now=0, source=None, revision=None):
        with self.transaction():
            op = self.op(request)
            offer = self.db.execute('SELECT * FROM offers WHERE request=?', (request,)).fetchone()
            slot = self.db.execute('SELECT * FROM service_slot WHERE singleton=1').fetchone()
            if slot['reserved'] != request or slot['instance'] != instance:
                raise Rejected('Service reservation identity')
            if not offer or op['state'] != 'offered' or offer['granted'] or offer['expires'] <= now:
                raise Rejected('offer order/expiry')
            if (offer['token'], offer['instance'], offer['source'], offer['revision']) != (token, instance, op['source'], op['revision']):
                raise Rejected('offer identity')
            if source is not None and source != offer['source']:
                raise Rejected('grant source identity')
            if revision is not None and revision != offer['revision']:
                raise Rejected('grant revision identity')
            self._claim(op)
            self.db.execute('UPDATE offers SET granted=1 WHERE request=?', (request,))
        return True

    def arbitrate(self, request):
        if self.approach != 'C':
            raise Rejected('not arbiter route')
        with self.transaction():
            self._claim(self.op(request))
        return True

    def _identity(self, op, frame):
        self._intent_fence(op)
        if frame.get('request_id') != op['request']:
            raise Rejected('wire request_id')
        kind = frame.get('kind')
        if kind == 'Response':
            if type(frame.get('protocol_version')) is not int or frame.get('protocol_version') != 3 or type(frame.get('ok')) is not bool:
                raise Rejected('wire protocol/response')
            if frame['ok'] and frame.get('error_code') is not None:
                raise Rejected('inconsistent success error_code')
        elif kind == 'Event':
            event = frame.get('event', {})
            # v3 has no event protocol field; validate negotiated response version
            # and active connection/request binding, and every payload source.
            if event.get('volume_id') != op['source']:
                raise Rejected('event source')
            if event.get('type') not in ('Entries', 'EntryStale', 'VolumeEntriesStale', 'Checkpoint', 'FindEntryByIdentity', 'ResolveParentPath', 'SourceState', 'SourceProvider', 'Sources'):
                raise Rejected('event type')
            if event['type'] in ('FindEntryByIdentity', 'ResolveParentPath'):
                lookup_id = event.get('lookup_id')
                if not isinstance(lookup_id, str) or not lookup_id:
                    raise Rejected('lookup identity')
                old = self.db.execute('SELECT request FROM lookup_registry WHERE lookup=?', (lookup_id,)).fetchone()
                if old and old['request'] != op['request']:
                    raise Rejected('lookup replay across request')
                self.db.execute('INSERT OR IGNORE INTO lookup_registry VALUES (?,?)', (lookup_id, op['request']))
        else:
            raise Rejected('unexpected frame')
        authority = self.db.execute('SELECT * FROM control WHERE source=?', (op['source'],)).fetchone()
        if authority['revision'] != op['revision'] or authority['owner'] not in (None, op['token']):
            raise Rejected('late frame authority')
        if op['state'] not in ('sent', 'owned', 'offered'):
            raise Rejected('operation lifecycle')

    def frame(self, request, frame):
        with self.transaction():
            op = self.op(request)
            def entry_source(entry_id):
                row = self.db.execute("SELECT source,entry FROM global_rows WHERE source || ':' || entry=?", (entry_id,)).fetchone()
                if row is None:
                    raise ValueError('entry identity')
                return row['source'], row['entry']
            try:
                frame = project(frame, op['source'], entry_source)
            except (ValueError, KeyError, TypeError) as error:
                raise Rejected(str(error)) from error
            self._identity(op, frame)
            if frame['kind'] == 'Response' and not frame['ok']:
                code = frame.get('error_code')
                if code == 'index_service_busy':
                    if op['effects']:
                        raise Rejected('busy after actionable event')
                    if op['state'] == 'owned':
                        self._fence(op)
                        self._release(op)
                    self.db.execute('UPDATE service_slot SET reserved=NULL WHERE reserved=?', (request,))
                    self.db.execute("UPDATE operations SET state='busy' WHERE request=?", (request,))
                    self.db.execute("UPDATE intent SET state='pending' WHERE source=? AND generation=? AND state='claimed'", (op['source'], op['intent_generation']))
                    return 'ServiceBusy'
                classes = {'index_paused': 'Paused', 'index_rebuild_required': 'RebuildRequired',
                           'index_permission_required': 'PermissionRequired', 'index_failed': 'Provider',
                           'research_windows_io': 'WindowsIo'}
                # WindowsIo is Direct/local research input, NOT a new IPC code.
                if code == 'research_windows_io' and op['route'] != 'Direct':
                    raise Rejected('unknown code')
                if code not in classes:
                    raise Rejected('unknown/missing code')
                self._ensure_owned(op)
                status = {'Paused': 'PAUSED', 'RebuildRequired': 'REBUILD_REQUIRED', 'PermissionRequired': 'PERMISSION_REQUIRED'}.get(classes[code], 'ERROR')
                self.db.execute('UPDATE health SET status=?,cause=?,reconciliation=? WHERE source=?',
                                (status, frame.get('message'), int(bool(op['effects'])), op['source']))
                self._release(op)
                self.db.execute("UPDATE operations SET state='failed' WHERE request=?", (request,))
                self.db.execute("UPDATE intent SET state='pending' WHERE source=? AND generation=? AND state='claimed'", (op['source'], op['intent_generation']))
                return classes[code]
            self._ensure_owned(op)
            if frame['kind'] == 'Event':
                event = frame['event']
                if event['type'] in ('FindEntryByIdentity', 'ResolveParentPath'):
                    return 'lookup-read-only'
                if event['type'] == 'Entries':
                    self.db.execute('INSERT OR IGNORE INTO stage VALUES (?,?,?)', (request, '@inventory', '1'))
                    for entry, value in event['rows']:
                        self.db.execute('INSERT INTO stage VALUES (?,?,?) ON CONFLICT(request,entry) DO UPDATE SET value=excluded.value', (request, entry, value))
                if event['type'] == 'Sources':
                    return 'validated-read-only-discovery'
                if event['type'] == 'EntryStale':
                    self.db.execute('INSERT OR IGNORE INTO stage VALUES (?,?,?)', (request, '@stale:' + event['entry'], '1'))
                if event['type'] in ('SourceProvider', 'SourceState'):
                    self.db.execute('INSERT OR REPLACE INTO stage VALUES (?,?,?)', (request, '@' + event['type'], json.dumps(event)))
                if event['type'] == 'VolumeEntriesStale':
                    self.db.execute('INSERT OR IGNORE INTO stage VALUES (?,?,?)', (request, '@whole_volume', '1'))
                if event['type'] == 'Checkpoint':
                    # Staging checkpoint is not committed health.
                    self.db.execute('INSERT INTO stage VALUES (?,?,?) ON CONFLICT(request,entry) DO UPDATE SET value=excluded.value', (request, '@checkpoint', str(event['cursor'])))
                self.db.execute('UPDATE operations SET effects=effects+1 WHERE request=?', (request,))
                return 'staged'
            # Empty successful terminal establishes admission but conveys no
            # complete inventory evidence. Never turn it into a false READY.
            source_state = self.db.execute("SELECT value FROM stage WHERE request=? AND entry='@SourceState'", (request,)).fetchone()
            if source_state:
                state = json.loads(source_state['value'])
                if state['status'] in ('ERROR','PERMISSION_REQUIRED','REBUILD_REQUIRED','PAUSED','UNAVAILABLE'):
                    self.db.execute('UPDATE health SET status=?,cause=?,reconciliation=1 WHERE source=?', (state['status'], state.get('error'), op['source']))
                    self._release(op)
                    self.db.execute("UPDATE operations SET state='failed' WHERE request=?", (request,))
                    self.db.execute("UPDATE intent SET state='pending' WHERE source=? AND generation=? AND state='claimed'", (op['source'], op['intent_generation']))
                    return 'staged-fault-preserved'
            inventory = self.db.execute("SELECT 1 FROM stage WHERE request=? AND entry='@inventory'", (request,)).fetchone()
            if not op['effects'] or not inventory:
                if op['effects']:
                    self.db.execute('UPDATE health SET reconciliation=1 WHERE source=?', (op['source'],))
                self._release(op)
                self.db.execute("UPDATE operations SET state='no_effect_success' WHERE request=?", (request,))
                self.db.execute("UPDATE intent SET state='pending' WHERE source=? AND generation=? AND state='claimed'", (op['source'], op['intent_generation']))
                return 'no-effect-success-preserved'
            self._publish(op)
            self._release(op)
            self.db.execute("UPDATE operations SET state='completed' WHERE request=?", (request,))
            self.db.execute("UPDATE intent SET state='completed' WHERE source=? AND generation=? AND state='claimed'", (op['source'], op['intent_generation']))
            return 'published'

    def _ensure_owned(self, op):
        if op['state'] == 'sent' and self.approach == 'A':
            self._claim(op)
        elif op['state'] != 'owned':
            raise Rejected('no DB grant/arbiter')
        self._fence(op)

    def _release(self, op):
        self._fence(op)
        self.db.execute('UPDATE control SET owner=NULL,route=NULL,revision=revision+1 WHERE source=?', (op['source'],))
        self.db.execute('UPDATE service_slot SET reserved=NULL WHERE reserved=?', (op['request'],))

    def _publish(self, op):
        self._fence(op)
        source = op['source']
        # All derivative invalidation, new rows, counters and health within ONE
        # SQLite transaction. This is a proposed publication boundary, not an
        # assertion that production's statement sequence already is atomic.
        full = op['command'] != 'Resume' or bool(self.db.execute("SELECT 1 FROM stage WHERE request=? AND entry='@whole_volume'", (op['request'],)).fetchone())
        if full:
            self.db.execute('UPDATE global_rows SET stale=1 WHERE source=?', (source,))
            self.db.execute('UPDATE managed SET enabled=0 WHERE source=?', (source,))
            for table in ('ai_jobs', 'ai_items', 'ai_analysis'):
                self.db.execute(f"UPDATE {table} SET state='stale' WHERE source=?", (source,))
        for row in self.db.execute("SELECT entry,value FROM stage WHERE request=? AND entry NOT LIKE '@%'", (op['request'],)).fetchall():
            self.db.execute('INSERT INTO global_rows VALUES (?,?,?,0) ON CONFLICT(source,entry) DO UPDATE SET value=excluded.value,stale=0', (source, row['entry'], row['value']))
            self.db.execute("INSERT INTO managed VALUES (?,?,1,'') ON CONFLICT(source,entry) DO UPDATE SET enabled=1", (source, row['entry']))
            for table in ('ai_jobs', 'ai_items', 'ai_analysis'):
                self.db.execute(f"INSERT INTO {table} VALUES (?,?,'pending') ON CONFLICT(source,entry) DO UPDATE SET state='pending'", (source, row['entry']))
        for row in self.db.execute("SELECT entry FROM stage WHERE request=? AND entry LIKE '@stale:%'", (op['request'],)).fetchall():
            name = row['entry'][len('@stale:'):]
            self.db.execute('UPDATE global_rows SET stale=1 WHERE source=? AND entry=?', (source, name))
            self.db.execute('UPDATE managed SET enabled=0 WHERE source=? AND entry=?', (source, name))
            for table in ('ai_jobs', 'ai_items', 'ai_analysis'):
                self.db.execute(f"UPDATE {table} SET state='stale' WHERE source=? AND entry=?", (source, name))
        provider = self.db.execute("SELECT value FROM stage WHERE request=? AND entry='@SourceProvider'", (op['request'],)).fetchone()
        if provider:
            self.db.execute('UPDATE health SET provider=? WHERE source=?', (json.loads(provider['value'])['provider'], source))
        checkpoint = self.db.execute("SELECT value FROM stage WHERE request=? AND entry='@checkpoint'", (op['request'],)).fetchone()
        count = self.db.execute('SELECT count(*) FROM global_rows WHERE source=? AND stale=0', (source,)).fetchone()[0]
        self.db.execute("UPDATE health SET status='READY',cause=NULL,checkpoint=?,full_time=CASE WHEN ? THEN 100 ELSE full_time END,incremental_time=101,count=?,reconciliation=0 WHERE source=?",
                        (int(checkpoint[0]) if checkpoint else 42, int(full), count, source))
        self.db.execute('DELETE FROM stage WHERE request=?', (op['request'],))

    def interrupt(self, request, action='cancel', no_effect_proof=False):
        with self.transaction():
            op = self.op(request)
            self._intent_fence(op)
            if op['state'] not in ('sent','offered','owned'):
                raise Rejected('operation lifecycle')
            current = self.db.execute('SELECT * FROM control WHERE source=?', (op['source'],)).fetchone()
            if current['revision'] != op['revision'] or current['owner'] not in (None, op['token']):
                raise Rejected('interrupt stale owner')
            if op['state'] == 'owned':
                self._fence(op)
                self._release(op)
            # Unknown AFTER send is not absence-of-effects even if no local rows.
            uncertain = action == 'transport' and not no_effect_proof
            if op['effects'] or uncertain:
                self.db.execute('UPDATE health SET reconciliation=1 WHERE source=?', (op['source'],))
            state = 'unknown' if uncertain else action
            self.db.execute('UPDATE service_slot SET reserved=NULL WHERE reserved=?', (request,))
            self.db.execute('UPDATE operations SET state=? WHERE request=?', (state, request))
            self.db.execute("UPDATE intent SET state=? WHERE source=? AND generation=? AND state IN ('pending','claimed')", ('cancelled' if action == 'cancel' else 'pending', op['source'], op['intent_generation']))
        return state

    def recover(self):
        # A newly launched process discovers pending work only from SQLite.
        with self.transaction():
            for row in self.db.execute("SELECT * FROM operations WHERE state IN ('sent','offered','owned')").fetchall():
                op = dict(row)
                # This harness only stores self-created controlled child PIDs.
                # A wake/reopen is NOT permission to recover a live owner. PID
                # reuse conservatively blocks recovery; no wall clock lease.
                try:
                    os.kill(op['actor_pid'], 0)
                except ProcessLookupError:
                    pass
                except PermissionError:
                    continue
                else:
                    continue
                try:
                    self._intent_fence(op)
                except Rejected:
                    continue
                current = self.db.execute('SELECT * FROM control WHERE source=?', (op['source'],)).fetchone()
                if current['revision'] != op['revision'] or current['owner'] not in (None, op['token']):
                    continue
                if op['state'] == 'owned':
                    try:
                        self._fence(op)
                    except Rejected:
                        continue
                    self._release(op)
                if op['effects']:
                    self.db.execute('UPDATE health SET reconciliation=1 WHERE source=?', (op['source'],))
                self.db.execute('UPDATE service_slot SET reserved=NULL WHERE reserved=?', (op['request'],))
                self.db.execute("UPDATE operations SET state='restart_fenced' WHERE request=?", (op['request'],))
                self.db.execute("UPDATE intent SET state='pending' WHERE source=? AND generation=? AND state='claimed'", (op['source'], op['intent_generation']))
        return [dict(r) for r in self.db.execute("SELECT * FROM intent WHERE state='pending'")]

    def transfer(self, request, proof):
        # An externally proven NotSent or terminal, fully released no-effect
        # reservation is necessary. Transport loss alone is never proof.
        with self.transaction():
            op = self.op(request)
            self._intent_fence(op)
            if op['state'] not in ('sent','offered','owned'):
                raise Rejected('operation lifecycle')
            current = self.db.execute('SELECT * FROM control WHERE source=?', (op['source'],)).fetchone()
            if current['revision'] != op['revision'] or current['owner'] not in (None, op['token']):
                raise Rejected('transfer stale authority')
            if proof not in ('NotSent', 'released_no_effect') or op['effects'] or op['state'] == 'unknown':
                raise Rejected('Direct replay forbidden')
            if op['state'] == 'owned':
                self._fence(op)
                self._release(op)
            revision = self.db.execute('SELECT revision FROM control WHERE source=?', (op['source'],)).fetchone()[0]
            self.db.execute("UPDATE operations SET route='Direct',state='sent',revision=?,token=? WHERE request=?", (revision, request + '-direct', request))
            self._claim(self.op(request))
        return True

    def service_restart(self, instance='service-2'):
        slot = self.db.execute('SELECT * FROM service_slot WHERE singleton=1').fetchone()
        if slot['reserved']:
            self.interrupt(slot['reserved'], action='transport')
        self.db.execute('UPDATE service_slot SET instance=?,reserved=NULL WHERE singleton=1', (instance,))
        return True

    def external_owner(self, source='s1', status='READY', cause=None, metadata='new-owner'):
        # Independently authorized discovery/settings/new operation writer.
        with self.transaction():
            self.db.execute('UPDATE control SET revision=revision+1,owner=NULL,route=NULL WHERE source=?', (source,))
            self.db.execute('UPDATE health SET status=?,cause=?,metadata=? WHERE source=?', (status, cause, metadata, source))
        return True

    def raw_write(self, status='ERROR'):
        # Actual independent SQLite writer with NO cooperative arbiter guard.
        self.db.execute('UPDATE health SET status=?,cause=? WHERE source=?', (status, 'bypassed-arbiter', 's1'))
        return self.snapshot()

    def close(self):
        self.db.close()
