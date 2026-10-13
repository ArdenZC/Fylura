# Zen Canvas — Active Development Decision Ledger

- **Snapshot checked:** 2026-10-13
- **Owner phase:** ACTIVE DEVELOPMENT — no public release has occurred; no current release schedule, RC or publication gate
- **Live GitHub source of truth:** [repository](https://github.com/ArdenZC/Zen-Canvas), tracked issues/PRs and their immutable evidence
- **Frozen reference baseline:** master 3275ac7c72df8ae4139a30f16c7f649971324e97
- **Baseline tree:** 009576241cd4e8e2ba12cc644a368314f599faa6
- **Runtime baseline in repository:** internal package 0.1.40, DB Schema 37, Global Index IPC v3
- **Latest independently checked master Full Validation:** [run 38229431247](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38229431247), scheduled 2026-10-12, **22/22 jobs success** (Windows/macOS Rust and release compiles, CI aggregates, benchmarks, packaging and audit). Green workflow is NOT a claim that the open native/production defects are fixed.

## 1. Interpretation rules and change control

This document is a **point-in-time Owner reconciliation**, not automatic authorization to implement, merge, close, expand schema, run a release or waive CI. Re-read live master/PR/Issue before action; a current issue comment may supersede an older task description. Preserve previously accepted exact-HEAD evidence and unresolved counterexamples. An accepted research/test result is not a production behavior fix.

Zen Canvas has **never publicly shipped**. Internal QA package 0.1.40 and old development schema/IPC builds do not create a public legacy compatibility requirement. K1/K2-T0/K2-G1 old-binary upgrade-isolation experiments are retained for eventual post-public-release update design but do **not** block normal development, production correctness work or future product capability work. This does not weaken same-version runtime data integrity, atomicity, file safety, recovery, source truth or fresh-install quality.

Working model: ChatGPT/Owner performs architecture, research, source analysis, bounded design, status governance and independent PR acceptance; Codex Cloud is the only production-code/test-harness implementation environment. Windows local machine is for separately authorized **native acceptance, not development**. No Codex Review, unapproved merge, Issue closure, master mutation, release, CI bypass or blanket heavy CI rerun.

## 2. What is merged versus still unproven

The merged product foundation includes Library/Browse/Preview, Safe Trash/journal/Restore, automation, Zero-Burden runtime foundations and AI-only PM-01/PM-02A/PM-02B/PM-03. Bounded Windows Global Index Recovery #323 was merged through PR #327 with explicit old native-evidence exceptions, not a whole-platform unconditional PASS.

Recent true merge events:
- [PR #335](https://github.com/ArdenZC/Zen-Canvas/pull/335) / Issue #329: closed and merged as master df37531f785193a948409e2f8cb36369561c6a6f. First-run scope persistence source repair accepted; original clean-first-run scoped native result was PASS, but live watcher refresh, some Global Index states and comprehensive cross-platform native acceptance remain separate limitations. Do not call #329 OPEN or PR #335 Draft.
- [PR #379](https://github.com/ArdenZC/Zen-Canvas/pull/379): merged test-only synthetic Index Scope benchmark, not a production filter.
- [PR #377](https://github.com/ArdenZC/Zen-Canvas/pull/377): merged Windows Service busy diagnostics as master 5c818dcbe4f55a8331db884de4e37a2c41953887; **did not repair** the typed busy/source-status problem.
- [PR #378](https://github.com/ArdenZC/Zen-Canvas/pull/378): merged Managed Scan false-exclusion characterization as current master 3275ac7c72df8ae4139a30f16c7f649971324e97; exact final PR-head [CI 38128792996](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38128792996) succeeded (19 success, 8 conditional skipped). Production filter **not repaired**.

The repository's STATUS.md and ROADMAP.md contain valuable historical decisions but their former top-level current-state language predates this reconciliation. Do not rewrite historic completed outcomes to imply they never happened; take the latest snapshot/Issue evidence for current assignment.

## 3. Eleven open GitHub Issues — exact disposition

| Issue | Track | Evidence/state on 2026-10-13 | Next bounded action |
| --- | --- | --- | --- |
| [#380](https://github.com/ArdenZC/Zen-Canvas/issues/380) | P0 Windows Service busy/source truth | Original Issue is a **narrow trusted IPC v3 request-level busy** decode/Coordinator persistence fault; PR #377 diagnostic merged. Existing PR #386 is wider D1-A **not independently mergeable**; K2 comments are separate future-upgrade research. | Owner-approved narrow Codex Cloud fix: trusted error_code typing + safe non-admitted source-state restoration, tests, Draft PR. Stop if owner/revision-safe rollback needs a larger architecture. |
| [#328](https://github.com/ArdenZC/Zen-Canvas/issues/328) | SQLite writer contention | Real Windows native startup/recovery briefly recorded database-is-locked/unavailable; recovered later; root writer/process unproven. **Not the same as #380 request busy**. | Separate cause-focused native/source identification without generic timeout increases. |
| [#381](https://github.com/ArdenZC/Zen-Canvas/issues/381) | Managed Scan false exclusions and stale correctness | Real production candidate [PR #383](https://github.com/ArdenZC/Zen-Canvas/pull/383) OPEN/Draft. Hosted Windows 100k transaction lock exceeded 5000 ms writer-liveness budget (D3: 5284 ms) and metadata writer timed out. D3R Linux SQL-plan optimization around 1.4% is not a new Windows PASS; static D4-S8R domain inventory accepted and frozen. Provisional v38 is **not allocated**. | Separate design decision for bounded atomic visibility/reader-writer safety, then exact Windows performance/FTS/history regression. Do not merge #383 or discard user states. |
| [#270](https://github.com/ArdenZC/Zen-Canvas/issues/270) | macOS Tahoe lifecycle | [PR #384](https://github.com/ArdenZC/Zen-Canvas/pull/384) test-only diagnostics: 16/16 native Hosted background launch aborts, including no-Tray control; precise exception root not captured. | Specialist native stack/cause isolation before production patch. No icon-only assumption. |
| [#353](https://github.com/ArdenZC/Zen-Canvas/issues/353) | Windows PrivateUsage resource trend | [PR #356](https://github.com/ArdenZC/Zen-Canvas/pull/356) diagnostic only. Historical HARD growth and later PASS both real, causal leak not proven. | Causal reproduction + allocator/registry/lifetime evidence; no threshold relaxation. |
| [#364](https://github.com/ArdenZC/Zen-Canvas/issues/364) | Global Search 500k high fanout | Candidate key-only SQL CTE helps selected name-prefix tiers; 500k pooled p95 remains over 100ms and cross-run variance unresolved. | Narrow paired SQL/pooled measurement; exclude regressing extension-prefix tier. No blanket production SQL edit. |
| [#370](https://github.com/ArdenZC/Zen-Canvas/issues/370) | ISG-00 low-value indexing economics | Static map and synthetic benchmark in [PR #373](https://github.com/ArdenZC/Zen-Canvas/pull/373); benchmark [#379](https://github.com/ArdenZC/Zen-Canvas/pull/379) merged. Synthetic 40% excluded corpus is designed, **not real-world savings**. | Reconcile docs and remaining native cost-evidence gaps; no live filter activation. |
| [#371](https://github.com/ArdenZC/Zen-Canvas/issues/371) | ISG-01 Index Scope design | [ADR proposal #372](https://github.com/ArdenZC/Zen-Canvas/pull/372) remains Proposed/Draft; not Accepted. Source/Policy/AI/Consent/Mutation authorities separate. | Owner revise ADR with frozen test semantics before approving policy. |
| [#382](https://github.com/ArdenZC/Zen-Canvas/issues/382) | ISG-02 policy Oracle | [PR #387](https://github.com/ArdenZC/Zen-Canvas/pull/387) OPEN/Draft; 16 contract tests in a standalone policy harness and Hosted CI passed, **Owner research evidence accepted** on Issue #382, no production Smart Lean. | Bounded Owner merge-worthiness/governance review; keep product default unchanged. |
| [#330](https://github.com/ArdenZC/Zen-Canvas/issues/330) | Owner multi-track reconciliation | OPEN Owner governance; older Issue and document inventory predates #329/#377/#378 merges. | Maintain this live ledger, resolve risk/ownership conflicts and correct STATUS/ROADMAP pointer; no omnibus refactor. |
| [#283](https://github.com/ArdenZC/Zen-Canvas/issues/283) | ZenDecisionBench/Preference | Offline evidence frozen at INCONCLUSIVE_LOW_DELTA; research paused. No Laya/Jev/online preference authority installed. | Revisit only on new evidence/product hypothesis; no default production task. |

## 4. Twelve unmerged Draft PRs — disposition and dependency

All 12 are **OPEN / Draft** on the snapshot date. None of their code should be described as master functionality.

| PR | Exact head prefix | Classification | Owner disposition |
| --- | --- | --- | --- |
| [#356](https://github.com/ArdenZC/Zen-Canvas/pull/356) | ab2acf3d | Memory diagnostic | Hold until #353 cause/evidence review; historical CI failure retained. |
| [#372](https://github.com/ArdenZC/Zen-Canvas/pull/372) | 50fdd57c | ADR proposal | Proposed only, require policy-contract amendment. |
| [#373](https://github.com/ArdenZC/Zen-Canvas/pull/373) | 8672c7cc | Cost research docs | Reconcile historical planning language and measured addendum. |
| [#383](https://github.com/ArdenZC/Zen-Canvas/pull/383) | 584621c3 | **Production candidate** | **DESIGN BLOCKED**: writer-liveness 100k & publication/data protection, v38 provisional, no independent merge. |
| [#384](https://github.com/ArdenZC/Zen-Canvas/pull/384) | e6a2426a | macOS diagnostic | Accepted experiment result does not identify repair; specialist review. |
| [#385](https://github.com/ArdenZC/Zen-Canvas/pull/385) | 8ab50cb6 | #380 D0 test-only | Preserve bounded model; no production repair. |
| [#386](https://github.com/ArdenZC/Zen-Canvas/pull/386) | d260131f | **D1-A production candidate** | Broad admission changes and uncovered Rebuild path; **independent MERGE NO-GO**. |
| [#387](https://github.com/ArdenZC/Zen-Canvas/pull/387) | 57ad28bf | ISG-02 test-only | Research evidence accepted; separate Owner merge decision. |
| [#388](https://github.com/ArdenZC/Zen-Canvas/pull/388) | 9e144da2 | Generation publication prototype | Existing FTS/Managed/AI/Library paths still outside authority; production NO-GO. |
| [#389](https://github.com/ArdenZC/Zen-Canvas/pull/389) | bbdd3db2 | Legacy upgrade K1 | Historical negative research; not current public legacy obligation. |
| [#390](https://github.com/ArdenZC/Zen-Canvas/pull/390) | e1e99389 | Legacy upgrade K2-T0 | Historical negative research; not a current development blocker. |
| [#391](https://github.com/ArdenZC/Zen-Canvas/pull/391) | 4f9f742d | Legacy upgrade G1 | Windows/macOS Hosted OS observations accepted; normal CI cancelled/aggregate failed; **MERGE HOLD** and not a current legacy requirement. |

The K2-related Draft PRs should **not** be merged merely to make the Issue list look smaller. Any archival/closure requires a separate Owner decision preserving the evidence; this ledger does not perform those mutations.

## 5. Dependencies and parallel engineering lanes

### Lane A — Source truth, database and scan correctness

1. #380 narrow trusted request-busy source-health repair, independent of #328 real SQLite lock; in authorized minimal Codex development scope. Do not opportunistically merge #386 D1-A.
2. #328 actual lock-owner identification from native evidence plus isolated test; no speculative global busy_timeout increase.
3. #381 atomic stale/reconcile publication design: avoid holding SQLite writer lock beyond 5s; protect file identity, user corrections/tags/history, FTS, Managed/AI/analysis, old-visible rows and crash recovery. PR #383 is an existing large candidate **blocked**, not a blank new task.
4. Coordinate provisional schema version only after a bounded implementation decision. Internal unreleased v37 does not require public backward compatibility, but a live latest-version user's committed truth still must not be silently destroyed.

### Lane B — OS-native and lifecycle stability

- #270 macOS exception root (including no-tray M1) needs genuine native stack information and Owner review.
- #353 Windows memory PrivateUsage causal verification, distinguish heap growth, allocator cache and leak; preserve HARD thresholds.
- Verify #329 completed-source first-run and watcher/Global Index native residuals separately without reopening its already accepted closed scope absent a new reproduction.

### Lane C — Resource-aware indexing and search

- ISG-00/01/02 measured baseline, proposed ADR and tested pure Oracle precede any production Smart Lean/default switch.
- Distinguish OS protected physical roots from ordinary user same-name directories, dependency/cache classifications, searchable metadata, Managed Scope, AI eligibility, content consent and mutation rights.
- #364 narrow high-fanout CTE requires pooled 500k repeatability; no unconditional global replacement.

### Lane D — Feature and architecture evolution

- Preserve primary product: Files Library + Browse, Quick Preview, Global Search, Organize/Cleanup review, Automation, History/Restore.
- Resolve TD-001/TD-015 compatibility ownership before expansion of Library tabs/split/shelf/notes, and TD-002/003/006/008/010 as bounded refactors with tests; do not block all feature design on generic technical-debt zero.
- System One/Laya/Jev and online preference are **not** production-approved. Future embedding strategy is a separate measured product decision with candidate-filter cost controls.

### Change-interference boundaries

- #380 Service/Coordinator vs #381 Managed Scan: separate PR scopes. Their designs may share DB-level state/FS correctness, but should not be force-merged into one patch.
- #381 provisional Schema v38 vs #380 broader experimental Schema v38: neither is allocated. No racing migrations.
- #381 Managed Scope vs #371 Global Index Scope: different producer/coverage authorities. No quiet global exclusion default change.
- Windows Service request busy vs SQLite BUSY(5): distinct causes and remedies.
- macOS tray abort vs generic package compatibility: a background lifecycle blocker is not disproved by compile success.
- Research PR workflow file changes trigger existing high-risk/full-performance routing: never waive missing gates, but avoid repeating heavyweight work without an explicit contract.

## 6. Decision ledger: next Owner stages (no calendar deadlines)

| Sequence | Deliverable | Exit rule |
| --- | --- | --- |
| A0 | This GitHub-live active-development ledger and top-level document pointer | Docs-only Draft PR, diff limited to docs; independent Owner review before merge |
| A1 | #380 narrow request-busy correction | Distinct tagged error, revision-safe source state, deterministic regression, exact-head CI; stop on unknown status ownership |
| A2 | #381 production architecture/lock-liveness disposition | Evidence-backed bounded transaction/visibility design and data-owner map, not merely SQL micro-optimization |
| B1 | #270 native crash cause | Captured actionable exception stack/trace or explicit environment diagnostic limit; no speculative icon-only fix |
| B2 | #353 memory trend cause | Repeatable metric/cause on frozen harness; zero threshold relaxation |
| C1 | #387 accepted research integration decision + ADR-0014 amendment | Preserves proposed vs accepted semantics; no default activation |
| C2 | #364 tier-scoped benchmark follow-up | Paired pooled results and adversarial ranking/FTS correctness |
| D1 | Architecture/feature backlog after critical truth contract stabilized | Bounded modules with source identity and ownership; no all-at-once refactor |

No arbitrary release date, RC gate, no mandatory pre-release legacy upgrading, and no claim that all remaining research must be merged. The Owner decides scheduling by severity, causal dependencies, user value and manageable PR interference.

## 7. How to update this ledger

For each issue and PR revision record: *entity/status, production-vs-test-only, current exact SHA/tree, latest substantive CI with timestamp, accepted evidence, unresolved counterexamples, owner next action, and dependency*. Do not mark a native environment test as passed when only Hosted synthetic/test-model work ran. Check GitHub again immediately before changing the queue; historical comments remain auditable but the latest verified snapshot supersedes obsolete phrases.
