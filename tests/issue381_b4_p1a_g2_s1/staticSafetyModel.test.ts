import { describe, expect, it } from "vitest";
import {
  GIB,
  MAX_HOST_DELTA_BYTES,
  MAX_IMAGE_BYTES,
  MAX_NOMINAL_VOLUME_BYTES,
  createCleanupSession,
  assessCleanup,
  advanceCleanup,
  inspectCleanup,
  evaluateSafetyPlan,
  nextCleanupIntent,
  type CleanupEvent,
  type CleanupIntent,
  type CleanupSession,
  type WindowsSafetyPlan,
} from "./staticSafetyModel";
import { makeCleanupState, makeMacSafePlan, makeWindowsCleanupState, makeWindowsSafePlan } from "./fixtures";

function hasIssue(result: ReturnType<typeof evaluateSafetyPlan>, code: string): boolean {
  return result.issues.some((issue) => issue.code === code);
}

function macImageRoot(plan: ReturnType<typeof makeMacSafePlan>) {
  return plan.devicesAfter.find((device) => device.deviceRole === "image-root")!;
}

function macImageVolume(plan: ReturnType<typeof makeMacSafePlan>) {
  return plan.devicesAfter.find((device) => device.deviceRole === "apfs-volume")!;
}

describe("G2-S1 pure static safety model", () => {
  it("admits the isolated Windows plan and emits only exact-object, never-executed intents", () => {
    const plan = makeWindowsSafePlan();
    const before = structuredClone(plan);
    const result = evaluateSafetyPlan(plan);

    expect(result.status).toBe("DRY_RUN_SAFE");
    expect(result.executionMode).toBe("STATIC_ONLY");
    expect(result.physicalPassClaimed).toBe(false);
    expect(result.s2Eligibility).toBe("NOT_QUALIFIED");
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
    expect(result.s2Eligibility).toBe("NOT_QUALIFIED");
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

  const invalidNumericValues: Array<[string, unknown]> = [
    ["NaN", Number.NaN],
    ["positive infinity", Number.POSITIVE_INFINITY],
    ["negative infinity", Number.NEGATIVE_INFINITY],
    ["negative", -1],
    ["zero", 0],
    ["fraction", 1.5],
    ["unsafe integer", Number.MAX_SAFE_INTEGER + 1],
    ["missing", undefined],
    ["null", null],
    ["wrong type", "1048576"],
  ];

  it.each(invalidNumericValues)("rejects invalid resource dimensions across both platforms: %s", (_label, value) => {
    const setters: Array<[string, (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => void]> = [
      ["host before free", (plan) => { plan.before!.availableBytes = value as number; }],
      ["host before cap", (plan) => { plan.before!.independentAvailableCapBytes = value as number; }],
      ["host after free", (plan) => { plan.after!.availableBytes = value as number; }],
      ["host after cap", (plan) => { plan.after!.independentAvailableCapBytes = value as number; }],
      ["nominal volume", (plan) => { plan.capacity.nominalVolumeBytes = value as number; }],
      ["maximum image", (plan) => { plan.capacity.maximumImageBytes = value as number; }],
      ["observed image", (plan) => { plan.capacity.observedImageBytes = value as number; }],
      ["maximum host delta", (plan) => { plan.capacity.maximumHostDeltaBytes = value as number; }],
      ["observed host delta", (plan) => { plan.capacity.observedHostDeltaBytes = value as number; }],
      ["Windows baseline disk size", (plan) => { if (plan.platform === "windows") plan.disksBefore[0].sizeBytes = value as number; }],
      ["Windows target disk size", (plan) => { if (plan.platform === "windows") plan.disksAfter[2].sizeBytes = value as number; }],
      ["macOS baseline device size", (plan) => { if (plan.platform === "macos") plan.devicesBefore[0].sizeBytes = value as number; }],
      ["macOS image root size", (plan) => { if (plan.platform === "macos") macImageRoot(plan).sizeBytes = value as number; }],
    ];

    for (const [label, set] of setters) {
      const factories = label.startsWith("Windows")
        ? [makeWindowsSafePlan]
        : label.startsWith("macOS")
          ? [makeMacSafePlan]
          : [makeWindowsSafePlan, makeMacSafePlan];
      for (const makePlan of factories) {
        const plan = makePlan();
        set(plan);
        const result = evaluateSafetyPlan(plan);
        expect(result.status, label).not.toBe("DRY_RUN_SAFE");
        expect(result.intents, label).toEqual([]);
      }
    }
  });

  it.each([
    ["nominal volume above 256 MiB", (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => { plan.capacity.nominalVolumeBytes = MAX_NOMINAL_VOLUME_BYTES + 1; }],
    ["image above 320 MiB", (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => { plan.capacity.maximumImageBytes = MAX_IMAGE_BYTES + 1; }],
    ["host delta cap above 320 MiB", (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => { plan.capacity.maximumHostDeltaBytes = MAX_HOST_DELTA_BYTES + 1; }],
    ["observed image above its bounded cap", (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => { plan.capacity.observedImageBytes = MAX_IMAGE_BYTES + 1; }],
    ["observed host delta above its bounded cap", (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => { plan.capacity.observedHostDeltaBytes = MAX_HOST_DELTA_BYTES + 1; }],
  ] as Array<[string, (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => void]>) (
    "enforces the fixed virtual-volume and host-increment ceilings on both platforms: %s", (_label, mutate) => {
      for (const makePlan of [makeWindowsSafePlan, makeMacSafePlan]) {
        const plan = makePlan();
        mutate(plan);
        const result = evaluateSafetyPlan(plan);
        expect(result.status).not.toBe("DRY_RUN_SAFE");
        expect(result.intents).toEqual([]);
      }
    },
  );

  it("requires both host snapshots and independent caps to prove at least 4 GiB remains", () => {
    for (const makePlan of [makeWindowsSafePlan, makeMacSafePlan]) {
      for (const mutate of [
        (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => { plan.before!.availableBytes = 4 * GIB - 1; },
        (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => { plan.after!.independentAvailableCapBytes = 4 * GIB - 1; },
      ]) {
        const plan = makePlan();
        mutate(plan);
        const result = evaluateSafetyPlan(plan);
        expect(result.status).not.toBe("DRY_RUN_SAFE");
        expect(result.intents).toEqual([]);
      }
    }
  });

  it.each([
    ["missing capacity proof", (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => { (plan as unknown as { capacity?: unknown }).capacity = undefined; }],
    ["missing backing knowledge", (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => { (plan.capacity as unknown as { backingKnown?: unknown }).backingKnown = undefined; }],
    ["missing allocation mode", (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => { (plan.capacity as unknown as { allocation?: unknown }).allocation = undefined; }],
    ["missing sparse flag", (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => { (plan.capacity as unknown as { sparse?: unknown }).sparse = undefined; }],
    ["missing bounded-growth flag", (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => { (plan.capacity as unknown as { unboundedGrowth?: unknown }).unboundedGrowth = undefined; }],
    ["missing import prohibition", (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => { (plan.capacity as unknown as { importsExistingStorage?: unknown }).importsExistingStorage = undefined; }],
    ["missing target kind", (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => { (plan.capacity as unknown as { targetKind?: unknown }).targetKind = undefined; }],
    ["missing before host inventory", (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => { plan.before = null; }],
    ["missing after host inventory", (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => { plan.after = null; }],
  ] as Array<[string, (plan: ReturnType<typeof makeWindowsSafePlan> | ReturnType<typeof makeMacSafePlan>) => void]>) (
    "fails closed when required capacity proof is absent: %s", (_label, mutate) => {
      for (const makePlan of [makeWindowsSafePlan, makeMacSafePlan]) {
        const plan = makePlan();
        mutate(plan);
        const result = evaluateSafetyPlan(plan);
        expect(result.status).not.toBe("DRY_RUN_SAFE");
        expect(result.intents).toEqual([]);
      }
    },
  );

  it.each([
    ["lost Windows baseline", (plan: ReturnType<typeof makeWindowsSafePlan>) => { plan.disksAfter = [plan.disksAfter[2]]; }],
    ["changed Windows fingerprint", (plan: ReturnType<typeof makeWindowsSafePlan>) => { plan.disksAfter[0].fingerprint = "replacement-fingerprint"; }],
    ["changed Windows identity", (plan: ReturnType<typeof makeWindowsSafePlan>) => { plan.disksAfter[0].identity = "replacement-identity"; }],
    ["Windows identity reused with case-only variation", (plan: ReturnType<typeof makeWindowsSafePlan>) => { plan.disksAfter[0].identity = plan.disksAfter[0].identity!.toUpperCase(); }],
    ["changed Windows critical attribute", (plan: ReturnType<typeof makeWindowsSafePlan>) => { plan.disksAfter[1].friendlyName = "renamed host disk"; }],
    ["changed Windows size", (plan: ReturnType<typeof makeWindowsSafePlan>) => { plan.disksAfter[1].sizeBytes = (plan.disksAfter[1].sizeBytes ?? 0) + 1; }],
    ["changed Windows disk number", (plan: ReturnType<typeof makeWindowsSafePlan>) => { plan.disksAfter[1].diskNumber = 8; }],
    ["changed Windows host role", (plan: ReturnType<typeof makeWindowsSafePlan>) => { plan.disksAfter[1].isOsDisk = true; }],
    ["duplicate Windows baseline record", (plan: ReturnType<typeof makeWindowsSafePlan>) => { plan.disksAfter.push({ ...plan.disksAfter[0] }); }],
    ["duplicate Windows fingerprint", (plan: ReturnType<typeof makeWindowsSafePlan>) => { plan.disksAfter[2].fingerprint = plan.disksBefore[0].fingerprint; }],
    ["reused Windows baseline identity", (plan: ReturnType<typeof makeWindowsSafePlan>) => { plan.disksAfter[2].identity = plan.disksBefore[0].identity; plan.vhd.attachedDiskIdentities = [plan.disksBefore[0].identity]; }],
  ] as Array<[string, (plan: ReturnType<typeof makeWindowsSafePlan>) => void]>) (
    "rejects Windows baseline continuity failure: %s with no intents", (_label, mutate) => {
      const plan = makeWindowsSafePlan();
      mutate(plan);
      const result = evaluateSafetyPlan(plan);
      expect(result.status).not.toBe("DRY_RUN_SAFE");
      expect(result.intents).toEqual([]);
    },
  );

  it.each([
    ["lost macOS baseline", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.devicesAfter = [plan.devicesAfter[2]]; }],
    ["changed macOS fingerprint", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.devicesAfter[0].fingerprint = "replacement-fingerprint"; }],
    ["changed macOS identity", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.devicesAfter[0].identity = "replacement-identity"; }],
    ["changed macOS host classification", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.devicesAfter[1].isInternalHostDevice = false; }],
    ["changed macOS host hierarchy", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.devicesAfter[2].parentIdentity = "/dev/disk0"; }],
    ["changed macOS baseline size", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.devicesAfter[2].sizeBytes = (plan.devicesAfter[2].sizeBytes ?? 0) + 1; }],
    ["duplicate macOS baseline record", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.devicesAfter.push({ ...plan.devicesAfter[0] }); }],
    ["duplicate macOS fingerprint", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.devicesAfter[2].fingerprint = plan.devicesBefore[0].fingerprint; }],
    ["reused macOS host identity", (plan: ReturnType<typeof makeMacSafePlan>) => { macImageVolume(plan).identity = "/dev/disk2s5"; plan.attachResults[0].deviceIdentity = "/dev/disk2s5"; plan.attachResults[0].mountedVolumeIdentity = "/dev/disk2s5"; }],
    ["host Data volume selected by identity", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.attachResults[0].deviceIdentity = "/dev/disk2s5"; plan.devicesAfter = [...plan.devicesBefore]; }],
  ] as Array<[string, (plan: ReturnType<typeof makeMacSafePlan>) => void]>) (
    "rejects macOS baseline continuity failure: %s with no intents", (_label, mutate) => {
      const plan = makeMacSafePlan();
      mutate(plan);
      const result = evaluateSafetyPlan(plan);
      expect(result.status).not.toBe("DRY_RUN_SAFE");
      expect(result.intents).toEqual([]);
    },
  );

  it("accepts a fully linked multi-record macOS image hierarchy as a static-only model", () => {
    const plan = makeMacSafePlan();
    const result = evaluateSafetyPlan(plan);
    expect(result.status).toBe("DRY_RUN_SAFE");
    expect(result.executionMode).toBe("STATIC_ONLY");
    expect(result.physicalPassClaimed).toBe(false);
    expect(result.s2Eligibility).toBe("NOT_QUALIFIED");
    expect(result.intents.every((intent) => intent.execution === "NEVER_EXECUTED")).toBe(true);
  });

  it("can represent additional same-image APFS volumes without confusing their mount binding", () => {
    const plan = makeMacSafePlan();
    const container = plan.devicesAfter.find((device) => device.deviceRole === "apfs-container")!;
    const secondVolume = {
      identity: "MODEL:IMAGE:APFS-VOLUME:002",
      fingerprint: "new-udrw-image-apfs-volume-fingerprint-002",
      isInternalHostDevice: false,
      deviceRole: "apfs-volume" as const,
      parentIdentity: container.identity,
      sourceImageIdentity: plan.image.returnedImageIdentity,
      sizeBytes: MAX_NOMINAL_VOLUME_BYTES,
    };
    plan.devicesAfter.push(secondVolume);
    plan.attachResults[0].deviceIdentities.push(secondVolume.identity);

    const result = evaluateSafetyPlan(plan);
    expect(result.status).toBe("DRY_RUN_SAFE");
    expect(result.intents.find((intent) => intent.operation === "VERIFY_EXACT_APFS_VOLUME")).toMatchObject({ deviceIdentity: plan.attachResults[0].mountedVolumeIdentity });
    expect(result.s2Eligibility).toBe("NOT_QUALIFIED");
  });

  it.each([
    ["missing root origin", (plan: ReturnType<typeof makeMacSafePlan>) => { macImageRoot(plan).sourceImageIdentity = null; }],
    ["stale image origin", (plan: ReturnType<typeof makeMacSafePlan>) => {
      macImageRoot(plan).sourceImageIdentity = "stale-image";
    }],
    ["mismatched creation record", (plan: ReturnType<typeof makeMacSafePlan>) => {
      plan.attachResults[0].creationRecordIdentity = "stale-create-record";
    }],
    ["orphan image child", (plan: ReturnType<typeof makeMacSafePlan>) => {
      const child = { identity: "orphan-volume", fingerprint: "orphan-volume-fp", isInternalHostDevice: false, deviceRole: "apfs-volume" as const, parentIdentity: "unknown-root", sourceImageIdentity: plan.image.returnedImageIdentity, sizeBytes: MAX_NOMINAL_VOLUME_BYTES };
      plan.devicesAfter.push(child);
      plan.attachResults[0].deviceIdentities.push(child.identity);
    }],
    ["mismatched returned image identity", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.attachResults[0].imageIdentity = "another-image"; }],
    ["child from another image", (plan: ReturnType<typeof makeMacSafePlan>) => { macImageVolume(plan).sourceImageIdentity = "foreign-image"; }],
    ["new device parented into host hierarchy", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.devicesAfter.find((device) => device.deviceRole === "partition")!.parentIdentity = "/dev/disk2"; }],
    ["root reuses existing host hierarchy identity", (plan: ReturnType<typeof makeMacSafePlan>) => { macImageRoot(plan).identity = "/dev/disk2"; }],
    ["attach omits one discovered image device", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.attachResults[0].deviceIdentities.pop(); }],
    ["attach repeats an image device identity", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.attachResults[0].deviceIdentities.push(plan.attachResults[0].deviceIdentities[0]); }],
    ["second ambiguous image root", (plan: ReturnType<typeof makeMacSafePlan>) => {
      const root = { ...macImageRoot(plan), identity: "MODEL:IMAGE:ROOT:002", fingerprint: "new-udrw-image-root-fingerprint-002" };
      plan.devicesAfter.push(root);
      plan.attachResults[0].deviceIdentities.push(root.identity!);
    }],
    ["mount points to a root instead of an APFS volume", (plan: ReturnType<typeof makeMacSafePlan>) => { plan.attachResults[0].mountedVolumeIdentity = macImageRoot(plan).identity; plan.attachResults[0].deviceIdentity = macImageRoot(plan).identity; }],
    ["APFS volume above 256 MiB", (plan: ReturnType<typeof makeMacSafePlan>) => { macImageVolume(plan).sizeBytes = MAX_NOMINAL_VOLUME_BYTES + 1; }],
    ["image root above 320 MiB", (plan: ReturnType<typeof makeMacSafePlan>) => { macImageRoot(plan).sizeBytes = MAX_IMAGE_BYTES + 1; }],
  ] as Array<[string, (plan: ReturnType<typeof makeMacSafePlan>) => void]>) (
    "rejects macOS image hierarchy without complete provenance: %s", (_label, mutate) => {
      const plan = makeMacSafePlan();
      mutate(plan);
      const result = evaluateSafetyPlan(plan);
      expect(result.status).not.toBe("DRY_RUN_SAFE");
      expect(result.intents).toEqual([]);
    },
  );

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
    macImageRoot(mac).fingerprint = null;

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
    macImageVolume(plan).fingerprint = "g1:/dev/disk2s5:Data:APFS:/System/Volumes/Data";
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
    expect(hasIssue(result, "MAC_IMAGE_HIERARCHY_ROOT_INVALID") || hasIssue(result, "MAC_MOUNTED_VOLUME_BINDING_INVALID")).toBe(true);
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

function cleanupEvent(
  session: CleanupSession,
  intent: CleanupIntent,
  overrides: Record<string, unknown> = {},
): CleanupEvent {
  const state = inspectCleanup(session)!;
  const common = {
    targetIdentity: intent.targetIdentity,
    observedOwnerToken: state.ownerToken,
    observedDeviceIdentity: state.deviceIdentity,
    observedImageIdentity: state.imageIdentity,
  };
  const body = intent.operation === "UNMOUNT_EXACT_MOUNTPOINT"
    ? { kind: "unmount", succeeded: true, confirmedUnmounted: true }
    : intent.operation === "DETACH_KNOWN_DEVICE"
      ? { kind: "detach", succeeded: true, forced: false }
      : intent.operation === "VERIFY_DEVICE_RELEASE"
        ? { kind: "release-check", identityAbsent: true }
        : { kind: "remove-image", succeeded: true };
  return { ...common, ...body, ...overrides } as CleanupEvent;
}

function advanceWith(
  session: CleanupSession,
  intent: CleanupIntent,
  overrides: Record<string, unknown> = {},
) {
  return advanceCleanup(session, intent, cleanupEvent(session, intent, overrides));
}

describe("G2-S1 exact-resource cleanup state machine", () => {
  it("requires ordered opaque intents and matching observations before exact image removal", () => {
    const session = makeCleanupState();
    const unmount = nextCleanupIntent(session)!;
    expect(unmount).toMatchObject({ operation: "UNMOUNT_EXACT_MOUNTPOINT", targetIdentity: "/tmp/zen-g2-s1/case-001/mount", force: false, execution: "NEVER_EXECUTED" });
    expect(nextCleanupIntent(session)).toBe(unmount);
    expect(advanceWith(session, unmount)?.phase).toBe("UNMOUNTED");

    const detach = nextCleanupIntent(session)!;
    expect(detach).toMatchObject({ operation: "DETACH_KNOWN_DEVICE", targetIdentity: "/dev/disk9s1", force: false, execution: "NEVER_EXECUTED" });
    expect(advanceWith(session, detach)?.phase).toBe("DETACHED");

    const verifyRelease = nextCleanupIntent(session)!;
    expect(verifyRelease).toMatchObject({ operation: "VERIFY_DEVICE_RELEASE", targetIdentity: "/dev/disk9s1", execution: "NEVER_EXECUTED" });
    expect(advanceWith(session, verifyRelease)?.phase).toBe("RELEASE_VERIFIED");

    const remove = nextCleanupIntent(session)!;
    expect(remove).toMatchObject({ operation: "REMOVE_EXACT_OWNED_IMAGE", targetIdentity: "/tmp/zen-g2-s1/case-001/bounded.dmg", force: false, execution: "NEVER_EXECUTED" });
    expect(advanceWith(session, remove)?.phase).toBe("REMOVED");
    expect(inspectCleanup(session)).toMatchObject({ originalFailure: "simulated verification failure", phase: "REMOVED" });
    expect(assessCleanup(session)).toMatchObject({
      status: "FAIL",
      cleanupStatus: "DRY_RUN_SAFE",
      originalFailure: "simulated verification failure",
      physicalPassClaimed: false,
      s2Eligibility: "NOT_QUALIFIED",
    });
    expect(nextCleanupIntent(session)).toBeNull();
  });

  it("keeps the original failure and blocks removal when release cannot be proven", () => {
    const session = makeCleanupState();
    const unmount = nextCleanupIntent(session)!;
    advanceWith(session, unmount);
    const detach = nextCleanupIntent(session)!;
    advanceWith(session, detach);
    const release = nextCleanupIntent(session)!;
    const state = advanceWith(session, release, { identityAbsent: false });

    expect(state).toMatchObject({ phase: "CLEANUP_INCOMPLETE", cleanupFailure: "device release could not be proven", originalFailure: "simulated verification failure" });
    expect(assessCleanup(session)).toMatchObject({ status: "FAIL", cleanupStatus: "CLEANUP_INCOMPLETE" });
    expect(nextCleanupIntent(session)).toBeNull();
  });

  it("cleans up Windows by returned VHD object identity without requiring a drive-letter selector", () => {
    const session = makeWindowsCleanupState();
    const detach = nextCleanupIntent(session)!;
    expect(detach).toMatchObject({ operation: "DETACH_KNOWN_DEVICE", targetIdentity: "returned-vhd-object-001", force: false, execution: "NEVER_EXECUTED" });
    expect(advanceWith(session, detach)?.phase).toBe("DETACHED");
    const release = nextCleanupIntent(session)!;
    expect(advanceWith(session, release)?.phase).toBe("RELEASE_VERIFIED");
    expect(nextCleanupIntent(session)).toMatchObject({ operation: "REMOVE_EXACT_OWNED_IMAGE", targetIdentity: "C:\\Temp\\zen-g2-s1\\case-001\\bounded.vhdx", force: false, execution: "NEVER_EXECUTED" });
  });

  it("reports removal failure while retaining the original modeled failure", () => {
    const session = makeCleanupState();
    for (const operation of ["UNMOUNT_EXACT_MOUNTPOINT", "DETACH_KNOWN_DEVICE", "VERIFY_DEVICE_RELEASE"] as const) {
      const intent = nextCleanupIntent(session)!;
      expect(intent.operation).toBe(operation);
      advanceWith(session, intent);
    }
    const remove = nextCleanupIntent(session)!;
    const state = advanceWith(session, remove, { succeeded: false });
    expect(state).toMatchObject({ phase: "CLEANUP_INCOMPLETE", originalFailure: "simulated verification failure", cleanupFailure: "exact owned image removal failed" });
    expect(assessCleanup(session)).toMatchObject({ status: "FAIL", cleanupStatus: "CLEANUP_INCOMPLETE" });
    expect(nextCleanupIntent(session)).toBeNull();
  });

  it.each([
    ["failed unmount", { succeeded: false, confirmedUnmounted: false }],
    ["unmount reported without confirmation", { succeeded: true, confirmedUnmounted: false }],
    ["skipped unmount", { kind: "detach", succeeded: true, forced: false }],
    ["reverse release event", { kind: "release-check", identityAbsent: true }],
    ["wrong mount identity", { targetIdentity: "/Volumes/runner" }],
    ["changed device identity", { observedDeviceIdentity: "/dev/disk99" }],
    ["changed image identity", { observedImageIdentity: "foreign-image" }],
    ["changed ownership", { observedOwnerToken: "other-owner" }],
    ["missing ownership observation", { observedOwnerToken: null }],
  ] as Array<[string, Record<string, unknown>]>) ("fails closed on unverified or out-of-order cleanup event: %s", (_label, overrides) => {
    const session = makeCleanupState();
    const intent = nextCleanupIntent(session)!;
    const state = advanceWith(session, intent, overrides);
    expect(state).toMatchObject({ phase: "CLEANUP_INCOMPLETE", originalFailure: "simulated verification failure" });
    expect(nextCleanupIntent(session)).toBeNull();
  });

  it.each([
    ["detach failure", { succeeded: false, forced: false }],
    ["forced detach", { succeeded: true, forced: true }],
    ["detach target mismatch", { targetIdentity: "/dev/disk77" }],
  ] as Array<[string, Record<string, unknown>]>) (
    "does not permit image removal after unsafe detach: %s", (_label, overrides) => {
      const session = makeCleanupState();
      const unmount = nextCleanupIntent(session)!;
      advanceWith(session, unmount);
      const detach = nextCleanupIntent(session)!;
      const state = advanceWith(session, detach, overrides);
      expect(state).toMatchObject({ phase: "CLEANUP_INCOMPLETE", originalFailure: "simulated verification failure" });
      expect(nextCleanupIntent(session)).toBeNull();
    },
  );

  it("detects device, image, and owner identity drift after an earlier cleanup transition", () => {
    for (const key of ["observedDeviceIdentity", "observedImageIdentity", "observedOwnerToken"]) {
      const session = makeCleanupState();
      const unmount = nextCleanupIntent(session)!;
      advanceWith(session, unmount);
      const detach = nextCleanupIntent(session)!;
      const state = advanceWith(session, detach, { [key]: "changed-during-cleanup" });
      expect(state?.phase).toBe("CLEANUP_INCOMPLETE");
      expect(nextCleanupIntent(session)).toBeNull();
    }
  });

  it("rejects a repeated stale transition rather than advancing the new phase", () => {
    const session = makeCleanupState();
    const unmount = nextCleanupIntent(session)!;
    advanceWith(session, unmount);
    const repeated = advanceWith(session, unmount);
    expect(repeated).toMatchObject({ phase: "CLEANUP_INCOMPLETE", originalFailure: "simulated verification failure" });
    expect(nextCleanupIntent(session)).toBeNull();
  });

  it("does not accept transition evidence or an intent issued for another cleanup session", () => {
    const first = makeCleanupState();
    const second = makeCleanupState();
    const firstIntent = nextCleanupIntent(first)!;
    const secondState = advanceWith(second, firstIntent);
    expect(secondState).toMatchObject({ phase: "CLEANUP_INCOMPLETE" });
    expect(nextCleanupIntent(second)).toBeNull();
  });

  it("does not trust a directly forged release phase and all-true proof flags", () => {
    const genuine = makeCleanupState();
    const forged = Object.assign({}, genuine, {
      phase: "RELEASE_VERIFIED",
      unmountedProven: true,
      detachedProven: true,
      releaseProven: true,
    }) as unknown as CleanupSession;
    expect(nextCleanupIntent(forged)).toBeNull();
    const forgedRemove = {
      operation: "REMOVE_EXACT_OWNED_IMAGE",
      targetIdentity: "/tmp/zen-g2-s1/case-001/bounded.dmg",
      force: false,
      execution: "NEVER_EXECUTED",
    } as CleanupIntent;
    expect(advanceCleanup(forged, forgedRemove, {
      kind: "remove-image",
      targetIdentity: forgedRemove.targetIdentity,
      observedOwnerToken: "owned-case-001",
      observedDeviceIdentity: "/dev/disk9s1",
      observedImageIdentity: "created-udrw-image-001",
      succeeded: true,
    })).toBeNull();
    expect(nextCleanupIntent(genuine)?.operation).toBe("UNMOUNT_EXACT_MOUNTPOINT");
  });

  it("blocks invalid initial ownership, escaped paths, and linked mount points", () => {
    const base = {
      platform: "macos" as const,
      sandboxPath: "/tmp/zen-g2-s1/case-001",
      imagePath: "/tmp/zen-g2-s1/case-001/bounded.dmg",
      mountPoint: "/tmp/zen-g2-s1/case-001/mount",
      deviceIdentity: "/dev/disk9s1",
      imageIdentity: "created-udrw-image-001",
      ownerToken: "owned-case-001",
      observedOwnerToken: "owned-case-001",
      imageIsOwned: true,
      imageIsSymlinkOrReparsePoint: false,
      mountPointIsSymlinkOrReparsePoint: false,
      originalFailure: "simulated verification failure",
    };
    const invalidStarts = [
      { ...base, observedOwnerToken: "different-owner" },
      { ...base, imagePath: "/tmp/foreign.dmg" },
      { ...base, mountPointIsSymlinkOrReparsePoint: true },
      { ...base, imageIsOwned: false },
    ];
    for (const start of invalidStarts) {
      const session = createCleanupSession(start);
      expect(nextCleanupIntent(session)).toBeNull();
      expect(inspectCleanup(session)?.phase).toBe("CLEANUP_INCOMPLETE");
    }
  });
});
