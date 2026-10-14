# Issue #414 T10-B2-G2-S1 — Static Safety Model

## Purpose and boundary

This directory records the isolated, deterministic safety-contract model for G2-S1. The authorized branch is `research/issue-414-b2-g2-s1-static-safety`, based on Owner-verified master `26e4255f487b61a4f985491f757e65631739646d` (Tree `956f49af8a9aad3d5ac88eb42d4c72be137165cf`). The separately sealed G1 branch and its evidence were read only; none of that evidence was copied or changed.

The executable code lives only in `tests/issue381_b4_p1a_g2_s1/`. It uses plain in-memory records and pure decision functions. It imports no filesystem, child-process, PowerShell, Hyper-V, DiskPart, `hdiutil`, or `diskutil` API. Every emitted operation is typed data tagged `NEVER_EXECUTED`; there is no dispatcher or command string builder. No disk, image, partition, volume, mount, or host storage was created or changed.

`DRY_RUN_SAFE` means only that the supplied synthetic facts pass this static model and that a bounded dry-run intent list can be described. It is not a physical `PASS`, environment capability result, G2 execution, durability claim, or authorization for G2-S2/G3.

## Common admission rules

- Match a 64-character lower-case SHA-256 source identity.
- Require an exact canonical absolute root, one unique owned sandbox child, exact contained image and mount paths, matching ownership tokens, no symlink/reparse points, and an absent image target.
- Require pre- and post-plan host identity snapshots with unchanged host fingerprint, known free-space evidence, and independent caps. Both reported and independently bounded free space must be at least 4 GiB; a reported value above its independent cap is rejected.
- Every free-space, capacity, image-size, disk-size, and image-owned device-size input must be present, finite, a positive safe integer byte count, and within its applicable cap. `NaN`, either infinity, negatives, zero, fractions, unsafe integers, missing values, and wrong runtime types fail closed. Missing host/capacity evidence is `ENVIRONMENT_UNAVAILABLE`; invalid numeric values are rejected with no intents.
- Require a known backing store, fixed non-sparse bounded allocation, no source disk/partition import, and a file target. Nominal virtual volume is at most 256 MiB; image and host-delta caps are each at most 320 MiB. Observed image and host delta must remain positive and within their caps. The post-plan virtual disk must match the nominal volume; host baseline disk sizes are validated as positive safe integers but are not confused with the new virtual-image cap.
- Require pre-mutation and post-plan inventories and an explicit source/document crosscheck. Missing capacity/backing evidence returns `ENVIRONMENT_UNAVAILABLE`; a failed crosscheck or unsafe identity returns `SAFETY_REJECTION`; a failed positive control returns `FAIL`.
- Any rejected plan emits an empty intent list. The assessment always declares `executionMode: STATIC_ONLY` and `physicalPassClaimed: false`.

## Windows model

The model describes this order: create a fixed VHDX at the exact new sandbox path; bind attach to the returned VHD object with no drive letter; observe exactly one disk absent from the pre-mutation inventory; prove its stable identity/fingerprint, expected size, non-system/non-boot/non-OS role, and `RAW` style; bind initialization and partition creation to that exact disk identity; then bind NTFS formatting to the returned partition identity. Disk number, friendly name, drive letter, broad selector, and implicit default are invalid target selectors. Every pre-mutation disk must appear after the plan with the exact same identity, fingerprint, disk number, name, size, partition style, and host-role flags. Inventory identities, fingerprints, and disk numbers must be unique; Windows identity/fingerprint uniqueness is case-insensitive. A baseline fingerprint match or a case-only identity reuse is rejected.

Fixtures include both G1 Windows host records: disk 0 `Virtual_Disk NVME Premium` (boot/system) and disk 1 `Microsoft NVMe Direct Disk v2` (data). The test fingerprints are synthetic composites of G1's captured disk number, name, size, partition style, and role; G1 did not provide a hardware serial fingerprint. They are deny-list negative controls, not targets or copied G1 artifacts.

## macOS model

The model represents a structured attach as one result envelope, not one device: the envelope binds the exact created-image identity and creation-record identity to one image root and enumerates every new image-owned node. The graph may include an image root, partitions, APFS containers, and one or more APFS-volume nodes. There must be exactly one root; each child has one allowed parent inside that graph; every node is unique, reachable from the root, non-host, and explicitly associated with the exact returned image identity. The structured result must enumerate all and only new nodes. Orphans, cycles, host-parent links, baseline identity/fingerprint reuse, duplicate rows, incomplete enumerations, and mixed/stale image origins are rejected.

