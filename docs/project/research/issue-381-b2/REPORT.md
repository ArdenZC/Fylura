# #381-B2 test-only semantic research

## Authorization and immutable inputs

Owner [B2 execution GO](https://github.com/ArdenZC/Zen-Canvas/issues/381#issuecomment-6141964429) overrides historical design-only holds **only for this isolated test scope**. Freshly fetched master is `3275ac7c72df8ae4139a30f16c7f649971324e97`, tree `009576241cd4e8e2ba12cc644a368314f599faa6`. Existing Draft #383 remains at `584621c32054e16ac0a61afc669a04380a976b7f`; #394 is the separate static B1 overlay at `fc1550b96f55812213f09fa0e5912a6ec0a04ca1`. S8 is `4005073337f198a245773e30a76178749bfd2920`; S8R is `a48f8849b1ba1baadcf84e217d505e2298cf35a1`. The exact #381 body/comments, #383/#394 bodies and frozen FTS/cursor documents are retained under `evidence/`.

The 103/121/54 frozen labels and 209 registrations are not runtime authority coverage. B1's six test-only STG and ten test-only WFG exceptions remain binding. This model makes no claim about the remaining 169 handler callgraphs. No historic publicly released binary exists; same-version source/writer concurrency is tested, while old-binary upgrade research is not promoted to a current prerequisite.

## Model and independent truth

A source incarnation UID is separate from its path, synthetic physical observation and monotonic observation revision. Same-path replacement creates a new UID and supersedes the old source; retained source rows, tags, notes, manual classification, feedback and history remain attached to the old UID. A shared overlapping-root claim cannot resurrect a superseded physical incarnation. Notes are a **synthetic model field**, not a claim that production has a standalone per-file Notes API.

Each root owns enabled status, epoch, root/Watcher/applied revisions and run/session IDs **plus monotonic ownership revisions**. A complete expected `(UID, observation)` set and digest are supplied by the deterministic fixture. Membership is staged in bounded transactions; the exact set/digest is checked outside publication. SQL guards reject member mutation and sealed proof edits. A member revision CAS prevents appending between complete-set proof and sealing. GC skips staging/ready epochs. Expensive staging/proof is not moved inside the publication lock.

Publication performs five traced statements: `BEGIN IMMEDIATE`, indexed constant-size root CAS/pointer/health update, epoch state update, authority revision invalidation, `COMMIT`. Its SQL does not walk members/sources, run filesystem calls, update per-file stale bits, rebuild FTS or mass-update derived results. CAS binds root enabled/revision, Watcher continuity, source-writer watermark, run/session IDs/revisions and ready state. Wrong ownership rolls back. Failed/incomplete/cancelled/UNKNOWN/UNCERTAIN/gap runs retain the old pointer with `reconciliation_required`. Run acquisition and Watcher changes report reconciliation required until a successful complete publish.

The watermark is **global and conservatively rejects unrelated source writes**, a deliberate reduced-model choice. A newer observation or known superseded source fails closed under an old membership; the old epoch pointer stays authoritative, but an invalid source is not made actionable by replaying old data. Concurrent annotations have independent CAS, survive publication and cleanup, and do not authorize a stale source. History and result rows are retained. Async AI/content/analysis/duplicate job completion uses a same-transaction token check; an authority revision makes earlier current results logically non-actionable immediately, without a mass invalidation write.

`EffectiveVisibility` is the union of valid per-root published claims in the explicitly allowed enabled root set. List, SQL `COUNT(DISTINCT UID)`, selection and detail/preview eligibility run in one read transaction. Execution separately checks the current token, UID, observed path, physical identity and permission. Physical observations and permissions are fixture parameters: **no real OS identity, ACL or filesystem TOCTOU proof is claimed**. Root-bound lexical paths are validated only in the fixture; production symlink/junction/Unicode/case/known-folder policy is not implemented.

The independent Oracle uses Python root/member sets, source observations and a separately written FTS5 BM25 formula. ASCII fixture tokenization and one indexed document-text column make exact score comparisons inspectable. The SQLite model creates an in-memory FTS5 over **all and only globally effective-visible documents** for each small-set search; root scope filters that ranked relation **before LIMIT**. Root scope does not redefine global corpus statistics. This preserves the S8 decision and allows a visible document in A to invalidate B's rank even with unchanged B IDs. It is intentionally **not** a product-cost proposal, external-content/trigram qualification, product SQLite 3.51.3 test, content-artifact schema test or resolution of the broad-BM25 anomaly.

Cursor tokens bind actual scope/query, root seals, source/path/physical/observation seals and the complete current ranked result including scores. Global tokens also bind the global authority revision. Each validation recomputes current truth; no old FTS snapshot replay. Unrelated A changes permit B continuation only when B seals and the **exact** ranked rows remain unchanged. The full-result digest is a small-set correctness technique; no production cursor latency/memory claim is made.

Recovery uses SQLite WAL with FULL synchronous mode and five parent-controlled SIGKILL barriers. Before commit, only the old pointer recovers; after commit, only the complete new pointer recovers. Unpublished runs become abandoned/reconciliation required. Cleanup removes only unreferenced obsolete membership in bounded idempotent batches; it never deletes source/user/result history. This is process-crash evidence, **not power-loss/corruption/disk-full or migration recovery**.

## Required case mapping

All checks compare explicit expected/actual values and retain the event schedule. T01–T15 use the independent reference for source visibility and/or ranks; T16 uses a deterministic exact large UID list and separate expected revision arithmetic, with no large FTS/Oracle performance claim.

| Case | Required proof / implemented assertion |
|---|---|
| B2-T01 | Empty and one-file published views and independent BM25 |
| B2-T02 | E0 through staged batches/seal; immutable proof/member guards; GC skips candidate; complete E1 after commit |
| B2-T03 | Reader pinned E0 across commit; new same-transaction E1 list/count; root/run/session/Watcher CAS rejection |
| B2-T04 | Nested/overlapping root union, UID deduplication, A cannot erase B, explicit/empty scopes and disabled-root behavior |
| B2-T05 | Explicit selected `build` root; unrelated equal basename cannot grant query scope; no new production exclusion policy |
| B2-T06 | Same path/new physical UID; source watermark rejects stale publication; no annotation inheritance; B cannot resurrect old incarnation |
| B2-T07 | Independent DB connections interleave Watcher/upsert/root/run/session/source-watermark writes; A→B→A observation and reused run/session IDs still reject |
| B2-T08 | Incomplete/cancelled/gap/UNKNOWN/UNCERTAIN/failed coverage, missing member despite claimed complete, Watcher gap and proof-to-seal race retain E0 |
| B2-T09 | Concurrent newer tags/notes/classification/feedback, stale annotation CAS, publication and restartable cleanup preserve user truth |
| B2-T10 | AI/analysis/duplicate/content current results invalidated by pointer, late completion historical, fresh completion actionable, all history retained |
| B2-T11 | Current execution token plus independent path/physical/permission validation; hidden UID and stale token denied |
| B2-T12 | Exact effective corpus and root filter before LIMIT; hidden high-rank adversary demonstrates bad post-LIMIT underfill |
| B2-T13 | Independent BM25 scores/order; hidden mutation leaves effective rank unchanged but changes retained-global scores; reopen/rebuild equivalence |
| B2-T14 | Global expiry, safe B continuation for unrelated A change, real B score change invalidation, shared observation expiry, scope tampering denied |
| B2-T15 | P1 staging transaction / P2 sealed / P3 before COMMIT / P3 after COMMIT / P4 cleanup transaction SIGKILL; exact old-or-new, user history, FTS and cleanup restart |
| B2-T16 | 0/10k/100k staged in 1000-row batches, E0 after each batch, pinned reader, two competing writers with unchanged 5000ms timeout, five publication statements, exact E1 IDs/revision, user correction preserved |
| B2-N01 | Actual multi-commit stale-bit projection exposes a mixed generation that is neither E0 nor E1 |
| B2-N02 | Broken cursor replays old IDs while correct current-token validation rejects |
| B2-N03 | Actual path join incorrectly attaches old tags/notes to new UID |
| B2-N04 | Actual root-A global tombstone incorrectly hides independently valid root-B source |
| B2-N05 | Retained-global FTS BM25 differs from independent visible-corpus scores; post-LIMIT filtering underfills |
| B2-N06 | Unconditional publisher/worker updates ignore Watcher gap/revision and expose wrong IDs, false healthy and false current |

Negative records use `expected_rejection_observed`, never a generic broken-model PASS. Every negative contains a nonempty independent expected/actual counterexample diff. A missed rejection or any assertion failure blocks readiness. Seed `38120261013` is an immutable fixture identifier; no random draws occur. Source functions plus raw fixture inputs/event schedules are the replay specification; SQLite backup files are inspectable final projections, not replacements for replay.

## Evidence and qualification

The final raw run and environment are under `evidence/`; no JSON result was hand-edited. The manifest covers full source and evidence bytes. Commands/exit codes include the independent harness, repository documentation/governance checks and Git diff/scope validation. Each live fixture reports page size/count, WAL bytes, journal/synchronous/busy timeout settings. T16 retains each bounded-batch latency, actual BEGIN wait/publication lock time, writer timings, exact IDs/count/revision and traced SQL. Five subprocess commands, checkpoint messages and SIGKILL exit codes are recorded.

T16 records **one sample per size**, including deterministic thread-start/attempt barriers in publication lock time. Writer timestamps begin immediately before `BEGIN IMMEDIATE`; each attempt barrier is signaled by SQLite's actual statement trace callback while publication still owns the lock. they prove committed competing writers under the existing budget, not a calibrated OS scheduler latency distribution. There are no percentiles or Windows/macOS/product-schema/100k end-to-end claims. Ordinary source metadata and user-tag/note writers run on independent connections; both start while publication owns the writer reservation and complete after commit. A source correction increments the watermark and survives cleanup. Large-set expected IDs are recorded in full rather than inferred only from counts/digests.

The final Draft PR carries exact final HEAD/tree, full diff and natural CI results. Existing routing sees `tests/` as frontend changes, so frontend/aggregate checks may run; the Python harness is still independently executed in Codex Cloud. Green routed CI does not qualify Hosted Windows/macOS backend or native identity. Final-head replay source hashes must match the committed raw run, and production/frozen-input diff must remain zero.

**Handoff boundary:** only the isolated B2 contract may be ready for Owner review. Production consumer integration, persistence representation, joint schema allocation/migration/backup, actual native physical identity, product-linked SQLite FTS/query plans, cursor budgets and Hosted native 100k fairness remain separate B3+ gates. #381 is not fixed; #383 is not unblocked; no merge authorization is implied. Stop OPEN/Draft for independent Owner review.
