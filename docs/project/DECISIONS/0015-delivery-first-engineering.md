# ADR-0015: Delivery-First Engineering and Proportionate Review

Status: proposed — effective only after Owner review and merge

Date: 2026-10-15

## Context and concrete failure mode

Zen Canvas has a substantial, functioning desktop product (Files, Search,
Preview, Organization, History/Restore, AI-only migration), but recent work
has devoted disproportionate attention to repeated research contracts,
separate probe branches, revalidation receipts and review cycles. A
representative example is the T10-B04 NTFS probe under Issue #411: useful
source-level work and an R1 correction were completed and independently
verified, yet native execution remained unqualified owing to runner privilege
and environment constraints. The probe is a valid research artifact; its
unfinished qualification should not become a blanket prerequisite for
unrelated File Library or UI improvements.

This example establishes **process friction**, not a measured claim that
90% of time or tokens were spent on a particular activity. The project did
not maintain consistent time/throughput measurements for that percentage.
The goal is to remove needless work **without** concealing genuine data-loss
and release risks.

ADR-0007 already prescribes proportional validation and failure handling,
but did not prevent escalation by accretion of task-specific contracts.
The missing policy is **when to stop researching and ship**, who owns the
decision and what everyday review evidence is sufficient.

## Decision

1. **Default to delivering a useful change.** For a bounded ordinary
   enhancement or fix that preserves durable authority, the minimal path is
   a small outcome/acceptance brief → Codex Cloud implementation and focused
   regressions → applicable existing CI → one substantive Owner review →
   Owner-authorized merge. Do not force research, spec freeze, separate PRs
   or independent native proof when not relevant to the change.
2. **Make extra work decision-bearing.** Before creating a special harness,
   new branch/Issue, bespoke log/manifest, second review or re-running a broad
   test suite, state the unresolved claim and how the result would change
   the decision. Otherwise use the existing source, tests and CI.
3. **One coherent PR; correct what is real.** Review feedback should cite
   a reproducible problem, credible harm, affected code and the minimal
   fix. Apply corrections on the same PR, and recheck the changed region.
   Review round labels and separate audit chains are not achievements.
4. **Limit blockers to their actual harm.** A failure in native durability,
   privileged disk work, high-risk mutation or release proof blocks that
   *claim*; it does not block independent reversible features. Distinguish
   observed bugs, model-only outcomes, unverified risks and deliberately
   deferred tests. Never relabel an unrun scenario as PASS.
5. **Git/CI are the default provenance.** Ordinary changes do not need
   duplicate SHA-256 lists or independent rehash of every text/log. Keep
   exact source/merge-tree semantics and existing required CI; preserve
   stronger artifact/native provenance when identity is genuinely material.
6. **Keep the irreducible safety floor.** No automatic file deletion,
   data-destructive migration, lost user-owned facts, unverified execution
   identity, bypassed consent, expanded privileges, relaxed CI threshold or
   unsafe release for convenience. These changes retain scoped design,
   negative tests, rollback/recovery and explicit Owner authorization.
7. **Optimize for throughput and quality together.** In a brief weekly
   review, discuss observable user-facing merged changes, elapsed time to
   integrate, waiting/rework causes and regressions. Do not count
   documentation, gate labels or PR volume as user value; merging is not
   deployment/release. Adjust process if verification overwhelms delivery,
   without creating another expensive dashboard or mandatory reporting gate.
8. **Ownership:** ChatGPT/Owner defines product scope, research priorities
   and reviews; Codex Cloud implements code/tests; the Windows native host
   remains acceptance-only; no Codex Review; merge stays an Owner decision.

## Immediate application to the current project backlog

This decision is **retroactive to work that remains open when merged**.
Already-open research/test PRs do not become product prerequisites by mere
age or accumulated audit reports. Classify the **next action**, not the
history: ship a bounded feature/fix; resolve a credible direct safety
dependency; or park evidence without further work. Existing Issue/PR
records and accepted test results remain accessible.

For example, do not keep expanding the already source-accepted but
environment-blocked T10-B04 study unless a specific near-term production
decision depends on it. Do not attempt to merge a historically blocked
large production candidate such as #383 merely to reduce the open-PR count;
instead define the smallest safe user-facing correction against current
master. And do not merge #408 test-only research as if it activated F1.

No blanket deletion, auto-close, automatic Draft-PR merge, source rewrite,
CI bypass, or new compulsory backlog-audit project is authorized. When an
old item is revisited, update its **existing** discussion with an actual
decision only if needed. The existing product-specific safety and required
gate boundaries remain applicable to the features that rely on them.

## Task trigger examples

- Add a file note display/edit field reusing approved persistence: implement
  in the existing module, test basic save/reopen/error, use current CI.
  No new audit branch or source-by-source SHA manifest.
- Fix an ordinary name-based false exclusion in Managed Scan: preserve the
  existing protected OS/dependency-folder boundaries, add targeted
  adversarial regression cases and normal CI. Do not first demand a complete
  native disk fault-lifecycle experiment.
- Add an opt-in elevated volume operation or destructive schema migration:
  **not** an ordinary change; obtain a separate scoped plan and reversible
  safety proof, with existing contracts unchanged.
- An unqualified Windows directory flush test: retain the actual OS5
  observation and mark native durability `UNVERIFIED`; do not spend another
  day on a new probe unless the result gates a specific imminent safe
  product decision. User-requested real ENOSPC and physical power-loss
  remain deferred.

## Success criteria and trial

For the next two weekly cycles after this decision is merged, compare:
**(a)** useful feature/fix PRs merged, **(b)** median issue-to-merge elapsed
time where known, **(c)** avoidable second-review/duplicate-CI work,
**(d)** reopened regressions and actual escaped defects, and **(e)** time
blocked by relevant dependencies. The objective is a **lower amount of
non-decision-bearing work** and higher useful delivery, without a rising
pattern of regressions. These are a lightweight retrospective checklist,
not new mandatory dashboards, percentages, issue labels, check contexts or
numerical acceptance gates.

The standard development workflow itself is the operative handbook.
`AGENTS.md` points agents to it. Do **not** create a parallel permanent
process authority or rewrite the historical taskbooks to mimic this
decision. Any genuinely conflicting older *current* brief should be
updated when that work is resumed, not preemptively bulk-rewritten.

## Relationship to existing authority

- **ADR-0001:** `STATUS.md` remains the only source of current project
  phase; this decision makes no new active-status ledger.
- **ADR-0004:** head validation, merge integration, tree equivalence and
  required CI evidence are **unchanged**.
- **ADR-0007:** proportional validation and truthful failure handling are
  reinforced, **not weakened**. This decision provides an explicit
  default delivery path and review/complexity budget.
- **Security/identity/filesystem/permission/migration/release contracts:**
  unchanged and stronger when triggered by credible risk.
- **ADR-0014 Index Scope proposal:** an independent, unmerged design on a
  different topic; this ADR reserves 0015 for delivery process and does
  not implement Index Scope.

## External references (directional, not project-specific proof)

- DORA software delivery metrics and value-stream mapping:
  https://dora.dev/guides/dora-metrics/ and
  https://dora.dev/guides/value-stream-management/
- Google SRE Workbook, Eliminating Toil:
  https://sre.google/workbook/eliminating-toil/
- GitHub required-check behavior:
  https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks

## Acceptance

Proposed for Owner review as a documentation-only PR. It becomes binding
repository policy only when merged into `master`. No code/CI/workflow,
release/Schema, PR-merging permissions or existing required gates change
as part of this decision.
