# Issue #380 P3 — full-rebuild publication requires a persistence change

Disposition: `READY_FOR_OWNER_REVIEW / PRODUCTION_STOPPED / SCHEMA_42_REQUESTED`.
This is the stop-condition delivery requested by the Owner, not a production
repair or an allocation of Schema 42. No migration, runtime, test, CI routing,
Managed Library SourceUID/F1 file or activation permission is changed.

## Verified baseline and bounded outcome

GitHub and `origin/master` were checked on 2026-10-17:

- BASE: `1cc12833acb43319cdb6806a1b7109bacfdb4c25`.
- BASE tree: `d614c2520ad163f177de503800a82745d3e8d266`.
- Schema: 41. `db/schema.rs:970–999` retains Global Index ownership from
  Schema 39; versions 40/41 concern Managed Library persistence/indexes.
- [Baseline CI 38936074497](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38936074497)
  completed successfully at this BASE; it is not P3 qualification.
- GitHub confirms #398, #419 and #421 merged as `e2bf14404974fc0b5f6b5e15b7befd9c754a3e75`,
  `c08bf344aceecbe830faf75bf4610c98b9f722e1` and
  `5e25cbedc38fca085270626d619a48c4b08a97df`. Their admission, ownership,
  IMMEDIATE batch and fixture work must be retained.

The existing design provides atomic **individual writes**, not atomic corpus
publication. Preserving a usable old corpus while building a new one needs
durable versions of overwritten entries and derived state. Alternatively,
withholding an incomplete corpus across every consumer needs a persistent,
source-specific publication barrier and coordinated consumer/writer changes.
Neither is a bounded Coordinator-only repair under the existing contract.
The task's instruction 4 therefore stops production edits here.

## Exact visibility and recovery boundaries

All references below are repository-relative at BASE, not historical PR code.

| Boundary | Existing production behavior | Consequence |
| --- | --- | --- |
| Intent | `global_index/coordinator.rs:261`, `repository.rs:368`: coalesce durable intent without staling entries | Correct pre-admission behavior; retain it |
| Admission | `repository.rs:424–550`: short IMMEDIATE claim compares identity/owner snapshot, increments revision, sets owner/kind/route and indexing status | Writer authority, not a committed-corpus generation; claim also changes health/error projection |
| Invalidation | `repository.rs:909–945`: one owner-checked IMMEDIATE transaction stales all active volume rows, disables linked Managed entries, stales pending/running AI jobs/items and all linked analysis state, zeros count | The old corpus and derived eligibility disappear at this commit, before replacement enumeration succeeds |
| Replacement batches | `repository.rs:802–895`: each batch checks owner in its own IMMEDIATE transaction, overwrites `ON CONFLICT(id)`, sets `is_stale=0`, re-enables Managed membership and enqueues/reactivates AI work; notification follows commit | The first successful batch is visible and eligible for AI before later batches or terminal success |
| Physical indexes/counts | `db/schema.rs:2165–2189,2296–2355`: external-content FTS follows row insert/update/delete; count triggers follow active rows | Count and physical FTS reflect each batch; there is no unpublished version selector |
| Checkpoint | `repository.rs:623`: independent owner-guarded UPDATE | Its commit is not coupled to publication/terminal intent completion |
| Completion | `coordinator.rs:727–860`, `repository.rs:576–620`: terminal status/timestamps/intent completion and owner release are another UPDATE | Completion is not a switch of a staged dataset; it cannot roll back prior batches or derived effects |
| Reads | `search.rs:315–601`, `repository.rs:1003–1129,1154–1255`: search/identity/path/count paths use active rows and/or enabled volume, without a publication epoch | A read snapshot is internally consistent for that moment, but can consistently observe only batch 1 of a rebuild |
| Derived consumers | `managed_scope.rs:43,269`, `managed_worker_hardened.rs:61–150,335–410`, `legacy_queue.rs:42`: eligibility/validation use mutable rows, scope policy, fingerprint and enabled/stale flags | Search-only suppression cannot prevent provisional AI work or restore previous eligibility/state |

