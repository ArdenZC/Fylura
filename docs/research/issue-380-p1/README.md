# Issue #380 P1 — public error boundary scope blocker

**ISSUE_380_P1_OWNER_ESCALATION_REQUIRED**

**ISSUE_380_PRODUCT_FIX_NO_GO**

This is a bounded **diagnostic Draft**, not a P1 production candidate. The first
semantic boundary requires an Owner scope decision. No production behavior was
changed; only three characterization tests and their cfg(test) include were
added. No claim that P1 or #380 is fixed.

## Authority and source baseline

- [Owner production authorization](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6144080753).
- [D3 independent review](https://github.com/ArdenZC/Zen-Canvas/pull/397#issuecomment-6144062928).
- Branch: `fix/issue-380-p1-v3-ipc-admission-hardening`.
- Starting master: `3275ac7c72df8ae4139a30f16c7f649971324e97`.
- Starting Tree: `009576241cd4e8e2ba12cc644a368314f599faa6`.
- Current origin: `https://github.com/ArdenZC/Zen-Canvas.git`.

The Owner scope lists the four Windows IPC files and directly related tests,
then says: **“If complete P1 correctness requires expanding outside those
paths, stop at OWNER_ESCALATION_REQUIRED with smallest necessary scope and
tests; do not silently broaden.”** The user's execution brief additionally
requires escalation when the existing public error type cannot express the
required distinctions. This is that scope stop, not a new A/B/C research phase.

[Reference source manifest](reference-sources.json) records exact PR heads,
paths and SHA-256 for independently read #386, #393 and #397. No candidate
branch was merged, cherry-picked or used as the development baseline.

## First source blocker and actual failing contract probes

`coordinator.rs:19-29` defines the public `GlobalIndexError` with only:
Database, Provider(String), RebuildRequired(String), WindowsIo(u32), Paused.
`GlobalIndexProvider:67-90` requires that same type for its indexing methods.
`windows/mod.rs:299-312,406-455` returns it at the provider boundary.

Production `service_errors.rs:24-34` currently collapses the trusted busy and
permission error codes into Provider. Its server `classify:10-21` also maps an
ordinary Provider failure to permission. The public type supplies no typed
Service request rejection/outcome or permission variant. SQLite Database,
Paused, RebuildRequired and WindowsIo must not be repurposed as such a variant.

A Windows-local transport enum can distinguish NotSent, UnknownOutcome and
SinkFailure **inside the IPC loop**. That is viable and does not itself require
public scope expansion. It cannot preserve those distinctions after conversion
to the existing `GlobalIndexProvider -> GlobalIndexError` API: the remaining
Provider(String) carries only text. Recovering request classification from
message prefixes/contains would violate the contract. Writing a SourceState
as a side channel for an unadmitted busy request would also violate it.

The desired typed public-boundary assertions are genuinely executed against
the production decoder and verbatim production Rust definitions. Results:

| Probe input | Actual current class | Desired boundary contract | Result |
| --- | --- | --- | --- |
| Existing `index_service_busy` | Provider | Request-level ServiceBusy, distinct from Provider | FAIL |
| Existing `index_permission_required` | Provider | PermissionRequired, distinct from Provider | FAIL |
| Unrecognized/proposed `index_outcome_unknown` | Provider | Conservative distinguishable unclassified failure | FAIL |

The third code exists in #386's proposal, **not master's accepted code set**.
It is an adversarial unknown-code decoder probe, not evidence of a real
transport UnknownOutcome or a trusted native Service emission. In a repaired
P1 it must first be rejected/classified by the strict code/identity gate.
Actual transport interruption and native runtime tests were NOT_RUN here.
The first two probes establish the public busy/permission erasure directly.

- [Actual required-contract log](evidence/required-contract.log): **0 pass /
  3 fail**, Cargo exit **101**. No ignored tests or weakened assertions.
- [Characterization log](evidence/characterization.log): **3 pass**, confirming
  the old collisions and generating real SQLite lock contention.
- Real isolated SQLite contender returns BUSY **5**, remains
  `GlobalIndexError::Database`; Service busy is currently Provider. Bundled
  SQLite measured by this Rust fixture: **3.51.3**. No busy_timeout change to
  production or any real database.
- [Environment, commands, source/fixture hashes](evidence/probe.json).

The launcher exit 0 means **the expected scope blocker was reproduced**;
it does not mean the three required repaired-contract assertions passed.
The characterization tests passing likewise characterize the defect.

## Smallest Owner decision

Authorize only a typed public error carrier in
`src-tauri/src/global_index/coordinator.rs::GlobalIndexError`, plus necessary
exhaustive error matches. A concrete minimal option is three variants:
`ServiceBusy`, `PermissionRequired(String)` and
`ServiceOutcomeUnknown(String)`, or one typed ServiceRequest wrapper carrying
those reasons. No changes to provider trait signatures, scheduler, source
health writes, Rebuild logic, repository SQL or schema are needed merely for
this type extension. Existing Database retains local SQLite/extended codes;
NotSent can remain a Windows-local transport type. A separate DatabaseBusy
string wrapper is not required for genuine local BUSY(5).

**This proposal is not implemented or authorized by this Draft.** In particular,
no #386 Coordinator busy `continue`/rollback branch is copied. Such state
behavior needs P2 owner/revision/intent authority, not this type extension.
If Owner instead intends P1 distinctions to end at the internal IPC enum and
accepts their erasure at the public boundary, that narrower acceptance must be
explicit: it leaves downstream request-versus-source classification unresolved.

## Source reconciliation with #386

Reference HEAD `d260131f9d6ec354f2361e6c7460c79e62e38391`, normal
[CI 38161607367](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38161607367)
SUCCESS; still unmerged Draft. Prior CI does not qualify this branch.

| Prior design | P1 disposition |
| --- | --- |
| Local InvalidRequest / NotSent / MissingAdmission / UnknownOutcome / SinkFailure | Sound direction to reimplement after scope decision; not copied here |
| GlobalIndexError ServiceBusy / PermissionRequired / ServiceOutcomeUnknown | Exact outside-scope dependency; needs the narrow public-type authorization above |
| Sources marker after Service busy reservation/operation/source checks | Source-read; would need actual runtime/client ordering tests; not implemented/qualified here |
| SourceProvider + volume-stale + Checkpoint full/Rebuild completion, provider + Checkpoint incremental completion | Source-read; zero-event and empty-volume runtime proof remains pending, no false READY claim |
| `EntryStale { volume_id, entry_id }` in #386 service.rs | **Reject direct reuse:** master v3 only has entry_id; no pretend existing volume field |
| Checkpoint cursor | Keep actual `Option<String>`; no integer assumption |
| Request/lookup binding and no replay after partial stream | Necessary directions; full production regressions not executed here |
| Coordinator busy skip/recovery changes | Outside P1 and insufficient for ownership; not copied |

Current USN Stale path (`windows/usn.rs:135-141`) first performs a
source-scoped `find_entry_by_identity` and then uses its returned entry ID.
A future allowed adapter can investigate that per-request read provenance to
validate legacy EntryStale without inventing a volume field. This is a source
observation, not a completed replay/lookup safety proof or a demand for schema.

## Validation and qualification limits

Run the reproducible offline source probe:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/issue-380-p1/run_error_boundary_probe.py --output .tmp-tests/p1/review --fixture .tmp-tests/p1/review-fixture
```

The launcher extracts production DbError, GlobalIndexError,
IndexServiceResponse, protocol and status constants verbatim with hashes, and
includes actual production service_errors.rs and the committed test module.
It does not substitute mock error or response types. This is a reduced Linux
Rust crate, not the full desktop, Service Runtime, codec/request loop or native
SCM qualification. Its additional three required-contract tests intentionally
fail; raw output is preserved.

Full-project Linux targeted Rust tests are blocked before execution by missing
`glib-2.0 >=2.70` pkg-config dependencies:
[command/exit](evidence/full-project-local.json),
[original log](evidence/full-project-local.log). No system packages or CI gates
were changed. Initial format check identified one new-test formatting issue;
standard cargo fmt corrected it, and final format check passes.

Published source-run HEAD/Tree are stated in probe.json. The final delivery
repeats the probe on the final PR HEAD and records source-hash equality and
exact identities in the PR body. Natural CI runs the committed Windows
characterization tests where routed; its exact-head source and same-tree
merge-integration evidence are recorded separately. Native qualification is
reported only if actually run. Green diagnostic CI is not P1 acceptance.

## Uncompleted P1 acceptance and P2/P3 dependencies

| Required P1 regression | Status |
| --- | --- |
| Busy before any Sink writer | NOT_IMPLEMENTED / NOT_RUN runtime test |
| Request ID and protocol version checks | NOT_IMPLEMENTED / NOT_RUN |
| Cross-source Entries/legacy EntryStale/Volume states | NOT_IMPLEMENTED / NOT_RUN |
| Missing/duplicate Admission and event ordering | NOT_IMPLEMENTED / NOT_RUN |
| True SourceState permission/rebuild/provider errors | Public taxonomy collision reproduced; runtime repair NOT_DONE |
| Legal Initial/Rebuild completion | NOT_IMPLEMENTED / NOT_RUN |
| Empty Rebuild vs empty Incremental | NOT_IMPLEMENTED / NOT_RUN; no completion claim |
| Partial events then disconnect | NOT_IMPLEMENTED / NOT_RUN |
| Local SQLite vs remote Service busy | Real local BUSY distinction characterized; full client SinkFailure path NOT_RUN |
| Lookup ID duplicate/mix/replay | NOT_IMPLEMENTED / NOT_RUN |
| No Direct replay after rejection/unknown | NOT_IMPLEMENTED / NOT_RUN |
| Legitimate Direct no-effect proof | NOT_IMPLEMENTED / NOT_RUN; ultimate DB ownership is P2 |
| Reject before unauthorized Sink side effects | NOT_IMPLEMENTED / NOT_RUN |
| Existing normal v3 flows | Existing CI only; no new P1 repaired-behavior qualification |

All original tests remain; #393 diagnostics are unchanged. No claims of fixing
Coordinator public Rebuild pre-admission health/stale writes, independently
durable Rebuild Intent, source/operation revision CAS, atomic Global/Managed/AI
publication or complete crash/restart recovery. Native Windows long-term
Service/SCM experience and #328 SQLite lock competition remain unresolved.
No schema v38, IPC v4, release, installer, ranking/FTS, watcher or #381 changes.

Stop at an **OPEN/Draft diagnostic PR** for the smallest Owner scope decision.
No production implementation continues, no merge/close/master mutation or
Codex Review occurs. Final status remains
**ISSUE_380_P1_OWNER_ESCALATION_REQUIRED / ISSUE_380_PRODUCT_FIX_NO_GO**.
