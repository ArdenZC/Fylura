import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { desktopDir, documentDir, downloadDir, tempDir } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, History, LoaderCircle, RefreshCw, Search, Sparkles, Trash2, XCircle } from "lucide-react";
import { tauriApi } from "../../api/tauriApi";
import { useI18nContext, useNavigationContext } from "../../contexts/AppContexts";
import type {
  AnalysisDetector,
  AnalysisDetectorDescriptor,
  AnalysisFinding,
  AnalysisFindingEvidence,
  AnalysisFindingPage,
  AnalysisRun,
  AIProductFeatureReadiness,
  CleanupExecutionResult,
  OperationPreview,
  OperationPreviewResult,
  StartAnalysisRunRequest
} from "../../types/domain";
import type { Translator, View } from "../../types/ui";
import { formatBytes } from "../../utils/format";
import { useFileMutationUnavailableCode } from "../../utils/fileMutationCapability";
import { isBrowserMockEnabled } from "../../utils/runtimeMode";
import { localizedStableError, readableError, compactPath } from "../../utils/viewHelpers";
import { cn } from "../../utils/tw";
import {
  Button,
  ConfirmDialog,
  DurableTaskStatus,
  MetricStrip,
  NoticeBanner,
  SegmentedControl,
  SideSheet,
  StateBlock,
  contentPanel,
  metadataText,
  pageSurface,
  quietText,
  sectionDescription,
  sectionHeading,
  WorkflowSteps,
  type WorkflowStepState
} from "../shared/ui";
import {
  AI_RECHECK_BATCH_SIZE,
  FINDING_PAGE_SIZE,
  FINDING_ROW_HEIGHT,
  durableRunState,
  isAnalysisFinding,
  isCleanupPreviewExecutable,
  isCleanupPreviewScopeExecutable,
  isFindingSelectable,
  isRunInProgress,
  cleanupSelectionFingerprint,
  normalizeScopePaths,
  scopeKey,
  scopePaths,
  type CleanupTier
} from "./cleanupModel";
export { cleanupSelectionFingerprint, reconcileAuthoritativeFindingUpdates } from "./cleanupModel";
import { FindingRow, tierLabel } from "./FindingRow";
import type { CleanupApi, CleanupMutationKind, CleanupMutationOwner } from "./cleanupControllerTypes";
import { useCleanupAnalysisController } from "./useCleanupAnalysisController";
import { useCleanupExecutionController } from "./useCleanupExecutionController";
import { useCleanupSelectionController } from "./useCleanupSelectionController";
import { AIReadinessNotice, cleanupReadinessText } from "../shared/AIReadinessNotice";

const FINDING_VIRTUALIZATION_THRESHOLD = 20;

type Props = {
  initialRoots?: string[];
  api?: CleanupApi;
  t?: Translator;
  onError?: (message: string) => void;
  onNavigate?: (view: View) => void;
};

function browserPresentationCleanupPath(kind: "downloads" | "desktop" | "documents" | "temp"): string | null {
  if (!isBrowserMockEnabled()) return null;
  const fixture = new URLSearchParams(globalThis.location?.search ?? "").get("pm01-cleanup");
  if (!fixture || !["analysis-failure", "current-assessment", "request-unavailable", "preview", "visibility", "visibility-many"].includes(fixture)) return null;
  // Explicit PM-01 browser fixtures use synthetic paths; native scope selection remains OS-owned.
  return {
    downloads: "C:/Users/Zen/Downloads",
    desktop: "C:/Users/Zen/Desktop",
    documents: "C:/Users/Zen/Documents",
    temp: "C:/Users/Zen/AppData/Local/Temp"
  }[kind];
}

type AiWorkState = "idle" | "running" | "canceling";
type AiOperation = {
  id: number;
  epoch: number;
  scopeEpoch: number;
  tierEpoch: number;
  tier: CleanupTier;
  runId: string;
  action: "review" | "selected-safe";
  cancelRequested: boolean;
};

export function StorageCleanupView(props: Props = {}) {
  if (props.t) return <StorageCleanupPanel {...props} t={props.t} />;
  return <StorageCleanupViewWithContext {...props} />;
}

function StorageCleanupViewWithContext(props: Omit<Props, "t" | "onError" | "onNavigate">) {
  const { t } = useI18nContext();
  const { onError, setView } = useNavigationContext();
  return <StorageCleanupPanel {...props} t={t} onError={onError} onNavigate={setView} />;
}

