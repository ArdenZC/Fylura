"""Research Service producer: computation/Pipe-like JSON output ONLY.

No sqlite import, database pathname, repository/sink or writable DB handle.
This is not the Windows Service binary; it models the source-grounded
Service-host -> PipeSink -> Desktop DatabaseIndexSink direction on master.
"""
import json
import os
import sys

request = json.loads(sys.stdin.read())
assert request['protocol_version'] == 3 and request['request_id']
print(json.dumps({'pid': os.getpid(), 'database_handle': None,
                  'computed_before_client_owner': True,
                  'frame': {'kind': 'event', 'request_id': request['request_id'],
                            'event': {'event': 'entries', 'entries': [{'volume_id': request['source'], 'name': 'kept', 'value': 'computed-service-value'}]}}}))
