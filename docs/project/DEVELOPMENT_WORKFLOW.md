# Zen Canvas Development Workflow

This workflow is the default engineering operating model for new work after G1. A narrower security/task contract may add stricter gates; it may not weaken these rules silently.

## Delivery-first operating rule

**Primary objective: ship observable user value without losing data or violating
existing security/CI rules.** Engineering effort must not be measured by number
of gates, reports, passes, branches, SHA receipts or tokens consumed. The normal
path for an already-scoped, reversible feature or bug fix is:

```text
small user outcome + acceptance criteria
→ Codex Cloud implements focused change and regression test
→ existing applicable CI
→ one substantive Owner review (fix concrete defects in same PR)
→ Owner merge → verify user-facing behavior
```

This **does not** make Research, Architecture Freeze and extra review mandatory
stages for ordinary work. Use them only when the triggering uncertainty changes
durable authority, product meaning, user data, platform safety, permissions,
release guarantees or an irreversible operation.

### Change classification: choose the lightest sufficient proof

| Work | Default path | Extra evidence only if ... |
| --- | --- | --- |
| UI, wording, layout, file details, bounded reversible behavior | implement directly; focused unit/UI check + existing routed CI; single review | actual permission/authority changes or an observed accessibility/native bug |
| Scoped scan/search/index correctness, query/performance fix | implement directly once current owner and invariants are known; regression and relevant measured benchmark + applicable CI | it mutates durable user facts, changes filesystem safety, performs migration, or changes accepted rank/cursor semantics |
| Persistence or file mutation affecting real user data | verify backup/rollback/identity and the relevant native or integration boundary; peer/Owner safety review | further fault/native proof is needed for a credible irreversible-loss mechanism |
| Schema migration, auto-delete, privilege expansion, release, unsafe native disk operations | scoped design and explicit Owner authorization before execution; applicable mandatory security/native/recovery checks | governed by the existing strict contract, never silently relaxed |

Risk classification is **claim- and impact-specific, not a license to classify
production changes as docs-only**. Existing CI routing and required checks
remain unchanged; ADR-0004 and ADR-0007 retain their scope.

### Prevent defensive work from spawning more defensive work

1. Each new research spike, bespoke harness, evidence package, manual hash
   catalog, standalone Issue or extra validation run must answer a **specific
   unresolved decision** that blocks the current change. State what outcome
   would change the next action. If none, do not create it.
2. On a normal PR, review once for intended behavior, regression coverage,
   affected authority/security, and applicable CI. Ask for an R1 correction
   **only on a concrete reproducible defect or meaningful unverified claim**.
   Do not automatically request a second independent review, a new PR, or
   R2/R3/R4 labels. Recheck only changed logic and relevant tests.
3. Do not require a new immutable SHA256 manifest or raw-log archive for an
   ordinary Git-hosted code/doc edit. Git diffs, commit SHA, PR and existing
   CI are the normal evidence. Artifact identity, binary release, native
   qualification, regulated/audited proofs and irreversible migrations may
   legitimately need richer receipts; explain why before collecting them.
4. Do not rerun an unchanged broad CI matrix solely because a document,
   review comment or unrelated branch changed. Existing required CI may run
   automatically; do not disable it. Never manufacture a current-head pass
   from an older commit or equate conditional skip with executed coverage.
5. Avoid multi-step approval ladders for independently safe work. A blocker
   must identify **protected invariant → plausible harm → evidence → exact
   blocked scope → minimal remedy or bounded deferral**. Unknown exotic
   scenarios can be tracked as `NOT_TESTED` without blocking unrelated
   features. A deferred **required release** test still blocks that release
   claim; it does not magically pass.
6. Prefer repair in the existing feature PR and a single decision thread.
   Use a distinct research branch or Issue only when reproducibility,
   independent ownership or parallel work genuinely warrants it. Do not
   make research completeness a substitute for product progress.

### Apply this to the existing open backlog immediately

**This rule applies to already-open Issues, Draft PRs and research branches on
the day it merges, not only to future tasks.** Before continuing an old thread,
ask whether the next action will deliver a user-facing improvement, repair a
reproduced defect, or answer an imminent irreversible-safety decision.

