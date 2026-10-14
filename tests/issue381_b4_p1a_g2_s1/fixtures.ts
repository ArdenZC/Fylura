import {
  GIB,
  MAX_HOST_DELTA_BYTES,
  MAX_IMAGE_BYTES,
  MAX_NOMINAL_VOLUME_BYTES,
  MIB,
  createCleanupSession,
  type CleanupSession,
  type CleanupStart,
  type MacSafetyPlan,
  type SimulatedDevice,
  type SimulatedDisk,
  type WindowsSafetyPlan,
} from "./staticSafetyModel";

const SOURCE_SHA256 = "a".repeat(64);
const TEST_HOST_FREE = 8 * GIB;

const g1WindowsSystemDisk: SimulatedDisk = {
  identity: "g1-host-disk-0-system",
  fingerprint: "g1:disk-0:Virtual_Disk-NVME-Premium:161061273600:GPT:boot+system",
  diskNumber: 0,
  friendlyName: "Virtual_Disk NVME Premium",
  sizeBytes: 150 * GIB,
  partitionStyle: "GPT",
  isSystem: true,
  isBoot: true,
  isOsDisk: true,
};

const g1WindowsDataDisk: SimulatedDisk = {
  identity: "g1-host-disk-1-data",
  fingerprint: "g1:disk-1:Microsoft-NVMe-Direct-Disk-v2:236223201280:GPT:data",
  diskNumber: 1,
  friendlyName: "Microsoft NVMe Direct Disk v2",
  sizeBytes: 220 * GIB,
  partitionStyle: "GPT",
  isSystem: false,
  isBoot: false,
  isOsDisk: false,
};

const g1MacHostDataVolume: SimulatedDevice = {
  identity: "/dev/disk2s5",
  // Composite read-only G1 tuple: Data / APFS / /System/Volumes/Data.
  fingerprint: "g1:/dev/disk2s5:Data:APFS:/System/Volumes/Data",
  isInternalHostDevice: true,
  deviceRole: "host",
  parentIdentity: "/dev/disk2",
  sourceImageIdentity: null,
  sizeBytes: 200 * GIB,
};

export function makeWindowsSafePlan(): WindowsSafetyPlan {
  const root = "C:\\Temp\\zen-g2-s1";
  const sandbox = `${root}\\case-001`;
  const target = `${sandbox}\\bounded.vhdx`;
  const mountPoint = `${sandbox}\\mount`;
  const newDisk: SimulatedDisk = {
    identity: "returned-vhd-disk-guid-001",
    fingerprint: "new-vhdx-image-fingerprint-001",
    diskNumber: 7,
    friendlyName: "Model-only returned VHDX",
    sizeBytes: MAX_NOMINAL_VOLUME_BYTES,
    partitionStyle: "RAW",
    isSystem: false,
    isBoot: false,
    isOsDisk: false,
  };

  return {
    platform: "windows",
    source: { expectedSha256: SOURCE_SHA256, observedSha256: SOURCE_SHA256 },
    paths: {
      root: { lexical: root, canonical: root.replaceAll("\\", "/"), symlinkOrReparsePoint: false },
      sandbox: { lexical: sandbox, canonical: sandbox.replaceAll("\\", "/"), symlinkOrReparsePoint: false },
      target: { lexical: target, canonical: target.replaceAll("\\", "/"), symlinkOrReparsePoint: false },
      mountPoint: { lexical: mountPoint, canonical: mountPoint.replaceAll("\\", "/"), symlinkOrReparsePoint: false },
      targetExistsBefore: false,
      mountPointExists: true,
      mountPointEmpty: true,
      sandboxUnique: true,
      ownerToken: "owned-case-001",
      observedOwnerToken: "owned-case-001",
    },
    before: {
      hostFingerprint: "g1-backed-windows-host",
      availableBytes: TEST_HOST_FREE,
      independentAvailableCapBytes: TEST_HOST_FREE,
    },
    after: {
      hostFingerprint: "g1-backed-windows-host",
      availableBytes: TEST_HOST_FREE,
      independentAvailableCapBytes: TEST_HOST_FREE,
    },
    capacity: {
      nominalVolumeBytes: MAX_NOMINAL_VOLUME_BYTES,
      maximumImageBytes: MAX_IMAGE_BYTES,
      observedImageBytes: MAX_NOMINAL_VOLUME_BYTES,
      maximumHostDeltaBytes: MAX_HOST_DELTA_BYTES,
      observedHostDeltaBytes: MAX_NOMINAL_VOLUME_BYTES,
      allocation: "fixed",
      sparse: false,
      unboundedGrowth: false,
      importsExistingStorage: false,
      targetKind: "sandbox-file",
      backingKnown: true,
    },
    inventoryEvidence: { beforeMutationCaptured: true, afterPlanCaptured: true, crosscheck: "PASS" },
    positiveControl: "PASS",
    selection: "returned-cim-object",
    vhd: {
      exactPath: target,
      returnedObjectIdentity: "returned-vhd-object-001",
      attachedDiskIdentities: [newDisk.identity],
      driveLetterAssigned: false,
    },
    disksBefore: [{ ...g1WindowsSystemDisk }, { ...g1WindowsDataDisk }],
    disksAfter: [{ ...g1WindowsSystemDisk }, { ...g1WindowsDataDisk }, newDisk],
    initializeDiskIdentity: newDisk.identity,
    partition: { identity: "returned-partition-object-001", diskIdentity: newDisk.identity },
    format: { partitionIdentity: "returned-partition-object-001", fileSystem: "NTFS" },
  };
}

