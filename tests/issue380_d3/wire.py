"""Research projection of the existing IPC v3 serde envelopes.

No wire field is added. GlobalEntryInput's unrelated filesystem metadata is
outside this proof: fixture entries keep only volume_id/name/value, then the
adapter projects them to the oracle's small inventory. NOT a replacement Rust
serde/parser qualification. Events have NO protocol_version or source envelope;
source authority is validated from payload / database entry membership.
"""
from copy import deepcopy


def project(wire, source, entry_source):
    if not isinstance(wire, dict):
        raise ValueError('frame object')
    kind = wire.get('kind')
    if kind == 'response':
        data = wire.get('response')
        if not isinstance(data, dict):
            raise ValueError('response envelope')
        return dict(data, kind='Response')
    if kind != 'event':
        raise ValueError('unexpected frame')
    data = deepcopy(wire.get('event'))
    if not isinstance(data, dict):
        raise ValueError('event object')
    types = {'entries': 'Entries', 'entry_stale': 'EntryStale', 'volume_entries_stale': 'VolumeEntriesStale',
             'checkpoint': 'Checkpoint', 'source_state': 'SourceState', 'source_provider': 'SourceProvider',
             'resolve_parent_path': 'ResolveParentPath', 'find_entry_by_identity': 'FindEntryByIdentity', 'sources': 'Sources'}
    if data.get('event') not in types:
        raise ValueError('unknown event')
    data['type'] = types[data.pop('event')]
    if data['type'] == 'Entries':
        entries = data.pop('entries', None)
        if not isinstance(entries, list) or any(r.get('volume_id') != source for r in entries):
            raise ValueError('entry source')
        data['volume_id'] = source  # LOCAL validated context, never serialized.
        data['rows'] = [(r['name'], r['value']) for r in entries]
    elif data['type'] == 'EntryStale':
        actual_source, name = entry_source(data['entry_id'])
        if actual_source != source:
            raise ValueError('entry membership')
        data['volume_id'], data['entry'] = actual_source, name
    elif data['type'] == 'Sources':
        if any(row['id'] != source for row in data.get('sources', [])):
            raise ValueError('discovery source')
        data['volume_id'] = source
    elif data['type'] == 'Checkpoint':
        data['cursor'] = int(data['journal_cursor'])
    return {'kind': 'Event', 'request_id': wire.get('request_id'), 'event': data}