- **Resume active delivery:** choose the smallest independent user defect/fix
  that can be tested against current `master`. Reuse earlier research and
  targeted regressions; do not inherit old approval ladders by default.
- **Park finished research:** once source/model evidence is sufficient for
  its original bounded question, stop extending it. Keep linked evidence and
  record `research complete / product not implemented` if that distinction
  matters. An unmerged test-only PR is not product progress.
- **Stop duplicating obsolete drafts:** historical designs superseded by a
  merged implementation (or a newer decision) should not be rebased only to
  make their old CI green. Preserve valuable findings as historical references;
  **review before closing** so no unique code or unresolved security bug is
  lost. Do not auto-merge stale, large experimental PRs.
- **Localize blocked claims:** a blocked Windows directory-sync/power-loss
  qualification is not a global ban on ordinary Files UI, metadata notes or
  narrow safe scan filtering. Still refuse any product path that actually
  relies on the unproved durability guarantee.
- **No one-time cleanup bureaucracy:** do not launch a dedicated audit project,
  parallel backlog ledger, hundreds of archival comments, one more safety
  harness or mandatory bulk review to enact this. Triage as each already-open
  item is touched and place the decision in its existing thread.
- **Limit simultaneous active implementation by capacity, not a new quota:**
  prioritize one user-visible change and one clearly reproduced defect; add a
  foundational task only where its dependency is real. Paused work consumes no
  new Codex/CI cycles until activated for a specific purpose.

**Current observed examples (illustrative, not an authorization to close or
merge):** `#408` is extensive **test-only** F1 work; `#383` is a blocked
oversized production attempt and not a safe direct merge; `#411` B04 native
probe remains unexecuted while its source work is accepted; `#414` real
ENOSPC and physical power-loss work is deferred by Owner. Re-evaluate on
current GitHub facts before acting on any of these. Do not touch live product
code or existing required checks in a backlog-cleanup step.

### Handoff, limits and measurements

- Codex Cloud owns implementation and tests; Owner/ChatGPT owns research,
  product scope, review and merge decision. The Windows workstation performs
  user-approved native **acceptance only**. No Codex Review.
- An ordinary task brief fits in a short goal, allowed scope, 2–5 observable
  acceptance examples, necessary invariants and test command/CI expectation.
  Link existing contracts rather than restating pages of frozen hashes.
- An ordinary review result answers: **what changed for users; what actually
  passed; what still blocks this PR**. Provide a PR link and compact exception
  details. Elaborate raw logs and tree/hash tables only for a disputed,
  high-risk or explicitly required identity claim.
- When a task is drifting into repeated audit cycles without user value,
  compare the expected cost of further research to the credible failure it
  protects against; either fix the concrete defect, reduce scope, or park
  it with a truthful limitation and work on a safe independent feature.
- In a lightweight weekly product review, track **user-visible changes merged,
  elapsed issue-to-merge time, reopened/regressed changes and blocked waiting
  time**. These are local proxies, not DORA deployment-frequency claims:
  `merge` is not `release`. Review the balance of throughput and defects;
  never reward PR count or token reduction at the expense of correctness.
  Avoid creating a second persistent metrics ledger solely to measure this.
- Initial operating target: reserve the *majority* of time for implementation
  and user-facing validation; spend materially less on writing review
  artifacts. This is a direction, **not a rigid percentage gate**.

**Safety floor:** keep all authoritative filesystem confirmation, identity,
journal/recovery, consent/privacy, schema compatibility/rollback, required CI
and platform/release proof. Bypassing an existing gate needs an approved change
to its owner; it is not allowed by this delivery-first rule. See
[ADR-0015](DECISIONS/0015-delivery-first-engineering.md).

## Master-plan alignment gate

The Master Development Plan is the long-horizon direction, not a default task
checklist. Read [`MASTER_DEVELOPMENT_PLAN.md`](MASTER_DEVELOPMENT_PLAN.md) when
the task involves:

- a new initiative or new Wave;
- cross-Wave scope or a product-direction change;
- a new durable subsystem or architecture change; or
- ambiguity about whether the task belongs in the current initiative.

