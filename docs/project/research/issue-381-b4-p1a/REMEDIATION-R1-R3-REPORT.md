# B4-P1A R1–R3 test-harness remediation

## Decision

- `P1A_HARNESS_REMEDIATION_COMPLETE`
- `P1A_BLOCKED_ENVIRONMENT`
- `READY_FOR_OWNER_REREVIEW`

The focused harness now meets all declared R1–R3 expectations. The overall P1A result remains blocked because `P1A-T10 = ENVIRONMENT_UNAVAILABLE`: this Linux executor did not provide hosted Windows and macOS real-file-handle evidence. No Linux result is used as a substitute. This report does not authorize P1B or a product implementation.

## Source and scope

- Repository: `ArdenZC/Zen-Canvas`; existing PR #408 branch `test/issue-381-b4-p1a-f1-lifecycle` remains the only PR being updated.
- Schema 38 base `master`: HEAD `ab3915e3cdcc7c729317498b3b97501862832a91`, Tree `ca22989646575bba9a7224ce10becbb89c01e0ee`.
- Original PR #408 start: HEAD `12dfea7ec49e8ad576db4029a3da2fdadbe41ba1`, Tree `60e5a1a8aa400360ef27ffb11ec0ad4e01cf3daf`.
- Code under the formal remediation run: HEAD `d2b916ac5889072945005036c7594dea2e23dbb2`, Tree `080a2210a11ca001abd2ea98694653705a400f61`.
- The final evidence/report commit, final PR HEAD/Tree, full diff hash and exact-HEAD CI are recorded in the updated PR #408 description.

The only source changes are in `tests/issue381_b4_p1a_f1/{src/db.rs,src/main.rs,oracle.py,run.py}`. Evidence and reports are under this research directory. No production code, Schema/migration, root `Cargo.lock`, CI workflow/gate, validation plan, or `tests/ruleProposalTask07.test.ts` was changed. Schema 38 and the existing `library_generation_authority=('legacy', 0, NULL, 1)` value remain intact. The test-only `BEGIN IMMEDIATE` prototype is not a product-approved design.

## R1 — real pagination and Oracle verification

The preserved pre-fix probe ran against original PR HEAD `12dfea7…`. Calling `query_rows(root=None, limit=Some(1))` produced the actual SQLite binding error `Wrong number of parameters passed to query. Got 2, needed 3`. The probe exited 0 only because that expected error was observed. Its exact source, stdout/stderr and receipt are in [the raw evidence directory](evidence/remediation-r1-r3-v1/raw/).

The fixed query binds every root/limit combination correctly and a separate `query_rows_page` executes SQL `LIMIT` and `OFFSET`. Tests cover all four root/no-root and limit/no-limit combinations, first/next/boundary/empty pages, tied ranks and scope-before-limit. The underfill adversary returns 4 root rows when global `LIMIT 7` is incorrectly applied before the root predicate; the scoped query returns all 7 requested rows. Global effective-visible FTS BM25 statistics and contract weights `6.0/1.5` remain unchanged.

The independent Python Oracle does not use SQLite. It checks all 19 snapshots (12 historical full-result snapshots plus 7 added real pages), including complete ID ordering and exact rank float bits: 19/19 pass. Cursor invalidation is explicitly a test model only; this harness has no product Cursor implementation, so it makes no product Cursor-contract claim.

## R2 — deterministic Pin/GC and lifecycle matrix

The original harness source and output are retained. A targeted run of the archived original test shows the old `P1A-T05` emitted 12/12 identical Pin-first outcomes; its schedule started GC only after pin registration and used a 15 ms sleep. The archived N04 emitted `EXPECTED_REJECTION` from a forged-JSON hash inequality without invoking `verify_seal`. This reproduces the old coverage gap, not a passing proof. The targeted baseline output and receipt are `raw/baseline-r2-r3.stdout.jsonl` and `raw/baseline-r2-r3-reproduction.json`.

The replacement matrix has eight named cases synchronized with explicit channels, with no sleep used to order the races:

| Case | Outcome and state checked |
| --- | --- |
| Pin registration first, GC requests second | Live lease row blocks GC; slot file stays present until lease release, then is removed. |
| GC retirement/reclaim transaction first | Late pin registration is rejected; no lease row is written and the reclaimed file is absent. |
| Registration then retirement before open | The real lease/read-only handle protects the retiring slot. |
| Real read-only transaction held during GC | GC is blocked until handle and lease release; file then disappears. |
| Open failure after registration | Injected missing-slot open fails, lease release changes SQLite state, and GC removes the file. |
| Lease expiry | Expired persisted lease no longer prevents GC. |
| Child process exit and DB reopen | Observed child exit is followed by test-only recovery; persisted lease is cleared and GC succeeds. |
| Slot ID reuse and old lease replay | New generation exists, old generation file is gone, and replay is rejected by generation fencing. |

