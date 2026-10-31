# B2 isolated run result

**16/16 positive cases pass; 6/6 counterexamples have observed expected rejection.** Independent raw-output inspection confirms all case IDs, nonempty negative diffs, exact source hashes, and integrity/FK checks for 27 SQLite backup snapshots.

Source-candidate HEAD `d1e5ab957c6b72428910c3d2e9dbaf57e4344c4d`, tree `ff0c7810219ab4a957cfda758132be4c0958a010`. The final evidence commit adds only research outputs; exact final HEAD/tree and final-head replay/CI are in the Draft PR closeout.

Python `3.12.14 (main, Aug 25 2026, 14:00:49) [Clang 22.1.3 ]`; SQLite `3.53.1`; FTS5 exists. SQLite source ID: `2026-05-05 10:34:17 c88b22011a54b4f6fbd149e9f8e4de77658ce58143a1af0e3785e4e6475127e9`. All compile options and OS details are in `evidence/environment.json`.

## Raw contention samples

One measurement per size, milliseconds. These are disposable Linux SQLite model timings, not product/native qualification or percentiles.

| Rows | Publication lock | Source writer elapsed | Tag/note writer elapsed | WAL bytes |
|---:|---:|---:|---:|---:|
| 0 | 0.450028 | 1.309865 | 3.350244 | 234872 |
| 10000 | 0.422818 | 1.254052 | 0.282589 | 2043552 |
| 100000 | 0.639290 | 1.320421 | 3.335282 | 4218912 |

The unchanged competing-writer threshold is **5000ms**. Each publication traces five fixed statements and both writers commit. All staging latencies and exact ID lists/revisions remain in unmodified `results.jsonl`.

## Source SHA-256

| Source | SHA-256 |
|---|---|
| `model.py` | `33f14139a9d4125f9078b2e32c5d6a91fb976994933053a3c9e0dc1437d3a261` |
| `oracle.py` | `40313c9036c6b7f96f7b3400c3f019d968163bf4713433279f5ae861b83358e2` |
| `run.py` | `ab305ff06f0fec6aa4bebb95ed77326516e9da25673fb24bc868fc10d30f82bf` |

Five SIGKILL checkpoints recovered exactly E0 or E1, retained user data, passed independent visible FTS comparisons and restartable cleanup. No OS power-loss/physical identity or production fix is claimed.

Documentation/governance, diff and production-source equality checks exit zero; actual commands and stdout/stderr are retained in `evidence/commands.json` and adjacent text files. The initial unconfigured documentation invocation failed because `DOCS_DIFF_BASE` was missing; the recorded configured invocation checks all five added input/report Markdown files. Earlier harness runs were development/debugging only and are not the committed qualification run.

Readiness applies **only** to this B2 model. Stop Draft/unmerged for Owner independent review.