Ordinary bug fixes, component migrations, bounded refactors and implementation
of an already-authorized task do not require a full Master Plan preload. The
task still must respect the current task brief, `STATUS.md`, existing
authority/security contracts and the stop conditions below. Do not recursively
preload linked documentation.

If implementation appears to require any of the following, stop and escalate
before coding further:

- a cross-Wave feature that the current initiative does not authorize;
- a new durable authority or schema migration;
- replacement of an existing safety/read/mutation/query/watcher authority;
- a supported-platform change;
- a performance-threshold reduction used to make a feature pass; or
- a broad native-integration subsystem that belongs to a later Wave.

## Code maintainability gate

Before materially expanding an existing source module, read
[`CODE_MAINTAINABILITY.md`](CODE_MAINTAINABILITY.md) and inspect the responsibilities already owned by that file/module.

Do not default to appending all new behavior to the existing feature file. A feature name is not a sufficient single responsibility.

Decomposition is required or must be explicitly reviewed when the change would cause a file/module to own multiple independent lifecycles or infrastructure concerns such as:

- request/session ownership plus cache/storage lifecycle;
- scheduler/admission plus an independent executor/queue;
- orchestration plus substantial filesystem/network/database/native-helper I/O;
- generic domain/service logic plus large platform-native implementations;
- multiple durable/state authorities;
- large in-file tests/fixtures that obscure production behavior.

Treat a global coordination lock covering filesystem I/O, `fsync`, provider/network calls, subprocess/native-helper work or other slow/external work as a design smell requiring explicit review.

File length is a signal, not a mechanical limit. Around 500–800 lines of hand-written production logic should trigger an active cohesion check; 1000+ lines normally require either decomposition or a clear review explanation; 1500+ lines must not gain another independent responsibility without a specific reviewed exception. Generated/data-heavy/cohesive code may legitimately differ.

Do not evade this rule by creating dozens of meaningless micro-files. The target is a small stable module surface with clear ownership and the smallest coherent decomposition.

For substantial new subsystems and Codex/agent taskbooks, include maintainability/module-boundary review in the Definition of Done. Independent reviewers may make decomposition merge-blocking when the current structure contributes to correctness, concurrency, lifecycle, locking or testability risk.

## Initiative lifecycle

**The full lifecycle below applies to cross-cutting or high-risk
initiatives—not every small bug fix or UI enhancement.** Ordinary bounded,
reversible implementation follows the short delivery-first path above.

```text
Research (when needed)
→ Spec (lightweight by default)
→ Architecture Freeze (only when triggered)
→ Wave/Track (only for larger initiatives)
→ PR → Review → applicable Integration Gate → Closeout
```

The stages have distinct purposes:

- **Research** establishes the problem, evidence and affected current authorities.
- **Spec** fixes user outcome, scope, non-goals, acceptance and validation intent.
- **Architecture Freeze** records any authority, persistence, platform, permission or recovery decision before implementation.
- **Wave/Track** breaks an approved initiative into bounded execution units with one coherent branch per unit where needed.
- **PR** packages the intended diff, exact-head evidence, risks and unverified areas for review.
- **Review** checks behavior, authority boundaries, scope, maintainability/module ownership and evidence rather than only code style.
- **Integration Gate** verifies the applicable checks and current-truth updates at the exact head before merge.
- **Closeout** records the merge result, deferred work, local test-artifact cleanup and branch/content-equivalence cleanup.

Do not begin production implementation from an informal idea when the change moves architecture authority, schema, platform safety, user-file mutation or product ownership. Write the initiative/spec first. **Do not apply that prerequisite to ordinary fixes that preserve those boundaries.**

## One coherent branch per initiative

Default branch naming:

- `feat/<initiative>` — product capability.
- `fix/<scope>` — bounded defect/remediation.
- `chore/<scope>` — governance, tooling or maintenance without product behavior change.
- `docs/<scope>` — documentation-only work.

Avoid permanent integration branches. If an initiative requires parallel sub-work, integrate through explicitly bounded branches/PRs and close them when their content is absorbed.

Never commit directly to `master` for non-trivial work.

## Local worktree lifecycle