function StorageCleanupPanel({
  initialRoots,
  api = tauriApi,
  t,
  onError,
  onNavigate
}: Props & { t: Translator }) {
  const [selectedRoots, setSelectedRoots] = useState<string[]>(() => normalizeScopePaths(initialRoots ?? []));
  const [detectors, setDetectors] = useState<AnalysisDetectorDescriptor[]>([]);
  const [, setRuns] = useState<AnalysisRun[]>([]);
  const [run, setRun] = useState<AnalysisRun | null>(null);
  const [runDetectors, setRunDetectors] = useState<AnalysisDetector[]>([]);
  const [findings, setFindings] = useState<AnalysisFinding[]>([]);
  const [findingCache, setFindingCache] = useState<Record<string, AnalysisFinding>>({});
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [activeTier, setActiveTier] = useState<CleanupTier>("safe");
  const [evidenceByFinding, setEvidenceByFinding] = useState<Record<string, AnalysisFindingEvidence[]>>({});
  const [expandedEvidence, setExpandedEvidence] = useState<Set<string>>(() => new Set());
  const [reviewFinding, setReviewFinding] = useState<AnalysisFinding | null>(null);
  const [preview, setPreview] = useState<OperationPreviewResult | null>(null);
  const [confirmPreviewOpen, setConfirmPreviewOpen] = useState(false);
  const [executionResult, setExecutionResult] = useState<CleanupExecutionResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadingFindings, setLoadingFindings] = useState(false);
  const [isMutating, setIsMutating] = useState(false);
  const [mutationKind, setMutationKind] = useState<CleanupMutationKind | null>(null);
  const [failedMutationKind, setFailedMutationKind] = useState<CleanupMutationKind | null>(null);
  const [aiWorkState, setAiWorkState] = useState<AiWorkState>("idle");
  const isAiWorking = aiWorkState !== "idle";
  const [aiReadiness, setAiReadiness] = useState<AIProductFeatureReadiness | null>(null);
  const [aiReadinessLoading, setAiReadinessLoading] = useState(false);
  const [aiReadinessError, setAiReadinessError] = useState(false);
  const [isCheckingAIReadiness, setIsCheckingAIReadiness] = useState(false);
  const [aiStatus, setAiStatus] = useState("");
  const [error, setError] = useState("");
  const [unsupported, setUnsupported] = useState(false);
  const findingListRef = useRef<HTMLDivElement | null>(null);
  const findingsEpoch = useRef(0);
  const scopeEpoch = useRef(0);
  const activeTierRef = useRef<CleanupTier>(activeTier);
  const activeTierEpoch = useRef(0);
  const aiOperationEpoch = useRef(0);
  const previewRequestEpoch = useRef(0);
  const previewSelectionFingerprint = useRef<string | null>(null);
  const runRef = useRef<AnalysisRun | null>(null);
  const requestKeyRef = useRef<string | null>(null);
  const scanIntentInFlight = useRef(false);
  const mutationOwnerRef = useRef<CleanupMutationOwner | null>(null);
  const mutationKindRef = useRef<CleanupMutationKind | null>(null);
  const mutationSequenceRef = useRef(0);
  const aiWorkStateRef = useRef<AiWorkState>("idle");
  const aiOperationRef = useRef<AiOperation | null>(null);
  const aiOperationSequenceRef = useRef(0);
  const aiReadinessRequestEpoch = useRef(0);
  const aiReadinessCheckInFlight = useRef(false);
  const interactionLockedRef = useRef(false);
  const scopeHydrated = useRef(Boolean(normalizeScopePaths(initialRoots ?? []).length));
  const initialRootsPropKey = useRef(scopeKey(initialRoots ?? []));
  const defaultSelectionRuns = useRef(new Set<string>());
  const mutationUnavailable = useFileMutationUnavailableCode();

  useEffect(() => {
    runRef.current = run;
  }, [run]);

  interactionLockedRef.current = Boolean(mutationOwnerRef.current) || aiWorkState !== "idle";

  const updateAiWorkState = useCallback((nextState: AiWorkState) => {
    aiWorkStateRef.current = nextState;
    interactionLockedRef.current = Boolean(mutationOwnerRef.current) || nextState !== "idle";
    setAiWorkState(nextState);
  }, []);

  const beginMutation = useCallback((kind: CleanupMutationKind, runId: string | null = runRef.current?.id ?? null) => {
    if (mutationOwnerRef.current || aiWorkStateRef.current !== "idle") return null;
    const owner: CleanupMutationOwner = {
      id: ++mutationSequenceRef.current,
      kind,
      scopeEpoch: scopeEpoch.current,
      runId
    };
    mutationOwnerRef.current = owner;
    mutationKindRef.current = kind;
    interactionLockedRef.current = true;
    setIsMutating(true);
    setMutationKind(kind);
    setFailedMutationKind(null);
    return owner;
  }, []);

  const releaseMutation = useCallback((owner: CleanupMutationOwner) => {
    if (mutationOwnerRef.current?.id !== owner.id) return false;
    mutationOwnerRef.current = null;
    mutationKindRef.current = null;
    interactionLockedRef.current = aiWorkStateRef.current !== "idle";
    setIsMutating(false);
    setMutationKind(null);
    return true;
  }, []);

  useEffect(() => () => {
    if (aiOperationRef.current) aiOperationRef.current.cancelRequested = true;
    aiOperationRef.current = null;
    aiOperationEpoch.current += 1;
    mutationOwnerRef.current = null;
    mutationKindRef.current = null;
    setMutationKind(null);
    scanIntentInFlight.current = false;
    requestKeyRef.current = null;
    previewRequestEpoch.current += 1;
    previewSelectionFingerprint.current = null;
    findingsEpoch.current += 1;
  }, []);

  const invalidatePreviewState = useCallback(() => {
    previewRequestEpoch.current += 1;
    setPreview(null);
    setConfirmPreviewOpen(false);
    setExecutionResult(null);
    setReviewFinding(null);
  }, []);

  const {
    selectedFindingIds,
    selectionRevision,
    selectedFindings,
    selectedBytes,
    runReclaimable,
    commitSelection,
    resetSelection,
    selectBackendDefaultSafeFindings,
    buildSelections,
    reconcileUpdatedFindings,
    removeSelectionsForIds
  } = useCleanupSelectionController({ findingCache, setFindingCache, run, invalidatePreviewState });

  const reportError = useCallback((value: unknown) => {
    const raw = readableError(value);
    const message = localizedStableError(raw, t);
    setError(message);
    setFailedMutationKind(mutationKindRef.current);
    onError?.(message);
  }, [onError, t]);

  const replaceCopy = useCallback((key: Parameters<Translator>[0], replacements: Record<string, string | number> = {}) => {
    return Object.entries(replacements).reduce(
      (text, [name, value]) => text.replaceAll(`{${name}}`, String(value)),
      t(key)
    );
  }, [t]);

  const refreshAIReadiness = useCallback(async () => {
    const epoch = ++aiReadinessRequestEpoch.current;
    setAiReadinessLoading(true);
    setAiReadinessError(false);
    try {
      if (!api.getAIFeatureReadiness) throw new Error("cleanup_ai_readiness_unavailable");
      const next = await api.getAIFeatureReadiness();
      if (epoch !== aiReadinessRequestEpoch.current) return null;
      setAiReadiness(next);
      return next;
    } catch {
      if (epoch !== aiReadinessRequestEpoch.current) return null;
      setAiReadiness(null);
      setAiReadinessError(true);
      return null;
    } finally {
      if (epoch === aiReadinessRequestEpoch.current) setAiReadinessLoading(false);
    }
  }, [api]);

  useEffect(() => {
    void refreshAIReadiness().catch(() => undefined);
    return () => { aiReadinessRequestEpoch.current += 1; };
  }, [refreshAIReadiness]);

  const resetReviewStateForScopeChange = useCallback(() => {
    scopeEpoch.current += 1;
    findingsEpoch.current += 1;
    activeTierEpoch.current += 1;
    aiOperationEpoch.current += 1;
    previewRequestEpoch.current += 1;
    requestKeyRef.current = null;
    scanIntentInFlight.current = false;
    defaultSelectionRuns.current.clear();
    if (aiOperationRef.current) aiOperationRef.current.cancelRequested = true;
    aiOperationRef.current = null;
    mutationOwnerRef.current = null;
    mutationKindRef.current = null;
    setRun(null);
    setRunDetectors([]);
    setFindings([]);
    setFindingCache({});
    setNextCursor(null);
    resetSelection();
    setEvidenceByFinding({});
    setExpandedEvidence(new Set());
    setReviewFinding(null);
    setPreview(null);
    setConfirmPreviewOpen(false);
    setExecutionResult(null);
    setAiStatus("");
    setError("");
    setMutationKind(null);
    setFailedMutationKind(null);
    setLoadingFindings(false);
    setIsMutating(false);
    updateAiWorkState("idle");
  }, [resetSelection, updateAiWorkState]);

  const applyScopeSelection = useCallback((roots: string[]) => {
    if (interactionLockedRef.current) return;
    const nextRoots = normalizeScopePaths(roots);
    if (scopeKey(selectedRoots) !== scopeKey(nextRoots)) resetReviewStateForScopeChange();
    scopeHydrated.current = true;
    setSelectedRoots(nextRoots);
    setError("");
  }, [resetReviewStateForScopeChange, selectedRoots]);

  useEffect(() => {
    if (interactionLockedRef.current) return;
    const nextKey = scopeKey(initialRoots ?? []);
    if (nextKey === initialRootsPropKey.current) return;
    initialRootsPropKey.current = nextKey;
    applyScopeSelection(initialRoots ?? []);
  }, [applyScopeSelection, aiWorkState, initialRoots, isMutating]);

  const { loadFindings, loadRunDetails } = useCleanupAnalysisController({
    api,
    initialRoots,
    run,
    activeTier,
    activeTierRef,
    activeTierEpoch,
    findingsEpoch,
    scopeEpoch,
    runRef,
    scopeHydrated,
    defaultSelectionRuns,
    setDetectors,
    setRuns,
    setRun,
    setRunDetectors,
    setFindings,
    setFindingCache,
    setNextCursor,
    setSelectedRoots,
    setLoading,
    setLoadingFindings,
    setUnsupported,
    selectBackendDefaultSafeFindings,
    reportError
  });

  const { previewSelected, moveSelectedToSafeTrash } = useCleanupExecutionController({
    api,
    run,
    runRef,
    selectedFindings,
    selectionRevision,
    buildSelections,
    resetSelection,
    preview,
    setPreview,
    setConfirmPreviewOpen,
    setExecutionResult,
    setError,
    scopeEpoch,
    aiOperationEpoch,
    previewRequestEpoch,
    previewSelectionFingerprint,
    mutationOwnerRef,
    interactionLockedRef,
    mutationUnavailable,
    beginMutation,
    releaseMutation,
    invalidatePreviewState,
    loadRunDetails,
    reportError,
    t
  });

  const chooseScope = useCallback(async () => {
    if (interactionLockedRef.current) return;
    try {
      const selected = await open({ directory: true, multiple: false, title: t("storageCleanupChooseScope") });
      if (typeof selected === "string" && selected.trim()) {
        applyScopeSelection([selected]);
      }
    } catch (scopeError) {
      reportError(scopeError);
    }
  }, [applyScopeSelection, reportError, t]);

  const chooseQuickScope = useCallback(async (kind: "downloads" | "desktop" | "documents" | "temp") => {
    if (interactionLockedRef.current) return;
    try {
      const presentationPath = browserPresentationCleanupPath(kind);
      const path = presentationPath ?? (kind === "downloads"
        ? await downloadDir()
        : kind === "desktop"
          ? await desktopDir()
          : kind === "documents"
            ? await documentDir()
            : await tempDir());
      applyScopeSelection([path]);
    } catch (scopeError) {
      reportError(scopeError);
    }
  }, [applyScopeSelection, reportError]);

  const startScan = useCallback(async () => {
    if (scanIntentInFlight.current || interactionLockedRef.current) return;
    if (!selectedRoots.length) {
      setError(t("storageCleanupScopeRequired"));
      return;
    }
    if (!api.startAnalysisRun) {
      setUnsupported(true);
      return;
    }
    setError("");
    setExecutionResult(null);
    previewRequestEpoch.current += 1;
    setPreview(null);
    previewSelectionFingerprint.current = null;
    setConfirmPreviewOpen(false);
    resetSelection();
    setFindingCache({});
    defaultSelectionRuns.current.clear();
    scanIntentInFlight.current = true;
    const requestedScopeEpoch = scopeEpoch.current;
    const requestKey = `cleanup-${crypto.randomUUID()}`;
    requestKeyRef.current = requestKey;
    const request: StartAnalysisRunRequest = {
      scope: { kind: "approvedCleanupPaths", paths: selectedRoots },
      detectorIds: detectors.filter((detector) => detector.supportsApprovedPaths).map((detector) => detector.detectorId),
      requestKey
    };
    const mutationOwner = beginMutation("scan", null);
    if (!mutationOwner) {
      requestKeyRef.current = null;
      scanIntentInFlight.current = false;
      return;
    }
    const ownsScanIntent = () => requestedScopeEpoch === scopeEpoch.current
      && requestKeyRef.current === requestKey
      && scanIntentInFlight.current
      && mutationOwnerRef.current?.id === mutationOwner.id;
    try {
      const started = await api.startAnalysisRun(request);
      if (!ownsScanIntent()) return;
      setRun(started);
      // The start mutation is settled once the backend has created the run. Keep
      // the subsequent authoritative read separate so the user can cancel a
      // running scan without an old scan finally clearing its lock.
      releaseMutation(mutationOwner);
      await loadRunDetails(started.id, true, requestedScopeEpoch);
    } catch (startError) {
      if (ownsScanIntent()) reportError(startError);
    } finally {
      if (requestKeyRef.current === requestKey) {
        requestKeyRef.current = null;
        scanIntentInFlight.current = false;
        releaseMutation(mutationOwner);
      }
    }
  }, [api, beginMutation, detectors, loadRunDetails, reportError, releaseMutation, resetSelection, selectedRoots, t]);

  const cancelRun = useCallback(async () => {
    if (!run || !api.cancelAnalysisRun) return;
    if (interactionLockedRef.current) return;
    const expectedScopeEpoch = scopeEpoch.current;
    const runId = run.id;
    const mutationOwner = beginMutation("cancel", runId);
    if (!mutationOwner) return;
    try {
      await api.cancelAnalysisRun(runId);
      if (expectedScopeEpoch !== scopeEpoch.current || mutationOwnerRef.current?.id !== mutationOwner.id) return;
      await loadRunDetails(runId, false, expectedScopeEpoch);
    } catch (cancelError) {
      if (expectedScopeEpoch === scopeEpoch.current && mutationOwnerRef.current?.id === mutationOwner.id) reportError(cancelError);
    } finally {
      releaseMutation(mutationOwner);
    }
  }, [api, beginMutation, loadRunDetails, releaseMutation, reportError, run]);

  const retryRun = useCallback(async () => {
    if (!run || !api.retryAnalysisRun) return;
    if (interactionLockedRef.current) return;
    const expectedScopeEpoch = scopeEpoch.current;
    const runId = run.id;
    const mutationOwner = beginMutation("retry", runId);
    if (!mutationOwner) return;
    setError("");
    resetSelection();
    previewRequestEpoch.current += 1;
    setPreview(null);
    previewSelectionFingerprint.current = null;
    setConfirmPreviewOpen(false);
    defaultSelectionRuns.current.delete(runId);
    try {
      const retried = await api.retryAnalysisRun(runId);
      if (expectedScopeEpoch !== scopeEpoch.current || mutationOwnerRef.current?.id !== mutationOwner.id) return;
      setRun(retried);
      await loadRunDetails(retried.id, true, expectedScopeEpoch);
    } catch (retryError) {
      if (expectedScopeEpoch === scopeEpoch.current && mutationOwnerRef.current?.id === mutationOwner.id) reportError(retryError);
    } finally {
      releaseMutation(mutationOwner);
    }
  }, [api, beginMutation, loadRunDetails, releaseMutation, reportError, resetSelection, run]);

  const revealFinding = useCallback(async (finding: AnalysisFinding) => {
    if (!finding.pathSnapshot || !api.revealInFolder) return;
    try {
      await api.revealInFolder(finding.pathSnapshot);
    } catch (revealError) {
      reportError(revealError);
    }
  }, [api, reportError]);

  const toggleEvidence = useCallback(async (finding: AnalysisFinding) => {
    const expectedScopeEpoch = scopeEpoch.current;
    if (expandedEvidence.has(finding.id)) {
      setExpandedEvidence((current) => {
        const next = new Set(current);
        next.delete(finding.id);
        return next;
      });
      return;
    }
    if (!evidenceByFinding[finding.id] && api.listAnalysisFindingEvidence) {
      try {
        const evidence = await api.listAnalysisFindingEvidence(finding.id);
        if (expectedScopeEpoch !== scopeEpoch.current) return;
        setEvidenceByFinding((current) => ({ ...current, [finding.id]: evidence }));
      } catch (evidenceError) {
        if (expectedScopeEpoch === scopeEpoch.current) reportError(evidenceError);
        return;
      }
    }
    if (expectedScopeEpoch !== scopeEpoch.current) return;
    setExpandedEvidence((current) => {
      const next = new Set(current);
      next.add(finding.id);
      return next;
    });
  }, [api, evidenceByFinding, expandedEvidence, reportError]);

  const acknowledgeReviewFinding = useCallback(async () => {
    if (!reviewFinding || !api.getAnalysisFinding || !api.setAnalysisFindingDecision) return;
    if (interactionLockedRef.current) return;
    const expectedScopeEpoch = scopeEpoch.current;
    const expectedTierEpoch = activeTierEpoch.current;
    const expectedTier = activeTierRef.current;
    const findingId = reviewFinding.id;
    const expectedRunId = runRef.current?.id;
    const mutationOwner = beginMutation("acknowledge", expectedRunId ?? null);
    if (!mutationOwner) return;
    const ownsRequest = () => expectedScopeEpoch === scopeEpoch.current
      && expectedTierEpoch === activeTierEpoch.current
      && activeTierRef.current === expectedTier
      && runRef.current?.id === expectedRunId
      && mutationOwnerRef.current?.id === mutationOwner.id;
    try {
      const current = await api.getAnalysisFinding(findingId);
      if (!ownsRequest()) return;
      if (!current || current.tier !== "review" || current.status !== "active") throw new Error("cleanup_finding_changed");
      const decision = await api.setAnalysisFindingDecision({
        findingKey: current.findingKey,
        decision: "acknowledged",
        expectedRevision: current.decisionRevision ?? 0
      });
      if (decision.decision !== "acknowledged") throw new Error("cleanup_review_not_acknowledged");
      const refreshed = await api.getAnalysisFinding(current.id);
      if (!ownsRequest()) return;
      if (!refreshed || !isFindingSelectable(refreshed)) throw new Error("cleanup_review_not_executable");
      setFindingCache((cache) => ({ ...cache, [refreshed.id]: refreshed }));
      commitSelection((selected) => new Set(selected).add(refreshed.id));
      setReviewFinding(null);
      const currentRun = runRef.current;
      if (currentRun && ownsRequest()) await loadFindings(currentRun.id, activeTierRef.current, null, false, currentRun.revision);
    } catch (reviewError) {
      if (ownsRequest()) reportError(reviewError);
    } finally {
      releaseMutation(mutationOwner);
    }
  }, [api, beginMutation, commitSelection, loadFindings, releaseMutation, reportError, reviewFinding]);

  const revalidateFinding = useCallback(async (finding: AnalysisFinding) => {
    if (!api.revalidateAnalysisFinding) return;
    if (interactionLockedRef.current) return;
    const expectedScopeEpoch = scopeEpoch.current;
    const expectedTierEpoch = activeTierEpoch.current;
    const expectedTier = activeTierRef.current;
    const expectedRunId = runRef.current?.id;
    const mutationOwner = beginMutation("revalidate", expectedRunId ?? null);
    if (!mutationOwner) return;
    const ownsRequest = () => expectedScopeEpoch === scopeEpoch.current
      && expectedTierEpoch === activeTierEpoch.current
      && activeTierRef.current === expectedTier
      && runRef.current?.id === expectedRunId
      && mutationOwnerRef.current?.id === mutationOwner.id;
    try {
      const refreshed = await api.revalidateAnalysisFinding(finding.id);
      if (!ownsRequest()) return;
      setFindingCache((cache) => ({ ...cache, [refreshed.id]: refreshed }));
      commitSelection((selected) => {
        const next = new Set(selected);
        if (!isFindingSelectable(refreshed)) next.delete(refreshed.id);
        return next;
      });
      const currentRun = runRef.current;
      if (currentRun && ownsRequest()) await loadFindings(currentRun.id, activeTierRef.current, null, false, currentRun.revision);
    } catch (revalidateError) {
      if (ownsRequest()) reportError(revalidateError);
    } finally {
      releaseMutation(mutationOwner);
    }
  }, [api, beginMutation, commitSelection, loadFindings, releaseMutation, reportError]);

  const toggleFinding = useCallback((finding: AnalysisFinding) => {
    if (interactionLockedRef.current) return;
    if (finding.tier === "caution" || finding.status !== "active") return;
    if (!isFindingSelectable(finding)) {
      if (finding.tier === "review" && finding.decision !== "acknowledged") setReviewFinding(finding);
      return;
    }
    commitSelection((current) => {
      const next = new Set(current);
      if (next.has(finding.id)) next.delete(finding.id);
      else next.add(finding.id);
      return next;
    });
  }, [commitSelection]);

  const invalidateReadbackFailures = useCallback((findingIds: readonly string[]) => {
    if (!findingIds.length) return;
    const failed = new Set(findingIds);
    setFindingCache((cache) => {
      const next = { ...cache };
      for (const id of failed) delete next[id];
      return next;
    });
    removeSelectionsForIds(findingIds);
  }, [removeSelectionsForIds]);

  const changeActiveTier = useCallback((nextTier: CleanupTier) => {
    if (interactionLockedRef.current) return;
    if (nextTier === activeTierRef.current) return;
    activeTierRef.current = nextTier;
    activeTierEpoch.current += 1;
    findingsEpoch.current += 1;
    invalidatePreviewState();
    setReviewFinding(null);
    setActiveTier(nextTier);
  }, [invalidatePreviewState]);

  const recheckReviewFindings = useCallback(async (selectedSafeFindingIds?: readonly string[]) => {
    const isSelectedSafeAssessment = selectedSafeFindingIds !== undefined;
    if (!run || !api.analyzeCleanupCandidatesWithAI || !api.getAIFeatureReadiness
      || (isSelectedSafeAssessment && !api.getAnalysisFinding)) {
      reportError(t("storageCleanupAIUnsupported"));
      return;
    }
    if (interactionLockedRef.current || aiReadinessCheckInFlight.current) return;
    const targetFindingIds = isSelectedSafeAssessment
      ? [...new Set(selectedSafeFindingIds)]
      : [];
    if (isSelectedSafeAssessment && (activeTierRef.current !== "safe"
      || !targetFindingIds.length
      || targetFindingIds.some((id) => !selectedFindingIds.has(id)))) {
      setAiStatus(t("storageCleanupAISelectedSelectionChanged"));
      return;
    }
    const expectedScopeEpoch = scopeEpoch.current;
    const expectedTierEpoch = activeTierEpoch.current;
    const expectedTier = isSelectedSafeAssessment ? "safe" : activeTierRef.current;
    const reviewRunId = run.id;
    const reviewRunRevision = run.revision;
    const expectedSelectionRevision = selectionRevision.current;
    aiReadinessCheckInFlight.current = true;
    setIsCheckingAIReadiness(true);
    let currentReadiness: AIProductFeatureReadiness | null = null;
    try {
      currentReadiness = await refreshAIReadiness();
    } finally {
      aiReadinessCheckInFlight.current = false;
      setIsCheckingAIReadiness(false);
    }
    if (expectedScopeEpoch !== scopeEpoch.current
      || expectedTierEpoch !== activeTierEpoch.current
      || activeTierRef.current !== expectedTier
      || runRef.current?.id !== reviewRunId
      || (runRef.current?.revision ?? -1) < reviewRunRevision) return;
    if (isSelectedSafeAssessment && selectionRevision.current !== expectedSelectionRevision) {
      setAiStatus(t("storageCleanupAISelectedSelectionChanged"));
      return;
    }
    if (!currentReadiness || currentReadiness.cleanup.state !== "ready") {
      setAiStatus(currentReadiness
        ? cleanupReadinessText(
          currentReadiness.cleanup.state,
          currentReadiness.cleanup.reason,
          currentReadiness.cleanup.provider.state,
          currentReadiness.cleanup.provider.reason,
          t
        )
        : t("cleanupReadinessUnavailable"));
      return;
    }
    const operationEpoch = aiOperationEpoch.current + 1;
    aiOperationEpoch.current = operationEpoch;
    const operation: AiOperation = {
      id: ++aiOperationSequenceRef.current,
      epoch: operationEpoch,
      scopeEpoch: expectedScopeEpoch,
      tierEpoch: expectedTierEpoch,
      tier: expectedTier,
      runId: reviewRunId,
      action: isSelectedSafeAssessment ? "selected-safe" : "review",
      cancelRequested: false
    };
    aiOperationRef.current = operation;
    previewRequestEpoch.current += 1;
    previewSelectionFingerprint.current = null;
    setPreview(null);
    setConfirmPreviewOpen(false);
    setExecutionResult(null);
    setReviewFinding(null);
    updateAiWorkState("running");
    setAiStatus("");
    let processed = 0;
    let skipped = 0;
    let mutationFailed = 0;
    let readbackFailed = 0;
    let total = 0;
    const ownsOperation = () => expectedScopeEpoch === scopeEpoch.current
      && expectedTierEpoch === activeTierEpoch.current
      && activeTierRef.current === expectedTier
      && operationEpoch === aiOperationEpoch.current
      && aiOperationRef.current?.id === operation.id
      && !operation.cancelRequested
      && runRef.current?.id === reviewRunId
      && runRef.current.revision >= reviewRunRevision;
    const ownsSettlement = () => expectedScopeEpoch === scopeEpoch.current
      && expectedTierEpoch === activeTierEpoch.current
      && activeTierRef.current === expectedTier
      && aiOperationRef.current?.id === operation.id
      && runRef.current?.id === reviewRunId
      && runRef.current.revision >= reviewRunRevision;
    const canceledStatus = () => replaceCopy(isSelectedSafeAssessment
      ? "storageCleanupAISelectedCanceledSummary"
      : "storageCleanupAIRecheckCanceledSummary", {
      processed,
      total,
      skipped,
      mutationFailed,
      readbackFailed
    });
    const stopAfterCancellation = () => {
      if (!operation.cancelRequested) return false;
      if (aiOperationRef.current?.id === operation.id && expectedScopeEpoch === scopeEpoch.current) {
        setAiStatus(canceledStatus());
      }
      return true;
    };
    const reconcileSettledBatch = async (
      findingIds: readonly string[],
      options: { mutationFailed: boolean }
    ) => {
      if (!ownsSettlement()) return false;
      if (!findingIds.length) return true;
      const readbacks = await Promise.allSettled(findingIds.map(async (id) => {
        if (!api.getAnalysisFinding) return null;
        return api.getAnalysisFinding(id);
      }));
      if (!ownsSettlement()) return false;
      const authoritative: AnalysisFinding[] = [];
      const failedReadbackIds: string[] = [];
      readbacks.forEach((result, index) => {
        const id = findingIds[index];
        if (result.status === "fulfilled" && result.value && isAnalysisFinding(result.value)) authoritative.push(result.value);
        else failedReadbackIds.push(id);
      });
      readbackFailed += failedReadbackIds.length;
      if (authoritative.length) reconcileUpdatedFindings(authoritative);
      if (failedReadbackIds.length) invalidateReadbackFailures(failedReadbackIds);
      if (options.mutationFailed) removeSelectionsForIds(findingIds);
      return true;
    };
    const finalizeCanceledOperation = async () => {
      if (!ownsSettlement()) return false;
      await loadRunDetails(reviewRunId, true, expectedScopeEpoch);
      if (!ownsSettlement()) return false;
      const currentRun = runRef.current;
      if (currentRun) {
        await loadFindings(currentRun.id, activeTierRef.current, null, false, currentRun.revision);
        if (!ownsSettlement()) return false;
      }
      if (ownsSettlement()) setAiStatus(canceledStatus());
      return true;
    };
    try {
      const ids: string[] = [...targetFindingIds];
      if (!isSelectedSafeAssessment) {
        let cursor: string | null = null;
        do {
          if (operation.cancelRequested) {
            setAiStatus(canceledStatus());
            return;
          }
          if (!ownsOperation()) return;
          setAiStatus(replaceCopy("storageCleanupAIRecheckCollecting", { count: ids.length }));
          const page: AnalysisFindingPage | undefined = await api.listAnalysisFindings?.({
            runId: reviewRunId,
            tier: "review",
            status: "active",
            cursor,
            limit: FINDING_PAGE_SIZE
          });
          if (!page) throw new Error("cleanup_findings_unavailable");
          if (stopAfterCancellation()) return;
          if (!ownsOperation()) return;
          ids.push(...page.findings.filter((finding) => finding.tier === "review" && finding.status === "active").map((finding) => finding.id));
          cursor = page.nextCursor;
        } while (cursor);
      }
      total = ids.length;
      if (!ids.length) {
        setAiStatus(t(isSelectedSafeAssessment ? "storageCleanupAISelectedNoTargets" : "storageCleanupAINoTargets"));
        return;
      }
      for (let offset = 0; offset < ids.length; offset += AI_RECHECK_BATCH_SIZE) {
        if (operation.cancelRequested) {
          setAiStatus(canceledStatus());
          return;
        }
        if (!ownsOperation()) return;
        let batch = ids.slice(offset, offset + AI_RECHECK_BATCH_SIZE);
        setAiStatus(replaceCopy(
          isSelectedSafeAssessment ? "storageCleanupAISelectedWorkingProgress" : "storageCleanupAIRecheckWorkingProgress",
          { processed, total: ids.length }
        ));
        if (isSelectedSafeAssessment) {
          const beforeAnalysis = await Promise.allSettled(batch.map((id) => api.getAnalysisFinding!(id)));
          if (stopAfterCancellation()) return;
          if (!ownsOperation()) return;
          const eligible: AnalysisFinding[] = [];
          const rejectedIds: string[] = [];
          beforeAnalysis.forEach((result, index) => {
            const finding = result.status === "fulfilled" ? result.value : null;
            if (finding && isAnalysisFinding(finding) && finding.runId === reviewRunId
              && finding.tier === "safe" && isFindingSelectable(finding)) {
              eligible.push(finding);
            } else {
              rejectedIds.push(batch[index]);
            }
          });
          if (eligible.length) reconcileUpdatedFindings(eligible);
          if (rejectedIds.length) {
            skipped += rejectedIds.length;
            invalidateReadbackFailures(rejectedIds);
          }
          batch = eligible.map((finding) => finding.id);
          if (!batch.length) continue;
        }
        let updatedIds: string[] = [];
        let mutationFailedForBatch = false;
        try {
          const updated = await api.analyzeCleanupCandidatesWithAI(reviewRunId, batch);
          const batchIds = new Set(batch);
          const returnedIds = new Set(updated.map((candidate) => candidate.id).filter((id) => batchIds.has(id)));
          processed += returnedIds.size;
          skipped += Math.max(0, batch.length - returnedIds.size);
          // Re-read every requested item, including partial/no-op responses. The API response is not durable authority.
          updatedIds = [...batch];
        } catch {
          mutationFailedForBatch = true;
          mutationFailed += batch.length;
          updatedIds = [...batch];
        }
        if (!(await reconcileSettledBatch(updatedIds, { mutationFailed: mutationFailedForBatch && !isSelectedSafeAssessment }))) return;
        if (operation.cancelRequested) {
          await finalizeCanceledOperation();
          return;
        }
        if (!ownsOperation()) return;
        if (activeTierRef.current !== "review" || activeTierEpoch.current !== expectedTierEpoch) {
          // The durable mutation may finish after the user leaves Review; only the current tier may be reloaded below.
          continue;
        }
      }
      if (!ownsOperation()) return;
      setAiStatus(replaceCopy(isSelectedSafeAssessment
        ? "storageCleanupAISelectedDoneSummary"
        : "storageCleanupAIRecheckDoneSummary", { total, processed, skipped, mutationFailed, readbackFailed }));
      await loadRunDetails(reviewRunId, true, expectedScopeEpoch);
      if (!ownsOperation()) return;
      const currentRun = runRef.current;
      if (currentRun) await loadFindings(currentRun.id, activeTierRef.current, null, false, currentRun.revision);
    } catch (aiError) {
      if (stopAfterCancellation()) return;
      if (ownsOperation()) reportError(aiError);
    } finally {
      if (aiOperationRef.current?.id === operation.id) {
        aiOperationRef.current = null;
        updateAiWorkState("idle");
      }
    }
  }, [api, invalidateReadbackFailures, loadFindings, loadRunDetails, reconcileUpdatedFindings, refreshAIReadiness, removeSelectionsForIds, reportError, replaceCopy, run, selectedFindingIds, selectionRevision, t, updateAiWorkState]);

  const cancelAiRecheck = useCallback(() => {
    const operation = aiOperationRef.current;
    if (!operation || aiWorkStateRef.current !== "running") return;
    operation.cancelRequested = true;
    aiOperationEpoch.current += 1;
    setAiStatus(t(operation.action === "selected-safe"
      ? "storageCleanupAISelectedCanceling"
      : "storageCleanupAIRecheckCanceling"));
    updateAiWorkState("canceling");
  }, [t, updateAiWorkState]);

  const virtualizeFindings = findings.length > FINDING_VIRTUALIZATION_THRESHOLD;
  const virtualizer = useVirtualizer({
    count: findings.length,
    getScrollElement: () => findingListRef.current,
    estimateSize: () => FINDING_ROW_HEIGHT,
    overscan: 5,
    getItemKey: (index) => findings[index]?.id ?? index
  });
  useEffect(() => {
    if (!virtualizeFindings) return;
    const frame = requestAnimationFrame(() => {
      virtualizer.measure();
      findingListRef.current?.querySelectorAll<HTMLElement>("[data-index]").forEach((element) => virtualizer.measureElement(element));
    });
    return () => cancelAnimationFrame(frame);
  }, [activeTier, evidenceByFinding, expandedEvidence, findings, virtualizeFindings, virtualizer]);
  const virtualItems = virtualizer.getVirtualItems();
  const renderedVirtualItems = virtualItems.length
    ? virtualItems
    : findings.slice(0, FINDING_VIRTUALIZATION_THRESHOLD).map((_, index) => ({ index, start: index * FINDING_ROW_HEIGHT, size: FINDING_ROW_HEIGHT, key: findings[index]?.id ?? index }));

  const onFindingListKeyDown = useCallback((event: KeyboardEvent<HTMLDivElement>) => {
    if ((event.key === "Enter" || event.key === " ") && event.target === event.currentTarget) {
      event.preventDefault();
    }
  }, []);

  const runState = run ? durableRunState(run) : "idle";
  const runIsPartial = runState === "partial";
  const runScope = run ? scopePaths(run) : selectedRoots;
  const canReviewFindings = Boolean(run && !isRunInProgress(run) && run.findingsPublished > 0);
  const previewExecutableCount = preview?.previews.filter(isCleanupPreviewExecutable).length ?? 0;
  const previewBlockedCount = preview ? preview.previews.length - previewExecutableCount : 0;
  const previewScopeExecutable = preview
    ? isCleanupPreviewScopeExecutable(preview, selectedFindings.map((finding) => finding.id))
    : false;
  const cleanupWorkflowCurrent = executionResult ? 3 : preview ? 2 : run && canReviewFindings ? 1 : 0;
  const cleanupScanWorkflowState: WorkflowStepState = !run || runState === "running"
    ? "current"
    : runState === "failed"
      ? "blocked"
      : "done";
  const cleanupReviewWorkflowState: WorkflowStepState = cleanupWorkflowCurrent > 1
    ? "done"
    : cleanupWorkflowCurrent === 1
      ? "current"
      : "pending";
  const cleanupSafeTrashWorkflowState: WorkflowStepState = preview && !previewScopeExecutable
    ? "blocked"
    : cleanupWorkflowCurrent > 2
      ? "done"
      : cleanupWorkflowCurrent === 2
        ? "current"
        : "pending";
  const cleanupRestoreWorkflowState: WorkflowStepState = cleanupWorkflowCurrent === 3 ? "current" : "pending";
  const renderFindingRow = (finding: AnalysisFinding, index: number, start?: number) => (
    <FindingRow
      key={finding.id}
      finding={finding}
      selected={selectedFindingIds.has(finding.id)}
      evidence={evidenceByFinding[finding.id]}
      evidenceExpanded={expandedEvidence.has(finding.id)}
      t={t}
      index={index}
      virtualized={virtualizeFindings}
      measureElement={virtualizeFindings ? virtualizer.measureElement : undefined}
      style={virtualizeFindings && start !== undefined ? { transform: `translateY(${start}px)` } : undefined}
      onToggle={toggleFinding}
      onReveal={revealFinding}
      onToggleEvidence={toggleEvidence}
      onRevalidate={revalidateFinding}
      interactionLocked={isMutating || isAiWorking}
      tierLabel={tierLabel}
    />
  );

  return (
    <>
      <div className={cn(pageSurface, "grid content-start gap-4")} data-cleanup-authority="analysis-run-finding" data-cleanup-preview-state={preview ? "ready" : "none"}>
        <NoticeBanner tone="info" title={t("storageCleanupSafetyTitle")}>
          {t("storageCleanupSafetyDesc")}
        </NoticeBanner>

        <AIReadinessNotice
          feature="cleanup"
          readiness={aiReadiness}
          loading={aiReadinessLoading}
          error={aiReadinessError}
          t={t}
          setView={onNavigate ?? (() => undefined)}
          onRetry={() => void refreshAIReadiness().catch(() => undefined)}
        />

        <WorkflowSteps
          label={t("storageCleanupWorkflowLabel")}
          steps={[
            { label: t("storageCleanupWorkflowScan"), detail: t("storageCleanupWorkflowScanDetail"), state: cleanupScanWorkflowState, stateLabel: workflowStateLabel(cleanupScanWorkflowState, t) },
            { label: t("storageCleanupWorkflowReview"), detail: t("storageCleanupWorkflowReviewDetail"), state: cleanupReviewWorkflowState, stateLabel: workflowStateLabel(cleanupReviewWorkflowState, t) },
            { label: t("storageCleanupWorkflowSafeTrash"), detail: t("storageCleanupWorkflowSafeTrashDetail"), state: cleanupSafeTrashWorkflowState, stateLabel: workflowStateLabel(cleanupSafeTrashWorkflowState, t) },
            { label: t("storageCleanupWorkflowRestore"), detail: t("storageCleanupWorkflowRestoreDetail"), state: cleanupRestoreWorkflowState, stateLabel: workflowStateLabel(cleanupRestoreWorkflowState, t) }
          ]}
        />

        {run ? (
          <section className="grid grid-cols-1 divide-y divide-[var(--zc-divider)] border-y border-[var(--zc-divider)] sm:grid-cols-3 sm:divide-x sm:divide-y-0" data-cleanup-summary aria-label={t("storageCleanupRunMetricsLabel")}>
            <div className="grid gap-1 px-3 py-2 first:pl-0 sm:px-4 sm:first:pl-0"><span className={metadataText}>{t("storageCleanupSummaryCandidates")}</span><strong className="text-base tabular-nums text-[var(--zc-text-primary)]">{run.findingsPublished.toLocaleString()}</strong></div>
            <div className="grid gap-1 px-3 py-2 sm:px-4"><span className={metadataText}>{t("storageCleanupSummaryReclaimable")}</span><strong className="text-base tabular-nums text-[var(--zc-success-text)]">{formatBytes(runReclaimable.bytes)}</strong></div>
            <div className="grid gap-1 px-3 py-2 last:pr-0 sm:px-4 sm:last:pr-0"><span className={metadataText}>{t("storageCleanupSummaryRecovery")}</span><strong className="text-base text-[var(--zc-text-primary)]">{t("storageCleanupSummaryRecoveryValue")}</strong></div>
          </section>
        ) : null}

        <NoticeBanner tone="info" title={t("storageCleanupSafetyNoticeTitle")}>
          {t("storageCleanupSafetyNoticeDesc")}
        </NoticeBanner>

        <section className={cn(contentPanel, "grid gap-3 p-4")} data-cleanup-scope>
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <h2 className={sectionHeading}>{t("storageCleanupCurrentScope")}</h2>
              <p className={sectionDescription}>
                {selectedRoots.length
                  ? selectedRoots.map((root) => replaceCopy("storageCleanupScopeValue", { path: root })).join(" / ")
                  : t("storageCleanupNoScopeSelected")}
              </p>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Button variant="secondary" size="compact" disabled={isMutating || isAiWorking} onClick={() => void chooseScope().catch(() => undefined)}>
                <FolderOpen size={15} aria-hidden="true" />{t("storageCleanupChooseFolder")}
              </Button>
              <Button variant="primary" size="compact" disabled={!selectedRoots.length || isMutating || isAiWorking || isRunInProgress(run)} onClick={() => void startScan().catch(() => undefined)}>
                {isMutating && !run ? <LoaderCircle size={15} className="animate-spin" aria-hidden="true" /> : <Search size={15} aria-hidden="true" />}
                {run ? t("storageCleanupRescan") : t("storageCleanupScanScope")}
              </Button>
              {run && isRunInProgress(run) ? (
                <Button variant="secondary" size="compact" disabled={isMutating || isAiWorking || run.cancelRequested} onClick={() => void cancelRun().catch(() => undefined)}>
                  <XCircle size={15} aria-hidden="true" />{run.cancelRequested ? t("storageCleanupRunCanceling") : t("storageCleanupCancelScan")}
                </Button>
              ) : null}
            </div>
          </div>
          <div className="flex flex-wrap gap-2" aria-label={t("storageCleanupQuickScopesLabel")}>
            <Button variant="ghost" size="compact" disabled={isMutating || isAiWorking} onClick={() => void chooseQuickScope("downloads").catch(() => undefined)}>{t("storageCleanupQuickDownloads")}</Button>
            <Button variant="ghost" size="compact" disabled={isMutating || isAiWorking} onClick={() => void chooseQuickScope("desktop").catch(() => undefined)}>{t("storageCleanupQuickDesktop")}</Button>
            <Button variant="ghost" size="compact" disabled={isMutating || isAiWorking} onClick={() => void chooseQuickScope("documents").catch(() => undefined)}>{t("storageCleanupQuickDocuments")}</Button>
            <Button variant="ghost" size="compact" disabled={isMutating || isAiWorking} onClick={() => void chooseQuickScope("temp").catch(() => undefined)}>{t("storageCleanupQuickTemp")}</Button>
          </div>
        </section>

        {unsupported ? (
          <StateBlock tone="warning" title={t("storageCleanupDurableUnavailableTitle")} description={t("storageCleanupDurableUnavailableDesc")} />
        ) : null}
        {error ? <NoticeBanner tone="error" title={failedMutationKind === "preview" ? t("storageCleanupPreviewUnavailableTitle") : failedMutationKind === "safe_trash" ? t("storageCleanupExecutionFailed") : t("storageCleanupLoadFailed")} action={<Button variant="ghost" size="compact" onClick={() => { setError(""); setFailedMutationKind(null); }}>{t("close")}</Button>}>{error}</NoticeBanner> : null}

        {mutationKind === "preview" ? <DurableTaskStatus state="running" title={t("storageCleanupPreviewLoading")} description={t("storageCleanupPreviewUnavailableDesc")} density="compact" /> : null}
        {mutationKind === "safe_trash" ? <DurableTaskStatus state="running" title={t("storageCleanupExecuting")} description={t("storageCleanupSafetyNoticeDesc")} density="compact" /> : null}

        {loading ? <DurableTaskStatus state="running" title={t("storageCleanupDurableLoading")} description={t("storageCleanupDurableLoadingDesc")} density="compact" /> : null}

        {run ? (
          <section className="grid gap-3" data-analysis-run-id={run.id} data-cleanup-analysis-state={runState}>
            <MetricStrip
              ariaLabel={t("storageCleanupRunMetricsLabel")}
              density="compact"
              items={[
                { label: t("storageCleanupSafeTier"), value: run.safeCount.toLocaleString(), tone: "green" },
                { label: t("storageCleanupReviewTier"), value: run.reviewCount.toLocaleString(), tone: "amber" },
                { label: t("storageCleanupCautionTier"), value: run.cautionCount.toLocaleString(), tone: "red" },
                { label: t("storageCleanupReclaimable"), value: formatBytes(runReclaimable.bytes), hint: runReclaimable.estimated ? t("storageCleanupEstimateHint") : undefined }
              ]}
            />
            {isRunInProgress(run) ? (
              <DurableTaskStatus
                state="running"
                title={run.cancelRequested ? t("storageCleanupRunCanceling") : t("storageCleanupRunRunning")}
                description={replaceCopy("storageCleanupRunScope", { scope: runScope.map((path) => compactPath(path, 80)).join(" / ") || t("storageCleanupNoScopeSelected") })}
                progress={{
                  label: replaceCopy("storageCleanupDetectorProgress", { completed: run.detectorsCompleted, total: Math.max(run.detectorsTotal, 1) }),
                  value: run.detectorsCompleted,
                  max: Math.max(run.detectorsTotal, 1),
                  indeterminate: run.detectorsTotal === 0
                }}
                action={<Button variant="secondary" size="compact" disabled={isMutating || isAiWorking || run.cancelRequested} onClick={() => void cancelRun().catch(() => undefined)}>{t("storageCleanupCancelScan")}</Button>}
                density="compact"
              />
            ) : null}
            {runIsPartial ? (
              <NoticeBanner
                tone="warning"
                title={run.findingsPublished === 0 ? t("storageCleanupRunPartialEmptyTitle") : t("storageCleanupRunPartialTitle")}
                action={api.retryAnalysisRun ? <Button variant="secondary" size="compact" disabled={isMutating || isAiWorking} onClick={() => void retryRun().catch(() => undefined)}><RefreshCw size={14} aria-hidden="true" />{t("storageCleanupRunRetry")}</Button> : undefined}
              >
                {run.findingsPublished === 0
                  ? t("storageCleanupRunPartialEmptyDesc")
                  : replaceCopy("storageCleanupRunPartialDesc", { warnings: run.warningCount, errors: run.errorCount, failed: run.detectorsFailed })}
              </NoticeBanner>
            ) : null}
            {!isRunInProgress(run) && !runIsPartial && runState === "completed" ? (
              <NoticeBanner tone="success" title={run.findingsPublished === 0 ? t("storageCleanupRunCompletedEmptyTitle") : t("storageCleanupRunCompleted")}>
                {run.findingsPublished === 0
                  ? t("storageCleanupRunCompletedEmptyDesc")
                  : replaceCopy("storageCleanupRunCompletedDesc", { detectors: run.detectorsCompleted, findings: run.findingsPublished })}
              </NoticeBanner>
            ) : null}
            {runState === "failed" ? (
                <NoticeBanner tone="error" title={t("storageCleanupRunFailed")} action={api.retryAnalysisRun ? <Button variant="secondary" size="compact" disabled={isMutating || isAiWorking} onClick={() => void retryRun().catch(() => undefined)}>{t("storageCleanupRunRetry")}</Button> : undefined}>
                {run.errorMessage ? localizedStableError(run.errorMessage, t) : t("storageCleanupRunFailedDesc")}
              </NoticeBanner>
            ) : null}
            {runState === "canceled" ? <NoticeBanner tone="info" title={run.findingsPublished === 0 ? t("storageCleanupRunCanceledEmptyTitle") : t("storageCleanupRunCanceled")} action={<Button variant="secondary" size="compact" disabled={isMutating || isAiWorking} onClick={() => void startScan().catch(() => undefined)}>{t("storageCleanupRescan")}</Button>}>{run.findingsPublished === 0 ? t("storageCleanupRunCanceledEmptyDesc") : t("storageCleanupRunCanceledDesc")}</NoticeBanner> : null}
            {runDetectors.some((detector) => detector.status === "failed") ? <span className={quietText}>{t("storageCleanupDetectorFailureHint")}</span> : null}
          </section>
        ) : null}

        {!loading && !run ? (
          <StateBlock
            tone="neutral"
            title={t("storageCleanupChooseScopeEmptyTitle")}
            description={t("storageCleanupChooseScopeEmptyDesc")}
            primaryAction={<Button variant="primary" disabled={!selectedRoots.length || isMutating || isAiWorking} onClick={() => void startScan().catch(() => undefined)}><Search size={15} aria-hidden="true" />{t("storageCleanupScanScope")}</Button>}
          />
        ) : null}

        {run && canReviewFindings ? (
          <>
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div>
                <h2 className={sectionHeading}>{t("storageCleanupFindingsTitle")}</h2>
                <p className={sectionDescription}>{t("storageCleanupFindingsDescription")}</p>
              </div>
              {activeTier === "review" && api.analyzeCleanupCandidatesWithAI ? (
                <Button variant="secondary" size="compact" disabled={isCheckingAIReadiness || aiWorkState === "canceling" || isMutating || (aiWorkState === "idle" && (!run.reviewCount || aiReadiness?.cleanup.state !== "ready"))} onClick={() => {
                  if (aiWorkState === "running") cancelAiRecheck();
                  else if (aiWorkState === "idle") void recheckReviewFindings().catch(() => undefined);
                }}>
                  {aiWorkState !== "idle" || isCheckingAIReadiness ? <LoaderCircle size={14} className="animate-spin" aria-hidden="true" /> : <Sparkles size={14} aria-hidden="true" />}
                  {aiWorkState === "running" ? t("storageCleanupAIRecheckCancel") : aiWorkState === "canceling" ? t("storageCleanupAIRecheckCanceling") : t("storageCleanupAIRecheck")}
                </Button>
              ) : null}
            </div>
            {aiStatus ? <NoticeBanner tone="info" title={t("storageCleanupAIRecheck")}>{aiStatus}</NoticeBanner> : null}
            <SegmentedControl
              value={activeTier}
              ariaLabel={t("storageCleanupFindingTabsLabel")}
              disabled={isMutating || isAiWorking}
              onChange={changeActiveTier}
              options={[
                { value: "safe", label: replaceCopy("storageCleanupTierTab", { label: t("storageCleanupSafeTier"), count: run.safeCount }) },
                { value: "review", label: replaceCopy("storageCleanupTierTab", { label: t("storageCleanupReviewTier"), count: run.reviewCount }) },
                { value: "caution", label: replaceCopy("storageCleanupTierTab", { label: t("storageCleanupCautionTier"), count: run.cautionCount }) }
              ]}
            />
            <section className="min-w-0 rounded-[var(--zc-radius-panel)] border border-[var(--zc-border)] bg-[var(--zc-surface)]" data-cleanup-findings data-cleanup-findings-virtualized={virtualizeFindings}>
              {loadingFindings && !findings.length ? <DurableTaskStatus state="running" title={t("storageCleanupFindingsLoading")} description={t("storageCleanupFindingsLoadingDesc")} density="compact" /> : null}
              {!loadingFindings && !findings.length ? <StateBlock tone={activeTier === "caution" ? "info" : "neutral"} title={t("storageCleanupNoFindingsTitle")} description={t("storageCleanupNoFindingsDesc")} density="compact" /> : null}
              {findings.length ? (
                <div
                  ref={findingListRef}
                  className={cn("overflow-auto outline-none", virtualizeFindings ? "h-[min(62vh,720px)]" : "max-h-[min(62vh,720px)]")}
                  data-cleanup-findings-scroll-owner={virtualizeFindings ? "virtualizer" : "normal-flow"}
                  role="list"
                  aria-label={t("storageCleanupFindingsListLabel")}
                  tabIndex={0}
                  onKeyDown={onFindingListKeyDown}
                >
                  {virtualizeFindings ? (
                    <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
                      {renderedVirtualItems.map((virtualItem) => renderFindingRow(findings[virtualItem.index], virtualItem.index, virtualItem.start))}
                    </div>
                  ) : (
                    <div className="min-w-0">
                      {findings.map((finding, index) => renderFindingRow(finding, index))}
                    </div>
                  )}
                </div>
              ) : null}
              {nextCursor ? <div className="flex justify-center border-t border-[var(--zc-divider)] p-3"><Button variant="secondary" size="compact" disabled={loadingFindings || isMutating || isAiWorking} onClick={() => void loadFindings(run.id, activeTierRef.current, nextCursor, true, run.revision).catch(() => undefined)}>{t("storageCleanupLoadMore")}</Button></div> : null}
            </section>
          </>
        ) : null}

        {run && selectedFindingIds.size ? (
          <footer className="sticky bottom-0 z-10 flex flex-wrap items-center justify-between gap-3 rounded-[var(--zc-radius-panel)] border border-[var(--zc-border)] bg-[var(--zc-surface-overlay)] px-4 py-3 shadow-[var(--zc-shadow-raised)]" data-cleanup-selection-summary>
            <div className="min-w-0">
              <strong className="block text-sm text-[var(--zc-text-primary)]">{replaceCopy("storageCleanupSelectionSummary", { count: selectedFindingIds.size, size: formatBytes(selectedBytes) })}</strong>
              <span className={metadataText}>{t("storageCleanupSelectionSafety")}</span>
              <span className={`${metadataText} block`} data-cleanup-current-assessment-guidance>
                {selectedFindings.some((finding) => finding.tier === "safe")
                  ? selectedFindings.some((finding) => finding.tier === "review")
                    ? t("storageCleanupSelectionMixedAssessmentGuidance")
                    : t("storageCleanupSelectionSafeAssessmentGuidance")
                  : t("storageCleanupSelectionReviewAssessmentGuidance")}
              </span>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              {activeTier === "safe" && selectedFindings.some((finding) => finding.tier === "safe")
                && api.analyzeCleanupCandidatesWithAI && api.getAnalysisFinding ? (
                <Button
                  variant="secondary"
                  disabled={isMutating || isCheckingAIReadiness || (isAiWorking && aiOperationRef.current?.action !== "selected-safe")
                    || (aiOperationRef.current?.action === "selected-safe" && aiWorkState === "canceling")
                    || (!isAiWorking && aiReadiness?.cleanup.state !== "ready")}
                  onClick={() => {
                    if (aiOperationRef.current?.action === "selected-safe" && aiWorkState === "running") cancelAiRecheck();
                    else if (!isAiWorking) {
                      const safeIds = selectedFindings.filter((finding) => finding.tier === "safe").map((finding) => finding.id);
                      void recheckReviewFindings(safeIds).catch(() => undefined);
                    }
                  }}
                >
                  {isCheckingAIReadiness || (isAiWorking && aiOperationRef.current?.action === "selected-safe")
                    ? <LoaderCircle size={14} className="animate-spin" aria-hidden="true" />
                    : <Sparkles size={14} aria-hidden="true" />}
                  {aiOperationRef.current?.action === "selected-safe"
                    ? (aiWorkState === "canceling" ? t("storageCleanupAISelectedCanceling") : t("storageCleanupAISelectedCancel"))
                    : t("storageCleanupAISelectedAnalyze")}
                </Button>
              ) : null}
              <Button variant="primary" disabled={isMutating || isAiWorking || isCheckingAIReadiness || Boolean(mutationUnavailable) || !api.previewCleanupOperations} onClick={() => void previewSelected().catch(() => undefined)}><Trash2 size={15} aria-hidden="true" />{t("storageCleanupPreviewSelectedSafeTrash")}</Button>
            </div>
          </footer>
        ) : null}

        {executionResult ? (
          <NoticeBanner tone={executionResult.failed > 0 ? "warning" : "success"} title={executionResult.failed > 0 ? (executionResult.moved > 0 ? t("storageCleanupExecutionPartial") : t("storageCleanupExecutionFailed")) : t("storageCleanupExecutionDone")} action={<Button variant="secondary" size="compact" onClick={() => onNavigate?.("restore")}><History size={14} aria-hidden="true" />{t("storageCleanupExecutionHistory")}</Button>}>
            {replaceCopy("storageCleanupExecutionSummary", { moved: executionResult.moved, skipped: executionResult.skipped, failed: executionResult.failed })}
          </NoticeBanner>
        ) : null}
      </div>

      <SideSheet
        open={Boolean(preview)}
        title={t("storageCleanupPreviewReadyTitle")}
        description={t("storageCleanupPreviewReadyDesc")}
        closeLabel={t("close")}
        onClose={() => invalidatePreviewState()}
        footer={preview ? <div className="flex flex-wrap justify-end gap-2"><Button variant="primary" disabled={isMutating || isAiWorking || Boolean(mutationUnavailable) || !previewScopeExecutable} onClick={() => setConfirmPreviewOpen(true)}>{t("storageCleanupPreviewConfirm")}</Button></div> : undefined}
      >
        {preview ? (
          <div className="grid gap-4">
            <MetricStrip ariaLabel={t("storageCleanupPreviewMetricsLabel")} density="compact" items={[{ label: t("storageCleanupPreviewItems"), value: preview.total.toLocaleString() }, { label: t("storageCleanupPreviewExecutable"), value: previewExecutableCount.toLocaleString(), tone: "green" }, { label: t("storageCleanupPreviewBlocked"), value: previewBlockedCount.toLocaleString(), tone: "amber" }]} />
            {preview.previews.slice(0, 24).map((item) => <div key={item.id} className="grid gap-1 rounded-[var(--zc-radius-row)] border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] p-3"><strong className="truncate text-sm text-[var(--zc-text-primary)]">{item.old_name}</strong><span className={quietText}>{compactPath(item.source_path, 90)}</span><span className={metadataText}>{isCleanupPreviewExecutable(item) ? t("storageCleanupPreviewExecutable") : t("storageCleanupPreviewBlocked")}</span></div>)}
            {preview.truncated || preview.hasMore ? <NoticeBanner tone="info">{replaceCopy("storageCleanupPreviewTruncated", { shown: Math.min(preview.previews.length, 24), total: preview.total })}</NoticeBanner> : null}
          </div>
        ) : null}
      </SideSheet>

      <ConfirmDialog
        open={Boolean(reviewFinding)}
        tone="warning"
        title={t("storageCleanupReviewConfirmTitle")}
        description={reviewFinding ? replaceCopy("storageCleanupReviewConfirmDesc", { name: reviewFinding.title }) : undefined}
        emphasis={t("storageCleanupReviewConfirmEmphasis")}
        confirmLabel={t("storageCleanupReviewConfirmAction")}
        cancelLabel={t("cancel")}
        isProcessing={isMutating}
        onConfirm={() => void acknowledgeReviewFinding().catch(() => undefined)}
        onCancel={() => setReviewFinding(null)}
      />

      <ConfirmDialog
        open={confirmPreviewOpen && Boolean(preview)}
        tone="warning"
        title={t("storageCleanupConfirmSafeTrashTitle")}
        description={replaceCopy("storageCleanupConfirmSafeTrashDesc", { count: selectedFindingIds.size, size: formatBytes(selectedBytes) })}
        emphasis={t("storageCleanupConfirmSafeTrashEmphasis")}
        confirmLabel={t("storageCleanupMoveToSafeTrash")}
        cancelLabel={t("cancel")}
        isProcessing={isMutating}
        onConfirm={() => void moveSelectedToSafeTrash().catch(() => undefined)}
        onCancel={() => setConfirmPreviewOpen(false)}
      />
    </>
  );
}

function workflowStateLabel(state: WorkflowStepState, t: Translator): string {
  if (state === "done") return t("workflowStateDone");
  if (state === "current") return t("workflowStateCurrent");
  if (state === "blocked") return t("workflowStateBlocked");
  return t("workflowStatePending");
}