export function makeMacSafePlan(): MacSafetyPlan {
  const root = "/tmp/zen-g2-s1";
  const sandbox = `${root}/case-001`;
  const target = `${sandbox}/bounded.dmg`;
  const mountPoint = `${sandbox}/mount`;
  const imageIdentity = "created-udrw-image-001";
  const creationRecordIdentity = "image-create-record-001";
  const imageRoot: SimulatedDevice = {
    identity: "MODEL:IMAGE:ROOT:001",
    fingerprint: "new-udrw-image-root-fingerprint-001",
    isInternalHostDevice: false,
    deviceRole: "image-root",
    parentIdentity: null,
    sourceImageIdentity: imageIdentity,
    sizeBytes: MAX_IMAGE_BYTES,
  };
  const imagePartition: SimulatedDevice = {
    identity: "MODEL:IMAGE:PARTITION:001",
    fingerprint: "new-udrw-image-partition-fingerprint-001",
    isInternalHostDevice: false,
    deviceRole: "partition",
    parentIdentity: imageRoot.identity,
    sourceImageIdentity: imageIdentity,
    sizeBytes: MAX_NOMINAL_VOLUME_BYTES,
  };
  const apfsContainer: SimulatedDevice = {
    identity: "MODEL:IMAGE:APFS-CONTAINER:001",
    fingerprint: "new-udrw-image-apfs-container-fingerprint-001",
    isInternalHostDevice: false,
    deviceRole: "apfs-container",
    parentIdentity: imagePartition.identity,
    sourceImageIdentity: imageIdentity,
    sizeBytes: MAX_NOMINAL_VOLUME_BYTES,
  };
  const apfsVolume: SimulatedDevice = {
    identity: "MODEL:IMAGE:APFS-VOLUME:001",
    fingerprint: "new-udrw-image-apfs-volume-fingerprint-001",
    isInternalHostDevice: false,
    deviceRole: "apfs-volume",
    parentIdentity: apfsContainer.identity,
    sourceImageIdentity: imageIdentity,
    sizeBytes: MAX_NOMINAL_VOLUME_BYTES,
  };
  const hostDisk: SimulatedDevice = {
    identity: "/dev/disk0",
    fingerprint: "g1:/dev/disk0:physical-internal",
    isInternalHostDevice: true,
    deviceRole: "host",
    parentIdentity: null,
    sourceImageIdentity: null,
    sizeBytes: 500 * GIB,
  };
  const hostDataParent: SimulatedDevice = {
    identity: "/dev/disk2",
    fingerprint: "g1:/dev/disk2:physical-internal",
    isInternalHostDevice: true,
    deviceRole: "host",
    parentIdentity: null,
    sourceImageIdentity: null,
    sizeBytes: 500 * GIB,
  };

  return {
    platform: "macos",
    source: { expectedSha256: SOURCE_SHA256, observedSha256: SOURCE_SHA256 },
    paths: {
      root: { lexical: root, canonical: root, symlinkOrReparsePoint: false },
      sandbox: { lexical: sandbox, canonical: sandbox, symlinkOrReparsePoint: false },
      target: { lexical: target, canonical: target, symlinkOrReparsePoint: false },
      mountPoint: { lexical: mountPoint, canonical: mountPoint, symlinkOrReparsePoint: false },
      targetExistsBefore: false,
      mountPointExists: true,
      mountPointEmpty: true,
      sandboxUnique: true,
      ownerToken: "owned-case-001",
      observedOwnerToken: "owned-case-001",
    },
    before: {
      hostFingerprint: "g1-backed-macos-host",
      availableBytes: TEST_HOST_FREE,
      independentAvailableCapBytes: TEST_HOST_FREE,
    },
    after: {
      hostFingerprint: "g1-backed-macos-host",
      availableBytes: TEST_HOST_FREE,
      independentAvailableCapBytes: TEST_HOST_FREE,
    },
    capacity: {
      nominalVolumeBytes: MAX_NOMINAL_VOLUME_BYTES,
      maximumImageBytes: MAX_IMAGE_BYTES,
      observedImageBytes: MAX_NOMINAL_VOLUME_BYTES,
      maximumHostDeltaBytes: MAX_HOST_DELTA_BYTES,
      observedHostDeltaBytes: MAX_NOMINAL_VOLUME_BYTES,
      allocation: "fixed",
      sparse: false,
      unboundedGrowth: false,
      importsExistingStorage: false,
      targetKind: "sandbox-file",
      backingKnown: true,
    },
    inventoryEvidence: { beforeMutationCaptured: true, afterPlanCaptured: true, crosscheck: "PASS" },
    positiveControl: "PASS",
    selection: "structured-attach-result",
    image: { exactPath: target, format: "UDRW", sparse: false, returnedImageIdentity: imageIdentity, creationRecordIdentity },
    devicesBefore: [{ ...hostDisk }, { ...hostDataParent }, { ...g1MacHostDataVolume }],
    devicesAfter: [{ ...hostDisk }, { ...hostDataParent }, { ...g1MacHostDataVolume }, imageRoot, imagePartition, apfsContainer, apfsVolume],
    attachResults: [{
      recordIdentity: "structured-attach-result-001",
      imageIdentity,
      creationRecordIdentity,
      rootDeviceIdentity: imageRoot.identity,
      deviceIdentities: [imageRoot.identity!, imagePartition.identity!, apfsContainer.identity!, apfsVolume.identity!],
      mountedVolumeIdentity: apfsVolume.identity,
      deviceIdentity: apfsVolume.identity,
      sourceImagePath: target,
      mountPoints: [mountPoint],
      fileSystem: "APFS",
      mountPointEmpty: true,
      mounted: true,
    }],
    globalEraseOrPartitionIntent: false,
  };
}

