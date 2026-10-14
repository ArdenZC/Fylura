/**
 * G2-S1 static safety contract model.
 *
 * This module is deliberately pure: it has no filesystem, process, platform
 * disk, or command-execution imports. Its outputs are typed descriptions for
 * review and are never dispatched to an operating system.
 */

export const GIB = 1_073_741_824;
export const MIB = 1_048_576;
export const MIN_HOST_FREE_BYTES = 4 * GIB;
export const MAX_NOMINAL_VOLUME_BYTES = 256 * MIB;
export const MAX_IMAGE_BYTES = 320 * MIB;
export const MAX_HOST_DELTA_BYTES = 320 * MIB;

export type ModelStatus =
  | "DRY_RUN_SAFE"
  | "SAFETY_REJECTION"
  | "ENVIRONMENT_UNAVAILABLE"
  | "FAIL"
  | "CLEANUP_INCOMPLETE";

export type SafetyIssue = {
  code: string;
  severity: Exclude<ModelStatus, "DRY_RUN_SAFE" | "CLEANUP_INCOMPLETE">;
  detail: string;
};

type SourceProof = {
  expectedSha256: string;
  observedSha256: string;
};

type PathIdentity = {
  lexical: string;
  canonical: string;
  symlinkOrReparsePoint: boolean;
};

type PathProof = {
  root: PathIdentity;
  sandbox: PathIdentity;
  target: PathIdentity;
  mountPoint: PathIdentity;
  targetExistsBefore: boolean;
  mountPointExists: boolean;
  mountPointEmpty: boolean;
  sandboxUnique: boolean;
  ownerToken: string;
  observedOwnerToken: string | null;
};

type HostSnapshot = {
  hostFingerprint: string;
  availableBytes: number | null;
  independentAvailableCapBytes: number | null;
};

type CapacityProof = {
  nominalVolumeBytes: number;
  maximumImageBytes: number;
  observedImageBytes: number;
  maximumHostDeltaBytes: number;
  observedHostDeltaBytes: number;
  allocation: "fixed" | "dynamic";
  sparse: boolean;
  unboundedGrowth: boolean;
  importsExistingStorage: boolean;
  targetKind: "sandbox-file" | "physical-device";
  backingKnown: boolean;
};

type CommonSafetyPlan = {
  platform: "windows" | "macos";
  source: SourceProof;
  paths: PathProof;
  before: HostSnapshot | null;
  after: HostSnapshot | null;
  capacity: CapacityProof;
  inventoryEvidence: {
    beforeMutationCaptured: boolean;
    afterPlanCaptured: boolean;
    crosscheck: "PASS" | "FAIL" | "MISSING";
  };
  positiveControl: "PASS" | "FAIL" | "MISSING";
};

export type SimulatedDisk = {
  identity: string | null;
  fingerprint: string | null;
  diskNumber: number | null;
  friendlyName: string | null;
  sizeBytes: number | null;
  partitionStyle: "RAW" | "GPT" | "MBR" | "UNKNOWN";
  isSystem: boolean | null;
  isBoot: boolean | null;
  isOsDisk: boolean | null;
};

export type WindowsSafetyPlan = CommonSafetyPlan & {
  platform: "windows";
  selection: "returned-cim-object" | "disk-number" | "friendly-name" | "drive-letter" | "broad-selector" | "implicit-default";
  vhd: {
    exactPath: string;
    returnedObjectIdentity: string | null;
    attachedDiskIdentities: Array<string | null>;
    driveLetterAssigned: boolean;
  };
  disksBefore: SimulatedDisk[];
  disksAfter: SimulatedDisk[];
  initializeDiskIdentity: string | null;
  partition: { identity: string | null; diskIdentity: string | null } | null;
  format: { partitionIdentity: string | null; fileSystem: string | null } | null;
};

export type SimulatedDevice = {
  identity: string | null;
  fingerprint: string | null;
  isInternalHostDevice: boolean | null;
};

export type MacSafetyPlan = CommonSafetyPlan & {
  platform: "macos";
  selection: "structured-attach-result" | "guessed-device-path" | "broad-selector" | "implicit-default";
  image: {
    exactPath: string;
    format: "UDRW" | "UDIF" | "SPARSE" | "SPARSEBUNDLE" | "UNKNOWN";
    sparse: boolean;
    returnedImageIdentity: string | null;
  };
  devicesBefore: SimulatedDevice[];
  devicesAfter: SimulatedDevice[];
  attachResults: Array<{
    recordIdentity: string | null;
    deviceIdentity: string | null;
    sourceImagePath: string | null;
    mountPoints: string[];
    fileSystem: string | null;
    mountPointEmpty: boolean | null;
    mounted: boolean;
  }>;
  globalEraseOrPartitionIntent: boolean;
};

