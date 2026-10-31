"""Run only new K2-T0 cases; immutable K1 probes reused, prior corpus not run."""
import pathlib,subprocess,sys
ROOT=pathlib.Path(__file__).resolve().parents[2];OUT=ROOT/'.tmp-tests/issue380-k2-t0';OUT.mkdir(parents=True,exist_ok=True)
for name in ['build_reused_probe.py','test_feasibility.py']:
    r=subprocess.run([sys.executable,str(ROOT/'tests/issue380_k2_activation'/name)],cwd=ROOT,capture_output=True,text=True)
    (OUT/(name+'.stdout.txt')).write_text(r.stdout);(OUT/(name+'.stderr.txt')).write_text(r.stderr)
    print(r.stdout,end='');print(r.stderr,end='',file=sys.stderr)
    if r.returncode:sys.exit(r.returncode)
