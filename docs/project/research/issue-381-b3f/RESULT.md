# #381-B3-F qualification result

**F1 FEASIBLE TEST-ONLY; F2 EXACT RANK PROVEN / MUTABLE-INDEX GENERATION BLOCKED; F0 EXPECTED REJECTION OBSERVED.**

Source candidate `35bd609ce1d0002d86596a31e8c5904d56e7d61a` / tree `88fa267c0521ea9a2a4505520516a24f63d646ae`. Final evidence commit changes no probe source; final-head replay and natural CI are bound in the Draft PR closeout.

760 full independent comparisons pass, including exact float bits and complete order. Native reference indexes exist only in small fixtures. All compressed native JSONL contains full rows and schedules unchanged. Six real SIGKILL barriers recover old/new only; immutable-read/clock replay/cursor/GC guards and retained source/user data pass.

Native bundled SQLite **3.51.3**, source ID `2026-03-13 10:38:09 737ae4a34738ffa0c3ff7f9bb18df914dd1cad163f28fd6b6e114a344fe6d618`, FTS5/trigram present. Rust 1.97.1; all resolved registry packages match product lock checksums. This is Codex Linux source/build-feature parity, not full application or native Windows/macOS qualification.

## Single-run costs

Preparation milliseconds include independently reported shared source membership/statistics work. F1 shadow preparation/build/copy is separate from publication. No distributions or p95 claims.

| Synthetic docs | Phase | Effective docs | F1 shadow ms | F2 statistics ms | Shared ledger ms | Publication lock ms | Source writer ms | Tag writer ms |
|---:|---|---:|---:|---:|---:|---:|---:|---:|
| 0 | E0 | 0 | 7.627043 | 0.000071 | 0.000240 | 0.222260 | — | — |
| 0 | E1 root delta | 0 | 5.200279 | 0.000070 | 0.000121 | 0.436919 | 3.860191 | 1.901123 |
| 10000 | E0 | 8000 | 354.213433 | 13.352351 | 45.292268 | 0.295619 | — | — |
| 10000 | E1 root delta | 7990 | 99.012757 | 22.018387 | 53.814589 | 0.556436 | 2.460042 | 3.8275330000000003 |
| 100000 | E0 | 80000 | 3865.344899 | 132.451960 | 460.948391 | 0.918483 | — | — |
| 100000 | E1 root delta | 79990 | 934.774767 | 238.871124 | 653.253107 | 0.720920 | 2.302537 | 3.772788 |

All writer elapsed/lock records satisfy the unchanged **5000ms** budget. Initial physical index fixture creation, every bounded preparation/GC batch, query/plan, live DB/WAL/index-btree bytes and actual pin lifetime are retained in raw output. The 0-file writers operate on persisted empty-corpus user state; no file/tag ownership is invented.

| Synthetic docs | Phase | Query / scope | F0 ms | F1 ms | F2 ms | Exact rows |
|---:|---|---|---:|---:|---:|---:|
| 0 | E0 | common / global | 0.013230 | 0.048172 | 0.023895 | 0 |
| 0 | E0 | common raretoken / global | 0.012699 | 0.016624 | 0.015213 | 0 |
| 0 | E1 root delta | common / global | 0.013080 | 0.053530 | 0.024726 | 0 |
| 0 | E1 root delta | common raretoken / global | 0.014011 | 0.020070 | 0.055633 | 0 |
| 10000 | E0 | common / global | 32.438025 | 28.253332 | 30.641759 | 8000 |
| 10000 | E0 | common raretoken / global | 16.324143 | 8.096610 | 12.635769 | 9 |
| 10000 | E1 root delta | common / global | 32.432447 | 29.572211 | 30.966450 | 7990 |
| 10000 | E1 root delta | common raretoken / global | 15.813817 | 8.481811 | 12.504533 | 8 |
| 100000 | E0 | common / global | 562.857267 | 386.337349 | 556.798783 | 80000 |
| 100000 | E0 | common raretoken / global | 162.595560 | 75.555516 | 123.042456 | 81 |
| 100000 | E1 root delta | common / global | 615.624299 | 431.155381 | 625.041477 | 79990 |
| 100000 | E1 root delta | common raretoken / global | 195.208746 | 86.539979 | 150.386228 | 80 |

F1 root churn performs a whole-index-file online backup plus changed-document FTS operations and root projection work; its copy/delta costs are explicit. F2 never rebuilds an effective index for ranking, but scans native phrase candidates for effective df and rejects fresh requests while physical terms diverge from the published proof. The visible rename negative shows actual missing E0 candidates if that guard is ignored.

Review [REPORT.md](REPORT.md) for the comparison and required source integration. No production files, schema version, CI route or defaults changed. Stop Draft/unmerged for Owner independent review.
