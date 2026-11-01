export interface ScannedEntry {
  path: string;
  name: string;
  extension: string;
  size: number;
  mtime: number;
  isDir: boolean;
  stateCode: number;
}

export interface ScanProgressPayload {
  jobId: string;
  jobKind: "foreground" | "background";
  root: string;
  scanned: number;
  files: number;
  directories: number;
  skipped: number;
  errors: number;
  elapsedMs: number;
}

export interface ScanBatchPayload {
  jobId: string;
  jobKind: "foreground" | "background";
  root: string;
  batchIndex: number;
  entries: ScannedEntry[];
  progress: ScanProgressPayload;
}

export type ScanSummary = ScanProgressPayload;

export interface ManagedScanRequest {
  roots: string[];
  requestKey?: string | null;
  dedupe: boolean;
}

export interface ScanRootDto {
  id: string;
  normalizedPath: string;
  displayName: string;
  sourceKind: string;
  enabled: boolean;
  healthStatus: string;
  currentGeneration: number;
  activeRunId: string | null;
  activeGeneration: number | null;
  revision: number;
  lastSuccessfulGeneration: number | null;
  lastFullScanAt: number | null;
  needsReconciliation: boolean;
  lastErrorCode: string | null;
  lastErrorMessage: string | null;
  watcherRevision: number;
  watcherAppliedRevision: number;
  watcherLastEventAt: number | null;
  watcherLastAppliedAt: number | null;
  watcherLastErrorCode: string | null;
  watcherLastErrorMessage: string | null;
  watcherRuleRecoveryRequired?: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface ScanSessionRootDto {
  sessionId: string;
  requestedIndex: number;
  requestedPath: string;
  normalizedRequestedPath: string;
  resolution: string;
  effectiveRootId: string | null;
  effectivePath: string | null;
  effectiveIndex: number | null;
  runId: string | null;
  status: string;
  reason: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface ScanRunDto {
  id: string;
  scanRootId: string;
  rootPath: string;
  generation: number;
  parentSessionId: string | null;
  status: string;
  phase: string;
  scannedFiles: number;
  scannedDirectories: number;
  processedBytes: number;
  warningsCount: number;
  errorsCount: number;
  metadataErrorCount: number;
  coverageErrorCount: number;
  coverageComplete: boolean;
  staleReconciliationAllowed: boolean;
  cancelRequested: boolean;
  revision: number;
  sessionRevision: number;
  startedAt: number | null;
  finishedAt: number | null;
  lastCheckpointAt: number | null;
  errorCode: string | null;
  errorMessage: string | null;
  resultJson: string | null;
  watcherRevisionAtStart: number;
  createdAt: number;
  updatedAt: number;
}

export interface ScanSessionDto {
  id: string;
  requestKey: string | null;
  canonicalRequestHash: string | null;
  status: string;
  phase: string;
  cancelRequested: boolean;
  requestedRootCount: number;
  effectiveRootCount: number;
  completedRootCount: number;
  failedRootCount: number;
  cancelledRootCount: number;
  coveredRootCount: number;
  unstartedRootCount: number;
  dedupeRequested: boolean;
  dedupeDispatchState: string;
  dedupeAttemptCount: number;
  dedupeJobId: string | null;
  dedupeLastError: string | null;
  scannedFiles: number;
  scannedDirectories: number;
  warningsCount: number;
  errorsCount: number;
  revision: number;
  startedAt: number | null;
  finishedAt: number | null;
  lastCheckpointAt: number | null;
  errorCode: string | null;
  errorMessage: string | null;
  resultJson: string | null;
  createdAt: number;
  updatedAt: number;
  roots: ScanSessionRootDto[];
}

export interface ManagedScanStartDto {
  session: ScanSessionDto;
  runs: ScanRunDto[];
}

export interface ManagedScanSnapshotDto {
  session: ScanSessionDto;
  runs: ScanRunDto[];
}

export interface ManagedScanEvent {
  eventId: string;
  runId: string;
  scanRootId: string;
  parentSessionId: string | null;
  generation: number;
  runRevision: number;
  sessionRevision: number;
  status: string;
  runPhase: string;
  sessionPhase: string;
  scannedFiles: number;
  scannedDirectories: number;
  processedBytes: number;
  warningsCount: number;
  errorsCount: number;
  currentPath: string | null;
  errorCode: string | null;
  errorMessage: string | null;
  timestamp: number;
}

export interface WatcherReconciliationStatus {
  scanRootId: string;
  path: string;
  rootRevision: number;
  watcherRevision: number;
  watcherAppliedRevision: number;
  pending: boolean;
  needsReconciliation: boolean;
  healthStatus: string;
  activeRunId: string | null;
  lastEventAt: number | null;
  lastAppliedAt: number | null;
  lastErrorCode: string | null;
  lastErrorMessage: string | null;
  pendingBatch?: number;
  timestamp: number;
}

export interface DedupeProgressPayload {
  dedupeJobId: string;
  parentScanJobId: string | null;
  processed: number;
  total: number;
  status: string;
  phase?: string;
  processedBytes?: number;
  totalBytes?: number;
  revision?: number;
  warningCount?: number;
  errorCount?: number;
}

export interface DedupeCompletePayload {
  dedupeJobId: string;
  parentScanJobId: string | null;
  status: "completed" | "completed_with_warnings" | "cancelled" | "failed" | "interrupted";
  success: boolean;
  error: string | null;
  phase?: string;
  revision?: number;
  processedBytes?: number;
  totalBytes?: number;
  warningCount?: number;
  errorCode?: string | null;
}

export interface OperationProgressPayload {
  kind: "execute" | "restore" | "materialize";
  batchId: string;
  processed: number;
  total: number;
  currentPath: string;
  phase?: string;
  processedItems?: number;
  totalItems?: number;
  processedBytes?: number;
  totalBytes?: number;
}

export interface GlobalHotkeyErrorPayload {
  message: string;
}

export interface GlobalHotkeyStatus {
  requestedAccelerator: string;
  effectiveAccelerator: string | null;
  registered: boolean;
  error: string | null;
  revision: number;
}

export type SearchWindowPhase = "hidden" | "showing" | "visible_collapsed" | "visible_expanded" | "hiding";

export interface SearchWindowSnapshot {
  sessionId: number;
  revision: number;
  phase: SearchWindowPhase;
}

export interface MainWindowReadyRequest {
  nonce: number;
  generation: number;
  sessionId: number | null;
  revision: number | null;
}

export interface TauriSearchFileResult {
  id: string;
  path: string;
  name: string;
  extension: string;
  size: number;
  mtime: number;
  isDir: boolean;
  stateCode: number;
  rank: number;
}