### Main/common checkout role

The common/main checkout is the preferred stable repository entrypoint. Outside bounded owned work, it should normally return to `master + clean tracked state`. A temporarily non-master main checkout is not automatically a global blocker.

The blocker is using unknown or stale state as a new task baseline, destructive cleanup without ownership or preservation proof, or shared common-repository integrity failure where Git topology itself is unreliable. An unhealthy main checkout does not automatically block unrelated healthy linked-worktree activity.

### Bounded linked worktrees

A linked worktree should normally represent one coherent active task, review, integration or exact-SHA evidence purpose. Do not reuse an unrelated historical worktree merely because its directory exists. Before reuse, verify its branch, HEAD, status and purpose.

### Ownership / disposition

The task that creates a worktree normally owns its closeout. When the task or PR is merged, superseded or abandoned, give the worktree an explicit disposition: retire it, intentionally retain it for a current active or unresolved purpose with that reason recorded, or record the precise reason cleanup is blocked.

Do not create permanent enum or state vocabulary for these dispositions; use natural language. “Forgotten” or “historical” alone is not a valid long-term reason to retain a worktree.

### Safe retirement

Before worktree removal, verify relevant topology, committed-work preservation, staged/unstaged/conflicted/untracked state, and evidence ownership or disposition. A local branch ref protects only committed history. Ignored files are not automatically disposable. Unknown local content prevents destructive cleanup.

### Branch/worktree separation

Branch deletion and worktree removal are distinct. A worktree may be removable while its branch or ref is intentionally retained. A branch may be content-equivalent to the accepted merge while the worktree still contains local evidence requiring disposition. Continue using the existing squash content-equivalence rule; do not invent another merge-equivalence mechanism.

### Signals, not authorities

Treat `[gone]` upstream, detached HEAD, branch ahead count, age and worktree count as investigation signals only. None independently authorizes deletion.

### Repository repair vs cleanup

If normal Git operations fail because of a missing or corrupt object, invalid ref, common repository integrity problem or worktree metadata corruption, treat it as repository recovery. Do not solve repository corruption by deleting arbitrary refs or worktrees merely to make cleanup green. Repair trustworthy Git integrity first where possible.

If multiple refs or objects are damaged or ownership is uncertain, stop bounded cleanup and report the repository-repair scope.

### Git-aware removal / prune

Prefer `git worktree remove <path>` after preservation checks. Do not use force removal merely because cleanup is inconvenient. `git worktree prune` is maintenance after topology is understood; it is not a discovery or ownership tool. No scheduled or automatic prune belongs in CI.

### No numerical policy

Do not introduce a maximum worktree count, worktree TTL, branch TTL or age-based deletion. Age and count are signals only.

## Scope hygiene

Before changing files:

1. record current branch and `HEAD`;
2. confirm the intended base;
3. inspect existing callers and authority contracts;
4. inspect the responsibility/module boundary of files that will be materially expanded;
5. identify unrelated changes and leave them untouched;
6. stage only intended paths.

Do not use broad staging to absorb unrelated work.

## Task brief standard

A bounded task brief should compress context rather than amplify it. Prefer
these sections:

- Goal
- Scope
- Do not change
- Relevant invariants
- Start here — symbols, files, callers and tests
- Read only if triggered
- Acceptance
- Validation

For large source files, use symbol-first investigation by default:

```text
rg target symbol → inspect callers → focused ranges → neighboring types/tests
```

Do not require a complete read of a large file unless subsystem ownership or a
specific correctness claim makes that necessary.

## Atomic commits

Each commit should have one reviewable purpose. Prefer messages such as:

- `docs: install project current-truth layer`
- `fix(watcher): preserve reconciliation revision ownership`
- `refactor(runtime): split search lifecycle controller`

Avoid messages such as `update`, `fix stuff` or `refactor all`.

A large initiative may have multiple atomic commits; a small focused change may have one.

## Merge strategy

Default project strategy: **squash merge** for ordinary feature, fix, governance and integration PRs.

Use a merge commit only when preserving a meaningful multi-parent topology is itself useful and explicitly reviewed. Rebase merge is not the default project strategy.

