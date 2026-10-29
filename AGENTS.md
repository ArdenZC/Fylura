# Zen Canvas Agent Instructions

This file is the stable repository constitution. `docs/project/STATUS.md` owns
changing project stage, baseline, active task, release state and next phase.
Do not copy those changing facts into this file or infer them from historical
documents.

## Default task start

For an ordinary bounded task, use this context sequence:

```text
AGENTS.md
→ docs/project/STATUS.md
→ the current task brief linked from STATUS
→ inspect relevant production owners / callers / tests
```

Read only the documents triggered by the task's domain or by an unresolved
ambiguity. Do not recursively preload linked documentation. The routing index
is `docs/project/README.md`.

| Trigger | Read when triggered |
| --- | --- |
| product ownership or navigation | `docs/project/PRODUCT_MAP.md` |
| state, persistence or durable authority | relevant section of `docs/project/ARCHITECTURE_MAP.md`; `docs/project/DECISIONS/` when ambiguous |
| filesystem mutation, Restore or Safe Trash | relevant security, identity and recovery contracts |
| Tauri command or capability permissions | `docs/security/TAURI_COMMAND_PERMISSION_MATRIX.md` and affected permission authorities |
| macOS mutation | the macOS mutation threat model |
| new initiative, cross-wave scope or product direction | `docs/project/MASTER_DEVELOPMENT_PLAN.md` plus `docs/project/ROADMAP.md` |
| large responsibility expansion | `docs/project/CODE_MAINTAINABILITY.md` |
| release or publication | release contracts and `docs/project/DEVELOPMENT_WORKFLOW.md` |
| governance/process change | `docs/project/DEVELOPMENT_WORKFLOW.md` |
| another named domain contract | that security, remediation, design or QA contract |

## Default delivery mode — implement first, escalate only for credible risk