export function makeCleanupState(): CleanupSession {
  const sandboxPath = "/tmp/zen-g2-s1/case-001";
  const start: CleanupStart = {
    platform: "macos",
    sandboxPath,
    imagePath: `${sandboxPath}/bounded.dmg`,
    mountPoint: `${sandboxPath}/mount`,
    deviceIdentity: "/dev/disk9s1",
    imageIdentity: "created-udrw-image-001",
    ownerToken: "owned-case-001",
    observedOwnerToken: "owned-case-001",
    imageIsOwned: true,
    imageIsSymlinkOrReparsePoint: false,
    mountPointIsSymlinkOrReparsePoint: false,
    originalFailure: "simulated verification failure",
  };
  return createCleanupSession(start);
}

export function makeWindowsCleanupState(): CleanupSession {
  const sandboxPath = "C:\\Temp\\zen-g2-s1\\case-001";
  const start: CleanupStart = {
    platform: "windows",
    sandboxPath,
    imagePath: `${sandboxPath}\\bounded.vhdx`,
    mountPoint: `${sandboxPath}\\no-drive-letter-target`,
    deviceIdentity: "returned-vhd-object-001",
    imageIdentity: "created-vhdx-image-001",
    ownerToken: "owned-case-001",
    observedOwnerToken: "owned-case-001",
    imageIsOwned: true,
    imageIsSymlinkOrReparsePoint: false,
    mountPointIsSymlinkOrReparsePoint: false,
    originalFailure: "simulated verification failure",
  };
  return createCleanupSession(start);
}
