# #381 D4-S1 research bundle

This folder contains the design-only Unified Visibility Authority ADR candidate and reproducible source inventory. It changes no product source, Schema version, IPC contract, CI, or PR state.

## Reproduce the direct stale-reference inventory

From the repository root at any checkout containing this folder:

```sh
python3 docs/project/research/issue-381-d4-s1/source-audit/build_inventory.py \
  3275ac7c72df8ae4139a30f16c7f649971324e97 \
  docs/project/research/issue-381-d4-s1/source-audit
```

The generator uses only Python standard library and `git show`/`git ls-tree` on frozen master. `stale-reference-inventory.csv` records every Rust reference line; `function-inventory.csv` groups those lines under the nearest source function or module/type field. The function grouping is a reproducible text heuristic, not a Rust parser. `ADR-001-unified-visibility-authority.md` adds manual review of indirect callers, SQL triggers/views, frontend consumers, background recovery and bridge behavior.

The literal database-write audit is reproduced with:

```sh
python3 docs/project/research/issue-381-d4-s1/source-audit/build_write_inventory.py \
  3275ac7c72df8ae4139a30f16c7f649971324e97 \
  docs/project/research/issue-381-d4-s1/source-audit
```

It emits every literal relevant SQL mutation line and groups mutations by function. Dynamically assembled SQL and trigger bodies require the manual review recorded in the ADR.

No tests, CI, Hosted Windows/macOS job, benchmark, Codex Review, PR mutation, merge, Issue close, or Release was run for D4-S1.
