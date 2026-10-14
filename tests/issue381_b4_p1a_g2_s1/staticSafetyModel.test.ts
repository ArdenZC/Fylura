import { describe, expect, it } from "vitest";
import {
  GIB,
  MAX_HOST_DELTA_BYTES,
  MAX_IMAGE_BYTES,
  MAX_NOMINAL_VOLUME_BYTES,
  assessCleanup,
  advanceCleanup,
  evaluateSafetyPlan,
  nextCleanupIntent,
  type CleanupState,
  type WindowsSafetyPlan,
} from "./staticSafetyModel";
import { makeCleanupState, makeMacSafePlan, makeWindowsSafePlan } from "./fixtures";

function hasIssue(result: ReturnType<typeof evaluateSafetyPlan>, code: string): boolean {
  return result.issues.some((issue) => issue.code === code);
}

describe("G2-S1 pure static safety model", () => {
  it("admits the isolated Windows plan and emits only exact-object, never-executed intents", () => {
    const plan = makeWindowsSafePlan();
    const before = structuredClone(plan);
    const result = evaluateSafetyPlan(plan);

    expect(result.status).toBe("DRY_RUN_SAFE");
    expect(result.executionMode).toBe("STATIC_ONLY");
    expect(result.physicalPassClaimed).toBe(false);
    expect(plan).toEqual(before);
    expect(result.intents.map((intent) => intent.operation)).toEqual([
      "CREATE_FIXED_VHDX_FILE",
      "ATTACH_RETURNED_VHD_OBJECT",
      "INITIALIZE_EXACT_DISK_OBJECT",
      "CREATE_PARTITION_ON_EXACT_DISK",
      "FORMAT_EXACT_PARTITION_NTFS",
      "REGISTER_CLEANUP_OBLIGATION",
    ]);
    expect(result.intents.every((intent) => intent.execution === "NEVER_EXECUTED")).toBe(true);
    expect(result.intents.some((intent) => intent.operation === "REGISTER_CLEANUP_OBLIGATION" && intent.force)).toBe(false);
  });

  it("admits the isolated macOS plan using one structured attach result and exact UDRW origin", () => {
    const result = evaluateSafetyPlan(makeMacSafePlan());

    expect(result.status).toBe("DRY_RUN_SAFE");
    expect(result.executionMode).toBe("STATIC_ONLY");
    expect(result.physicalPassClaimed).toBe(false);
    expect(result.intents.map((intent) => intent.operation)).toEqual([
      "CREATE_UDRW_APFS_IMAGE",
      "ATTACH_EXACT_IMAGE_STRUCTURED",
      "VERIFY_EXACT_APFS_VOLUME",
      "REGISTER_CLEANUP_OBLIGATION",
    ]);
    expect(result.intents.every((intent) => intent.execution === "NEVER_EXECUTED")).toBe(true);
    expect(result.intents.some((intent) => intent.operation.includes("ERASE") || intent.operation.includes("PARTITION"))).toBe(false);
  });

  const commonWindowsRejections: Array<[string, (plan: WindowsSafetyPlan) => void, string]> = [
    ["source hash mismatch", (plan) => { plan.source.observedSha256 = "b".repeat(64); }, "SOURCE_IDENTITY_UNVERIFIED"],
    ["pre-existing backing path", (plan) => { plan.paths.targetExistsBefore = true; }, "TARGET_PREEXISTS"],
    ["root symlink", (plan) => { plan.paths.root.symlinkOrReparsePoint = true; }, "PATH_ROOT_LINK"],
    ["reparse point", (plan) => { plan.paths.sandbox.symlinkOrReparsePoint = true; }, "PATH_SANDBOX_LINK"],
    ["target symlink", (plan) => { plan.paths.target.symlinkOrReparsePoint = true; }, "PATH_TARGET_LINK"],
    ["mount-point reparse point", (plan) => { plan.paths.mountPoint.symlinkOrReparsePoint = true; }, "PATH_MOUNTPOINT_LINK"],
    ["path escape", (plan) => { plan.paths.target.lexical = "C:\\outside\\disk.vhdx"; plan.paths.target.canonical = "C:/outside/disk.vhdx"; }, "TARGET_ESCAPES_SANDBOX"],
    ["unowned sandbox", (plan) => { plan.paths.observedOwnerToken = "someone-else"; }, "SANDBOX_OWNERSHIP_UNPROVEN"],
    ["non-unique sandbox", (plan) => { plan.paths.sandboxUnique = false; }, "SANDBOX_OWNERSHIP_UNPROVEN"],
    ["dynamic allocation", (plan) => { plan.capacity.allocation = "dynamic"; }, "ALLOCATION_NOT_BOUNDED"],
    ["sparse allocation", (plan) => { plan.capacity.sparse = true; }, "ALLOCATION_NOT_BOUNDED"],
    ["unbounded growth", (plan) => { plan.capacity.unboundedGrowth = true; }, "ALLOCATION_NOT_BOUNDED"],
    ["source partition import", (plan) => { plan.capacity.importsExistingStorage = true; }, "EXISTING_STORAGE_IMPORT_FORBIDDEN"],
    ["physical device target", (plan) => { plan.capacity.targetKind = "physical-device"; }, "NON_FILE_TARGET_FORBIDDEN"],
    ["nominal volume over 256 MiB", (plan) => { plan.capacity.nominalVolumeBytes = MAX_NOMINAL_VOLUME_BYTES + 1; }, "NOMINAL_VOLUME_OUT_OF_BOUNDS"],
    ["image cap over 320 MiB", (plan) => { plan.capacity.maximumImageBytes = MAX_IMAGE_BYTES + 1; }, "IMAGE_CAP_OUT_OF_BOUNDS"],
    ["observed image over cap", (plan) => { plan.capacity.observedImageBytes = MAX_IMAGE_BYTES + 1; }, "OBSERVED_IMAGE_OUT_OF_BOUNDS"],
    ["host delta cap over 320 MiB", (plan) => { plan.capacity.maximumHostDeltaBytes = MAX_HOST_DELTA_BYTES + 1; }, "HOST_DELTA_CAP_OUT_OF_BOUNDS"],
    ["observed host delta over cap", (plan) => { plan.capacity.observedHostDeltaBytes = MAX_HOST_DELTA_BYTES + 1; }, "OBSERVED_HOST_DELTA_OUT_OF_BOUNDS"],
    ["overstated independent free space", (plan) => { plan.after!.availableBytes = 9 * GIB; }, "HOST_CAPACITY_AFTER_OVERSTATED"],
    ["TOCTOU host fingerprint change", (plan) => { plan.after!.hostFingerprint = "changed-host"; }, "HOST_IDENTITY_CHANGED"],
    ["failed source/document crosscheck", (plan) => { plan.inventoryEvidence.crosscheck = "FAIL"; }, "SOURCE_CROSSCHECK_NOT_PASSED"],
    ["missing before inventory", (plan) => { plan.inventoryEvidence.beforeMutationCaptured = false; }, "INVENTORY_EVIDENCE_MISSING"],
    ["missing post-plan inventory", (plan) => { plan.inventoryEvidence.afterPlanCaptured = false; }, "INVENTORY_EVIDENCE_MISSING"],
    ["default selector", (plan) => { plan.selection = "implicit-default"; }, "WINDOWS_SELECTOR_UNSAFE"],
    ["disk-number selector", (plan) => { plan.selection = "disk-number"; }, "WINDOWS_SELECTOR_UNSAFE"],
    ["friendly-name selector", (plan) => { plan.selection = "friendly-name"; }, "WINDOWS_SELECTOR_UNSAFE"],
    ["drive-letter selector", (plan) => { plan.selection = "drive-letter"; }, "WINDOWS_SELECTOR_UNSAFE"],
    ["broad selector", (plan) => { plan.selection = "broad-selector"; }, "WINDOWS_SELECTOR_UNSAFE"],
    ["drive-letter assignment", (plan) => { plan.vhd.driveLetterAssigned = true; }, "WINDOWS_DRIVE_LETTER_FORBIDDEN"],
  ];

  it.each(commonWindowsRejections)("rejects common unsafe plan: %s without emitting intents", (_label, mutate, code) => {
    const plan = makeWindowsSafePlan();
    mutate(plan);
    const result = evaluateSafetyPlan(plan);

    expect(result.status).not.toBe("DRY_RUN_SAFE");
    expect(hasIssue(result, code)).toBe(true);
    expect(result.intents).toEqual([]);
    expect(result.physicalPassClaimed).toBe(false);
  });

  it("classifies unknown backing or free-space as unavailable rather than safe", () => {
    const backing = makeWindowsSafePlan();
    backing.capacity.backingKnown = false;
    const unknownBefore = makeWindowsSafePlan();
    unknownBefore.before!.availableBytes = null;
    const unknownAfterCap = makeWindowsSafePlan();
    unknownAfterCap.after!.independentAvailableCapBytes = null;

    for (const plan of [backing, unknownBefore, unknownAfterCap]) {
      const result = evaluateSafetyPlan(plan);
      expect(result.status).toBe("ENVIRONMENT_UNAVAILABLE");
      expect(result.intents).toEqual([]);
    }
  });

  it("fails closed when either pre- or post-plan free space is below 4 GiB", () => {
    const before = makeWindowsSafePlan();
    before.before!.availableBytes = 4 * GIB - 1;
    const after = makeWindowsSafePlan();
    after.after!.independentAvailableCapBytes = 4 * GIB - 1;

    for (const plan of [before, after]) {
      const result = evaluateSafetyPlan(plan);
      expect(result.status).toBe("ENVIRONMENT_UNAVAILABLE");
      expect(result.intents).toEqual([]);
    }
  });

  it("rejects missing or failed static positive-control evidence with distinct outcomes", () => {
    const missing = makeWindowsSafePlan();
    missing.positiveControl = "MISSING";
    const failed = makeWindowsSafePlan();
    failed.positiveControl = "FAIL";

    const missingResult = evaluateSafetyPlan(missing);
    const failedResult = evaluateSafetyPlan(failed);
    expect(missingResult.status).toBe("SAFETY_REJECTION");
    expect(failedResult.status).toBe("FAIL");
    expect(missingResult.intents).toEqual([]);
    expect(failedResult.intents).toEqual([]);
  });

  it("rejects G1 Windows system and data disk fingerprints as targets", () => {
    const systemDisk = makeWindowsSafePlan();
    systemDisk.disksAfter[2].fingerprint = systemDisk.disksBefore[0].fingerprint;
    const dataDisk = makeWindowsSafePlan();
    dataDisk.disksAfter[2].fingerprint = dataDisk.disksBefore[1].fingerprint;

    for (const plan of [systemDisk, dataDisk]) {
      const result = evaluateSafetyPlan(plan);
      expect(result.status).toBe("SAFETY_REJECTION");
      expect(hasIssue(result, "WINDOWS_BASELINE_HOST_DISK_TARGET")).toBe(true);
      expect(result.intents).toEqual([]);
    }
    expect(systemDisk.disksBefore[0].friendlyName).toBe("Virtual_Disk NVME Premium");
    expect(dataDisk.disksBefore[1].friendlyName).toBe("Microsoft NVMe Direct Disk v2");
  });

  it("rejects unknown baseline or post-plan inventory identities", () => {
    const windows = makeWindowsSafePlan();
    windows.disksBefore[0].identity = null;
    const mac = makeMacSafePlan();
    mac.devicesAfter[2].fingerprint = null;

    const windowsResult = evaluateSafetyPlan(windows);
    const macResult = evaluateSafetyPlan(mac);
    expect(hasIssue(windowsResult, "WINDOWS_BASELINE_IDENTITY_UNPROVEN")).toBe(true);
    expect(hasIssue(macResult, "MAC_POST_INVENTORY_IDENTITY_UNPROVEN")).toBe(true);
    expect(windowsResult.intents).toEqual([]);
    expect(macResult.intents).toEqual([]);
  });

  it("rejects missing VHD return identity and a missing disk identity", () => {
    const missingVhd = makeWindowsSafePlan();
    missingVhd.vhd.returnedObjectIdentity = null;
    const missingDisk = makeWindowsSafePlan();
    missingDisk.vhd.attachedDiskIdentities = [null];

    const vhdResult = evaluateSafetyPlan(missingVhd);
    const diskResult = evaluateSafetyPlan(missingDisk);
    expect(hasIssue(vhdResult, "WINDOWS_VHD_RETURN_IDENTITY_MISSING")).toBe(true);
    expect(hasIssue(diskResult, "WINDOWS_RETURNED_DISK_IDENTITY_AMBIGUOUS")).toBe(true);
    expect(vhdResult.intents).toEqual([]);
    expect(diskResult.intents).toEqual([]);
  });

  it("rejects two newly returned Windows disks and disk-number initialization", () => {
    const multiple = makeWindowsSafePlan();
    const second = { ...multiple.disksAfter[2], identity: "unexpected-second-disk", fingerprint: "second-fingerprint" };
    multiple.disksAfter.push(second);
    multiple.vhd.attachedDiskIdentities.push(second.identity);
    const diskNumber = makeWindowsSafePlan();
    diskNumber.initializeDiskIdentity = "7";

    const multipleResult = evaluateSafetyPlan(multiple);
    const diskNumberResult = evaluateSafetyPlan(diskNumber);
    expect(hasIssue(multipleResult, "WINDOWS_NEW_DISK_CARDINALITY_OR_BINDING")).toBe(true);
    expect(hasIssue(diskNumberResult, "WINDOWS_INITIALIZE_TARGET_MISMATCH")).toBe(true);
    expect(multipleResult.intents).toEqual([]);
    expect(diskNumberResult.intents).toEqual([]);
  });

  it.each([
    ["non-RAW returned disk", (plan: WindowsSafetyPlan) => { plan.disksAfter[2].partitionStyle = "GPT"; }, "WINDOWS_DISK_NOT_RAW"],
    ["wrong virtual disk size", (plan: WindowsSafetyPlan) => { plan.disksAfter[2].sizeBytes = MAX_NOMINAL_VOLUME_BYTES - 1; }, "WINDOWS_DISK_SIZE_MISMATCH"],
    ["boot disk", (plan: WindowsSafetyPlan) => { plan.disksAfter[2].isBoot = true; }, "WINDOWS_HOST_DISK_TARGET"],
    ["unbound partition", (plan: WindowsSafetyPlan) => { plan.partition!.diskIdentity = "other-disk"; }, "WINDOWS_PARTITION_BINDING_MISMATCH"],
    ["wrong partition formatting target", (plan: WindowsSafetyPlan) => { plan.format!.partitionIdentity = "other-partition"; }, "WINDOWS_FORMAT_BINDING_OR_FILESYSTEM_MISMATCH"],
    ["wrong filesystem", (plan: WindowsSafetyPlan) => { plan.format!.fileSystem = "exFAT"; }, "WINDOWS_FORMAT_BINDING_OR_FILESYSTEM_MISMATCH"],
  ] as Array<[string, (plan: WindowsSafetyPlan) => void, string]>) ("rejects Windows identity or format failure: %s", (_label, mutate, code) => {
    const plan = makeWindowsSafePlan();
    mutate(plan);
    const result = evaluateSafetyPlan(plan);
    expect(result.status).toBe("SAFETY_REJECTION");
    expect(hasIssue(result, code)).toBe(true);
    expect(result.intents).toEqual([]);
  });

  it("rejects the exact G1 macOS host Data volume fingerprint as an image target", () => {
    const plan = makeMacSafePlan();
    plan.devicesAfter[2].fingerprint = "g1:/dev/disk2s5:Data:APFS:/System/Volumes/Data";
    const result = evaluateSafetyPlan(plan);

    expect(plan.devicesBefore.some((device) => device.identity === "/dev/disk2s5")).toBe(true);
    expect(result.status).toBe("SAFETY_REJECTION");
    expect(hasIssue(result, "MAC_HOST_DEVICE_TARGET")).toBe(true);
    expect(result.intents).toEqual([]);
  });

  it("rejects a macOS attach result that names an existing G1 host device", () => {
    const plan = makeMacSafePlan();
    plan.attachResults[0].deviceIdentity = "/dev/disk2s5";
    plan.devicesAfter = [...plan.devicesBefore];
    const result = evaluateSafetyPlan(plan);

    expect(result.status).toBe("SAFETY_REJECTION");
    expect(hasIssue(result, "MAC_NEW_DEVICE_CARDINALITY_OR_BINDING")).toBe(true);
    expect(result.intents).toEqual([]);
  });

  it.each([
    ["missing attach identity", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.attachResults[0].deviceIdentity = null; }, "MAC_ATTACH_IDENTITY_MISSING"],
    ["double mount", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.attachResults[0].mountPoints.push(plan.attachResults[0].mountPoints[0]); }, "MAC_ATTACH_CARDINALITY_OR_DOUBLE_MOUNT"],
    ["multiple attach results", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.attachResults.push({ ...plan.attachResults[0], recordIdentity: "second-attach" }); }, "MAC_ATTACH_CARDINALITY_OR_DOUBLE_MOUNT"],
    ["wrong backing image origin", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.attachResults[0].sourceImagePath = "/tmp/foreign.dmg"; }, "MAC_IMAGE_ORIGIN_MISMATCH"],
    ["outside mount point", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.attachResults[0].mountPoints[0] = "/Volumes/runner"; }, "MAC_MOUNTPOINT_AMBIGUOUS_OR_MISMATCH"],
    ["mount-point reparse path", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.paths.mountPoint.symlinkOrReparsePoint = true; }, "PATH_MOUNTPOINT_LINK"],
    ["guessed device selector", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.selection = "guessed-device-path"; }, "MAC_SELECTOR_UNSAFE"],
    ["broad device selector", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.selection = "broad-selector"; }, "MAC_SELECTOR_UNSAFE"],
    ["implicit device default", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.selection = "implicit-default"; }, "MAC_SELECTOR_UNSAFE"],
    ["non-empty mount point", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.attachResults[0].mountPointEmpty = false; }, "MAC_VOLUME_VERIFICATION_FAILED"],
    ["wrong filesystem", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.attachResults[0].fileSystem = "HFS+"; }, "MAC_VOLUME_VERIFICATION_FAILED"],
    ["wrong image format", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.image.format = "UDIF"; }, "MAC_IMAGE_FORMAT_UNSAFE"],
    ["wrong image path", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.image.exactPath = "/tmp/foreign.dmg"; }, "MAC_IMAGE_PATH_MISMATCH"],
    ["sparse image", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.image.sparse = true; }, "MAC_IMAGE_FORMAT_UNSAFE"],
    ["global disk operation", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.globalEraseOrPartitionIntent = true; }, "MAC_GLOBAL_DISK_OPERATION_FORBIDDEN"],
  ] as Array<[string, (plan: ReturnType<typeof makeMacSafePlan>) => void, string]>) ("rejects macOS origin, identity, mount or filesystem failure: %s", (_label, mutate, code) => {
    const plan = makeMacSafePlan();
    mutate(plan);
    const result = evaluateSafetyPlan(plan);
    expect(result.status).toBe("SAFETY_REJECTION");
    expect(hasIssue(result, code)).toBe(true);
    expect(result.intents).toEqual([]);
  });
});

