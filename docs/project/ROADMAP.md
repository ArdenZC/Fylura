# Zen Canvas Roadmap

> **Owner scope correction (2026-10-13): the product remains in ongoing development, NOT release preparation.** Current authoritative engineering queue and cross-track dependency register: [Active Development Decision Ledger — 2026-10-13](tasks/ACTIVE-DEVELOPMENT-LEDGER-2026-10-13.md). Older W6/RC/release sequencing below is retained for history, not activated now. Internal pre-public-release legacy-upgrade K1/K2/G1 research is not a blocker; core runtime correctness, macOS/Windows native stability, Index Scope, measured performance and feature maturity remain normal development work.

The roadmap records authorized sequencing and current execution truth. Long-horizon direction remains owned by [`MASTER_DEVELOPMENT_PLAN.md`](MASTER_DEVELOPMENT_PLAN.md).

Last verified: 2026-10-05

## Completed

### G1 — Engineering OS

**COMPLETE.** Project-state, architecture-ownership, technical-debt, workflow and closeout rules are durable.

### M1 / M1.1 — Mutation correctness and portability closeout

**COMPLETE.** Mutation correctness, provider and portability remediation are closed at their reviewed baselines.

### W0 — File Library / Preview specification

**COMPLETE.** W0 froze the Library/Browse product model, identity contracts, Preview Core/Host boundaries, Read/Materialization and WorkScheduler ownership, performance gates and Wave sequencing.

### W1 — File Library / Preview Foundation

**COMPLETE.** Shared runtime foundation delivered.

### W2 — File Library 2.0 Experience

**COMPLETE / CLOSED.** Authority: [W2 initiative](initiatives/W2-file-library-experience.md) and [W2-12 closeout](tasks/W2-12-FILE-LIBRARY-EXPERIENCE-CLOSEOUT-RESULT.md).

### W3 — Preview Platform

**COMPLETE / CLOSED.** Existing Preview Core/Host remains the authority for later experience work.

### W4 — Native Integration

**COMPLETE / CLOSED.** Final closeout: [W4 final current truth](tasks/W4-NATIVE-INTEGRATION-FINAL-CURRENT-TRUTH.md).

### TD-014 — Cleanup Ledger Physical Identity Normalization

**COMPLETE / CLOSED.**

### W5 — Release / Hardening

**COMPLETE / CLOSED.** Technical release qualification and packaging readiness were established; historical manual/native release-path evidence remains explicitly unverified where recorded.

### W6-01 — Product Maturity Audit

**COMPLETE.** Public release was not recommended; maturity work was required.

### W6-02 — First Value & Recovery Maturity

**COMPLETE / MERGED.**

### W6-03 — Product Hierarchy & Progressive Disclosure

**COMPLETE / MERGED.**

### W6-04 — File Library Calm-Surface Review / Bounded Remediation

**COMPLETE / CLOSED.** Production remediation and focused native evidence closed the bounded Filter popover P2 while preserving Query/Library/Browse/Preview authority.

### W6-05 — Whole-Product Native Experience Audit