export type SafetyPlan = WindowsSafetyPlan | MacSafetyPlan;

type NeverExecuted = { execution: "NEVER_EXECUTED" };

export type DryRunIntent = NeverExecuted & (
  | { operation: "CREATE_FIXED_VHDX_FILE"; path: string; maximumBytes: number }
  | { operation: "ATTACH_RETURNED_VHD_OBJECT"; imagePath: string; returnedObjectIdentity: string; noDriveLetter: true }
  | { operation: "INITIALIZE_EXACT_DISK_OBJECT"; diskIdentity: string; partitionStyle: "GPT" }
  | { operation: "CREATE_PARTITION_ON_EXACT_DISK"; diskIdentity: string; partitionIdentity: string }
  | { operation: "FORMAT_EXACT_PARTITION_NTFS"; partitionIdentity: string; fileSystem: "NTFS" }
  | { operation: "CREATE_UDRW_APFS_IMAGE"; path: string; maximumBytes: number; sparse: false }
  | { operation: "ATTACH_EXACT_IMAGE_STRUCTURED"; imagePath: string; imageIdentity: string; mountPoint: string }
  | { operation: "VERIFY_EXACT_APFS_VOLUME"; deviceIdentity: string; mountPoint: string }
  | { operation: "REGISTER_CLEANUP_OBLIGATION"; resourceIdentity: string; force: false }
);

export type SafetyAssessment = {
  status: ModelStatus;
  issues: SafetyIssue[];
  intents: DryRunIntent[];
  executionMode: "STATIC_ONLY";
  physicalPassClaimed: false;
};

function issue(
  issues: SafetyIssue[],
  code: string,
  detail: string,
  severity: SafetyIssue["severity"] = "SAFETY_REJECTION",
): void {
  issues.push({ code, detail, severity });
}

