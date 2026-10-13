# ISG-02 test-only reference conformance

Authority: [Issue #382](https://github.com/ArdenZC/Zen-Canvas/issues/382) and
[Owner frozen semantics, #371 comment 6107816377](https://github.com/ArdenZC/Zen-Canvas/issues/371#issuecomment-6107816377).
ADR-0014 remains PROPOSED. Smart Lean remains a synthetic snapshot mode.

Starting verified master: `3275ac7c72df8ae4139a30f16c7f649971324e97`,
tree `009576241cd4e8e2ba12cc644a368314f599faa6`.
Both [master CI](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38130172667)
and [Full Validation](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38229431247)
succeeded for that SHA. No CI runs were queued/in progress at task start.

## Module boundary

`tests.rs` registers this directory under `#[cfg(test)]`; its parent is already
`#[cfg(test)]`. These modules use only the Rust standard library. There are no
production calls, native adapters, filesystem walks, database/FTS operations,
network requests, AI calls, settings changes, schema/IPC changes, workers or
workflow changes. The evaluator receives shared immutable references and returns
a value. Reconcile/Backfill hints are enum values with no executor.

- `model.rs`: `policy_evaluate(context, immutable_snapshot)` represents frozen
  `PolicyEvaluate`. Four result shapes carry evidence/need/missing evidence.
  SourceHealth is input evidence, while CoverageProjection counts only evaluated
  results and checked stage accounting. Neither emits source Ready.
- `oracle.rs`: independent hand-written 18-witness expected-result ledger. It
  imports only output vocabulary and literal identities, never Context,
  Snapshot, matching/ancestry functions or the ISG-00E benchmark/CorpusClass.
  Expected results are authored directly, including reasons and evidence.
- `matrix.rs`: fixtures, the 16 adversarial test groups and deterministic
  sampling. Fixture construction and oracle output construction are separate.

The reference model consumes **synthetic trusted facts**, not native evidence.
Protected identities are supplied by the snapshot's source-bound known-folder
registry. Contextual performance evidence binds a project/owner ancestor and
an evidence handle to a specific subtree. Names alone supply no classification.
Scope roots delimit lawful lookup; explicit choices are separate rules. A
narrower explicit Include beats weaker performance evidence; an equally deep
explicit Exclude wins the conflict. Graph-only occurrences consume a
source-verified included-descendant need bound to a complete immutable snapshot; they cannot become search/AI payloads.

Path matching uses components, Windows ASCII case and extended DOS/UNC transport
spellings. Unicode canonical spelling, when present, is an explicit source fact;
the model does not invent Unicode equivalence or implement a native normalizer.
The canonical path still requires the correct root/physical identity and birth
generation. Native case/Unicode resolution and native protection-registry
acquisition are **UNVERIFIED**, as are actual MFT/USN/Spotlight data, native
alias/permission behavior and native stage counts. Platform parity is abstract
conformance only. Synthetic stage-accounting booleans are not native proof.

The canonical fingerprint is a collision-free structured snapshot rather than
a digest: sorted/deduplicated roots, rules and protection registry plus stable source/volume/provider IDs,
source, volume, provider, mount and permission epochs, mode, revision, platform and
known-folder version. Cache keys also bind the full immutable Context, including
path/rename epoch, parent/birth identities, alias kind/target/source/permission,
Unicode evidence and health. Older keys can remain in this bounded experiment
cache, but can never alias newer keys; publication validates the current
complete fingerprint against the stage receipt (as well as source/revision) and
rejects obsolete generations, including same-revision rules/roots/known-folder drift.
Parent chains and alias traversal each stop at 32; the cache stores at most one
entry per sample in T16. This is no production cache design or adoption decision.

## Correctness matrix

| Test | Assertions |
| --- | --- |
| T01 | Broad compatibility; Smart Lean/Custom inert synthetic snapshots; immutable inputs |
| T02 | Projects/Windows, Design/build, Configs/AppData, Program Files2 remain lawful includes |
| T03 | docs/src/README/manifests/.vscode; incomplete evidence yields reasoned Undetermined |
| T04 | Dependency/build/cache context, ancestor, evidence handle and subtree required |
| T05 | Nested specificity, weaker/broader Include, same-depth conflict, OS/permission dominance |
| T06 | Known-folder provenance bound to source/volume/version, never basename |
| T07 | Graph-only anchors; valid zero, unverified zero and failed source accounting |
| T08 | Missing parents, unavailable/busy/permission/partial/checkpoint reasons and SourceHealth |
| T09 | Windows case, UNC/extended paths, component boundaries, NFC/NFD source fact + identity fallback |
| T10 | Hardlink paths, typed symlink/junction/reparse targets, cycles, bounds, cross-volume/permission guards |
| T11 | Rename/move, Include-to-Exclude Reconcile and Exclude-to-Include Backfill values only |
| T12 | Canonical fingerprint, cache changes, policy/stable identities and source/volume/provider/mount/permission replay |
| T13 | Windows Service/Direct and abstract macOS parity, no native claim |
| T14 | Immutable mock Global/Managed/AI/consent/corrections/history/files/schema/IPC sentinels |
| T15 | Out-of-order/double/missing parents, FRN/inode birth reuse and checkpoint gaps |
| T16 | 1k/10k, two seeds, hand-authored oracle, reshuffle, cache on/off, counters and purity |

T14's sentinels illustrate disjoint authorities, not a production integration
or migration test. Its stronger safety boundary is the absence of mutable
references, production imports and any IO API in the reference evaluator.

## Reproduce

From repository root with the pinned toolchain (`1.97.1`):

```sh
rustc --edition 2021 -D warnings --test src-tauri/src/global_index/tests/isg02/mod.rs -o /tmp/isg-02-tests
/tmp/isg-02-tests --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --features desktop-runtime --lib global_index::tests::isg02 -- --nocapture
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
DOCS_DIFF_BASE=3275ac7c72df8ae4139a30f16c7f649971324e97 DOCS_DIFF_HEAD=HEAD npm run test:docs
```

The standalone command tests the exact same modules on Linux without unsupported
product dependencies. Product Cargo integration runs through existing normal
Windows/macOS PR CI, including exact-head and merge-integration lanes. No manual
Full Validation or large benchmark dispatch is added.

T16 uses xorshift64 seeds `0x49534702` and `0x6107816377`; each seed tests 1,000
and 10,000 unique path occurrences, spanning all 18 oracle witnesses. Every
sample is evaluated twice uncached and twice cached, once in shuffled order.
Expected cache counters are `evaluations=N`, `hits=N`. The total is 22,000
samples / 88,000 result comparisons. `--nocapture` prints seed, sample count,
witness histogram, actual timing and bounded cache counters. Timing has no
native performance/savings interpretation. Tests are default, not ignored.

No tests above authorize production filtering, historical stale/delete,
Smart Lean activation, an Accepted ADR, native acceptance or release. Delivery
stops at Owner review of a Draft PR; merge and issue closure remain Owner actions.
