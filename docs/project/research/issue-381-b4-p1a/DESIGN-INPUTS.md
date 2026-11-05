# Reviewed design inputs

This test package was scoped after reading the complete [Issue #406](https://github.com/ArdenZC/Zen-Canvas/issues/406), all three design contracts at Draft PR #405, and the cited #399/#400 research evidence.

## Draft PR #405 contracts

The reviewed contract snapshot is PR head `a96b2c96775581338e0cc63f1b3811dcd569b6df`:

- `OWNER-F1-SLOT-LIFECYCLE-DECISION.md`
- `P1A-CODEX-RESEARCH-SCOPE-AND-TEST-GATES.md`
- `EVIDENCE-DEPENDENCIES-AND-RISKS.md`

The test scope is the isolated F1 slot lifecycle. Only the new harness under `tests/issue381_b4_p1a_f1/` and evidence under this research directory are in bounds. Product code, SQLite schema, production data, CI workflow gates, merge, release, and Codex Review are outside this task. A native-platform gate that cannot run must remain `ENVIRONMENT_UNAVAILABLE` and keep the overall outcome blocked.

The harness mirrors the relevant product settings for measurement: pool max size 8, WAL, `synchronous=NORMAL`, 5,000 ms busy timeout, FTS5 trigram, and BM25 column weights 6.0/1.5. It keeps the schema-38 generation-authority sentinel `('legacy', 0, NULL, 1)` intact. `BEGIN IMMEDIATE` per reader is measured as a prototype cost and is not treated as accepted product behavior.

## PR #399 research evidence

The reviewed research snapshot is PR head `93e38861ff39ee0e0bfd4cbd094d999e60a425b0`:

- `README.md`, `REPORT.md`, `RESULT.md`
- `source-parity.json`, `summary.json`, `commands.json`

It records a Linux SQLite 3.51.3 research build and one-off measurements (100k build about 3,865 ms, root delta about 935 ms, pointer work about 0.72 ms, high-hit query about 386 ms). Those measurements have no percentile distribution and do not establish Windows/macOS behavior, native filesystem durability, power-loss safety, or production readiness. The present harness therefore repeats only the specified synthetic sizes, reports nearest-rank percentiles, and labels all observations as host-specific.

## PR #400 ownership and cutover evidence

The reviewed research snapshot is PR head `609a75a3b81eec13e25e4967dec2fd8ffc97a01c`:

- `OWNER-IDENTITY-SCHEMA-CONTRACT.md`
- `CUTOVER-AND-TEST-GATES.md`
- `SOURCE-FK-OWNERSHIP-MATRIX.csv`

The authority pointer remains separate from mutable legacy FTS and user-owned identity facts. Path equality alone does not grant UID identity or transfer user facts. The F1 negative cases exercise BM25/filter-order and stale mutable-FTS failure modes without changing or opening the product database.

## Resulting evidence rule

Process-kill recovery and `fsync` are not described as sudden power-loss proof. The capacity-failure test is injected/model-only. Missing/corrupt slot targets use read-only reconciliation without mutable FTS fallback or destructive repair. T10 remains unavailable until native Windows and macOS real-handle evidence exists.
