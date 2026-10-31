# Issue #380 D3 — admission and owner comparative proof

D3 is **test-only architecture evidence**, not a production fix or native Windows
Service acceptance. Recommend **A**, conditional on implementing every guarded
writer/publication prerequisite below. B also satisfies the bounded safety
suite but requires an additional research Offer/Grant wire contract. C's
cooperative arbiter passes its registered routes and **fails full writer
coverage**: an independent SQLite connection bypasses it. Production remains
**NO-GO**. No production implementation, schema version or IPC version is
allocated here.

## Authority and baseline

- [Latest Owner authorization](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6142628010).
- [Architecture contract](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6142561334).
- [Source-grounded review](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6142385721).
- Master HEAD: `3275ac7c72df8ae4139a30f16c7f649971324e97`.
- Master Tree: `009576241cd4e8e2ba12cc644a368314f599faa6`.
- Origin: `https://github.com/ArdenZC/Zen-Canvas.git`.
- Independent branch: `test/issue-380-d3-admission-owner-oracle`.
- Owner input bodies, IDs, URLs, update times and SHA-256:
  [owner-inputs.json](owner-inputs.json).

No candidate PR was merged or used as the base. #393 is diagnostic evidence,
not a repaired implementation. #381 remains independent. The Owner's current
ACTIVE DEVELOPMENT instructions supersede the older STATUS release history.
All additions are confined to the two authorized new-file directories.

## Execution and raw evidence