describe("G2-S1 exact-resource cleanup state machine", () => {
  it("requires unmount, known-device detach, verified release, then exact owned-image removal", () => {
    let state = makeCleanupState();
    const unmount = nextCleanupIntent(state);
    expect(unmount).toMatchObject({ operation: "UNMOUNT_EXACT_MOUNTPOINT", targetIdentity: state.mountPoint, force: false, execution: "NEVER_EXECUTED" });
    state = advanceCleanup(state, { kind: "unmount", targetIdentity: state.mountPoint, succeeded: true, confirmedUnmounted: true });

    const detach = nextCleanupIntent(state);
    expect(detach).toMatchObject({ operation: "DETACH_KNOWN_DEVICE", targetIdentity: "/dev/disk9s1", force: false });
    state = advanceCleanup(state, { kind: "detach", targetIdentity: "/dev/disk9s1", succeeded: true, forced: false });

    const verifyRelease = nextCleanupIntent(state);
    expect(verifyRelease).toMatchObject({ operation: "VERIFY_DEVICE_RELEASE", targetIdentity: "/dev/disk9s1", force: false });
    state = advanceCleanup(state, { kind: "release-check", targetIdentity: "/dev/disk9s1", identityAbsent: true });

    const remove = nextCleanupIntent(state);
    expect(remove).toMatchObject({ operation: "REMOVE_EXACT_OWNED_IMAGE", targetIdentity: state.imagePath, force: false });
    state = advanceCleanup(state, { kind: "remove-image", targetIdentity: state.imagePath, succeeded: true });
    expect(state.phase).toBe("REMOVED");
    expect(state.originalFailure).toBe("simulated verification failure");
    expect(assessCleanup(state)).toMatchObject({
      status: "FAIL",
      cleanupStatus: "DRY_RUN_SAFE",
      originalFailure: "simulated verification failure",
      physicalPassClaimed: false,
    });
    expect(nextCleanupIntent(state)).toBeNull();
  });

  it("keeps the original failure and blocks removal when release cannot be proven", () => {
    let state = makeCleanupState();
    state = advanceCleanup(state, { kind: "unmount", targetIdentity: state.mountPoint, succeeded: true, confirmedUnmounted: true });
    state = advanceCleanup(state, { kind: "detach", targetIdentity: state.deviceIdentity, succeeded: true, forced: false });
    state = advanceCleanup(state, { kind: "release-check", targetIdentity: state.deviceIdentity, identityAbsent: false });

    expect(state.phase).toBe("CLEANUP_INCOMPLETE");
    expect(state.cleanupFailure).toBe("device release could not be proven");
    expect(state.originalFailure).toBe("simulated verification failure");
    expect(assessCleanup(state)).toMatchObject({
      status: "FAIL",
      cleanupStatus: "CLEANUP_INCOMPLETE",
      cleanupFailure: "device release could not be proven",
    });
    expect(nextCleanupIntent(state)).toBeNull();
  });

  it("cleans up Windows by returned VHD object identity without requiring a drive-letter selector", () => {
    const sandboxPath = "C:\\Temp\\zen-g2-s1\\case-001";
    let state: CleanupState = {
      ...makeCleanupState(),
      platform: "windows" as const,
      phase: "ATTACHED" as const,
      sandboxPath,
      imagePath: `${sandboxPath}\\bounded.vhdx`,
      mountPoint: `${sandboxPath}\\no-drive-letter-target`,
      deviceIdentity: "returned-vhd-object-001",
    };

    expect(nextCleanupIntent(state)).toMatchObject({
      operation: "DETACH_KNOWN_DEVICE",
      targetIdentity: "returned-vhd-object-001",
      force: false,
    });
    state = advanceCleanup(state, { kind: "detach", targetIdentity: "returned-vhd-object-001", succeeded: true, forced: false });
    expect(state.phase).toBe("DETACHED");
    state = advanceCleanup(state, { kind: "release-check", targetIdentity: "returned-vhd-object-001", identityAbsent: true });
    expect(state.phase).toBe("RELEASE_VERIFIED");
    expect(nextCleanupIntent(state)).toMatchObject({ operation: "REMOVE_EXACT_OWNED_IMAGE", targetIdentity: state.imagePath, force: false });
  });

  it("reports exact image removal failure while retaining the original modeled failure", () => {
    let state = makeCleanupState();
    state = advanceCleanup(state, { kind: "unmount", targetIdentity: state.mountPoint, succeeded: true, confirmedUnmounted: true });
    state = advanceCleanup(state, { kind: "detach", targetIdentity: state.deviceIdentity, succeeded: true, forced: false });
    state = advanceCleanup(state, { kind: "release-check", targetIdentity: state.deviceIdentity, identityAbsent: true });
    state = advanceCleanup(state, { kind: "remove-image", targetIdentity: state.imagePath, succeeded: false });

    expect(state.phase).toBe("CLEANUP_INCOMPLETE");
    expect(state.originalFailure).toBe("simulated verification failure");
    expect(state.cleanupFailure).toBe("exact owned image removal failed");
    expect(assessCleanup(state).status).toBe("FAIL");
    expect(assessCleanup(state).cleanupStatus).toBe("CLEANUP_INCOMPLETE");
    expect(nextCleanupIntent(state)).toBeNull();
  });

  it.each([
    ["failed unmount", (state: ReturnType<typeof makeCleanupState>) => advanceCleanup(state, { kind: "unmount", targetIdentity: state.mountPoint, succeeded: false, confirmedUnmounted: false })],
    ["forced detach", (state: ReturnType<typeof makeCleanupState>) => {
      const unmounted = advanceCleanup(state, { kind: "unmount", targetIdentity: state.mountPoint, succeeded: true, confirmedUnmounted: true });
      return advanceCleanup(unmounted, { kind: "detach", targetIdentity: state.deviceIdentity, succeeded: true, forced: true });
    }],
    ["wrong resource identity", (state: ReturnType<typeof makeCleanupState>) => advanceCleanup(state, { kind: "unmount", targetIdentity: "/Volumes/runner", succeeded: true, confirmedUnmounted: true })],
  ] as Array<[string, (state: ReturnType<typeof makeCleanupState>) => ReturnType<typeof makeCleanupState>]>) ("leaves cleanup incomplete without masking original failure: %s", (_label, fail) => {
    const state = fail(makeCleanupState());
    expect(state.phase).toBe("CLEANUP_INCOMPLETE");
    expect(state.originalFailure).toBe("simulated verification failure");
    expect(nextCleanupIntent(state)).toBeNull();
  });

  it("refuses cleanup if mount/image identity, ownership, or containment changes", () => {
    const unowned = makeCleanupState();
    unowned.observedOwnerToken = "different-owner";
    const escaped = makeCleanupState();
    escaped.imagePath = "/tmp/foreign.dmg";
    const linkedMount = makeCleanupState();
    linkedMount.mountPointIsSymlinkOrReparsePoint = true;

    for (const state of [unowned, escaped, linkedMount]) {
      expect(nextCleanupIntent(state)).toBeNull();
      expect(advanceCleanup(state, { kind: "unmount", targetIdentity: state.mountPoint, succeeded: true, confirmedUnmounted: true }).phase).toBe("CLEANUP_INCOMPLETE");
    }
  });

  it("does not trust a claimed release phase without the state-machine proofs", () => {
    const forged = {
      ...makeCleanupState(),
      phase: "RELEASE_VERIFIED" as const,
      detachedProven: true,
      releaseProven: false,
    };
    expect(nextCleanupIntent(forged)).toBeNull();
    expect(advanceCleanup(forged, { kind: "remove-image", targetIdentity: forged.imagePath, succeeded: true }).phase).toBe("CLEANUP_INCOMPLETE");
  });
});
