# K2-G1 disposable native API research

Run only in an isolated Windows/macOS checkout:

```sh
python3 tests/issue380_k2_g1/run.py --output .tmp-tests/issue380-k2-g1/native
```

No third-party packages or product installation. Controlled Python actors are
TEST_MODEL, not shipped or reconstructed Zen Canvas. Windows ctypes calls are
real CreateFileW/handle identity; macOS uses real flock/fcntl/fstat. Every request,
response, confirmed SQLite commit, source hash and file identity goes to results.json.
The runner cleans only its own unique fixture and stops its own child processes.
Use `--allow-linux-smoke` solely for execution plumbing; no platform qualification.
Execution exit 0 means experiments completed, including negative outcomes; inspect
candidate summaries and raw exchanges for backup failure, writes and missing locks.
Production remains NO-GO even if a bounded held-window candidate succeeds.
