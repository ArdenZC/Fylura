"""Independent, read-only reference oracle.

Does not import Candidate, execute its transition code, infer correctness from
its returned labels, or compare one candidate against another. It computes
expected published views directly from the input inventory and user correction
ledger, and checks immutable baseline facts plus monotonic authority rules.
"""
import copy


PUBLISHED = ('health', 'global_rows', 'managed', 'ai_jobs', 'ai_items', 'ai_analysis')


def view(snapshot, source='s1', reconcile=False):
    result = {table: [copy.deepcopy(r) for r in snapshot[table] if r['source'] == source] for table in PUBLISHED}
    if reconcile:
        for row in result['health']:
            row.pop('reconciliation')
    return result


class Oracle:
    def __init__(self, initial):
        self.initial = copy.deepcopy(initial)
        self.observations = []

    def unchanged(self, actual, source='s1', baseline=None, allow_reconciliation=False):
        expected = view(baseline or self.initial, source, allow_reconciliation)
        observed = view(actual, source, allow_reconciliation)
        if observed != expected:
            raise AssertionError('committed source/published data overwritten: ' + source)
        self.observations.append('no committed mutation: ' + source)

    def pending(self, actual, source='s1', state='pending'):
        rows = [r for r in actual['intent'] if r['source'] == source]
        assert len(rows) == 1 and rows[0]['key'] == 'rebuild-' + source and rows[0]['state'] == state, 'durable intent/coalescing'
        self.observations.append('durable intent: ' + state)

    def incomplete(self, actual, source='s1'):
        self.unchanged(actual, source, allow_reconciliation=True)
        health = next(r for r in actual['health'] if r['source'] == source)
        assert health['reconciliation'] == 1, 'partial/unknown outcome must be visible'
        assert not any(r['route'] == 'Direct' for r in actual['operations']), 'unknown replayed Direct'
        self.observations.append('truthful reconciliation; original published snapshot retained')

    def published(self, actual, inventory, checkpoint=90, source='s1', full=True, provider=None):
        original = view(self.initial, source)
        health = copy.deepcopy(original['health'][0])
        health.update(status='READY', cause=None, checkpoint=checkpoint, full_time=100,
                      incremental_time=101, count=len(inventory) if full else len(set(r['entry'] for r in original['global_rows']) | set(e for e, _ in inventory)), reconciliation=0)
        if provider is not None:
            health['provider'] = provider
        if not full:
            health['full_time'] = original['health'][0]['full_time']
        expected_global = {r['entry']: dict(r, stale=1 if full else r['stale']) for r in original['global_rows']}
        for entry, value in inventory:
            expected_global[entry] = {'source': source, 'entry': entry, 'value': value, 'stale': 0}
        expected_managed = {r['entry']: dict(r, enabled=0 if full else r['enabled']) for r in original['managed']}
        for entry, _ in inventory:
            if entry in expected_managed:
                expected_managed[entry]['enabled'] = 1
            else:
                expected_managed[entry] = {'source': source, 'entry': entry, 'enabled': 1, 'correction': ''}
        expected = {'health': [health], 'global_rows': list(expected_global.values()), 'managed': list(expected_managed.values())}
        for table in ('ai_jobs', 'ai_items', 'ai_analysis'):
            rows = {r['entry']: dict(r, state='stale' if full else r['state']) for r in original[table]}
            for entry, _ in inventory:
                rows[entry] = {'source': source, 'entry': entry, 'state': 'pending'}
            expected[table] = list(rows.values())
        normalize = lambda obj: {t: sorted(rows, key=lambda r: r.get('entry', '')) for t, rows in obj.items()}
        assert normalize(view(actual, source)) == normalize(expected), 'atomic publication/derivative/checkpoint ledger mismatch'
        self.observations.append('publication matches independent inventory/correction ledger')

    def revision_advanced(self, before, after, source='s1'):
        get = lambda s: next(r for r in s['control'] if r['source'] == source)
        assert get(after)['revision'] > get(before)['revision'], 'ABA revision did not advance'
        self.observations.append('monotonic ABA fence')

    def expected_rejection(self, actual, invariant, baseline=None):
        try:
            if invariant == 'unknown_no_replay':
                self.incomplete(actual)
            elif invariant == 'coherent_derivatives':
                rows = [r for r in actual['global_rows'] if r['source'] == 's1']
                stale = {r['entry'] for r in rows if r['stale']}
                assert all(not r['enabled'] for r in actual['managed'] if r['source'] == 's1' and r['entry'] in stale), 'global/managed torn publication'
            else:
                self.unchanged(actual, baseline=baseline)
        except AssertionError as error:
            return {'expected_rejection_observed': True, 'violated_invariant': invariant, 'oracle_failure': str(error)}
        raise AssertionError('negative control was NOT rejected: ' + invariant)
