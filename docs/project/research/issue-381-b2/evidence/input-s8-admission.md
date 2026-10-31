# FTS and Cursor — Next-Stage Admission Contract

S8 sets exact inputs and open decisions. It does not run SQLite experiments, query-plan validation or scale benchmarks.

## Exact runtime input

The next authorized product FTS query-plan stage must use the SQLite library linked by the product candidate. Record from that same runtime connection:

```sql
SELECT sqlite_version(), sqlite_source_id();
PRAGMA compile_options;
```

Admission requires a recorded product build/source proving **SQLite 3.51.3**. A developer host's `sqlite3`, the D4 research host's 3.53.1, or an independently linked Python package is not a substitute. Bind the exact app/migration/query SHA, Schema 37 and candidate DDL/trigger hash, DB user_version, `files_fts`/content FTS definitions, relevant data counts, page/WAL sizes, query text, binds, root/scope, cursor version and manifest revisions.

## Decisions before a measurement run

1. **BM25 corpus:** the D4-S2 accepted research contract is BM25 over exactly the effective visible managed corpus unless Owner explicitly approves another definition. Filtering hidden rows out of the final result after `bm25()` does not establish visible-corpus ranking because hidden FTS rows may affect corpus statistics. Owner must confirm the corpus contract; S8 does not change it.
2. **Root-scoped Cursor:** the SQL must itself be constrained to the token's root, with current root/source seals applied on every page. A global root-set change invalidates a global cursor. A root-B cursor can survive unrelated root-A change only if B's actual query scope, B seals, shared source identity and exact visible ranked result remain unchanged. Any shared source/path/physical identity change invalidates B too. Replaying an old FTS snapshot is not a fallback.
3. **Digest cost:** D4-S6's research token hashes the complete ranked result tuple set; its validation work is O(number of matches) plus materialization/sort/serialization. Do not assume it fits production or replace it with a page-only check without Owner-approved proof. The measurement must separate query plan, MATCH/BM25, ranking materialization, digest, cursor validation, total latency, peak memory and SQLite temp/WAL cost.
4. **Scale budgets:** use deterministic 100k, 500k and 1M file populations with root overlap, same-path old/new identities, hidden/unverified rows and realistic name/content lengths. Before any benchmark, Owner must set allowed p95/p99 latency, peak memory, temporary storage and cursor-validation overhead for global and root-only cases. No numeric target is currently approved by this phase; S8 invents none.

## Query-plan-only admission before scale work

After #380/client generation and the production schema/query candidate are approved, first capture `EXPLAIN QUERY PLAN` in product SQLite 3.51.3 for:

- File Library first page, exact/deferred count, FTS MATCH + BM25 order, next-page cursor and root-only query;
- equivalent `content_artifact_fts` active-result query;
- Global Index FTS as an independent control;
- the exact current stale filter and candidate Effective Visibility join over the same fixture.

Capture join order, virtual-table MATCH usage, materialization, full scans, temp B-tree/sort, automatic indexes and where root/source membership constrains the result. Validate result IDs/count/order against a small independent oracle before any scale measurement. Explain the observed broad-BM25 anomaly on 3.51.3. No large benchmark starts until the plan and BM25 semantic decision pass Owner review.

## Later scale evidence (not authorized in S8)

At each 100k/500k/1M scale, compare current anchor/revision cursor to the approved new token and query plan with the same product library/build, deterministic fixture, page sizes 50/200, global and root scopes, rare/medium/broad MATCH, first/next/deep pages, warm/cold cache and identical indexes. Report p50/p95/p99/max and raw records across repeated runs; include latency, process memory, temp storage, DB/WAL growth, reader pin duration, and visibility/cursor race outcome. The exact repetition count and any numeric acceptance remain Owner decisions before execution.

The stage passes only against the Owner-defined semantics and budgets, with no hidden ID leakage, no cursor resurrection, consistent reopen/rebuild, and explainable product SQLite 3.51.3 plans. These are future gates, not results claimed here.