After squash merge, the source branch's commits will not be ancestors of `master`. Branch closeout therefore uses content equivalence when ancestor checks are insufficient.

## Validation by risk

Run focused checks first, then the applicable repository gates. Validation is based on the claim that needs proof, not on repeating a fixed command list after every repository change.

During the editing loop, prefer focused validation. Run expensive broad applicable checks primarily on a stable candidate rather than after every small edit.

A previous successful result may remain useful for development reasoning when the inputs relevant to that claim, its required environment and any explicit freshness requirement have not changed. Re-run the validation when those conditions no longer hold, when the earlier result failed or was incomplete, or when the current integration/artifact stage explicitly requires fresh evidence.

Previous evidence must not be promoted into a new exact-head claim. A later production commit still requires the exact-head evidence required by the current project and CI contract unless that contract explicitly permits tree/content equivalence, a documentation-only successor or another documented reuse rule. Artifact/native evidence likewise remains bound to the exact source, artifact identity and environment actually exercised.

The current CI workflow and change-classification contract own hosted minimum validation routing. Local development and taskbooks may use narrower focused checks while iterating, but they must not maintain a competing repository path-routing table, waive required hosted lanes or reclassify production changes merely to reduce validation cost. If existing CI routing appears unnecessarily broad, change the owning routing contract through a separately reviewed and tested change rather than bypassing it inside a task.

A required gate that is red, missing or incomplete remains blocking until it passes or the owning contract is separately reviewed and changed. Proportional failure handling must not be used to reinterpret an existing required gate as optional.

When an expensive validation is repeated within the same task, record the reason when it is not obvious from the changed inputs. Valid reasons include a relevant source/test/gate change, a material claim-relevant environment or toolchain change, an earlier failed/incomplete run, a newly required exact-head integration claim, changed artifact identity or an explicit freshness requirement.

Failure handling should be proportional to the protected claim. Fail closed at the narrowest boundary that fully contains credible irreversible harm, authority violation, unsafe persistence or incorrect release. When continuing cannot create such harm, prefer truthful partial/reconciliation state, degradation or reporting rather than unnecessarily blocking a broader product or engineering workflow.

A permanent blocker should have an identifiable protected claim, credible harm and blocking scope. Temporary remediation gates should not silently become permanent project policy after their original condition is gone. Conversely, removing or weakening an existing required gate requires a separately reviewed change to the contract or routing authority that owns it.

General proportionality guidance never weakens a stricter accepted security, platform, filesystem, persistence, permission, recovery or domain-specific contract.

Typical production checks include:

```bash
npm run typecheck
npm test
npm run test:remediation
npm run test:performance:architecture
npm run build:check
npm run verify:rust
npm run verify:security
```

Use the current CI workflow contract for Windows/macOS, performance and package coverage. High-risk filesystem/platform/schema/performance work requires the matching full validation.

Docs-only work may use the docs fast path only when the diff is genuinely documentation-only. Do not classify production changes as docs-only to reduce CI cost.

Never claim a gate passed unless it ran successfully on the stated commit/environment.

### Local test artifact hygiene

Local validation must be disk-safe as well as logically correct.

- Prefer repository/worktree-local ignored directories for generated test data, especially `.tmp-tests/`, `.tmp-performance-fixtures/`, `.performance-temp/`, `.performance-cache/` and `.performance-artifacts/`.
- On Windows, if the worktree is on a non-system drive, repository-controlled tests/benchmarks must not default fixture, staging, cache or large temp output to `C:` through `%TEMP%`, `%TMP%`, `std::env::temp_dir()` or similar APIs. Configure the command/test harness to use a worktree-local ignored temp root on the worktree drive.
- Tests that create fixture trees or files must have an explicit ownership/cleanup strategy. Cleanup must be bounded and scoped only to task-owned paths.
- Do not solve cleanup by deleting unrelated developer state or shared dependency caches such as `node_modules`, Cargo registry/git caches or shared build caches.
- Before reporting a task complete, remove task-owned test/benchmark/staging artifacts and verify the intended temporary roots are clean. If a lock or local security policy prevents deletion, record the exact path and treat cleanup as unresolved rather than silently leaving garbage behind.
- CI runner-owned temp/workspace paths are allowed, but tests must remain portable and must not encode a developer-specific `C:` path.