Each case records actual SQLite transaction/state results and file existence. Pool max remains 8 and SQLite busy timeout remains 5,000 ms. Per-reader `BEGIN IMMEDIATE` is only the isolated prototype under test; these results do not approve it for production.

## R3 — actual manifest verification

The prior N04 check only compared a forged JSON hash to the expected hash. The replacement writes actual invalid/replayed manifests into an isolated temporary directory and calls the complete test-harness validation entrance (`verify_manifest_file`, also reached by `verify_seal`). It validates actual SQLite bytes and length, read-only integrity/count/source ID, slot ID, format, corpus digest, root/source revisions, and expected manifest digest.

All eight variants are rejected: wrong Slot ID, file hash, file length, Root Revision, Source Revision, corpus hash, old manifest format without version bindings, and an old-slot manifest replayed against the new slot. After each rejection the authority tuple, old slot bytes/file, and synthetic user facts remain unchanged. This is validation in the isolated harness, not a production verifier implementation.

## Matrix and historical evidence

The machine-readable matrix reports all 16 prior gates and all 11 focused R1–R3 expectations satisfied; the independent Oracle reports 19/19. `P1A-T10` remains `ENVIRONMENT_UNAVAILABLE`, and process kill/fsync results are not power-loss evidence. See [the updated gate matrix](REMEDIATION-R1-R3-MATRIX.md) and [the exact run receipt](evidence/remediation-r1-r3-v1/run-receipt.json).

Historical evidence is not rewritten or relabeled. Original `evidence/final/MANIFEST.sha256` is byte-for-byte unchanged. Its 86 recorded hashes are also verified by `evidence/previous-pr408-source/original-MANIFEST-rebased.sha256`; that manifest redirects only the four intentionally replaced test source paths to the frozen original-HEAD files under `evidence/previous-pr408-source/tests/`. The original 800 SQLite lock-wait samples, 300 performance samples, 12/12 Oracle result, CI #38511901852 failure, and original `P1A_BLOCKED_ENVIRONMENT` conclusion remain preserved.

| Evidence | Original | Remediation run | Change and reason |
| --- | ---: | ---: | --- |
| Independent Oracle snapshots | 12 | 19 | +7: retained all 12 full results and added seven limited pages. |
| Pin/GC case records | 12 repeated Pin-first attempts | 8 distinct synchronized cases | −4: replaced repeats with both race orders and lifecycle/ABA assertions. |
| SQLite lock-wait samples | 800 | 800 | No change; retained the 8-connection × 100-iteration matrix. |
| Performance samples | 300 | 300 | No change; retained 100 samples at each of 0/10k/100k synthetic sizes. |

The old CI run [#38511901852](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38511901852) failed on the separate Schema 37 assertion in `tests/ruleProposalTask07.test.ts`; Issue #409 owns that repair. That file and all CI gates were left unchanged. The exact CI for the final remediation HEAD is reported in PR #408.

## Reproduction commands and evidence

The complete command vector, exit code, stdout/stderr byte lengths and SHA-256 values, source hashes, environment and precise source HEAD/Tree are in [environment-and-commands.json](evidence/remediation-r1-r3-v1/environment-and-commands.json). Each command in the formal run exited 0:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tests/issue381_b4_p1a_f1/run.py
cargo tree -e features --locked --manifest-path tests/issue381_b4_p1a_f1/Cargo.toml
cargo fmt --check --manifest-path tests/issue381_b4_p1a_f1/Cargo.toml
cargo build --release --locked --manifest-path tests/issue381_b4_p1a_f1/Cargo.toml
/tmp/issue381-b4-p1a-target/release/issue381-b4-p1a-f1 --remediation-only
/tmp/issue381-b4-p1a-target/release/issue381-b4-p1a-f1 --samples 100
```

The runner invoked the independent Oracle on the compressed native output; it exited 0 with 19 checked, 19 passed, 0 failed, exact float bits and complete ordered rows. The separate baseline reproductions and the new focused/full raw output are retained under `evidence/remediation-r1-r3-v1/raw/`. Its `MANIFEST.sha256` covers the raw evidence and receipts.

The PR remains OPEN/Draft for Owner review. No Codex Review, merge, release, issue close, or P1B work was performed.
