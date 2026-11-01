// TypeScript declarations for the native resource baseline collection CLI.
// Keep the evidence classes explicit; CALIBRATION_ONLY is not a release pass.
export declare const RESOURCE_TEST_NAME: string;

export interface NativeResourceEvidence {
  platform: string;
  epochs: number;
  resourceTrendClassification: "HARD PASS" | "BLOCKED";
  registryClassification: string;
  hardSignalAvailable: boolean;
  hardResourceGrowth: boolean;
  rssBytes: (number | null)[];
  privateCommittedBytes: (number | null)[];
  handles: (number | null)[];
  fileDescriptors: (number | null)[];
  rssNetBytes: number | null;
  privateCommittedNetBytes: number | null;
  handleNet: number | null;
  fdNet: number | null;
  afterDisposeRegistryZero: boolean;
  previewCycles: number;
  thumbnailCycles: number;
  targetSwitchCycles: number;
}

export interface CalibrationRun {
  run?: number;
  elapsedMs?: number;
  exitCode: number | null;
  signal: string | null;
  error: string | null;
  evidence: Pick<NativeResourceEvidence, "platform" | "resourceTrendClassification"> | null;
  stderrTail?: string;
}

export interface CalibrationSummary {
  requestedRuns: number;
  validEvidenceRuns: number;
  nativeHardPassRuns: number;
  nativeBlockedRuns: number;
  processFailureRuns: number;
  platforms: string[];
  fullyCollected: boolean;
  allNativeHardPass: boolean;
  interpretation: "CALIBRATION_ONLY_NOT_PRODUCT_QUALIFICATION";
}

export declare function extractResourceEvidence(output: string): NativeResourceEvidence;
export declare function summarizeCalibration(runs: CalibrationRun[]): CalibrationSummary;
export declare function collectResourceCalibration(options: {
  binary: string;
  runs?: number;
  output?: string;
  sourceSha?: string | null;
  spawnImpl?: (...args: any[]) => any;
}): Promise<{
  schema: number;
  purpose: string;
  binary: { sha256: string; bytes: number; sourceSha: string | null };
  summary: CalibrationSummary;
  runs: CalibrationRun[];
}>;