## Exact-head evidence

Validation evidence must record the exact commit SHA. If a follow-up commit changes production code, earlier exact-head results are evidence for the earlier commit only.

A docs-only follow-up may reference the immediately preceding validated production head, but must state that distinction explicitly.

Earlier evidence may remain semantically informative during development when the claim's relevant inputs have not changed, but it does not satisfy a later production commit's required exact-head gate unless the current project/CI contract explicitly permits that form of evidence reuse.

## Pull request policy

- An ordinary PR needs one substantive code review. A reviewer may request specific corrections, but further independent audit rounds require a concrete new defect, changed risk or required gate, not a default R2/R3 ritual.
- Prefer one PR for one coherent initiative or review fix.
- Broad initiatives start as Draft unless the scope explicitly says otherwise.
- PR descriptions state scope, authority changes, module/responsibility changes, risk, verification and known unverified areas.
- Review fixes stay on the same initiative when possible instead of creating permanent chains of integration branches.
- Never merge merely because CI is green; authority, maintainability, review and closeout gates still apply.

## Current-truth update before merge

Before final merge of an initiative, update as applicable:

- `docs/project/STATUS.md` — applicable product/runtime baseline, initiative state, validation, schema/version/release state.
- `docs/project/ARCHITECTURE_MAP.md` — authority/ownership changes.
- `docs/project/PRODUCT_MAP.md` — user/workspace ownership changes.
- `docs/project/TECH_DEBT.md` — debt opened, changed or closed.
- `docs/project/RISK_REGISTER.md` — project-level risk changed.
- `docs/project/ROADMAP.md` — sequencing/authorization changed.
- `docs/project/DECISIONS/` — accepted cross-cutting decisions.

Do not duplicate current state into a new closeout file.

A final squash-merge SHA cannot be known before merge. The initiative PR therefore records the exact head it validated; once merged, a bounded documentation-only closeout may record the actual initiative merge SHA and branch cleanup. That closeout does **not** need to predict or self-reference its own future squash-merge SHA unless the closeout itself changes production behavior or a long-lived authority. Keep product/runtime baselines, exact validation heads and governance/closeout evidence as separate facts.

## Closeout and branch cleanup

An initiative is closed only when:

1. intended changes are merged;
2. final initiative merge SHA is known;
3. required exact-head validation is recorded;
4. task-owned local test/benchmark/fixture/staging artifacts have been removed, or an exact cleanup blocker/path is explicitly recorded as unresolved;
5. the task-owned worktree is retired, intentionally retained for a current unresolved or active purpose with that reason recorded, or cleanup is blocked and the exact blocker is recorded;
6. `STATUS.md` reflects the merged initiative state;
7. deferred/unverified items are explicit;
8. source/integration branches are deleted after ancestor or content-equivalence verification, or an exact branch-preservation blocker is recorded; associated worktree cleanup is safely completed or an exact preservation/cleanup blocker is recorded.

Branch deletion and worktree removal are separate closeout decisions.

For squash-integrated branches, compare the branch diff/content against the merge result before deletion. An `ahead` count alone is not proof that work is missing.

A documentation-only closeout PR is bookkeeping for the already merged initiative. Its own merge SHA does not recursively become another required initiative baseline unless it changes product/runtime behavior or a durable governance authority beyond the closeout facts already reviewed.

## Release state

Keep these states separate:

```text
Implemented → Validated → Packaged → Released
```

A successful NSIS/DMG build is `Packaged`, not `Released`. A GitHub release/tag or other explicitly published distribution is required for `Released`.

## Architecture decision trigger

Create an ADR when a change:

- moves durable authority or persistence ownership;
- changes supported platforms;
- changes filesystem mutation/recovery strategy;
- changes cross-window permissions;
- introduces a new long-lived subsystem or queue;
- changes merge/CI/release governance materially.

ADRs are for long-lived architecture or governance decisions, not ordinary implementation details, local bug fixes, formatting or routine documentation edits.

Small local refactors that preserve these facts do not need an ADR.
