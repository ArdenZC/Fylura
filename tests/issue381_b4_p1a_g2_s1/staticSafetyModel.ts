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
  deviceRole: "host" | "image-root" | "partition" | "apfs-container" | "apfs-volume" | "UNKNOWN";
  parentIdentity: string | null;
  sourceImageIdentity: string | null;
  sizeBytes: number | null;
};

export type MacSafetyPlan = CommonSafetyPlan & {
  platform: "macos";
  selection: "structured-attach-result" | "guessed-device-path" | "broad-selector" | "implicit-default";
  image: {
    exactPath: string;
    format: "UDRW" | "UDIF" | "SPARSE" | "SPARSEBUNDLE" | "UNKNOWN";
    sparse: boolean;
    returnedImageIdentity: string | null;
    creationRecordIdentity: string | null;
  };
  devicesBefore: SimulatedDevice[];
  devicesAfter: SimulatedDevice[];
  attachResults: Array<{
    recordIdentity: string | null;
    imageIdentity: string | null;
    creationRecordIdentity: string | null;
    rootDeviceIdentity: string | null;
    deviceIdentities: string[];
    mountedVolumeIdentity: string | null;
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
  s2Eligibility: "NOT_QUALIFIED";
};

function issue(
  issues: SafetyIssue[],
  code: string,
  detail: string,
  severity: SafetyIssue["severity"] = "SAFETY_REJECTION",
): void {
  issues.push({ code, detail, severity });
}

function validatePositiveBytes(value: unknown, code: string, label: string, issues: SafetyIssue[]): void {
  if (value === null || value === undefined) {
    issue(issues, `${code}_UNKNOWN`, `${label} is missing`, "ENVIRONMENT_UNAVAILABLE");
  } else if (typeof value !== "number" || !Number.isSafeInteger(value) || value <= 0) {
    issue(issues, `${code}_INVALID`, `${label} must be a positive safe integer byte count`);
  }
}

function isUniqueNonEmpty(
  values: Array<string | null | undefined>,
  normalize: (value: string) => string = (value) => value.trim(),
): boolean {
  const normalized = values.map((value) => typeof value === "string" ? normalize(value) : "");
  return normalized.every(Boolean) && new Set(normalized).size === normalized.length;
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
    if (!plan.before.hostFingerprint?.trim() || !plan.after.hostFingerprint?.trim() || plan.before.hostFingerprint !== plan.after.hostFingerprint) {
      issue(issues, "HOST_IDENTITY_CHANGED", "host fingerprint changed between inventory checks (TOCTOU)");
    }
    for (const [label, snapshot] of [["before", plan.before], ["after", plan.after]] as const) {
      validatePositiveBytes(snapshot.availableBytes, `HOST_CAPACITY_${label.toUpperCase()}_AVAILABLE`, `${label} available bytes`, issues);
      validatePositiveBytes(snapshot.independentAvailableCapBytes, `HOST_CAPACITY_${label.toUpperCase()}_CAP`, `${label} independent free-space cap`, issues);
      if (typeof snapshot.availableBytes === "number" && typeof snapshot.independentAvailableCapBytes === "number" && snapshot.availableBytes > snapshot.independentAvailableCapBytes) {
        issue(issues, `HOST_CAPACITY_${label.toUpperCase()}_OVERSTATED`, `${label} free-space report exceeds the independent cap`);
      }
      if (typeof snapshot.availableBytes === "number" && snapshot.availableBytes < MIN_HOST_FREE_BYTES ||
          typeof snapshot.independentAvailableCapBytes === "number" && snapshot.independentAvailableCapBytes < MIN_HOST_FREE_BYTES) {
        issue(issues, `HOST_CAPACITY_${label.toUpperCase()}_BELOW_FLOOR`, `${label} host free space must be at least 4 GiB`, "ENVIRONMENT_UNAVAILABLE");
      }
    }
  }

  const capacity = plan.capacity;
  validatePositiveBytes(capacity.nominalVolumeBytes, "NOMINAL_VOLUME", "nominal volume size", issues);
  validatePositiveBytes(capacity.maximumImageBytes, "IMAGE_CAP", "maximum image size", issues);
  validatePositiveBytes(capacity.observedImageBytes, "OBSERVED_IMAGE", "observed image size", issues);
  validatePositiveBytes(capacity.maximumHostDeltaBytes, "HOST_DELTA_CAP", "maximum host-space delta", issues);
  validatePositiveBytes(capacity.observedHostDeltaBytes, "OBSERVED_HOST_DELTA", "observed host-space delta", issues);
  if (capacity.backingKnown !== true) {
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
  if (capacity.allocation !== "fixed" || capacity.sparse !== false || capacity.unboundedGrowth !== false) {
    issue(issues, "ALLOCATION_NOT_BOUNDED", "only fixed, non-sparse, bounded allocation is admissible");
  }
  if (capacity.importsExistingStorage !== false) {
    issue(issues, "EXISTING_STORAGE_IMPORT_FORBIDDEN", "source disk or partition import is forbidden");
  }
  if (capacity.targetKind !== "sandbox-file") {
    issue(issues, "NON_FILE_TARGET_FORBIDDEN", "a physical-device target is never admissible");
  }
}

function validateWindows(plan: WindowsSafetyPlan, issues: SafetyIssue[]): void {
  const identityKey = (value: string | null): string => value?.trim().toLowerCase() ?? "";
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

  const validateDisk = (disk: SimulatedDisk, label: string): void => {
    if (!disk.identity?.trim() || !disk.fingerprint?.trim() || !disk.friendlyName?.trim() ||
        !Number.isSafeInteger(disk.diskNumber) || (disk.diskNumber ?? -1) < 0 ||
        typeof disk.isSystem !== "boolean" || typeof disk.isBoot !== "boolean" || typeof disk.isOsDisk !== "boolean" ||
        !["RAW", "GPT", "MBR"].includes(disk.partitionStyle)) {
      issue(issues, `WINDOWS_${label}_IDENTITY_UNPROVEN`, `${label.toLowerCase()} disk identity and critical attributes must be complete`);
    }
    validatePositiveBytes(disk.sizeBytes, `WINDOWS_${label}_DISK_SIZE`, `${label.toLowerCase()} disk size`, issues);
  };
  if (plan.disksBefore.length === 0) {
    issue(issues, "WINDOWS_BASELINE_IDENTITY_UNPROVEN", "pre-mutation disk inventory cannot be empty");
  }
  plan.disksBefore.forEach((disk) => validateDisk(disk, "BASELINE"));
  plan.disksAfter.forEach((disk) => validateDisk(disk, "POST_INVENTORY"));

  const beforeIds = plan.disksBefore.map((disk) => disk.identity);
  const beforeFingerprints = plan.disksBefore.map((disk) => disk.fingerprint);
  const beforeNumbers = plan.disksBefore.map((disk) => disk.diskNumber);
  const postIds = plan.disksAfter.map((disk) => disk.identity);
  const postFingerprints = plan.disksAfter.map((disk) => disk.fingerprint);
  const postNumbers = plan.disksAfter.map((disk) => disk.diskNumber);
  if (!isUniqueNonEmpty(beforeIds, (value) => value.trim().toLowerCase()) ||
      !isUniqueNonEmpty(beforeFingerprints, (value) => value.trim().toLowerCase()) ||
      new Set(beforeNumbers).size !== beforeNumbers.length) {
    issue(issues, "WINDOWS_BASELINE_IDENTITY_AMBIGUOUS", "baseline disk identity, fingerprint, and disk number must be unique");
  }
  if (!isUniqueNonEmpty(postIds, (value) => value.trim().toLowerCase()) ||
      !isUniqueNonEmpty(postFingerprints, (value) => value.trim().toLowerCase()) ||
      new Set(postNumbers).size !== postNumbers.length) {
    issue(issues, "WINDOWS_POST_INVENTORY_IDENTITY_AMBIGUOUS", "post-plan disk identity, fingerprint, and disk number must be unique");
  }

  const postById = new Map(plan.disksAfter.map((disk) => [identityKey(disk.identity), disk]));
  for (const baseline of plan.disksBefore) {
    const post = postById.get(identityKey(baseline.identity));
    if (!post) {
      issue(issues, "WINDOWS_BASELINE_DEVICE_DISAPPEARED", "every original host disk must remain present after the plan");
      continue;
    }
    if (
      post.identity !== baseline.identity || post.fingerprint !== baseline.fingerprint || post.diskNumber !== baseline.diskNumber ||
      post.friendlyName !== baseline.friendlyName || post.sizeBytes !== baseline.sizeBytes ||
      post.partitionStyle !== baseline.partitionStyle || post.isSystem !== baseline.isSystem ||
      post.isBoot !== baseline.isBoot || post.isOsDisk !== baseline.isOsDisk
    ) {
      issue(issues, "WINDOWS_BASELINE_DEVICE_CHANGED", "original host disk identity, fingerprint, or critical attributes changed");
    }
  }

  const baselineIds = new Set(beforeIds.map((identity) => identityKey(identity)));
  const baselineFingerprints = new Set(beforeFingerprints.map((fingerprint) => identityKey(fingerprint)));
  const newDisks = plan.disksAfter.filter((disk) => disk.identity && !baselineIds.has(identityKey(disk.identity)));
  const attachedIds = plan.vhd.attachedDiskIdentities.filter((identity): identity is string => Boolean(identity));
  if (plan.vhd.attachedDiskIdentities.some((identity) => identity === null) || attachedIds.length !== 1) {
    issue(issues, "WINDOWS_RETURNED_DISK_IDENTITY_AMBIGUOUS", "exactly one non-empty disk identity must be bound to the returned VHD object");
  }
  if (newDisks.length !== 1 || attachedIds.length !== 1 || newDisks[0]?.identity !== attachedIds[0]) {
    issue(issues, "WINDOWS_NEW_DISK_CARDINALITY_OR_BINDING", "exactly one previously unseen disk must match the returned VHD object");
  }
  const targetDisk = newDisks[0];
  if (targetDisk && (!targetDisk.fingerprint || typeof targetDisk.isSystem !== "boolean" || typeof targetDisk.isBoot !== "boolean" || typeof targetDisk.isOsDisk !== "boolean")) {
    issue(issues, "WINDOWS_DISK_IDENTITY_UNPROVEN", "new disk identity and system/boot/OS flags must be known");
  }
  if (targetDisk && (targetDisk.isSystem !== false || targetDisk.isBoot !== false || targetDisk.isOsDisk !== false)) {
    issue(issues, "WINDOWS_HOST_DISK_TARGET", "system, boot, or OS disks are prohibited targets");
  }
  if (targetDisk && baselineFingerprints.has(identityKey(targetDisk.fingerprint))) {
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
  if (!plan.image.creationRecordIdentity?.trim()) {
    issue(issues, "MAC_IMAGE_CREATION_PROVENANCE_MISSING", "image creation record identity must be explicit");
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

  const validateDevice = (device: SimulatedDevice, label: string): void => {
    if (!device.identity?.trim() || !device.fingerprint?.trim() || typeof device.isInternalHostDevice !== "boolean") {
      issue(issues, `MAC_${label}_IDENTITY_UNPROVEN`, `${label.toLowerCase()} device identity, fingerprint, and host classification must be explicit`);
    }
    validatePositiveBytes(device.sizeBytes, `MAC_${label}_DEVICE_SIZE`, `${label.toLowerCase()} device size`, issues);
  };
  if (plan.devicesBefore.length === 0) {
    issue(issues, "MAC_BASELINE_IDENTITY_UNPROVEN", "pre-mutation device inventory cannot be empty");
  }
  plan.devicesBefore.forEach((device) => validateDevice(device, "BASELINE"));
  plan.devicesAfter.forEach((device) => validateDevice(device, "POST_INVENTORY"));

  const beforeIds = plan.devicesBefore.map((device) => device.identity);
  const beforeFingerprints = plan.devicesBefore.map((device) => device.fingerprint);
  const postIds = plan.devicesAfter.map((device) => device.identity);
  const postFingerprints = plan.devicesAfter.map((device) => device.fingerprint);
  if (!isUniqueNonEmpty(beforeIds) || !isUniqueNonEmpty(beforeFingerprints)) {
    issue(issues, "MAC_BASELINE_IDENTITY_AMBIGUOUS", "baseline device identities and fingerprints must be unique");
  }
  if (!isUniqueNonEmpty(postIds) || !isUniqueNonEmpty(postFingerprints)) {
    issue(issues, "MAC_POST_INVENTORY_IDENTITY_AMBIGUOUS", "post-plan device identities and fingerprints must be unique");
  }

  const beforeIdSet = new Set(beforeIds);
  const postById = new Map(plan.devicesAfter.map((device) => [device.identity, device]));
  for (const baseline of plan.devicesBefore) {
    if (baseline.deviceRole !== "host" || baseline.sourceImageIdentity !== null) {
      issue(issues, "MAC_BASELINE_DEVICE_ROLE_INVALID", "every baseline record must be an existing host device, not image-owned");
    }
    const post = postById.get(baseline.identity);
    if (!post) {
      issue(issues, "MAC_BASELINE_DEVICE_DISAPPEARED", "every original host device must remain present after the plan");
      continue;
    }
    if (
      post.fingerprint !== baseline.fingerprint || post.isInternalHostDevice !== baseline.isInternalHostDevice ||
      post.deviceRole !== baseline.deviceRole || post.parentIdentity !== baseline.parentIdentity ||
      post.sourceImageIdentity !== baseline.sourceImageIdentity || post.sizeBytes !== baseline.sizeBytes
    ) {
      issue(issues, "MAC_BASELINE_DEVICE_CHANGED", "original host device identity, fingerprint, hierarchy, or critical attributes changed");
    }
    if (baseline.parentIdentity && !beforeIdSet.has(baseline.parentIdentity)) {
      issue(issues, "MAC_BASELINE_HIERARCHY_INCOMPLETE", "baseline host-device parent must also appear in the pre-mutation inventory");
    }
  }

  const baselineIds = new Set(beforeIds);
  const baselineFingerprints = new Set(beforeFingerprints);
  const newDevices = plan.devicesAfter.filter((device) => device.identity && !baselineIds.has(device.identity));
  if (plan.attachResults.length !== 1 || plan.attachResults.some((result) => result.mountPoints.length !== 1)) {
    issue(issues, "MAC_ATTACH_CARDINALITY_OR_DOUBLE_MOUNT", "one structured attach envelope and exactly one intended mount point are required");
  }
  const attach = plan.attachResults.length === 1 ? plan.attachResults[0] : undefined;
  if (!attach?.recordIdentity?.trim() || !attach.imageIdentity?.trim() || !attach.creationRecordIdentity?.trim() ||
      !attach.rootDeviceIdentity?.trim() || !attach.mountedVolumeIdentity?.trim() || !attach.deviceIdentity?.trim() ||
      !plan.image.returnedImageIdentity?.trim()) {
    issue(issues, "MAC_ATTACH_IDENTITY_MISSING", "attach, image, root device, and mounted volume identities must be explicit");
  }
  if (attach?.imageIdentity !== plan.image.returnedImageIdentity ||
      attach?.creationRecordIdentity !== plan.image.creationRecordIdentity) {
    issue(issues, "MAC_IMAGE_CREATION_PROVENANCE_MISMATCH", "attach record must reference the exact image creation record and returned image identity");
  }

  const newById = new Map(newDevices.map((device) => [device.identity, device]));
  const roots = newDevices.filter((device) => device.deviceRole === "image-root");
  const root = roots.length === 1 ? roots[0] : undefined;
  if (newDevices.length === 0 || roots.length !== 1 || !root || root.identity !== attach?.rootDeviceIdentity || root.parentIdentity !== null) {
    issue(issues, "MAC_IMAGE_HIERARCHY_ROOT_INVALID", "exactly one new image root must be bound to the attach envelope with no parent");
  }
  if (!attach || !isUniqueNonEmpty(attach.deviceIdentities) || attach.deviceIdentities.length !== newDevices.length ||
      newDevices.some((device) => !attach.deviceIdentities.includes(device.identity ?? "")) ||
      attach.deviceIdentities.some((identity) => !newById.has(identity))) {
    issue(issues, "MAC_IMAGE_HIERARCHY_INVENTORY_MISMATCH", "attach output must enumerate every and only new image-owned device exactly once");
  }

  const parentRoles: Record<Exclude<SimulatedDevice["deviceRole"], "host" | "UNKNOWN">, string[]> = {
    "image-root": [],
    partition: ["image-root"],
    "apfs-container": ["image-root", "partition"],
    "apfs-volume": ["image-root", "partition", "apfs-container"],
  };
  const imageIdentity = plan.image.returnedImageIdentity;
  for (const device of newDevices) {
    if (!device.identity || !device.fingerprint || device.isInternalHostDevice !== false ||
        device.sourceImageIdentity !== imageIdentity || device.deviceRole === "host" || device.deviceRole === "UNKNOWN") {
      issue(issues, "MAC_IMAGE_DEVICE_ORIGIN_INVALID", "every new device must be non-host and explicitly originate from this exact image");
    }
    if (baselineFingerprints.has(device.fingerprint ?? "")) {
      issue(issues, "MAC_HOST_DEVICE_TARGET", "new image-device fingerprint must not reuse any pre-mutation host identity, including /dev/disk2s5");
    }
    if (typeof device.sizeBytes === "number" && device.sizeBytes > plan.capacity.maximumImageBytes) {
      issue(issues, "MAC_IMAGE_DEVICE_SIZE_EXCEEDS_IMAGE_CAP", "an image-owned device cannot exceed the bounded image size");
    }
    if (device.deviceRole === "apfs-volume" && typeof device.sizeBytes === "number" && device.sizeBytes > plan.capacity.nominalVolumeBytes) {
      issue(issues, "MAC_APFS_VOLUME_SIZE_EXCEEDS_VOLUME_CAP", "an APFS volume cannot exceed the nominal virtual-volume cap");
    }
    if (device.deviceRole !== "image-root") {
      const parent = device.parentIdentity ? newById.get(device.parentIdentity) : undefined;
      if (!parent || !parentRoles[device.deviceRole as keyof typeof parentRoles]?.includes(parent.deviceRole)) {
        issue(issues, "MAC_IMAGE_HIERARCHY_PARENT_INVALID", "every image child must have one allowed parent inside the same new image hierarchy");
      } else if (typeof device.sizeBytes === "number" && typeof parent.sizeBytes === "number" && device.sizeBytes > parent.sizeBytes) {
        issue(issues, "MAC_IMAGE_HIERARCHY_SIZE_INVALID", "a child device cannot claim more capacity than its image-owned parent");
      }
    }

    let current: SimulatedDevice | undefined = device;
    const seen = new Set<string>();
    while (current && current.deviceRole !== "image-root") {
      if (!current.identity || seen.has(current.identity)) break;
      seen.add(current.identity);
      current = current.parentIdentity ? newById.get(current.parentIdentity) : undefined;
    }
    if (!current || current.deviceRole !== "image-root" || current.identity !== root?.identity) {
      issue(issues, "MAC_IMAGE_HIERARCHY_ORPHAN_OR_CYCLE", "every new device must reach the one exact image root without cycles or host-parent links");
    }
  }

  const mountedVolume = attach?.mountedVolumeIdentity ? newById.get(attach.mountedVolumeIdentity) : undefined;
  if (!mountedVolume || mountedVolume.deviceRole !== "apfs-volume" || mountedVolume.sourceImageIdentity !== imageIdentity) {
    issue(issues, "MAC_MOUNTED_VOLUME_BINDING_INVALID", "the mounted APFS volume must be an image-owned APFS-volume node in the exact hierarchy");
  }
  if (attach && attach.deviceIdentity !== attach.mountedVolumeIdentity) {
    issue(issues, "MAC_MOUNTED_VOLUME_BINDING_INVALID", "legacy selected device identity must match the explicitly mounted APFS volume");
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
  const deviceIdentity = attach.mountedVolumeIdentity as string;
  return [
    { operation: "CREATE_UDRW_APFS_IMAGE", path: plan.image.exactPath, maximumBytes: plan.capacity.maximumImageBytes, sparse: false, execution },
    { operation: "ATTACH_EXACT_IMAGE_STRUCTURED", imagePath: plan.image.exactPath, imageIdentity: plan.image.returnedImageIdentity as string, mountPoint: attach.mountPoints[0], execution },
    { operation: "VERIFY_EXACT_APFS_VOLUME", deviceIdentity, mountPoint: attach.mountPoints[0], execution },
    { operation: "REGISTER_CLEANUP_OBLIGATION", resourceIdentity: deviceIdentity, force: false, execution },
  ];
}

export function evaluateSafetyPlan(plan: SafetyPlan): SafetyAssessment {
  const issues: SafetyIssue[] = [];
  try {
    if (!plan || typeof plan !== "object" || (plan.platform !== "windows" && plan.platform !== "macos")) {
      issue(issues, "SAFETY_PLAN_OR_PLATFORM_MISSING", "a supported complete safety-plan object is required", "ENVIRONMENT_UNAVAILABLE");
    } else if (!plan.capacity || typeof plan.capacity !== "object") {
      issue(issues, "CAPACITY_PROOF_UNAVAILABLE", "required capacity evidence is missing or unavailable", "ENVIRONMENT_UNAVAILABLE");
    } else {
      validateCommon(plan, issues);
      if (plan.platform === "windows") validateWindows(plan, issues);
      else validateMac(plan, issues);
    }
  } catch {
    issue(issues, "MALFORMED_SAFETY_PLAN", "required safety-plan facts were missing or malformed");
  }

  if (issues.length > 0) {
    const status: ModelStatus = issues.some((item) => item.severity === "FAIL")
      ? "FAIL"
      : issues.some((item) => item.severity === "ENVIRONMENT_UNAVAILABLE")
        ? "ENVIRONMENT_UNAVAILABLE"
        : "SAFETY_REJECTION";
    return { status, issues, intents: [], executionMode: "STATIC_ONLY", physicalPassClaimed: false, s2Eligibility: "NOT_QUALIFIED" };
  }
  return { status: "DRY_RUN_SAFE", issues: [], intents: buildIntents(plan), executionMode: "STATIC_ONLY", physicalPassClaimed: false, s2Eligibility: "NOT_QUALIFIED" };
}

export type CleanupPhase = "ATTACHED" | "MOUNTED" | "UNMOUNTED" | "DETACHED" | "RELEASE_VERIFIED" | "REMOVED" | "CLEANUP_INCOMPLETE";

declare const cleanupSessionBrand: unique symbol;
declare const cleanupIntentBrand: unique symbol;

/** Opaque handle. The backing state exists only in this module's WeakMap. */
export type CleanupSession = { readonly [cleanupSessionBrand]: true };

export type CleanupStart = {
  platform: "windows" | "macos";
  sandboxPath: string;
  imagePath: string;
  mountPoint: string;
  deviceIdentity: string;
  imageIdentity: string;
  ownerToken: string;
  observedOwnerToken: string | null;
  imageIsOwned: boolean;
  imageIsSymlinkOrReparsePoint: boolean;
  mountPointIsSymlinkOrReparsePoint: boolean;
  originalFailure: string;
};

export type CleanupIntent = Readonly<{
  operation: "UNMOUNT_EXACT_MOUNTPOINT" | "DETACH_KNOWN_DEVICE" | "VERIFY_DEVICE_RELEASE" | "REMOVE_EXACT_OWNED_IMAGE";
  targetIdentity: string;
  force: false;
  execution: "NEVER_EXECUTED";
  readonly [cleanupIntentBrand]: true;
}>;

type CleanupEventBase = {
  targetIdentity: string;
  observedOwnerToken: string | null;
  observedDeviceIdentity: string;
  observedImageIdentity: string;
};

export type CleanupEvent =
  | (CleanupEventBase & { kind: "unmount"; succeeded: boolean; confirmedUnmounted: boolean })
  | (CleanupEventBase & { kind: "detach"; succeeded: boolean; forced: boolean })
  | (CleanupEventBase & { kind: "release-check"; identityAbsent: boolean })
  | (CleanupEventBase & { kind: "remove-image"; succeeded: boolean });

export type CleanupSnapshot = {
  platform: "windows" | "macos";
  phase: CleanupPhase;
  sandboxPath: string;
  imagePath: string;
  mountPoint: string;
  deviceIdentity: string;
  imageIdentity: string;
  ownerToken: string;
  originalFailure: string;
  cleanupFailure: string | null;
};

export type CleanupAssessment = {
  status: ModelStatus;
  cleanupStatus: ModelStatus;
  phase: CleanupPhase;
  originalFailure: string;
  cleanupFailure: string | null;
  physicalPassClaimed: false;
  s2Eligibility: "NOT_QUALIFIED";
};

type InternalCleanup = CleanupStart & {
  phase: CleanupPhase;
  cleanupFailure: string | null;
  sequence: number;
  pendingIntent: CleanupIntent | null;
};

const cleanupSessions = new WeakMap<object, InternalCleanup>();
const cleanupIntentOwners = new WeakMap<object, { session: object; sequence: number }>();

function validCleanupStart(start: CleanupStart): string | null {
  const sandbox = canonicalPath(start.sandboxPath, start.platform);
  const image = canonicalPath(start.imagePath, start.platform);
  const mountPoint = canonicalPath(start.mountPoint, start.platform);
  if (!sandbox || !image || !mountPoint || dirname(image) !== sandbox || dirname(mountPoint) !== sandbox) {
    return "cleanup paths are invalid or escape the owned sandbox";
  }
  if (!start.ownerToken || start.ownerToken !== start.observedOwnerToken) return "cleanup ownership token is not verified";
  if (!start.deviceIdentity || !start.imageIdentity || !start.imageIsOwned) return "cleanup resource identity or image ownership is unproven";
  if (start.imageIsSymlinkOrReparsePoint || start.mountPointIsSymlinkOrReparsePoint) return "cleanup target contains a link or reparse point";
  return null;
}

export function createCleanupSession(start: CleanupStart): CleanupSession {
  const handle = Object.freeze({}) as CleanupSession;
  const startError = validCleanupStart(start);
  cleanupSessions.set(handle, {
    ...start,
    phase: start.platform === "macos" ? "MOUNTED" : "ATTACHED",
    cleanupFailure: startError,
    sequence: 0,
    pendingIntent: null,
  });
  if (startError) cleanupSessions.get(handle)!.phase = "CLEANUP_INCOMPLETE";
  return handle;
}

function snapshot(state: InternalCleanup): CleanupSnapshot {
  return {
    platform: state.platform,
    phase: state.phase,
    sandboxPath: state.sandboxPath,
    imagePath: state.imagePath,
    mountPoint: state.mountPoint,
    deviceIdentity: state.deviceIdentity,
    imageIdentity: state.imageIdentity,
    ownerToken: state.ownerToken,
    originalFailure: state.originalFailure,
    cleanupFailure: state.cleanupFailure,
  };
}

export function inspectCleanup(session: CleanupSession): CleanupSnapshot | null {
  const state = cleanupSessions.get(session as object);
  return state ? snapshot(state) : null;
}

function cleanupIncomplete(state: InternalCleanup, reason: string): CleanupSnapshot {
  state.phase = "CLEANUP_INCOMPLETE";
  state.cleanupFailure = reason;
  state.pendingIntent = null;
  state.sequence += 1;
  return snapshot(state);
}

export function assessCleanup(session: CleanupSession): CleanupAssessment {
  const state = cleanupSessions.get(session as object);
  if (!state) {
    return {
      status: "SAFETY_REJECTION",
      cleanupStatus: "CLEANUP_INCOMPLETE",
      phase: "CLEANUP_INCOMPLETE",
      originalFailure: "",
      cleanupFailure: "cleanup session handle was not created by the model",
      physicalPassClaimed: false,
      s2Eligibility: "NOT_QUALIFIED",
    };
  }
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
    s2Eligibility: "NOT_QUALIFIED",
  };
}

export function nextCleanupIntent(session: CleanupSession): CleanupIntent | null {
  const state = cleanupSessions.get(session as object);
  if (!state || state.phase === "CLEANUP_INCOMPLETE" || state.phase === "REMOVED") return null;
  if (validCleanupStart(state)) return null;
  if (state.pendingIntent) return state.pendingIntent;

  let operation: CleanupIntent["operation"] | null = null;
  let targetIdentity = "";
  if (state.phase === "MOUNTED" && state.platform === "macos") {
    operation = "UNMOUNT_EXACT_MOUNTPOINT";
    targetIdentity = state.mountPoint;
  } else if (state.phase === "ATTACHED" || state.phase === "UNMOUNTED") {
    operation = "DETACH_KNOWN_DEVICE";
    targetIdentity = state.deviceIdentity;
  } else if (state.phase === "DETACHED") {
    operation = "VERIFY_DEVICE_RELEASE";
    targetIdentity = state.deviceIdentity;
  } else if (state.phase === "RELEASE_VERIFIED") {
    operation = "REMOVE_EXACT_OWNED_IMAGE";
    targetIdentity = state.imagePath;
  }
  if (!operation || !targetIdentity) return null;

  const intent = Object.freeze({
    operation,
    targetIdentity,
    force: false as const,
    execution: "NEVER_EXECUTED" as const,
  }) as CleanupIntent;
  cleanupIntentOwners.set(intent, { session: session as object, sequence: state.sequence });
  state.pendingIntent = intent;
  return intent;
}

export function advanceCleanup(
  session: CleanupSession,
  intent: CleanupIntent,
  event: CleanupEvent,
): CleanupSnapshot | null {
  const state = cleanupSessions.get(session as object);
  if (!state) return null;
  const owner = cleanupIntentOwners.get(intent as object);
  if (!owner || owner.session !== session || owner.sequence !== state.sequence || state.pendingIntent !== intent) {
    return cleanupIncomplete(state, "cleanup transition requires the current issued intent in this exact sequence");
  }
  if (validCleanupStart(state)) return cleanupIncomplete(state, "cleanup identity, ownership, containment, or image proof changed");
  if (
    event.targetIdentity !== intent.targetIdentity || event.observedOwnerToken !== state.ownerToken ||
    event.observedDeviceIdentity !== state.deviceIdentity || event.observedImageIdentity !== state.imageIdentity
  ) return cleanupIncomplete(state, "cleanup event resource or owner identity changed");

  if (intent.operation === "UNMOUNT_EXACT_MOUNTPOINT" && event.kind === "unmount") {
    if (!event.succeeded || !event.confirmedUnmounted) return cleanupIncomplete(state, "exact mount absence could not be verified");
    state.phase = "UNMOUNTED";
  } else if (intent.operation === "DETACH_KNOWN_DEVICE" && event.kind === "detach") {
    if (!event.succeeded || event.forced) return cleanupIncomplete(state, "known-resource detach failed or requested force");
    state.phase = "DETACHED";
  } else if (intent.operation === "VERIFY_DEVICE_RELEASE" && event.kind === "release-check") {
    if (!event.identityAbsent) return cleanupIncomplete(state, "device release could not be proven");
    state.phase = "RELEASE_VERIFIED";
  } else if (intent.operation === "REMOVE_EXACT_OWNED_IMAGE" && event.kind === "remove-image") {
    if (!event.succeeded) return cleanupIncomplete(state, "exact owned image removal failed");
    state.phase = "REMOVED";
  } else {
    return cleanupIncomplete(state, "cleanup event is repeated, reversed, or mismatched to the current intent");
  }

  state.cleanupFailure = null;
  state.pendingIntent = null;
  state.sequence += 1;
  return snapshot(state);
}
