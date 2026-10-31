"""Controlled independent SQLite process, JSON pipe barriers, no sleep ordering."""
import json
import os
import sys
from model import Candidate, Rejected


def main():
    candidate = Candidate(sys.argv[1], sys.argv[2])
    print(json.dumps({'barrier': 'process_ready', 'pid': os.getpid()}), flush=True)
    for line in sys.stdin:
        command = json.loads(line)
        try:
            method, args = command['method'], command.get('args', {})
            if method == 'lock':
                candidate.db.execute('BEGIN IMMEDIATE')
                result = {'barrier': 'write_lock_held'}
            elif method == 'unlock':
                candidate.db.execute('ROLLBACK')
                result = True
            elif method == 'crash_intent':
                candidate.db.execute('BEGIN IMMEDIATE')
                candidate.db.execute("INSERT INTO intent VALUES ('s1','rebuild-s1','pending',0,1)")
                print(json.dumps({'ok': True, 'result': {'barrier': 'intent_before_commit'}}), flush=True)
                sys.stdin.readline()
                os._exit(48)
            elif method == 'crash_publication':
                op = candidate.op(args['request'])
                candidate.db.execute('BEGIN IMMEDIATE')
                candidate._fence(op)
                candidate.db.execute('UPDATE global_rows SET stale=1 WHERE source=?', (op['source'],))
                print(json.dumps({'ok': True, 'result': {'barrier': 'after_first_SQL_before_commit'}}), flush=True)
                # Parent has read the old snapshot at the barrier before killing
                # this process. No synthetic rollback callback is invoked.
                sys.stdin.readline()
                os._exit(47)
            elif method == 'exit':
                candidate.close()
                return
            else:
                result = getattr(candidate, method)(**args)
            print(json.dumps({'ok': True, 'result': result}), flush=True)
        except Rejected as error:
            print(json.dumps({'ok': False, 'rejected': str(error)}), flush=True)
        except Exception as error:
            print(json.dumps({'ok': False, 'error': type(error).__name__, 'message': str(error),
                              'sqlite_errorcode': getattr(error, 'sqlite_errorcode', None),
                              'sqlite_errorname': getattr(error, 'sqlite_errorname', None)}), flush=True)


if __name__ == '__main__':
    main()
