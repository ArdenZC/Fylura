# D4-S8 Test Contracts and Dependency Gates

This file names the future formal contracts assigned by the ledgers. S8 does not run them; D4 production remains NO-GO. Test IDs in the CSVs resolve here.

| ID | Contract | Required evidence and preservation assertions | Gate / owner |
|---|---|---|---|
| `TC-STATIC-01` | Inventory/fixture classification | Rebuild from frozen inputs; IDs and counts stable; test-only modules remain excluded from production groups; all scanned raw records have a disposition. | #381 research owner |
| `TC-VIS-01` | Effective Visibility across active File Library | Against one pointer snapshot, compare complete oracle membership, pages, exact counts, detail, filters, selections, previews and root scope. No hidden, `legacy_unverified`, failed-root or replaced source leaks. | #381 File Library; after physical/manifest design acceptance |
| `TC-IPC-02` | Command and renderer parity / legacy bypass | Exercise all registered commands, API facade, DTO and browser mock. Legacy path-ID commands and old Service clients cannot read/write a manifest-era DB. Active detail/reveal/delete/restore require current source and manifest token; renderer checks alone cannot authorize. | #381 IPC + #380 generation gate |
| `TC-IDENT-03` | Source/path/physical identity | Same-path replacement creates a distinct source with no tags/history/fingerprint/content/AI inheritance; verified rename preserves one source; hard-link aliases and overlapping roots stay distinct/explicit; actions recheck physical ID, path entry, volume and directory boundary. | #381 identity design plus Windows/macOS native gate |
| `TC-TRIGGER-04` | Trigger and pointer side effects | Enumerate actual trigger/view DDL. Show files row writes fire the expected FTS/content triggers; a manifest pointer change does not. Verify explicit manifest-bound invalidation/revision and rollback without deleting retained rows. | #381 database/FTS owner |
| `TC-TAG-05` | Tags and batch metadata | Preserve tag catalog, FKs, labels and associations across hide/path reuse. Batch selection write CASes manifest/source/target set; on concurrent publication reject atomically and refresh. | #381 File Library metadata |
| `TC-HISTORY-06` | Historical read and derived action | Operation/classification/analysis/finding history remains readable from durable history ID without current visibility filter. History path alone cannot reveal/restore/delete or bind to replacement; every derived operation re-resolves current physical identity. | #381 owning history/operation modules |
| `TC-FTS-07` | FTS/content search | Use exact product-linked SQLite 3.51.3. External-content `files_fts`, content FTS, rename/delete/rowid and reopen/rebuild agree; active search applies Effective Visibility; preserve hidden source terms as retained data without leaking them. BM25 corpus contract is separately Owner-approved. | #381 FTS gate; product SQLite build evidence required |
| `TC-SCAN-08` | Root generation / Watcher / recovery | Incomplete/cancelled/old run, Watcher gap, root/session revision change or uncertain identity cannot publish. Root-local change preserves an unrelated valid root; overlapping source remains only if its other root/path/identity seals remain valid. Restart yields old or complete new pointer only. | #381 Scanner; #380 generation gate |
| `TC-WORKER-09` | Asynchronous result fencing | Race Content, FTS/catalog, Dedupe, Analysis, Classification, AI, Organization, Rule and Automation completion with source/root/manifest/cancel/policy change. Late results are rejected/superseded or kept as history; never attach by path to replacement. | Each owning #381 worker module |
| `TC-GLOBAL-10` | Global Index independence and bridge | Managed publication does not rewrite Global Index volume/entry stale status, FTS or unrelated queue. A managed bridge validates both Global Index identity and current Managed File source/manifest at admission and completion. | Global Index + #381 bridge |
| `TC-DBGEN-11` | Cross-generation client/process isolation | Old app/renderer/IPC/Service/worker/open connection is rejected before reading or writing the new DB generation; process already running, second process, restart, same-path replacement and installer recovery cases included. No Schema 37 fallback writer. | #380 mandatory Owner gate; not passed |
| `TC-FTSCURSOR-12` | FTS rank and Cursor scope | Cursor pages query only current effective sources. Any global collection/rank authority change expires a global token. Root B may survive root A change only if B's actual SQL scope, Root/Source seals and exact ranked result are unchanged; shared-source change expires it. Record full digest CPU/memory/input cost. | #381 FTS/Cursor; Owner must choose corpus and numeric resource budgets |
| `TC-SCHEMA-13` | Schema 37 preservation / migration | Disposable real Schema 37 DB: preserve `files.id`, rowid crosswalk, each FK and count/digest for tags, operation/classification/analysis history, content artifacts, fingerprints, duplicate groups/members and FTS. `foreign_key_check`, integrity, trigger/FTS checks, interrupted/duplicate migration and verified backup restoration all pass. | Joint #380/#381 migration plan; no version assigned |

### Responsibility assignment

- `db/queries/library`, `db/commands`, DTOs and `src/views`: File Library/current detail and renderer adapter owners.
- `db/queries/scan`, `scanner.rs`, `watcher.rs`: root/scanner/Watcher/run/recovery owner.
- `content.rs`, `db/queries/dedupe`, `db/queries/analysis`, `db/classification`, `ai`, `organization`, `rule_proposals`, `db/automation`: each repository/worker owner holds its source/run/policy revision checks.
- `db/schema.rs` and migration layer: coordinated database/FTS owner; #380 and #381 must approve one version and client rollout.
- `global_index/*`: keep Global Index independent; coordinate only explicit managed-scope/AI bridge calls.
- Tauri handlers and renderer API/type/mock surfaces: #381 IPC integration owner plus #380 generation gate.

### Validation boundary

S8 validates source positions and static ledger invariants only. It deliberately does not run SQLite Harness tests, app tests, full CI, Hosted Windows/macOS, product FTS experiments, query-plan research, or benchmarks. Those are distinct authorized gates, not implied by a complete inventory.