function canonicalPath(value: string, platform: CommonSafetyPlan["platform"]): string | null {
  const path = value.replaceAll("\\", "/");
  const windows = platform === "windows";
  if (windows ? !/^[a-zA-Z]:\//.test(path) : !path.startsWith("/")) return null;
  const prefix = windows ? `${path.slice(0, 2).toLowerCase()}/` : "/";
  const remainder = windows ? path.slice(3) : path.slice(1);
  const segments = remainder.split("/").filter(Boolean);
  if (segments.some((segment) => segment === "." || segment === "..")) return null;
  const normalizedSegments = windows ? segments.map((segment) => segment.toLowerCase()) : segments;
  return `${prefix}${normalizedSegments.join("/")}`.replace(/\/$/, "") || prefix;
}

function dirname(value: string): string {
  const index = value.lastIndexOf("/");
  return index <= 0 ? value.slice(0, 1) : value.slice(0, index);
}

function validatePaths(plan: CommonSafetyPlan, issues: SafetyIssue[]): void {
  const entries = [
    ["root", plan.paths.root],
    ["sandbox", plan.paths.sandbox],
    ["target", plan.paths.target],
    ["mountPoint", plan.paths.mountPoint],
  ] as const;
  const normalized = new Map<string, string>();
  for (const [name, path] of entries) {
    const lexical = canonicalPath(path.lexical, plan.platform);
    const canonical = canonicalPath(path.canonical, plan.platform);
    if (!lexical || !canonical || lexical !== canonical) {
      issue(issues, `PATH_${name.toUpperCase()}_IDENTITY_MISMATCH`, `${name} is not an exact absolute canonical path`);
    } else {
      normalized.set(name, canonical);
    }
    if (path.symlinkOrReparsePoint) {
      issue(issues, `PATH_${name.toUpperCase()}_LINK`, `${name} is a symlink or reparse point`);
    }
  }

  const root = normalized.get("root");
  const sandbox = normalized.get("sandbox");
  const target = normalized.get("target");
  const mountPoint = normalized.get("mountPoint");
  if (root && sandbox && (dirname(sandbox) !== root || sandbox === root)) {
    issue(issues, "SANDBOX_NOT_UNIQUE_CHILD", "sandbox must be one dedicated child of the exact approved root");
  }
  if (sandbox && target && dirname(target) !== sandbox) {
    issue(issues, "TARGET_ESCAPES_SANDBOX", "backing image must be a direct child of the owned sandbox");
  }
  if (sandbox && mountPoint && dirname(mountPoint) !== sandbox) {
    issue(issues, "MOUNTPOINT_ESCAPES_SANDBOX", "mount point must be a direct child of the owned sandbox");
  }
  if (plan.paths.targetExistsBefore) {
    issue(issues, "TARGET_PREEXISTS", "target image path must be absent before the plan");
  }
  if (!plan.paths.sandboxUnique || !plan.paths.ownerToken || plan.paths.ownerToken !== plan.paths.observedOwnerToken) {
    issue(issues, "SANDBOX_OWNERSHIP_UNPROVEN", "sandbox uniqueness and exact ownership token must match");
  }
}

function validateCommon(plan: CommonSafetyPlan, issues: SafetyIssue[]): void {
  const digestPattern = /^[0-9a-f]{64}$/;
  if (!digestPattern.test(plan.source.expectedSha256) || plan.source.expectedSha256 !== plan.source.observedSha256) {
    issue(issues, "SOURCE_IDENTITY_UNVERIFIED", "verified source identity is absent or does not match");
  }

  validatePaths(plan, issues);
  if (!plan.inventoryEvidence.beforeMutationCaptured || !plan.inventoryEvidence.afterPlanCaptured) {
    issue(issues, "INVENTORY_EVIDENCE_MISSING", "both pre-mutation and post-plan inventory snapshots are required");
  }
  if (plan.inventoryEvidence.crosscheck !== "PASS") {
    issue(
      issues,
      "SOURCE_CROSSCHECK_NOT_PASSED",
      "official-document and inventory crosscheck must explicitly pass",
      plan.inventoryEvidence.crosscheck === "MISSING" ? "ENVIRONMENT_UNAVAILABLE" : "SAFETY_REJECTION",
    );
  }
  if (plan.positiveControl === "FAIL") {
    issue(issues, "POSITIVE_CONTROL_FAILED", "static positive control failed", "FAIL");
  } else if (plan.positiveControl === "MISSING") {
    issue(issues, "POSITIVE_CONTROL_MISSING", "static positive control evidence is required");
  }

  if (!plan.before || !plan.after) {
    issue(issues, "HOST_INVENTORY_UNAVAILABLE", "both host inventory snapshots are required", "ENVIRONMENT_UNAVAILABLE");
  } else {
    if (!plan.before.hostFingerprint || !plan.after.hostFingerprint || plan.before.hostFingerprint !== plan.after.hostFingerprint) {
      issue(issues, "HOST_IDENTITY_CHANGED", "host fingerprint changed between inventory checks (TOCTOU)");
    }
    for (const [label, snapshot] of [["before", plan.before], ["after", plan.after]] as const) {
      if (snapshot.availableBytes === null || snapshot.independentAvailableCapBytes === null) {
        issue(issues, `HOST_CAPACITY_${label.toUpperCase()}_UNKNOWN`, `${label} free-space evidence is unavailable`, "ENVIRONMENT_UNAVAILABLE");
        continue;
      }
      if (snapshot.availableBytes > snapshot.independentAvailableCapBytes) {
        issue(issues, `HOST_CAPACITY_${label.toUpperCase()}_OVERSTATED`, `${label} free-space report exceeds the independent cap`);
      }
      if (snapshot.availableBytes < MIN_HOST_FREE_BYTES || snapshot.independentAvailableCapBytes < MIN_HOST_FREE_BYTES) {
        issue(issues, `HOST_CAPACITY_${label.toUpperCase()}_BELOW_FLOOR`, `${label} host free space must be at least 4 GiB`, "ENVIRONMENT_UNAVAILABLE");
      }
    }
  }

  const capacity = plan.capacity;
  if (!capacity.backingKnown) {
    issue(issues, "BACKING_STORAGE_UNKNOWN", "unknown backing storage is unavailable for a bounded plan", "ENVIRONMENT_UNAVAILABLE");
  }
  if (capacity.nominalVolumeBytes <= 0 || capacity.nominalVolumeBytes > MAX_NOMINAL_VOLUME_BYTES) {
    issue(issues, "NOMINAL_VOLUME_OUT_OF_BOUNDS", "nominal volume must be positive and no larger than 256 MiB");
  }
  if (capacity.maximumImageBytes < capacity.nominalVolumeBytes || capacity.maximumImageBytes > MAX_IMAGE_BYTES) {
    issue(issues, "IMAGE_CAP_OUT_OF_BOUNDS", "image cap must cover the nominal volume and remain at or below 320 MiB");
  }
  if (capacity.observedImageBytes < 0 || capacity.observedImageBytes > capacity.maximumImageBytes) {
    issue(issues, "OBSERVED_IMAGE_OUT_OF_BOUNDS", "observed image size exceeds its independent cap");
  }
  if (capacity.maximumHostDeltaBytes < 0 || capacity.maximumHostDeltaBytes > MAX_HOST_DELTA_BYTES) {
    issue(issues, "HOST_DELTA_CAP_OUT_OF_BOUNDS", "host delta cap must be at or below 320 MiB");
  }
  if (capacity.observedHostDeltaBytes < 0 || capacity.observedHostDeltaBytes > capacity.maximumHostDeltaBytes) {
    issue(issues, "OBSERVED_HOST_DELTA_OUT_OF_BOUNDS", "observed host delta exceeds its independent cap");
  }
  if (capacity.allocation !== "fixed" || capacity.sparse || capacity.unboundedGrowth) {
    issue(issues, "ALLOCATION_NOT_BOUNDED", "only fixed, non-sparse, bounded allocation is admissible");
  }
  if (capacity.importsExistingStorage) {
    issue(issues, "EXISTING_STORAGE_IMPORT_FORBIDDEN", "source disk or partition import is forbidden");
  }
  if (capacity.targetKind !== "sandbox-file") {
    issue(issues, "NON_FILE_TARGET_FORBIDDEN", "a physical-device target is never admissible");
  }
}

function validateWindows(plan: WindowsSafetyPlan, issues: SafetyIssue[]): void {
  const targetPath = canonicalPath(plan.paths.target.lexical, "windows");
  if (plan.selection !== "returned-cim-object") {
    issue(issues, "WINDOWS_SELECTOR_UNSAFE", "disk number, friendly name, drive letter, broad selector, and default selection are forbidden");
  }
  if (!targetPath || canonicalPath(plan.vhd.exactPath, "windows") !== targetPath || !targetPath.endsWith(".vhdx")) {
    issue(issues, "WINDOWS_VHD_PATH_MISMATCH", "VHDX creation must bind the exact new sandbox path");
  }
  if (!plan.vhd.returnedObjectIdentity) {
    issue(issues, "WINDOWS_VHD_RETURN_IDENTITY_MISSING", "attach must return a validated VHD object identity");
  }
  if (plan.vhd.driveLetterAssigned) {
    issue(issues, "WINDOWS_DRIVE_LETTER_FORBIDDEN", "modeled attach must suppress drive-letter assignment");
  }

  if (plan.disksBefore.length === 0 || plan.disksBefore.some((disk) =>
    !disk.identity || !disk.fingerprint || disk.isSystem === null || disk.isBoot === null || disk.isOsDisk === null
  )) {
    issue(issues, "WINDOWS_BASELINE_IDENTITY_UNPROVEN", "every pre-mutation disk must have a stable identity, fingerprint, and host-role flags");
  }
  if (plan.disksAfter.some((disk) => !disk.identity || !disk.fingerprint)) {
    issue(issues, "WINDOWS_POST_INVENTORY_IDENTITY_UNPROVEN", "every post-plan disk must have a stable identity and fingerprint");
  }
  const postIds = plan.disksAfter.map((disk) => disk.identity).filter((value): value is string => Boolean(value));
  if (new Set(postIds).size !== postIds.length) {
    issue(issues, "WINDOWS_POST_INVENTORY_IDENTITY_AMBIGUOUS", "post-plan disk identities must be unique");
  }

  const beforeIds = new Set(plan.disksBefore.map((disk) => disk.identity).filter((value): value is string => Boolean(value)));
  const beforeFingerprints = new Set(plan.disksBefore.map((disk) => disk.fingerprint).filter((value): value is string => Boolean(value)));
  const newDisks = plan.disksAfter.filter((disk) => disk.identity && !beforeIds.has(disk.identity));
  const attachedIds = plan.vhd.attachedDiskIdentities.filter((identity): identity is string => Boolean(identity));
  if (plan.vhd.attachedDiskIdentities.some((identity) => identity === null) || attachedIds.length !== 1) {
    issue(issues, "WINDOWS_RETURNED_DISK_IDENTITY_AMBIGUOUS", "exactly one non-empty disk identity must be bound to the returned VHD object");
  }
  if (newDisks.length !== 1 || attachedIds.length !== 1 || newDisks[0]?.identity !== attachedIds[0]) {
    issue(issues, "WINDOWS_NEW_DISK_CARDINALITY_OR_BINDING", "exactly one previously unseen disk must match the returned VHD object");
  }
  const targetDisk = newDisks[0];
  if (targetDisk && (!targetDisk.fingerprint || targetDisk.isSystem === null || targetDisk.isBoot === null || targetDisk.isOsDisk === null)) {
    issue(issues, "WINDOWS_DISK_IDENTITY_UNPROVEN", "new disk identity and system/boot/OS flags must be known");
  }
  if (targetDisk && (targetDisk.isSystem || targetDisk.isBoot || targetDisk.isOsDisk)) {
    issue(issues, "WINDOWS_HOST_DISK_TARGET", "system, boot, or OS disks are prohibited targets");
  }
  if (targetDisk && beforeFingerprints.has(targetDisk.fingerprint ?? "")) {
    issue(issues, "WINDOWS_BASELINE_HOST_DISK_TARGET", "a device matching any pre-mutation host disk fingerprint is prohibited");
  }
  if (targetDisk && targetDisk.partitionStyle !== "RAW") {
    issue(issues, "WINDOWS_DISK_NOT_RAW", "new target disk must be RAW before planned initialization");
  }
  if (targetDisk && targetDisk.sizeBytes !== plan.capacity.nominalVolumeBytes) {
    issue(issues, "WINDOWS_DISK_SIZE_MISMATCH", "returned disk size must equal the bounded nominal volume size");
  }
  if (!targetDisk?.identity || plan.initializeDiskIdentity !== targetDisk.identity) {
    issue(issues, "WINDOWS_INITIALIZE_TARGET_MISMATCH", "initialization must bind the exact newly returned disk object");
  }
  if (!plan.partition?.identity || plan.partition.diskIdentity !== targetDisk?.identity) {
    issue(issues, "WINDOWS_PARTITION_BINDING_MISMATCH", "partition must be created from the exact returned disk identity");
  }
  if (!plan.format?.partitionIdentity || plan.format.partitionIdentity !== plan.partition?.identity || plan.format.fileSystem !== "NTFS") {
    issue(issues, "WINDOWS_FORMAT_BINDING_OR_FILESYSTEM_MISMATCH", "NTFS formatting must bind the exact validated partition object");
  }
}

function validateMac(plan: MacSafetyPlan, issues: SafetyIssue[]): void {
  const targetPath = canonicalPath(plan.paths.target.lexical, "macos");
  const mountPath = canonicalPath(plan.paths.mountPoint.lexical, "macos");
  if (plan.selection !== "structured-attach-result") {
    issue(issues, "MAC_SELECTOR_UNSAFE", "only a parsed structured attach result may bind the target device");
  }
  if (!targetPath || canonicalPath(plan.image.exactPath, "macos") !== targetPath || !targetPath.endsWith(".dmg")) {
    issue(issues, "MAC_IMAGE_PATH_MISMATCH", "image creation must bind the exact new sandbox image path");
  }
  if (!plan.image.returnedImageIdentity) {
    issue(issues, "MAC_IMAGE_RETURN_IDENTITY_MISSING", "image creation must produce a verifiable image identity");
  }
  if (plan.image.format !== "UDRW" || plan.image.sparse) {
    issue(issues, "MAC_IMAGE_FORMAT_UNSAFE", "only non-sparse writable UDRW image format is modeled as safe");
  }
  if (plan.globalEraseOrPartitionIntent) {
    issue(issues, "MAC_GLOBAL_DISK_OPERATION_FORBIDDEN", "global disk erase or partition intent is forbidden");
  }
  if (!plan.paths.mountPointExists || !plan.paths.mountPointEmpty) {
    issue(issues, "MAC_MOUNTPOINT_NOT_EMPTY_OR_MISSING", "the exact contained mount point must exist and be empty");
  }

  if (plan.devicesBefore.length === 0 || plan.devicesBefore.some((device) =>
    !device.identity || !device.fingerprint || device.isInternalHostDevice === null
  )) {
    issue(issues, "MAC_BASELINE_IDENTITY_UNPROVEN", "every baseline device must have a stable identity, fingerprint, and host classification");
  }
  if (plan.devicesAfter.some((device) =>
    !device.identity || !device.fingerprint || device.isInternalHostDevice === null
  )) {
    issue(issues, "MAC_POST_INVENTORY_IDENTITY_UNPROVEN", "every post-plan device must have a stable identity, fingerprint, and host classification");
  }
  const postIds = plan.devicesAfter.map((device) => device.identity).filter((value): value is string => Boolean(value));
  if (new Set(postIds).size !== postIds.length) {
    issue(issues, "MAC_POST_INVENTORY_IDENTITY_AMBIGUOUS", "post-plan device identities must be unique");
  }

  const beforeIds = new Set(plan.devicesBefore.map((device) => device.identity).filter((value): value is string => Boolean(value)));
  const beforeFingerprints = new Set(plan.devicesBefore.map((device) => device.fingerprint).filter((value): value is string => Boolean(value)));
  const newDevices = plan.devicesAfter.filter((device) => device.identity && !beforeIds.has(device.identity));
  if (plan.attachResults.length !== 1 || plan.attachResults.some((result) => result.mountPoints.length !== 1)) {
    issue(issues, "MAC_ATTACH_CARDINALITY_OR_DOUBLE_MOUNT", "exactly one structured attach result and one mount point are required");
  }
  const attach = plan.attachResults.length === 1 ? plan.attachResults[0] : undefined;
  if (!attach?.recordIdentity || !attach.deviceIdentity || !plan.image.returnedImageIdentity) {
    issue(issues, "MAC_ATTACH_IDENTITY_MISSING", "attach record, returned device, and image identities must be explicit");
  }
  if (newDevices.length !== 1 || !attach?.deviceIdentity || newDevices[0]?.identity !== attach.deviceIdentity) {
    issue(issues, "MAC_NEW_DEVICE_CARDINALITY_OR_BINDING", "exactly one device absent from baseline must match the structured attach result");
  }
  const targetDevice = newDevices[0];
  if (targetDevice && (!targetDevice.fingerprint || targetDevice.isInternalHostDevice === null)) {
    issue(issues, "MAC_DEVICE_IDENTITY_UNPROVEN", "new device fingerprint and host-device classification must be known");
  }
  if (targetDevice && (targetDevice.isInternalHostDevice || beforeFingerprints.has(targetDevice.fingerprint ?? ""))) {
    issue(issues, "MAC_HOST_DEVICE_TARGET", "a baseline or internal host device is prohibited, including G1 disk2s5");
  }
  if (attach?.sourceImagePath && targetPath && canonicalPath(attach.sourceImagePath, "macos") !== targetPath) {
    issue(issues, "MAC_IMAGE_ORIGIN_MISMATCH", "structured attach result must prove the exact new image as backing origin");
  } else if (!attach?.sourceImagePath) {
    issue(issues, "MAC_IMAGE_ORIGIN_MISSING", "structured attach result must include a backing image origin");
  }
  if (!mountPath || !attach?.mountPoints[0] || canonicalPath(attach.mountPoints[0], "macos") !== mountPath) {
    issue(issues, "MAC_MOUNTPOINT_AMBIGUOUS_OR_MISMATCH", "mount point must be the exact unique contained empty path");
  }
  if (attach && (attach.fileSystem !== "APFS" || attach.mountPointEmpty !== true || attach.mounted !== true)) {
    issue(issues, "MAC_VOLUME_VERIFICATION_FAILED", "attached volume must be mounted, empty at target, and verified APFS");
  }
}

function buildIntents(plan: SafetyPlan): DryRunIntent[] {
  const execution = "NEVER_EXECUTED" as const;
  if (plan.platform === "windows") {
    const diskIdentity = plan.vhd.attachedDiskIdentities[0] as string;
    const partitionIdentity = plan.partition?.identity as string;
    return [
      { operation: "CREATE_FIXED_VHDX_FILE", path: plan.vhd.exactPath, maximumBytes: plan.capacity.maximumImageBytes, execution },
      { operation: "ATTACH_RETURNED_VHD_OBJECT", imagePath: plan.vhd.exactPath, returnedObjectIdentity: plan.vhd.returnedObjectIdentity as string, noDriveLetter: true, execution },
      { operation: "INITIALIZE_EXACT_DISK_OBJECT", diskIdentity, partitionStyle: "GPT", execution },
      { operation: "CREATE_PARTITION_ON_EXACT_DISK", diskIdentity, partitionIdentity, execution },
      { operation: "FORMAT_EXACT_PARTITION_NTFS", partitionIdentity, fileSystem: "NTFS", execution },
      { operation: "REGISTER_CLEANUP_OBLIGATION", resourceIdentity: diskIdentity, force: false, execution },
    ];
  }
  const attach = plan.attachResults[0];
  const deviceIdentity = attach.deviceIdentity as string;
  return [
    { operation: "CREATE_UDRW_APFS_IMAGE", path: plan.image.exactPath, maximumBytes: plan.capacity.maximumImageBytes, sparse: false, execution },
    { operation: "ATTACH_EXACT_IMAGE_STRUCTURED", imagePath: plan.image.exactPath, imageIdentity: plan.image.returnedImageIdentity as string, mountPoint: attach.mountPoints[0], execution },
    { operation: "VERIFY_EXACT_APFS_VOLUME", deviceIdentity, mountPoint: attach.mountPoints[0], execution },
    { operation: "REGISTER_CLEANUP_OBLIGATION", resourceIdentity: deviceIdentity, force: false, execution },
  ];
}

export function evaluateSafetyPlan(plan: SafetyPlan): SafetyAssessment {
  const issues: SafetyIssue[] = [];
  validateCommon(plan, issues);
  if (plan.platform === "windows") validateWindows(plan, issues);
  else validateMac(plan, issues);

  if (issues.length > 0) {
    const status: ModelStatus = issues.some((item) => item.severity === "FAIL")
      ? "FAIL"
      : issues.some((item) => item.severity === "ENVIRONMENT_UNAVAILABLE")
        ? "ENVIRONMENT_UNAVAILABLE"
        : "SAFETY_REJECTION";
    return { status, issues, intents: [], executionMode: "STATIC_ONLY", physicalPassClaimed: false };
  }
  return { status: "DRY_RUN_SAFE", issues: [], intents: buildIntents(plan), executionMode: "STATIC_ONLY", physicalPassClaimed: false };
}

export type CleanupPhase = "ATTACHED" | "MOUNTED" | "UNMOUNTED" | "DETACHED" | "RELEASE_VERIFIED" | "REMOVED" | "CLEANUP_INCOMPLETE";

export type CleanupState = {
  platform: "windows" | "macos";
  phase: CleanupPhase;
  sandboxPath: string;
  imagePath: string;
  mountPoint: string;
  deviceIdentity: string;
  ownerToken: string;
  observedOwnerToken: string | null;
  imageIsOwned: boolean;
  imageIsSymlinkOrReparsePoint: boolean;
  mountPointIsSymlinkOrReparsePoint: boolean;
  unmountedProven: boolean;
  detachedProven: boolean;
  releaseProven: boolean;
  originalFailure: string;
  cleanupFailure: string | null;
};

export type CleanupIntent = {
  operation: "UNMOUNT_EXACT_MOUNTPOINT" | "DETACH_KNOWN_DEVICE" | "VERIFY_DEVICE_RELEASE" | "REMOVE_EXACT_OWNED_IMAGE";
  targetIdentity: string;
  force: false;
  execution: "NEVER_EXECUTED";
};

export type CleanupEvent =
  | { kind: "unmount"; targetIdentity: string; succeeded: boolean; confirmedUnmounted: boolean }
  | { kind: "detach"; targetIdentity: string; succeeded: boolean; forced: boolean }
  | { kind: "release-check"; targetIdentity: string; identityAbsent: boolean }
  | { kind: "remove-image"; targetIdentity: string; succeeded: boolean };

export type CleanupAssessment = {
  status: ModelStatus;
  cleanupStatus: ModelStatus;
  phase: CleanupPhase;
  originalFailure: string;
  cleanupFailure: string | null;
  physicalPassClaimed: false;
};

export function assessCleanup(state: CleanupState): CleanupAssessment {
  const cleanupStatus: ModelStatus = state.phase === "REMOVED"
    ? "DRY_RUN_SAFE"
    : state.phase === "CLEANUP_INCOMPLETE"
      ? "CLEANUP_INCOMPLETE"
      : "SAFETY_REJECTION";
  return {
    status: state.originalFailure ? "FAIL" : cleanupStatus,
    cleanupStatus,
    phase: state.phase,
    originalFailure: state.originalFailure,
    cleanupFailure: state.cleanupFailure,
    physicalPassClaimed: false,
  };
}

function cleanupIncomplete(state: CleanupState, reason: string): CleanupState {
  return { ...state, phase: "CLEANUP_INCOMPLETE", cleanupFailure: reason };
}

export function nextCleanupIntent(state: CleanupState): CleanupIntent | null {
  const sandbox = canonicalPath(state.sandboxPath, state.platform);
  const image = canonicalPath(state.imagePath, state.platform);
  const mountPoint = canonicalPath(state.mountPoint, state.platform);
  if (
    !sandbox || !image || !mountPoint || dirname(image) !== sandbox || dirname(mountPoint) !== sandbox ||
    !state.ownerToken || state.ownerToken !== state.observedOwnerToken || !state.deviceIdentity ||
    state.imageIsSymlinkOrReparsePoint || state.mountPointIsSymlinkOrReparsePoint
  ) return null;
  if ((state.platform === "windows" && state.phase === "MOUNTED") || (state.platform === "macos" && state.phase === "ATTACHED")) return null;
  if (state.phase === "UNMOUNTED" && (state.platform !== "macos" || !state.unmountedProven)) return null;
  if (state.phase === "DETACHED" && (!state.detachedProven || (state.platform === "macos" && !state.unmountedProven))) return null;
  if (state.phase === "RELEASE_VERIFIED" && (!state.detachedProven || !state.releaseProven || (state.platform === "macos" && !state.unmountedProven))) return null;
  if (state.phase === "MOUNTED") {
    return { operation: "UNMOUNT_EXACT_MOUNTPOINT", targetIdentity: state.mountPoint, force: false, execution: "NEVER_EXECUTED" };
  }
  if (state.phase === "ATTACHED" || state.phase === "UNMOUNTED") {
    if (!state.deviceIdentity) return null;
    return { operation: "DETACH_KNOWN_DEVICE", targetIdentity: state.deviceIdentity, force: false, execution: "NEVER_EXECUTED" };
  }
  if (state.phase === "DETACHED") {
    if (!state.deviceIdentity) return null;
    return { operation: "VERIFY_DEVICE_RELEASE", targetIdentity: state.deviceIdentity, force: false, execution: "NEVER_EXECUTED" };
  }
  if (state.phase === "RELEASE_VERIFIED") {
    if (!state.imageIsOwned) return null;
    return { operation: "REMOVE_EXACT_OWNED_IMAGE", targetIdentity: state.imagePath, force: false, execution: "NEVER_EXECUTED" };
  }
  return null;
}

export function advanceCleanup(state: CleanupState, event: CleanupEvent): CleanupState {
  const intent = nextCleanupIntent(state);
  if (!intent) return cleanupIncomplete(state, "cleanup identity, ownership, or release proof is incomplete");
  if (event.targetIdentity !== intent.targetIdentity) return cleanupIncomplete(state, "cleanup target identity did not match the known resource");

  if (state.phase === "MOUNTED" && event.kind === "unmount") {
    if (!event.succeeded || !event.confirmedUnmounted) return cleanupIncomplete(state, "exact mount absence could not be verified");
    return { ...state, phase: "UNMOUNTED", unmountedProven: true, cleanupFailure: null };
  }
  if ((state.phase === "ATTACHED" || state.phase === "UNMOUNTED") && event.kind === "detach") {
    if (!event.succeeded || event.forced) return cleanupIncomplete(state, "known-resource detach failed or requested force");
    return { ...state, phase: "DETACHED", detachedProven: true, cleanupFailure: null };
  }
  if (state.phase === "DETACHED" && event.kind === "release-check") {
    if (!event.identityAbsent) return cleanupIncomplete(state, "device release could not be proven");
    return { ...state, phase: "RELEASE_VERIFIED", releaseProven: true, cleanupFailure: null };
  }
  if (state.phase === "RELEASE_VERIFIED" && event.kind === "remove-image") {
    if (!event.succeeded) return cleanupIncomplete(state, "exact owned image removal failed");
    return { ...state, phase: "REMOVED", cleanupFailure: null };
  }
  return cleanupIncomplete(state, "cleanup event does not match the required state transition");
}
