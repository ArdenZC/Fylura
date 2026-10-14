# Issue #414 G2-S1 R1 fail-closed model remediation

## Scope and base

- Repository: `ArdenZC/Zen-Canvas`
- Branch: `research/issue-414-b2-g2-s1-static-safety`
- Frozen starting HEAD: `2d51938321550332653ad73649892d49b87ed3d6`
- Frozen starting Tree: `f64e818c2c972d4829bfe3006a92301f981a48da`
- Expected parent: frozen starting HEAD above.
- Remote HEAD was checked before edits and equaled the frozen starting HEAD. No branch recreation, rebase, or force push was used.
- Existing `attempt-001/**` files were not edited. Their before/after SHA-256 values are preserved in `attempt-001-preservation.sha256`.

Only the three pre-authorized research documents, files under `tests/issue381_b4_p1a_g2_s1/`, and this new `evidence/attempt-002/` archive are in scope. No production source, Schema, migration, dependency, lockfile, CI, workflow, master, or historical evidence was changed.

## R1 findings and changes

### R1-A — numeric admission

The prior relational checks allowed `NaN` and did not require finite, safe integer resource measurements. The model now rejects non-positive, missing, wrong-type, non-integer, non-finite, and unsafe-integer byte counts across host free-space/caps, volume and image dimensions, host deltas, Windows disk sizes, and macOS image-device sizes. Nominal volumes remain capped at 256 MiB; image and host-delta caps remain capped at 320 MiB; APFS volume and image-root node sizes are checked against their applicable caps. Missing capacity or host-space evidence is unavailable. Both pre- and post-plan reported space and independent caps must prove at least 4 GiB.

Regression coverage: `rejects invalid resource dimensions across both platforms`, `enforces the fixed virtual-volume and host-increment ceilings on both platforms`, `requires both host snapshots and independent caps to prove at least 4 GiB remains`, and `fails closed when required capacity proof is absent`. Every refusal checks non-`DRY_RUN_SAFE` and `intents=[]`.

### R1-B — inventory continuity

The prior validators accepted post inventories after original host devices disappeared or changed. Windows now requires exact continuity of every original identity, fingerprint, disk number, friendly name, size, partition style, and system/boot/OS flag; identity, fingerprint, and disk number ambiguity is rejected, with case-insensitive Windows identity/fingerprint matching. macOS now requires every original device, fingerprint, host classification, hierarchy parent, image-origin field, and size to remain unchanged. New targets must have identities and fingerprints distinct from all baseline records. Windows host system/boot/data devices and macOS `/dev/disk2s5` plus its `/dev/disk2` parent cannot be reclassified as image nodes.

Regression coverage: `rejects Windows baseline continuity failure`, `rejects macOS baseline continuity failure`, `rejects G1 Windows system and data disk fingerprints as targets`, `rejects the exact G1 macOS host Data volume fingerprint as an image target`, and the inventory identity/missing-record cases. Every refusal checks non-`DRY_RUN_SAFE` and `intents=[]`.

### R1-C — cleanup proof

The prior exported mutable cleanup record trusted caller-writable `phase` and boolean proof fields. It is replaced with an opaque frozen session handle whose current phase, sequence, and pending intent live in a private `WeakMap`. Every action is a frozen `NEVER_EXECUTED` intent registered to that one handle and sequence. Only the current issued intent, with a matching action, target, observed owner token, device identity, and image identity, can advance the session. Stale, forged, skipped, reversed, repeated, failed, mismatched, or cross-session events make cleanup incomplete. Cleanup failure retains the original operation failure.

Regression coverage: ordered success flows for both platforms; direct `RELEASE_VERIFIED` plus all-true proof-flag forgery; fake and cross-session intents; skipped/reverse/repeated transitions; changed device/image/owner identity during cleanup; failed/forced detach; missing release evidence; invalid ownership/path/link; and combined original plus cleanup failure. No caller-visible phase or boolean can create a removal intent.

### R1-D — macOS image hierarchy

