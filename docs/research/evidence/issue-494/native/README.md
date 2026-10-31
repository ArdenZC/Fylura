# Corrected Candidate B native evidence

These JSONL files were emitted by the branch-scoped native job in [workflow run
40693012419](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40693012419). Both runners checked
out implementation source SHA `c454ac6283092b67143820777376aa4c3b18b222`, tree
`c7892c22c7803697264b2b5a888d4e3e1ec0e711`.

Each platform passed 23/23 adversarial Candidate B correctness cases and emitted official 100k and
1M Release fixtures. Every fixture contains 13 queries, with 5 warmups and 30 measured calls per
query, plus fixture, compact-view build and write/rebuild records. Query percentiles use the empirical
order statistic `ceil((n - 1) * q)` with zero-based indexing and no interpolation. For n=30, p99 is
the maximum observed call, not a stable tail estimate.

## Windows

- Job: `Issue 494 native A/B (windows-latest)`, successful.
- Total job time: about 62 minutes; the existing timeout remains 180 minutes.
- Correctness test execution: 23 passed in 342.64 s; the full step also includes Release compile.
- 100k scale probe: 188.50 s test execution, plus 2m45s compile in that step.
- 1M scale probe: 2,485.21 s (41m25s) test execution; the Release binary was reused.
- Artifact: [issue494-native-ab-windows-latest-40693012419](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40693012419)
- Artifact ID: `12822276783`
- Archive SHA-256: `0bc24bd490c4fad44c0441792d8cc12661cb749a548475582b59df9dd01f4235`
- 100k JSONL SHA-256: `f760475a1acf676b3ad043a907d3541db6936372298ff03875a50d3f58654a7b`
- 1M JSONL SHA-256: `b0edce7c9276926b657c0a82ab68d246c3601735621760c05a734d16cbd801d4`

## macOS

- Job: `Issue 494 native A/B (macos-latest)`, successful.
- Total job time: about 18 minutes.
- Correctness: 23 passed; test execution 37.42 s after Release compile.
- 100k scale probe: 48.05 s test execution; 1M scale probe: 404.54 s (6m45s).
- Artifact: [issue494-native-ab-macos-latest-40693012419](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40693012419)
- Artifact ID: `12818518612`
- Archive SHA-256: `dac71ec601765366cac19364cd573ca04ae489a5fa3730d7e4bb0855f6ff0ece`
- 100k JSONL SHA-256: `079217cb74b4b773241eec5e8f07a0294559a84ada9301e1cc088b5eba9f34f2`
- 1M JSONL SHA-256: `4d5ee9128707683a6d28c268da52d97742e64815479995b606a66583fdd60bda`

## Files

- `issue494-windows-latest-100k.jsonl`
- `issue494-windows-latest-1m.jsonl`
- `issue494-macos-latest-100k.jsonl`
- `issue494-macos-latest-1m.jsonl`

From the repository root, regenerate the performance, resource and write/rebuild tables with:

```bash
python3 docs/research/evidence/issue-494/summarize.py
```

The previous Windows run `40666549498` hit its existing 180-minute cap after 137.7 minutes of
Debug correctness work and the 100k fixture. The completed rerun above reduced the repeated fixture
work and used one Release profile; no timeout was increased.