Windows Direct initial enumeration stales before opening/enumerating MFT
(`windows/mod.rs:48–70`). The default provider rebuild delegates to initial
indexing (`coordinator.rs:120`). Service Rebuild forwards the same stale and
Entries events after validated admission (`windows/service.rs:398–425`).
MFT's temporary SQLite file stages **native records**, not a published Global
Index generation: it streams entries to the sink and then commits a checkpoint
(`windows/mft.rs:78–125`). It does not stage Managed/AI state or survive as a
repository recovery authority.

macOS initial and full reconciliation both stale before streaming
(`macos/mod.rs:397–459`). Full reconciliation can occur inside an incremental
operation. A gate based only on `operation_kind='rebuild'` misses this path,
source reappearance and initial-baseline replacement. The publication contract
must cover every whole-corpus replacement boundary, not just the Rebuild button.

## Why existing ownership does not close this gap

`global_entries.id` is a stable native-identity-derived primary key
(`models.rs:336,380`; `db/schema.rs:2133`). There is only one stored metadata
value per ID and one `is_stale` flag. Managed membership, analysis state and AI
jobs refer to that identity (`db/schema.rs:2191–2265`). No published-generation
pointer, staged checkpoint or previous-health snapshot exists in this domain.
Schema 40/41's Library authority is a separate domain and is not available for
Global Index reuse or activation in this task.

`ensure_source_operation_owner_tx` (`repository.rs:1714`) correctly rejects
foreign-volume, disabled or superseded writes within each transaction. It does
not associate a row with its writing revision. Once an admitted batch commits,
revoking that owner prevents subsequent writes but cannot restore overwritten
metadata, old Managed eligibility or AI state. Dropping the sink unregisters
local activity, not the committed batches (`coordinator.rs:355`).

The restart contract must remain: unknown Service work blocks Direct replay;
only validated Service admission can reconcile it; abandoned Direct work is
reclaimable only under the existing process-identity rules
(`repository.rs:1775–1851`). Pending rebuild intent supports a later explicit
wake/admission; it is neither a rollback log nor an atomic publication marker.

## Rejected bounded shortcuts

- Delaying READY or intent completion alone leaves batch rows and AI effects
  exposed. Current indexing/paused/error health already does not gate these reads.
- Delaying stale invalidation keeps removed old entries and overwrites shared
  IDs in place: readers see a mixed corpus and old versions are still lost.
- Hiding sources only while an owner is present re-exposes partial data when
  failure/cancel releases the owner. Incremental full reconciliation also evades
  a rebuild-kind-only condition. Gating every consumer on a new interpretation
  of existing status/intent fields is a cross-module persistence-contract change;
  it also requires preserving committed health and checkpoint independently.
- Setting `enabled=0` confiscates user policy and revokes ownership. Marking all
  rows active again fabricates membership and cannot recover overwritten values.
- In-memory buffering or a temporary MFT file has no crash-safe publication
  contract. Copying it into live rows at completion still performs O(N) work in
  the cutover transaction. Holding one 100k+ writer transaction is prohibited.
- Refusing every replacement before admission would avoid new destructive
  commits, but disables full-rebuild recovery rather than delivering P3; it also
  does not repair a corpus already interrupted under Schema 41. No such product
  behavior change is introduced without the Owner's choice.

## Schema 42 requirement and impact for Owner assignment

**Requested next version: Schema 42, subject to Owner allocation after checking
all concurrent migrations.** This report reserves no number and proposes no DDL.
Owner must select the persistence design before production implementation:

1. Persist a source-specific unpublished build identity bound to the current
   operation revision/token, source identity and coalesced intent. Separate its
   candidate checkpoint/count/completeness from the published checkpoint,
   committed health/error and published corpus. Reuse existing admission and
   late-write fencing; do not create another scanner, scheduler or intent queue.
2. Provide durable staging/versioned metadata with stable **logical** identities
   preserved for user references. Select a complete corpus by a bounded atomic
   pointer/barrier transition. Decide physical row/FTS keys and all foreign-key
   adapters together; adding a generation column to a single-ID upsert is not
   sufficient to retain both versions. Do not migrate Managed Library `files.id`.
3. Include Managed membership and AI eligibility/input bindings in publication.
   Unpublished entries cannot enqueue runnable AI. In-flight results from an
   obsolete published/build generation must fail completion revalidation, while
   user corrections, canceled-job terminality, current scope/provider policy
   and previously committed real errors survive. Avoid an O(N) derived-state
   rewrite in the publish transaction; choose versioned projections or an equally
   bounded design explicitly.