**The default for a bounded feature or bug fix is working code delivered through
one ordinary PR, not a research/audit program.** See the binding
[delivery-first section](docs/project/DEVELOPMENT_WORKFLOW.md#delivery-first-operating-rule)
and [ADR-0015](docs/project/DECISIONS/0015-delivery-first-engineering.md).

- Write a short outcome, scope and acceptance note; inspect the nearest owners
  and tests; let Codex Cloud implement code and focused checks immediately.
  Do not demand a separate feasibility study, ADR, research branch or test
  harness for routine changes with unchanged durable authority.
- Prefer one coherent PR and one substantive review. Fix real, reproducible
  defects in that PR; do not auto-create a new Issue, remediation PR, R2/R3
  cycle or independent evidence archive for every review comment.
- Use existing tests/CI and Git identity. **Manual SHA-256 catalogs, repeated
  rehashes, duplicate CI reruns, per-check raw-log receipts and new safety
  gates are exceptional**, justified by an actual identity/provenance,
  release, migration or irreversible-mutation risk. They are not a default
  definition of done.
- A blocking finding names the exact credible harm, reproduction/evidence,
  affected scope and smallest remedy. Isolate unrelated work and keep
  delivering safely independent changes; do not global-NO-GO by association.
- Measure useful merged functionality, elapsed delivery time and avoidable
  rework. Reports, gate counts and research artifacts are not product output.
- Never bypass an **existing required** CI/security gate, conceal an
  unverified platform claim or loosen user-file mutation, permission,
  schema/rollback or privacy protections for speed. Changing required
  governance needs its owning approved process; deferred work stays honest.
- Owner/ChatGPT owns product decisions, research and review; Codex Cloud
  implements code; the Owner Windows machine is acceptance-only.
  Never use Codex Review. Merges remain Owner decisions.


### Before an ordinary PR — fast feedback by default

- Inspect changed paths and run \`npm run dev:preflight -- --plan\` before
  pushing a normal code PR. Execute the relevant mapped checks locally when the
  toolchain is available; otherwise name the environment constraint and rely on
  applicable CI. The plan is a developer feedback habit, **not a new required
  approval gate** or a reason to duplicate costly full suites.
- For parallel Codex work, first identify the owning source module and open
  implementation PRs. Separate editing scopes by existing architecture
  authority; one worker owns a shared file at a time. Coordinate unavoidable
  overlap explicitly before the second worker modifies it. Never force-push
  another worker's branch or promote mocked native acceptance into PASS.
- Make a short handoff: user-visible result, PR, relevant checks, concrete
  blockers. Prefer current \`STATUS.md\` snapshot to scanning old taskbooks.

## Current truth and scope

- `STATUS.md` is the project-level source for changing baseline, initiative,
  track, task, phase, validation, schema, platform and release facts.
- `README.md` defines source precedence and conditional routing. Historical
  taskbooks, audits, prompts and PR closeouts are evidence unless current truth
  explicitly links them for the task.
- Honor the task brief's scope and non-goals. A documentation task does not
  authorize production, schema, runtime, security-authority or CI-routing work.
- Keep one durable authority per domain. Do not create a second current-status
  file, ledger, queue, index, mutation journal or recovery authority.

## Git, baseline and worktree safety

- Verify `git status --short`, branch, `HEAD` and `HEAD^{tree}` before editing;
  verify the requested base against the actual remote/local object.
- Use an isolated branch/worktree for non-trivial work. Never commit directly
  to `master`.
- Preserve unrelated dirty, staged, conflicted, untracked and ignored content.
  Do not reset, clean, rebase, merge, force-push or overwrite unexplained local
  state. Stage only explicitly intended paths.
- A branch ref preserves committed work only. Before retiring a worktree, prove
  topology, local-state disposition and evidence ownership; prefer Git-aware
  removal and never use force cleanup merely for convenience.
- Keep validation and delivery claims bound to the exact commit and tree that
  were tested. Do not promote old evidence to a later production head.

## Durable authority and truth boundaries

| Product area | Durable authority |
| --- | --- |
| system-wide metadata search | Global Index / Global Search repository |
| managed file browsing | File Library Query V2 |
| cross-page managed selection | `LibrarySelectionV1` plus backend resolution |
| scan and watcher health | durable scan sessions/runs and backend reconciliation |
| duplicate truth | durable Dedupe runs/groups/members |
| storage analysis | Analysis Run/Finding/Evidence/Decision |
| organization review | Organization Plan / Plan Item ledger |
| filesystem execution | authoritative Operation Preview and operation journal |
| cleanup and restore | Safe Trash/cleanup journal plus operation and recovery ledgers |
| automation rules | Rule Repository V2 and catalog revision |
| content understanding | Content Scope Policy, Content Run and Content Artifact |
| managed AI | existing Managed AI queue and provider policy |
| settings | persisted versioned settings/provider contracts |

Renderer stores, rows, page samples, local counts and lifecycle state are
projections unless an accepted architecture decision says otherwise. Do not
make renderer paths execution truth, merge Global Search with File Library or
Content Search, or accept a local sample as a complete total/group/decision.
Compatibility adapters may translate into one authority temporarily; they must
not become permanent second authorities without an explicit deletion condition.

AI/provider output remains advisory and consent-bound. It must not silently
mutate files, accept organization decisions, enable/run rules, authorize
cleanup, send managed content to cloud or bypass preview/confirmation.

## Preserved product invariants

- Global Search keeps backend file ordering authoritative; punctuation remains
  literal; IME composition owns query/navigation until commit; `no_source` is
  distinct from an ordinary empty result; Search Window activation stays
  ID-only within its restricted permission boundary.
- Watcher/root health keeps permission required, reconciliation required,
  partial coverage and retry exhausted distinct.
- Rule Repository V2 remains the automation mutation authority. Do not restore
  the removed whole-object commands `save_user_rule`, `delete_user_rule` or
  `get_user_rules`.
- A loaded page, rendered row, local count or proposal sample is never a
  complete authoritative total, group or impact set; use backend aggregates,
  exact/deferred counts or durable ledger projections.

Moving durable authority, persistence ownership, platform support, window
permissions or filesystem mutation/recovery strategy requires an accepted ADR
under `docs/project/DECISIONS/` before implementation.

## Filesystem, platform and permission safety

All user-file mutation preserves this chain:

```text
intent → authoritative preview → explicit confirmation where required
→ backend revalidation / identity checks → durable journal or Safe Trash
→ filesystem mutation → durable outcome → History / Restore
```

Filesystem strategy is backend-owned; the renderer must not infer safety from a
path or OS check. Product targets are Windows and macOS 13+ Apple Silicon.
Do not claim native macOS evidence from Windows cross-compilation or promote
browser/mock evidence into native acceptance.

When a Tauri command changes, keep Rust registration, build generation input,
capabilities, permission matrix, frontend facade/browser mock and contract
tests synchronized. Do not broaden the Search Window mutation boundary.
SQLite remains the persistence authority; schema/migrations require authorized
migration and rollback/future-version gates. Use the centralized frontend
API/event layer. Do not migrate `files.id` outside an authorized initiative.
Browser mocks must stay deterministic and honest.

## Validation and evidence

- Use scripts defined by `package.json`. Run focused checks first, then the
  applicable broad gates on a stable candidate.
- Do not weaken thresholds, silently downgrade a required gate, or classify
  production work as docs-only. A red, missing or incomplete required gate is
  blocking under its owning contract.
- Never claim a test, visual state, platform check, package or release passed
  unless it actually ran. Record exact SHA, environment and skipped/unverified
  boundaries.
- Local fixtures and caches belong in worktree-local ignored roots; clean only
  verified task-owned temporary artifacts at closeout. Preserve shared caches.
- For large source files, default to symbol-first investigation:
  `rg target symbol → inspect callers → focused ranges → neighboring tests`.

For user-facing UI changes, use shared tokens/primitives/i18n and verify the
applicable keyboard, focus, modal, screen-reader, reduced-motion, contrast,
loading/failure and narrow-layout behavior. Static inspection alone is not
accessibility or native acceptance.

## Stop ongoing redundant verification

These delivery rules also apply **mid-task**. For current F1/T10,
native-probe, Index Scope or Global Index investigations: do not order more
optional models, re-audits, manual hash receipts or CI reruns on unchanged
sources just to reach a larger PASS count. See the
[current-verification stop rule](docs/project/DEVELOPMENT_WORKFLOW.md#stop-ongoing-optional-verification-now-not-just-future-tasks).
Use existing results honestly; retain real blocking required failures
without expanding them to independent work.

## Handoff brevity

For an ordinary change, report **what the user can now do**, the PR/commit,
focused tests and applicable CI result, and any concrete unresolved blocker.
Omit exhaustive head/tree tables, every skipped conditional CI leg and
historical evidence replay unless necessary for the exact claim. A successful
source review is not a native or release result.

## Closeout

Before handoff, inspect the explicit diff and final status, update applicable
current-truth files, run claim-scoped validation, remove owned temporary
artifacts and report changed files, authority/compatibility impact, commands,
visual/native evidence, acceptance, deferred items and human-review risks.
Review and merge remain owner decisions; green CI does not authorize merge.