The prior one-new-device count could not model an image root with partition, APFS-container, or multiple APFS-volume records. The model now requires exactly one new image root and an explicit complete list of all and only image-owned nodes. Each child must use an allowed parent role and reach that root without cycles or host ancestry. Every node must be non-host, use a unique identity/fingerprint, fit its size cap, and link to the exact returned image. The attach envelope must match both the returned image identity and creation-record identity. Its selected mount must name an APFS-volume node in this graph and map to the exact contained mount point. Extra same-image APFS volumes can be represented without changing which volume is mounted.

Regression coverage: valid multi-node and multi-volume synthetic hierarchies plus missing/stale/mismatched image provenance, incomplete/duplicate enumeration, orphan, host-parent, duplicate-root, baseline identity reuse, wrong mounted volume, oversized volume, and oversized image-root negatives. All negative cases assert non-`DRY_RUN_SAFE` and `intents=[]`.

The Apple structured-output shape remains unverified against canonical documentation and the selected native runner. All safety assessments, including cleanup assessments, explicitly carry `s2Eligibility: NOT_QUALIFIED`. No S2 eligibility or authorization is inferred from these synthetic tests.

## RED/GREEN and typecheck evidence

- `red-vitest.log`: first tests-only RED run, **27 failed / 77 passed**. It exposed the intended fail-open cases and an additional test-fixture alias leak (baseline fixture objects were shared across runs).
- The fixture was then changed to return independent baseline records. `red-vitest-isolated.log` is the clean tests-only RED rerun against the unchanged model: **24 failed / 80 passed**. These failures demonstrate the reviewed fail-open behavior and the multi-node completeness gap before source remediation.
- Final Vitest command: `node_modules/.bin/vitest run tests/issue381_b4_p1a_g2_s1/staticSafetyModel.test.ts --reporter=verbose`. `vitest.stdout.log` and `vitest.stderr.log` are the final unedited focused streams. Result: **1 file passed, 145 tests passed, 0 failed**, exit code 0 (`vitest.exit-code.txt`). This exceeds the prior 71-case suite while retaining its Windows/macOS positive lifecycle and failure behavior with cleanup re-expressed through the opaque API.
- Final strict TypeScript command: `node_modules/.bin/tsc --noEmit --target ES2022 --module ESNext --moduleResolution Bundler --strict --skipLibCheck --types vitest/globals tests/issue381_b4_p1a_g2_s1/staticSafetyModel.ts tests/issue381_b4_p1a_g2_s1/fixtures.ts tests/issue381_b4_p1a_g2_s1/staticSafetyModel.test.ts`. `typecheck.stdout.log` and `typecheck.stderr.log` are the final unedited streams. Exit code 0 (`typecheck.exit-code.txt`).
- No full CI, build, native platform command, disk operation, ENOSPC, power-loss, or durability test was run.

## Historical preservation and risk boundary

The four original attempt-001 evidence files remain byte-identical to the frozen starting branch; see `attempt-001-preservation.sha256`. The original attempt-001 manifest itself was not regenerated or changed.

This run demonstrates deterministic in-memory TypeScript behavior only. It does not prove actual Windows/macOS inventory continuity, provider identities, free-space reporting, image allocation, APFS hierarchy output, mounting, unmounting, detach, deletion, power-loss behavior, or ENOSPC behavior. `G2_S2_NATIVE_DISK_OPERATIONS_NO_GO`, `G3_REAL_ENOSPC_NO_GO`, `T10_NATIVE_GATE_BLOCKED_ENVIRONMENT`, `P1A_FULL_NO_GO`, `P1B_PRODUCT_NO_GO`, and `ISSUE_381_PRODUCT_FIX_NO_GO` remain.

The final commit ID, Tree, exact changed-path list, scope audit, push confirmation, and clean-worktree result are provided in the task handoff after commit/push. They are not self-embedded in this file because a Git commit cannot contain a record of its own final object ID or Tree hash without changing that object.