4. Atomically publish only a validated complete baseline with its checkpoint,
   count, terminal health and completed intent revision, using owner/identity CAS.
   Degraded/partial native collection and cancellation must not assert complete
   publication. A terminal-response loss after commit must be idempotently
   distinguishable from an unpublished build. New intent must not be swallowed.
5. On failure/cancel/crash, retain the prior published generation where it exists;
   otherwise expose an explicit unavailable/incomplete source. Retain pending
   intent and real failure information. Reconcile abandoned staging only through
   existing admission/wake paths; fence old Service/Direct results and isolate
   volumes. Garbage-collect unreachable generations in bounded transactions
   without changing the logical published view or adding idle polling.
6. Migrate existing rows conservatively: Schema 41 may already contain an
   interrupted corpus. Neither `last_full_index_at`, owner absence nor status
   alone proves those rows constitute the last complete corpus. Specify how
   legacy completeness is established or fails closed, preserve all existing
   health/identity/checkpoints/intents, and retain future-version rejection and
   rollback compatibility gates.

Affected owners: Global repository/schema/FTS/count maintenance; Coordinator
Sink and terminal paths; Windows Service event application/Direct and MFT;
macOS baseline/reconcile; Global Search snapshot/identity/path readers;
Managed Scope and AI enqueue/claim/completion/read projections. Audit other
`global_entries` consumers when assigning implementation. The separate #417
Library SourceUID/F1 migration, files and activation authority remain untouched.
IPC v3 is not assumed to need a version change: first assess whether the existing
validated admission/events/terminal result can drive the local staging contract.

The publish transaction must do bounded metadata/CAS work; existing batch
IMMEDIATE transactions and the 5000 ms busy timeout are retained. That timeout
limits waiting for a lock, **not** time spent holding it. The whole-volume stale
transaction already scales with row/derived-state count and is not evidence of
meeting the five-second writer budget. Qualify the chosen cutover using existing
performance lanes, without adding a duplicate 100k/500k matrix.

## Evidence, reuse and remaining acceptance

This stop decision is source-level evidence, not a newly executed native fault
reproduction. No Rust tests or platform qualification were run for this docs-only
delivery; baseline CI must not be promoted into evidence of P3 correctness.
The Draft PR's existing routed documentation/governance CI is the delivery gate.

Reuse these existing tests when implementing the Owner-selected design:

- `tests.rs::rebuild_intent_coalesces_without_changing_committed_source_health_or_entries`,
  `source_revision_fences_late_stale_and_checkpoint_writes_after_disable`,
  `unknown_service_owner_cannot_be_replayed_by_direct_after_restart`,
  `abandoned_direct_owner_is_recovered_when_its_process_is_gone`,
  `source_operation_claims_are_independent_per_volume` and
  `local_operation_registration_cleans_up_on_commit_failure_and_cancel`.
- `coordinator_recovery_tests.rs::multivolume_busy_and_source_failures_keep_independent_durable_statuses`,
  `typed_recovery_survives_cycle_and_next_admitted_rebuild_reaches_ready`,
  `repeated_same_source_lifecycle_requests_are_serialized_and_rebuild_wakes_coalesce`
  and `sqlite_busy_while_finishing_releases_the_local_owner_for_service_recovery`.
- `lock_contention_tests.rs::batch_upsert_immediate_transaction_waits_for_short_writer_and_commits_atomically`
  covers batch atomicity, not whole-rebuild publication. Existing snapshot/status
  tests do not interrupt a real multi-batch admitted replacement.
- `hardening_tests.rs` already covers canceled jobs, policy switches and in-flight
  scope disable; extend these bindings rather than introducing a second AI suite.

Still **NOT PASSED / NOT FIXED**: old corpus visibility after admitted invalidation;
failure of batch 2 after batch 1 commits; cancel/crash/reopen between any of stale,
batch, checkpoint and terminal commits; provisional Managed/AI execution;
published-state recovery after late terminal results or Service/Direct switching;
idempotent publication and bounded cleanup. Existing revision fencing is retained,
but these are additional whole-rebuild obligations. Windows/macOS physical-machine
acceptance, real power-loss behavior and the historical #328 lock holder remain
unverified. No merge, Issue closure, Release or Codex Review is authorized.
