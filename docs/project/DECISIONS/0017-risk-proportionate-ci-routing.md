# ADR-0017: Risk-Proportionate CI Routing and Evidence Tiers

**Status:** Owner approved direction on 2026-11-01; effective only upon merge.
**Scope:** Engineering CI governance and acceptance meaning. No change to
filesystem mutation, persistence authority, source permissions, benchmark
budgets, GitHub branch protection, or product release status.

## Context

On `master@c2e26672923526e6a29da3ea3b5fd317301e125a`,
`classifyCiChanges.mjs` routes **any** `.github/workflows/**` edit into
all six 100k performance suites. An exact-head PR affecting query-only
Global Search code also runs a full Windows Global Index service
pause/recovery qualification. Both can consume costly CI and propagate
unrelated platform failures to the four protected status checks.

The earlier delivery-first rules already require proportionate review, but
the automated CI policy does not fully implement them. Recent examples:
- #525's workflow-only improvement was blocked by a strict Workspace
  Foundation memory trend, despite no app code change.
- #518's Windows source authorization → index → query OFF/ON → revoke →
  old-ID activation denial → restart GUI acceptance passed, but a Windows
  service Pause failure and five monotonically increasing macOS RSS values
  (net 80 KiB) stopped the integration. These failures are **not** proven
  harmless; their cause remains open.

## Decision

1. Keep the existing protected status checks, Quality aggregation, source
   checkout/merge-tree verification and fail-closed behavior for missing
   base or unknown production owners.
2. Route CI configuration and classifier-only changes through existing
   governance/frontend validation and relevant workflow contracts, **not**
   automatically through all six performance domains or Windows native
   Global Index service admission.
3. Only exact allowlisted pure query/relevance files are exempted from the
   Windows service qualification. New `global_index/**` owners,
   `repository.rs`, `coordinator.rs`, scope authorization, Windows
   providers and shared engine changes remain protected. Mixed changes
   select the union of relevant suites.
4. Distinguish `MERGE_SAFE_DEFAULT_OFF`, `ACTIVATION_NOT_READY`, and
   `RELEASE_NOT_READY` in Owner reports. Evidence classes `PASS`,
   `FAIL`, `UNVERIFIED`, `INCONCLUSIVE` do not weaken any existing
   mandatory CI job by themselves.
5. Actual relaxation/replacement of unstable resource sampling, Windows
   Pause semantics, or Quality aggregation is **not** part of this change.
   An independent measurement baseline and applicable targeted/native tests
   must precede any gate change. Do not relabel missing native capability as
   successful product qualification.
6. Use existing CI-metrics and GitHub records for two review cycles; compare
   useful merged fixes, avoidable reruns, first-pass failures and escaped
   defects. Roll back routing changes if a concrete missed regression occurs.

## Boundaries retained

File identity, consent, authorized sources, revoke-to-zero, durable journaling
and restore, safe-trash/AI advisory separation, schema migration,
rollback/recovery and cross-window permissions remain **hard** safety
contracts. An unknown/mixed native change cannot be justified by the
presence of harmless query edits. A default-OFF compiled experiment cannot
be activated or publicly released without its separate native/product
qualification.

## Implementation and validation

- `scripts/classifyCiChanges.mjs`: conservative path ownership routing.
- `tests/ciRoutingContract.test.ts`: positives for native owner and
  negative allowlist for query-only/control-plane changes; required status
  check names and normal Quality aggregation remain present.
- `docs/project/DEVELOPMENT_WORKFLOW.md`: documents existing three-layer
  verification semantics without introducing another approval queue.
- CI on exact PR head and required protected status checks must be green
  before merge. No feature activation, model/data migrations or release.

**Rollback:** restore the classifier and corresponding contract tests to
their prior all-domains routing without weakening security gates.
