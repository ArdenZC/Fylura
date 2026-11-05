# #381-B4-P0 — Bounded Codex Cloud implementation permission & acceptance grid

**Scope-go conditional on live master Schema37, candidate version38 arbitration.** This is an Owner executable instruction for a future independent Draft PR; it is not a production merge.

## Branch, allowed code, no-go

Branch suggestion: `fix/issue-381-b4-p0-expand-only-identity-schema`. Base on *fresh master* (frozen reference 3275ac7c72df8ae4139a30f16c7f649971324e97, tree 009576241cd4e8e2ba12cc644a368314f599faa6). Read PR #383 schema.rs patch specifically, but DO NOT use its branch or its v38 provisional FTS trigger. Read #400 B4-01 and this exact SQL contract, and #398 current cross-track P1 changes; report real master/version just before coding.

Allowed modifications ONLY:
1. `src-tauri/src/db/schema.rs` (conditional CURRENT_SCHEMA_VERSION, new private expand-only create/validation helpers, existing migrate branches strictly scoped).
2. `src-tauri/tests/migrations.rs` and `src-tauri/src/db/**` strictly related *test-only* modules or an exact-source additional adjacent test module if needed for fault injection; do not alter other production DB code.
3. `docs/project/research/issue-381-b4-p0/**` raw test/DDL verification, data-preservation and validation reports.

No changes: legacy `files` structure/rows/columns, File Library read/write SQL, scanner/reconcile/Watcher, restore/operation, tags, Content/Dedupe/AI, Global Index #380, FTS virtual table/triggers, new SQLite database file routing, IPC, UI, CI workflow/performance thresholds, settings/index-policy, tooling, installer/Release. No broad existing tests rewrites or production feature flags. Future F1 shadow slot work is P1, **not this PR**.

## Required status and exact evidence

| ID | Must demonstrate | Stop if |
| --- | --- | --- |
| P0-T01 | Fresh and repeated boot DB at v38 stays legacy; four dormant tables | any query/scan semantics switched |
| P0-T02 | Schema37 representative legacy full user facts are unchanged after migration | tags, content, dedupe, restore/history lost or transferred |
| P0-T03 | Older supported schema 16/36 remains upgradeable to new version; no retroactive public binary matrix | fixture migration regressions |
| P0-T04 | forced DDL failure rollback is all-or-nothing including user_version | residue/stale trigger/partial table |
| P0-T05 | #383 provisional v38 detected as incompatible, **before any ensure mutation**, rejected with stable error | same-version hybrid silently healed |
| P0-T06 | pre-existing colliding table in v37 rejected and rollback | name collision silently treated success |
| P0-T07 | same-path new incarnation cannot inherit existing tags from mere schema installation | tag migration without proof |
| P0-T08 | FK/CHECK negatives and double UID/observation revision verify schema constraints | unverified claim labelled verified |
| P0-T09 | existing read/search/tag/scan/restore flows still rely on legacy and new tables remain dormant | new ledger used by production |
| P0-T10 | Rust compile/format/clippy, SQLite bundled 3.51.3, actual related CI and lock timing, native qualification listed pass/skip | source/hash or CI scope misstated |

**Expected delivery:** one OPEN/Draft PR, exact base/master HEAD/Tree, final HEAD/Tree and full changed-file Diff, version arbitration, SQL signature and collision diagnostics, raw test command/exits and Database::open migration success/failure, normal exact-head CI. **Do not force manual unrelated full CI or Codex Review**. Stop at `ISSUE_381_B4_P0_EXPAND_ONLY_CANDIDATE_READY_FOR_OWNER_REVIEW` *only* if ten cases qualify; otherwise `ISSUE_381_B4_P0_BLOCKED` with first source-proven issue. Always `ISSUE_381_PRODUCT_FIX_NO_GO`; no broad implementation/merge/master write/Schema39 guess.