Run with Python 3.10+ and its standard library; no network or installation:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 tests/issue380_d3/run.py --output .tmp-tests/d3/review
PYTHONDONTWRITEBYTECODE=1 python3 tests/issue380_d3/verify_evidence.py docs/project/research/issue-380-d3/evidence
```

The runner creates independent disposable WAL databases and controlled child
processes, exchanges JSON readiness/transaction barriers, and removes its DBs
at exit. No sleep, timer lease, retry loop, native product or user directory is
used. Fifteen-second process timeouts are hang bounds, not correctness evidence.
Every adverse ordering follows a pipe acknowledgment or committed transaction.

- [Summary and individual assertion results](evidence/summary.json).
- [Original input/output traces and actual snapshots](evidence/raw.jsonl).
- [Actual environment, commands, exit codes, SQLite source ID and hashes](evidence/environment.json).
- [Runner](../../../../tests/issue380_d3/run.py),
  [independent oracle](../../../../tests/issue380_d3/oracle.py),
  [candidate implementations](../../../../tests/issue380_d3/model.py),
  [adversarial scenarios](../../../../tests/issue380_d3/scenarios.py).
- [Research-only SQLite fixture](../../../../tests/issue380_d3/fixture.sql),
  [controlled process](../../../../tests/issue380_d3/process.py),
  [Service producer model](../../../../tests/issue380_d3/producer.py),
  [v3 envelope projection](../../../../tests/issue380_d3/wire.py),
  [evidence integrity checker](../../../../tests/issue380_d3/verify_evidence.py).

Raw JSONL retains actual PIDs, child exits, every command/result, unsafe SQL,
full final snapshots and a per-case snapshot catalog. A snapshot response uses
a SHA-256 reference to that catalog to avoid repeating identical data; the
integrity checker verifies every reference and snapshot digest. These are real
observations of the research databases, not predicted/fabricated product logs.

The published raw run identifies its **source execution commit** in the
environment. Its output files and this report are committed afterward. The
final delivery re-executes the final PR HEAD, checks equality of source hashes
and all semantic case results, and records that HEAD/Tree plus final-run digests
in the PR description. Do not relabel the committed earlier raw run as a
separate execution of the later documentation/evidence commit. Final HEAD,
Tree, full diff link/digest and natural CI are in the Draft PR description.

## Actual comparison

The final suite has **163** independently checked cases: A **53**, B **56**, C
**54**. All registered-route T01–T14 cases pass; all **18** N controls (six per
candidate) produce actual rejected violations. C's additional coverage case
records `BLOCKED_EXPECTED_REJECTION`, rather than presenting the unsafe
arbiter as successful. Runner exit is **0**, meaning the comparative research
contract is met, not that all candidates or production pass.

| Criterion | A: first lawful v3 event / terminal | B: Offer/Grant | C: persistent arbiter attempt |
| --- | --- | --- | --- |
| Required T01–T14, configured writer routes | PASS | PASS | PASS |
| N01–N06 detected by independent Oracle | 6/6 | 6/6 | 6/6 |
| Independent processes on guarded routes | PASS | PASS | PASS |
| Arbitrary existing SQL connection fenced by arbiter | Not a mandatory SQL/OS authority claim | Not a mandatory SQL/OS authority claim | **BLOCKED; actual bypass committed** |
| Smallest source edit surface for a future implementation | Coordinator intent/prewrites; incoming v3 validation; all sink/repository effect gates; typed transport/error and Direct transfer | All A prerequisites plus Service reservation/offer/grant/instance lifecycle and client handshake | All publication/intent prerequisites plus rerouting or enforcing every DB writer through one authority |
| Durable requirements | Independent intent generation; monotonic source revision/owner; operation tombstones; staged publication/reconciliation | Same plus offer/instance/grant binding | Same plus genuinely exclusive writer authority; current cooperative record is insufficient |
| Wire requirement | Existing v3 envelope; no added wire field | Explicit research Offer/Grant; production version decision remains Owner's | No extra indexing wire assumed; writer control surface must actually be universal |
| Lock/performance risks | Immediate SQLite transactions serialize writes; staging/storage cost; BUSY remains a distinct DB outcome | Same plus reservation lifetime, stalled/lost grants and cleanup | Central bottleneck/dead owner recovery; uncontrolled SQL access breaks coverage |
| Native/product evidence | NOT_RUN | NOT_RUN | NOT_RUN |
| Recommendation | **Prefer for bounded safety / smallest wire surface** | Safe research alternative; no demonstrated safety benefit requiring new wire | Reject current attempt as unified authority |

A/B PASS requires **all legitimate production writers to adopt the proposed
conditional boundary**. It is not permission to retain unguarded SQL callers.
The C bypass warns of precisely that coverage obligation for every approach;
A/B are not claimed to revoke arbitrary SQLite or OS access. C's stronger
unified-writer claim is deliberately tested beyond its registered API and
rejected. The common T suite uses identical fixtures, adversarial schedules
and reference oracle; B adds handshake-specific expiry/loss cases and C adds
the separately identified coverage qualification.

## Ownership and publication mechanism

The independent Oracle imports no candidate transition functions. It computes
published rows from the original inventory and retained correction ledger,
checks health/counters/checkpoints against immutable baselines, and rejects
actual unsafe database snapshots. Candidates share the same research
publication implementation for a fair comparison of admission mechanisms;
passing one candidate is never used as the Oracle for another.

1. Rebuild request commits only a separate intent, with idempotent per-source
   coalescing and an independent monotonic intent generation. It does not
   rewrite real REBUILD_REQUIRED, PermissionRequired or ERROR health. An old
   coalesced request cannot cancel a later intent generation.
2. Requests capture source revision, unique request/operation identity and
   route. A verifies every incoming v3 envelope/payload and claims source owner
   in the SAME `BEGIN IMMEDIATE` transaction as the first accepted effect.
   The source-grounded Service producer computes before that claim but has
   neither a database pathname nor a SQLite/repository/sink handle.
3. B additionally reserves a distinct Service slot, checks source, offers a
   token bound to instance/request/source/revision, then accepts DB claim and
   Grant. Logical-input expiry releases an ungranted reservation. Expired,
   duplicate, mismatched, restarted or lost grants cannot produce DB effects.
   This is research state, not existing IPC v3 Offer/Grant functionality.
4. All staged entries, source state/provider and checkpoints are fenced before
   effects. Database entry membership validates EntryStale; payload volumes
   validate Entries; request-scoped lookup identity rejects replay. Source
   metadata/new-owner writes advance revision. Status equality cannot undo
   that ABA fence. No read/update split is allowed outside the transaction.
5. Published Global/Managed/AI truth is retained while a stream is staged.
   Successful admitted publication combines stale flags, retained/new rows,
   Managed enabled state, three AI derivative tables, counts, checkpoint and
   completion clocks in ONE transaction. Existing user corrections are never
   removed. Resume delta publication does not blanket-stale retained entries
   or advance the full-index clock.
6. Partial stream or unknown transport sets a durable reconciliation flag,
   retains the old published snapshot and annotations, and never Direct
   replays. Staging remains bound to its operation as recovery/audit data;
   it is not searchable. A staged real fault survives even an inconsistent
   successful terminal; it does not become false READY.
7. Direct transfer requires an explicit modeled NotSent or fully released
   no-effect proof, retains request/intent identity, rotates the fenced route
   token/revision, and leaves exactly one source writer. No observed events
   is not itself no-effect proof. Current production does not yet expose this
   trusted outcome classification; the fixtures do not claim to prove it.
8. A fresh process recovers only dead controlled-process owners. A live-owner
   wake cannot release or steal them. The harness checks its recorded child
   PID without a clock lease; PID reuse conservatively blocks recovery. This
   is Linux controlled-process evidence, not a native Windows owner-lifetime
   protocol or a production crash detector.

**Zero-event completion is conservative:** a correlated successful terminal
claims/releases ownership but preserves committed health/data/clocks and leaves
Rebuild pending. A checkpoint-only stream is marked reconciliation. No full
inventory is invented from absence of events. This satisfies the safety test,
but does not claim an empty-volume Rebuild completes. Owner must choose the
production completion contract; the current PipeSink does not emit an Entries
frame for an empty batch. No automatic retry is introduced to hide this gap.

## Contract coverage

| Contract | Actual checks in raw cases | A | B | C registered routes |
| --- | --- | --- | --- | --- |
| T01 | Start busy preserves entire published view; explicit existing wake succeeds | PASS | PASS | PASS |
| T02 | Resume busy; wake delta preserves retained rows/full clock | PASS | PASS | PASS |
| T03 | Rebuild(one), prior five degraded states, pending intent, zero prewrite | PASS | PASS | PASS |
| T04 | Rebuild(all) mixed busy/success; another source's true fault and aggregate isolation | PASS | PASS | PASS |
| T05 | Independent owner defeats CAS; producer computed before failed claim; duplicate source race | PASS | PASS | PASS |
| T06 | Real restart, durable replay/coalesce/cancel; killed pre-intent commit; old-generation cancel | PASS | PASS | PASS |
| T07 | Late busy/event/interrupt after READY, PermissionRequired, true RebuildRequired, ERROR | PASS | PASS | PASS |
| T08 | Same-status READY ABA, monotonic revision advances | PASS | PASS | PASS |
| T09 | Wrong/missing/noninteger version, request/source/lookup, malformed/error codes; all nine legal event kinds | PASS | PASS plus offer identity/expiry | PASS |
| T10 | Zero-event OK and checkpoint-only OK; no false READY, clock or inventory | PASS | PASS | PASS |
| T11 | Four cut points × pause/cancel/shutdown/real restart; live-owner wake; B lost offer/grant and instance restart | PASS | PASS | PASS |
| T12 | Partial staged stream + UNKNOWN; restart; forbidden Direct replay | PASS | PASS | PASS |
| T13 | Proven no-effect Direct transfer; independent competing request cannot write | PASS | PASS | PASS |
| T14 | Full derived publication, actual SQLite BUSY(5), process killed inside transaction, retained corrections, real faults | PASS | PASS | PASS |

For A, the B-named `after_offer` cut is explicitly pure Service computation
before the first client claim; `after_grant` is a first validated read-only
lookup that durably claims owner with zero actionable effects. C uses its
persistent arbiter claim. No nonexistent A Offer/Grant wire phase is claimed.

| Negative | Real unsafe action | Oracle rejection |
| --- | --- | --- |
| N01 | Persist INDEXING/clear cause, then merely skip busy ERROR | Speculative committed health remains |
| N02 | Restore READY over a newer PermissionRequired owner | New owner's cause/health overwritten |
| N03 | Pre-admission SQL stales entries/disables Managed/zeros count | Published user/index truth destroyed |
| N04 | Late event/error after same-status newer revision | ABA-fenced owner overwritten |
| N05 | Mark unknown partly applied operation Direct and pretend READY | Reconciliation removed / Direct replay |
| N06 | Treat a synthetic resource-lease artifact as Service admission, then write SQL | No Service reservation; real unauthorized effects |

Every negative records executed SQL, resulting snapshot, a nonempty Oracle
failure and `expected_rejection_observed=true`. The scheduler artifact is
explicitly synthetic; no real WorkScheduler or Service admission is claimed.
The old defect evidence in #393 is retained and not edited/deleted. Here the
unsafe algorithms fail the SAME Oracle under actual SQLite effects while
safe paths pass; green is not merely a new error-enum assertion.

## Source-grounded implementation boundary

Environment provenance hashes six full master source files. The relevant
existing ownership paths are:

- `coordinator.rs:218-256`: public Rebuild writes fault-like health and mass
  stale before admission; `:299-380`: DatabaseIndexSink writes entry, stale,
  checkpoint, health/provider and lookup paths; `:545-563,638-662`: speculative
  progress and late errors. Discovery/unavailable/legacy/settings owners also
  need separate revision-fenced authority; they cannot be silently excluded.
- `repository.rs:207-241`: unconditional source-state update; `:341-367`:
  Global/Managed/AI/count mass stale across separate statements. Its diagnostic
  source revision hash is not the model's monotonic writer CAS.
- `windows/service_host.rs:195-278`: Service admission is distinct from the
  scheduler; `PipeSink:366+` emits events/lookup requests, not product DB writes.
- `windows/service.rs:258-291`: currently consumes incoming frames without the
  proposed per-request binding. `wire.py` follows its existing serde envelopes:
  Events have no invented protocol-version field; Responses do. Entries and
  EntryStale derive source from payload/database membership, not a new envelope.
- `windows/mod.rs:333-455`: all streaming events and Direct fallback must use
  one conditional sink. Unknown transport must not become Direct auto replay.

The fixture is NOT Schema 37 and is NOT a production migration proposal. It
adds intent generation, revision/operation records, staging, reconciliation
and B reservation state solely for proof. A production choice therefore needs
Owner approval of minimal durable records and ALL affected writers, not a
claim that today's existing fields already suffice. This research does not
allocate Schema v38, add IPC v4, alter Service/SCM or fix #328/#381.

## Evidence limits and decision

This is actual Linux/Python SQLite WAL and process evidence. Windows/macOS
native Service, PipeSink binary, SCM, cancellation transport, product Rust
serde and bundled product SQLite were **NOT_RUN**. The v3 adapter tests
existing envelope/identity semantics; unrelated GlobalEntryInput filesystem
fields are deliberately reduced fixtures, not full Rust codec qualification.
B/C are research prototypes, not deployed protocols/authorities.

Ordinary GitHub CI is recorded separately in the PR. Existing CI does not
invoke this standalone Python harness; generic CI success cannot replace the
163-case evidence or native acceptance. No workflow, routing or gate changes,
manual dispatch, Codex Review, merge, close, release or master write occurs.

**Architecture recommendation:** choose A as the least-wire-change safety
candidate, with independent durable intent generation, conditional source
ownership on every legitimate writer, staged atomic publication, conservative
zero-event handling and fail-closed UNKNOWN. Do not implement only a ServiceBusy
`continue`, status rollback or scheduler lease. B is available if Owner later
requires producer-side pre-work Grant/liveness; this proof does not justify
its extra wire merely for sophistication. C needs a real universal writer
boundary before it can be recommended.

**Research disposition:**
`ISSUE_380_D3_COMPARATIVE_TEST_ONLY_READY_FOR_OWNER_REVIEW`.
**Production disposition:** **NO-GO — Owner decision and native qualification
remain outstanding.** Stop at the independent Draft PR for manual review; no
new unbounded research phase or production work is started.
