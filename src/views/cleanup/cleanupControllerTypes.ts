import type { TauriApi } from "../../api/tauriApi";

export type CleanupApi = Partial<Pick<
  TauriApi,
  | "listAnalysisDetectors"
  | "startAnalysisRun"
  | "cancelAnalysisRun"
  | "retryAnalysisRun"
  | "getAnalysisRun"
  | "getActiveAnalysisRun"
  | "listAnalysisRuns"
  | "listAnalysisRunDetectors"
  | "listAnalysisFindings"
  | "getAnalysisFinding"
  | "listAnalysisFindingEvidence"
  | "setAnalysisFindingDecision"
  | "revalidateAnalysisFinding"
  | "previewCleanupOperations"
  | "moveCleanupCandidatesToSafeTrash"
  | "revealInFolder"
  | "getAIFeatureReadiness"
  | "getAISettings"
  | "analyzeCleanupCandidatesWithAI"
  | "onAnalysisRunUpdated"
  | "onAnalysisFindingsPublished"
  | "onAnalysisDetectorUpdated"
>>;

export type CleanupMutationKind =
  | "select"
  | "scan"
  | "cancel"
  | "retry"
  | "acknowledge"
  | "revalidate"
  | "preview"
  | "safe_trash";

export type CleanupMutationOwner = {
  id: number;
  kind: CleanupMutationKind;
  scopeEpoch: number;
  runId: string | null;
};
