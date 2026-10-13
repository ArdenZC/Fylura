#!/usr/bin/env python3
"""Run only the bounded K1 probes/tests; never dispatch CI or touch user data."""
import pathlib,subprocess,sys
ROOT=pathlib.Path(__file__).resolve().parents[2]
for script in ['build_probe.py','test_isolation.py']:
    result=subprocess.run([sys.executable,str(ROOT/'tests/issue380_db_open_generation'/script)],cwd=ROOT,capture_output=True,text=True)
    out=ROOT/'.tmp-tests/issue380-k1';out.mkdir(parents=True,exist_ok=True)
    (out/(script+'.stdout.txt')).write_text(result.stdout)
    (out/(script+'.stderr.txt')).write_text(result.stderr)
    print(result.stdout,end='');print(result.stderr,end='',file=sys.stderr)
    if result.returncode:sys.exit(result.returncode)