The intended mount binding names one APFS-volume node from that same graph, not the root, partition, or a host volume. It must map to the one exact contained empty mount point and report mounted APFS. Additional same-image APFS-volume records may be represented, but they must be included in the graph and cannot replace or ambiguously alias the selected mount. Every baseline device—including `/dev/disk2s5` and its modeled `/dev/disk2` parent—must remain unchanged in the post inventory. The model does not claim that this synthetic graph is the exact plist shape on a particular macOS release: the Apple command-output contract and runner behavior remain unverified. Every assessment therefore carries `s2Eligibility: NOT_QUALIFIED`; `DRY_RUN_SAFE` means only that the synthetic graph satisfies this static contract, never S2 permission.

The negative fixture binds the G1 runner Data volume tuple `/dev/disk2s5`, `Data`, `APFS`, `/System/Volumes/Data`. This tuple was read from the archived G1 attempt-002 read-only inventory on `research/issue-414-b2-g1-hosted-preflight`; the composite is only a synthetic deny-list identity in this test, not an immutable device serial.

## Cleanup state machine

Cleanup sessions are opaque frozen handles backed by a module-private `WeakMap`; callers cannot construct a phase or mutate proof booleans to authorize deletion. Each `NEVER_EXECUTED`, `force: false` action is an opaque intent registered to one session and sequence. Advancing requires the exact current intent plus a matching event for the expected action, target, device identity, image identity, and owner token. Replaying, skipping, reversing, cross-session, stale, failed, or mismatched events make cleanup incomplete. macOS requires unmount → detach → confirmed absence → exact owned-image removal; Windows requires detach → confirmed absence → exact owned-image removal. A cleanup failure retains the original operation failure.

These events are synthetic test inputs, not cryptographic or OS-backed observations. The opaque sequence proves that the in-memory decision function cannot be advanced by supplying a later phase or proof flags alone; it does not prove that a real provider performed any transition.

`assessCleanup()` reports the original modeled operation failure separately from cleanup status. Thus a cleanup failure cannot replace the original failure: the overall status remains `FAIL` when an original failure exists, while the cleanup field reports `CLEANUP_INCOMPLETE`. No image-removal intent is available until release is verified.

## Validation and evidence

The original focused matrix and raw runner output remain in `evidence/attempt-001/` byte-identically. R1 RED/GREEN Vitest and strict TypeScript logs, receipt, and SHA-256 list are appended under `evidence/attempt-002/`. The model was run on a Linux development environment through Vitest; that proves only deterministic TypeScript model behavior. It does not verify Windows/macOS provider behavior, filesystem durability, host free-space reporting, actual symlink/reparse resolution, real VHDX/UDRW allocation, detach semantics, power-loss recovery, or physical ENOSPC behavior.

Owner-provided official-document references and the model-to-source crosswalk are recorded in [SOURCE-CHECK.md](SOURCE-CHECK.md). The documented semantics remain review inputs, not command-level certification. The static fixture's `crosscheck: PASS` is a synthetic precondition used to verify that a missing/failed check is fail-closed; it is not evidence that the source pages were live-probed by the test.

## Owner review checklist before any S2 proposal

1. Re-read the Microsoft and Apple references in `SOURCE-CHECK.md`; confirm provider/version-specific parameter and structured-output behavior independently.
2. Confirm the platform-specific baseline continuity checks and the new disk/device numeric bounds against provider-specific inventories.
3. Review the synthetic macOS hierarchy model against canonical Apple documentation and the selected macOS release's structured output; until then its capability remains `NOT_QUALIFIED` and S2 stays NO-GO.
4. Review the sequence-bound cleanup contract and remember that its events are synthetic inputs, not native release proof.
5. Review both attempt manifests, raw logs, exact branch HEAD/Tree, allowlisted diff, and the byte-identical attempt-001 receipt.
6. Keep G1, T10-native, P1A, P1B, and Issue #381 product-fix blockers unchanged. S1 review does not authorize S2, G3, CI/workflow work, or native disk actions.
