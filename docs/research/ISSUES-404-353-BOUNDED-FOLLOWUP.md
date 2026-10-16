# #404 / #353 bounded follow-up

Date: 2026-10-16. Base: `4d3c56d8d24c18f971d237f05907ad2a66d80975`
(Schema 41, after #430). Its master CI 38904859791 completed successfully.
The current user request authorizes investigation and bounded repairs. Neither
historical failure is reclassified or closed by this change.

## Outcome and scope

- #404: remove one demonstrably redundant SQL lookup in an existing-identity
  observation. This is a small contention-cost reduction, **not a proven repair
  of the historical SQLite BUSY failure**.
- #353: add same-process counts for three owners omitted from the existing
  settled-session totals. This is test-only diagnosis, **not a memory fix**.
- Acceptance: preserve identity precedence and cross-root conflict fencing;
  preserve the five memory epochs, workload, warm-up, sampler and HARD rule.
  No migration, retries, timeout changes, scheduler changes or CI routing edits.

## #404 finding

`observe_source_for_root_tx` used
`root_prior_uid.or_else(...).or(latest_source_uid_for_path_tx(...)?)`.
Rust evaluates the argument of `Option::or` even when the left side is `Some`.
Thus every file observation performed a cross-root path query under the scan's
SQLite write transaction, including when the root claim or legacy attribution
already determined the result. Use an explicit match to perform the fallback
only for `None`. Root claim → legacy attribution → unique cross-root claim
precedence, including ambiguous-path handling, remains unchanged.

An isolated Python SQLite 3.53.1 check used the repository's exact root-claims
DDL/indexes and fallback SQL with 100,000 synthetic claims. 500 redundant
lookups took 2.9 ms; the plan used the path/state index plus a temporary GROUP BY
tree. This is illustrative, not a Rust/Windows product benchmark or a guaranteed
speedup. It is far too small to establish the cause of a historical 5-second
BUSY timeout. The five newly scanned roots in the pressure fixture may have no
existing identity; those first observations still need this fallback.

Historical CI 38854131109 attempt 1 remains a genuine `persist_scan_batch` /
finalization BUSY failure. The historical lock holder is still unknown. #430's
all-state physical index is already in the base and is not duplicated here.
The new cross-root replacement regression ensures the fallback still marks
the earlier root claim conflicted when neither higher-priority owner exists.

## #353 finding and falsifiable observation

Historical CI 38854131109 attempt 2 recorded PrivateUsage samples
`[6651904, 6774784, 6782976, 7159808, 8966144]`, with flat handle counts and zero
settled Browse/Preview session registries. These totals do not include Read
Gate lease records, Preview asset records/bytes or Thumbnail cache entries.
They therefore do not rule out retention in those owners. Nor do they prove it.

The existing steady-state test now records those owners at idle, after each of
the same five memory samples, and after runtime disposal. All values are held
in fixed-size observations and emitted after measurement, before the existing
HARD assertion. Test-only accessors do not prune expired records, evict caches
or mutate owners. The lease-expiry regression checks repeated sampling leaves
the expired record retained.

Prediction: if one of these owners accumulates retained records across the
failing epochs, the same failing process will show increasing owner counts.
If counts remain stable, this particular retention hypothesis lacks support;
allocator commit/fragmentation, allocation size changes, and other owners
remain unmeasured. Entry counts are not heap bytes. This does not replace
#356's HeapWalk work or attribute a different run's memory to this process.

## Validation boundary

Local validation uses Linux with Rust 1.97.1 and installed GTK/WebKit build
dependencies. Windows PrivateUsage and the native pressure harness cannot be
qualified on Linux. Required hosted checks remain authoritative; historical
failures remain preserved even if the new candidate passes.

Validation results are reported in the accompanying PR. #404 and #353 remain
open; no root-cause-complete, native acceptance or release claim is made.
