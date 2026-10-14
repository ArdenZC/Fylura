# Issue #414 T10-B2-G2-S1 — Static Safety Model

## Purpose and boundary

This directory records the isolated, deterministic safety-contract model for G2-S1. The authorized branch is `research/issue-414-b2-g2-s1-static-safety`, based on Owner-verified master `26e4255f487b61a4f985491f757e65631739646d` (Tree `956f49af8a9aad3d5ac88eb42d4c72be137165cf`). The separately sealed G1 branch and its evidence were read only; none of that evidence was copied or changed.

The executable code lives only in `tests/issue381_b4_p1a_g2_s1/`. It uses plain in-memory records and pure decision functions. It imports no filesystem, child-process, PowerShell, Hyper-V, DiskPart, `hdiutil`, or `diskutil` API. Every emitted operation is typed data tagged `NEVER_EXECUTED`; there is no dispatcher or command string builder. No disk, image, partition, volume, mount, or host storage was created or changed.

`DRY_RUN_SAFE` means only that the supplied synthetic facts pass this static model and that a bounded dry-run intent list can be described. It is not a physical `PASS`, environment capability result, G2 execution, durability claim, or authorization for G2-S2/G3.

## Common admission rules

- Match a 64-character lower-case SHA-256 source identity.
- Require an exact canonical absolute root, one unique owned sandbox child, exact contained image and mount paths, matching ownership tokens, no symlink/reparse points, and an absent image target.
- Require pre- and post-plan host identity snapshots with unchanged host fingerprint, known free-space evidence, and independent caps. Both reported and independently bounded free space must be at least 4 GiB; a reported value above its independent cap is rejected.
- Require a known backing store, fixed non-sparse bounded allocation, no source disk/partition import, and a file target. Nominal volume is at most 256 MiB; image and host-delta caps are each at most 320 MiB. Observed image and host delta must remain within their caps.
- Require pre-mutation and post-plan inventories and an explicit source/document crosscheck. Missing capacity/backing evidence returns `ENVIRONMENT_UNAVAILABLE`; a failed crosscheck or unsafe identity returns `SAFETY_REJECTION`; a failed positive control returns `FAIL`.
- Any rejected plan emits an empty intent list. The assessment always declares `executionMode: STATIC_ONLY` and `physicalPassClaimed: false`.

## Windows model

The model describes this order: create a fixed VHDX at the exact new sandbox path; bind attach to the returned VHD object with no drive letter; observe exactly one disk absent from the pre-mutation inventory; prove its stable identity/fingerprint, expected size, non-system/non-boot/non-OS role, and `RAW` style; bind initialization and partition creation to that exact disk identity; then bind NTFS formatting to the returned partition identity. Disk number, friendly name, drive letter, broad selector, and implicit default are invalid target selectors. A baseline fingerprint match is rejected even if a later record claims a different identity.

Fixtures include both G1 Windows host records: disk 0 `Virtual_Disk NVME Premium` (boot/system) and disk 1 `Microsoft NVMe Direct Disk v2` (data). The test fingerprints are synthetic composites of G1's captured disk number, name, size, partition style, and role; G1 did not provide a hardware serial fingerprint. They are deny-list negative controls, not targets or copied G1 artifacts.

## macOS model

The model allows only a non-sparse UDRW image at the exact new sandbox path and one structured attach result. Its device identity must be absent from the baseline; its backing origin must resolve to the exact image; it must identify exactly one contained, empty mount point and a mounted APFS volume. Baseline/internal host fingerprints, guessed device paths, multiple attach results, multiple mount points, and global erase/partition intent are rejected.

The negative fixture binds the G1 runner Data volume tuple `/dev/disk2s5`, `Data`, `APFS`, `/System/Volumes/Data`. This tuple was read from the archived G1 attempt-002 read-only inventory on `research/issue-414-b2-g1-hosted-preflight`; the composite is only a synthetic deny-list identity in this test, not an immutable device serial.

## Cleanup state machine

Cleanup actions are also descriptions tagged `NEVER_EXECUTED` and `force: false`. macOS proceeds through exact mount-point unmount, known returned-device detach, proof that the identity is absent, and removal of the exact owned image. Windows proceeds from the attached VHD object to exact-object detach, absence verification, and exact owned-image removal; its plan does not require a drive letter. State transition evidence is carried separately for unmount, detach, and release proof. A forged later phase, mismatched resource, escaped path, ownership mismatch, symlink/reparse point, forced detach, unmount failure, or unproven release blocks deletion.

`assessCleanup()` reports the original modeled operation failure separately from cleanup status. Thus a cleanup failure cannot replace the original failure: the overall status remains `FAIL` when an original failure exists, while the cleanup field reports `CLEANUP_INCOMPLETE`. No image-removal intent is available until release is verified.

## Validation and evidence

The focused matrix and unedited local runner output are in `evidence/attempt-001/`. The model was run on a Linux development environment through Vitest; that proves only deterministic TypeScript model behavior. It does not verify Windows/macOS provider behavior, filesystem durability, host free-space reporting, actual symlink/reparse resolution, real VHDX/UDRW allocation, detach semantics, power-loss recovery, or physical ENOSPC behavior.

Owner-provided official-document references and the model-to-source crosswalk are recorded in [SOURCE-CHECK.md](SOURCE-CHECK.md). The documented semantics remain review inputs, not command-level certification. The static fixture's `crosscheck: PASS` is a synthetic precondition used to verify that a missing/failed check is fail-closed; it is not evidence that the source pages were live-probed by the test.

## Owner review checklist before any S2 proposal

1. Re-read the Microsoft and Apple references in `SOURCE-CHECK.md`; confirm provider/version-specific parameter and structured-output behavior independently.
2. Review every validator and ensure no returned identity can be confused with a baseline host device, stale object, guessed number, or path alias.
3. Decide whether identity fingerprints, mount-point proof, free-space independent cap, and Windows no-drive-letter cleanup obligations are sufficiently defined for a separate S2 contract.
4. Review `attempt-001/SHA256SUMS.txt`, source/test hashes, exact branch HEAD/Tree and allowlisted diff.
5. Keep G1, T10-native, P1A, P1B, and Issue #381 product-fix blockers unchanged. S1 review does not authorize S2, G3, CI/workflow work, or native disk actions.