**COMPLETE / CLOSED.** Accepted result/evidence squash merge: `master@507253589c2bbc9924f643ddd38456e2716138dd` (#199).

Final product audit outcome: **DEGRADED**.

Final matrix: `PASS 45 / FAIL 6 / DEGRADED 7 / UNVERIFIED 22 / total 80`.

Finding severity: `P0=0 / P1=0 / P2=5 / P3=0`.

Final retained evidence ZIP SHA-256: `0659F2BAEF45666D9380C623B179B9513D5643281B21B0B0411824D2EC0EFDA3`.

### W6-06 — Zen Visual System & UX Redesign

**COMPLETE / CLOSED — V26 TARGET DESIGN FROZEN.**

Final freeze squash merge / W6-07 activation baseline: `master@6b435dbf49c609a95a4d95935090825f003e7a5d` (#206).

Final owner freeze score: **93.4 / 100**; retained freeze threshold: **92 / 100**.

Final authority:

- [Final Design Freeze Audit](tasks/W6-06-FINAL-DESIGN-FREEZE-AUDIT.md)
- [Design Freeze Closeout Result](tasks/W6-06-DESIGN-FREEZE-CLOSEOUT-RESULT.md)
- [V26 Freeze Manifest](../design/w6-06/07-V26-FREEZE-MANIFEST.md)
- [W6-07 Implementation Handoff](../design/w6-06/07-W6-07-IMPLEMENTATION-HANDOFF.md)

## Current

### Post-#323 Historical Audit Residual & Technical Debt Reconciliation — Owner Gate

Status: **ACTIVE — SPECIFICATION ONLY / OWNER GOVERNANCE; NO PRODUCTION IMPLEMENTATION AUTHORIZED.**

Authority: [initiative](initiatives/post-323-audit-residual-technical-debt-reconciliation.md), [issue #330](https://github.com/ArdenZC/Zen-Canvas/issues/330) and [Owner reconciliation matrix](tasks/POST-323-AUDIT-RESIDUAL-TECH-DEBT-RECONCILIATION-RESULT.md).

Accepted starting point: `master@1619e335468c432da5a5b3a6e246e1ea1c7db32c` / tree `70c1e97eb68a01506a796140277af640a5e0fe60`; merge-after master CI [37722150775](https://github.com/ArdenZC/Zen-Canvas/actions/runs/37722150775) SUCCESS.

Current minimum sequence:

1. Truth/matrix reconciliation.
2. #329 clean-install Onboarding scan-scope persistence — **FIX BEFORE REBASELINE**.
3. #328 transient SQLite lock/unavailable — **FIX BEFORE REBASELINE**.
4. Reconcile historical Global Index zero-source residual against #328 evidence.
5. Freeze [ADR-0012 / #332](DECISIONS/0012-files-authority-and-compatibility-retirement.md) for TD-001 + TD-015 before new File Workspace implementation.
6. Freeze [ADR-0013 / #333](DECISIONS/0013-managed-ai-eligibility-and-queue-authority.md) for TD-006 before AI Eligibility implementation.
7. Activate Post-AI-only Product / Architecture Rebaseline only after the minimum blockers close.

#270 remains a separate macOS compatibility blocker for supported macOS resident/release claims. #283 research remains PAUSED at `INCONCLUSIVE_LOW_DELTA`. Release publication remains deferred.

## Completed Windows Global Index Recovery — #323

**COMPLETE / MERGED / CLOSED.** PR #327 squash-merged as `master@1619e335468c432da5a5b3a6e246e1ea1c7db32c` / tree `70c1e97eb68a01506a796140277af640a5e0fe60`; merge-after master CI 37722150775 is SUCCESS. Scoped Windows Paused/automatic-rebuild recovery is Owner accepted with explicit evidence exceptions; historical native qualification remains INCOMPLETE. #328 and #329 are separate OPEN defects.

## Resident / Interactive Performance Qualification

Status: **COMPLETE / MERGED — owner performance review passed; PR #269 squash-merged to `master@cc8870f63bd83033f3c7b79afa96bf3ce11821f7`; Windows TARGET deviation accepted; macOS resident remains UNVERIFIED / tracked by #270**.

Authority: [Resident / Interactive Performance Qualification initiative](initiatives/resident-interactive-performance-qualification.md) and [activation taskbook](tasks/ZB-RESIDENT-INTERACTIVE-PERFORMANCE-QUALIFICATION-ACTIVATION.md).

The completed foundation sequence is:

1. [ZB-01 — Database Resident Footprint](tasks/ZB-01-DATABASE-RESIDENT-FOOTPRINT-RESULT.md) — **MERGED**.
2. [ZB-02 — Idle Polling Removal](tasks/ZB-02-IDLE-POLLING-REMOVAL-RESULT.md) — **MERGED**.
3. [ZB-03 — Runtime Resource Governance](tasks/ZB-03-RUNTIME-RESOURCE-GOVERNANCE-RESULT.md) — **MERGED**.
4. [ZB-04 — On-Demand UI Runtime](tasks/ZB-04-ON-DEMAND-UI-RUNTIME-RESULT.md) — **MERGED**.
5. [ZB-05 — Native Global Search Runtime](tasks/ZB-05-NATIVE-GLOBAL-SEARCH-RUNTIME-RESULT.md) — **MERGED** through PR #266.
6. [Zero-Burden Cross-Track Audit Remediation](initiatives/zero-burden-cross-track-audit-remediation.md) — **COMPLETE / MERGED** through PR #267 to `master@6d38208741d186988468ec92b632dd3669a03aa5`.
7. **Resident / Interactive Performance Qualification — COMPLETE / MERGED.**
8. **AI Semantic Authority Foundation — CURRENT / OWNER-APPROVED FOR MERGE.**

Qualification established attributable Windows resident-process evidence and credible interactive foreground evidence. The one-CPU traversal repair keeps filesystem work on the scanner worker already carrying background QoS. Owner performance review accepts the remaining Windows <=2x scheduler-interference TARGET deviation without promoting it to HARD; broader production tuning is not supported by the non-monotonic causal matrix. Existing thresholds are not relaxed, historical misses remain in evidence, and no new cross-platform absolute RSS cap is invented. macOS resident startup remains unverified under #270. AI Semantic Authority is next after PR #269 merges.

## Release qualification residuals — W6-10

W6-10A / RC1 remains **COMPLETE / FROZEN**. W6-10B remains **BLOCKED** because B02 SmartScreen and B03 Unknown Publisher/UAC evidence require a valid evidence host. W6-10C remains **DEFERRED / UNVERIFIED — OWNER SKIPPED** without a supported Apple Silicon Mac. Full supported-platform release PASS is **NOT CLAIMED**; RC2 is not authorized; publication remains deferred. See the [W6-10C disposition](tasks/W6-10C-MACOS-RELEASE-QUALIFICATION-DISPOSITION.md) and [W6-10B result](tasks/W6-10B-R2-WINDOWS-RC1-CLEAN-PROFILE-REQUALIFICATION-RESULT.md).

## Current visual authority — Solid / Calm Demo V2

The owner-approved [Solid / Calm Demo V2 freeze](../design/w6-09/SOLID-CALM-V2-FREEZE-MANIFEST.md) remains canonical for material and presentation. V26 remains a structural, navigation, information-architecture and unsuperseded-interaction reference. Liquid Glass is revoked. This visual authority is independent of the current engineering initiative.

## Later planned maturity sequence

Each later Track requires its own authority.

### W6-08 — Cross-Platform Quick Preview Experience

**COMPLETE through PR #240 — Issue #239.** The bounded improvement of the
existing first-party Preview experience using current `ZenFloatingQuickPreview`
/ Preview Core architecture closed the repository/browser presentation gap.
The typed/folder residual is explicitly **ACCEPTED DEFER** for exact-head native
verification in W6-09. Explorer Preview Handler remains supplementary shell
integration. Browser PASS != Native PASS.

### Residual Product Defect Closure Gate — before W6-09

Before W6-09 starts, each retained W6-05 residual area must have one explicit
owner-reviewed disposition. The gate is a closure/disposition requirement,
not a new remediation Track or an invitation to reopen every historical bug.

| Retained residual area | Required disposition before W6-09 |
| --- | --- |
| Cleanup extended-path rejection | **CLOSED / FIXED** |
| Typed/folder Quick Preview gaps | **ACCEPTED DEFER** — W6-08 implementation and browser/integration evidence close the known presentation/support gap at repository level; exact-head Windows/macOS native Quick Preview UI re-verification is carried into W6-09 Whole-Product Native Regression, where a reproduced Preview defect may receive bounded native correction and re-verification |
| Global Index unavailable/zero-source state | **ACCEPTED DEFER** — exact-head native source/state re-evaluation remains open |
| Organization Plan safe-preview degradation | **ENVIRONMENT-SPECIFIC** — reproduce on supported native fixtures and preserve fail-closed behavior |
| Browse / first-scan recovery friction | **ACCEPTED DEFER** — exact-head native first-launch/restart recovery re-evaluation remains open |

The disposition must link to the evidence and owner decision that supports it;
rediscovery without disposition does not satisfy the gate. The entry
dispositions above are not release waivers. W6-08 is complete through PR #240;
the typed/folder Preview residual remains **ACCEPTED DEFER** for exact-head
native verification: Browser PASS != Native PASS.

### W6-09 — Whole-Product Native Regression

**COMPLETE / CLOSED / MERGED — OWNER WINDOWS NATIVE PRODUCT ACCEPTANCE PASS; SOLID / CALM WINDOWS VISUAL ACCEPTANCE PASS; FULL SUPPORTED-PLATFORM NATIVE PASS NOT CLAIMED — Issue #241 closed; PR #242 squash-merged to `master@164608f90b8233303fefcf9660822daea0ecb857`.**

Accepted production functional baseline: `88fc663392371049fda2d71b85bd4815d073bfe0` / tree `5ca511f055b02bb511bc0873ffc5c2a2d26efadf`. First-entry Browse, admitted ephemeral-session truth, Quick Preview PDF/Markdown/image/pinned behavior and relevant authority boundaries are accepted. Fresh hosted production-head CI 35608830144 is **SUCCESS**.

Production source for the presentation migration is `8fd246476e025636d4606a44d23688865ab89cc2` / tree `475a8d0e0295abbe37b3afa115738fa3602785e7`; fresh hosted CI 35685110417 is **SUCCESS**.

Current Windows native captures are accepted functional/native regression evidence, and Owner Windows exact-head Solid / Calm visual acceptance is **PASS**. The evidence is retained externally at `F:\CargoTarget\w6-09-solid-calm-owner-review-00787888\windows\` with ZIP `F:\CargoTarget\w6-09-solid-calm-owner-review-00787888.zip` and SHA-256 `43236C3E82ACF409261436E598EBE191EA048715CCCB2832C84795C705F0BEF8`. Full supported-platform native PASS is **NOT CLAIMED**. Windows DPI/scaling, Forced Colors, real macOS GUI/Retina, release-path/release-binary acceptance, Narrator/VoiceOver, native Reduced Motion and remaining lifecycle/mutation residuals are explicitly accepted for W6-10. See the [W6-09 closeout result](tasks/W6-09-WHOLE-PRODUCT-NATIVE-REGRESSION-CLOSEOUT-RESULT.md).

W6-09 activation baseline: `master@20781c8dc4dc8f24f0ed7d2ce860f5fd62d35ec9`; tree `2535499a23be61786543bab19c71e35ee7a1d36f`.

### W6-10 — Release Re-entry

**W6-10B BLOCKED — SECURITY PRESENTATION EVIDENCE HOST REQUIRED.** R2 self-state gates passed, but B02 SmartScreen and B03 Unknown Publisher/UAC remain UNVERIFIED on available hosts. **W6-10C is DEFERRED / UNVERIFIED — OWNER SKIPPED** because no real supported Apple Silicon Mac is available; see [`W6-10C-MACOS-RELEASE-QUALIFICATION-DISPOSITION.md`](tasks/W6-10C-MACOS-RELEASE-QUALIFICATION-DISPOSITION.md). No RC2 is authorized. Full supported-platform PASS is not claimed; publication remains deferred.

## Publication disposition

[v0.1.40 publication action](tasks/RELEASE-0.1.40-PUBLICATION-ACTION.md) remains:

> **DEFERRED — PRODUCT MATURITY NOT YET ACCEPTED / DO NOT EXECUTE**

No tag or GitHub Release may be created while this deferral is active.

## Explicit non-goals for maturity work

Do not solve maturity by adding updater infrastructure, OCR/RAG/plugin/agent breadth, another Preview engine, another AI feature, new durable authorities, or weaker AI/filesystem safety gates.

The objective is to make the existing product coherent, attractive, understandable and trustworthy while preserving the engineering strengths already built.

## W6 execution principle

> **Product reconstruction is the mainline. Technical debt blocks W6 only when it threatens correctness, authority, supported-platform evidence, or release gates.**
